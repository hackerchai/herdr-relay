use crate::{Error, Result};
use std::os::unix::{
    fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt},
    io::AsRawFd,
};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct NativeTarget {
    pub path: PathBuf,
    device: u64,
    inode: u64,
    changed: (i64, i64),
}

impl NativeTarget {
    pub fn same_instance(&self, other: &Self) -> bool {
        (self.device, self.inode) == (other.device, other.inode)
    }
    pub fn same_generation(&self, other: &Self) -> bool {
        self.same_instance(other) && self.changed == other.changed
    }
    pub fn reserve(&self) -> Result<std::fs::File> {
        let uid = unsafe { libc::geteuid() };
        let directory = PathBuf::from(format!("/tmp/herdr-relay-{uid}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::new("LOCAL_IO", e.to_string())),
        }
        let meta = std::fs::symlink_metadata(&directory)
            .map_err(|e| Error::new("LOCAL_IO", e.to_string()))?;
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(Error::new(
                "UNSAFE_TARGET",
                "Relay lock directory must be private and owned by this user",
            ));
        }
        // Lock by inode, not session spelling. Never unlink lock files: unlinking
        // would let a new process lock a different inode while a waiter holds it.
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join(format!("{}-{}.lock", self.device, self.inode)))
            .map_err(|e| Error::new("LOCAL_IO", e.to_string()))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            let code = if e.kind() == std::io::ErrorKind::WouldBlock {
                "TARGET_BUSY"
            } else {
                "LOCAL_IO"
            };
            return Err(Error::new(
                code,
                "another Relay client already occupies this Herdr instance",
            ));
        }
        Ok(lock)
    }
    pub fn session(config: &Path, name: &str) -> Result<Self> {
        if name.is_empty()
            || name.len() > 64
            || matches!(name, "." | "..")
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(Error::new(
                "INVALID_TARGET",
                "select an exact Herdr session name (1-64 ASCII letters, digits, dots, underscores or hyphens)",
            ));
        }
        let directory = if name == "default" {
            config.to_path_buf()
        } else {
            let entries = std::fs::read_dir(config.join("sessions"))
                .map_err(|e| Error::new("TARGET_OFFLINE", e.to_string()))?;
            let exact = entries
                .filter_map(std::result::Result::ok)
                .any(|entry| entry.file_name() == name);
            if !exact {
                return Err(Error::new(
                    "TARGET_OFFLINE",
                    "no running session with that exact spelling",
                ));
            }
            config.join("sessions").join(name)
        };
        let path = directory.join("herdr-client.sock");
        Self::socket(&path)
    }

    pub fn socket(path: &Path) -> Result<Self> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(Error::new(
                "INVALID_TARGET",
                "native socket path must be absolute without parent traversal",
            ));
        }
        let uid = unsafe { libc::geteuid() };
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            let metadata = std::fs::symlink_metadata(&current)
                .map_err(|e| Error::new("TARGET_OFFLINE", e.to_string()))?;
            if metadata.file_type().is_symlink() {
                // macOS exposes these system-owned aliases. No user-controlled
                // symlink is accepted anywhere in a target path.
                let system_alias = cfg!(target_os = "macos")
                    && metadata.uid() == 0
                    && matches!(current.to_str(), Some("/var" | "/tmp"))
                    && std::fs::canonicalize(&current).ok()
                        == Some(PathBuf::from("/private").join(current.strip_prefix("/").unwrap()));
                if !system_alias {
                    return Err(Error::new(
                        "UNSAFE_TARGET",
                        "symlinks are not accepted in a native socket path",
                    ));
                }
            }
            let metadata = std::fs::metadata(&current)
                .map_err(|e| Error::new("UNSAFE_TARGET", e.to_string()))?;
            if current == path {
                if !metadata.file_type().is_socket()
                    || metadata.uid() != uid
                    || metadata.mode() & 0o022 != 0
                {
                    return Err(Error::new(
                        "UNSAFE_TARGET",
                        "target must be a socket owned by this user and not writable by other users",
                    ));
                }
                return Ok(Self {
                    path: path.to_path_buf(),
                    device: metadata.dev(),
                    inode: metadata.ino(),
                    changed: (metadata.ctime(), metadata.ctime_nsec()),
                });
            }
            if !metadata.is_dir()
                || (metadata.uid() != uid && metadata.uid() != 0)
                || (metadata.mode() & 0o022 != 0
                    && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0))
            {
                return Err(Error::new(
                    "UNSAFE_TARGET",
                    "target directory ownership or permissions are unsafe",
                ));
            }
        }
        Err(Error::new("INVALID_TARGET", "missing native socket"))
    }

    pub fn check_current(&self) -> Result<()> {
        let current = Self::socket(&self.path)?;
        if !self.same_generation(&current) {
            return Err(Error::new(
                "TARGET_CHANGED",
                "Herdr instance changed; select it again",
            ));
        }
        Ok(())
    }
    pub async fn connect(&self) -> Result<tokio::net::UnixStream> {
        self.check_current()?;
        let stream = tokio::net::UnixStream::connect(&self.path)
            .await
            .map_err(|e| Error::new("TARGET_OFFLINE", e.to_string()))?;
        let peer = stream
            .peer_cred()
            .map_err(|e| Error::new("UNSAFE_TARGET", e.to_string()))?;
        if peer.uid() != unsafe { libc::geteuid() } {
            return Err(Error::new(
                "UNSAFE_TARGET",
                "native socket peer belongs to another user",
            ));
        }
        let current = Self::socket(&self.path)?;
        if !self.same_generation(&current) {
            return Err(Error::new(
                "TARGET_CHANGED",
                "Herdr instance changed during connection",
            ));
        }
        Ok(stream)
    }
}
