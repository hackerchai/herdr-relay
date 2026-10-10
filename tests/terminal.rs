use herdr_relay::terminal::TerminalState;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

#[test]
fn restores_terminal_flags_after_a_client_dies_in_raw_mode() {
    unsafe {
        let (mut master, mut slave) = (-1, -1);
        assert_eq!(
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            0
        );
        let _master = OwnedFd::from_raw_fd(master);
        let slave = OwnedFd::from_raw_fd(slave);
        let mut before = std::mem::zeroed::<libc::termios>();
        assert_eq!(libc::tcgetattr(slave.as_raw_fd(), &mut before), 0);
        let guard = TerminalState::capture(slave.as_fd()).unwrap();
        let mut raw = before;
        libc::cfmakeraw(&mut raw);
        assert_eq!(libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &raw), 0);
        guard.restore().unwrap();
        let mut after = std::mem::zeroed::<libc::termios>();
        assert_eq!(libc::tcgetattr(slave.as_raw_fd(), &mut after), 0);
        assert_eq!(
            (
                after.c_iflag,
                after.c_oflag,
                after.c_cflag,
                after.c_lflag,
                after.c_cc
            ),
            (
                before.c_iflag,
                before.c_oflag,
                before.c_cflag,
                before.c_lflag,
                before.c_cc
            )
        );
        // Offline keys are neither echoed nor handed to the next client.
        let input = herdr_relay::terminal::OfflineInput::capture(slave.as_fd()).unwrap();
        assert_eq!(
            libc::write(_master.as_raw_fd(), b"offline\n".as_ptr().cast(), 8),
            8
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(!input.discard().unwrap());
        assert_eq!(
            libc::write(_master.as_raw_fd(), b"\x03".as_ptr().cast(), 1),
            1
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(input.discard().unwrap());
        assert_eq!(
            libc::write(_master.as_raw_fd(), b"late\n".as_ptr().cast(), 5),
            5
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        drop(input);
        let mut queued = -1;
        assert_eq!(
            libc::ioctl(slave.as_raw_fd(), libc::FIONREAD, &mut queued),
            0
        );
        assert_eq!(
            queued, 0,
            "restoring the terminal must flush unread offline keys"
        );
    }
}
