//! rubash#621: unquoted indexed compound expansion of whitespace-padded
//! values created a spurious leading empty element.
//!
//! The field-split transport prefixes the word with a bookkeeping marker;
//! the indexed consumer left it in the accumulator, so a leading separator
//! pushed a marker-only first part decoding to `""`. GNU strips leading
//! IFS whitespace (subst.c:3142 `list_string`: "Remove sequences of
//! whitespace at the beginning of STRING, as long as those characters
//! appear in IFS"):
//!
//! ```c
//! /* subst.c:3119 list_string(), :3142-3155 */
//! for (s = string; *s && issep(*s) && ifs_whitespace(*s); s++);
//! if (!*s) return NULL;
//! ```
//!
//! The consumer now skips the transport prefix up front (both copies:
//! executor + declare storage); the post-split map re-adds it per part,
//! so downstream bytes are unchanged. Assoc readback was already correct
//! and is pinned.
//!
//! Expectations come from the C source above; spot-checked against GNU Bash
//! 5.3.20 locally (the repo oracle is WSL 5.3.0 — no version-specific
//! behavior is asserted here).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

/// `plain=($v)` with leading IFS whitespace yields exactly the fields, no
/// leading empty element.
fn assert_split(value: &str, rendered: &str, count: &str) {
    let script = format!("v={value}; plain=($v); declare -p plain; echo \"n=${{#plain[@]}}\"");
    let out = run(&script);
    assert_eq!(
        out.stdout,
        format!("declare -a plain={rendered}\nn={count}\n"),
        "{script} -> stdout {:?}",
        out.stdout
    );
    assert_eq!(out.status, 0);
}

#[test]
fn leading_spaces() {
    assert_split("' 1 '", "([0]=\"1\")", "1");
}

#[test]
fn leading_tab() {
    assert_split("$'\\t1\\t'", "([0]=\"1\")", "1");
}

#[test]
fn leading_and_interior() {
    assert_split("'  a b  '", "([0]=\"a\" [1]=\"b\")", "2");
}

#[test]
fn multi_word_padded() {
    let out = run("a='  x '; b='  y '; plain=($a $b); declare -p plain");
    assert_eq!(out.stdout, "declare -a plain=([0]=\"x\" [1]=\"y\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn declare_path_padded() {
    let out = run("v=' 1 '; declare -a plain=($v); declare -p plain");
    assert_eq!(out.stdout, "declare -a plain=([0]=\"1\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn trailing_only_unchanged() {
    assert_split("'1 '", "([0]=\"1\")", "1");
}

#[test]
fn interior_unchanged() {
    assert_split("'a  b'", "([0]=\"a\" [1]=\"b\")", "2");
}

#[test]
fn all_whitespace_yields_empty() {
    assert_split("'   '", "()", "0");
}

#[test]
fn quoted_keeps_padding() {
    let out = run("v=' 1 '; plain=(\"$v\"); declare -p plain");
    assert_eq!(out.stdout, "declare -a plain=([0]=\" 1 \")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn assoc_readback_untouched() {
    let out = run("map[k]=' 1 '; echo \"<${map[k]}>\"");
    assert_eq!(out.stdout, "< 1 >\n");
    assert_eq!(out.status, 0);
}

#[test]
fn custom_ifs_leading_empty_kept() {
    // Non-whitespace IFS keeps a leading empty field (list_string only
    // strips IFS *whitespace*) — the fix must not drop it.
    let out = run("IFS=:; v=':a:b'; plain=($v); declare -p plain");
    assert_eq!(
        out.stdout,
        "declare -a plain=([0]=\"\" [1]=\"a\" [2]=\"b\")\n"
    );
    assert_eq!(out.status, 0);
}
