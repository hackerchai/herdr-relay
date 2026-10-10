use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};

/// A saved terminal configuration, independent of the native client's lifetime.
pub struct TerminalState {
    fd: OwnedFd,
    original: libc::termios,
}

impl TerminalState {
    pub fn capture(fd: BorrowedFd<'_>) -> std::io::Result<Self> {
        let fd = fd.try_clone_to_owned()?;
        let mut original = std::mem::MaybeUninit::uninit();
        if unsafe { libc::tcgetattr(fd.as_raw_fd(), original.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            original: unsafe { original.assume_init() },
        })
    }

    pub fn restore(&self) -> std::io::Result<()> {
        // Discard pending keystrokes before handing the terminal back. On
        // macOS, TCSANOW can also set PENDIN while switching out of raw mode.
        if unsafe { libc::tcsetattr(self.fd.as_raw_fd(), libc::TCSAFLUSH, &self.original) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Temporary input ownership while no native client is running. Reads are
/// nonblocking via VMIN/VTIME, without changing shared file-description flags.
pub struct OfflineInput {
    state: TerminalState,
}
impl OfflineInput {
    pub fn capture(fd: BorrowedFd<'_>) -> std::io::Result<Self> {
        let state = TerminalState::capture(fd)?;
        let mut mode = state.original;
        mode.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHONL | libc::ISIG | libc::IEXTEN);
        mode.c_cc[libc::VMIN] = 0;
        mode.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(state.fd.as_raw_fd(), libc::TCSAFLUSH, &mode) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { state })
    }
    /// Discard a bounded batch. Ctrl-C cancels; no other byte is retained.
    pub fn discard(&self) -> std::io::Result<bool> {
        let mut bytes = [0u8; 4096];
        let n = unsafe {
            libc::read(
                self.state.fd.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if n < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) {
                return Ok(false);
            }
            return Err(error);
        }
        Ok(bytes[..n as usize].contains(&3))
    }
}
