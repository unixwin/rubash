//! rubash#2606 — the bind registry keyed entries by the WRITTEN
//! string, so one physical key registered under two spellings
//! (`"\C-u"` vs the raw byte via $'"\025"', vs `"\C-U"`) lived in
//! two slots. GNU keys keymaps by the TRANSLATED BYTE SEQUENCE:
//! rl_generic_bind (lib/readline/bind.c:383) runs
//! rl_translate_keyseq (bind.c:516) first and walks byte slots, so
//! every spelling of a sequence is the same slot — later binds
//! override, unbinds (bind.def:391 unbind_keyseq, which also
//! translates first) remove by any spelling, and the dumpers read
//! back the canonical written form (`\C-u`, `\C-?`, `\e…`)
//! regardless of the spelling registered under.
//!
//! Expectations below are the GNU bash 5.3.20 oracle's verbatim
//! stdout for the same scripts (probe files p1–p14 of the lane
//! report). Note the `bind -r` argument is the BARE sequence
//! (bind.def:170 hands the raw option argument to unbind_keyseq):
//! a double-quoted argument is a different, three-byte sequence
//! and removes nothing (last cell).

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run(script: &str) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn issue2606_x_override_via_raw_byte_spelling() {
    let r = run(r##"bind -x '"\C-u": echo first'; bind -x $'"\025": echo second'; bind -X"##);
    assert_eq!(r, "\"\\C-u\" \"echo second\"\n");
}

#[test]
fn issue2606_x_override_via_uppercase_spelling() {
    // rl_translate_keyseq folds \C-U onto the same byte as \C-u
    // (the base letter is masked with 0x1f).
    let r = run(r##"bind -x '"\C-u": echo first'; bind -x '"\C-U": echo upper'; bind -X"##);
    assert_eq!(r, "\"\\C-u\" \"echo upper\"\n");
}

#[test]
fn issue2606_x_raw_spelling_first_reads_back_canonical() {
    // Registered under the raw byte, the entry still reads back in
    // the canonical written form — the map stores bytes, the
    // dumper renders them.
    let r = run(r##"bind -x $'"\025": echo first'; bind -X"##);
    assert_eq!(r, "\"\\C-u\" \"echo first\"\n");
}

#[test]
fn issue2606_function_override_via_raw_byte_spelling() {
    let r = run(
        r##"bind '"\C-u": kill-whole-line'; bind -q kill-whole-line; bind $'"\025": unix-line-discard'; bind -q kill-whole-line; bind -q unix-line-discard"##,
    );
    assert_eq!(
        r,
        "kill-whole-line can be invoked via \"\\C-u\".\n\
         kill-whole-line is not bound to any keys.\n\
         unix-line-discard can be invoked via \"\\C-u\".\n"
    );
}

#[test]
fn issue2606_macro_override_via_octal_spelling() {
    let r = run(r##"bind '"\C-g": "first"'; bind -s; bind $'"\007": "second"'; bind -s"##);
    assert_eq!(r, "\"\\C-g\": \"first\"\n\"\\C-g\": \"second\"\n");
}

#[test]
fn issue2606_remove_x_via_bare_raw_spelling() {
    let r = run(
        r##"bind -x '"\C-u": echo cmd'; bind -r $'\025'; echo "remove rc=$?"; bind -X; echo '--- X listing end'"##,
    );
    assert_eq!(r, "remove rc=0\n--- X listing end\n");
}

#[test]
fn issue2606_remove_x_via_bare_backslash_spelling() {
    let r = run(
        r##"bind -x '"\C-u": echo cmd'; bind -r '\C-u'; echo "remove rc=$?"; bind -X; echo '--- X listing end'"##,
    );
    assert_eq!(r, "remove rc=0\n--- X listing end\n");
}

#[test]
fn issue2606_remove_function_via_bare_raw_spelling() {
    // The slot is nulled, not restored: the default binding the
    // registration displaced (unix-line-discard) does not come
    // back either (GNU probe p6b).
    let r = run(
        r##"bind '"\C-u": kill-whole-line'; bind -r $'\025'; echo "remove rc=$?"; bind -q kill-whole-line; bind -q unix-line-discard"##,
    );
    assert_eq!(
        r,
        "remove rc=0\n\
         kill-whole-line is not bound to any keys.\n\
         unix-line-discard is not bound to any keys.\n"
    );
}

#[test]
fn issue2606_keymap_scoped_identity() {
    // Byte identity holds per keymap: the two spellings collide
    // inside vi-insert and stay invisible to the emacs map.
    let r = run(
        r##"bind -m vi-insert -x '"\C-u": echo vi-first'; bind -m vi-insert -x $'"\025": echo vi-second'; bind -m vi-insert -X; echo '--- emacs command map:'; bind -X; echo '--- end'"##,
    );
    assert_eq!(
        r,
        "\"\\C-u\" \"echo vi-second\"\n--- emacs command map:\n--- end\n"
    );
}

#[test]
fn issue2606_chain_remove_then_reregister_raw_spelling() {
    // Chained: register -> remove (bare form) -> re-register under
    // the raw-byte spelling. The re-registration must clear the
    // removal tombstone recorded under the other spelling.
    let r = run(
        r##"bind -x '"\C-u": echo one'; bind -r '\C-u'; bind -X; echo '--- re-register'; bind -x $'"\025": echo two'; bind -X"##,
    );
    assert_eq!(r, "--- re-register\n\"\\C-u\" \"echo two\"\n");
}

#[test]
fn issue2606_meta_spellings_share_one_slot() {
    // \M-z and \ez translate to the same bytes (ESC, z); the
    // canonical readback is the \e form.
    let r = run(
        r##"bind '"\ez": kill-whole-line'; bind -q kill-whole-line; bind '"\M-z": unix-line-discard'; bind -q kill-whole-line; bind -q unix-line-discard"##,
    );
    assert_eq!(
        r,
        "kill-whole-line can be invoked via \"\\ez\".\n\
         kill-whole-line is not bound to any keys.\n\
         unix-line-discard can be invoked via \"\\C-u\", \"\\ez\".\n"
    );
}

#[test]
fn issue2606_del_spellings_share_one_slot() {
    // \C-?, the raw DEL byte and \d are one slot; canonical
    // readback is \C-?.
    let r = run(
        r##"bind -x '"\C-?": echo del-first'; bind -x $'"\177": echo del-second'; bind -X; bind -x '"\d": echo del-third'; bind -X"##,
    );
    assert_eq!(
        r,
        "\"\\C-?\" \"echo del-second\"\n\"\\C-?\" \"echo del-third\"\n"
    );
}

#[test]
fn issue2606_distinct_keys_stay_distinct_under_override() {
    let r = run(
        r##"bind -x '"\C-g": echo g1'; bind -x '"\C-u": echo u1'; bind -x $'"\025": echo u2'; bind -X"##,
    );
    assert_eq!(r, "\"\\C-g\" \"echo g1\"\n\"\\C-u\" \"echo u2\"\n");
}

#[test]
fn issue2606_quoted_remove_argument_is_a_different_sequence() {
    // GNU quirk, pinned as-is: `bind -r '"\C-u"'` (the argument
    // still carrying its double quotes) translates to the
    // three-byte sequence [0x22, 0x15, 0x22], which is bound to
    // nothing — the removal silently succeeds and the entry
    // survives (GNU probe p14).
    let r = run(r##"bind -x '"\C-u": echo cmd'; bind -r '"\C-u"'; echo "remove rc=$?"; bind -X"##);
    assert_eq!(r, "remove rc=0\n\"\\C-u\" \"echo cmd\"\n");
}
