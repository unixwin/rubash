//! Issue rubash#363 — external shebang scripts executed in-process leaked
//! their variable assignments into the caller's scope (and saw the
//! caller's unexported variables).
//!
//! GNU anchor: execute_cmd.c execute_disk_command -> shell_execve
//! (execute_cmd.c:6139-6233). A `#!` script is exec'd by the kernel as a
//! NEW interpreter process; a shebangless text file hits the ENOEXEC tail
//! (execute_cmd.c:6237-6260) where the forked child sh_longjmps to
//! subshell_top_level and restarts through shell.c:429-464
//! shell_reinitialize. BOTH are fresh shells seeded only from the exported
//! environment — the script's assignments, readonly marks, functions and
//! options die with its process, and it never sees the caller's
//! unexported variables (variables.c:511-526 initialize_shell_variables).
//!
//! Root cause in rubash: the direct-script branch of
//! execute_same_shell_script ran execute_direct_shell_script with
//! exec_model_entry=false (fork model), so the in-process child shared the
//! parent's typed variable store (`shell_state.variables`) — env-map
//! writes were restored, typed writes were not — and read the parent's
//! unexported variables. Fixed by selecting the exec model there, the same
//! fresh-shell machinery the ENOEXEC path (f4e85826) and ${THIS_SH}
//! invocations already use: full ShellState save/restore, child
//! environment rebuilt from exported variables only.
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; probes under
//! target/p0fix/i363 in the lane worktree, plus the issue's direnv
//! stdlib m1/m2/use1 probes).
//!
//! Documented residual: `$$` inside such scripts reports the caller's pid
//! (a real child would report its own) — inherent to in-process execution.

use std::io::Write;
use std::process::{Command, Stdio};

/// Runs the probe as a script FILE (the issue's probes are script-file
/// form; stdin mode has a separate pre-existing relative-redirect-after-cd
/// divergence that would make these tests vacuous).
fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("i363-probe-{}-{}.sh", std::process::id(), seq));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[allow(dead_code)]
fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash(script: &str) -> (String, String, Option<i32>) {
    rubash_script(script)
}

/// A shebang script's plain assignment does not leak into the caller
/// (the issue's minimal reproducer).
#[test]
fn shebang_script_assignment_does_not_leak() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\nMARKER_FROM_STUB=set-by-stub\\n' > stub\n\
                  chmod +x stub\n./stub\necho \"after: MARKER=[${MARKER_FROM_STUB:-unset}]\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "after: MARKER=[unset]\n");
    assert_eq!(code, Some(0));
}

/// An exported variable from inside the script does not leak either
/// (GNU: the child process's environment dies with it).
#[test]
fn shebang_script_export_does_not_leak() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\nexport EXP_STUB=yes\\n' > stub2\n\
                  chmod +x stub2\n./stub2\necho \"exp=[${EXP_STUB:-unset}]\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "exp=[unset]\n");
    assert_eq!(code, Some(0));
}

/// The script sees only the caller's EXPORTED variables (fresh shell).
#[test]
fn shebang_script_sees_only_exported_caller_vars() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\necho \"sees PLOCAL=[${PLOCAL:-unset}] PEXP=[${PEXP:-unset}]\"\\n' > stub3\n\
                  chmod +x stub3\nPLOCAL=hidden\nexport PEXP=shown\n./stub3\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "sees PLOCAL=[unset] PEXP=[shown]\n");
    assert_eq!(code, Some(0));
}

/// A shebangless text script (ENOEXEC shape) is a fresh shell too — its
/// assignments do not leak and it sees only exported caller variables.
#[test]
fn shebangless_script_is_also_fresh() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf 'CHILD_NSB=leakme\\necho \"nosb-sees PLOCAL=[${PLOCAL:-unset}]\"\\n' > nosb\n\
                  chmod +x nosb\nPLOCAL=hidden\n./nosb\necho \"parent CHILD_NSB=[${CHILD_NSB:-unset}]\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "nosb-sees PLOCAL=[unset]\nparent CHILD_NSB=[unset]\n"
    );
    assert_eq!(code, Some(0));
}

/// A `readonly` bound inside the script does not constrain the caller.
#[test]
fn shebang_script_readonly_does_not_leak() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\nreadonly RV_STUB=ro\\n' > stub4\n\
                  chmod +x stub4\n./stub4\nRV_STUB=ok && echo \"overwrite-ok [${RV_STUB}]\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "overwrite-ok [ok]\n");
    assert_eq!(code, Some(0));
}

/// `exit N` inside the script is contained: the caller continues and $? is
/// the script's status (regression guard for the containment the issue's
/// m2 probe verified).
#[test]
fn shebang_script_exit_is_contained() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\ncd /tmp\\nexit 3\\n' > side\n\
                  chmod +x side\n./side\necho \"rc=$? still-here\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert!(stdout.starts_with("rc=3 still-here"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

/// A function defined inside the script does not leak into the caller.
#[test]
fn shebang_script_function_does_not_leak() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\nstub_fn() { echo from-stub; }\\n' > stub5\n\
                  chmod +x stub5\n./stub5\nstub_fn 2>/dev/null || echo \"fn-gone rc=$?\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "fn-gone rc=127\n");
    assert_eq!(code, Some(0));
}

/// PATH-resolved script invocation is isolated the same way (the issue
/// probed both direct and PATH lookup).
#[test]
fn path_lookup_script_isolated() {
    let script = "d=/tmp/i363-path-$$\nmkdir -p \"$d/bin\"\ncd \"$d\"\n\
                  printf '#!/usr/bin/env bash\\nMARKER_FROM_STUB=set-by-stub\\n' > bin/stub-p\n\
                  chmod +x bin/stub-p\nPATH=\"$d/bin:$PATH\"\nstub-p\necho \"PATHlookup=[${MARKER_FROM_STUB:-unset}]\"\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "PATHlookup=[unset]\n");
    assert_eq!(code, Some(0));
}

/// The direnv `use()` shape from the issue: the stub's `cmd=$1` must not
/// clobber the caller's local `cmd` (the use_log: command not found chain).
#[test]
fn direnv_use_shape_cmd_not_clobbered() {
    let script = "mkdir -p /tmp/i363-probe\ncd /tmp/i363-probe\n\
                  printf '#!/usr/bin/env bash\\ncmd=corrupted\\n' > stub6\n\
                  chmod +x stub6\nuse_fn() {\n  local cmd=$1\n  ./stub6\n  echo \"cmd-survived=[$cmd]\"\n}\nuse_fn julia 2.1\n";
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "cmd-survived=[julia]\n");
    assert_eq!(code, Some(0));
}
