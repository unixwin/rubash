//! rubash#3254 — `src/main.rs` startup diagnostics must derive the program
//! name from argv[0], never from a literal `bash:` / `rubash:`.
//!
//! GNU (bash 5.3.20, measured with the local build in
//! `~/workspace/rubash-audit/build-gnu/bash-5.3/`, probe script
//! `~/workspace/fix-3254/probes/gnu-3254-oracle.sh`):
//! * `shell.c:467 set_shell_name` binds `$0` to argv[0] verbatim;
//!   `shell.c:475/493` parse the options before `set_argv0`
//!   (variables.c:1547-1555) can apply an imported `BASH_ARGV0`, so an
//!   import does NOT rename the option-parse diagnostics — both `-o`
//!   names stay argv[0] (`shell.c:492 this_command_name = shell_name`
//!   during the short-option scan; `shell.c:529` clears it before
//!   `run_shopt_alist`, which is why `-O` carries no second name).
//! * `error.c:90-120 get_name_for_error` — the prolog name is
//!   `dollar_vars[0]` verbatim; an empty `$0` stays empty (#3355) and
//!   only a genuinely absent name falls back to PROGRAM (`"bash"`).
//!   The usage block (`shell.c:2057-2103 show_shell_usage`) prints
//!   `shell_name`, which `set_shell_name` rescues to PROGRAM when
//!   argv[0] is empty — so an empty argv[0] prints an empty diagnostic
//!   name but a `bash` usage block.
//! * Long options name themselves WITHOUT dashes (`shell.c:863-867`:
//!   `init-file`, `rcfile`).
//! * The debugger pair (`shell.c:1547-1566 start_debugger`) fires with
//!   the `$0` bound at driver time: the `-c` `$0` operand, the script
//!   name, or the invocation name (`shell.c:764/:795`).
//! * A missing/unreadable `--init-file`/`--rcfile` file is SILENT in
//!   GNU (no diagnostic, rc 0) — that is a separate divergence, NOT
//!   nailed here; only the program NAME of the product diagnostic is
//!   in scope for the init-file arm.
//!
//! Every expectation below is the GNU byte; the base prints the
//! literal instead, so each test fails on the base (fail-first by
//! construction — the expected strings differ literally from the
//! base's hardcoded literals) and passes after the fix.

use std::fs;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const STARTER: &str = "/usr/local/share/bashdb/bashdb-main.inc";

