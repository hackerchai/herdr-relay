use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader, copy, split};

pub async fn bridge_with_handshake<A, B>(
    a: A,
    b: B,
    deadline: std::time::Duration,
) -> crate::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    bridge_native(a, b, deadline).await.map(|_| ())
}

#[derive(Debug, PartialEq, Eq)]
pub enum NativeEnd {
    Client,
    Server,
}

pub async fn bridge_native<A, B>(
    a: A,
    b: B,
    deadline: std::time::Duration,
) -> crate::Result<NativeEnd>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut ar, mut aw) = split(a);
    let (br, mut bw) = split(b);
    let mut br = BufReader::with_capacity(16 * 1024, br);
    let lost = |e: std::io::Error| crate::Error::new("CONNECTION_LOST", e.to_string());
    tokio::select! {
        result = copy(&mut ar, &mut bw) => result.map(|_| NativeEnd::Client).map_err(lost),
        result = async {
            // Peek through BufReader: every byte of the initial reply, including
            // any coalesced native frames, remains in the same bounded stream.
            let first = tokio::time::timeout(deadline, br.fill_buf()).await
                .map_err(|_| crate::Error::new("HANDSHAKE_TIMEOUT", "Herdr did not answer the native hello within 4 seconds"))?
                .map_err(lost)?;
            if first.is_empty() { return Err(crate::Error::new("HANDSHAKE_FAILED", "Herdr closed before answering native hello")); }
            // copy flushes the TLS writer whenever its source becomes idle.
            // copy_buf only flushes at EOF, stranding the tail of a screen
            // until unrelated client input happens to drive the TLS session.
            copy(&mut br, &mut aw).await.map(|_| NativeEnd::Server).map_err(lost)
        } => result,
    }
}

/// Copies opaque native bytes, ending both directions when either side closes.
pub async fn bridge<A, B>(a: A, b: B) -> std::io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut ar, mut aw) = split(a);
    let (mut br, mut bw) = split(b);
    tokio::select! {
        result = copy(&mut ar, &mut bw) => result.map(|_| ()),
        result = copy(&mut br, &mut aw) => result.map(|_| ()),
    }
}
