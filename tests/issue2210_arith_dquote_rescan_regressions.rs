//! rubash#2210 — an arithmetic expansion inside a double-quoted string
//! corrupted backslashes in SIBLING expansions' values: the mutable
//! word walker's trailing unescape pass
//! (`unescape_remaining_shell_escapes`, gated on the source word
//! containing `$(` or a backtick) ran over the fully assembled word and
//! could not tell deferred SOURCE escape pairs (its legitimate input)
//! from backslash bytes that arrived as expansion DATA (a plain
//! variable's value), so `\(` in `$body` lost its backslash whenever
//! `$((...))` shared the string.
//!
//! GNU subst.c never re-scans assembled text: every parameter-expansion
//! result merged into a word passes through `quote_string`
//! (subst.c:4773), which CTLESC-prefixes every character, at the
//! `add_quoted_string:` label of expand_word_internal (subst.c:11862),
//! and `dequote_string` (subst.c:4807) only strips CTLESC bytes. The
//! fix ports that invariant to the mutable walker the same way the
//! assignment-RHS entry already did for rubash#218: parameter results
//! ride as ASSIGN_EXPANSION_BACKSLASH through the walk, the trailing
//! unescape skips them, and the entry that ran the pass restores them.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! (2026-10-30). Unix-only: the shapes spell POSIX helper idioms
//! (`printf` formats with `\n`) the Windows goldens lane covers via the
//! upstream suites.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2210-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run `script` as ./case.sh inside the case dir and return stdout.
fn run_script(case: &Case, script: &str) -> String {
    let path = case.dir.join("case.sh");
    std::fs::write(&path, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The filed minimal repro: arithmetic expansion + plain-variable
/// sibling inside one double-quoted assignment RHS.
#[test]
fn issue2210_minimal_repro_assignment_arith_and_var() {
    let case = make_case("minimal");
    let out = run_script(
        &case,
        r#"body='printf "[%s]\n" "a\(b"'
s="x=$((1+1)) $body"
printf '%s\n' "$s"
"#,
    );
    assert_eq!(out, "x=2 printf \"[%s]\\n\" \"a\\(b\"\n");
}

/// The full corruption matrix from the issue: every `\X` class member
/// inside the sibling variable's value must survive byte-for-byte when
/// an arithmetic expansion shares the quoted string. The first nine
/// were dropped by master; the last four were already preserved and
/// pin the boundary of the class.
#[test]
fn issue2210_corruption_matrix() {
    let case = make_case("matrix");
    let out = run_script(
        &case,
        r#"for esc in '\(' '\)' '\$' '\\' '\"' '\`' '\;' '\|' '\a' '\1' '\/' '\ ' '\n'; do
  body="printf '[%s]' \"a${esc}b\""
  s="x=$((1+1)) $body"
  printf '%s => %s\n' "$esc" "$s"
done
"#,
    );
    let mut expected = String::new();
    for esc in [
        "\\(", "\\)", "\\$", "\\\\", "\\\"", "\\`", "\\;", "\\|", "\\a", "\\1", "\\/", "\\ ", "\\n",
    ] {
        expected.push_str(&format!("{esc} => x=2 printf '[%s]' \"a{esc}b\"\n"));
    }
    assert_eq!(out, expected);
}

/// Argument position (no assignment): the same construction as a
/// command argument corrupts the same way on master.
#[test]
fn issue2210_argument_position_arith_and_var() {
    let case = make_case("argpos");
    let out = run_script(
        &case,
        r#"body='printf "[%s]\n" "a\(b"'
printf '%s\n' "x=$((1+1)) $body"
"#,
    );
    assert_eq!(out, "x=2 printf \"[%s]\\n\" \"a\\(b\"\n");
}

/// Order does not matter: sibling BEFORE the arithmetic expansion.
#[test]
fn issue2210_arith_after_sibling() {
    let case = make_case("after");
    let out = run_script(
        &case,
        r#"body='printf "[%s]\n" "a\(b"'
s="$body x=$((1+1))"
printf '%s\n' "$s"
"#,
    );
    assert_eq!(out, "printf \"[%s]\\n\" \"a\\(b\" x=2\n");
}

/// Sibling shapes that trigger: braced `${body}`, `$*`, and a plain
/// variable in an UNQUOTED word (assignment and argument positions),
/// plus a comsub (in place of the arithmetic expansion) in argument
/// position — all shared the walker's trailing unescape pass.
#[test]
fn issue2210_sibling_forms() {
    let case = make_case("siblings");
    let out = run_script(
        &case,
        r#"body='printf "[%s]\n" "a\(b"'
body2='a\(b'
s="x=$((1+1)) ${body}"
printf 'braced: %s\n' "$s"
set -- "$body"
s="x=$((1+1)) $*"
printf 'star: %s\n' "$s"
printf 'unquoted-arg-arith: %s\n' x=$((1+1))$body2
printf 'unquoted-arg-comsub: %s\n' x=$(echo 1)$body2
printf 'quoted-arg-comsub: %s\n' "x=$(echo 1) $body"
"#,
    );
    assert_eq!(
        out,
        "braced: x=2 printf \"[%s]\\n\" \"a\\(b\"\n\
         star: x=2 printf \"[%s]\\n\" \"a\\(b\"\n\
         unquoted-arg-arith: x=2a\\(b\n\
         unquoted-arg-comsub: x=1a\\(b\n\
         quoted-arg-comsub: x=1 printf \"[%s]\\n\" \"a\\(b\"\n"
    );
}

/// Data classes beyond `\(`: a doubled backslash pair, a
/// backslash-space, and a backslash-dollar inside the sibling value
/// are data, not escape syntax.
#[test]
fn issue2210_data_backslash_classes() {
    let case = make_case("classes");
    let out = run_script(
        &case,
        r#"body3='a\\b'
body4='a\ b'
body5='a\$b'
printf 'pair: %s\n' "x=$((1+1)) $body3"
printf 'space: %s\n' "x=$((1+1)) $body4"
printf 'dollar: %s\n' "x=$((1+1)) $body5"
"#,
    );
    assert_eq!(
        out,
        "pair: x=2 a\\\\b\nspace: x=2 a\\ b\ndollar: x=2 a\\$b\n"
    );
}

/// A SOURCE-literal `\(` inside the double quotes is not escape syntax
/// either (inside `"` a backslash is special only before $ ` " \ or
/// newline — subst.c CBSDQUOTE): GNU keeps both bytes. Master dropped
/// the backslash here too, in assignment and argument positions, and
/// also after a resolved `\\` pair (`"a\\(b"` -> `a\(b`).
#[test]
fn issue2210_source_literal_in_dquotes() {
    let case = make_case("srclit");
    let out = run_script(
        &case,
        r#"s="x=$((1+1)) a\(b"
printf 'assign: %s\n' "$s"
printf 'arg: %s\n' "x=$(echo 1) a\(b"
printf 'after-pair: %s\n' "a\\(b $((1+1))"
s="x=$((1+1)) a\;b"
printf 'semicolon: %s\n' "$s"
"#,
    );
    assert_eq!(
        out,
        "assign: x=2 a\\(b\narg: x=1 a\\(b\nafter-pair: a\\(b 2\nsemicolon: x=2 a\\;b\n"
    );
}

/// `local` and `export` assignments take the same quoted-RHS path.
#[test]
fn issue2210_local_and_export_assignments() {
    let case = make_case("localexp");
    let out = run_script(
        &case,
        r#"body='printf "[%s]\n" "a\(b"'
f() { local t="x=$((1+1)) $body"; printf 'local: %s\n' "$t"; }
f
export s2="x=$((1+1)) $body"
printf 'export: %s\n' "$s2"
"#,
    );
    assert_eq!(
        out,
        "local: x=2 printf \"[%s]\\n\" \"a\\(b\"\nexport: x=2 printf \"[%s]\\n\" \"a\\(b\"\n"
    );
}

/// A redirect target built the same way names the file with the
/// backslash intact. Redirect targets expand through the &self
/// `expand_word` gate (expand_word.rs) over the NON-mut walker; the
/// expansion-result protection was ported there under rubash#2216
/// (this pin was parked #[ignore]d under #2210 until that port
/// landed).
#[test]
fn issue2210_redirect_target() {
    let case = make_case("redir");
    let out = run_script(
        &case,
        r#"body2='a\(b'
printf 'hi\n' > "t_$((1+1))_$body2"
printf '%s\n' t_*
"#,
    );
    assert_eq!(out, "t_2_a\\(b\n");
}

/// rubash#2216 sibling (probe3 R4): the gate also opens on a command
/// substitution in the source word; a sibling variable's data
/// backslash in the same redirect operand stays intact.
#[test]
fn issue2216_redirect_comsub_trigger() {
    let case = make_case("redir-comsub");
    let out = run_script(
        &case,
        r#"body2='a\(b'
printf 'hi\n' > "u_$(echo 1)_$body2"
printf '%s\n' u_*
"#,
    );
    assert_eq!(out, "u_1_a\\(b\n");
}

/// rubash#2216 sibling (probe3 R5): the corrupted backslash comes
/// from the redirect operand's own double-quoted source literal
/// rather than from a variable's value.
#[test]
fn issue2216_redirect_source_literal() {
    let case = make_case("redir-literal");
    let out = run_script(
        &case,
        r#"printf 'hi\n' > "u_$((1+1))_a\(b"
printf '%s\n' u_*
"#,
    );
    assert_eq!(out, "u_2_a\\(b\n");
}

/// rubash#2216 boundary control (probe3 R3): a redirect operand with
/// the variable but no `$(`/backtick in the source word never opens
/// the trailing-unescape gate; the backslash survives on both the
/// pre- and post-fix engine.
#[test]
fn issue2216_redirect_var_only_control() {
    let case = make_case("redir-varonly");
    let out = run_script(
        &case,
        r#"body2='a\(b'
printf 'hi\n' > "u_$body2"
printf '%s\n' u_*
"#,
    );
    assert_eq!(out, "u_a\\(b\n");
}

/// The git test-lib shape end to end: a body string carried through a
/// `$*` embedding whose first line holds the arithmetic expansion,
/// then `eval`'d — a `sed` spy must receive the body's `\(` `\)` `\1`
/// byte-for-byte (this is the t0060 #84/#85/#86 mechanism).
#[test]
fn issue2210_eval_embedding_sed_spy() {
    let case = make_case("evalspy");
    let out = run_script(
        &case,
        r#"sed() { printf 'spy2=[%s]\n' "$2"; }
body='d=$(echo /x/y/z/w | sed -e "s|^\([^/]*/[^/]*\)/.*|\1|"); echo "d=[$d]"'
test_eval_() {
  eval "
    trace_level_=$(($1+1))
    $2"
}
test_eval_ 0 "$body"
"#,
    );
    assert_eq!(out, "d=[spy2=[s|^\\([^/]*/[^/]*\\)/.*|\\1|]]\n");
}

/// Boundary controls that were already correct on master and must
/// stay correct: no arithmetic expansion anywhere near the sibling,
/// a comsub sibling carrying the backslashes (command-substitution
/// output has its own protection), an unquoted assignment RHS, and a
/// heredoc body.
#[test]
fn issue2210_boundary_controls() {
    let case = make_case("controls");
    let out = run_script(
        &case,
        r#"body='printf "[%s]\n" "a\(b"'
body2='a\(b'
s="x=1 $body"
printf 'no-arith: %s\n' "$s"
s="x=$((1+1)) $(printf 'a\(b')"
printf 'comsub-sibling: %s\n' "$s"
s=x=$((1+1))$body2
printf 'unquoted-rhs: %s\n' "$s"
cat <<EOF
heredoc: x=$((1+1)) $body2
EOF
"#,
    );
    assert_eq!(
        out,
        "no-arith: x=1 printf \"[%s]\\n\" \"a\\(b\"\n\
         comsub-sibling: x=2 a\\(b\n\
         unquoted-rhs: x=2a\\(b\n\
         heredoc: x=2 a\\(b\n"
    );
}

/// Overcorrection guards: SOURCE escape pairs in unquoted words still
/// resolve (that resolution is the trailing pass's legitimate job),
/// and double-quoted source escapes keep GNU's dquote set semantics,
/// all with an arithmetic expansion present in the word.
#[test]
fn issue2210_source_escapes_still_resolve() {
    let case = make_case("srcresolve");
    let out = run_script(
        &case,
        r#"printf 'unquoted-arith: %s\n' a\(b$((1+1))
printf 'unquoted-comsub: %s\n' a\(b$(echo 2)
printf 'dq-backslash: %s\n' "a\\b $((1+1))"
printf 'dq-dollar: %s\n' "a\$b $((1+1))"
printf 'dq-squote: %s\n' "a\'b $((1+1))"
"#,
    );
    assert_eq!(
        out,
        "unquoted-arith: a(b2\n\
         unquoted-comsub: a(b2\n\
         dq-backslash: a\\b 2\n\
         dq-dollar: a$b 2\n\
         dq-squote: a\\'b 2\n"
    );
}
