//! Boundary contract — invariant C: verbatim fidelity (byte-exact argv).
//!
//! GNU hands execve the raw word bytes (execute_cmd.c:6119-6127
//! shell_execve) and never rewrites argv; rubash's per-child dialect
//! contract (docs/platform-identity.md) keeps that guarantee on Windows:
//! POSIX-aware children (winuxcmd tree / dispatcher) receive the word bytes
//! VERBATIM, native children get uniform Win32 translation of path-shaped
//! operands only. Nothing may re-expand, re-quote, or re-render:
//!
//! C①  single-quoted file operands reach the builtin wrappers verbatim —
//!     the #511/#555 family (cat/rm/cp/touch/mkdir/rmdir) must never act on
//!     an expansion neighbor of a `'$FIX/x'` operand;
//! C②  git rev specs and branch names (`origin/feat/x`, `HEAD~1..HEAD`,
//!     `HEAD -- path`, slash branches) cross the shell boundary with zero
//!     conversion;
//! C③  message/env channels are byte-exact: `commit -m`-shaped message
//!     bytes survive rubash's argv to a child, and GIT_SSH_COMMAND-style
//!     env values pass through untouched.
//!
//! CONTRACT tests: expected green on current master.
//! The ② leg uses the installed niugit (WINUXCMD_HOME) and skips elsewhere.
//! Note for auditors: the deployed `opt/niugit/git.exe` LAUNCHER drops `"`
//! bytes from argv (filed issue); the rubash-side fidelity this file pins
//! is proven against children that decode argv correctly.
//!
//! The contract's verification target IS the Windows-native behavior: the
//! file is `#![cfg(windows)]`-gated as a whole. unix CI only verifies
//! compilation (the file cfg-strips to an empty test binary there), not the
//! cases — the cases need a Windows runner.

#![cfg(windows)]

use std::path::Path;
use std::process::Command;

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

/// C① POSITIVE + NEGATIVE (the #511/#555 family): a single-quoted
/// `'$FIX/x'` operand must fail on the LITERAL name with GNU's own
/// diagnostic, and must never touch the expansion neighbor at `$FIX/x`.
/// Probed across cat/rm/cp/touch — the silent-wrong-file class.
#[test]
fn single_quoted_operands_reach_wrappers_verbatim() {
    if !cfg!(windows) {
        return; // the quoted-operand wrappers behave POSIX-identically elsewhere
    }
    let scratch = scratch_dir("verbatim-family");
    let posix = to_posix_form(&scratch);
    // The neighbor tree the bug class would silently damage.
    std::fs::create_dir_all(scratch.join("zz")).unwrap();
    std::fs::write(scratch.join("zz").join("c"), b"WRONG").unwrap();
    std::fs::write(scratch.join("zz").join("r"), b"KEEP").unwrap();
    std::fs::write(scratch.join("zz").join("src"), b"KEEP").unwrap();
    std::fs::write(scratch.join("zz").join("t"), b"KEEP").unwrap();

    let script = format!(
        "FIX='{posix}/zz'; \
         out=$(cat '$FIX/c' 2>&1); rc=$?; \
         [ $rc -eq 1 ] || exit 11; \
         case \"$out\" in *'$FIX/c'*) ;; *) exit 12;; esac; \
         rm '$FIX/r' 2>/dev/null; [ $? -eq 1 ] || exit 13; \
         [ -f \"$FIX/r\" ] || exit 14; \
         cp '$FIX/src' '$FIX/dst' 2>/dev/null; [ $? -eq 1 ] || exit 15; \
         [ ! -e \"$FIX/dst\" ] || exit 16; \
         touch '$FIX/t' 2>/dev/null; [ $? -eq 1 ] || exit 17; \
         [ \"$(cat \"$FIX/t\")\" = KEEP ] || exit 18; \
         echo FAMILY_OK"
    );
    let (code, stdout, stderr) = run_rubash(&script);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("FAMILY_OK"));
    // Negative half in the parent: the neighbors really are intact.
    assert_eq!(
        std::fs::read_to_string(scratch.join("zz").join("r")).unwrap(),
        "KEEP",
        "rm acted on the expansion neighbor — #555 class regression"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.join("zz").join("c")).unwrap(),
        "WRONG",
        "cat read the expansion neighbor — #511 regression"
    );
    assert!(
        !scratch.join("zz").join("dst").exists(),
        "cp created the expansion neighbor"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// C② POSITIVE: git rev specs and branch names cross the shell boundary
/// with zero conversion — `origin/feat/x` is a rev (not a path), a range
/// `HEAD~1..HEAD`, a `--`-separated rev, and a slash branch all resolve.
/// Uses the installed niugit; skips when the toolchain is absent.
#[test]
fn git_rev_specs_and_branch_names_survive_verbatim() {
    if !cfg!(windows) {
        return;
    }
    let Some(home) = winuxcmd_home() else {
        eprintln!("skipping: winuxcmd installation not found (set WINUXCMD_HOME)");
        return;
    };
    let git = Path::new(&home).join("opt").join("niugit").join("git.exe");
    assert!(git.is_file(), "niugit git.exe missing under {home}");
    let git_display = git.to_string_lossy().replace('\\', "/");
    let repo = scratch_dir("verbatim-git");
    let repo_posix = to_posix_form(&repo);
    let script = format!(
        "export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t; \
         cd '{repo_posix}' || exit 9; \
         '{git_display}' init -q -b main . || exit 9; \
         echo a > a.txt; '{git_display}' add a.txt; '{git_display}' commit -q -m c1 || exit 9; \
         echo b > b.txt; '{git_display}' add b.txt; '{git_display}' commit -q -m c2 || exit 9; \
         '{git_display}' branch origin/feat/x || exit 10; \
         '{git_display}' rev-parse --verify origin/feat/x >/dev/null || exit 11; \
         [ \"$('{git_display}' log --oneline HEAD~1..HEAD | wc -l)\" -eq 1 ] || exit 12; \
         [ \"$('{git_display}' log --oneline HEAD -- a.txt | wc -l)\" -eq 1 ] || exit 13; \
         '{git_display}' checkout -q -b feat/slash.main || exit 14; \
         '{git_display}' rev-parse --verify feat/slash.main >/dev/null || exit 15; \
         echo REV_OK"
    );
    let (code, stdout, stderr) = run_rubash(&script);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("REV_OK"));
    let _ = std::fs::remove_dir_all(&repo);
}

