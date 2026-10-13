//! unixwin/rubash#592 — POSIX path-operand pre-translation completeness.
//!
//! The pre-#592 argv funnel handed drive-form operands (`/d/...`) to
//! POSIX-aware children verbatim and relied on each tool resolving them
//! itself. The shipped toolchain broke that premise three ways: sed/du had
//! no operand resolution at all (WinuxCmd#1178), niugit existence-gates it
//! (`git -C /d/existing` worked while `worktree add /d/new` silently landed
//! at `D:\d\new`), and the niubash host masked the gap only for top-level
//! commands — so the SAME command drifted by execution context (issue
//! title: sed/du fail inside for-loops/subshells, fine at top level).
//!
//! The contract now (docs/windows-path-contract.md L2/L4): the ENGINE
//! translates path-shaped operands for EVERY external child class,
//! regardless of execution context (top level / loop / subshell / function
//! / script / -c), argument position (positional operand, flag-consumed
//! value, to-be-created target) and target existence — with the grammar
//! exemptions of the four data-primary tools (grep/sed/awk/find) staying
//! verbatim (invariant C, niubash#62) and `__RUBASH_ARGV_DIALECT=legacy`
//! reverting the widening.
//!
//! Layers pinned here (live, real children):
//! ① the context × position × existence matrix (sed operand, tee target);
//! ② the verbatim exemption对照 (grep -F pattern, sed address script,
//!    find -name value, env value) — data operands keep their bytes;
//! ③ the legacy bypass regression (existing translates, missing verbatim);
//! ④ one real `git worktree add <posix new path>` end-to-end (the incident
//!    that widened the issue scope).
//!
//! The funnel-level twins of ①/②/③ live in `src/executor/path.rs` tests
//! (`drive_form_arguments_translate_for_the_posix_aware_child`,
//! `data_primary_operands_stay_verbatim_for_the_applet`,
//! `legacy_dialect_reverts_drive_form_translation_for_the_applet`) and run
//! without a toolchain; the units here need the installed winuxcmd tree
//! (WINUXCMD_HOME) and skip with a note elsewhere (CI).
//!
//! The contract's verification target IS the Windows-native behavior: the
//! file is `#![cfg(windows)]`-gated as a whole. unix CI only verifies
//! compilation (the file cfg-strips to an empty test binary there).

#![cfg(windows)]

use std::path::{Path, PathBuf};
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

fn run_script_file(
    script_path: &Path,
    extra_env: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg(script_path);
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

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-592-{tag}-{}", std::process::id()));
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

/// env with the installed toolchain first on PATH (the tools must resolve
/// through the same PATH walk the engine uses for children).
fn tool_path_env(home: &str) -> String {
    let bin = Path::new(home).join("usr").join("bin");
    let bin_str = bin.to_string_lossy().replace('\\', "/");
    format!("{bin_str};{}", std::env::var("PATH").unwrap_or_default())
}

/// ① The context × position × existence matrix. sed is the reported tool
/// (no operand self-resolution in the shipped 1.1.5): every execution
/// context must feed it the translated Windows form — an existing operand
/// prints the line, a missing operand's error names the `D:\...` spelling
/// (proving the form without depending on tool success). The to-create
/// target leg uses `tee`, whose operand is a file it creates.
#[test]
fn posix_operand_matrix_context_position_existence() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let scratch = scratch_dir("matrix");
    let posix = to_posix_form(&scratch);
    std::fs::write(scratch.join("in.txt"), b"line1\n").unwrap();
    let tool_path = tool_path_env(&home);
    let env = [("PATH", tool_path.as_str())];

    // The same operand across every execution context: top level, for
    // loop, subshell, brace group, function, pipeline, command
    // substitution.
    let contexts: &[(&str, &str)] = &[
        ("top-level", "sed -n 1p '{p}/in.txt'"),
        ("for-loop", "for i in 1; do sed -n 1p '{p}/in.txt'; done"),
        ("subshell", "(sed -n 1p '{p}/in.txt')"),
        ("brace-group", "{ sed -n 1p '{p}/in.txt'; }"),
        ("function", "f592() { sed -n 1p '{p}/in.txt'; }; f592"),
        ("pipeline", "sed -n 1p '{p}/in.txt' | cat"),
        (
            "command-substitution",
            "x=$(sed -n 1p '{p}/in.txt'); echo \"got:$x\"",
        ),
    ];
    for (name, template) in contexts {
        let (code, stdout, stderr) = run_rubash(&template.replace("{p}", &posix), &env);
        assert_eq!(
            code,
            Some(0),
            "context {name}: sed must receive the translated operand — \
             stdout: {stdout:?} stderr: {stderr:?}"
        );
        assert!(
            stdout.contains("line1"),
            "context {name}: operand did not resolve (stdout {stdout:?}, stderr {stderr:?})"
        );
    }

    // Script FILE context (the driver shape docs/windows-path-contract.md
    // §5.3 recommends): the same translation must hold without -c.
    let script = scratch.join("matrix.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nfor i in 1; do sed -n 1p '{posix}/in.txt'; done\n"),
    )
    .unwrap();
    let (code, stdout, stderr) = run_script_file(&script, &env);
    assert_eq!(
        code,
        Some(0),
        "script-file context: stdout {stdout:?} stderr {stderr:?}"
    );
    assert!(stdout.contains("line1"), "script-file stdout {stdout:?}");

    // Existence leg: a MISSING operand must still be TRANSLATED — sed's
    // error echoes the form it received, and it must be the Windows
    // spelling in every context (the pre-#592 behavior echoed `/d/...`).
    for (name, template) in [
        ("top-level", "sed -n 1p '{p}/missing-592'"),
        (
            "for-loop",
            "for i in 1; do sed -n 1p '{p}/missing-592'; done",
        ),
    ] {
        let (code, _stdout, stderr) = run_rubash(&template.replace("{p}", &posix), &env);
        assert_eq!(code, Some(2), "context {name}: sed rc");
        let native_form = scratch.join("missing-592").to_string_lossy().into_owned();
        assert!(
            stderr.contains(&native_form),
            "context {name}: missing operand must translate (received form in \
             stderr {stderr:?}, want {native_form:?})"
        );
    }

    // Position leg — to-be-created target: tee's operand is a file it
    // creates. Pre-#592 it landed verbatim under a tool without operand
    // resolution; it must land at the POSIX-named location.
    let (code, _stdout, stderr) = run_rubash(
        &format!("for i in 1; do echo created-592 | tee '{posix}/created.txt'; done"),
        &env,
    );
    assert_eq!(code, Some(0), "tee rc: {stderr:?}");
    assert!(
        scratch.join("created.txt").is_file(),
        "to-create target must land at the POSIX-named location"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.join("created.txt")).unwrap(),
        "created-592\n"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

