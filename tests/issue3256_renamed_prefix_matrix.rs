//! rubash#3256 (b) migration — the renamed-shell diagnostic matrix.
//!
//! GNU has exactly ONE name source for every diagnostic face
//! (error.c:90-120 get_name_for_error): non-interactive shells
//! print $0 verbatim (a path invocation prints the full path),
//! script frames print the script name, interactive shells print
//! base_pathname(shell_name), and sys_error (error.c:275-281)
//! never carries a line segment. The stopped faces of #3256 each
//! kept a private derivation — a hard-coded "rubash" fallback, the
//! shell-name carrier printed VERBATIM as the interactive name, or
//! (for alias) a process-env prolog that cannot even see the
//! interactive carrier — and the renamed-matrix experiment in the
//! #3256 dossier measured four divergence classes at the CLI:
//! interactive alias printed the literal name plus a spurious line
//! segment, interactive export/readonly/unset printed the full
//! invocation path, interactive bare exec printed the literal
//! name, and the TIMEFORMAT diagnostic printed the full path.
//!
//! The (b) wave migrated every face onto the single argv[0]
//! carrier chain (builtins::prefix, rubash#2791). This pin locks
//! the matrix: renamed invocations (arg0 rename, a physically
//! copied binary invoked by path) x the lib-reachable faces
//! (alias, export, readonly, unset, exec, cd's sys_error face) x
//! the -c / script / interactive forms, plus direct calls of the
//! one public prefix face (builtins::set::builtin_error_prefix,
//! the TIMEFORMAT diagnostic's prefix).
//!
//! Fail-first account (base 20ba3aea, before the migration): the
//! interactive cells and the chain cells of
//! `builtin_error_prefix_derives_every_arm` are RED (literal name,
//! full path, spurious line segment); the -c and script cells are
//! GREEN on both sides — the startup seed already fed those faces
//! the invocation name, and they lock the migration (and the
//! trap_exec literal-prefix shim's removal) against regression.

#![cfg(unix)]

use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

fn rubash() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
}

/// A `-c` invocation under a renamed argv[0].
fn dash_c_renamed(command: &str) -> std::process::Output {
    let mut cmd = rubash();
    cmd.arg0("zimsh");
    cmd.args(["-c", command])
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash -c")
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is utf-8")
}

fn fixture_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3256-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::canonicalize(&dir).expect("canonicalize fixture dir")
}

fn write_bad_interpreter(dir: &std::path::Path) {
    let path = dir.join("badinterp.sh");
    std::fs::write(&path, "#!/nonexistent-interp-3256\necho hi\n")
        .expect("write badinterp fixture");
    // The interpreter refusal is only reached when the file itself
    // is executable; without the bit the spawn fails earlier with
    // EACCES (the experiment's fixture is chmod +x for this reason).
    let mut perms = std::fs::metadata(&path)
        .expect("fixture metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&path, perms).expect("chmod badinterp fixture");
}

/// An interactive session fed `input` on a pipe, under a renamed
/// argv[0]. Job-control warnings and the prompt share stderr, so
/// callers assert on the diagnostic lines themselves.
fn interactive_arg0(input: &str, cwd: &std::path::Path) -> String {
    let mut cmd = rubash();
    cmd.arg0("zimsh");
    let mut child = cmd
        .arg("-i")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(input.as_bytes())
        .expect("feed interactive input");
    let output = child.wait_with_output().expect("interactive session ends");
    stderr_of(&output)
}

/// An interactive session of a physically renamed copy of the
/// binary, invoked by its absolute path (so the startup shell_name
/// carrier holds the full path — the form that exposed the
/// verbatim-carrier divergence).
fn interactive_path_copy(input: &str, dir: &std::path::Path) -> String {
    let copy = dir.join("zimsh");
    std::fs::copy(env!("CARGO_BIN_EXE_rubash"), &copy).expect("copy binary under renamed name");
    let mut child = Command::new(&copy)
        .arg("-i")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn renamed copy -i");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(input.as_bytes())
        .expect("feed interactive input");
    let output = child.wait_with_output().expect("interactive session ends");
    stderr_of(&output)
}

#[test]
fn dash_c_renamed_faces_carry_the_invocation_name() {
    // GNU (renamed matrix R1): zimsh: line 1: <builtin>: -z: invalid
    // option, followed by the builtin's usage line (no prolog).
    let out = dash_c_renamed("exec -z");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_of(&out),
        "zimsh: line 1: exec: -z: invalid option\nexec: usage: exec [-cl] [-a name] [command [argument ...]] [redirection ...]\n"
    );

    let out = dash_c_renamed("alias -z");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_of(&out),
        "zimsh: line 1: alias: -z: invalid option\nalias: usage: alias [-p] [name[=value] ... ]\n"
    );
}

