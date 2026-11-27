//! rubash#2418 — bind surface: variable defaults and the `-q`/`-p`
//! relationship.
//!
//! Face 1 (defaults): GNU lib/readline/bind.c:1874 binds
//! `bind-tty-special-chars` to `_rl_bind_stty_chars` and bind.c:1883
//! binds `convert-meta` to `_rl_convert_meta_chars_to_ascii`; both
//! ints are initialized to 1 in lib/readline/readline.c, and the GNU
//! bash 5.3.20 oracle's `bind -v` (clean env, verified 2026-11-01)
//! prints both `on`. rubash's table (built from an earlier WT probe)
//! carried both as `off` — the other 46 entries match the oracle
//! dump exactly.
//!
//! Face 2 (`-q` vs `-p`): the issue reported GNU's `-q` omitting
//! `\e[1~` for beginning-of-line while `-p` includes it. That
//! asymmetry does NOT exist in GNU: query_bindings
//! (builtins/bind.def:344-367) and rl_function_dumper both consume
//! rl_invoking_keyseqs over the same current keymap (bind.def:205-216),
//! and the oracle matrix (default emacs, -m vi-insert, -m vi-command,
//! -m emacs-meta, -m emacs-ctlx, fresh/interactive) shows the query
//! listing exactly the dump's sequences for beginning-of-line in
//! every keymap — `\e[1~` present in both for emacs, absent from
//! both for the vi maps. These pins lock rubash to that GNU matrix so
//! any future divergence is caught here instead of by anecdote.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    code: i32,
}

fn run(script: &str) -> Run {
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    let out = Command::new(RUBASH)
        .args(["-c", script])
        .env("INPUTRC", EMPTY_INPUTRC)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn issue2418_bind_v_defaults_match_readline_c() {
    let r = run("bind -v");
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.lines().count(), 48, "dump: {:?}", r.stdout);
    assert!(
        r.stdout.contains("set bind-tty-special-chars on"),
        "bind -v: {:?}",
        r.stdout
    );
    assert!(
        r.stdout.contains("set convert-meta on"),
        "bind -v: {:?}",
        r.stdout
    );
    let plain = run("bind -V");
    assert!(
        plain
            .stdout
            .contains("bind-tty-special-chars is set to `on'"),
        "bind -V: {:?}",
        plain.stdout
    );
    assert!(
        plain.stdout.contains("convert-meta is set to `on'"),
        "bind -V: {:?}",
        plain.stdout
    );
}

#[test]
fn issue2418_query_and_dump_agree_for_beginning_of_line() {
    // Default (emacs) keymap: `\e[1~` appears in BOTH the query and
    // the dump on GNU — there is no -q omission rule for it.
    let q = run("bind -q beginning-of-line");
    assert_eq!(q.code, 0);
    assert_eq!(
        q.stdout,
        "beginning-of-line can be invoked via \"\\C-a\", \"\\eOH\", \"\\e[1~\", \"\\e[H\".\n"
    );
    let p = run("bind -p");
    assert!(
        p.stdout.contains("\"\\e[1~\": beginning-of-line"),
        "dump: {:?}",
        p.stdout
    );

    // The vi maps bind beginning-of-line without `\e[1~` on GNU, in
    // query and dump alike.
    let vi_ins = run("bind -m vi-insert -q beginning-of-line");
    assert_eq!(
        vi_ins.stdout,
        "beginning-of-line can be invoked via \"\\eOH\", \"\\e[H\".\n"
    );
    let vi_cmd = run("bind -m vi-command -q beginning-of-line");
    assert_eq!(
        vi_cmd.stdout,
        "beginning-of-line can be invoked via \"\\eOH\", \"\\e[H\", \"0\".\n"
    );
}
