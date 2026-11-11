//! rubash#3194 — `bind -X` listed shell-command bindings in
//! REGISTRATION order; GNU lists them in translated-byte order.
//! print_unix_command_map (bashline.c:4729) prints the command map
//! through rl_macro_dumper (lib/readline/bind.c:2899), which walks
//! key slots 0..KEYMAP_SIZE and recurses into submaps in place —
//! so the listing order is the byte order of the translated
//! sequences, with a bare prefix (the submap's ANYOTHERKEY slot,
//! index 256) printing AFTER the sequences through it. That is the
//! same scan order rl_invoking_keyseqs_in_map uses and the order
//! `bind -s` / `bind -q` already follow in this engine.
//!
//! Expectations are the GNU bash 5.3.20 oracle's verbatim stdout
//! for the same scripts (lane probes p15 / g2 / g4 / g5 / g6 / g7).

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
fn issue3194_two_keys_list_in_byte_order_not_registration_order() {
    // The original p15 shape: \C-u registered first, \C-g second;
    // GNU lists \C-g (0x07) before \C-u (0x15).
    let r = run(r##"bind -x '"\C-u": echo u'; bind -x '"\C-g": echo g'; bind -X"##);
    assert_eq!(r, "\"\\C-g\" \"echo g\"\n\"\\C-u\" \"echo u\"\n");
}

#[test]
fn issue3194_scrambled_registration_lists_in_byte_order() {
    // GNU probe g2, registration order deliberately scrambled
    // across control keys, a prefixed sequence, a meta sequence
    // and a plain character.
    let r = run(
        r##"bind -x '"\C-u": echo u'; bind -x '"\C-g": echo g'; bind -x '"a": echo a'; bind -x '"\C-x\C-f": echo xf'; bind -x '"\ea": echo ea'; bind -x '"\C-b": echo b'; bind -X"##,
    );
    assert_eq!(
        r,
        "\"\\C-b\" \"echo b\"\n\
         \"\\C-g\" \"echo g\"\n\
         \"\\C-u\" \"echo u\"\n\
         \"\\C-x\\C-f\" \"echo xf\"\n\
         \"\\ea\" \"echo ea\"\n\
         \"a\" \"echo a\"\n"
    );
}

#[test]
fn issue3194_prefix_sequence_prints_after_sequences_through_it() {
    // GNU probes g4/g5 (both registration orders give the same
    // listing): the bare "\C-x" entry lives in the submap's
    // ANYOTHERKEY slot (index 256, the scan's last slot), so it
    // prints after "\C-x\C-f".
    let r = run(
        r##"bind -x '"\C-x\C-f": echo xf'; bind -x '"\C-x": echo x'; bind -x '"\C-g": echo g'; bind -X"##,
    );
    assert_eq!(
        r,
        "\"\\C-g\" \"echo g\"\n\
         \"\\C-x\\C-f\" \"echo xf\"\n\
         \"\\C-x\" \"echo x\"\n"
    );
    let r = run(
        r##"bind -x '"\C-x": echo x'; bind -x '"\C-x\C-f": echo xf'; bind -x '"\C-g": echo g'; bind -X"##,
    );
    assert_eq!(
        r,
        "\"\\C-g\" \"echo g\"\n\
         \"\\C-x\\C-f\" \"echo xf\"\n\
         \"\\C-x\" \"echo x\"\n"
    );
}

#[test]
fn issue3194_keymap_scoped_listing_is_sorted_within_the_map() {
    let r = run(
        r##"bind -m vi-insert -x '"\C-u": echo viu'; bind -m vi-insert -x '"\C-g": echo vig'; bind -m vi-insert -X; echo '--- emacs:'; bind -x '"\C-u": echo emu'; bind -X"##,
    );
    assert_eq!(
        r,
        "\"\\C-g\" \"echo vig\"\n\
         \"\\C-u\" \"echo viu\"\n\
         --- emacs:\n\
         \"\\C-u\" \"echo emu\"\n"
    );
}

#[test]
fn issue3194_chain_remove_and_reregister_keeps_byte_order() {
    // Chained (GNU probe g6): register scrambled, remove the
    // byte-smallest, re-register it — the listing is byte order
    // throughout, never re-registration order.
    let r = run(
        r##"bind -x '"\C-u": echo u'; bind -x '"\C-b": echo b'; bind -x '"\C-g": echo g'; bind -r '\C-b'; bind -x '"\C-b": echo b2'; bind -X"##,
    );
    assert_eq!(
        r,
        "\"\\C-b\" \"echo b2\"\n\
         \"\\C-g\" \"echo g\"\n\
         \"\\C-u\" \"echo u\"\n"
    );
}

#[test]
fn issue3194_macro_listing_order_is_the_same_scan_lock() {
    // Regression lock (GNU probe g7): `bind -s` already lists in
    // scan order on the base — the -X fix must not disturb it.
    let r = run(r##"bind '"\C-u": "mu"'; bind '"\C-g": "mg"'; bind -s"##);
    assert_eq!(r, "\"\\C-g\": \"mg\"\n\"\\C-u\": \"mu\"\n");
}
