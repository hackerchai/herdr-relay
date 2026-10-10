use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Pair,
    Control,
    Attach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub version: u8,
    pub origin: String,
    pub publisher: String,
    pub connector: String,
    pub purpose: Purpose,
    pub grant_version: Option<u64>,
    pub target: Option<String>,
    pub channel_id: String,
    pub boot_id: String,
}

pub async fn bind<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    expected: &Context,
) -> Result<()> {
    expected.validate()?;
    let encoded = serde_json::to_vec(expected).map_err(|_| protocol())?;
    if encoded.len() > 8192 {
        return Err(protocol());
    }
    let (mut reader, mut writer) = tokio::io::split(stream);
    let send = async {
        writer.write_u32(encoded.len() as u32).await.map_err(io)?;
        writer.write_all(&encoded).await.map_err(io)?;
        writer.flush().await.map_err(io)
    };
    let receive = async {
        let length = reader.read_u32().await.map_err(io)? as usize;
        if length == 0 || length > 8192 {
            return Err(protocol());
        }
        let mut encoded = vec![0; length];
        reader.read_exact(&mut encoded).await.map_err(io)?;
        let actual: Context = serde_json::from_slice(&encoded).map_err(|_| protocol())?;
        if &actual != expected {
            return Err(Error::new(
                "CONTEXT_MISMATCH",
                "peer origin, identities, purpose, target or generation did not match",
            ));
        }
        Ok(())
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::try_join!(send, receive)
    })
    .await
    .map_err(|_| Error::new("BIND_TIMEOUT", "peer context exchange exceeded 10 seconds"))??;
    Ok(())
}

impl Context {
    pub fn validate(&self) -> Result<()> {
        let origin = url::Url::parse(&self.origin).map_err(|_| protocol())?;
        let hex_id = |s: &str, length| {
            s.len() == length
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if self.version != 1
            || match self.purpose {
                Purpose::Pair => self.grant_version.is_some(),
                _ => !self
                    .grant_version
                    .is_some_and(|v| v > 0 && v <= 9_007_199_254_740_991),
            }
            || origin.scheme() != "https"
            || origin.origin().ascii_serialization() != self.origin
            || !origin.username().is_empty()
            || origin.password().is_some()
            || !hex_id(&self.publisher, 64)
            || !hex_id(&self.connector, 64)
            || !hex_id(&self.channel_id, 32)
            || !hex_id(&self.boot_id, 32)
            || self.target.as_ref().is_some_and(|s| !hex_id(s, 32))
            || (matches!(self.purpose, Purpose::Attach) != self.target.is_some())
        {
            return Err(protocol());
        }
        Ok(())
    }
}

fn protocol() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid or oversized channel context")
}
fn io(e: std::io::Error) -> Error {
    Error::new("CONNECTION_LOST", e.to_string())
}

pub fn random_id() -> Result<String> {
    let mut bytes = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| Error::new("RANDOM_FAILED", "operating system randomness unavailable"))?;
    Ok(hex::encode(bytes))
}

pub async fn send<S: AsyncWrite + Unpin, T: Serialize>(stream: &mut S, message: &T) -> Result<()> {
    let encoded = serde_json::to_vec(message).map_err(|_| protocol())?;
    if encoded.is_empty() || encoded.len() > 8192 {
        return Err(protocol());
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        stream.write_u32(encoded.len() as u32).await.map_err(io)?;
        stream.write_all(&encoded).await.map_err(io)?;
        stream.flush().await.map_err(io)
    })
    .await
    .map_err(|_| Error::new("CONTROL_TIMEOUT", "control send exceeded 10 seconds"))?
}

pub async fn receive<S: AsyncRead + Unpin, T: serde::de::DeserializeOwned>(
    stream: &mut S,
) -> Result<T> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let length = stream.read_u32().await.map_err(io)? as usize;
        if length == 0 || length > 8192 {
            return Err(protocol());
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await.map_err(io)?;
        serde_json::from_slice(&bytes).map_err(|_| protocol())
    })
    .await
    .map_err(|_| Error::new("CONTROL_TIMEOUT", "control receive exceeded 10 seconds"))?
}
