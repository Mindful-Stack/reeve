//! The real binary in a real pseudo-terminal: it starts in raw mode, `q` exits 0, and the terminal is handed back (the
//! alternate screen left, the cursor shown, canonical mode restored) on a normal exit and after a panic.
#![cfg(unix)]
mod support;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use support::*;

const ENTER_ALT: &[u8] = b"\x1b[?1049h";
const LEAVE_ALT: &[u8] = b"\x1b[?1049l";

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}
fn position(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
fn last_position(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

struct Run {
    out: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

impl Run {
    /// The pty's line discipline is canonical (cooked): what a shell expects after the TUI.
    fn canonical(&self) -> bool {
        let t = self.master.get_termios().expect("a unix pty has termios");
        t.local_flags
            .contains(nix::sys::termios::LocalFlags::ICANON)
    }
    /// Waits for the child, then for the reader to drain everything it wrote (exit does not imply the last bytes
    /// were read).
    fn finish(&mut self) -> (portable_pty::ExitStatus, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let st = loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                break st;
            }
            assert!(Instant::now() < deadline, "the TUI did not exit");
            std::thread::sleep(Duration::from_millis(20));
        };
        // Join off-thread so a reader that never sees EOF fails the test instead of hanging it.
        let reader = self.reader.take().unwrap();
        let (done, joined) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(reader.join());
        });
        joined
            .recv_timeout(Duration::from_secs(10))
            .expect("the pty reader did not finish after the TUI exited")
            .expect("the pty reader panicked");
        (st, self.out.lock().unwrap().clone())
    }
}

impl Drop for Run {
    /// A failed assertion must not leave the child running.
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn spawn(url: &str, extra_env: &[(&str, &str)]) -> (Run, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!("url = \"{url}\"\ntoken = \"t\"\n"),
    )
    .unwrap();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_reeve"));
    cmd.arg("tui");
    cmd.env("REEVE_CONFIG_DIR", dir.path());
    cmd.env_remove("REEVE_URL");
    cmd.env_remove("REEVE_TOKEN");
    cmd.env_remove("REEVE_TUI_PANIC_AFTER_DRAW");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave); // the child holds the only slave: its exit ends the reader with EOF or EIO
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = pair.master.take_writer().unwrap();
    let out = Arc::new(Mutex::new(Vec::new()));
    let sink = out.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    (
        Run {
            out,
            reader: Some(reader),
            master: pair.master,
            child,
            writer,
        },
        dir,
    )
}

fn wait_for(run: &Run, what: &str, pred: impl Fn(&[u8]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pred(&run.out.lock().unwrap()) {
        assert!(
            Instant::now() < deadline,
            "{what}: output so far {:?}",
            String::from_utf8_lossy(&run.out.lock().unwrap())
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q_exits_0_and_hands_the_terminal_back() {
    let url = serve(Script::new()).await;
    let (mut run, _dir) = spawn(&url, &[]);
    wait_for(&run, "the board", |o| {
        contains(o, ENTER_ALT) && contains(o, b"reeve")
    });
    assert!(!run.canonical(), "raw mode while the TUI runs");
    run.writer.write_all(b"q").unwrap();
    run.writer.flush().unwrap();
    let (st, out) = run.finish();
    assert!(st.success(), "{st:?}");
    assert!(
        position(&out, LEAVE_ALT) > position(&out, ENTER_ALT),
        "left the alternate screen after entering it"
    );
    assert!(
        last_position(&out, SHOW_CURSOR) > position(&out, LEAVE_ALT),
        "the cursor is shown again, after leaving the alternate screen"
    );
    assert!(run.canonical(), "canonical mode is restored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_still_hands_the_terminal_back_before_the_message() {
    let url = serve(Script::new()).await;
    let (mut run, _dir) = spawn(&url, &[("REEVE_TUI_PANIC_AFTER_DRAW", "1")]);
    let (st, out) = run.finish();
    assert!(!st.success());
    assert!(contains(&out, ENTER_ALT), "entered the alternate screen");
    let leave = position(&out, LEAVE_ALT).expect("left the alternate screen");
    let msg = position(&out, b"forced by").expect("the panic message");
    assert!(
        leave < msg,
        "the terminal is restored before the panic message prints"
    );
    assert!(run.canonical(), "canonical mode is restored after a panic");
}
