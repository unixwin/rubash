//! rubash#3413 — a `BASH_ARGV0` imported from the environment with an
//! EMPTY value is still an import: it must seed `$0` (and the diagnostic
//! name) as the empty string, not fall back to argv[0].
//!
//! GNU (third_party/bash, Bash-5.3 patch 20):
//! * `variables.c:368 initialize_shell_variables` — every `name=value`
//!   environment string is bound and marked `att_exported|att_imported`
//!   (bind_variable + VSETATTR att_imported); `BASH_ARGV0=` therefore
//!   creates the variable with an empty value cell. There is no
//!   empty-value filter anywhere in the import path.
//! * `variables.c:608` — `set_argv0()` runs at the tail of
//!   initialization; `variables.c:1547-1555 set_argv0` tests only
//!   existence + imported (`v && imported_p (v)`), then
//!   `variables.c:1528-1545 assign_bash_argv0` copies the value verbatim
//!   into `dollar_vars[0]` and `shell_name` — only a NULL pointer is
//!   rejected, the empty string is a legal seed.
//!
//! rubash's binary entry re-seeded the import through
//! `.filter(|value| !value.is_empty())`, collapsing "set to empty" into
//! "unset" and overwriting the imported empty value with the argv[0]
//! fallback. Measured against GNU bash 5.3.20: `BASH_ARGV0= bash -c
//! 'echo [$0]'` prints `[]` and diagnostics print an empty name
//! (`: line 1: ...`). Tests only.

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
            std::env::temp_dir().join(format!("rubash-3413-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn base_cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_rubash"));
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir);
        c
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        let norm = |b: &[u8]| String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S");
        (
            norm(&out.stdout),
            norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Invoke with `BASH_ARGV0=<value>` present in the environment
    /// (the empty string included — that is the face under test).
    #[cfg(unix)]
    fn run_with_import(&self, value: &str, args: &[&str]) -> (String, String, i32) {
        let out = self
            .base_cmd()
            .env("BASH_ARGV0", value)
            .args(args)
            .output()
            .unwrap();
        self.finish(out)
    }

    /// Control: invoke with `BASH_ARGV0` absent from the environment.
    fn run_without_import(&self, args: &[&str]) -> (String, String, i32) {
        let out = self.base_cmd().args(args).output().unwrap();
        self.finish(out)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
#[cfg(unix)]
fn issue3413_empty_import_dollar_zero_is_empty() {
    let s = Scratch::new("dollar-zero");
    let (out, err, rc) = s.run_with_import("", &["-c", "echo \"[$0]\""]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[]\n", "stderr: {err}");
}

#[test]
#[cfg(unix)]
fn issue3413_empty_import_bash_argv0_reads_back_empty() {
    let s = Scratch::new("readback");
    let (out, err, rc) = s.run_with_import("", &["-c", "echo \"[$BASH_ARGV0]\""]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[]\n", "stderr: {err}");
}

#[test]
#[cfg(unix)]
fn issue3413_empty_import_diagnostic_prefix_is_empty() {
    let s = Scratch::new("diag-prefix");
    let (out, err, rc) = s.run_with_import("", &["-c", "rubash3413_no_such_command"]);
    assert_eq!(out, "", "stderr: {err}");
    assert_eq!(rc, 127, "stderr: {err}");
    assert_eq!(
        err, ": line 1: rubash3413_no_such_command: command not found\n",
        "stderr: {err}"
    );
}

#[test]
#[cfg(unix)]
fn issue3413_empty_import_builtin_diagnostic_prefix_is_empty() {
    // The builtin prolog faces read the same report-name chain as the
    // external not-found face: an empty import empties their name too.
    let s = Scratch::new("diag-builtin");
    let (out, err, rc) = s.run_with_import("", &["-c", "cd /rubash3413_no_such_dir"]);
    assert_eq!(out, "", "stderr: {err}");
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(
        err, ": line 1: cd: /rubash3413_no_such_dir: No such file or directory\n",
        "stderr: {err}"
    );
}

#[test]
#[cfg(unix)]
fn issue3413_nonempty_import_seeds_dollar_zero_and_bash_argv0() {
    // Control (GNU variables.c set_argv0/assign_bash_argv0): a non-empty
    // import becomes dollar_vars[0] AND the BASH_ARGV0 value — the shell
    // prints the imported name twice.
    let s = Scratch::new("nonempty");
    let (out, err, rc) = s.run_with_import("imported-name", &["-c", "echo \"[$0][$BASH_ARGV0]\""]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[imported-name][imported-name]\n", "stderr: {err}");
}

#[test]
fn issue3413_unset_import_falls_back_to_invocation_name() {
    // Control: with no import at all, $0 is still the invocation name
    // (argv[0] verbatim) — the existence-based step must not disturb
    // the fallback face.
    let s = Scratch::new("unset-control");
    let (out, err, rc) = s.run_without_import(&["-c", "echo \"[$0]\""]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        format!("[{}]\n", env!("CARGO_BIN_EXE_rubash")),
        "stderr: {err}"
    );
}
