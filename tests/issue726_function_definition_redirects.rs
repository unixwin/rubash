//! Issue rubash#726 regression: a function *definition*-level redirect list
//! must bind the body's default streams.
//!
//! GNU stores a definition's redirect list on the function's body command
//! and applies it when the body runs — AFTER the call-site redirects
//! (execute_cmd.c:854 do_redirections over tc->redirects; the call's
//! redirects are installed at execute_cmd.c:5606 before dispatch). rubash
//! bound only the numbered (>2) entries of the definition and dropped the
//! stdio legs, so `f(){ echo OUT; echo ERR >&2; } 2>&1 >err; f 1>out` sent
//! OUT to stdout instead of `err`.
//!
//! Byte-checked against WSL GNU Bash 5.3.0 script-file probes; the `bash`
//! on PATH is the Niubash shim (circular here).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

impl Outcome {
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }
}

fn run_rubash(slug: &str, script: &str) -> Outcome {
    let dir =
        std::env::temp_dir().join(format!("rubash-issue726-fn-{slug}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue726 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes()).expect("write probe");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    Outcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

/// The definition redirect list binds the body's default streams after the
/// call-site redirects: `2>&1` snapshots fd 1 (= the call's `out`), then
/// `>err` rebinds fd 1, so OUT lands in `err` and ERR (fd 2) in `out`.
#[test]
fn function_definition_dup_then_file_splits_streams() {
    let out = run_rubash(
        "dup-file",
        "f(){ echo OUT; echo ERR >&2; } 2>&1 >err; f 1>out\n\
         printf 'stdout_tail\\n'\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "stdout_tail\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("err"), "OUT\n");
    assert_eq!(out.file("out"), "ERR\n");
}

/// A definition-level `2>&1` with only a call-site stdout redirect makes both
/// body streams share the call target.
#[test]
fn function_definition_dup_with_call_site_stdout() {
    let out = run_rubash(
        "dup-call",
        "g(){ echo OUT2; echo ERR2 >&2; } 2>&1; g 1>out2\n\
         printf 'stdout_tail\\n'\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "stdout_tail\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("out2"), "OUT2\nERR2\n");
}

/// A call-site stderr WRITE must not clobber a definition-level `2>&1`: the
/// definition binds fd 2 to the live-stdout marker after the call site, so
/// ERR prints and `e` stays empty.
#[test]
fn function_definition_dup_overrides_call_site_stderr_write() {
    let out = run_rubash(
        "dup-vs-call-stderr",
        "f(){ echo OUT; echo ERR >&2; } 2>&1; f 2>e\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "OUT\nERR\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("e"), "");
}

/// Guard: the call-site path rubash#725 fixed still orders left to right
/// (`f 2>&1 >/dev/null` keeps ERR visible).
#[test]
fn function_call_site_dup_still_orders_left_to_right() {
    let out = run_rubash(
        "call-dup",
        "f(){ echo OUT; echo ERR >&2; }\n\
         f 2>&1 >/dev/null\n\
         f >/dev/null 2>&1\n\
         echo done\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "ERR\ndone\n");
    assert_eq!(out.stderr, "");
}

/// Guard: `>&file` (r_err_and_out) still routes both streams for a group and
/// a function definition.
#[test]
fn redirect_err_and_out_file_unchanged() {
    let out = run_rubash(
        "err-and-out",
        "{ echo A; echo B >&2; } >&g\n\
         fd(){ echo C; echo D >&2; } >&fdef; fd\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("g"), "A\nB\n");
    assert_eq!(out.file("fdef"), "C\nD\n");
}

/// Guard: a definition numbered output binding (`5>f5`) still reaches a body
/// `>&5`.
#[test]
fn function_definition_numbered_output_unchanged() {
    let out = run_rubash("numbered-out", "d3(){ echo X >&5; } 5>f5; d3\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("f5"), "X\n");
}
