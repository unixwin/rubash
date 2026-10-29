//! niubash#646 — HISTTIMEFORMAT on the provider display face (the
//! niubash REPL shape: a host HistoryProvider owns the list; reedline
//! records each accepted line before the engine dispatches it).
//!
//! GNU shape being mirrored (bash 5.3.20): every listed entry carries a
//! stamp, because every add_history stamps its entry at the add moment
//! (readline/history.c hist_inittime, history.c:322-333), and
//! display_history renders it through HISTTIMEFORMAT — set AND
//! non-empty (`histtimefmt && *histtimefmt`, builtins/history.def:400,
//! 406) — via histtime (history.def:351-368). On the provider contract
//! the engine's observation of the add is the interactive dispatch
//! (`pre_process_interactive_line`), so the dispatch-time stamp IS the
//! add time; entries the engine never observed (loaded before the first
//! dispatch) take their read time, the histfile.c add_history shape —
//! a stamping shell never renders the stamp-less `??`.
//!
//! Oracle: GNU bash 5.3.20 interactive, TZ=UTC,
//! HISTTIMEFORMAT='%Y-%m-%d %H:%M:%S ' — `history` shows the formatted
//! stamp between the listing's two spaces and the entry text; the epoch
//! form `%s` matches the entry's add second.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The host store fixture (the niubash#2006 ListProvider shape): the
/// host records accepted lines into `entries` before dispatch. This
/// double predates the niubash#622 stamp channel — its
/// `entry_timestamps` is the trait default (empty) — so the stamps come
/// from the engine's dispatch-time ledger, which is exactly the
/// niubash-master end-to-end shape.
#[derive(Debug)]
struct ListProvider {
    entries: Vec<String>,
    base: usize,
}

impl Default for ListProvider {
    fn default() -> Self {
        // readline/history.c:113 `int history_base = 1` — the unstifled
        // GNU initial value (the trait default; the fixture states it
        // explicitly so the double cannot silently list from 0).
        Self {
            entries: Vec::new(),
            base: 1,
        }
    }
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
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash646-display-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn executor_with(provider: Rc<RefCell<ListProvider>>) -> rubash::executor::Executor {
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider);
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    executor
}

/// The host accepted `line` (recorded it into the store) and handed it to
/// the engine — the niubash REPL's per-line funnel, up to the dispatch
/// observation the stamp ledger reconciles at.
fn accept_line(
    executor: &mut rubash::executor::Executor,
    provider: &Rc<RefCell<ListProvider>>,
    line: &str,
) {
    provider.borrow_mut().entries.push(line.to_string());
    rubash::script_driver::pre_process_interactive_line(executor, line);
}

fn run_script(dir: &Path, executor: &mut rubash::executor::Executor, script: &str) -> String {
    let code = rubash::script_driver::run_source(executor, script, true);
    assert_eq!(code, 0, "script status");
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("history output");
    let _ = std::fs::remove_dir_all(dir);
    out
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// HISTTIMEFORMAT set non-empty renders through the constant format for
/// every entry — including ones the engine never observed (the fixture's
/// pre-populated list), which take their read-time stamp like a
/// histfile.c load: a stamping shell never renders `??`.
#[test]
fn provider_display_renders_format_for_every_entry() {
    let dir = tempdir("render");
    let provider = Rc::new(RefCell::new(ListProvider {
        entries: vec!["echo one".to_string(), "echo two".to_string()],
        base: 1,
    }));
    let mut executor = executor_with(provider.clone());
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT='T '\nhistory > out.txt 2> err.txt\n");
    let out = run_script(&dir, &mut executor, &script);
    assert_eq!(out, "    1  T echo one\n    2  T echo two\n");
}

/// The dispatch-time stamp is the add time: the displayed `%s` epoch of
/// an accepted line lands inside the observation window around its
/// dispatch (builtins/history.def histtime renders the entry's stamp;
/// the stamp is the engine's dispatch observation, the provider
/// contract's add moment).
#[test]
fn accept_time_stamp_lands_in_the_dispatch_window() {
    let dir = tempdir("window");
    let provider = Rc::new(RefCell::new(ListProvider::default()));
    let mut executor = executor_with(provider.clone());
    provider.borrow_mut().entries.push("echo first".to_string());
    let before = now_epoch();
    rubash::script_driver::pre_process_interactive_line(&mut executor, "echo first");
    let after = now_epoch();
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT='%s '\nhistory > out.txt 2> err.txt\n");
    let out = run_script(&dir, &mut executor, &script);
    // Line shape `%5d  <stamp><entry>`: the stamp is the second
    // whitespace token (builtins/history.def:409 `%5d%c %s%s`).
    let epoch: i64 = out
        .trim_start()
        .split_whitespace()
        .nth(1)
        .and_then(|token| token.parse().ok())
        .unwrap_or(-1);
    assert!(
        epoch >= before && epoch <= after,
        "displayed stamp {epoch} outside dispatch window [{before}, {after}]; output: {out}"
    );
}

/// `history -d` moves the survivors with their stamps attached
/// (remove_history_range carries the HIST_ENTRY timestamp fields):
/// deleting the middle entry leaves first and third rendering their own
/// accept-window epochs, not each other's.
#[test]
fn delete_keeps_survivor_stamps_attached() {
    let dir = tempdir("delete");
    let provider = Rc::new(RefCell::new(ListProvider::default()));
    let mut executor = executor_with(provider.clone());
    let mut windows = Vec::new();
    for line in ["echo one", "echo two", "echo three"] {
        provider.borrow_mut().entries.push(line.to_string());
        let before = now_epoch();
        rubash::script_driver::pre_process_interactive_line(&mut executor, line);
        windows.push((before, now_epoch()));
    }
    let d = dir.to_string_lossy().replace('\\', "/");
    let script =
        format!("cd {d}\nhistory -d 2\nHISTTIMEFORMAT='%s '\nhistory > out.txt 2> err.txt\n");
    let out = run_script(&dir, &mut executor, &script);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "two survivors after -d 2: {out}");
    assert!(lines[0].ends_with("echo one"), "{out}");
    assert!(lines[1].ends_with("echo three"), "{out}");
    // Survivor windows: "echo one" is windows[0], "echo three" (now the
    // second listed line) is windows[2] — the deleted middle entry's
    // window is gone with it.
    for (line, (before, after)) in lines.iter().zip([(windows[0]), (windows[2])]) {
        let epoch: i64 = line
            .trim_start()
            .split_whitespace()
            .nth(1)
            .and_then(|token| token.parse().ok())
            .unwrap_or(-1);
        assert!(
            epoch >= before && epoch <= after,
            "survivor stamp {epoch} outside its window [{before}, {after}]; output: {out}"
        );
    }
}

