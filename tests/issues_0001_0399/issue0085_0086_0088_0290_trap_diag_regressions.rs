//! W4 backfill slice 1 — trap handling and fatal-diagnostic faces.
//!
//! * rust#85 — `trap` option validation: `-p` and `-P` together are an
//!   error (rc 2, "cannot specify both -p and -P"); bare `trap -P` with
//!   no signal names is an error (rc 2, "-P requires at least one signal
//!   name").
//! * rust#86 — identifier validation per builtin: `unset '1x'` is silent
//!   success in GNU, while `export 'bad-name=1'` and `readonly 'f\1'`
//!   fail with "not a valid identifier" and rc 1.
//! * rust#88 — `set -x` traces arithmetic commands (`+ ((  i++  ))`).
//! * rust#290 — under `set -u`, expanding `$!` with no background job is
//!   a fatal unbound-variable error.
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
fn issue85_trap_p_and_capital_p_are_mutually_exclusive() {
    let s = Scratch::new("i85");
    let (out, err, rc) = s.run("trap -p -P EXIT\necho rc=$?\ntrap -P\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=2\nrc=2\n");
    assert_eq!(
        err,
        "$S/case.sh: line 1: trap: cannot specify both -p and -P\n$S/case.sh: line 3: trap: -P requires at least one signal name\n"
    );
}

#[test]
fn issue86_identifier_validation_per_builtin() {
    let s = Scratch::new("i86");
    let (out, err, rc) = s.run(
        "unset '1x'\necho rc=$?\nexport 'bad-name=1'\necho rc=$?\nreadonly 'f\\1'\necho rc=$?\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0\nrc=1\nrc=1\n");
    assert_eq!(
        err,
        "$S/case.sh: line 3: export: `bad-name=1': not a valid identifier\n$S/case.sh: line 5: readonly: `f\\1': not a valid identifier\n"
    );
}

#[test]
fn issue88_xtrace_covers_arithmetic_commands() {
    let s = Scratch::new("i88");
    let (out, err, rc) = s.run("set -x\ni=0\n(( i++ ))\necho \"i=$i\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "i=1\n");
    assert!(
        err.contains("+ ((  i++  ))"),
        "xtrace must include the arithmetic command, got: {err}"
    );
}

#[test]
fn issue290_nounset_last_bg_pid_is_fatal() {
    let s = Scratch::new("i290");
    let (out, err, rc) = s.run("set -u\necho \"t=[$!]\"\necho after\n");
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(err, "$S/case.sh: line 2: $!: unbound variable\n");
}
