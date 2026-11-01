//! rubash#2386 — `bind` ignored the CURRENT keymap. The factory
//! tables themselves landed with #2338, but every listing, query,
//! registration and removal still answered from the caller's named
//! map or a keymap-less user layer, so after switching the editing
//! mode the builtin kept reporting (and editing) the emacs world.
//!
//! GNU ground (vendored bash 5.3 tree):
//! - builtins/bind.def:205-216 — the builtin runs against
//!   rl_get_keymap(): `-m` installs its map as the current one for
//!   the builtin; without `-m` everything uses the live keymap.
//! - lib/readline/bind.c:2743/2753/2811 rl_macro_dumper /
//!   rl_function_dumper / rl_print_keybinding read `_rl_keymap`;
//!   bind.c:2097/2104 sv_editmode moves `_rl_keymap` when the
//!   `editing-mode` variable changes; bind.c:2517 the `keymap`
//!   variable setter calls rl_set_keymap; bind.c:3018-3023 the
//!   variable reads back as rl_get_keymap_name(_rl_keymap) — the
//!   FIRST registered name, so vi-movement reads back `vi`.
//! - builtins/set.def:424-447 set_edit_mode — `set -o vi` /
//!   `set -o emacs` bind the `editing-mode` readline variable, so
//!   they move the same current keymap.
//! - subst of the -r rule: rl_unbind_keyseq_in_map /
//!   rl_unbind_function_in_mapq act on that same map only.
//!
//! Pin shape: the full factory dump is 497 lines — the pin compares
//! STRUCTURE (line counts of each keymap's dump, which are static in
//! the engine tables and were byte-matched against the GNU bash
//! 5.3.20 oracle capture when the tables landed) plus spot entries,
//! never an embedded copy of the whole dump. Expectations verified
//! against the oracle binary on 2026-11-01.

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
fn issue2386_factory_dump_shape_in_default_keymap() {
    let r = run("bind -p");
    assert_eq!(r.code, 0);
    // Leading blank line, then one line per bound sequence plus one
    // `# name (not bound)` comment per unbound function.
    assert!(r.stdout.starts_with('\n'), "dump: {:?}", r.stdout);
    assert_eq!(line_count(&r.stdout), 497, "dump: {:?}", r.stdout);
    for spot in [
        "\"\\C-a\": beginning-of-line",
        "\"\\C-k\": kill-line",
        "\"\\C-x\\C-e\": edit-and-execute-command",
        "\"\\eOH\": beginning-of-line",
        "# alias-expand-line (not bound)",
    ] {
        assert!(r.stdout.contains(spot), "missing {spot:?} in dump");
    }
    let plain = run("bind -P");
    assert_eq!(line_count(&plain.stdout), 177, "-P: {:?}", plain.stdout);
}

#[test]
fn issue2386_bind_p_follows_set_o_vi() {
    let r = run("set -o vi; bind -p");
    assert_eq!(line_count(&r.stdout), 424, "dump: {:?}", r.stdout);
    assert!(r.stdout.contains("\"\\C-w\": vi-unix-word-rubout"));
    assert!(
        !r.stdout.contains("\"\\C-a\": beginning-of-line"),
        "emacs binding leaked into the vi dump"
    );
}

#[test]
fn issue2386_bind_p_follows_editing_mode_variable() {
    let r = run("bind 'set editing-mode vi'; bind -p");
    assert_eq!(line_count(&r.stdout), 424, "dump: {:?}", r.stdout);
}

#[test]
fn issue2386_keymap_variable_selects_movement_map_and_reads_back_canonically() {
    let r = run("bind 'set keymap vi-command'; bind -p");
    assert_eq!(line_count(&r.stdout), 225, "dump: {:?}", r.stdout);
    let vars = run("bind 'set keymap vi-command'; bind -v");
    assert!(
        vars.stdout.contains("set keymap vi\n"),
        "readback must be the canonical first name: {:?}",
        vars.stdout
    );
}

#[test]
fn issue2386_query_uses_current_keymap() {
    // In vi-insert, backward-char is reachable only via the escape
    // sequences; \C-b belongs to the emacs map alone.
    let r = run("set -o vi; bind -q backward-char");
    assert_eq!(r.code, 0);
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2386_registration_lands_in_current_keymap() {
    let vi = run("set -o vi; bind '\"\\C-a\": kill-line'; bind -p");
    assert!(vi.stdout.contains("\"\\C-a\": kill-line"));
    let emacs = run("set -o vi; bind '\"\\C-a\": kill-line'; bind -m emacs -p");
    assert!(emacs.stdout.contains("\"\\C-a\": beginning-of-line"));
    assert!(
        !emacs.stdout.contains("\"\\C-a\": kill-line"),
        "vi registration leaked into the emacs map"
    );
}

#[test]
fn issue2386_removal_lands_in_current_keymap() {
    let vi = run("set -o vi; bind -r '\\C-a'; bind -p");
    assert!(
        !vi.stdout.contains("\"\\C-a\""),
        "vi-insert \\C-a survived a current-map removal: {:?}",
        vi.stdout
    );
    // The same keystroke in the emacs map is untouched.
    let emacs = run("set -o vi; bind -r '\\C-a'; bind -m emacs -q beginning-of-line");
    assert!(
        emacs.stdout.contains("\"\\C-a\""),
        "emacs beginning-of-line lost its \\C-a: {:?}",
        emacs.stdout
    );
}

#[test]
fn issue2386_unbind_function_is_scoped_to_the_dash_m_keymap() {
    let r = run("set -o vi; bind -m emacs -u beginning-of-line; bind -m emacs -p");
    assert!(
        r.stdout.contains("# beginning-of-line (not bound)"),
        "emacs dump after -u: {:?}",
        r.stdout
    );
    // The current (vi-insert) map is not the -u target.
    let vi = run("set -o vi; bind -m emacs -u beginning-of-line; bind -p");
    assert_eq!(line_count(&vi.stdout), 424, "dump: {:?}", vi.stdout);
}

#[test]
fn issue2386_default_mode_layering_spot_checks() {
    // A user remap shadows the factory entry in the same map.
    let r = run("bind '\"\\C-a\": kill-line'; bind -q beginning-of-line");
    assert!(!r.stdout.contains("\"\\C-a\""), "query: {:?}", r.stdout);
    assert!(r.stdout.contains("\"\\eOH\""), "query: {:?}", r.stdout);
    // Removing one factory sequence leaves the function's others.
    let r = run("bind -r '\\C-b'; bind -q backward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\n"
    );
}
