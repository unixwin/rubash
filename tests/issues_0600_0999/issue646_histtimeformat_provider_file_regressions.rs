//! niubash#646 — HISTTIMEFORMAT on the provider file faces (`history
//! -w` / `history -a` through a host HistoryProvider).
//!
//! GNU shape being mirrored (bash 5.3.20): sv_histtimefmt arms the file
//! faces on the variable's bare existence —
//! `history_write_timestamps = (v != 0)`, variables.c:6168-6177 — and
//! histfile.c then writes each stamp-bearing entry as TWO lines,
//! `#<epoch>` before the command (history_write_slow, histfile.c:738;
//! history_append, histfile.c:815), while HISTFILESIZE keeps counting
//! command lines (histfile.c:633).
//!
//! Two contracts pinned here:
//! 1. SAFETY — a provider that has not opted into the stamp channel
//!    (the trait defaults; the niubash-master reedline shape, whose
//!    pure-line file format reads `#<epoch>` lines back as entries) is
//!    never handed stamp lines: the engine calls the plain
//!    write/append, and the file stays byte-identical to pre-#646.
//! 2. THE CHANNEL — a provider that implements
//!    `entry_timestamps` + the timestamped write/append receives the
//!    parallel stamp vec whenever HISTTIMEFORMAT exists (set-but-empty
//!    included) and can emit the GNU `#<epoch>` file shape.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The stamp-blind host (trait defaults everywhere): its file format is
/// one encoded line per entry, exactly like reedline's — a `#<epoch>`
/// line in that file would be swallowed as an entry on the next load
/// (the niubash#646 file-face blocker). The engine MUST route its
/// write/append through the plain trait methods.
#[derive(Debug, Default)]
struct PlainProvider {
    entries: Vec<String>,
    written: Vec<String>,
    appended: Vec<String>,
}

impl rubash::history::HistoryProvider for PlainProvider {
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
    fn write_history(&mut self, path: &str) -> std::io::Result<()> {
        self.written.push(path.to_string());
        std::fs::write(path, self.entries.join("\n") + "\n")
    }
    fn read_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn append_history(&mut self, path: &str) -> std::io::Result<()> {
        self.appended.push(path.to_string());
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for entry in &self.entries {
            writeln!(file, "{entry}")?;
        }
        Ok(())
    }
    fn read_new_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
}

/// The niubash#622 host shape: the host stamps at its own add and
/// reports the parallel channel; the timestamped file faces emit the
/// GNU two-line shape.
#[derive(Debug, Default)]
struct StampAwareProvider {
    entries: Vec<String>,
    stamps: Vec<String>,
    timestamped_writes: usize,
    timestamped_appends: usize,
    plain_writes: usize,
    plain_appends: usize,
    last_write_stamps: Vec<String>,
    last_append_stamps: Vec<String>,
}

impl StampAwareProvider {
    fn with_entries(entries: &[&str]) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            entries: entries.iter().map(|e| e.to_string()).collect(),
            stamps: entries.iter().map(|_| format!("#{now}")).collect(),
            ..Default::default()
        }
    }
}

impl rubash::history::HistoryProvider for StampAwareProvider {
    fn entries(&mut self) -> std::io::Result<Vec<String>> {
        Ok(self.entries.clone())
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.entries.clear();
        self.stamps.clear();
        Ok(())
    }
    fn append(&mut self, command: String) -> std::io::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.entries.push(command);
        self.stamps.push(format!("#{now}"));
        Ok(())
    }
    fn replace(&mut self, entries: Vec<String>) -> std::io::Result<()> {
        // Position-preserving rewrites (history -d, the veto) keep the
        // aligned stamps; length changes resync with read-time stamps,
        // the histfile.c add_history shape.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.stamps.resize(entries.len(), format!("#{now}"));
        self.stamps.truncate(entries.len());
        self.entries = entries;
        Ok(())
    }
    fn entry_timestamps(&mut self) -> Vec<String> {
        self.stamps.clone()
    }
    fn write_history(&mut self, path: &str) -> std::io::Result<()> {
        self.plain_writes += 1;
        std::fs::write(path, self.entries.join("\n") + "\n")
    }
    fn write_history_timestamped(
        &mut self,
        path: &str,
        timestamps: &[String],
    ) -> std::io::Result<()> {
        self.timestamped_writes += 1;
        self.last_write_stamps = timestamps.to_vec();
        let mut out = String::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(stamp) = timestamps.get(index).filter(|stamp| !stamp.is_empty()) {
                out.push_str(stamp);
                out.push('\n');
            }
            out.push_str(entry);
            out.push('\n');
        }
        std::fs::write(path, out)
    }
    fn read_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn append_history(&mut self, path: &str) -> std::io::Result<()> {
        self.plain_appends += 1;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for entry in &self.entries {
            writeln!(file, "{entry}")?;
        }
        Ok(())
    }
    fn append_history_timestamped(
        &mut self,
        path: &str,
        timestamps: &[String],
    ) -> std::io::Result<()> {
        self.timestamped_appends += 1;
        self.last_append_stamps = timestamps.to_vec();
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(stamp) = timestamps.get(index).filter(|stamp| !stamp.is_empty()) {
                writeln!(file, "{stamp}")?;
            }
            writeln!(file, "{entry}")?;
        }
        Ok(())
    }
    fn read_new_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash646-file-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &Path, executor: &mut rubash::executor::Executor, script: &str) -> String {
    let code = rubash::script_driver::run_source(executor, script, true);
    assert_eq!(code, 0, "script status");
    let out = std::fs::read_to_string(dir.join("out.txt")).expect("history output");
    out
}

