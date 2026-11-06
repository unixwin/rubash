//! rubash#2916 — `bind -m EMACS` was rejected with
//! `` `EMACS': invalid keymap name `` while GNU accepts keymap
//! names in any case: bind.def:246-257 resolves `-m` through
//! rl_get_keymap_by_name, whose match is _rl_stricmp
//! (lib/readline/bind.c:2424 _rl_get_keymap_by_name). The
//! `set keymap` variable channel already matched case-insensitively
//! (rubash#2871); the `-m` option channel did not, so the two
//! channels disagreed.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
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
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| !line.contains("line editing not enabled"))
        .map(|line| format!("{line}\n"))
        .collect();
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    }
}

#[test]
fn issue2916_uppercase_emacs_is_accepted_and_lands_in_the_canonical_map() {
    let r =
        run("bind -m EMACS '\"\\C-z\": undo'; echo rc=$?; bind -m emacs -p | grep -F '\"\\C-z\"'");
    assert_eq!(r.stdout, "rc=0\n\"\\C-z\": undo\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2916_mixed_case_vi_insert_is_accepted() {
    let r = run(
        "bind -m Vi-Insert '\"\\C-z\": undo'; echo rc=$?; bind -m vi-insert -p | grep -F '\"\\C-z\"'",
    );
    assert_eq!(r.stdout, "rc=0\n\"\\C-z\": undo\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2916_mixed_case_emacs_ctlx_is_accepted() {
    let r = run(
        "bind -m emacs-CTLX '\"\\C-g\": undo'; echo rc=$?; bind -m emacs-ctlx -p | grep -F '\"\\C-g\"'",
    );
    assert_eq!(r.stdout, "rc=0\n\"\\C-g\": undo\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2916_unknown_keymap_name_still_fails_the_command() {
    // bind.def:253-257: an unresolvable name prints
    // `` bind: `NAME': invalid keymap name `` and the builtin
    // returns 1 without applying anything.
    let r = run("bind -m BOGUS '\"\\C-z\": undo'; echo rc=$?");
    assert_eq!(r.stdout, "rc=1\n");
    assert!(
        r.stderr.contains("`BOGUS': invalid keymap name"),
        "stderr: {}",
        r.stderr
    );
    // And the rejected binding must not have landed anywhere.
    let r = run(
        "bind -m BOGUS '\"\\C-z\": undo' 2>/dev/null; bind -p | grep -F '\"\\C-z\"'; echo done",
    );
    assert_eq!(r.stdout, "done\n");
}