/// C③ POSITIVE: `commit -m`-shaped message bytes survive rubash's argv
/// boundary byte-exactly. The child is a fresh rubash (a NATIVE-class
/// child for this path, so this pins the native-argv quoting: `$`,
/// backticks, single and double quotes, semicolons must all arrive
/// literally — GNU never rewrites argv, neither may we).
#[test]
fn message_bytes_survive_argv_to_native_child() {
    if !cfg!(windows) {
        return;
    }
    let rubash = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    let sentinel = "fix: keep $HOME and `bt` and \"dq\" and 'sq' and ;sep";
    let script = format!(
        "'{rubash}' -c 'printf \"[%s]\" \"$1\"' x '{sentinel}'",
        rubash = rubash,
        sentinel = sentinel.replace('\'', "'\\''"),
    );
    let (code, stdout, stderr) = run_rubash(&script);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(
        stdout,
        format!("[{sentinel}]"),
        "argv bytes changed across the rubash-to-native-child boundary \
         (got {stdout:?}) — message fidelity broken (commit -m class)"
    );
}

/// C③ NEGATIVE: an env channel value shaped like GIT_SSH_COMMAND must pass
/// through with zero re-rendering — POSIX-form option paths and embedded
/// spaces survive to BOTH child classes verbatim.
#[test]
fn git_ssh_command_env_value_passes_through_verbatim() {
    if !cfg!(windows) {
        return;
    }
    let value = "ssh -i /d/repo/keys/id_ed25519 -oBatchMode=yes";
    // POSIX-aware child (winuxcmd env) when installed.
    if winuxcmd_home().is_some() {
        let (code, stdout, stderr) = run_rubash(&format!(
            "export GIT_SSH_COMMAND='{value}'; env | grep '^GIT_SSH_COMMAND='"
        ));
        assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
        assert!(
            stdout.contains(&format!("GIT_SSH_COMMAND={value}")),
            "env channel mangled GIT_SSH_COMMAND: {stdout:?}"
        );
    }
    // Native child (cmd) must see the identical bytes.
    let (code, stdout, stderr) = run_rubash(&format!(
        "export GIT_SSH_COMMAND='{value}'; cmd /c \"set GIT_SSH_COMMAND\""
    ));
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stdout.contains(&format!("GIT_SSH_COMMAND={value}")),
        "native child received mangled GIT_SSH_COMMAND: {stdout:?}"
    );
}
