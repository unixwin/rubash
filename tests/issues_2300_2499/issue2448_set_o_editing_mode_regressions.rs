//! rubash#2448 — `set -o vi` / `set -o emacs`: a redundant FLAG_ON
//! write must re-assert the editing mode on the keymap registry, even
//! when the option cell was already on (the #2435 defect through the
//! `set -o` door; #2435 fixed the `shopt -o` door).
//!
//! GNU ground (vendored bash 5.3 tree):
//! - builtins/set.def:476-491 set_minus_o_option funnels EVERY `-o`
//!   write through SET_BINARY_O_OPTION_VALUE (set.def:247-249), which
//!   invokes the option's setter unconditionally — there is no
//!   transition check anywhere in the funnel.
//! - builtins/set.def:424-447 set_edit_mode (the vi/emacs entries'
//!   setter, set.def:200,235): a FLAG_ON write runs
//!   rl_variable_bind("editing-mode", name) every time; sv_editmode
//!   (lib/readline/bind.c:2092-2110) moves the current keymap. A
//!   FLAG_OFF write never rebinds the variable.
//!
//! Expectations probed against the GNU bash 5.3.20 oracle binary on
//! 2026-11-01. Discriminator: `bind -q backward-char` — the emacs
//! keymap reports "\C-b" among the invoking sequences, the vi-insert
//! keymap does not.
//!
//! Adjudication note: on master these CLI shapes were already green —
//! `execute_set_command`'s fast-path scan fires the bind
//! unconditionally during its attempt, which masked the slow path's
//! transition gate in `execute_set` on every probed route (~35
//! shapes). The gate was a source-level defect (its invariant
//! contradicts set.def:476-491 and disagreed with the fast-path and
//! shopt doors); the fix removes it by replaying the reported writes
//! in `execute_set`. These pins guard the funnel contract at the CLI
//! for both doors, so a future route that reaches the slow path
//! without the fast-path scan cannot revive the divergence silently.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

const EMACS_Q: &str = "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\n";
const VI_Q: &str = "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\n";

fn run(script: &str) -> (String, i32) {
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
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2448_set_o_vi_moves_the_keymap() {
    let (stdout, code) = run("set -o vi; bind -q backward-char");
    assert_eq!(code, 0);
    assert_eq!(stdout, VI_Q);
}

#[test]
fn issue2448_redundant_set_o_vi_rebinds_editing_mode() {
    // The option cell is already on when the second `set -o vi` runs;
    // GNU's setter fires anyway and pulls the keymap back from the
    // intervening bind-set emacs.
    let (stdout, code) =
        run("set -o vi; bind 'set editing-mode emacs'; set -o vi; bind -q backward-char");
    assert_eq!(code, 0);
    assert_eq!(stdout, VI_Q);
}

#[test]
fn issue2448_redundant_set_o_emacs_rebinds_editing_mode() {
    let (stdout, code) =
        run("set -o emacs; bind 'set editing-mode vi'; set -o emacs; bind -q backward-char");
    assert_eq!(code, 0);
    assert_eq!(stdout, EMACS_Q);
}

#[test]
fn issue2448_set_plus_o_vi_never_rebinds() {
    // FLAG_OFF: the option turns off, the editing mode (set through
    // the variable) stays where it was.
    let (stdout, code) = run("bind 'set editing-mode vi'; set +o vi; bind -q backward-char");
    assert_eq!(code, 0);
    assert_eq!(stdout, VI_Q);
}

#[test]
fn issue2448_multiple_o_writes_apply_in_operand_order() {
    // GNU applies each -o write as it is met: the LAST FLAG_ON
    // vi/emacs write of the invocation owns the keymap.
    let (stdout, _) =
        run("bind 'set editing-mode emacs'; set -o emacs -o vi; bind -q backward-char");
    assert_eq!(stdout, VI_Q);
    let (stdout, _) = run("bind 'set editing-mode vi'; set -o vi -o emacs; bind -q backward-char");
    assert_eq!(stdout, EMACS_Q);
}

#[test]
fn issue2448_redundant_set_o_vi_rebinds_after_shopt_door_set_the_cell() {
    // The cell was turned on through the shopt door; a redundant
    // `set -o vi` must still rebind (the doors share one funnel).
    let (stdout, code) =
        run("shopt -s -o vi; bind 'set editing-mode emacs'; set -o vi; bind -q backward-char");
    assert_eq!(code, 0);
    assert_eq!(stdout, VI_Q);
}