fn has_stamp_line(content: &str) -> bool {
    content.lines().any(|line| {
        line.starts_with('#') && line[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
    })
}

/// SAFETY: the stamp-blind provider's `history -w` under HISTTIMEFORMAT
/// goes through the PLAIN write — its one-line-per-entry file never
/// grows `#<epoch>` side lines a reader would swallow as entries.
#[test]
fn stamp_blind_write_stays_plain_under_histtimeformat() {
    let dir = tempdir("plain-write");
    let provider = Rc::new(RefCell::new(PlainProvider {
        entries: vec!["echo one".to_string(), "echo two".to_string()],
        ..Default::default()
    }));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT='%Y '\nhistory -w {d}/hist.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    assert_eq!(provider.borrow().written.len(), 1, "plain write called");
    let content = std::fs::read_to_string(dir.join("hist.txt")).expect("written file");
    assert_eq!(content, "echo one\necho two\n");
    assert!(!has_stamp_line(&content), "no stamp lines: {content}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE CHANNEL: a stamp-aware host receives the parallel stamps on -w
/// whenever HISTTIMEFORMAT exists and emits the GNU `#<epoch>` file
/// shape — the file metadata lines GNU bash writes (histfile.c:738).
#[test]
fn stamp_aware_write_emits_epoch_metadata_lines() {
    let dir = tempdir("aware-write");
    let provider = Rc::new(RefCell::new(StampAwareProvider::with_entries(&[
        "echo one", "echo two",
    ])));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT='%Y-%m-%d %H:%M:%S '\nhistory -w {d}/hist.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let host = provider.borrow();
    assert_eq!(host.timestamped_writes, 1, "timestamped write called");
    assert_eq!(host.plain_writes, 0);
    assert_eq!(host.last_write_stamps.len(), host.entries.len(), "parallel");
    assert!(
        host.last_write_stamps
            .iter()
            .all(|stamp| stamp.starts_with('#')
                && stamp[1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())),
        "stamps are #<epoch>: {:?}",
        host.last_write_stamps
    );
    drop(host);
    let content = std::fs::read_to_string(dir.join("hist.txt")).expect("written file");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "two stamped entries = four lines: {content}"
    );
    assert!(has_stamp_line(lines[0]), "entry 1 stamp line: {content}");
    assert_eq!(lines[1], "echo one");
    assert!(has_stamp_line(lines[2]), "entry 2 stamp line: {content}");
    assert_eq!(lines[3], "echo two");
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE CHANNEL, append face: -a carries the stamps for the pending tail
/// (histfile.c:815 history_append — a stamped entry is two file lines).
#[test]
fn stamp_aware_append_receives_parallel_stamps() {
    let dir = tempdir("aware-append");
    let provider = Rc::new(RefCell::new(StampAwareProvider::with_entries(&[
        "echo one",
    ])));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!("cd {d}\nHISTTIMEFORMAT='%X '\nhistory -a {d}/hist.txt\n");
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0);
    let host = provider.borrow();
    assert_eq!(host.timestamped_appends, 1);
    assert_eq!(host.last_append_stamps.len(), 1, "parallel to the list");
    assert!(host.last_append_stamps[0].starts_with('#'));
    drop(host);
    let content = std::fs::read_to_string(dir.join("hist.txt")).expect("appended file");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "stamp line then command line: {content}");
    assert!(has_stamp_line(&content), "{content}");
    assert_eq!(lines[1], "echo one");
    let _ = std::fs::remove_dir_all(&dir);
}

/// sv_histtimefmt arms the file faces on the variable's bare existence
/// (set-but-empty included, variables.c:6177) while the DISPLAY face
/// stays plain (`histtimefmt && *histtimefmt`, history.def:406) — both
/// faces in one session.
#[test]
fn empty_format_arms_the_write_but_not_the_display() {
    let dir = tempdir("empty");
    let provider = Rc::new(RefCell::new(StampAwareProvider::with_entries(&[
        "echo one",
    ])));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!(
        "cd {d}\nHISTTIMEFORMAT=''\nhistory -w {d}/hist.txt\nhistory > out.txt 2> err.txt\n"
    );
    let out = run_script(&dir, &mut executor, &script);
    let host = provider.borrow();
    assert_eq!(host.timestamped_writes, 1, "write face armed");
    drop(host);
    assert_eq!(out, "    1  echo one\n", "display face plain");
    let content = std::fs::read_to_string(dir.join("hist.txt")).expect("written file");
    assert!(has_stamp_line(&content), "{content}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// HISTTIMEFORMAT unset: plain calls on both faces, byte-identical to
/// the pre-#646 path (history_write_timestamps = 0, histfile.c:121).
#[test]
fn unset_format_keeps_plain_calls() {
    let dir = tempdir("unset");
    let provider = Rc::new(RefCell::new(StampAwareProvider::with_entries(&[
        "echo one",
    ])));
    let mut executor = rubash::executor::Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    let d = dir.to_string_lossy().replace('\\', "/");
    let script = format!(
        "cd {d}\nhistory -w {d}/w.txt\nhistory -a {d}/a.txt\nhistory > out.txt 2> err.txt\n"
    );
    let out = run_script(&dir, &mut executor, &script);
    let host = provider.borrow();
    assert_eq!(host.plain_writes, 1);
    assert_eq!(host.plain_appends, 1);
    assert_eq!(host.timestamped_writes, 0);
    assert_eq!(host.timestamped_appends, 0);
    drop(host);
    assert_eq!(out, "    1  echo one\n");
    let _ = std::fs::remove_dir_all(&dir);
}
