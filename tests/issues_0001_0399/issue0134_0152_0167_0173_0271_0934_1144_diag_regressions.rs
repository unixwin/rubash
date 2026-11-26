//! W4 backfill slice 1 — diagnostic routing faces.
//!
//! * rust#134 — an empty expansion used as a command name (`$E` with E
//!   unset) is a silent no-op with rc 0.
//! * rust#152 — an empty quoted command name (`"$E"`) and an empty
//!   expansion followed by arguments are "command not found" (rc 127).
//! * rust#167 — a syntax error inside `eval` is contained by a `2>`
//!   redirection on the eval / on the enclosing substitution: nothing
//!   reaches the real stderr, and the result is empty with rc 2 inside.
//! * rust#173 — executing a nonexistent slash path reports
//!   "No such file or directory" with rc 127 (not "Permission denied").
//! * rust#271 — the `unset` readonly diagnostic honors redirection; it
//!   is capturable via `$(... 2>&1)`.
//! * rust#934 — a failed redirection inside a brace group whose stderr
//!   is redirected to a file lands in the file, not on the real stderr.
//! * rust#1144 — `exec` of a nonexistent command reports
//!   "exec: NAME: not found" (on stdout, per GNU) and exits 127.
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
fn issue134_empty_unquoted_command_is_noop() {
    let s = Scratch::new("i134");
    let (out, err, rc) = s.run("unset E\n$E\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "");
}

#[test]
fn issue152_empty_quoted_or_with_args_is_not_found() {
    let s = Scratch::new("i152");
    let (out, err, rc) = s.run("E=\n\"$E\"\necho rc=$?\n$E extra\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=127\nrc=127\n");
    assert_eq!(
        err,
        "$S/case.sh: line 2: : command not found\n$S/case.sh: line 4: extra: command not found\n"
    );
}

#[test]
fn issue167_eval_syntax_error_honors_redirection() {
    let s = Scratch::new("i167");
    let (out, err, rc) = s.run(
        "eval 'echo ${x' 2>/dev/null\necho \"evalrc=$?\"\nout=$(eval 'echo ${x' 2>/dev/null)\necho \"comsub=[$out]\"\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "evalrc=2\ncomsub=[]\n");
    assert_eq!(err, "");
}

#[test]
fn issue173_missing_slash_path_reports_enoent() {
    let s = Scratch::new("i173");
    let (out, err, rc) = s.run("./nosuch-xyz/cmd\necho rc=$?\n/nonexistent-xyz/cmd\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=127\nrc=127\n");
    assert_eq!(
        err,
        "$S/case.sh: line 1: ./nosuch-xyz/cmd: No such file or directory\n$S/case.sh: line 3: /nonexistent-xyz/cmd: No such file or directory\n"
    );
}

#[test]
fn issue271_unset_readonly_diagnostic_is_capturable() {
    let s = Scratch::new("i271");
    let (out, err, rc) = s.run(
        "readonly RO=1\nunset RO 2>/dev/null\necho \"after\"\nV=$(unset RO 2>&1)\necho \"cap=[$V]\"\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "after\ncap=[$S/case.sh: line 4: unset: RO: cannot unset: readonly variable]\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue934_group_redirection_failure_lands_in_file() {
    let s = Scratch::new("i934");
    let (out, err, rc) = s.run(
        "{ echo hi < /nonexistent-xyz/f; } 2>err.txt\necho rc=$?\necho \"errfile:\"\ncat err.txt\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "rc=1\nerrfile:\n$S/case.sh: line 1: /nonexistent-xyz/f: No such file or directory\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue1144_exec_of_missing_command_reports_not_found() {
    let s = Scratch::new("i1144");
    let (out, err, rc) = s.run("exec nosuchcmd_xyz 2>&1\necho after\n");
    assert_eq!(rc, 127, "stderr: {err}");
    assert_eq!(out, "$S/case.sh: line 1: exec: nosuchcmd_xyz: not found\n");
}
