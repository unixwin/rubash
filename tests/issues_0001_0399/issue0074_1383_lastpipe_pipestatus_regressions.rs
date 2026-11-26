//! W4 backfill slice 1 — lastpipe and PIPESTATUS faces.
//!
//! * rust#74 — `shopt -s lastpipe`: the last pipeline element runs in the
//!   current shell, so variables it sets survive the pipeline.
//! * rust#1383 — `PIPESTATUS` still describes the last foreground pipeline
//!   after an intervening `if` compound command runs.
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
fn issue74_lastpipe_runs_last_element_in_current_shell() {
    let s = Scratch::new("i74");
    let (out, err, rc) = s.run("shopt -s lastpipe\nv=unset\necho hello | read v\necho \"v=$v\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "v=hello\n");
}

#[test]
fn issue1383_pipestatus_survives_intervening_if() {
    let s = Scratch::new("i1383");
    let (out, err, rc) = s.run("true | true\nif false; then :; fi\necho \"ps=${PIPESTATUS[@]}\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "ps=1\n");
}
