use herdr_relay::{identity::Identity, names, pairing::Store};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::PermissionsExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Terminal {
    child: Child,
    master: File,
    output: String,
}

impl Terminal {
    fn start(command: &mut Command) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize {
            ws_row: 30,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (master, slave) = unsafe {
            assert_eq!(
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut size
                ),
                0
            );
            (File::from_raw_fd(master), File::from_raw_fd(slave))
        };
        let child = command
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .unwrap();
        Self {
            child,
            master,
            output: String::new(),
        }
    }

    fn until(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.output.contains(expected) {
            assert!(
                Instant::now() < deadline,
                "missing {expected:?}: {:?}",
                self.output
            );
            let mut fd = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut fd, 1, 50) } > 0 {
                let mut bytes = [0; 16384];
                let count = self.master.read(&mut bytes).unwrap();
                assert!(count > 0, "terminal closed: {:?}", self.output);
                self.output
                    .push_str(&String::from_utf8_lossy(&bytes[..count]));
            }
        }
    }

    fn keys(&mut self, keys: &[u8]) {
        self.output.clear();
        self.master.write_all(keys).unwrap();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn unavailable_hosts_stay_in_host_menu_and_keep_management_actions() {
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    // Keep a TCP port reserved without listening: checks fail locally, without
    // contacting a real relay or using any of the user's paired hosts.
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket
        .bind(
            &"127.0.0.1:0"
                .parse::<std::net::SocketAddr>()
                .unwrap()
                .into(),
        )
        .unwrap();
    let origin = format!(
        "https://{}",
        socket.local_addr().unwrap().as_socket().unwrap()
    );
    let host = temp.path().join("host");
    let client = temp.path().join("client");
    Identity::initialize(&host).unwrap();
    let identity = Identity::initialize(&client).unwrap();
    let publisher = Store::open(&host, &origin).unwrap();
    let connector = Store::open(&client, &origin).unwrap();
    let now = herdr_relay::pairing::now();
    let invitation = publisher.invite(now).unwrap();
    let request = connector.prepare(&invitation, now).unwrap();
    let receipt = publisher
        .redeem(&identity.certificate, &request, now)
        .unwrap();
    connector
        .complete(&invitation, &request, &receipt, now)
        .unwrap();
    names::remember(
        &client,
        &origin,
        &invitation.publisher,
        Some("offline-host"),
        None,
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-relay"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("HERDR_") {
            command.env_remove(key);
        }
    }
    command
        .args(["--profile"])
        .arg(&client)
        .args(["connect", "--origin", &origin])
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("HERDR_RELAY_UPDATE_CHECK", "off")
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .env("TERM", "xterm-256color");
    let mut terminal = Terminal::start(&mut command);
    terminal.until("checking…");
    terminal.keys(b"\ra");
    terminal.until("unique local alias");
    assert!(!terminal.output.contains("Working…"));
    terminal.keys(b"\x1b");
    terminal.until("unavailable");
    // Filtering must preserve disabled state and the original row index.
    // Enter stays on the list, and a following rename still works there.
    terminal.keys(b"/offline-host\r\ra");
    terminal.until("unique local alias");
    assert!(!terminal.output.contains("Working…"));
    assert!(!terminal.output.contains("NETWORK_LOST"));
    terminal.keys(b"\x1b");
    terminal.until("checking…");
    // The enabled pairing action must still open while hosts are unavailable.
    terminal.keys(b"j\r");
    terminal.until("pasted");
}
