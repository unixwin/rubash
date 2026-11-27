//! W4 backfill slice 1 — redirection / fd faces.
//!
//! * rust#113 — process substitution works as a redirection source
//!   (`cat <(echo hi)`).
//! * rust#137 — a failed redirection on a brace group and on a subshell
//!   reports the GNU diagnostic once and sets rc 1 for both.
//! * rust#216 — a heredoc body redirected to /dev/stderr with `>`
//!   reaches stderr, not stdout.
//! * rust#263 — `>&5-` (dup-then-close move form) is parsed as a
//!   redirection operator, not a filename; the per-command close does
//!   not destroy the shell's fd 5.
//! * rust#264 — `<>` creates the target file on open (`: <> newfile`).
//! * rust#266 — redirecting from an unopened fd reports
//!   "N: Bad file descriptor" and rc 1.
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

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
            std::env::temp_dir().join(format!("rubash-bfw4-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        let norm = |b: &[u8]| String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S");
        (
            norm(&out.stdout),
            norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        self.finish(out)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue113_process_substitution_as_input() {
    let s = Scratch::new("i113");
    let (out, err, rc) = s.run("cat <(echo hi)\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\n");
}

#[test]
fn issue137_failed_redirection_on_group_and_subshell() {
    let s = Scratch::new("i137");
    let (out, err, rc) = s.run(
        "{ echo hi; } < /nonexistent-xyz/f\necho rc=$?\n( echo hi ) < /nonexistent-xyz/f\necho rc=$?\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1\nrc=1\n");
    assert_eq!(
        err,
        "$S/case.sh: line 1: /nonexistent-xyz/f: No such file or directory\n$S/case.sh: line 3: /nonexistent-xyz/f: No such file or directory\n"
    );
}

#[test]
fn issue216_heredoc_redirected_to_dev_stderr() {
    let s = Scratch::new("i216");
    let (out, err, rc) =
        s.run("usage() { cat <<EOF\nhelptext\nEOF\n}\nusage > /dev/stderr\necho stdout-done\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "stdout-done\n");
    assert_eq!(err, "helptext\n");
}

#[test]
fn issue263_dup_then_close_move_form_parses() {
    let s = Scratch::new("i263");
    let (out, err, rc) = s.run("exec 5>&1\necho one >&5-\necho two >&5\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "one\ntwo\nrc=0\n");
    assert_eq!(err, "");
}

#[test]
fn issue264_read_write_redirect_creates_file() {
    let s = Scratch::new("i264");
    let (out, err, rc) = s.run(
        "rm -f newfile\n: <> newfile\necho rc=$?\ntest -f newfile && echo created\nwc -c < newfile\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0\ncreated\n0\n");
}

#[test]
fn issue266_redirect_from_unopened_fd() {
    let s = Scratch::new("i266");
    let (out, err, rc) = s.run("echo hi <&7\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, "$S/case.sh: line 1: 7: Bad file descriptor\n");
}
