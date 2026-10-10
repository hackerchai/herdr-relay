//! Opt-in connection phase timing. `HERDR_RELAY_TIMING=1` prints each
//! finished operation to stderr; any other non-empty value is a file that
//! receives one appended line per operation. Timings never include secrets.
use std::{cell::RefCell, future::Future, io::Write, time::Duration};
use tokio::time::Instant;

pub struct Timeline {
    label: &'static str,
    started: Instant,
    last: Instant,
    phases: Vec<(&'static str, Duration)>,
}
impl Timeline {
    fn new(label: &'static str) -> Self {
        let now = Instant::now();
        Self {
            label,
            started: now,
            last: now,
            phases: vec![],
        }
    }
    fn mark(&mut self, phase: &'static str) {
        let now = Instant::now();
        self.phases.push((phase, now - self.last));
        self.last = now;
    }
    pub fn render(&self) -> String {
        let mut line = format!(
            "{} total={}ms",
            self.label,
            (self.last - self.started).as_millis()
        );
        for (phase, elapsed) in &self.phases {
            line.push_str(&format!(" {phase}={}ms", elapsed.as_millis()));
        }
        line
    }
}

tokio::task_local! {
    static CURRENT: RefCell<Timeline>;
}

/// Record the end of a phase in the innermost running timeline, if any.
pub fn mark(phase: &'static str) {
    let _ = CURRENT.try_with(|timeline| timeline.borrow_mut().mark(phase));
}

/// Run an operation with its own timeline and return it.
pub async fn record<T>(label: &'static str, operation: impl Future<Output = T>) -> (T, Timeline) {
    let cell = RefCell::new(Timeline::new(label));
    let (value, cell) = CURRENT
        .scope(cell, async {
            let value = operation.await;
            (
                value,
                CURRENT.with(|t| std::mem::replace(&mut *t.borrow_mut(), Timeline::new(label))),
            )
        })
        .await;
    (value, cell)
}

/// Run an operation and report its timeline when timing is enabled.
pub async fn scope<T>(label: &'static str, operation: impl Future<Output = T>) -> T {
    let Some(destination) = std::env::var_os("HERDR_RELAY_TIMING").filter(|v| !v.is_empty()) else {
        return operation.await;
    };
    let (value, timeline) = record(label, operation).await;
    let line = format!("herdr-relay timing: {}\n", timeline.render());
    if destination == "1" {
        eprint!("{line}");
    } else if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(destination)
    {
        let _ = file.write_all(line.as_bytes());
    }
    value
}
