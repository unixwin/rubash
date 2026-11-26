//! Issue rubash#2660: a background job started inside a redirected
//! compound command kept the OUTER stdout — `( echo bg-out & wait )
//! >/dev/null` printed `bg-out` to the real stdout. Async children of
//! a compound command did not inherit the compound's redirections
//! (the run-read suite's read2.sub produced ~2000 leaked lines this
//! way).
//!
//! GNU semantics: the compound's redirections are applied to the
//! shell's own descriptors before the body runs (execute_cmd.c:1576
//! execute_in_subshell applies do_redirections in the forked child;
//! a brace group applies them in place), and an async command is a
//! fork of that shell (execute_cmd.c:2837 execute_connection '&'),
//! so the child is born holding the redirected descriptors.
//!
//! Root cause: rubash resolves an async spawn's stdio from the
//! command's OWN redirects on top of a hardcoded inherit base
//! (`resolve_background_stdio`), never consulting the executor's live
//! fd-table bindings — the enclosing compound's `>/dev/null` lives
//! only in the fd table (the flat body never touches the process's
//! real descriptors), so the spawn inherited the process stdout.
//! Fix: seed the resolver's base stdio (slots 1/2) from the current
//! fd-table write endpoints, the same source a foreground spawn
//! inherits from; the command's own redirects still override, and
//! the command-substitution capture pipe still overrides an
//! unredirected slot.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-11-08).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's first seed shape.
#[test]
fn async_in_redirected_subshell_stdout_is_discarded() {
    let (stdout, stderr, _) = rubash("( echo bg-out & wait ) >/dev/null\necho done1");
    assert_clean(&stderr);
    assert_eq!(stdout, "done1\n");
}

/// The issue's second seed shape (eval body).
#[test]
fn async_eval_in_redirected_subshell_stdout_is_discarded() {
    let (stdout, stderr, _) = rubash("( eval 'echo eval-bg' & wait ) >/dev/null\necho done2");
    assert_clean(&stderr);
    assert_eq!(stdout, "done2\n");
}

/// The run-read suite's actual shape, scaled down: an async eval
/// inside a subshell whose stdout is discarded, in a loop.
#[test]
fn async_in_redirected_subshell_loop_leaks_nothing() {
    let (stdout, stderr, _) = rubash(
        "for i in 1 2 3 4 5; do ( eval 'echo leak' & wait ) >/dev/null; done\necho done-loop",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "done-loop\n");
}

/// Redirected to a file: the async child's output lands in the file,
/// not on the terminal.
#[test]
fn async_in_redirected_subshell_writes_to_the_file() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2660-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let file = dir.join("out.txt");
    let script = format!("( echo to-file & wait ) > {}\necho done3", file.display());
    let (stdout, stderr, _) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(stdout, "done3\n");
    let contents = std::fs::read_to_string(&file).expect("read redirected file");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(contents, "to-file\n");
}

/// A brace group binds its redirections in the current shell; an
/// async child born inside inherits them the same way.
#[test]
fn async_in_redirected_brace_group_stdout_is_discarded() {
    let (stdout, stderr, _) = rubash("{ echo grp-bg & wait; } >/dev/null\necho done5");
    assert_clean(&stderr);
    assert_eq!(stdout, "done5\n");
}

/// The compound's stderr redirect is inherited too, and the async
/// command's own `>&2` dup resolves against the inherited slot.
#[test]
fn async_in_redirected_subshell_stderr_is_discarded() {
    let (stdout, stderr, _) = rubash("( echo err-bg >&2 & wait ) 2>/dev/null\necho done6");
    assert_eq!(stdout, "done6\n");
    assert_clean(&stderr);
}

/// An async EXTERNAL (the direct-spawn path, no wrapper respawn)
/// inherits the compound's redirection the same way.
#[test]
#[cfg(unix)]
fn async_external_in_redirected_subshell_stdout_is_discarded() {
    let (stdout, stderr, _) = rubash("( /bin/echo ext-bg & wait ) >/dev/null\necho done9");
    assert_clean(&stderr);
    assert_eq!(stdout, "done9\n");
}

/// Control: with no enclosing redirection the async child's output
/// still reaches the real stdout (the fix must not over-discard).
#[test]
fn async_without_redirect_still_prints() {
    let (stdout, stderr, _) = rubash("( echo visible & wait )\necho done7");
    assert_clean(&stderr);
    assert_eq!(stdout, "visible\ndone7\n");
}

/// Control: the async command's OWN redirect still wins over the
/// enclosing compound's.
#[test]
fn async_own_redirect_overrides_compound() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2660-own-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let file = dir.join("own.txt");
    let script = format!(
        "( echo own-redir > {} & wait ) >/dev/null\necho done8",
        file.display()
    );
    let (stdout, stderr, _) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(stdout, "done8\n");
    let contents = std::fs::read_to_string(&file).expect("read own-redirect file");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(contents, "own-redir\n");
}
