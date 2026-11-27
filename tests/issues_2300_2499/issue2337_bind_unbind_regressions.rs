//! rubash#2337 — the default keymap was effectively immutable in the
//! wrong direction: `bind -r` / `bind -u` removed only session overlay
//! entries, so unbinding a DEFAULT binding silently did nothing
//! (`bind -u backward-char` still answered `"\C-b"` afterwards).
//!
//! GNU ground: readline's keymaps are single mutable maps and the
//! unbind paths mutate them, defaults included —
//! builtins/bind.def:375-388 unbind_command →
//! rl_unbind_function_in_map(function, rl_get_keymap());
//! builtins/bind.def:390-427 unbind_keyseq →
//! rl_bind_keyseq(seq, NULL) on the current map, succeeding silently
//! when nothing is bound. Two oracle-verified edges (GNU 5.3.20):
//! naming a bare keymap PREFIX removes nothing (`bind -r "\C-x"`
//! leaves `\C-x\C-?` bound, because unbind_keyseq acts only when the
//! sequence resolves to a function), and any session binding kind
//! (-x command, macro, function) shadows the default for its exact
//! sequence.
//!
//! rubash keeps the defaults in static tables, so the registry
//! records removals and queries subtract them; registering a
//! sequence again clears its removal.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
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
    }
}

#[test]
fn issue2337_unbind_function_removes_the_defaults() {
    let r = run("bind -u backward-char; bind -q backward-char; echo rc=$?");
    assert_eq!(r.stdout, "backward-char is not bound to any keys.\nrc=1\n");
}

#[test]
fn issue2337_unbind_keyseq_removes_the_default() {
    let r = run("bind -r '\\C-k'; bind -q kill-line; echo rc=$?");
    assert_eq!(r.stdout, "kill-line is not bound to any keys.\nrc=1\n");
    // -r of a sequence that was never bound still succeeds silently.
    let r = run("bind -r '\\C-b'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
}

#[test]
fn issue2337_unbind_is_scoped_to_the_keymap() {
    let r = run("bind -m vi-insert -r '\\e[D'; bind -q backward-char; bind -m vi-insert -q backward-char; echo rc=$?");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\nbackward-char can be invoked via \"\\eOD\".\nrc=0\n"
    );
    let r = run("bind -m vi-insert -u backward-char; bind -q backward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2337_rebinding_shadows_and_revives() {
    // Rebinding \C-b to another function removes it from
    // backward-char and adds it to forward-char (rl_generic_bind
    // replaces the slot).
    let r = run("bind '\"\\C-b\": forward-char'; bind -q backward-char; bind -q forward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\nforward-char can be invoked via \"\\C-b\", \"\\C-f\", \"\\eOC\", \"\\e[C\".\n"
    );
    // Unbind then rebind: the binding comes back.
    let r = run("bind -r '\\C-b'; bind '\"\\C-b\": backward-char'; bind -q backward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2337_session_bindings_still_unbind() {
    // Control: a session-new binding is created and removed.
    let r = run("bind '\"\\ez\": backward-char'; bind -q backward-char; bind -r '\\ez'; bind -q backward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\", \"\\ez\".\nbackward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2337_removing_a_keymap_prefix_removes_nothing() {
    // Oracle: unbind_keyseq acts only when the sequence resolves to a
    // function; "\C-x" names the ctlx submap, so this is a silent
    // no-op and the subtree binding survives.
    let r = run("bind -r '\\C-x'; bind -q backward-kill-line");
    assert_eq!(
        r.stdout,
        "backward-kill-line can be invoked via \"\\C-x\\C-?\".\n"
    );
}
