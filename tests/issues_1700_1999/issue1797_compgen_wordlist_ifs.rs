//! Issue rubash#1797 regression: `compgen -W` split the wordlist on
//! shell whitespace only, ignoring $IFS — a newline inside the
//! wordlist never split, and a tempenv IFS was not honored.
//!
//! GNU pcomplete.c:863 gen_wordlist_matches splits with subst.c:2616
//! split_at_delims at delims == NULL, which reads ifs_value — the
//! current $IFS (tempenv frame included), defaulting to " \t\n".
//! A maximal run of IFS characters is one separator and produces no
//! empty fields; quoting follows subst.c:2199 skip_to_delim (quotes,
//! escapes, backquotes and $(...)/${...} spans group, delimiters
//! inside a span are content). Each piece then goes through
//! expand_words_shellexp: quoted pieces stay whole, unquoted pieces
//! are expanded and field-split at the same IFS.
//!
//! Oracle: GNU bash 5.3.20 — every expectation below was produced by
//! the oracle binary (see the test names for the shapes).

use std::process::Command;

fn rubash(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

fn candidates(script: &str) -> Vec<String> {
    let (stdout, _) = rubash(script);
    stdout.lines().map(str::to_string).collect()
}

#[test]
fn wordlist_splits_on_newline_with_default_ifs() {
    assert_eq!(
        candidates("compgen -W $'a\\nb c' ''"),
        ["a", "b", "c"],
        "newline is a default-IFS character and must split"
    );
}

#[test]
fn wordlist_honors_tempenv_ifs() {
    assert_eq!(candidates("IFS=: compgen -W 'a:b' ''"), ["a", "b"]);
    assert_eq!(
        candidates("compgen -W 'a:b' ''"),
        ["a:b"],
        "colon is not in the default IFS"
    );
    assert_eq!(
        candidates("IFS=$'\\n' compgen -W $'a\\nb\\nc' ''"),
        ["a", "b", "c"]
    );
}

#[test]
fn wordlist_delimiter_runs_produce_no_empty_fields() {
    assert_eq!(candidates("IFS=: compgen -W 'a::b' ''"), ["a", "b"]);
    assert_eq!(candidates("IFS=: compgen -W ':a:' ''"), ["a"]);
    assert_eq!(
        candidates("IFS=': ' compgen -W 'a: b::c ' ''"),
        ["a", "b", "c"]
    );
    let (stdout, rc) = rubash("IFS=: compgen -W ':' ''");
    assert_eq!(stdout, "", "a lone delimiter run yields no words");
    assert_eq!(rc, Some(1), "no matches means compgen fails");
}

#[test]
fn wordlist_empty_ifs_splits_nowhere() {
    assert_eq!(candidates("IFS= compgen -W 'a b' ''"), ["a b"]);
}

#[test]
fn wordlist_quoting_still_groups() {
    assert_eq!(candidates("compgen -W '\"a b\" c' ''"), ["a b", "c"]);
    assert_eq!(candidates("compgen -W 'a\\ b c' ''"), ["a b", "c"]);
    // An escaped IFS character is field content, not a separator, and
    // an escaped $ starts no expansion.
    assert_eq!(candidates("IFS=: compgen -W 'a\\:b' ''"), ["a:b"]);
    assert_eq!(candidates("compgen -W 'a\\$HOME' ''"), ["a$HOME"]);
    assert_eq!(candidates("IFS=: compgen -W 'a\" : \"b' ''"), ["a : b"]);
    assert_eq!(candidates("IFS=: compgen -W \"'a:b':c\" ''"), ["a:b", "c"]);
}

#[test]
fn wordlist_pieces_expand_with_ifs_field_splitting() {
    // Unquoted piece: expansion result is field-split at the IFS.
    assert_eq!(candidates("x='p:q'; IFS=: compgen -W '$x' ''"), ["p", "q"]);
    // Quoted piece: expansion stays one field.
    assert_eq!(candidates("x='p:q'; IFS=: compgen -W '\"$x\"' ''"), ["p:q"]);
    // A non-IFS space inside an unquoted piece survives both splits.
    assert_eq!(candidates("IFS=: compgen -W 'a b:c' ''"), ["a b", "c"]);
    // Command substitution and ${...} spans group at split time; the
    // expanded result is field-split afterwards.
    assert_eq!(
        candidates("IFS=: compgen -W '$(echo \"a:b\")' ''"),
        ["a", "b"]
    );
    assert_eq!(candidates("IFS=: compgen -W '${y:-d:e}' ''"), ["d", "e"]);
}
