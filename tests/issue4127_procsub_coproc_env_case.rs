//! rubash#4127 — on Windows, environment variable names that differ
//! only by case (`V` vs `v`, `foo` vs `FOO` vs `Foo`) collapsed into
//! a single entry when a process-substitution or coproc child was
//! spawned: the std `Command` environment block is keyed
//! case-insensitively, so the first spelling survived carrying the
//! LAST value, and every spelling read that one value in the child.
//! Shell variable names are case-sensitive (and GNU on Windows keeps
//! them distinct), so each spelling must arrive with its own value.
//!
//! The fix routes case-colliding entries through the existing
//! respawn state-file transport in `spill_respawn_env_entries`
//! (Windows-only rule): the first spelling stays inline, later
//! variants ride the file and are spliced back verbatim at import.
//!
//! The collision pins are Windows-only (`#[cfg(windows)]`); the
//! final test pins the same shapes on every platform as the
//! no-collapse baseline (already true off Windows, and it must stay
//! true — the Windows rule must not leak behavior changes elsewhere).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i4127-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// Run `script` as a script FILE with the scratch dir as cwd/HOME;
    /// returns (stdout, stderr, rc).
    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env("HOME", &self.dir)
            .env("LC_ALL", "C")
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn assert_script(&self, script: &str, want_stdout: &str) {
        let (out, err, rc) = self.run(script);
        assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}\nscript:\n{script}");
        assert_eq!(
            out, want_stdout,
            "stdout mismatch; stderr:\n{err}\nscript:\n{script}"
        );
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

const SETUP_AND_ECHO: &str = r#"
V=upper; v=lower; foo=1; FOO=2; Foo=3
"#;

#[cfg(windows)]
#[test]
fn issue4127_input_procsub_env_names_keep_their_case() {
    let s = Scratch::new("procsub-in");
    let script = format!(
        "{SETUP_AND_ECHO}out=$(cat < <(echo \"$V $v | $foo $FOO $Foo\"))\necho \"in:[$out]\"\n"
    );
    s.assert_script(&script, "in:[upper lower | 1 2 3]\n");
}

#[cfg(windows)]
#[test]
fn issue4127_coproc_env_names_keep_their_case() {
    let s = Scratch::new("coproc");
    let script = format!(
        "{SETUP_AND_ECHO}coproc {{ echo \"$V $v | $foo $FOO $Foo\"; }}\nread -r line <&${{COPROC[0]}}\necho \"coproc:[$line]\"\n"
    );
    s.assert_script(&script, "coproc:[upper lower | 1 2 3]\n");
}

/// Baseline: the same shapes keep every spelling distinct. Runs on
/// every platform — off Windows this was never broken, and the
/// Windows-only spill rule must not change it.
#[test]
fn issue4127_env_names_keep_their_case_baseline() {
    let s = Scratch::new("baseline");
    let script = format!(
        "{SETUP_AND_ECHO}out=$(cat < <(echo \"$V $v | $foo $FOO $Foo\"))\necho \"in:[$out]\"\n"
    );
    s.assert_script(&script, "in:[upper lower | 1 2 3]\n");
}
