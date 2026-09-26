//! `scan_substitution_spans` walks `chars` (char indices) but needs a byte
//! offset when it slices `raw`. Passing the char index straight into
//! `&raw[cursor + 1..]` panicked as soon as a `$(` was preceded by any
//! multi-byte character ("start byte index N is not a char boundary").
//!
//! Covers the non-ASCII prefix case in double quotes, plain words and inside a
//! parameter expansion.

use std::process::Command;

fn rubash(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn cjk_before_command_substitution_in_double_quotes() {
    assert_eq!(
        rubash("echo \"该文件（应为）: $(echo hi)\""),
        "该文件（应为）: hi\n"
    );
}

#[test]
fn cjk_around_command_substitution() {
    assert_eq!(rubash("echo \"abc（x）: $(echo hi)\""), "abc（x）: hi\n");
}

#[test]
fn cjk_before_parameter_expansion_in_double_quotes() {
    assert_eq!(rubash("v=1\necho \"（应为）: ${v}\""), "（应为）: 1\n");
}

#[test]
fn cjk_prefix_without_any_substitution() {
    assert_eq!(rubash("echo \"纯中文\""), "纯中文\n");
}

// niubash#92 (rubash 1.1.0): `&raw[cursor + 1..]` in scan_substitution_spans
// used a char index as a byte offset, so two or more multibyte characters
// immediately before `$(` panicked with "not a char boundary" and the
// process exited 127.

#[test]
fn cjk_adjacent_to_command_substitution() {
    assert_eq!(rubash("echo \"中文$(echo h)\""), "中文h\n");
    assert_eq!(rubash("echo \"中文文$(echo h)\""), "中文文h\n");
    assert_eq!(rubash("echo 中文$(echo h)"), "中文h\n");
    assert_eq!(rubash("echo \"文件数=$(echo 3)\""), "文件数=3\n");
    assert_eq!(rubash("echo $(echo 中文)$(echo h)"), "中文h\n");
}

#[test]
fn cjk_quoted_fragment_before_second_substitution() {
    // The literal+substitution fragment path must carry raw bytes through
    // field materialization: widening bytes to chars re-encoded the UTF-8
    // literal as mojibake.
    assert_eq!(rubash("echo \"中文$(echo h)\"$(echo x)"), "中文hx\n");
}

#[test]
fn quoted_literal_whitespace_survives_split_policy() {
    // `"a b$(echo h)"$(echo x)` is split-eligible (the trailing $() is
    // unquoted) but the quoted literal space is data, not an IFS delimiter.
    assert_eq!(rubash("echo \"a b$(echo h)\"$(echo x)"), "a bhx\n");
    assert_eq!(rubash("echo \"中 文$(echo h)\"$(echo x)"), "中 文hx\n");
}

// Same byte/char-boundary class one subsystem over: the comsub heredoc
// header scanner widened each byte with `as char`, so a delimiter char
// whose UTF-8 carries a 0x85/0xa0 continuation byte (悠 = U+60A0, the
// U+E0A0 powerline glyph, ...) satisfied `is_whitespace` mid-char and the
// `&line[start..index]` slice panicked inside `$(...)`.

#[test]
fn multibyte_heredoc_delimiter_inside_command_substitution() {
    assert_eq!(
        rubash("x=$(cat <<E\u{60a0}F\nhi\nE\u{60a0}F\n); echo \"$x\""),
        "hi\n"
    );
    assert_eq!(
        rubash("echo \"$(cat <<E\u{60a0}F\nhi\nE\u{60a0}F\n)\""),
        "hi\n"
    );
    assert_eq!(
        rubash("x=$(cat <<中文\nbody\n中文\n); echo \"$x\""),
        "body\n"
    );
}

#[test]
fn escaped_multibyte_heredoc_delimiter_inside_command_substitution() {
    // `<<E\悠F` quotes the char: the delimiter is E悠F. The escape skip must
    // step over the whole char, not just two bytes.
    assert_eq!(
        rubash("x=$(cat <<E\\\u{60a0}F\nhi\nE\u{60a0}F\n); echo \"$x\""),
        "hi\n"
    );
}

// niubash#139 (rubash lexer): `skip_parenthesized_unit_corrected` collected
// `chars[index..]` into `rest` (a string starting AT the current char) and
// then sliced `&rest[1..]` — a char boundary only for single-byte chars. A
// multibyte character inside `$(...)` combined with `${...}` in one word
// panicked with "start byte index 1 is not a char boundary". The fix steps
// over `ch.len_utf8()` bytes. GNU oracle (bash 5.3): values concatenate
// verbatim.

fn rubash_full(script: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn multibyte_inside_comsub_next_to_parameter_expansion() {
    // The exact issue-139 trigger shapes: `${v}` or `${#v}` sharing one
    // double-quoted word with a `$(...)` whose body is multibyte.
    for (script, want) in [
        ("v=hello; echo \"${v}$(echo 中)\"", "hello中\n"),
        ("v=hello; echo \"${v}$(echo 中文)\"", "hello中文\n"),
        ("v=he; echo \"${v}$(echo \u{1f680})\"", "he\u{1f680}\n"),
        ("v=abc; echo \"${#v} $(echo 中文)\"", "3 中文\n"),
        ("v=abc; echo \"$(echo 中文) ${#v}\"", "中文 3\n"),
        // Multibyte deeper inside the comsub body, not just the result.
        ("v=ok; echo \"${v}$(echo a中b)\"", "oka中b\n"),
        ("v=ok; echo \"x${v}$(echo 中)y\"", "xok中y\n"),
    ] {
        let (stdout, stderr, status) = rubash_full(script);
        assert_eq!(stdout, want, "script: {script}");
        assert_eq!(status, 0, "stderr: {stderr}");
    }
}
