//! Issue 2457: the `login_shell` shopt face — what GNU actually does.
//!
//! The issue as filed claimed `shopt -q login_shell` inside a `( )`
//! subshell reads the cleared global and reports NOT set, citing
//! builtins/shopt.def:440. The oracle refutes that half: shopt.def:440
//! (`shopt_login_shell = login_shell`) sits in reset_shopt_options,
//! which runs at shell init (shell.c:2025) — NOT at subshell entry —
//! and the query reads the separate shadow variable
//! (shopt.def:160/244). A fork copies the shadow verbatim, so the
//! plain query inside `( )`, a pipeline element, or a command
//! substitution keeps reporting the invocation value. rubash's
//! stored bit already behaves exactly like that shadow, and the
//! first test below pins the match so the filed prescription
//! (consult the latch on the QUERY face) can never regress it.
//!
//! The real divergence on this face is the SETTER. GNU's login_shell
//! entry carries a custom setter (shopt.def:244, set_login_shell at
//! shopt.def:762-766) that IGNORES the requested value and re-copies
//! the CURRENT login_shell global:
//!
//!   - `shopt -u login_shell` at the top of a login shell: stays ON
//!     (the global is still 1).
//!   - `shopt -s login_shell` in a non-login shell: stays OFF.
//!   - either spelling inside a `( )` region of a login shell:
//!     snaps OFF, because execute_in_subshell cleared the global
//!     (execute_cmd.c:1636-1637) and the attempt re-syncs from it.
//!
//! rubash applied the requested value like any other option. The
//! fix reports a login_shell set/unset ATTEMPT out of the pure
//! builtin and the executor re-derives the stored bit from the
//! effective login state — `invocation_login &&
//! !subshell_login_cleared`, the same expression the logout gate
//! reads (the latch tracks exactly GNU's global clears:
//! execute_cmd.c:1636-1637 / 5474-5475 boundaries set it, a command
//! substitution does not).
//!
//! Oracle: GNU bash 5.3.20, byte-compared on every shape below.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(tag: &str, args: &[&str]) -> Run {
    let dir = std::env::temp_dir().join(format!("rubash-issue2457-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    let output = Command::new(rubash_bin())
        .args(args)
        .stdin(Stdio::null())
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    let run = Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    };
    let _ = fs::remove_dir_all(&dir);
    run
}

#[test]
fn issue2457_query_reads_the_shadow_verbatim() {
    // The filed shape: plain query inside a `( )` subshell of a
    // login shell reports SET in GNU (stale shadow) — and in rubash.
    let r = run(
        "subshell-query",
        &["-l", "-c", "( shopt -q login_shell; echo \"sub rc=$?\" )"],
    );
    assert_eq!(r.stdout, "sub rc=0\n");
    assert_eq!(r.stderr, "");

    // Same staleness in a pipeline element …
    let r = run(
        "pipeline-query",
        &[
            "-l",
            "-c",
            "echo x | { shopt -q login_shell; echo \"pipe rc=$?\"; }",
        ],
    );
    assert_eq!(r.stdout, "pipe rc=0\n");

    // … and in a command substitution.
    let r = run(
        "comsub-query",
        &[
            "-l",
            "-c",
            "x=$(shopt -q login_shell; echo \"rc=$?\"); echo \"comsub $x\"",
        ],
    );
    assert_eq!(r.stdout, "comsub rc=0\n");

    // Non-login shells report unset everywhere, as before.
    let r = run(
        "nonlogin-query",
        &["-c", "( shopt -q login_shell; echo \"sub rc=$?\" )"],
    );
    assert_eq!(r.stdout, "sub rc=1\n");
}

#[test]
fn issue2457_setter_rederives_from_the_effective_global() {
    // -u at the top of a login shell cannot turn the report off:
    // set_login_shell re-copies the still-set global.
    let r = run(
        "unset-at-login-top",
        &[
            "-l",
            "-c",
            "shopt -u login_shell; shopt -q login_shell; echo \"rc=$?\"; shopt login_shell",
        ],
    );
    assert_eq!(r.stdout, "rc=0\nlogin_shell         \ton\n");
    assert_eq!(r.code, Some(0));

    // -s in a non-login shell cannot turn it on either.
    let r = run(
        "set-at-nonlogin-top",
        &[
            "-c",
            "shopt -s login_shell; shopt -q login_shell; echo \"rc=$?\"; shopt login_shell",
        ],
    );
    assert_eq!(r.stdout, "rc=1\nlogin_shell         \toff\n");
    // The trailing list query for an off option is itself status 1.
    assert_eq!(r.code, Some(1));

    // Inside a `( )` region the global is cleared, so EITHER
    // spelling snaps the report off …
    for (tag, attempt) in [("subshell-unset", "-u"), ("subshell-set", "-s")] {
        let script = format!("( shopt {attempt} login_shell; shopt -q login_shell; echo \"rc=$?\" )");
        let r = run(tag, &["-l", "-c", &script]);
        assert_eq!(r.stdout, "rc=1\n", "shape: {tag}");
    }

    // … including in a pipeline element (a 5474-5475 boundary).
    let r = run(
        "pipeline-set",
        &[
            "-l",
            "-c",
            "echo x | { shopt -s login_shell; shopt -q login_shell; echo \"pipe rc=$?\"; }",
        ],
    );
    assert_eq!(r.stdout, "pipe rc=1\n");

    // The subshell's snap-off never leaks back into the parent.
    let r = run(
        "parent-isolation",
        &[
            "-l",
            "-c",
            "( shopt -u login_shell ); shopt -q login_shell; echo \"parent rc=$?\"",
        ],
    );
    assert_eq!(r.stdout, "parent rc=0\n");
}
