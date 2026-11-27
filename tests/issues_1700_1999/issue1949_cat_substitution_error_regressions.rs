//! rubash#1949 — the `$(cat …)` shortcut arm crushed the real
//! strerror into a blanket `'...': No such file or directory`: a
//! directory operand, a not-a-directory traversal and an ELOOP
//! symlink all reported a quoted ENOENT. GNU subst.c:7143
//! command_substitute() runs the real `cat`, whose diagnostics are
//! `cat: PATH: <strerror>` (execute_cmd.c / the coreutils-style
//! builtin message). The word-level arm now prints the io::Error it
//! already receives through the same message helper the external cat
//! builtin uses, so the reason text can no longer drift from the
//! read failure that produced it.
//!
//! rubash#2027 update: GNU cat renders the operand through quotef
//! (gnulib quotearg shell-escape style), so an operand holding shell
//! special characters prints single-quoted (`cat: '<(echo hi)': No
//! such file or directory`). On Windows hosts these paths are temp
//! dirs under `C:\...` and the backslash plus drive colon are shell
//! special, so the expected operand here is the quotef rendering; the
//! original Unix-path measurements (`/tmp/rubash-i1949-*`) stay bare
//! exactly as pinned.
//!
//! All expectations measured against GNU bash 5.3.20.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i1949-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path).replace(&case.to_string_lossy().to_string(), "SH"),
            rc,
        )
    }
}

/// The gnulib quotearg shell-escape rule the cat shortcut's diagnostic
/// follows since rubash#2027: bare when every character is shell-safe,
/// otherwise the whole operand wrapped in `'...'`.
fn quotef(path: &str) -> String {
    let safe = |ch: char| {
        ch.is_alphanumeric()
            || matches!(
                ch,
                '_' | '.' | '/' | '-' | '+' | ',' | '@' | '%' | '~' | '#' | '{' | '}'
            )
    };
    if path.chars().all(safe) {
        return path.to_string();
    }
    format!("'{path}'")
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1949_directory_operand_reports_is_a_directory() {
    let s = Scratch::new("isdir");
    let dir = s.dir.join("somedir");
    fs::create_dir_all(&dir).unwrap();
    let script = format!("x=$(cat {})\necho \"rc=$? V=[$x]\"\n", dir.display());
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(
        err,
        format!(
            "cat: {}: Is a directory\n",
            quotef(&dir.display().to_string())
        )
    );
}

#[test]
fn issue1949_traversal_through_a_file_reports_not_a_directory() {
    let s = Scratch::new("notdir");
    let file = s.dir.join("plain.txt");
    fs::write(&file, "data\n").unwrap();
    let bad = format!("{}/child", file.display());
    let script = format!("x=$(cat {bad})\necho \"rc=$? V=[$x]\"\n");
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(err, format!("cat: {}: Not a directory\n", quotef(&bad)));
}

#[cfg(unix)]
#[test]
fn issue1949_symlink_loop_reports_eloop() {
    let s = Scratch::new("eloop");
    let a = s.dir.join("loop_a");
    let b = s.dir.join("loop_b");
    std::os::unix::fs::symlink(&b, &a).unwrap();
    std::os::unix::fs::symlink(&a, &b).unwrap();
    let script = format!("x=$(cat {})\necho \"rc=$? V=[$x]\"\n", a.display());
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(
        err,
        format!(
            "cat: {}: Too many levels of symbolic links\n",
            quotef(&a.display().to_string())
        )
    );
}

#[test]
fn issue1949_plain_enoent_stays_unquoted() {
    let s = Scratch::new("enoent");
    // GNU agreement guard: the ENOENT wording itself was never the
    // bug — the QUOTING was: #1949 removed the blanket `'...': No
    // such file or directory` crush. Since rubash#2027 the operand
    // quoting is GNU cat's own quotef rule (shell-unsafe operands
    // print quoted, plain paths print bare), so on Windows hosts the
    // backslash-bearing temp path is quoted exactly as GNU cat would
    // render it, and the assertion below follows quotef.
    let missing = s.dir.join("missing.txt");
    let script = format!("x=$(cat {})\necho \"rc=$? V=[$x]\"\n", missing.display());
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(
        err,
        format!(
            "cat: {}: No such file or directory\n",
            quotef(&missing.display().to_string())
        )
    );
}
