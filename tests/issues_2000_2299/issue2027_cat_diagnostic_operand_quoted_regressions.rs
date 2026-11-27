//! rubash#2027 — the `$(cat …)` shortcut arm printed its operand bare
//! in open/read diagnostics: rubash said
//! `cat: <(echo hi): Invalid argument` where GNU bash 5.3.20 says
//! `cat: '<(echo hi)': No such file or directory`. Two GNU faces:
//!
//! * coreutils cat renders the operand through quotef (gnulib
//!   quotearg shell-escape style), so an operand holding shell
//!   special characters prints single-quoted while a plain path
//!   (`/tmp`, `./missing`, `a+b`) prints bare;
//! * the failure reason for `<(echo hi)` is ENOENT — no file of that
//!   literal name exists — but Windows open(2) rejects the reserved
//!   `<`/`>` bytes with ERROR_INVALID_NAME before a lookup can fail,
//!   so the EINVAL/invalid-name text leaked. The invalid-name to
//!   ENOENT inference `posix_errors::path_error` already applies to
//!   wildcard-bearing names extends to the reserved `<`/`>` class.
//!
//! All expectations measured against GNU bash 5.3.20 (WSL, coreutils
//! cat 9.x): `cat: '<(echo hi)': No such file or directory`,
//! `cat: 'a b': No such file or directory`, `cat: a+b: No such file
//! or directory`.

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
            std::env::temp_dir().join(format!("rubash-i2027-{tag}-{}-{nanos}", std::process::id()));
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
        // The host environment is inherited (issue1371 precedent): the
        // `$(cat …)` shortcut admits only when a `cat` resolves on PATH
        // (`emulated_external_stands_down`), and the emulated external
        // cat is the diagnostic face under test. An env_clear'd PATH
        // strands the shape at `cat: command not found` (rc 127), which
        // is GNU-correct for a cat-less system but not this file's
        // subject.
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
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

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue2027_procsub_literal_operand_is_quoted_enoent() {
    let s = Scratch::new("procsub");
    // GNU agreement: the quoted string keeps `<(...)` literal, the open
    // fails, and cat names the operand quoted with the ENOENT text.
    let (out, err, rc) = s.run("x=$(cat \"<(echo hi)\")\necho \"rc=$? V=[$x]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(
        err,
        "cat: '<(echo hi)': No such file or directory\n".to_string()
    );
}

#[test]
fn issue2027_space_operand_is_quoted() {
    let s = Scratch::new("space");
    // GNU: `cat 'a b'` on a missing name prints `'a b'` — the space is
    // shell-special, the whole operand is wrapped.
    let (out, err, rc) = s.run("x=$(cat 'a b')\necho \"rc=$? V=[$x]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(err, "cat: 'a b': No such file or directory\n".to_string());
}

#[test]
fn issue2027_plain_path_stays_bare() {
    let s = Scratch::new("plain");
    // GNU agreement guard: quotef leaves shell-safe operands bare
    // (`cat: a+b: No such file or directory`) — the quoting is GNU
    // cat's own rule, not a blanket wrapper.
    let (out, err, rc) = s.run("x=$(cat a+b)\necho \"rc=$? V=[$x]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(err, "cat: a+b: No such file or directory\n".to_string());
}

#[test]
fn issue2027_success_operand_never_prints() {
    let s = Scratch::new("success");
    // Guard: the quoting lives only in the failure diagnostic — a read
    // that succeeds is content, verbatim.
    fs::write(s.dir.join("body.txt"), "payload\n").unwrap();
    let (out, err, rc) = s.run("x=$(cat body.txt)\nprintf 'rc=%s V=[%s]\\n' \"$?\" \"$x\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0 V=[payload]\n");
    assert_eq!(err, "");
}

#[cfg(unix)]
#[test]
fn issue2027_directory_reason_text_keeps_real_errno() {
    // Unix only: Linux open(2) on a directory read fails with EISDIR
    // ("Is a directory", the GNU text). A Windows fs::read on a
    // directory fails with ERROR_ACCESS_DENIED ("Permission denied")
    // before any directory semantics apply, so the byte pin below has
    // no Windows twin; the #1949 file owns the real-errno face.
    let s = Scratch::new("isdir");
    // rubash#1949 guard under the quotef rendering: the reason stays the
    // real errno (EISDIR), only the operand quoting changed. The temp
    // path carries shell-unsafe bytes on Windows hosts (`C:\...`), so
    // the expected operand is the quoted form there and bare on Unix.
    let dir = s.dir.join("somedir");
    fs::create_dir_all(&dir).unwrap();
    // Forward slashes: inside the script a bare `C:\Users` backslash is
    // an escape byte the word reader consumes (`\U` -> `U`), which is a
    // different word than the operand meant. Windows file APIs accept
    // `/` separators, and the drive colon keeps the operand outside the
    // shell-safe set, so the expected diagnostic stays quoted.
    let shown = dir.display().to_string().replace('\\', "/");
    let safe = shown.chars().all(|ch| {
        ch.is_alphanumeric()
            || matches!(
                ch,
                '_' | '.' | '/' | '-' | '+' | ',' | '@' | '%' | '~' | '#' | '{' | '}'
            )
    });
    let operand = if safe {
        shown.clone()
    } else {
        format!("'{shown}'")
    };
    let script = format!("x=$(cat {shown})\necho \"rc=$? V=[$x]\"\n");
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1 V=[]\n");
    assert_eq!(err, format!("cat: {operand}: Is a directory\n"));
}
