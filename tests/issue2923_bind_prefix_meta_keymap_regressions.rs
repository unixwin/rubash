//! rubash#2923 — the old-style arm's `prefix-meta` value
//! (`bind 'Control-t: prefix-meta'`) materializes a KEYMAP-valued
//! binding in GNU: rl_parse_and_bind special-cases the name
//! (lib/readline/bind.c:1845-1855, PREFIX_META_HACK) and calls
//! rl_generic_bind(ISKMAP, seq, emacs_meta_keymap, _rl_keymap), so
//! the whole meta keymap becomes reachable under the new prefix.
//! A function/macro later bound at a keymap-valued slot does not
//! replace the map: rl_generic_bind descends and stores it in the
//! target map's ANYOTHERKEY slot (bind.c, the last-key arm), which
//! is shared by every prefix pointing at that map; unbinding at a
//! keymap slot nulls that fallback and leaves the map in place.
//! rubash's flat registry had no keymap-valued kind, so the slot
//! was occupied (transpose-chars disappeared, like GNU) but zero
//! submap rows ever expanded. Every expectation below is the GNU
//! bash 5.3.20 oracle's verbatim output for the same script.

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
fn issue2923_prefix_meta_expands_the_meta_keymap() {
    let r = run("bind 'Control-t: prefix-meta'; bind -q transpose-words; bind -q shell-transpose-words; bind -q transpose-chars");
    assert_eq!(r.stdout, "transpose-words can be invoked via \"\\C-tt\", \"\\et\".\nshell-transpose-words can be invoked via \"\\C-t\\C-t\", \"\\e\\C-t\".\ntranspose-chars is not bound to any keys.\n");
    assert_eq!(r.stderr, "");
    // Every meta row gains a composed "\C-t…" row in the full dump
    // and the displaced transpose-chars row becomes a
    // `# ... (not bound)` comment, so the total grows by exactly
    // the composed count. Absolute numbers follow rubash's static
    // default tables (the rubash#2338 capture, taken where the
    // distro /etc/inputrc's arrow rows were in the oracle's dump:
    // 109 meta rows; a bare oracle with INPUTRC=/dev/null and no
    // system inputrc counts 489 -> 590 with 101 composed instead —
    // the growth invariant is what this pin locks).
    let r = run("bind -p | wc -l; bind 'Control-t: prefix-meta'; bind -p | wc -l; bind -p | grep -cF '\"\\C-t'");
    assert_eq!(r.stdout, "497\n606\n109\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_esc_binding_becomes_the_meta_fallback() {
    // ESC is the meta keymap's own slot: binding a function there
    // stores it as the map's ANYOTHERKEY fallback; the submap rows
    // are untouched.
    let r = run("bind 'ESC: undo'; bind -q undo; bind -q transpose-words");
    assert_eq!(r.stdout, "undo can be invoked via \"\\C-x\\C-u\", \"\\e\", \"\\C-_\".\ntranspose-words can be invoked via \"\\et\".\n");
    assert_eq!(r.stderr, "");
    // The ctlx slot behaves the same way (fallback shows as the
    // bare prefix, listed after the submap's own rows).
    let r = run("bind 'Control-x: undo'; bind -q undo");
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-x\\C-u\", \"\\C-x\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_function_at_a_keymap_slot_is_the_shared_fallback() {
    // Bound AFTER prefix-meta: undo becomes the meta map's
    // fallback, so it is invocable at BOTH prefixes of that map.
    let r = run("bind 'Control-t: prefix-meta'; bind 'Control-t: undo'; bind -q undo; bind -q transpose-words");
    assert_eq!(r.stdout, "undo can be invoked via \"\\C-t\", \"\\C-x\\C-u\", \"\\e\", \"\\C-_\".\ntranspose-words can be invoked via \"\\C-tt\", \"\\et\".\n");
    assert_eq!(r.stderr, "");
    // Bound BEFORE prefix-meta: the fallback predates the second
    // prefix and is shared with it the moment the slot appears.
    let r = run("bind 'ESC: undo'; bind 'Control-t: prefix-meta'; bind -q undo");
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-t\", \"\\C-x\\C-u\", \"\\e\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
    // A macro fallback is shared the same way and lists at both
    // prefixes in `bind -s`.
    let r = run("bind 'Control-t: prefix-meta'; bind '\"\\C-t\": \"MAC\"'; bind -s");
    assert_eq!(r.stdout, "\"\\C-t\": \"MAC\"\n\"\\e\": \"MAC\"\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_keymap_bind_discards_the_previous_function() {
    // rl_generic_bind's ISKMAP assignment replaces the slot: the
    // function bound there first does not survive as a fallback.
    let r = run("bind 'Control-t: undo'; bind 'Control-t: prefix-meta'; bind -q undo; bind -q transpose-words");
    assert_eq!(r.stdout, "undo can be invoked via \"\\C-x\\C-u\", \"\\C-_\".\ntranspose-words can be invoked via \"\\C-tt\", \"\\et\".\n");
    assert_eq!(r.stderr, "");
    // Same for a macro bound at the slot first.
    let r = run("bind '\"\\C-t\": \"MAC\"'; bind 'Control-t: prefix-meta'; bind -s; bind -q transpose-words");
    assert_eq!(
        r.stdout,
        "transpose-words can be invoked via \"\\C-tt\", \"\\et\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_unbind_at_a_keymap_slot_clears_only_the_fallback() {
    // `bind -r` at a keymap-valued slot nulls the target map's
    // ANYOTHERKEY (whichever prefix set it); the submap survives.
    let r = run("bind 'ESC: undo'; bind 'Control-t: prefix-meta'; bind -r '\\C-t'; bind -q undo");
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-x\\C-u\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
    let r = run("bind 'ESC: undo'; bind -r '\\e'; bind -q undo; bind -q transpose-words");
    assert_eq!(r.stdout, "undo can be invoked via \"\\C-x\\C-u\", \"\\C-_\".\ntranspose-words can be invoked via \"\\et\".\n");
    assert_eq!(r.stderr, "");
    // And the map itself is NOT removed by -r at its prefix.
    let r = run("bind 'Control-t: prefix-meta'; bind -r '\\C-t'; bind -q transpose-words");
    assert_eq!(
        r.stdout,
        "transpose-words can be invoked via \"\\C-tt\", \"\\et\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_meta_prefix_meta_uses_the_raw_byte() {
    // The prefix-meta arm bypasses rl_bind_key, so a META key's
    // sequence is the single raw byte (0xf4, dumped \364), never
    // decomposed into an ESC prefix (bind.c:1845-1855).
    let r = run("bind 'Meta-t: prefix-meta'; bind -q transpose-words");
    assert_eq!(
        r.stdout,
        "transpose-words can be invoked via \"\\et\", \"\\364t\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_prefix_meta_lands_in_the_current_keymap() {
    // The slot is created in _rl_keymap (here vi-insert after the
    // mode flip); the target is still emacs-meta (bind.c:1852).
    let r =
        run("bind 'set editing-mode vi'; bind 'Control-t: prefix-meta'; bind -q transpose-words");
    assert_eq!(r.stdout, "transpose-words can be invoked via \"\\C-tt\".\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_session_bindings_in_the_target_map_propagate() {
    // A binding made directly in emacs-meta shows through the
    // default ESC slot, and through a session prefix-meta slot too.
    let r = run("bind -m emacs-meta '\"t\": undo'; bind -q undo; bind -q transpose-words");
    assert_eq!(r.stdout, "undo can be invoked via \"\\C-x\\C-u\", \"\\et\", \"\\C-_\".\ntranspose-words is not bound to any keys.\n");
    assert_eq!(r.stderr, "");
    let r = run("bind -m emacs-meta '\"t\": undo'; bind 'Control-t: prefix-meta'; bind -q undo");
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-tt\", \"\\C-x\\C-u\", \"\\et\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
    let r = run("bind -m emacs-meta '\"t\": undo'; bind -p | grep -F '\"\\et\"'");
    assert_eq!(r.stdout, "\"\\et\": undo\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_macros_in_the_target_map_compose_too() {
    // rl_macro_dumper recurses keymaps exactly like the function
    // dumper: a macro bound in emacs-meta lists under the ESC
    // prefix of the standard map, and under a session slot's.
    let r = run("bind -m emacs-meta '\"t\": \"MAC\"'; bind -s");
    assert_eq!(r.stdout, "\"\\et\": \"MAC\"\n");
    assert_eq!(r.stderr, "");
    let r = run("bind -m emacs-meta '\"t\": \"MAC\"'; bind 'Control-x: prefix-meta'; bind -s");
    assert_eq!(r.stdout, "\"\\C-xt\": \"MAC\"\n\"\\et\": \"MAC\"\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_quoted_prefix_meta_is_a_null_slot_not_a_keymap() {
    // The quoted form goes through rl_bind_keyseq with
    // rl_named_function("prefix-meta") == NULL: the slot is
    // cleared and NO keymap is bound (the hack lives only in the
    // old-style arm). Observable state matches a verbatim store.
    let r = run("bind '\"\\C-t\": prefix-meta'; bind -q transpose-chars; bind -q transpose-words");
    assert_eq!(
        r.stdout,
        "transpose-chars is not bound to any keys.\ntranspose-words can be invoked via \"\\et\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_invoking_order_lists_the_fallback_after_its_submap() {
    // ANYOTHERKEY is the keymap array's last slot (index 256), so
    // in scan order a map's own rows precede the bare prefix that
    // carries its fallback — even for the anonymous keymap a
    // longer binding creates over a shorter one.
    let r = run("bind '\"\\C-t\": undo'; bind '\"\\C-tt\": undo'; bind -q undo");
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-tt\", \"\\C-t\", \"\\C-x\\C-u\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2923_self_referential_keymap_still_answers() {
    // GNU recurses forever here (the 5.3.20 oracle segfaults on
    // this query); the registry's expansion keeps a visited set
    // per path and returns the finite answer instead.
    let r = run("bind -m emacs-meta 'Control-t: prefix-meta'; bind -m emacs-meta -q transpose-words; bind -q transpose-words");
    assert_eq!(
        r.stdout,
        "transpose-words can be invoked via \"t\".\ntranspose-words can be invoked via \"\\et\".\n"
    );
    assert_eq!(r.stderr, "");
}