#[test]
fn script_renamed_faces_carry_the_script_name() {
    let dir = fixture_dir("script");
    // The bare-exec script cell is also the regression proof for
    // removing the trap_exec literal-prefix shim (rubash#3256 (b)
    // item 6): the builtin face now derives the script:line prefix
    // itself, byte-identical to what the shim used to substitute.
    std::fs::write(dir.join("s.sh"), "exec -z\n").expect("write script");
    std::fs::write(dir.join("a.sh"), "alias -z\n").expect("write script");
    for (script, expected) in [
        (
            "s.sh",
            "./s.sh: line 1: exec: -z: invalid option\nexec: usage: exec [-cl] [-a name] [command [argument ...]] [redirection ...]\n",
        ),
        (
            "a.sh",
            "./a.sh: line 1: alias: -z: invalid option\nalias: usage: alias [-p] [name[=value] ... ]\n",
        ),
    ] {
        let mut cmd = rubash();
        cmd.arg0("zimsh");
        let out = cmd
            .arg(format!("./{script}"))
            .current_dir(&dir)
            .stdin(Stdio::null())
            .output()
            .expect("spawn script form");
        assert_eq!(out.status.code(), Some(2), "{script}");
        assert_eq!(stderr_of(&out), expected, "{script}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dash_c_sys_error_bad_interpreter_names_the_invocation() {
    // sys_error (error.c:275-281): the invocation name alone, no
    // line segment, then the script path and the interpreter.
    let dir = fixture_dir("badinterp");
    write_bad_interpreter(&dir);
    let mut cmd = rubash();
    cmd.arg0("zimsh");
    let out = cmd
        .args(["-c", "exec ./badinterp.sh"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("spawn -c bad interpreter");
    // GNU classifies the bad-interpreter refusal as 126 (the file
    // was found and executable but could not be executed); 127 is
    // the not-found class.
    assert_eq!(out.status.code(), Some(126));
    assert_eq!(
        stderr_of(&out),
        format!(
            "zimsh: {}/badinterp.sh: /nonexistent-interp-3256: bad interpreter: No such file or directory\n",
            dir.display()
        )
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn builtin_error_prefix_derives_every_arm() {
    let prefix = rubash::builtins::set::builtin_error_prefix;

    // Interactive: base_pathname(shell_name) — never the carrier
    // verbatim (the pre-migration face printed the full path).
    let mut env = HashMap::new();
    env.insert("__RUBASH_INTERACTIVE".to_string(), "1".to_string());
    env.insert("__RUBASH_SHELL_NAME".to_string(), "/a/b/zimsh".to_string());
    assert_eq!(prefix(&env), "zimsh: ");

    // Line only: the name is the $0 carrier verbatim (BASH_ARGV0),
    // not a hard-coded brand literal.
    let mut env = HashMap::new();
    env.insert("__RUBASH_CURRENT_LINE".to_string(), "2".to_string());
    env.insert("BASH_ARGV0".to_string(), "/opt/zimsh".to_string());
    assert_eq!(prefix(&env), "/opt/zimsh: line 2: ");

    // No carriers at all: the chain's terminal fallback is GNU's
    // PROGRAM, "bash".
    assert_eq!(prefix(&HashMap::new()), "bash: ");

    // Script + line, and the eval frame's segment (the one arm the
    // chain has no counterpart for, kept by the face itself).
    let mut env = HashMap::new();
    env.insert("__RUBASH_SCRIPT_NAME".to_string(), "s.sh".to_string());
    env.insert("__RUBASH_CURRENT_LINE".to_string(), "5".to_string());
    assert_eq!(prefix(&env), "s.sh: line 5: ");
    env.insert("__RUBASH_EVAL_CONTEXT".to_string(), "1".to_string());
    assert_eq!(prefix(&env), "s.sh: eval: line 5: ");

    // A stamped frame file outranks the top-level script name (the
    // BASH_SOURCE[0] pairing, rubash#3035).
    let mut env = HashMap::new();
    env.insert("__RUBASH_SCRIPT_NAME".to_string(), "top.sh".to_string());
    env.insert("__RUBASH_CURRENT_FILE".to_string(), "def.sh".to_string());
    env.insert("__RUBASH_CURRENT_LINE".to_string(), "5".to_string());
    assert_eq!(prefix(&env), "def.sh: line 5: ");
}

#[test]
fn interactive_alias_reports_basename_without_a_line_segment() {
    // GNU (renamed matrix R2): zimsh: alias: -z: invalid option.
    // The pre-migration face printed the hard-coded name WITH a
    // line segment: rubash: line 1: alias: -z: invalid option.
    let dir = fixture_dir("inter-alias");
    let stderr = interactive_arg0("alias -z\nexit\n", &dir);
    assert!(
        stderr.contains("zimsh: alias: -z: invalid option"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("rubash: line 1: alias"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interactive_path_invocation_reports_basename_for_lib_faces() {
    // GNU (renamed matrix R3): the interactive name is the
    // basename even when the shell was invoked by full path. The
    // pre-migration faces printed the shell-name carrier verbatim:
    // <abspath>/zimsh: export: -z: invalid option.
    let dir = fixture_dir("inter-path");
    let stderr = interactive_path_copy("export -z\nreadonly -z\nunset -z\nexit\n", &dir);
    for expected in [
        "zimsh: export: -z: invalid option",
        "zimsh: readonly: -z: invalid option",
        "zimsh: unset: -z: invalid option",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in: {stderr}"
        );
    }
    assert!(
        !stderr.contains("/zimsh: export"),
        "verbatim path leaked: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interactive_exec_and_sys_error_report_the_basename() {
    // GNU (renamed matrix R4): zimsh: exec: -z: invalid option /
    // zimsh: /nonexistent-xyz-3256: No such file or directory /
    // zimsh: <abspath>/badinterp.sh: <interp>: bad interpreter: ...
    // The pre-migration faces printed the hard-coded name.
    let dir = fixture_dir("inter-exec");
    write_bad_interpreter(&dir);
    let stderr = interactive_arg0(
        "exec -z\nexec /nonexistent-xyz-3256\nexec ./badinterp.sh\nexit\n",
        &dir,
    );
    for expected in [
        "zimsh: exec: -z: invalid option",
        "zimsh: /nonexistent-xyz-3256: No such file or directory",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in: {stderr}"
        );
    }
    let badinterp = format!(
        "zimsh: {}/badinterp.sh: /nonexistent-interp-3256: bad interpreter: No such file or directory",
        dir.display()
    );
    assert!(
        stderr.contains(&badinterp),
        "missing {badinterp:?} in: {stderr}"
    );
    assert!(
        !stderr.contains("rubash: exec: -z"),
        "literal name leaked: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interactive_cd_sys_error_reports_the_basename() {
    // The sys_error renderer's interactive arm, through cd's face:
    // GNU: zimsh: cd: /nonexistent-xyz-3256: No such file or directory.
    // Path-invoked, the pre-migration face printed the full path.
    let dir = fixture_dir("inter-cd");
    let stderr = interactive_path_copy("cd /nonexistent-xyz-3256\nexit\n", &dir);
    assert!(
        stderr.contains("zimsh: cd: /nonexistent-xyz-3256: No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("/zimsh: cd:"),
        "verbatim path leaked: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interactive_timeformat_reports_the_basename() {
    // The builtin_error_prefix face's production consumer is the
    // TIMEFORMAT diagnostic (executor read_helpers). GNU:
    // zimsh: TIMEFORMAT: `Q': invalid format character. The
    // pre-migration face printed the full invocation path.
    // (The assignment-prefix form `TIMEFORMAT=... time ...` never
    // reaches the timing printer in either shell — the format is
    // set in its own statement first.)
    let dir = fixture_dir("inter-timeformat");
    let stderr = interactive_path_copy("TIMEFORMAT='%Q'\ntime sleep 0.01\nexit\n", &dir);
    assert!(
        stderr.contains("zimsh: TIMEFORMAT: `Q': invalid format character"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("/zimsh: TIMEFORMAT"),
        "verbatim path leaked: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
