//! The machine: its clock, its notification center, its browser, its timer.

use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::adapters::presenters::wording;
use crate::entities::task::Task;
use crate::entities::time::Timestamp;
use crate::use_cases::ports::{Clock, Notifier, Pace, Pacer};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp(elapsed.as_secs() as i64)
    }
}

fn quiet(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Opens a URL in the user's browser. Failing to is not worth a crash.
pub fn open_url(url: &str) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = quiet(program).arg(url).spawn();
}

/// Desktop notifications: Notification Center on macOS, `notify-send` elsewhere.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn announce(&self, news: &[&Task]) {
        let (title, body) = match news {
            [] => return,
            [task] => (
                format!("aki · {}", wording::headline(task)),
                format!("{} {}", task.item.short(), task.item.title),
            ),
            many => (
                format!("aki · {} new things to do", many.len()),
                many.iter()
                    .map(|task| task.item.short())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        };
        if cfg!(target_os = "macos") {
            // Passed as arguments, never spliced into the script.
            let script = "on run argv\n\
                          display notification (item 2 of argv) with title (item 1 of argv)\n\
                          end run";
            let _ = quiet("osascript")
                .args(["-e", script, &title, &body])
                .spawn();
        } else {
            let _ = quiet("notify-send").args([&title, &body]).spawn();
        }
    }
}

/// Looks again every `interval`, or sooner when asked to through the channel.
pub struct TimerPacer {
    pub interval: Duration,
    pub requests: Receiver<Pace>,
}

impl Pacer for TimerPacer {
    fn wait(&self) -> Pace {
        match self.requests.recv_timeout(self.interval) {
            Ok(pace) => pace,
            Err(RecvTimeoutError::Timeout) => Pace::Again,
            // Nobody is left to ask for anything: the interface is gone.
            Err(RecvTimeoutError::Disconnected) => Pace::Stop,
        }
    }
}
