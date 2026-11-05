//! The real terminal: raw mode and the alternate screen behind a guard, a panic hook that restores before the message
//! prints, the signal watcher (SIGTERM, SIGHUP, SIGINT, SIGQUIT), the production input stream (keys and resizes), and
//! the browser and clipboard.
use crate::tui::Input;
use crate::tui::runtime::Io;
use crossterm::{
    cursor::{Hide, Show},
    event::EventStream,
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::{StreamExt, stream::BoxStream};
use std::io::{Write, stdout};
use std::sync::{
    Once,
    atomic::{AtomicBool, Ordering},
};

static ENTERED: AtomicBool = AtomicBool::new(false);
static HOOK: Once = Once::new();

/// Restores the terminal when dropped (normal exit and `?` error paths).
pub struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// Raw mode and the alternate screen; installs (once) a panic hook that restores before the message prints.
pub fn enter() -> std::io::Result<Guard> {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            on_panic(restore(), || prev(info), || std::process::exit(101));
        }));
    });
    enable_raw_mode()?;
    ENTERED.store(true, Ordering::SeqCst);
    if let Err(e) = execute!(stdout(), EnterAlternateScreen, Hide) {
        restore();
        return Err(e);
    }
    Ok(Guard)
}

/// Leaves the alternate screen and raw mode; a no-op unless `enter` succeeded and nothing has restored since. Returns
/// whether this call restored.
pub fn restore() -> bool {
    restore_with(&ENTERED, || {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, Show);
    })
}

fn restore_with(entered: &AtomicBool, undo: impl FnOnce()) -> bool {
    let ran = entered.swap(false, Ordering::SeqCst);
    if ran {
        undo();
    }
    ran
}

/// A panic that restored the terminal ends the process once its message has printed: a panic in a spawned task would
/// otherwise leave the driver drawing over the restored shell. A panic with the terminal not entered (or already
/// restored) is an ordinary panic.
fn on_panic(restored: bool, report: impl FnOnce(), exit: impl FnOnce()) {
    report();
    if restored {
        exit();
    }
}

/// Registers SIGTERM, SIGHUP, SIGINT and SIGQUIT now (call it before `enter`, so a signal between the two is not lost)
/// and spawns the task that, on the first of them, restores the terminal and exits 0. Independent of the key stream:
/// crossterm's `EventStream` never ends and may not wake on a hangup. In raw mode Ctrl-C is a key (the core's quit);
/// a SIGINT here is one sent by `kill`. Needs a tokio runtime.
#[cfg(unix)]
pub fn watch_signals() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut signals: Vec<_> = [
        SignalKind::terminate(),
        SignalKind::hangup(),
        SignalKind::interrupt(),
        SignalKind::quit(),
    ]
    .into_iter()
    .filter_map(|k| signal(k).ok())
    .collect();
    tokio::spawn(async move {
        std::future::poll_fn(|cx| {
            // Short-circuiting is fine: when one is ready the task is done; otherwise every stream was polled and
            // holds the waker.
            let fired = signals.iter_mut().any(|s| s.poll_recv(cx).is_ready());
            if fired {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await;
        restore();
        std::process::exit(0);
    });
}

#[cfg(not(unix))]
pub fn watch_signals() {}

/// Keys and resizes. crossterm's `EventStream` does not end in practice (not on stdin EOF either: it reads the tty), so
/// the stream carries no end marker; an injected stream that ends exits the driver cleanly.
pub fn input() -> BoxStream<'static, Input> {
    EventStream::new()
        .map(|r| match r {
            Ok(e) => Input::Event(e),
            Err(e) => Input::Error(e),
        })
        .boxed()
}

/// The browser (`xdg-open`, or `open` on macOS) and the clipboard (OSC 52).
pub struct TerminalIo;
impl Io for TerminalIo {
    fn open(&self, url: &str) -> Result<(), String> {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let status = std::process::Command::new(opener)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("{opener}: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{opener} exited with {status}"))
        }
    }
    /// OSC 52: the terminal (and tmux with `set-clipboard on`) puts the text on the clipboard.
    fn copy(&self, text: &str) -> Result<(), String> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text);
        let mut out = stdout();
        write!(out, "\x1b]52;c;{b64}\x07")
            .and_then(|_| out.flush())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn restore_runs_once_however_often_it_is_called() {
        let entered = AtomicBool::new(true);
        let runs = AtomicUsize::new(0);
        for _ in 0..3 {
            restore_with(&entered, || {
                runs.fetch_add(1, Ordering::SeqCst);
            });
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let never = AtomicBool::new(false);
        assert!(!restore_with(&never, || {
            panic!("restoring a terminal that was never entered")
        }));
    }

    #[test]
    fn restore_reports_whether_it_ran() {
        let entered = AtomicBool::new(true);
        assert!(restore_with(&entered, || {}));
        assert!(!restore_with(&entered, || {}));
    }

    #[test]
    fn a_panic_that_restored_reports_then_exits() {
        let log = std::cell::RefCell::new(Vec::new());
        on_panic(
            true,
            || log.borrow_mut().push("report"),
            || log.borrow_mut().push("exit"),
        );
        assert_eq!(*log.borrow(), ["report", "exit"]);
    }

    #[test]
    fn a_panic_outside_the_tui_only_reports() {
        let log = std::cell::RefCell::new(Vec::new());
        on_panic(
            false,
            || log.borrow_mut().push("report"),
            || log.borrow_mut().push("exit"),
        );
        assert_eq!(*log.borrow(), ["report"]);
    }
}
