use crate::{Error, Result};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Data,
    Credit,
}
pub struct Frame<'a> {
    pub kind: Kind,
    pub channel: [u8; 16],
    pub offset: u64,
    pub payload: &'a [u8],
}
impl<'a> Frame<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self> {
        if !(32..=16416).contains(&bytes.len()) || bytes[0] != 1 || bytes[2..4] != [0, 0] {
            return Err(invalid());
        }
        let kind = match bytes[1] {
            1 => Kind::Data,
            2 => Kind::Credit,
            _ => return Err(invalid()),
        };
        let length = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
        if length != bytes.len() - 32 || (kind == Kind::Credit && length != 0) {
            return Err(invalid());
        }
        Ok(Self {
            kind,
            channel: bytes[4..20].try_into().unwrap(),
            offset: u64::from_be_bytes(bytes[20..28].try_into().unwrap()),
            payload: &bytes[32..],
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.payload.len() > 16384 || (self.kind == Kind::Credit && !self.payload.is_empty()) {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(32 + self.payload.len());
        bytes.extend_from_slice(&[1, if self.kind == Kind::Data { 1 } else { 2 }, 0, 0]);
        bytes.extend_from_slice(&self.channel);
        bytes.extend_from_slice(&self.offset.to_be_bytes());
        bytes.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.payload);
        Ok(bytes)
    }
}
fn invalid() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid relay frame")
}
