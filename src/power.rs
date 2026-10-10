//! Process-scoped sleep protection for a running server.
use serde::{Deserialize, Serialize};
use std::future::Future;

#[cfg(target_os = "macos")]
#[path = "power/macos.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "power/linux.rs"]
mod native;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod native {
    pub struct Api;
    impl super::PowerApi for Api {
        async fn acquire(
            &self,
            _: super::Kind,
        ) -> Result<Box<dyn super::Inhibitor>, super::Protection> {
            Err(super::Protection::Unsupported(
                "requires macOS or Linux with logind".into(),
            ))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum Protection {
    Checking,
    Held,
    Unavailable(String),
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub idle_sleep: Protection,
    pub system_sleep: Protection,
    pub lid_close: Protection,
}

impl Status {
    fn protection(&mut self, kind: Kind) -> &mut Protection {
        match kind {
            Kind::Idle => &mut self.idle_sleep,
            Kind::Sleep => &mut self.system_sleep,
            Kind::Lid => &mut self.lid_close,
        }
    }
}

impl std::fmt::Display for Protection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Checking => f.write_str("checking"),
            Self::Held => f.write_str("inhibitor held"),
            Self::Unavailable(reason) => write!(f, "unavailable ({reason})"),
            Self::Unsupported(reason) => write!(f, "OS controlled ({reason})"),
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Sleep protection:\n  Idle sleep: {}\n  System suspend: {}\n  Lid close: {}",
            self.idle_sleep, self.system_sleep, self.lid_close
        )
    }
}

pub const HELP: &str = "Sleep protection is enabled while the server runs, including reconnects.\nStopping the service releases its inhibitors. The display may turn off.\nmacOS: prevents idle sleep; lid-close and forced sleep remain OS controlled.\nFor MacBook closed-display use, connect power, an external display and input devices.\nLinux: requests idle, suspend and lid-switch inhibitors from logind; permissions may be required.\nA VM cannot keep its physical host awake. Check service status for the actual result.";

#[derive(Clone, Copy)]
enum Kind {
    Idle,
    Sleep,
    Lid,
}

// This boundary is the native OS API. Tests replace only the OS and its clock.
trait Inhibitor: Send {
    fn is_held(&self) -> bool;
}
trait PowerApi: Send + 'static {
    fn acquire(
        &self,
        kind: Kind,
    ) -> impl Future<Output = Result<Box<dyn Inhibitor>, Protection>> + Send;
}

