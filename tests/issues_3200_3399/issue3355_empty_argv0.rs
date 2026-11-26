//! rubash#3355 — empty argv[0]: `$0` must read back empty and diagnostics
//! must print an empty name (GNU bash 5.3.20, measured in #3256 REPORT §6).
//!
//! GNU (third_party/bash, Bash-5.3 patch 20):
//! * `shell.c:1779-1806 set_shell_name` — `dollar_vars[0]` keeps argv[0]
//!   verbatim (`""` stays `""`); the PROGRAM rescue (`shell_name =
//!   PROGRAM` when argv[0] is empty) rebinds only `shell_name`, it never
//!   reaches `$0`.
//! * `error.c:90-120 get_name_for_error` — non-interactive: BASH_SOURCE[0]
//!   if non-empty, else `dollar_vars[0]` verbatim. An empty `$0` is a
//!   non-NULL empty string, so the chain STOPS there: no
//!   `base_pathname(shell_name)` step, no PROGRAM step. Diagnostics print
//!   an empty name (`: line 1: ...`), not `bash:` and not `/:`.
//!
//! rubash used to seed `$0` through `shell_pwd_display_path`, whose
//! empty-input arm returns `"/"` — so `$0` read back `/` and every
//! diagnostic printed `/: ...`. Tests only.

use std::fs;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
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
            std::env::temp_dir().join(format!("rubash-3355-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn base_cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
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

    /// Invoke the built binary with argv[0] set to the empty string
    /// (unix exec mechanics via `CommandExt::arg0`).
    #[cfg(unix)]
    fn run_empty_argv0(&self, args: &[&str]) -> (String, String, i32) {
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_rubash"))
            .arg0("")
            .args(args)
            .output()
            .unwrap();
        self.finish(out)
    }

    /// Control: invoke with the normal argv[0] (the binary path).
    fn run_normal_argv0(&self, args: &[&str]) -> (String, String, i32) {
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_rubash"))
            .args(args)
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
#[cfg(unix)]
fn issue3355_dollar_zero_reads_back_empty() {
    let s = Scratch::new("dollar-zero");
    let (out, err, rc) = s.run_empty_argv0(&["-c", "echo \"[$0]\""]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[]\n", "stderr: {err}");
}

#[test]
#[cfg(unix)]
fn issue3355_bash_argv0_reads_back_empty() {
    let s = Scratch::new("bash-argv0");
    let (out, err, rc) = s.run_empty_argv0(&["-c", "echo \"[$BASH_ARGV0]\""]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[]\n", "stderr: {err}");
}

#[test]
#[cfg(unix)]
fn issue3355_command_not_found_prefix_is_empty() {
    let s = Scratch::new("diag-prefix");
    let (out, err, rc) = s.run_empty_argv0(&["-c", "rubash3355_no_such_command"]);
    assert_eq!(out, "", "stderr: {err}");
    assert_eq!(rc, 127, "stderr: {err}");
    assert_eq!(
        err, ": line 1: rubash3355_no_such_command: command not found\n",
        "stderr: {err}"
    );
}

#[test]
#[cfg(unix)]
fn issue3355_builtin_diagnostics_prefix_is_empty() {
    // Representative builtin faces (executor prolog, diagnostic.rs,
    // prefix.rs): all must print the empty name, not "/" and not "bash".
    let s = Scratch::new("diag-builtins");
    let (out, err, rc) = s.run_empty_argv0(&["-c", "cd /rubash3355_no_such_dir"]);
    assert_eq!(out, "", "stderr: {err}");
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(
        err, ": line 1: cd: /rubash3355_no_such_dir: No such file or directory\n",
        "stderr: {err}"
    );

    let (out, err, rc) = s.run_empty_argv0(&["-c", "echo $((1/0))"]);
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(
        err, ": line 1: 1/0: division by 0 (error token is \"0\")\n",
        "stdout: {out} stderr: {err}"
    );

    let (out, err, rc) = s.run_empty_argv0(&["-c", "ulimit -Z"]);
    assert_eq!(rc, 2, "stdout: {out} stderr: {err}");
    assert_eq!(
        err,
        ": line 1: ulimit: -Z: invalid option\nulimit: usage: ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]\n",
        "stdout: {out} stderr: {err}"
    );

    let (out, err, rc) = s.run_empty_argv0(&["-c", "readonly -p 1x"]);
    assert_eq!(rc, 1, "stdout: {out} stderr: {err}");
    assert_eq!(
        err, ": line 1: readonly: `1x': not a valid identifier\n",
        "stdout: {out} stderr: {err}"
    );

    let (out, err, rc) = s.run_empty_argv0(&["-c", "trap -p 1x"]);
    assert_eq!(rc, 1, "stdout: {out} stderr: {err}");
    assert_eq!(
        err, ": line 1: trap: 1x: invalid signal specification\n",
        "stdout: {out} stderr: {err}"
    );

    // Lib-flavor prolog faces (declare/set/shopt families): their
    // error_name used to be the hardcoded "rubash" brand.
    let (out, err, rc) = s.run_empty_argv0(&["-c", "declare -p 1x"]);
    assert_eq!(rc, 1, "stdout: {out} stderr: {err}");
    assert_eq!(
        err, ": line 1: declare: 1x: not found\n",
        "stdout: {out} stderr: {err}"
    );

    let (out, err, rc) = s.run_empty_argv0(&["-c", "set -o rubash3355_badopt"]);
    assert_eq!(rc, 2, "stdout: {out} stderr: {err}");
    assert_eq!(
        err, ": line 1: set: rubash3355_badopt: invalid option name\n",
        "stdout: {out} stderr: {err}"
    );

    let (out, err, rc) = s.run_empty_argv0(&["-c", "shopt -s rubash3355_badopt"]);
    assert_eq!(rc, 1, "stdout: {out} stderr: {err}");
    assert_eq!(
        err, ": line 1: shopt: rubash3355_badopt: invalid shell option name\n",
        "stdout: {out} stderr: {err}"
    );
}

#[test]
fn issue3355_normal_argv0_still_reports_invocation_name() {
    // Control: the presence-based $0 step must not change the normal
    // face — diagnostics still report $0 (the binary path) verbatim.
    let s = Scratch::new("diag-control");
    let (out, err, rc) = s.run_normal_argv0(&["-c", "rubash3355_no_such_command"]);
    assert_eq!(out, "", "stderr: {err}");
    assert_eq!(rc, 127, "stderr: {err}");
    assert_eq!(
        err,
        format!(
            "{}: line 1: rubash3355_no_such_command: command not found\n",
            env!("CARGO_BIN_EXE_rubash")
        ),
        "stderr: {err}"
    );
}
