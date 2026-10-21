//! unixwin/niubash#481 regression (engine side): a bare drive designator
//! in command position (`D:`) and as a `cd` operand (`cd D:`) were
//! rewritten by the product REPL's fast-path AST normalizer only — every
//! other read route (the interactive grouped reader behind the batch
//! gate: `D:; pwd`, `cd D: && pwd`; the engine's piped `-i` stdin driver
//! — niubash#483; nested contexts like function bodies and `eval`) handed
//! the bare word to the executor, which reported `D:: command not found`
//! / `cd: D:: No such file or directory`. Same submission, two outcomes
//! depending on which reader path the host gate picked.
//!
//! The fix moves the semantics into the engine's shared dispatch:
//! `execute_command` rewrites a bare `X:` node to the same `cd X:/` the
//! host pass emits (executor/windows_drive.rs), and `cd`'s operand
//! resolution folds the bare `X:` designator to the `/x/` drive root the
//! in-shell path funnel already accepts (builtins/cd.rs
//! `windows_bare_drive_operand`). `$PWD` keeps the canonical POSIX
//! persona spelling `/d/` (executor/path.rs `shell_pwd_display_path`
//! bare-root arm — `cd D:/` previously stranded `D:` into $PWD).
//!
//! Oracle: there is no GNU anchor (no drive letters upstream); the
//! contract is the Windows persona the host normalizer defines — both
//! read routes must agree byte-for-byte, and the exclusions the host
//! pass enforces (quoted `"D:"`, `D:` with arguments/redirects/
//! assignments, pipeline membership, `!`/`&`/`time` wrappers) must keep
//! GNU's command-not-found shape.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash(script: &str) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("spawn rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Run `rubash -i` with piped stdin — the engine-side stdin driver that
/// niubash#483 reports skips every host normalizer.
fn run_rubash_piped_i(lines: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(lines.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash -i");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Drive letter of the temp volume in both spellings (`d` / `D:`).
fn temp_drive() -> (String, String) {
    let win = std::env::temp_dir().to_string_lossy().replace('\\', "/");
    let letter = win.as_bytes()[0] as char;
    (
        letter.to_ascii_lowercase().to_string(),
        format!("{letter}:"),
    )
}

/// POSITIVE: `D:` and `cd D:` grouped with `;`/`&&` take the same drive
/// switch the normalized fast path produces — PWD lands on the canonical
/// `/d/` persona spelling, matching the host-side rewrite's end state.
#[test]
fn bare_drive_command_and_cd_operand_succeed_on_grouped_text() {
    let (lower, drive) = temp_drive();
    let expected_root = format!("/{lower}/");

    let (code, stdout, stderr) = run_rubash(&format!("{drive}; pwd"));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected_root, "D:; pwd -> {stdout:?}");

    let (code, stdout, stderr) = run_rubash(&format!("{drive} && pwd"));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected_root, "D: && pwd -> {stdout:?}");

    let (code, stdout, stderr) = run_rubash(&format!("cd {drive}; pwd"));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected_root, "cd D:; pwd -> {stdout:?}");

    let (code, stdout, stderr) = run_rubash(&format!("cd {drive} && pwd"));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(
        stdout.trim_end(),
        expected_root,
        "cd D: && pwd -> {stdout:?}"
    );
}