/// The recording veto (set +o history:
/// script_driver::interactive_history_veto_recording) drops the entry
/// and its stamp together — GNU never reaches add_history for a line
/// recorded outside remember_on_history, so there is no orphan stamp
/// left to render.
#[test]
fn recording_veto_drops_the_stamp_with_the_entry() {
    let dir = tempdir("veto");
    let provider = Rc::new(RefCell::new(ListProvider::default()));
    let mut executor = executor_with(provider.clone());
    accept_line(&mut executor, &provider, "echo kept");
    provider
        .borrow_mut()
        .entries
        .push("echo vetoed".to_string());
    rubash::script_driver::pre_process_interactive_line(&mut executor, "echo vetoed");
    rubash::script_driver::interactive_history_veto_recording(&mut executor);
    assert_eq!(provider.borrow_mut().entries.len(), 1);
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT='T '\nhistory > out.txt 2> err.txt\n");
    let out = run_script(&dir, &mut executor, &script);
    assert_eq!(out, "    1  T echo kept\n");
}

/// Set-but-empty HISTTIMEFORMAT does not render stamps on the display
/// face (`histtimefmt && *histtimefmt`, history.def:406) — the same
/// split from the file face (variables.c sv_histtimefmt arms the WRITE
/// on the variable's bare existence; pinned in the file regressions).
#[test]
fn empty_histtimeformat_lists_plain() {
    let dir = tempdir("empty");
    let provider = Rc::new(RefCell::new(ListProvider {
        entries: vec!["echo one".to_string()],
        base: 1,
    }));
    let mut executor = executor_with(provider);
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT=''\nhistory > out.txt 2> err.txt\n");
    let out = run_script(&dir, &mut executor, &script);
    assert_eq!(out, "    1  echo one\n");
}

/// `history -s` pushes through add_history: the pushed entry takes the
/// add-time stamp like any other (history.def -s → push_history →
/// check_add_history), so it renders in the listing like the rest.
#[test]
fn save_push_takes_an_add_time_stamp() {
    let dir = tempdir("save");
    let provider = Rc::new(RefCell::new(ListProvider {
        entries: vec!["echo one".to_string()],
        base: 1,
    }));
    let mut executor = executor_with(provider);
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!(
        "cd {d}\nHISTTIMEFORMAT='%s '\nhistory -s echo pushed\nhistory > out.txt 2> err.txt\n"
    );
    let out = run_script(&dir, &mut executor, &script);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[1].ends_with("echo pushed"), "{out}");
    let epoch: i64 = lines[1]
        .trim_start()
        .split_whitespace()
        .nth(1)
        .and_then(|token| token.parse().ok())
        .unwrap_or(-1);
    let now = now_epoch();
    assert!(
        epoch >= 0 && epoch <= now,
        "pushed entry stamp {epoch} not a real add time (now {now}): {out}"
    );
}
