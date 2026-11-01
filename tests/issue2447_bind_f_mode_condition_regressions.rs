//! rubash#2447 — `bind -f`: a `$if mode=` conditional must observe an
//! `set editing-mode` line earlier in the SAME file, and each binding
//! must land in the keymap current at its own position in the file.
//!
//! GNU ground (vendored bash 5.3 tree):
//! - lib/readline/bind.c:1067-1120 _rl_read_init_file walks the file
//!   line by line through rl_parse_and_bind — strictly sequential.
//! - lib/readline/bind.c:1291-1302: `$if mode=VALUE` tests the LIVE
//!   rl_editing_mode at that point in the file.
//! - lib/readline/bind.c:2092-2110 sv_editmode: an in-file
//!   `set editing-mode vi|emacs` sets rl_editing_mode AND moves
//!   _rl_keymap (vi_insertion_keymap / emacs_standard_keymap) at once,
//!   so later conditionals see the new mode and later bindings land in
//!   the new keymap; bindings BEFORE the set stay in the old keymap.
//!
//! Expectations probed against the GNU bash 5.3.20 oracle binary on
//! 2026-11-01. The pre-existing pin for the headline case lives in
//! tests/issue431_readline_surface_regressions.rs
//! (bind_f_mode_condition_arms_from_in_file_set).

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--norc", "-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn temp_file(name: &str, content: &str) -> String {
    let path = std::env::temp_dir().join(format!("rubash2447_{}_{}.rc", name, std::process::id()));
    std::fs::write(&path, content).expect("write temp inputrc");
    // Forward slashes: the path is embedded UNQUOTED in `-c` scripts,
    // where backslashes would parse as escape characters.
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn bind_f_in_file_set_editing_mode_vi_arms_mode_vi_block() {
    // The issue repro: GNU applies the block; the binding shows in the
    // post-file dump because the current keymap followed the set.
    let main = temp_file(
        "modevi",
        "set editing-mode vi\n$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n",
    );
    let (stdout, _, code) = run(&format!("bind -f {main}; bind -p | grep C-m"));
    assert_eq!(code, Some(0));
    assert!(stdout.contains("\"\\C-m\": vi-editing-mode"), "{stdout}");
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_mode_block_binding_lands_in_vi_insert_not_emacs() {
    // The conditional binding is stamped into the keymap current at
    // its position (vi-insert, after the in-file set) — flipping the
    // mode to emacs afterwards must NOT show it, flipping back must.
    let main = temp_file(
        "modevi",
        "set editing-mode vi\n$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n",
    );
    let (stdout, _, _) = run(&format!(
        "bind -f {main}; bind 'set editing-mode emacs'; bind -p | grep C-m"
    ));
    assert!(stdout.contains("\"\\C-m\": accept-line"), "{stdout}");
    assert!(!stdout.contains("vi-editing-mode"), "{stdout}");
    let (stdout, _, _) = run(&format!(
        "bind -f {main}; bind 'set editing-mode emacs'; bind 'set editing-mode vi'; bind -p | grep C-m"
    ));
    assert!(stdout.contains("\"\\C-m\": vi-editing-mode"), "{stdout}");
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_bindings_before_in_file_set_stay_in_previous_keymap() {
    // A binding BEFORE the in-file `set editing-mode vi` belongs to the
    // pre-file keymap (emacs): the vi dump keeps its factory `\C-g`
    // (self-insert), the emacs dump gains the file's abort binding.
    let main = temp_file(
        "presplit",
        "\"\\C-g\": abort\nset editing-mode vi\n$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n",
    );
    let (stdout, _, _) = run(&format!("bind -f {main}; bind -p | grep C-g"));
    assert!(stdout.contains("\"\\C-g\": self-insert"), "{stdout}");
    assert!(!stdout.contains("\"\\C-g\": abort"), "{stdout}");
    let (stdout, _, _) = run(&format!(
        "bind -f {main}; bind 'set editing-mode emacs'; bind -p | grep C-g"
    ));
    assert!(stdout.contains("\"\\C-g\": abort"), "{stdout}");
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_mode_emacs_else_arm_applies_after_in_file_vi_set() {
    // After the in-file set to vi, `$if mode=emacs` misses and its
    // $else arm is the one that applies.
    let main = temp_file(
        "elsearm",
        "set editing-mode vi\n$if mode=emacs\n\"\\C-m\": accept-line\n$else\n\"\\C-m\": vi-editing-mode\n$endif\n",
    );
    let (stdout, _, code) = run(&format!("bind -f {main}; bind -p | grep C-m"));
    assert_eq!(code, Some(0));
    assert!(stdout.contains("\"\\C-m\": vi-editing-mode"), "{stdout}");
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_in_file_set_editing_mode_emacs_arms_mode_emacs_from_vi_start() {
    // Mirror direction: the session starts in vi (via bind), the file
    // sets emacs and the mode=emacs block arms into the emacs keymap.
    let main = temp_file(
        "modeemacs",
        "set editing-mode emacs\n$if mode=emacs\n\"\\C-m\": beginning-of-line\n$endif\n",
    );
    let (stdout, _, code) = run(&format!(
        "bind 'set editing-mode vi'; bind -f {main}; bind -p | grep C-m"
    ));
    assert_eq!(code, Some(0));
    assert!(stdout.contains("\"\\C-m\": beginning-of-line"), "{stdout}");
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_mode_vi_block_stays_closed_without_in_file_set() {
    // Guard against over-arming: with no in-file set and the default
    // emacs mode, a bare `$if mode=vi` block never applies.
    let main = temp_file(
        "closed",
        "$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n",
    );
    let (stdout, _, code) = run(&format!("bind -f {main}; bind -p | grep C-m"));
    assert_eq!(code, Some(0));
    assert!(stdout.contains("\"\\C-m\": accept-line"), "{stdout}");
    assert!(!stdout.contains("vi-editing-mode"), "{stdout}");
    std::fs::remove_file(main).ok();
}