fn system_starter_exists() -> bool {
    PathBuf::from(STARTER).exists() || PathBuf::from("/usr/share/bashdb/bashdb-main.inc").exists()
}

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
            std::env::temp_dir().join(format!("rubash-3254-{tag}-{}-{nanos}", std::process::id()));
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

    /// Invoke the built binary with argv[0] set to `name`
    /// (unix exec mechanics via `CommandExt::arg0`).
    #[cfg(unix)]
    fn run_argv0(&self, name: &str, args: &[&str]) -> (String, String, i32) {
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_rubash"))
            .arg0(name)
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

fn assert_usage_block(stderr: &str, name: &str) {
    assert!(
        stderr.contains(&format!(
            "Usage:\t{name} [GNU long option] [option] ...\n\t{name} [GNU long option] [option] script-file ...\n"
        )),
        "usage block must carry {name:?}: {stderr:?}"
    );
    assert!(
        stderr.contains("GNU long options:\n"),
        "usage block must list long options: {stderr:?}"
    );
    assert!(
        stderr.ends_with("\t-abefhkmnptuvxBCEHPT or -o option\n"),
        "usage block tail: {stderr:?}"
    );
    assert!(
        !stderr.contains("bash [GNU long option]"),
        "usage block must not fall back to literal bash: {stderr:?}"
    );
    assert!(
        !stderr.contains("rubash:"),
        "usage block must not carry a literal rubash: prefix: {stderr:?}"
    );
}

/// Control (runs on every platform): with the harness argv[0] (the
/// binary path) the `-o` diagnostic prints that path twice — GNU
/// prints argv[0] verbatim, and the harness invokes by full path.
#[test]
fn issue3254_control_dash_o_bad_name_uses_invocation_path() {
    let s = Scratch::new("control-o");
    let bin = env!("CARGO_BIN_EXE_rubash");
    let (_out, err, rc) = s.run_normal_argv0(&["-o", "badname"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(
        err,
        format!("{bin}: line 0: {bin}: badname: invalid option name\n")
    );
}

/// Control (runs on every platform): `-c` with no command string.
#[test]
fn issue3254_control_dash_c_without_command_uses_invocation_path() {
    let s = Scratch::new("control-c");
    let bin = env!("CARGO_BIN_EXE_rubash");
    let (_out, err, rc) = s.run_normal_argv0(&["-c"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, format!("{bin}: -c: option requires an argument\n"));
}

#[test]
#[cfg(unix)]
fn issue3254_unknown_long_option_uses_argv0() {
    let s = Scratch::new("long-unknown");
    let (_out, err, rc) = s.run_argv0("myname", &["--bad-opt-xyz"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert!(
        err.starts_with("myname: --bad-opt-xyz: invalid option\n"),
        "stderr: {err:?}"
    );
    assert_usage_block(&err, "myname");
}

#[test]
#[cfg(unix)]
fn issue3254_dash_o_bad_name_uses_argv0() {
    let s = Scratch::new("dash-o");
    let (_out, err, rc) = s.run_argv0("myname", &["-o", "badname"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: line 0: myname: badname: invalid option name\n");
    let (_out, err, rc) = s.run_argv0("myname", &["+o", "badname"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: line 0: myname: badname: invalid option name\n");
}

#[test]
#[cfg(unix)]
fn issue3254_dash_cap_o_bad_name_uses_argv0() {
    let s = Scratch::new("dash-cap-o");
    let (_out, err, rc) = s.run_argv0("myname", &["-O", "badname"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: line 0: badname: invalid shell option name\n");
    let (_out, err, rc) = s.run_argv0("myname", &["+O", "badname"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: line 0: badname: invalid shell option name\n");
}

#[test]
#[cfg(unix)]
fn issue3254_invalid_short_option_uses_argv0() {
    let s = Scratch::new("short-invalid");
    let (_out, err, rc) = s.run_argv0("myname", &["-Z"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert!(
        err.starts_with("myname: -Z: invalid option\n"),
        "stderr: {err:?}"
    );
    assert_usage_block(&err, "myname");
}

#[test]
#[cfg(unix)]
fn issue3254_dash_c_without_command_uses_argv0() {
    let s = Scratch::new("dash-c");
    let (_out, err, rc) = s.run_argv0("myname", &["-c"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: -c: option requires an argument\n");
}

#[test]
#[cfg(unix)]
fn issue3254_init_file_without_argument_uses_bare_name() {
    let s = Scratch::new("init-file");
    let (_out, err, rc) = s.run_argv0("myname", &["--init-file"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: init-file: option requires an argument\n");
}

#[test]
#[cfg(unix)]
fn issue3254_rcfile_without_argument_uses_bare_name() {
    let s = Scratch::new("rcfile");
    let (_out, err, rc) = s.run_argv0("myname", &["--rcfile"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: rcfile: option requires an argument\n");
}

#[test]
#[cfg(unix)]
fn issue3254_profile_output_without_argument_uses_argv0() {
    let s = Scratch::new("profile-output");
    let (_out, err, rc) = s.run_argv0("myname", &["--profile-output"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(err, "myname: --profile-output: option requires an argument\n");
}

#[test]
#[cfg(unix)]
fn issue3254_debugger_missing_starter_uses_argv0() {
    if system_starter_exists() {
        return;
    }
    let s = Scratch::new("debugger");
    let (out, err, rc) = s.run_argv0("myname", &["--debugger", "-c", "echo hi"]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\n", "stderr: {err}");
    assert_eq!(
        err,
        "myname: /usr/local/share/bashdb/bashdb-main.inc: No such file or directory\n\
         myname: warning: cannot start debugger; debugging mode disabled\n"
    );
}

#[test]
#[cfg(unix)]
fn issue3254_debugger_missing_starter_uses_c_dollar_zero() {
    if system_starter_exists() {
        return;
    }
    let s = Scratch::new("debugger-c0");
    let (out, err, rc) = s.run_argv0("myname", &["--debugger", "-c", "echo hi", "customzero"]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\n", "stderr: {err}");
    assert_eq!(
        err,
        "customzero: /usr/local/share/bashdb/bashdb-main.inc: No such file or directory\n\
         customzero: warning: cannot start debugger; debugging mode disabled\n"
    );
}

#[test]
#[cfg(unix)]
fn issue3254_debugger_missing_starter_uses_script_name() {
    if system_starter_exists() {
        return;
    }
    let s = Scratch::new("debugger-script");
    let script = s.dir.join("script.sh");
    fs::write(&script, "echo \"script-0=$0\"\n").unwrap();
    let script_arg = script.to_str().unwrap().to_string();
    let (out, err, rc) = s.run_argv0("myname", &["--debugger", &script_arg]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, format!("script-0={script_arg}\n"), "stderr: {err}");
    assert_eq!(
        err,
        "$S/script.sh: /usr/local/share/bashdb/bashdb-main.inc: No such file or directory\n\
         $S/script.sh: warning: cannot start debugger; debugging mode disabled\n"
    );
}

#[test]
#[cfg(unix)]
fn issue3254_debugger_missing_starter_uses_argv0_on_stdin() {
    if system_starter_exists() {
        return;
    }
    let s = Scratch::new("debugger-stdin");
    let (out, err, rc) = s.run_argv0("myname", &["--debugger"]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "", "stderr: {err}");
    assert_eq!(
        err,
        "myname: /usr/local/share/bashdb/bashdb-main.inc: No such file or directory\n\
         myname: warning: cannot start debugger; debugging mode disabled\n"
    );
}

#[test]
#[cfg(unix)]
fn issue3254_empty_argv0_diagnostic_empty_but_usage_bash() {
    let s = Scratch::new("empty-argv0");
    let (_out, err, rc) = s.run_argv0("", &["--bad-opt-xyz"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert!(
        err.starts_with(": --bad-opt-xyz: invalid option\n"),
        "diagnostic name must stay empty: {err:?}"
    );
    // shell.c set_shell_name rescues an empty shell_name to PROGRAM,
    // so the usage block — unlike the diagnostic — prints `bash`.
    assert!(
        err.contains("Usage:\tbash [GNU long option] [option] ...\n\tbash [GNU long option] [option] script-file ...\n"),
        "usage block must rescue to bash: {err:?}"
    );
}

#[test]
#[cfg(unix)]
fn issue3254_full_path_argv0_printed_verbatim() {
    let s = Scratch::new("full-path-argv0");
    let (_out, err, rc) = s.run_argv0("/weird/path/foo", &["--bad-opt-xyz"]);
    assert_eq!(rc, 2, "stderr: {err}");
    assert!(
        err.starts_with("/weird/path/foo: --bad-opt-xyz: invalid option\n"),
        "stderr: {err:?}"
    );
    assert_usage_block(&err, "/weird/path/foo");
}

/// Scope pin: `--help` under a non-bash/sh name keeps the product help
/// (rubash#585) — only `show_shell_usage`, the GNU usage block on a
/// usage error, follows argv[0].
#[test]
#[cfg(unix)]
fn issue3254_help_under_other_name_keeps_product_help() {
    let s = Scratch::new("help-other-name");
    let (out, err, rc) = s.run_argv0("myname", &["--help"]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(err, "");
    assert!(
        out.starts_with("Usage: rubash [-c command] [script]\n"),
        "stdout: {out:?}"
    );
}
