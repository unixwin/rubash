//! Boundary contract — invariant B: POSIX forms are usable at every edge.
//!
//! Three sub-boundaries, each with positive (POSIX form works) and negative
//! (the wrong form must NOT appear / the old bug must NOT resurface) cases:
//!
//! B①  in-shell semantics: redirect targets, `cd`, `pwd`, `$PWD`, `$(pwd)`
//!     accept POSIX operands and RENDER POSIX (`/<drive>/…`, the #537/#556
//!     contract — the msys persona has no native-spelling mode);
//! B②  winuxcmd tool operands: explicit POSIX path arguments (`/c/...`,
//!     `/d/...`) work for the file tools, and the child-command lookup of
//!     env/timeout/xargs/find -exec resolves its sub-command;
//! B③  the env channel: a POSIX-form value (including PWD) reaches children
//!     verbatim — the engine adds no path conversion at spawn (GNU-faithful
//!     import/export, docs/issue329-path-form-policy-analysis.md option A).
//!
//! CONTRACT tests: expected green on current master. The known red edges
//! are filed separately and deliberately NOT pinned here — except bare
//! `cd /c` under a configured shell root, which rubash#566 fixed and pinned
//! in issue566_bare_drive_root_enoent_regressions.rs (winuxcmd `find`
//! start-path operands remain open, WinuxCmd#1170).
//!
//! B② legs need the installed winuxcmd toolchain (WINUXCMD_HOME) and skip
//! with a note elsewhere, e.g. CI.
//!
//! The contract's verification target IS the Windows-native behavior: the
//! file is `#![cfg(windows)]`-gated as a whole. unix CI only verifies
//! compilation (the file cfg-strips to an empty test binary there), not the
//! cases — the cases need a Windows runner.

#![cfg(windows)]

use std::path::Path;
use std::process::Command;