/// ② The verbatim exemption对照: the data primaries of the grammar tools
/// (and the filesystem-blind string transforms) keep their bytes while the
/// real path operands translate.
#[test]
fn data_primaries_stay_verbatim_while_paths_translate() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let scratch = scratch_dir("verbatim");
    let posix = to_posix_form(&scratch);
    std::fs::write(scratch.join("data.txt"), b"the /d/x marker\nother\n").unwrap();
    let tool_path = tool_path_env(&home);
    let env = [("PATH", tool_path.as_str())];

    // grep: the pattern operand is DATA — a `/d/x`-shaped pattern must
    // match the literal text (translation would corrupt it to `D:\x`),
    // while the file operand translates and resolves.
    let (code, stdout, stderr) = run_rubash(&format!("grep -F -c '/d/x' '{posix}/data.txt'"), &env);
    assert_eq!(code, Some(0), "grep rc: {stderr:?}");
    assert_eq!(stdout, "1\n", "grep -F '/d/x' must match the literal text");

    // sed: the script operand is DATA — `/n/d` is an address (delete lines
    // matching `n`), never a drive path.
    let (code, stdout, stderr) = run_rubash("printf 'n1\\nx\\n' | sed /n/d", &env);
    assert_eq!(code, Some(0), "sed rc: {stderr:?}");
    assert_eq!(stdout, "x\n", "sed address script must stay verbatim");

    // find: the start operand translates (the path position find cannot
    // resolve itself, WinuxCmd#1170) while a `-name` pattern value with a
    // drive-shaped form stays data.
    let (code, stdout, stderr) = run_rubash(
        &format!("find '{posix}' -maxdepth 1 -name 'data.txt' | grep -q data.txt"),
        &env,
    );
    assert_eq!(code, Some(0), "find start rc: {stderr:?}");
    // A `/d`-shaped `-name` value is DATA: find completes with no match and
    // no complaint (the funnel-level pin
    // data_primary_operands_stay_verbatim_for_the_applet locks the bytes).
    let (code, _stdout, stderr) = run_rubash(
        &format!("find '{posix}' -maxdepth 1 -name '/d*' > /dev/null"),
        &env,
    );
    assert_eq!(code, Some(0), "find -name '/d*' smoke rc: {stderr:?}");

    // dirname/basename: filesystem-blind string transforms (GNU
    // dirname.c/basename.c never stat their operand) — the GNU string
    // contract pins the POSIX display text (issue71).
    let (code, stdout, stderr) = run_rubash(
        &format!("echo \"$(dirname /a/b/c.txt)|$(basename /a/b/c.txt .txt)\""),
        &env,
    );
    assert_eq!(code, Some(0), "dirname/basename rc: {stderr:?}");
    assert_eq!(stdout, "/a/b|c\n", "string transforms must stay literal");

    // env: assignment VALUES are the env channel — verbatim by contract
    // (§4.5), never operand-translated.
    let (code, stdout, stderr) =
        run_rubash(&format!("env MK592={posix}/x cmd /c \"set MK592\""), &env);
    assert_eq!(code, Some(0), "env rc: {stderr:?}");
    assert!(
        stdout.contains(&format!("MK592={posix}/x")),
        "env value must cross verbatim, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

/// ③ The `__RUBASH_ARGV_DIALECT=legacy` bypass regression: the hatch
/// reverts the whole #592 widening — an existing drive target still
/// translates (the native funnel's gate), a missing one stays verbatim
/// (sed's error must echo the POSIX form).
#[test]
fn legacy_dialect_bypass_regression() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let scratch = scratch_dir("legacy");
    let posix = to_posix_form(&scratch);
    std::fs::write(scratch.join("in.txt"), b"line1\n").unwrap();
    let tool_path = tool_path_env(&home);

    let (code, stdout, stderr) = run_rubash(
        &format!(
            "export __RUBASH_ARGV_DIALECT=legacy; \
             for i in 1; do sed -n 1p '{posix}/in.txt'; done"
        ),
        &[("PATH", tool_path.as_str())],
    );
    assert_eq!(code, Some(0), "legacy existing rc: {stderr:?}");
    assert!(
        stdout.contains("line1"),
        "legacy existing stdout {stdout:?}"
    );

    let (code, _stdout, stderr) = run_rubash(
        &format!(
            "export __RUBASH_ARGV_DIALECT=legacy; \
             for i in 1; do sed -n 1p '{posix}/missing-592'; done"
        ),
        &[("PATH", tool_path.as_str())],
    );
    assert_eq!(code, Some(2), "legacy missing rc");
    // The legacy gate is the pre-#592 native funnel's: only NON-c drive
    // forms existence-gate (`/c/...` translates unconditionally in every
    // dialect). On a C:-temp host the missing operand therefore still
    // echoes translated; on any other drive it must echo verbatim.
    let drive_letter = posix.as_bytes()[1] as char;
    let native_missing = scratch.join("missing-592").to_string_lossy().into_owned();
    if drive_letter == 'c' {
        assert!(
            stderr.contains(&native_missing),
            "legacy /c/... missing operand translates in every dialect, stderr {stderr:?}"
        );
    } else {
        assert!(
            stderr.contains(&format!("{posix}/missing-592")),
            "legacy missing operand must stay verbatim, stderr {stderr:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&scratch);
}

/// ④ The incident end-to-end: `git -C <repo> worktree add <posix new path>`
/// must create the worktree at the POSIX-named location — pre-#592 the
/// verbatim operand reached niugit, which resolved it current-drive-root
/// relative and silently created `D:\d\...`.
#[test]
fn git_worktree_add_to_be_created_path_end_to_end() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let git = Path::new(&home).join("usr").join("bin").join("git.exe");
    if !git.is_file() {
        eprintln!("skipping: no git.exe under {home}");
        return;
    }
    let tool_path = tool_path_env(&home);
    let env = [
        ("PATH", tool_path.as_str()),
        ("GIT_AUTHOR_NAME", "t"),
        ("GIT_AUTHOR_EMAIL", "t@t"),
        ("GIT_COMMITTER_NAME", "t"),
        ("GIT_COMMITTER_EMAIL", "t@t"),
    ];
    let repo = scratch_dir("wt-repo");
    let repo_posix = to_posix_form(&repo);
    let wt = scratch_dir("wt-new-592");
    let wt_posix = to_posix_form(&wt);
    let (code, _stdout, stderr) = run_rubash(
        &format!(
            "cd '{repo_posix}' || exit 9; \
             git init -q -b main . || exit 9; \
             echo a > a.txt; git add a.txt; git commit -q -m init || exit 9; \
             git branch wtbranch-592 || exit 9; \
             git worktree add '{wt_posix}' wtbranch-592 || exit 9"
        ),
        &env,
    );
    assert_eq!(code, Some(0), "worktree add rc: {stderr:?}");
    // The POSIX-named location (same dir `to_posix_form` folded) must hold
    // the new worktree...
    assert!(
        wt.join("a.txt").is_file(),
        "worktree must be created at {wt:?} — the operand was not translated \
         (pre-#592 it landed drive-root relative, e.g. D:\\d\\...)"
    );
    assert!(wt.join(".git").is_file(), "worktree .git missing");

    // The registered worktree list must name the POSIX-named location too.
    let (code, stdout, stderr) = run_rubash(
        &format!("git -C '{repo_posix}' worktree list --porcelain"),
        &env,
    );
    assert_eq!(code, Some(0), "worktree list rc: {stderr:?}");
    assert!(
        stdout.contains(&wt.to_string_lossy().replace('\\', "/")),
        "worktree list must name {wt:?}, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&wt);
}
