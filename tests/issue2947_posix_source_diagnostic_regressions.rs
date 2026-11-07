//! rubash#2947 — POSIX-mode `source` diagnostic prints `.` instead of
//! `source`.
//!
//! GNU anchor: builtins/source.def registers TWO builtins over one
//! implementation — `source` (source.def:23) and `.` (source.def:39),
//! both calling source_builtin — and the file-not-found diagnostic is
//! `builtin_error (_("%s: file not found"), x)` (source.def:179), where
//! builtin_error prefixes `this_command_name`, the name the builtin was
//! INVOKED as (builtins/common.c:93-94). So POSIX mode's no-fallback
//! miss must name `source` when invoked as `source` and `.` when
//! invoked as `.`; rubash hardcoded the `.` spelling for both.
//!
//! Behaviour is otherwise identical (no cwd fallback in POSIX mode; a
//! failed special builtin in POSIX mode is fatal — the seeds wrap the
//! call in a subshell) and is NOT part of this issue.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (oracle probes on
//! master 4f747412): only the POSIX `source`-form name diverges.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

struct CaseDir(PathBuf);

impl CaseDir {
    fn new(label: &str) -> Self {
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rubash_issue2947_{}_{}_{}",
            std::process::id(),
            label,
            n
        ));
        fs::create_dir_all(&dir).expect("create case dir");
        // Present in the cwd on purpose: POSIX mode must NOT fall back
        // to it, default mode must find it.
        fs::write(dir.join("mylib.sh"), "SOURCED_OK=1\n").expect("write mylib.sh");
        Self(dir)
    }

    fn run(&self, script: &str) -> (String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", script])
            .current_dir(&self.0)
            .stdin(Stdio::null())
            .env_remove("BASH_ENV")
            .env_remove("WINUXSH_ROOT")
            .output()
            .expect("run rubash -c");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap_or_default()
    }
}

impl Drop for CaseDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn posix_source_miss_names_source() {
    // Regression seed: POSIX mode, invoked as `source`, missing from
    // PATH — the diagnostic names the invoked builtin.
    let dir = CaseDir::new("posix_source");
    let script = "( PATH=/usr/bin:/bin; set -o posix; source mylib.sh ) 2>err.txt; true";
    let (_, stderr) = dir.run(script);
    let err = dir.read("err.txt");
    assert!(
        err.contains("source: mylib.sh: file not found"),
        "err.txt={err:?} stderr={stderr:?}"
    );
}

#[test]
fn posix_dot_miss_names_dot() {
    // Control (mE4, both shells): invoked as `.`, the name stays `.`.
    let dir = CaseDir::new("posix_dot");
    let script = "( PATH=/usr/bin:/bin; set -o posix; . mylib.sh ) 2>err.txt; true";
    dir.run(script);
    let err = dir.read("err.txt");
    assert!(
        err.contains(".: mylib.sh: file not found"),
        "err.txt={err:?}"
    );
}

#[test]
fn source_dash_p_miss_names_source() {
    // The -p search arm shares the same diagnostic site: the invoked
    // name rule applies there too (GNU, default mode).
    let dir = CaseDir::new("dash_p_source");
    let (_, stderr) = dir.run("source -p /nonexistent_dir_xyz mylib.sh");
    assert!(
        stderr.contains("source: mylib.sh: file not found"),
        "stderr={stderr:?}"
    );
}

#[test]
fn dot_dash_p_miss_names_dot() {
    // Control: `. -p` keeps the dot name on both shells.
    let dir = CaseDir::new("dash_p_dot");
    let (_, stderr) = dir.run(". -p /nonexistent_dir_xyz mylib.sh");
    assert!(
        stderr.contains(".: mylib.sh: file not found"),
        "stderr={stderr:?}"
    );
}

#[test]
fn default_mode_miss_has_no_builtin_name() {
    // Control (mE1, both shells): outside POSIX mode a plain miss is a
    // plain open failure — no builtin name, no "file not found".
    let dir = CaseDir::new("default_miss");
    let (_, stderr) = dir.run("source nosuchfile.sh");
    assert!(
        stderr.contains("nosuchfile.sh: No such file or directory"),
        "stderr={stderr:?}"
    );
    assert!(!stderr.contains("file not found"), "stderr={stderr:?}");
}

#[test]
fn default_mode_cwd_fallback_still_works() {
    // Control (mE8, both shells): default mode DOES fall back to the
    // cwd and sources mylib.sh; only POSIX mode skips the fallback.
    let dir = CaseDir::new("cwd_fallback");
    let (stdout, stderr) = dir.run("PATH=/usr/bin:/bin; source mylib.sh; echo \"ok=$SOURCED_OK\"");
    assert_eq!(stdout, "ok=1\n", "stderr={stderr:?}");
}