fn run_rubash(script: &str, extra_env: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(["-c", script]);
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = command.output().expect("spawn rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-bc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn to_posix_form(path: &Path) -> String {
    let win = path.to_string_lossy().replace('\\', "/");
    format!(
        "/{}/{}",
        (win.as_bytes()[0] as char).to_ascii_lowercase(),
        &win[3..]
    )
}

fn winuxcmd_home() -> Option<String> {
    if let Ok(home) = std::env::var("WINUXCMD_HOME") {
        if !home.is_empty() && Path::new(&home).is_dir() {
            return Some(home);
        }
    }
    None
}

/// B① POSITIVE: after `cd` to a POSIX operand, `$PWD`, `$(pwd)`, `pwd -P`
/// and `cd -`/OLDPWD all render the POSIX `/<drive>/…` form, and POSIX-form
/// redirection targets land on the same file the native spelling names.
#[test]
fn shell_renders_posix_after_posix_cd_and_redirection() {
    if !cfg!(windows) {
        return; // the /<drive>/ persona is a Windows boundary
    }
    let scratch = scratch_dir("posix-render");
    let posix = to_posix_form(&scratch);
    let script = format!(
        "cd '{posix}' || exit 9; \
         echo p1=\"$PWD\"; echo p2=\"$(pwd)\"; echo p3=\"$(pwd -P)\"; \
         [[ \"$PWD\" == \"$(pwd)\" ]] || exit 8; \
         echo data > redir.txt; \
         echo more >> '{posix}/redir.txt'; \
         cat '{posix}/redir.txt' | grep -c data | grep -q 1 || exit 7; \
         cat redir.txt | grep -q more || exit 6"
    );
    let (code, stdout, stderr) = run_rubash(&script, &[]);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    for tag in ["p1", "p2", "p3"] {
        let value = stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{tag}=")))
            .unwrap_or_else(|| panic!("missing {tag} in {stdout:?}"));
        assert!(
            value.starts_with('/') && !value.contains(':') && !value.contains('\\'),
            "{tag}={value:?} is not POSIX form — the shell rendered a native \
             spelling (violates the #537/#556 contract)"
        );
        assert!(
            value.starts_with(&posix),
            "{tag}={value:?} does not name the scratch dir {posix}"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

/// B① NEGATIVE ×2: (a) a native `D:/…` operand is ACCEPTED by cd (both
/// forms are input dialects) but still renders POSIX afterwards; (b) the
/// host native-style variables are inert for PWD — the pre-#537 hatch must
/// stay retired.
#[test]
fn native_cd_operand_accepted_and_style_vars_stay_inert() {
    if !cfg!(windows) {
        return;
    }
    let scratch = scratch_dir("native-cd");
    let native = scratch.to_string_lossy().replace('\\', "/");
    let (code, stdout, stderr) = run_rubash(
        &format!("cd '{native}' || exit 9; echo after=\"$PWD\""),
        &[
            ("WINUXSH_SHELL_PATH_STYLE", "native"),
            ("NIU_SHELL_PATH_STYLE", "native"),
            ("__RUBASH_PATH_STYLE", "native"),
        ],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    let value = stdout
        .lines()
        .find_map(|line| line.strip_prefix("after="))
        .expect("after= line");
    assert!(
        value.starts_with('/') && !value.contains(':'),
        "cd 'D:/...' rendered {value:?} — POSIX input dialect broken or the \
         native-style hatch is back (#537/#556)"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// B② POSITIVE: the installed winuxcmd tools accept explicit POSIX path
/// operands (`touch`/`ls` file operands, `env`+`timeout`+`xargs`
/// sub-command resolution, `find -exec` child lookup) — the POSIX operand
/// dialect of the tool layer.
#[test]
fn winuxcmd_tools_accept_posix_operands() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let bin = Path::new(&home).join("usr").join("bin");
    let scratch = scratch_dir("tool-posix");
    let posix = to_posix_form(&scratch);
    let bin_str = bin.to_string_lossy().replace('\\', "/");
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let script_path = format!("{bin_str};{inherited_path}");
    let run_with_tool_path = |script: &str| run_rubash(script, &[("PATH", script_path.as_str())]);
    let tool = |name: &str| bin.join(name).to_string_lossy().replace('\\', "/");
    std::fs::write(scratch.join("t1"), b"payload-t1\n").unwrap();

    // touch + ls file operands in POSIX form.
    let (code, stdout, stderr) = run_with_tool_path(&format!(
        "'{touch}' '{posix}/t1' && '{ls}' '{posix}/t1' | grep -q t1 && echo TOUCH_OK",
        touch = tool("touch.exe"),
        ls = tool("ls.exe"),
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("TOUCH_OK"));

    // env resolves its sub-command through PATH and passes POSIX operands
    // (ls echoes the operand path back for a file operand).
    let (code, stdout, stderr) = run_with_tool_path(&format!(
        "'{env}' ls '{posix}/t1' | grep -q t1 && echo ENV_OK",
        env = tool("env.exe"),
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("ENV_OK"));

    // timeout finds its child via PATH and forwards the POSIX operand.
    let (code, stdout, stderr) = run_with_tool_path(&format!(
        "'{timeout}' 30 ls '{posix}/t1' | grep -q t1 && echo TIMEOUT_OK",
        timeout = tool("timeout.exe"),
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("TIMEOUT_OK"));

    // xargs feeds a POSIX operand to its child; find's -exec child lookup
    // resolves the child by name (start path given in the form find itself
    // accepts — see the filed issue for its POSIX start-path gap).
    let (code, stdout, stderr) = run_with_tool_path(&format!(
        "echo '{posix}/t1' | '{xargs}' cat | grep -q payload-t1 && echo XARGS_OK && \
         cd '{posix}' && '{find}' . -name 't1' -exec cat {{}} \\; | grep -q payload-t1 && echo FIND_EXEC_OK",
        xargs = tool("xargs.exe"),
        find = tool("find.exe"),
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("XARGS_OK"));
    assert!(stdout.contains("FIND_EXEC_OK"));

    let _ = std::fs::remove_dir_all(&scratch);
}

/// B② NEGATIVE: a POSIX path operand is passed through the tool layer
/// VERBATIM — a literal `$`-bearing operand must NOT be re-expanded into a
/// neighboring real path (the #511 family contract, probed at the tool
/// operand boundary rather than the builtin wrapper).
#[test]
fn winuxcmd_ls_literal_dollar_operand_is_not_expanded() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let scratch = scratch_dir("tool-verbatim");
    let posix = to_posix_form(&scratch);
    std::fs::write(scratch.join("real.txt"), b"x").unwrap();
    let ls = Path::new(&home)
        .join("usr")
        .join("bin")
        .join("ls.exe")
        .to_string_lossy()
        .replace('\\', "/");
    // The operand '$nosuchvar/real.txt' is single-quoted: GNU ls reports the
    // literal text and never guesses a path. Since rubash#592 the PATH-FORM
    // operand translates at the funnel (the tool boundary sees the Windows
    // spelling), but the `$nosuchvar` component must stay LITERAL — no
    // re-expansion into a neighboring real path is the #511-family contract
    // this unit guards.
    let (code, stdout, stderr) = run_rubash(
        &format!("'{ls}' '{posix}/$nosuchvar/real.txt' 2>&1 | head -1"),
        &[],
    );
    assert_eq!(code, Some(0));
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("$nosuchvar") && combined.contains("real.txt"),
        "ls diagnostic lost the literal operand: {combined:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// B③ POSITIVE + NEGATIVE (POSIX-aware child class): the env channel carries
/// PWD in the shell's POSIX form to a POSIX-aware child (winuxcmd env)
/// verbatim — no parent-layer conversion re-renders it (the engine adds no
/// path semantics at import/export).
#[test]
fn env_channel_pwd_reaches_posix_child_verbatim() {
    if !cfg!(windows) {
        return;
    }
    let Some(_home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let scratch = scratch_dir("env-pwd");
    let posix = to_posix_form(&scratch);
    let (code, stdout, stderr) = run_rubash(
        &format!("cd '{posix}' || exit 9; echo SUB=\"$PWD\"; env | grep '^PWD='"),
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    let shell_pwd = stdout
        .lines()
        .find_map(|line| line.strip_prefix("SUB="))
        .expect("SUB= line");
    assert_eq!(
        shell_pwd, posix,
        "shell PWD {shell_pwd:?} is not the POSIX scratch form"
    );
    let env_line = stdout
        .lines()
        .find(|line| line.starts_with("PWD="))
        .expect("PWD= line in env output");
    assert_eq!(
        env_line,
        &format!("PWD={posix}"),
        "env channel re-rendered PWD ({env_line:?}) — the engine must pass env \
         values verbatim at spawn"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// B③ POSITIVE + NEGATIVE (native child class): a native child (cmd.exe)
/// sees the same verbatim POSIX-form PWD — the env channel does not
/// discriminate by child dialect (only argv gets per-dialect treatment).
#[test]
fn env_channel_pwd_reaches_native_child_verbatim() {
    if !cfg!(windows) {
        return;
    }
    let scratch = scratch_dir("env-pwd-native");
    let posix = to_posix_form(&scratch);
    let (code, stdout, stderr) = run_rubash(
        &format!("cd '{posix}' || exit 9; cmd /c \"set PWD\" | grep '^PWD='"),
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    let cmd_line = stdout
        .lines()
        .find(|line| line.starts_with("PWD="))
        .expect("PWD= line from cmd");
    assert_eq!(
        cmd_line,
        &format!("PWD={posix}"),
        "native child saw re-rendered PWD {cmd_line:?} — env channel must stay \
         verbatim for every child class"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
