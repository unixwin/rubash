//! rubash#2874 — `bind -X` printed the `-x` command string raw:
//! inner double quotes broke the listing's quoting and backslashes
//! went out undoubled. GNU ground: print_unix_command_map
//! (bashline.c:4729) reuses rl_macro_dumper with a display hook, so
//! the command value passes through _rl_untranslate_macro_value
//! (lib/readline/bind.c:722) exactly like a macro value — `"` → `\"`,
//! `\` → `\\`, control bytes → `\C-x` / `\e` forms.
//!
//! The storage half is pinned by the same outputs: with a colon
//! separator bind_keyseq_to_unix_command stores the command VERBATIM
//! (rl_generic_bind, bashline.c:4898-4900) — `a\nb` keeps its literal
//! backslash, which the listing then doubles — while a whitespace
//! separator routes through rl_macro_bind (bashline.c:4893-4896) and
//! the value IS escape-translated at definition time (a stored TAB
//! lists as `\C-i`).

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
fn issue2874_x_listing_escapes_inner_quotes() {
    let r = run(r#"bind -x '"\C-x\C-e": echo "a b"'; bind -X"#);
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo \\\"a b\\\"\"\n");
}

#[test]
fn issue2874_colon_form_stores_backslash_sequences_verbatim() {
    // The stored command holds backslash + n (two characters); the
    // listing escapes the quote and doubles the backslash. A stored
    // LF would list as \C-j instead (see the macro pins, #2872).
    let r = run(r#"bind -x '"\C-x\C-e": echo "a\nb"'; bind -X"#);
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo \\\"a\\\\nb\\\"\"\n");
    let r = run(r#"bind -x '"\C-x\C-e": echo "a\tb"'; bind -X"#);
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo \\\"a\\\\tb\\\"\"\n");
}

#[test]
fn issue2874_colon_form_backslashes_double_on_listing() {
    // Two stored backslashes list as four; one lists as two.
    let r = run(r#"bind -x '"\C-x\C-e": echo a\\b'; bind -X"#);
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo a\\\\\\\\b\"\n");
    let r = run(r#"bind -x '"\C-x\C-e": echo a\b'; bind -X"#);
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo a\\\\b\"\n");
}

#[test]
fn issue2874_whitespace_form_translates_at_definition() {
    // Whitespace separator: rl_macro_bind translates the value, so
    // the stored command holds a real TAB and lists as \C-i.
    let r = run(r#"bind -x '"\C-x\C-e" "echo a\tb"'; bind -X"#);
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo a\\C-ib\"\n");
}

#[test]
fn issue2874_colon_form_real_control_bytes_list_escaped() {
    // A literal TAB byte inside the colon form is stored verbatim and
    // lists in \C-i form; no raw control byte reaches the output.
    let r = run("bind -x '\"\\C-x\\C-e\": echo \"a\tb\"'; bind -X");
    assert_eq!(r, "\"\\C-x\\C-e\" \"echo \\\"a\\C-ib\\\"\"\n");
    assert!(!r.contains('\t'), "raw TAB in listing: {r:?}");
}
