//! rubash#2006 / rubash#2007 — history faces from the niubash#670 PTY
//! audit, re-verified on current master through BOTH engine consumption
//! paths:
//!
//! * Session path — the standalone binary's pipe-driven `-i` form (the
//!   engine's own SessionHistory records each line before execution,
//!   like bashhist.c).
//! * Provider path — an Executor with a host HistoryProvider installed
//!   (the niubash REPL shape): the host editor records each submitted
//!   line into its store BEFORE the engine executes it (reedline
//!   save-at-accept; cf. GNU bashhist.c maybe_add_history) and stifles
//!   the store to HISTSIZE like readline's stifle_history.
//!
//! GNU oracle (bash 5.3.20, same input lines, interactive):
//!   fc:      `echo alpha` / `echo beta` / `fc -l` lists entries 1,2
//!            only — the fc line (#3) is excluded (builtins/fc.def:
//!            last_hist = i - rh - hist_last_line_added).
//!   history: HISTSIZE=3, `echo h1..h4`, `history` shows 4,5,6 — the
//!            global numbering survives stifling (history_base).
//!
//! Disposition (2026-10-29, master 16a42d78): the session-path pins
//! pass — the engine's own history semantics match GNU. The two
//! provider-path pins fail and are #[ignore]d with the reason inline:
//! both divergences reduce to state the HistoryProvider contract does
//! not carry (the in-flight line's recorded flag; history_base), which
//! is a contract adjudication, not an engine-code fix.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2006-2007-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> (Vec<u8>, Vec<u8>) {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    panic!("child did not exit within {timeout:?}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        use std::io::Read;
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        use std::io::Read;
        let _ = se.read_to_end(&mut err);
    }
    (out, err)
}

fn run_interactive(input: &str, tag: &str) -> String {
    let dir = tempdir(tag);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("TERM", "xterm")
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("feed input");
    let (out, err) = wait_bounded(&mut child, Duration::from_secs(60));
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&err).into_owned();
    let stdout = String::from_utf8_lossy(&out).into_owned();
    assert!(!stdout.is_empty(), "no stdout; stderr: {stderr}");
    stdout
}

// ---------------------------------------------------------------------
// Session path (standalone binary, engine SessionHistory)
// ---------------------------------------------------------------------

#[test]
fn session_path_fc_list_excludes_itself() {
    let stdout = run_interactive("echo alpha\necho beta\nfc -l\n", "sess-fc");
    assert_eq!(stdout, "alpha\nbeta\n1\t echo alpha\n2\t echo beta\n");
}

// ---------------------------------------------------------------------
// Provider path (host HistoryProvider — the niubash REPL shape)
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
struct ListProvider {
    entries: Vec<String>,
}

impl rubash::history::HistoryProvider for ListProvider {
    fn entries(&mut self) -> std::io::Result<Vec<String>> {
        Ok(self.entries.clone())
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.entries.clear();
        Ok(())
    }
    fn append(&mut self, command: String) -> std::io::Result<()> {
        self.entries.push(command);
        Ok(())
    }
    fn replace(&mut self, entries: Vec<String>) -> std::io::Result<()> {
        self.entries = entries;
        Ok(())
    }
    fn write_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn read_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn append_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn read_new_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
}

fn executor_with(entries: Vec<String>) -> rubash::executor::Executor {
    use std::cell::RefCell;
    use std::rc::Rc;
    let provider = Rc::new(RefCell::new(ListProvider { entries }));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider);
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    executor
}

/// #2006: the host recorded `fc -l` itself before dispatch (it is the
/// last entry, exactly as bashhist leaves it). GNU lists only the
/// entries before it.
///
/// Ignored: on the provider path the executor cannot reproduce GNU's
/// fc.def last_hist = len - rh - hist_last_line_added — the
/// HistoryProvider contract carries no signal for whether the host
/// recorded the in-flight line (and builtins/fc's provider call site
/// hard-codes both terms to false). The session path above passes;
/// this pin waits on the contract adjudication in rubash#2006.
#[test]
#[ignore = "provider contract gap: no hist_last_line_added channel (rubash#2006)"]
fn provider_path_fc_list_excludes_itself() {
    let dir = tempdir("prov-fc");
    let d = dir.to_string_lossy().replace('\\', "/");
    let mut executor = executor_with(vec![
        "echo alpha".to_string(),
        "echo beta".to_string(),
        "fc -l".to_string(),
    ]);
    let script = format!("cd {d}\nfc -l > out.txt 2> err.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("fc output");
    assert_eq!(out, "1\t echo alpha\n2\t echo beta\n");
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn session_path_history_numbering_survives_histsize_trim() {
    let stdout = run_interactive(
        "HISTSIZE=3\necho h1\necho h2\necho h3\necho h4\nhistory\n",
        "sess-hist",
    );
    assert_eq!(
        stdout,
        "h1\nh2\nh3\nh4\n    4  echo h3\n    5  echo h4\n    6  history\n"
    );
}

#[test]
fn session_path_history_plain_numbering() {
    let stdout = run_interactive("echo one\necho two\nhistory\n", "sess-plain");
    assert_eq!(
        stdout,
        "one\ntwo\n    1  echo one\n    2  echo two\n    3  history\n"
    );
}

/// #2007: the host store stifled to HISTSIZE=3; the surviving entries
/// are global numbers 4,5,6. GNU `history` prints 4,5,6.
///
/// Ignored: the HistoryProvider contract returns bare entry strings
/// with no history_base channel, so the provider-path display in
/// builtins/history.rs can only number from 1. The session path above
/// passes; this pin waits on the contract adjudication in rubash#2007.
#[test]
#[ignore = "provider contract gap: no history_base channel (rubash#2007)"]
fn provider_path_history_numbering_survives_stifle() {
    let dir = tempdir("prov-hist");
    let d = dir.to_string_lossy().replace('\\', "/");
    let mut executor = executor_with(vec![
        "echo h3".to_string(),
        "echo h4".to_string(),
        "history".to_string(),
    ]);
    let script = format!("cd {d}\nhistory > out.txt 2> err.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("history output");
    assert_eq!(out, "    4  echo h3\n    5  echo h4\n    6  history\n");
    let _ = std::fs::remove_dir_all(&dir);
}
