//! rubash#2435 — `shopt -s -o vi` flipped the option cell but not the
//! line-editing keymap (the #2386 sibling on the shopt funnel).
//!
//! GNU ground (vendored bash 5.3 tree):
//! - builtins/shopt.def:594-606 set_shopt_o_options routes every
//!   `-o` operand through builtins/set.def:476-491 set_minus_o_option
//!   — the ONE funnel `set -o` also uses.
//! - builtins/set.def:424-447 set_edit_mode (the vi/emacs entries'
//!   setter): a FLAG_ON write runs rl_variable_bind("editing-mode",
//!   name), and sv_editmode (lib/readline/bind.c:2092-2110) moves the
//!   current keymap the bind builtin reports. The setter fires on
//!   EVERY FLAG_ON write — a redundant `shopt -s -o vi` with the
//!   option already on rebinds the mode even after an intervening
//!   `bind 'set editing-mode emacs'` (oracle-verified) — while a
//!   FLAG_OFF write never touches the variable (`shopt -u -o vi`
//!   leaves the vi keymap current).
//!
//! Expectations verified against the GNU bash 5.3.20 oracle binary
//! on 2026-11-01 (dump line counts 497 emacs / 424 vi are the static
//! table shapes pinned by issue2386).

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

fn line_count(s: &str) -> usize {
    s.lines().count()
}

#[test]
fn issue2435_shopt_s_o_vi_moves_the_keymap() {
    let r = run("shopt -s -o vi; bind -p");
    assert_eq!(r.code, 0);
    assert_eq!(line_count(&r.stdout), 424, "dump: {:?}", r.stdout);
    assert!(r.stdout.contains("\"\\C-w\": vi-unix-word-rubout"));
    assert!(
        !r.stdout.contains("\"\\C-a\": beginning-of-line"),
        "emacs binding leaked into the vi dump"
    );
    let q = run("shopt -s -o vi; bind -q backward-char");
    assert_eq!(
        q.stdout,
        "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2435_shopt_u_o_vi_does_not_move_the_keymap_back() {
    // set_edit_mode's FLAG_OFF arm never rebinds editing-mode: the
    // option turns off, the vi keymap stays current.
    let r = run("shopt -s -o vi; shopt -u -o vi; bind -p");
    assert_eq!(line_count(&r.stdout), 424, "dump: {:?}", r.stdout);
    let listing = run("shopt -s -o vi; shopt -u -o vi; shopt -o vi");
    assert!(listing.stdout.contains("off"), "listing: {:?}", listing.stdout);
}

#[test]
fn issue2435_shopt_s_o_emacs_restores_the_emacs_keymap() {
    let r = run("shopt -s -o vi; shopt -s -o emacs; bind -p");
    assert_eq!(line_count(&r.stdout), 497, "dump: {:?}", r.stdout);
    let q = run("shopt -s -o vi; shopt -s -o emacs; bind -q backward-char");
    assert_eq!(
        q.stdout,
        "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2435_redundant_shopt_s_o_vi_rebinds_editing_mode() {
    // GNU fires set_edit_mode on every FLAG_ON write, not only on
    // off->on transitions: after the user rebinds the mode through
    // the variable, re-setting the already-on option binds it again.
    let r = run(
        "shopt -s -o vi; bind 'set editing-mode emacs'; shopt -s -o vi; bind -q backward-char",
    );
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\n"
    );
}
