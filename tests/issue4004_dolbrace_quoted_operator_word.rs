//! rubash#4004 — inside double quotes, a `${var op word}` fragment
//! embedded in a larger word lost its quote context on the
//! non-mutating expansion path, so a quoted `}` inside the operator
//! word ended the span early / the default word's quotes were treated
//! as active syntax: `"a b${W:-'}'}"` as a redirect target created
//! `a b}` where GNU bash 5.3.20 creates `a b'}'`.
//!
//! GNU semantics: subst.c:9777 `parameter_brace_expand` receives the
//! enclosing word's per-character quote flags; inside a
//! double-quoted span the `${var op word}` RHS is itself expanded as
//! a double-quoted string — subst.c:7982-7993
//! `parameter_brace_expand_rhs` runs `string_extract_double_quoted`
//! + `expand_string_for_rhs` with the quoted flag — so `'` characters
//! inside `:-`/`:=`/`:+`/... words stay literal data and a `}` inside
//! `'...'` never closes the `${...}` span
//! (parse.y:3877 `parse_matched_pair`, subst.c:1954-1960
//! `extract_dollar_brace_string`). redir.c:298-345
//! `redirection_expand` runs the target through the ordinary
//! word-expansion pipeline, so the quoted bit applies to redirect
//! targets exactly as it does to arguments.
//!
//! rubash root cause: the lexer strips the outer `"` delimiters of a
//! fully-quoted word and marks it STORAGE_WORD_PREFIX
//! (src/lexer/word.rs), so the non-mutating embedded-parameter walk
//! (src/executor/embedded_parameters.rs) never saw `in_double` and
//! dispatched the `${...}` fragment through plain `expand_word`,
//! which expands the operator RHS unquoted — quote removal ate the
//! `'...'` in `'}'`. The mutable walker already carried the bit as
//! SubstitutionQuoteContext::DoubleQuoted; the fix routes quoted
//! fragments through `expand_quoted_parameter_word`
//! (src/executor/parameter_words.rs), the &self port of the same
//! context, and reconstructs the outer `"..."` span for
//! STORAGE_WORD_PREFIX words whose delimiters were stripped.
//!
//! Oracle evidence: WSL GNU bash 5.3.20 (/usr/local/bin/bash),
//! LF-terminated script probes; expected filenames below are the GNU
//! results byte-for-byte.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash` in a per-test
//! temp working directory; created filenames are asserted on the
//! filesystem. `LC_ALL=C.UTF-8` is set on the child explicitly so the
//! pin is hermetic under any ambient locale.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Out {
    rc: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run(script: &str, cwd: &PathBuf) -> Out {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    Out {
        rc: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4004-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expect_ok(out: &Out, ctx: &str) {
    assert_eq!(
        out.rc,
        0,
        "{ctx}: rc {} != 0; stderr: {}",
        out.rc,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr not empty: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Issue reproducer verbatim: `"a b${W:-'}'}"` as a redirect target.
/// GNU creates the file `a b'}'`; rubash created `a b}`.
#[test]
fn redirect_quoted_default_squote() {
    let dir = temp_dir("repro");
    let out = run(": > \"a b${W:-'}'}\"", &dir);
    expect_ok(&out, "\"a b${W:-'}'}\" as redirect target");
    assert!(
        dir.join("a b'}'").is_file(),
        "file \"a b'}}'\" must be created; dir: {:?}",
        fs::read_dir(&dir).map(|d| d.collect::<Vec<_>>())
    );
    assert!(
        !dir.join("a b}").exists(),
        "file \"a b}}\" must NOT be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Same fragment embedded mid-word: quotes inside the default are
/// literal data, not syntax.
#[test]
fn redirect_quoted_default_squote_embedded() {
    let dir = temp_dir("embedded");
    let out = run(": > \"a${W:-'}'}b\"", &dir);
    expect_ok(&out, "\"a${W:-'}'}b\" as redirect target");
    assert!(
        dir.join("a'}'b").is_file(),
        "file \"a'}}'b\" must be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Default word carrying a quoted space: `"a${W:-'n o'}"` must yield
/// the single filename `a'n o'` — the space is protected inside the
/// quoted `${...}` RHS and must not leak an IFS_GLUE carrier into
/// the open() name.
#[test]
fn redirect_quoted_default_quoted_space() {
    let dir = temp_dir("qspace");
    let out = run(": > \"a${W:-'n o'}\"", &dir);
    expect_ok(&out, "\"a${W:-'n o'}\" as redirect target");
    assert!(
        dir.join("a'n o'").is_file(),
        "file \"a'n o'\" must be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Unquoted default word with a space inside double quotes:
/// `"a b${W:-c d}"` → `a bc d`, one field, one file.
#[test]
fn redirect_quoted_default_bare_space() {
    let dir = temp_dir("bspace");
    let out = run(": > \"a b${W:-c d}\"", &dir);
    expect_ok(&out, "\"a b${W:-c d}\" as redirect target");
    assert!(
        dir.join("a bc d").is_file(),
        "file \"a bc d\" must be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Whole-word quoted `${...}` operator (the shape that already
/// worked) stays correct.
#[test]
fn redirect_whole_word_quoted_default() {
    let dir = temp_dir("whole");
    let out = run(": > \"${W:-'}'}\"", &dir);
    expect_ok(&out, "\"${W:-'}'}\" as redirect target");
    assert!(dir.join("'}'").is_file(), "file \"'}}'\" must be created");
    let _ = fs::remove_dir_all(&dir);
}

/// Mixed-quote word: the `${...}` fragment sits inside `"..."` while
/// the word also has an unquoted tail — the QUOTED_EXPANSION_SITE
/// marker must carry the quoted bit to the fragment dispatch.
#[test]
fn redirect_mixed_quote_quoted_default() {
    let dir = temp_dir("mixed");
    let out = run(": > \"a${W:-'}'}\"tail", &dir);
    expect_ok(&out, "\"a${W:-'}'}\"tail as redirect target");
    assert!(
        dir.join("a'}'tail").is_file(),
        "file \"a'}}'tail\" must be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Argument position (non-redirect) agrees: echo prints the literal
/// quotes back.
#[test]
fn argument_quoted_default_squote() {
    let dir = temp_dir("arg");
    let out = run("printf '%s' \"a${W:-'}'}\"", &dir);
    expect_ok(&out, "printf \"a${W:-'}'}\"");
    assert_eq!(out.stdout, b"a'}'", "stdout must keep the literal quotes");
    let _ = fs::remove_dir_all(&dir);
}

/// Assignment RHS keeps the same semantics.
#[test]
fn assignment_quoted_default_squote() {
    let dir = temp_dir("assign");
    let out = run("v=\"a${W:-'}'}\"; printf '%s' \"$v\"", &dir);
    expect_ok(&out, "v=\"a${W:-'}'}\"");
    assert_eq!(out.stdout, b"a'}'", "assignment value must keep quotes");
    let _ = fs::remove_dir_all(&dir);
}

/// Negative control: the SAME operator word UNQUOTED still treats
/// `'...'` as active quote syntax — GNU produces `x}y`, not `x'}'y`.
/// A fix that globally preserved quotes inside `${...}` would fail
/// here.
#[test]
fn unquoted_default_still_strips_quotes() {
    let dir = temp_dir("unquoted");
    let out = run(": > x${W:-'}'}y", &dir);
    expect_ok(&out, "x${W:-'}'}y as redirect target");
    assert!(dir.join("x}y").is_file(), "file \"x}}y\" must be created");
    assert!(
        !dir.join("x'}'y").exists(),
        "file \"x'}}'y\" must NOT be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Negative control: an unquoted variable carrying a space still
/// splits — GNU reports ambiguous redirect and creates nothing.
#[test]
fn negative_unquoted_var_splits() {
    let dir = temp_dir("negvar");
    let out = run("V='x y'; : > $V", &dir);
    assert_eq!(out.rc, 1, "rc {} != 1 (GNU exits 1)", out.rc);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ambiguous redirect"),
        "stderr must report ambiguous redirect, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let created: Vec<String> = fs::read_dir(&dir)
        .expect("read temp dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        created.is_empty(),
        "no file may be created, found {created:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// `:?` diagnostics still fire through the quoted fragment route:
/// unset W with a message prints the message and exits nonzero.
/// GNU `bash -c` exits 127 here (script-file mode exits 1); rubash
/// matches the `-c` status exactly.
#[test]
fn quoted_default_error_operator_still_fails() {
    let dir = temp_dir("err");
    let out = run(": > \"a${W:?need W}\"", &dir);
    assert_eq!(out.rc, 127, "rc {} != 127 (GNU -c exits 127)", out.rc);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("need W"),
        "stderr must carry the :? message, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Graft guard (captain review): the quoted reroute must not fire for
/// a word that carries an assignment fragment — the rubash#1169 freeze
/// keys on the master's routing, and a quoted-entry render of an
/// earlier fragment reads the post-assignment value. GNU evaluates a
/// word's fragments strictly left-to-right, so the case subject is
/// `d/x`, not `x/x` (issue3045's freeze shape, verbatim).
#[test]
fn case_subject_assignment_freeze_keeps_master_route() {
    let dir = temp_dir("freeze");
    let out = run(
        "unset -v a; case \"${a:-d}/${a:=x}\" in (d/x) echo \"ok a=[$a]\";; (*) echo BAD;; esac",
        &dir,
    );
    expect_ok(&out, "case subject left-to-right freeze");
    assert_eq!(
        out.stdout, b"ok a=[x]\n",
        "earlier fragment must see the pre-assignment value"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Same freeze with the bare `=` assignment form.
#[test]
fn case_subject_bare_assign_freeze_keeps_master_route() {
    let dir = temp_dir("freezebare");
    let out = run(
        "unset -v a; case \"${a-d}/${a=x}\" in (d/x) echo \"ok a=[$a]\";; (*) echo BAD;; esac",
        &dir,
    );
    expect_ok(&out, "case subject bare-= freeze");
    assert_eq!(
        out.stdout, b"ok a=[x]\n",
        "earlier fragment must see the pre-assignment value"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Fragments after the assignment DO see the new value: the freeze is
/// per-fragment ordering, not a whole-word snapshot.
#[test]
fn echo_assignment_fragments_see_ordered_values() {
    let dir = temp_dir("ordered");
    let out = run("unset -v a; echo \"${a:-d}/${a:=x}/${a:-e}\"", &dir);
    expect_ok(&out, "ordered fragment values");
    assert_eq!(
        out.stdout, b"d/x/x\n",
        "fragments must render d, x, x in order"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A `:?` fragment ahead of the assignment still fires against the
/// pre-assignment state (a is unset at that point in the word).
#[test]
fn error_fragment_before_assignment_still_fires() {
    let dir = temp_dir("errfreeze");
    let out = run("unset -v a; echo \"${a:?need a}/${a:=x}\"", &dir);
    assert_eq!(out.rc, 127, "rc {} != 127 (GNU -c exits 127)", out.rc);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("need a"),
        "stderr must carry the :? message, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Heredoc body: the quoted-RHS rule applies there too (the dispatch's
/// heredoc site bit) — the quotes inside the default word stay
/// literal data instead of being quote-removed.
#[test]
fn heredoc_quoted_default_squote() {
    let dir = temp_dir("heredoc");
    let out = run("cat <<EOF\nx=${W:-'}'}\nEOF", &dir);
    expect_ok(&out, "heredoc quoted default");
    assert_eq!(
        out.stdout, b"x='}'\n",
        "heredoc body must keep the literal quotes"
    );
    let _ = fs::remove_dir_all(&dir);
}
