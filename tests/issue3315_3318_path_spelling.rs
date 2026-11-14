//! Issues rubash#3315 + rubash#3318 — path-spelling faces of the exec
//! builtin family.
//!
//! #3315: a plain external command invoked by a slash path must keep the
//! typed word as the child's argv[0] — GNU shell_execve (execute_cmd.c)
//! hands execve the command word verbatim, so `/bin/cat /nonexistent`
//! prints `/bin/cat: ...`. Rubash's in-process cat emulation hardcoded
//! `cat:` for its operand diagnostics; the prefix is now the typed word
//! (the same argv[0] the spawn funnel's `arg0` already carries).
//!
//! #3318: `$BASH` must be the INVOCATION path — GNU variables.c:776
//! get_bash_name keeps an absolute argv[0] verbatim (symlinks kept),
//! PWD-joins a `./` spelling, and PATH-walks a bare name to the
//! as-written join. Rubash seeded it from std::env::current_exe, which
//! canonicalizes symlinks, so a `/tmp/linkbash` invocation reported the
//! resolved binary path.

use std::process::Command;

fn rubash() -> &'static str {
    env!("CARGO_BIN_EXE_rubash")
}

/// #3315: the typed slash path is the emulated cat's diagnostic argv[0].
#[test]
fn emulated_cat_diagnostic_uses_typed_path() {
    let output = Command::new(rubash())
        .arg("-c")
        .arg("/bin/cat /nonexistent-xyz")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("/bin/cat: /nonexistent-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
}

/// #3315 guard: the bare-name spelling still prints `cat:`.
#[test]
fn emulated_cat_diagnostic_uses_bare_name() {
    let output = Command::new(rubash())
        .arg("-c")
        .arg("cat /nonexistent-xyz")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("cat: /nonexistent-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
}

/// #3318: invoking the binary through a symlink keeps the LINK as $BASH;
/// a `./` spelling PWD-joins; a PATH-found bare name reports the join.
#[test]
fn bash_var_keeps_invocation_path() {
    let dir = std::env::temp_dir().join("rubash3318");
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("linkbash3318");
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(rubash(), &link).unwrap();
    #[cfg(not(unix))]
    std::fs::copy(rubash(), &link).unwrap();

    // Absolute symlink invocation.
    let output = Command::new(&link)
        .arg("-c")
        .arg("echo $BASH")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        link.to_string_lossy(),
        "absolute symlink: stdout={stdout:?}"
    );

    // `./`-spelling invocation: $PWD + /linkbash3318.
    let output = Command::new("./linkbash3318")
        .arg("-c")
        .arg("echo $BASH")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("{}/linkbash3318", dir.to_string_lossy()),
        "./-spelling: stdout={stdout:?}"
    );

    // PATH-found bare name: the join as written.
    let output = Command::new("env")
        .arg(format!(
            "PATH={}:{}",
            dir.to_string_lossy(),
            "/usr/bin:/bin"
        ))
        .arg("linkbash3318")
        .arg("-c")
        .arg("echo $BASH")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("{}/linkbash3318", dir.to_string_lossy()),
        "PATH-found: stdout={stdout:?}"
    );
}
