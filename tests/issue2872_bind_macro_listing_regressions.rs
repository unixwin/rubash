//! rubash#2872 — `bind -s` / `bind -S` printed macro text raw: a
//! stored LF/TAB/ESC/0x18 byte went to the terminal byte-for-byte and
//! split the listing line. GNU ground: rl_macro_dumper passes every
//! macro value through _rl_untranslate_macro_value (lib/readline/
//! bind.c:2842, function at bind.c:722) with use_escapes = 0 before
//! printing, in BOTH the readable (`-s`) and plain (`-S`) forms:
//! ESC → `\e`, other control characters → `\C-x` (lowercased unctrl),
//! DEL → `\C-?`, meta bytes → `\M-…`, and `\` / `"` are
//! backslash-escaped so the printed form re-parses to the same value.

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
fn issue2872_macro_newline_prints_as_control_j() {
    let r = run(r#"bind '"\C-x\C-t": "echo hi\n"'; bind -s"#);
    assert_eq!(r, "\"\\C-x\\C-t\": \"echo hi\\C-j\"\n");
    let r = run(r#"bind '"\C-x\C-t": "echo hi\n"'; bind -S"#);
    assert_eq!(r, "\\C-x\\C-t outputs echo hi\\C-j\n");
}

#[test]
fn issue2872_macro_control_bytes_print_escaped() {
    // TAB → \C-i, ESC → \e, 0x18 → \C-x (GNU 5.3.20 probe, both forms).
    let r = run(r#"bind '"\C-x\C-t": "a\tb\ec\C-xd"'; bind -s"#);
    assert_eq!(r, "\"\\C-x\\C-t\": \"a\\C-ib\\ec\\C-xd\"\n");
    let r = run(r#"bind '"\C-x\C-t": "a\tb\ec\C-xd"'; bind -S"#);
    assert_eq!(r, "\\C-x\\C-t outputs a\\C-ib\\ec\\C-xd\n");
}

#[test]
fn issue2872_macro_backslash_and_quote_are_escaped() {
    // Stored text is a,\,b,",c: the listing doubles the backslash and
    // escapes the quote so the line stays one re-readable binding.
    let r = run(r#"bind '"\C-x\C-t": "a\\b\"c"'; bind -s"#);
    assert_eq!(r, "\"\\C-x\\C-t\": \"a\\\\b\\\"c\"\n");
    let r = run(r#"bind '"\C-x\C-t": "a\\b\"c"'; bind -S"#);
    assert_eq!(r, "\\C-x\\C-t outputs a\\\\b\\\"c\n");
}

#[test]
fn issue2872_macro_del_and_meta_print_canonically() {
    let r = run(r#"bind '"\C-x\C-t": "x\C-?y"'; bind -s"#);
    assert_eq!(r, "\"\\C-x\\C-t\": \"x\\C-?y\"\n");
    // \M-x stores ESC + x; the ESC renders as \e, not \M-.
    let r = run(r#"bind '"\C-x\C-t": "p\M-xq"'; bind -s"#);
    assert_eq!(r, "\"\\C-x\\C-t\": \"p\\exq\"\n");
}

#[test]
fn issue2872_plain_macro_text_is_unchanged() {
    let r = run(r#"bind '"\C-x\C-t": "plain"'; bind -s"#);
    assert_eq!(r, "\"\\C-x\\C-t\": \"plain\"\n");
    let r = run(r#"bind '"\C-x\C-t": "plain"'; bind -S"#);
    assert_eq!(r, "\\C-x\\C-t outputs plain\n");
}
