use crate::{Error, Result};
use rcgen::{CertificateParams, KeyPair, PKCS_ED25519, PublicKeyData};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub const DEVICE_NAME: &str = "device.herdr.invalid";

pub struct Identity {
    pub id: String,
    pub certificate: CertificateDer<'static>,
    pub(crate) private_key: PrivateKeyDer<'static>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Identity {
    pub fn initialize(profile: &Path) -> Result<Self> {
        private_directory(profile, true)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(profile.join("identity.lock"))
            .map_err(storage)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::new(
                "IDENTITY_BUSY",
                "another process is initializing this identity",
            ));
        }
        let directory = profile.join("identity");
        match std::fs::symlink_metadata(&directory) {
            Ok(_) => return Self::load(profile),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(storage(e)),
        }
        let key = KeyPair::generate_for(&PKCS_ED25519).map_err(crypto)?;
        let mut params = CertificateParams::new(vec![DEVICE_NAME.to_owned()]).map_err(crypto)?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, DEVICE_NAME);
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(3650);
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let certificate = params.self_signed(&key).map_err(crypto)?;
        let stage = tempfile::Builder::new()
            .prefix(".identity-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(profile)
            .map_err(storage)?;
        write_private(&stage.path().join("device.pk8"), &key.serialize_der())?;
        write_private(&stage.path().join("device.der"), certificate.der())?;
        File::open(stage.path())
            .and_then(|f| f.sync_all())
            .map_err(storage)?;
        // Both files become visible together. The profile lock serializes init,
        // and a partially missing existing identity is never regenerated.
        std::fs::rename(stage.path(), &directory).map_err(storage)?;
        File::open(profile)
            .and_then(|f| f.sync_all())
            .map_err(storage)?;
        Self::load(profile)
    }

    pub fn load(profile: &Path) -> Result<Self> {
        private_directory(profile, false)?;
        private_directory(&profile.join("identity"), false)?;
        let key = read_private(&profile.join("identity/device.pk8"))?;
        let key = PrivatePkcs8KeyDer::from(key);
        let pair = KeyPair::from_pkcs8_der_and_sign_algo(&key, &PKCS_ED25519).map_err(crypto)?;
        let certificate = CertificateDer::from(read_private(&profile.join("identity/device.der"))?);
        let spki = certificate_spki(&certificate)?;
        if pair.subject_public_key_info() != spki {
            return Err(Error::new(
                "IDENTITY_CORRUPT",
                "certificate and private key do not match; restore the original identity or create a new profile and pair again",
            ));
        }
        let id = hex::encode(Sha256::digest(spki));
        Ok(Self {
            id,
            certificate,
            private_key: key.into(),
        })
    }
}

pub(crate) fn private_directory(path: &Path, create: bool) -> Result<()> {
    directory(path, create, true)
}

pub(crate) fn user_directory(path: &Path, create: bool) -> Result<()> {
    directory(path, create, false)
}

fn directory(path: &Path, create: bool, private: bool) -> Result<()> {
    use std::path::{Component, PathBuf};
    if !path.is_absolute() || path.components().any(|p| matches!(p, Component::ParentDir)) {
        return Err(Error::new(
            "UNSAFE_IDENTITY",
            "profile path must be absolute without parent traversal",
        ));
    }
    let uid = unsafe { libc::geteuid() };
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if create {
            match std::fs::DirBuilder::new().mode(0o700).create(&current) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(storage(e)),
            }
        }
        let meta = std::fs::symlink_metadata(&current).map_err(storage)?;
        if meta.file_type().is_symlink() {
            let system_alias = cfg!(target_os = "macos")
                && meta.uid() == 0
                && matches!(current.to_str(), Some("/var" | "/tmp"))
                && std::fs::canonicalize(&current).ok()
                    == Some(PathBuf::from("/private").join(current.strip_prefix("/").unwrap()));
            if !system_alias {
                return Err(Error::new(
                    "UNSAFE_IDENTITY",
                    "profile paths must not contain user symlinks",
                ));
            }
        }
        let meta = std::fs::metadata(&current).map_err(storage)?;
        if !meta.is_dir()
            || (meta.uid() != uid && meta.uid() != 0)
            || (meta.mode() & 0o022 != 0 && !(meta.uid() == 0 && meta.mode() & 0o1000 != 0))
            || (current == path && (meta.uid() != uid || (private && meta.mode() & 0o077 != 0)))
        {
            return Err(Error::new(
                "UNSAFE_IDENTITY",
                "profile must be owned by this user with mode 0700 and safe parent directories",
            ));
        }
    }
    Ok(())
}

pub fn certificate_spki(der: &[u8]) -> Result<Vec<u8>> {
    use x509_parser::prelude::FromDer;
    let (rest, cert) = x509_parser::certificate::X509Certificate::from_der(der)
        .map_err(|_| Error::new("INVALID_CERTIFICATE", "invalid device certificate"))?;
    let public = cert.public_key();
    if !rest.is_empty()
        || public.algorithm.algorithm.to_id_string() != "1.3.101.112"
        || public.algorithm.parameters.is_some()
        || public.subject_public_key.data.len() != 32
        || public.subject_public_key.unused_bits != 0
    {
        return Err(Error::new(
            "INVALID_CERTIFICATE",
            "device identity requires an Ed25519 SPKI",
        ));
    }
    Ok(public.raw.to_vec())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(storage)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(storage)
}

fn read_private(path: &Path) -> Result<Vec<u8>> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path).map_err(|_| Error::new("IDENTITY_MISSING", "identity files unavailable; restore them or initialize a new profile and pair again"))?;
    let metadata = file.metadata().map_err(storage)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 8192
    {
        return Err(Error::new(
            "UNSAFE_IDENTITY",
            "identity files must be private, owned by this user, and at most 8 KiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes).map_err(storage)?;
    if bytes.len() > 8192 {
        return Err(Error::new(
            "UNSAFE_IDENTITY",
            "identity file exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn storage(e: std::io::Error) -> Error {
    Error::new("IDENTITY_STORAGE", e.to_string())
}
fn crypto(_e: rcgen::Error) -> Error {
    Error::new(
        "IDENTITY_CORRUPT",
        "cannot generate or read Ed25519 identity",
    )
}