/// POSITIVE: `$PWD` records the canonical POSIX drive-root spelling after
/// a bare-drive switch — including through a function body, the nested
/// context the host's AST pass never descended into — and `cd D:` keeps
/// `OLDPWD` honest. The piped `-i` stdin driver (niubash#483) resolves
/// the same operands the `-c` route does.
#[test]
fn bare_drive_switch_updates_pwd_to_posix_persona_everywhere() {
    let (lower, drive) = temp_drive();
    let expected_root = format!("/{lower}/");

    let (code, stdout, stderr) = run_rubash(&format!(
        "cd '{expected_root}repo' 2>/dev/null || cd '{expected_root}'; {drive}; echo PWD=$PWD"
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stdout.contains(&format!("PWD={expected_root}")),
        "bare {drive} left $PWD as {stdout:?} — POSIX persona contract broken"
    );

    let (code, stdout, stderr) = run_rubash(&format!(
        "cd '{expected_root}repo' 2>/dev/null || cd '{expected_root}'; f() {{ {drive}; }}; f; echo PWD=$PWD"
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stdout.contains(&format!("PWD={expected_root}")),
        "{drive} inside a function body left $PWD as {stdout:?}"
    );

    // `cd D:` from a known directory: success, PWD=/d/, OLDPWD intact.
    let (code, stdout, stderr) = run_rubash(&format!(
        "cd '{expected_root}repo' 2>/dev/null || cd '{expected_root}'; cd {drive}; echo PWD=$PWD OLDPWD=$OLDPWD"
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stdout.contains(&format!("PWD={expected_root} OLDPWD=")),
        "cd {drive} state: {stdout:?}"
    );

    // The piped `-i` face (niubash#483): same operands, same result.
    let (_code, stdout, stderr) =
        run_rubash_piped_i(&format!("cd {drive}\npwd\n{drive}\npwd\nexit\n"));
    let pwds: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with('/'))
        .collect();
    assert!(
        pwds.iter().all(|line| *line == expected_root) && pwds.len() >= 2,
        "piped -i drive-switch pwds: {stdout:?} stderr: {stderr:?}"
    );
}

/// POSITIVE residual: `cd D:/` — the literal rewrite product the host
/// pass emits — must also land the canonical `/d/` display, not strand
/// the designator `D:` in `$PWD` (the pre-fix engine dropped the root
/// separator while normalizing the display path).
#[test]
fn cd_slashed_drive_root_renders_posix_pwd() {
    let (lower, drive) = temp_drive();
    let expected_root = format!("/{lower}/");

    let (code, stdout, stderr) = run_rubash(&format!("cd {drive}/; echo PWD=$PWD; pwd"));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stdout.contains(&format!("PWD={expected_root}")),
        "cd {drive}/ rendered {stdout:?} — expected {expected_root}"
    );
}

/// NEGATIVE: the exclusions the host pass enforces must keep GNU's
/// command-not-found shape — a quoted `"D:"`, `D:` with an argument, an
/// assignment-prefixed `D:`, and `D:` inside a pipeline are command-name
/// data, not the drive-switch shorthand. A bare `D:` must never leak into
/// cwd state through these spellings.
#[test]
fn drive_designator_exclusions_keep_command_not_found() {
    let (lower, drive) = temp_drive();
    let base = format!("/{lower}/");

    // Every excluded shape must print the diagnostic, report the stage's
    // own status, and leave cwd on `base`. `D: | cat` still reports rc=0:
    // the pipeline status is the LAST stage's (cat), exactly like GNU —
    // the assertion key is the diagnostic plus the unmoved cwd.
    for (script, tag, expect_127) in [
        (
            format!("cd '{base}'; \"{drive}\"; echo rc=$?; pwd"),
            "quoted",
            true,
        ),
        (
            format!("cd '{base}'; {drive} arg; echo rc=$?; pwd"),
            "arg",
            true,
        ),
        (
            format!("cd '{base}'; V=1 {drive}; echo rc=$?; pwd"),
            "assign",
            true,
        ),
        (
            format!("cd '{base}'; {drive} | cat; echo rc=$?; pwd"),
            "pipe-out",
            false,
        ),
        (
            format!("cd '{base}'; true | {drive}; echo rc=$?; pwd"),
            "pipe-in",
            true,
        ),
    ] {
        let (_code, stdout, stderr) = run_rubash(&script);
        assert!(
            stderr.contains("command not found"),
            "{tag}: expected command-not-found diagnostic, stdout={stdout:?} stderr={stderr:?}"
        );
        if expect_127 {
            assert!(
                stdout.contains("rc=127"),
                "{tag}: expected status 127, stdout={stdout:?}"
            );
        }
        assert!(
            stdout.lines().last().is_some_and(|line| line == base),
            "{tag}: cwd moved off {base}: {stdout:?}"
        );
    }
}
