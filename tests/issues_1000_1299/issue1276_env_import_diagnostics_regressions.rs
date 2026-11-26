//! rubash#1276 (cosmetic batch): two startup/`env` diagnostic facets.
//!
//! (1) A malformed `BASH_FUNC_*` environment entry must produce
//! GNU's two import warnings; rubash imported-or-dropped it
//! silently. GNU anchor: `variables.c` function-definition import
//! validates the `%%` encoding and reports
//! `warning: f: ignoring function definition attempt` plus
//! `error importing function definition for `f'` before the name
//! simply misses (rc 127).
//!
//! (2) The `env` builtin must run its program with the argv[0] the
//! caller wrote; resolving the program to an absolute path first
//! rewrites argv[0], so a child shell's diagnostics and `$0` show
//! the full path instead of the invoked name. GNU's env (and GNU
//! bash's expectations of it) pass the name through verbatim.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

#[cfg(unix)]
use std::process::Stdio;

/// Facet (1): the two GNU import warnings appear on stderr, nothing
/// is injected, and calling the name is a plain 127.
#[test]
fn malformed_bash_func_import_warns_like_gnu() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("f; echo rc=$?")
        .env("BASH_FUNC_f%%", "() { echo INJECTED; }; echo PWNED")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("warning: f: ignoring function definition attempt"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("error importing function definition for `f'"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("f: command not found"), "stderr: {stderr}");
}

/// Facet (2): `env rbprobe ...` keeps argv[0] = `rbprobe` (the name
/// as written), so the child's `$0` is the bare name, not a resolved
/// absolute path.
#[test]
#[cfg(unix)]
fn env_builtin_passes_argv_zero_verbatim() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("rubash-issue1276-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1276 scratch dir");
    let probe = dir.join("rbprobe");
    std::fs::copy(env!("CARGO_BIN_EXE_rubash"), &probe).expect("copy rubash as rbprobe");
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755))
        .expect("chmod rbprobe");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, "env rbprobe -c 'echo \"zero=$0\"'\n")
        .expect("write probe script");
    let path = format!("{}:/usr/bin:/bin", dir.display());
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env("PATH", path)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "zero=rbprobe\n");
}