pub struct Guard {
    status: tokio::sync::watch::Receiver<Status>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Guard {
    pub fn start() -> Self {
        Self::with_api(native::Api)
    }

    fn with_api(api: impl PowerApi) -> Self {
        let (sender, status) = tokio::sync::watch::channel(Status {
            idle_sleep: Protection::Checking,
            system_sleep: Protection::Checking,
            lid_close: Protection::Checking,
        });
        let task = tokio::spawn(async move {
            let mut locks: [Option<Box<dyn Inhibitor>>; 3] = [None, None, None];
            loop {
                for (kind, lock) in [Kind::Idle, Kind::Sleep, Kind::Lid]
                    .into_iter()
                    .zip(&mut locks)
                {
                    if lock.as_ref().is_some_and(|held| held.is_held()) {
                        continue;
                    }
                    if matches!(
                        sender.borrow().clone().protection(kind),
                        Protection::Unsupported(_)
                    ) {
                        continue;
                    }
                    // OS power services can restart independently of Relay.
                    *lock = None;
                    sender.send_if_modified(|status| {
                        let current = status.protection(kind);
                        if *current == Protection::Held {
                            *current = Protection::Checking;
                            true
                        } else {
                            false
                        }
                    });
                    let acquired =
                        tokio::time::timeout(std::time::Duration::from_secs(5), api.acquire(kind))
                            .await
                            .unwrap_or_else(|_| {
                                Err(Protection::Unavailable("OS request timed out".into()))
                            });
                    let state = match acquired {
                        Ok(held) => {
                            *lock = Some(held);
                            Protection::Held
                        }
                        Err(state) => state,
                    };
                    let changed = sender.send_if_modified(|status| {
                        let current = status.protection(kind);
                        if *current == state {
                            return false;
                        }
                        *current = state;
                        true
                    });
                    if changed {
                        eprintln!("{}", *sender.borrow());
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
        Self {
            status,
            task: Some(task),
        }
    }

    pub fn status(&self) -> Status {
        if self.task.as_ref().is_some_and(|task| task.is_finished()) {
            let failure = Protection::Unavailable(
                "sleep protection task stopped; restart the service".into(),
            );
            return Status {
                idle_sleep: failure.clone(),
                system_sleep: failure.clone(),
                lid_close: failure,
            };
        }
        self.status.borrow().clone()
    }

    pub async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct FakeOs(Arc<AtomicUsize>);
    struct NativeLock(Arc<AtomicUsize>);
    impl Drop for NativeLock {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Inhibitor for NativeLock {
        fn is_held(&self) -> bool {
            true
        }
    }
    impl PowerApi for FakeOs {
        async fn acquire(&self, _kind: Kind) -> Result<Box<dyn Inhibitor>, Protection> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(NativeLock(self.0.clone())))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn server_holds_sleep_protection_until_it_stops() {
        let locks = Arc::new(AtomicUsize::new(0));
        let server = Guard::with_api(FakeOs(locks.clone()));
        tokio::task::yield_now().await;
        assert_eq!(
            server.status(),
            Status {
                idle_sleep: Protection::Held,
                system_sleep: Protection::Held,
                lid_close: Protection::Held,
            }
        );
        tokio::time::advance(std::time::Duration::from_secs(86_400)).await;
        tokio::task::yield_now().await;
        assert_eq!(locks.load(Ordering::SeqCst), 3);
        server.stop().await;
        assert_eq!(locks.load(Ordering::SeqCst), 0);
    }

    struct RecoveringOs {
        allowed: Arc<AtomicBool>,
        locks: Arc<AtomicUsize>,
    }

    struct RevocableOs(Arc<AtomicUsize>);
    struct RevocableLock {
        generation: Arc<AtomicUsize>,
        acquired: usize,
    }
    impl Inhibitor for RevocableLock {
        fn is_held(&self) -> bool {
            self.generation.load(Ordering::SeqCst) == self.acquired
        }
    }
    impl PowerApi for RevocableOs {
        async fn acquire(&self, _kind: Kind) -> Result<Box<dyn Inhibitor>, Protection> {
            let acquired = self.0.load(Ordering::SeqCst);
            if acquired == 1 {
                return Err(Protection::Unavailable("power service restarting".into()));
            }
            Ok(Box::new(RevocableLock {
                generation: self.0.clone(),
                acquired,
            }))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn lost_native_locks_are_reported_and_reacquired() {
        let generation = Arc::new(AtomicUsize::new(0));
        let server = Guard::with_api(RevocableOs(generation.clone()));
        tokio::task::yield_now().await;
        assert_eq!(server.status().lid_close, Protection::Held);
        generation.store(1, Ordering::SeqCst);
        tokio::time::advance(std::time::Duration::from_secs(30)).await;
        tokio::task::yield_now().await;
        assert!(matches!(
            server.status().lid_close,
            Protection::Unavailable(_)
        ));
        generation.store(2, Ordering::SeqCst);
        tokio::time::advance(std::time::Duration::from_secs(30)).await;
        tokio::task::yield_now().await;
        assert_eq!(server.status().lid_close, Protection::Held);
        server.stop().await;
    }

    struct PartialOs(Arc<AtomicUsize>);
    impl PowerApi for PartialOs {
        async fn acquire(&self, kind: Kind) -> Result<Box<dyn Inhibitor>, Protection> {
            match kind {
                Kind::Idle => FakeOs(self.0.clone()).acquire(kind).await,
                Kind::Sleep => std::future::pending().await,
                Kind::Lid => Err(Protection::Unsupported(
                    "lid close is controlled by the OS".into(),
                )),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn slow_or_unsupported_capabilities_do_not_hide_partial_protection_or_leak_locks() {
        let locks = Arc::new(AtomicUsize::new(0));
        let server = Guard::with_api(PartialOs(locks.clone()));
        tokio::task::yield_now().await;
        assert_eq!(server.status().idle_sleep, Protection::Held);
        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert!(matches!(
            server.status().system_sleep,
            Protection::Unavailable(_)
        ));
        assert!(matches!(
            server.status().lid_close,
            Protection::Unsupported(_)
        ));
        drop(server);
        tokio::task::yield_now().await;
        assert_eq!(locks.load(Ordering::SeqCst), 0);
    }
    impl PowerApi for RecoveringOs {
        async fn acquire(&self, kind: Kind) -> Result<Box<dyn Inhibitor>, Protection> {
            if !self.allowed.load(Ordering::SeqCst) {
                return Err(Protection::Unavailable("permission denied".into()));
            }
            FakeOs(self.locks.clone()).acquire(kind).await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_os_protection_recovers_without_restarting_the_server() {
        let allowed = Arc::new(AtomicBool::new(false));
        let locks = Arc::new(AtomicUsize::new(0));
        let server = Guard::with_api(RecoveringOs {
            allowed: allowed.clone(),
            locks: locks.clone(),
        });
        tokio::task::yield_now().await;
        assert!(matches!(
            server.status().idle_sleep,
            Protection::Unavailable(_)
        ));
        allowed.store(true, Ordering::SeqCst);
        tokio::time::advance(std::time::Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            locks.load(Ordering::SeqCst),
            0,
            "failed requests must back off"
        );
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(server.status().idle_sleep, Protection::Held);
        server.stop().await;
        assert_eq!(locks.load(Ordering::SeqCst), 0);
    }
}
