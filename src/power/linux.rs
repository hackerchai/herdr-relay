//! logind owns the read end of each inhibitor FIFO; our FDs own the locks.
use super::{Inhibitor, Kind, PowerApi, Protection};
use std::os::fd::{AsRawFd, OwnedFd};

pub(super) struct Api;
impl PowerApi for Api {
    async fn acquire(&self, kind: Kind) -> Result<Box<dyn Inhibitor>, Protection> {
        let what = match kind {
            Kind::Idle => "idle",
            Kind::Sleep => "sleep",
            Kind::Lid => "handle-lid-switch",
        };
        let connection = zbus::Connection::system().await.map_err(unavailable)?;
        let response = connection
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &(what, "Herdr Relay", "Sharing Herdr sessions", "block"),
            )
            .await
            .map_err(unavailable)?;
        let fd: zbus::zvariant::OwnedFd = response.body().deserialize().map_err(unavailable)?;
        // No helper process or persistent bus connection is needed. Closing this
        // CLOEXEC descriptor (including on a crash) releases only our inhibitor.
        Ok(Box::new(Lock(fd.into())))
    }
}

fn unavailable(error: zbus::Error) -> Protection {
    let reason = match &error {
        zbus::Error::MethodError(name, _, _)
            if name.as_str().ends_with("AccessDenied")
                || name.as_str().ends_with("InteractiveAuthorizationRequired") =>
        {
            "logind permission denied; see docs/sleep-protection.md for headless setup".into()
        }
        _ => format!("logind unavailable: {error}"),
    };
    Protection::Unavailable(reason)
}

struct Lock(OwnedFd);
impl Inhibitor for Lock {
    fn is_held(&self) -> bool {
        let mut fd = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: 0,
            revents: 0,
        };
        // poll always reports ERR/HUP/NVAL, even with no requested events. A
        // lost logind reader makes its write-side inhibitor FD report an error.
        unsafe { libc::poll(&mut fd, 1, 0) == 0 }
    }
}
