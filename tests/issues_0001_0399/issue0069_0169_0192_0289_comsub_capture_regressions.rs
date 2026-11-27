//! W4 backfill slice 1 — command substitution capture faces.
//!
//! * rust#69 — `$(eval "...")` result is captured, not executed uncaptured.
//! * rust#169 — a background job spawned inside `$(...)` must not corrupt
//!   the captured output.
//! * rust#192 — a temporary assignment prefix on a command inside `$(...)`
//!   (`FOO=bar echo hello`) behaves normally.
//! * rust#289 — `command` used inside `$(...)` must keep its stdout
//!   captured: `command -V`, `command -v` (builtin shell-function/command
//!   forms) and a command made silent by redirecting PATH must not leak to
//!   the real stdout or capture pieces of neighbouring text.
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
use std::path::{Path, PathBuf};
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

fn sed_path() -> String {
    for cand in ["/usr/bin/sed", "/bin/sed"] {
        if Path::new(cand).exists() {
            return cand.to_string();
        }
    }
    "/usr/bin/sed".to_string()
}

#[test]
fn issue69_eval_inside_comsub_is_captured() {
    let s = Scratch::new("i69");
    let (out, err, rc) = s.run("x=$(eval \"echo hi\")\necho \"[$x]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[hi]\n");
}

#[test]
fn issue169_background_job_inside_comsub_keeps_capture() {
    let s = Scratch::new("i169");
    let (out, err, rc) = s.run("OUT=\"$(for i in 1; do echo J & done; wait)\"\necho \"[$OUT]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[J]\n");
}

#[test]
fn issue192_temp_assignment_prefix_inside_comsub() {
    let s = Scratch::new("i192");
    let (out, err, rc) = s.run("out=$(FOO=bar echo hello)\necho \"[$out]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[hello]\n");
}

#[test]
fn issue289_command_builtin_forms_stay_captured() {
    let s = Scratch::new("i289");
    let sed = sed_path();
    let script = format!(
        "a=$(command -V 'sed')\necho \"a=[$a]\"\nb=$(command 'echo' hi)\necho \"b=[$b]\"\nc=z$(command -v 'sed')z\necho \"c=[$c]\"\nd=$(PATH=/usr/bin; command -v sed)\necho \"d=[$d]\"\n"
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        format!("a=[sed is {sed}]\nb=[hi]\nc=[z{sed}z]\nd=[{sed}]\n")
    );
    assert_eq!(err, "");
}
