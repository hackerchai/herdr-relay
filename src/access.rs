use crate::{
    Error, Result,
    identity::{Identity, certificate_spki},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub version: u8,
    pub origin: String,
    pub method: String,
    pub path: String,
    pub device_id: String,
    pub public_key: String,
    pub nonce: String,
    pub expires_at: String,
    pub payload_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub challenge: Challenge,
    pub signature: String,
}

impl Challenge {
    pub fn canonical(&self) -> Result<String> {
        crate::pairing::validate_origin(&self.origin)?;
        let expiration = self.expires_at.parse::<u64>().map_err(|_| invalid())?;
        let id = |s: &str, size| {
            s.len() == size
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let parts: Vec<_> = self.path.split('/').collect();
        let route = parts.len() >= 5
            && parts[0].is_empty()
            && parts[1] == "v1"
            && parts[2] == "devices"
            && id(parts[3], 64)
            && match (self.method.as_str(), parts[4]) {
                ("GET", "control") => parts.len() == 5,
                ("POST", "invitations" | "pair" | "state" | "access" | "status") => {
                    parts.len() == 5
                }
                ("POST", "channels") => {
                    parts.len() == 7
                        && id(parts[5], 32)
                        && matches!(parts[6], "release" | "authenticated")
                }
                ("GET", "channels") => {
                    parts.len() == 7
                        && id(parts[5], 32)
                        && matches!(parts[6], "publisher" | "connector")
                }
                _ => false,
            };
        if self.version != 1
            || self.origin.len() > 512
            || !(route || (self.method == "POST" && self.path == "/v1/operator/status"))
            || !id(&self.device_id, 64)
            || !id(&self.public_key, 64)
            || !id(&self.nonce, 32)
            || !id(&self.payload_sha256, 64)
            || expiration == 0
            || expiration > 9_007_199_254_740_991
            || expiration.to_string() != self.expires_at
        {
            return Err(invalid());
        }
        let spki = hex::decode(format!("302a300506032b6570032100{}", self.public_key))
            .map_err(|_| invalid())?;
        if hex::encode(Sha256::digest(spki)) != self.device_id {
            return Err(invalid());
        }
        Ok(format!(
            "herdr-relay-access-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            self.origin,
            self.method,
            self.path,
            self.device_id,
            self.nonce,
            self.expires_at,
            self.payload_sha256
        ))
    }
    /// Whether a challenge names exactly this operation. A pre-issued
    /// challenge for anything else must not be signed.
    pub fn issued_for(
        &self,
        origin: &str,
        method: &str,
        path: &str,
        public_key: &str,
        payload: &str,
    ) -> bool {
        self.origin == origin
            && self.method == method
            && self.path == path
            && self.public_key == public_key
            && self.payload_sha256 == hex::encode(Sha256::digest(payload.as_bytes()))
    }
    pub fn sign(self, identity: &Identity, now: u64) -> Result<Proof> {
        let bytes = self.canonical()?;
        let expiration: u64 = self.expires_at.parse().map_err(|_| invalid())?;
        if self.device_id != identity.id {
            return Err(invalid());
        }
        // The Relay enforces its original 60-second nonce lifetime. Endpoint
        // clocks may differ slightly; signing must never extend that lifetime
        // or turn an ordinary clock offset into an identity failure.
        const CLOCK_SKEW_SECONDS: u64 = 300;
        if expiration.saturating_add(CLOCK_SKEW_SECONDS) <= now
            || expiration > now.saturating_add(60 + CLOCK_SKEW_SECONDS)
        {
            return Err(Error::new(
                "CLOCK_SKEW",
                "local clock differs from the relay; enable automatic time synchronization",
            ));
        }
        let provider = rustls::crypto::ring::default_provider();
        let key = provider
            .key_provider
            .load_private_key(identity.private_key.clone_key())
            .map_err(|_| invalid())?;
        let signer = key
            .choose_scheme(&[rustls::SignatureScheme::ED25519])
            .ok_or_else(invalid)?;
        let signature = signer.sign(bytes.as_bytes()).map_err(|_| invalid())?;
        Ok(Proof {
            challenge: self,
            signature: hex::encode(signature),
        })
    }
}
pub fn public_key(identity: &Identity) -> Result<String> {
    let spki = certificate_spki(&identity.certificate)?;
    Ok(hex::encode(&spki[spki.len() - 32..]))
}
fn invalid() -> Error {
    Error::new(
        "ACCESS_INVALID",
        "invalid or expired relay access challenge",
    )
}
