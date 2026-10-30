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
//! provider-path pins reduced to state the HistoryProvider contract
//! did not carry (the in-flight line's recorded flag; history_base).
//! Both channels are now on the trait with the GNU C initializers as
//! defaults — `history_base` (readline history.c:113; stifling advances
//! it, history.c:389-404,675-698) and `last_line_added` (bashhist.c:174,
//! set by really_add_history:957-964) — and the provider display /
//! `history -d` / fc call sites consume them. Both pins are live.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2006-2007-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// The bounded wait itself is the shared helper in tests/common
/// (rubash#2197): this binary's private copy carried the pre-#2056
/// orphan-pipe hang shape and was deleted in favor of it.

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
    let outcome = wait_bounded(&mut child, Duration::from_secs(60));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    let (out, err) = (outcome.stdout, outcome.stderr);
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

/// The host store fixture. `base` models the stifled store's global
/// numbering (readline history_base): a store trimmed to HISTSIZE keeps
/// the numbers of the entries it dropped (rubash#2007). `last_added`
/// models the save-at-accept contract: the host recorded the submitted
/// line before dispatch (bashhist.c hist_last_line_added; rubash#2006).
#[derive(Debug, Default)]
struct ListProvider {
    entries: Vec<String>,
    base: usize,
    last_added: bool,
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
    fn history_base(&mut self) -> usize {
        self.base
    }
    fn last_line_added(&mut self) -> bool {
        self.last_added
    }
}

fn executor_with(
    entries: Vec<String>,
    base: usize,
    last_added: bool,
) -> rubash::executor::Executor {
    use std::cell::RefCell;
    use std::rc::Rc;
    let provider = Rc::new(RefCell::new(ListProvider {
        entries,
        base,
        last_added,
    }));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider);
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    executor
}

/// #2006: the host recorded `fc -l` itself before dispatch (it is the
/// last entry, exactly as bashhist leaves it). GNU lists only the
/// entries before it: fc.def:328-335 computes
/// `last_hist = i - rh - hist_last_line_added` ("the current command
/// line ... is already part of the history list"; PTY evidence
/// p3-gnu-03 vs p3-hist-03).
///
/// The provider contract now carries `last_line_added` (bashhist.c:174,
/// set by really_add_history:957-964) and the fc provider call site
/// feeds it — with the bridge's rh on (shell.c:1830-1842
/// init_interactive) — instead of hard-wiring both terms to false.
#[test]
fn provider_path_fc_list_excludes_itself() {
    let dir = tempdir("prov-fc");
    let d = dir.to_string_lossy().replace('\\', "/");
    let mut executor = executor_with(
        vec![
            "echo alpha".to_string(),
            "echo beta".to_string(),
            "fc -l".to_string(),
        ],
        1,
        true,
    );
    let script = format!("cd {d}\nfc -l > out.txt 2> err.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("fc output");
    assert_eq!(out, "1\t echo alpha\n2\t echo beta\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #2006 same-root face over a stifled store: `fc -l` numbering is
/// history_base-based (fc.def's emit prints i + history_base, exactly
/// like display_history), and the fc line is excluded even when the
/// requested range reaches the end of the list.
#[test]
fn provider_path_fc_list_range_numbers_from_base() {
    let dir = tempdir("prov-fc-range");
    let d = dir.to_string_lossy().replace('\\', "/");
    let mut executor = executor_with(
        vec![
            "echo a".to_string(),
            "echo b".to_string(),
            "echo c".to_string(),
            "fc -l".to_string(),
        ],
        5,
        true,
    );
    let script = format!("cd {d}\nfc -l > out.txt 2> err.txt\nfc -l 6 8 > range.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("fc output");
    assert_eq!(out, "5\t echo a\n6\t echo b\n7\t echo c\n");
    let range = std::fs::read_to_string(dir.join("range.txt")).expect("fc range output");
    assert_eq!(range, "6\t echo b\n7\t echo c\n");
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
/// are global numbers 4,5,6. GNU `history` prints 4,5,6 (bash 5.3.20
/// PTY evidence p3-gnu-04 vs p3-hist-04: 399/400/401, never 1/2/3).
///
/// The provider contract now carries `history_base` (readline
/// history.c:113; a stifled store advances it like stifle_history /
/// the stifled add_history, history.c:389-404,675-698) and the
/// provider-path display numbers from it instead of a hard-wired 1.
#[test]
fn provider_path_history_numbering_survives_stifle() {
    let dir = tempdir("prov-hist");
    let d = dir.to_string_lossy().replace('\\', "/");
    let mut executor = executor_with(
        vec![
            "echo h3".to_string(),
            "echo h4".to_string(),
            "history".to_string(),
        ],
        4,
        false,
    );
    let script = format!("cd {d}\nhistory > out.txt 2> err.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("history output");
    assert_eq!(out, "    4  echo h3\n    5  echo h4\n    6  history\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #2007 same-root face: positive `history -d` operands are DISPLAY
/// numbers, offset by history_base (builtins/history.def:243-260
/// `opt - history_base`; the session path mirrors it with shell.base).
/// With the store's base at 4, `-d 5` removes entries[1]; the survivor
/// keeps the list number its position now carries (5), exactly like the
/// session path — GNU numbers are base+index, not stable ids.
#[test]
fn provider_path_history_delete_offset_uses_base() {
    let dir = tempdir("prov-hist-d");
    let d = dir.to_string_lossy().replace('\\', "/");
    let mut executor = executor_with(
        vec![
            "echo a".to_string(),
            "echo b".to_string(),
            "echo c".to_string(),
        ],
        4,
        false,
    );
    let script = format!("cd {d}\nhistory -d 5\nhistory > out.txt 2> err.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("history output");
    assert_eq!(out, "    4  echo a\n    5  echo c\n");
    let _ = std::fs::remove_dir_all(&dir);
}
