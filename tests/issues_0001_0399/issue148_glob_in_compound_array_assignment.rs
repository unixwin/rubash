//! Issue rubash#148 regression: pathname expansion of compound array
//! assignment elements that mix quoted and unquoted segments.
//!
//! oh-my-bash `_omb_util_glob_expand` runs `eval -- "$1=($2)"` with nullglob
//! (lib/utils.sh:424), e.g. `eval 'files=("$OSH"/lib/*.bash)'`. GNU re-parses
//! the element text (arrayfunc.c:574 `parse_string_to_word_list`) and hands
//! the words to globbing via arrayfunc.c:610 `expand_words_no_vars` ->
//! subst.c:12602 `glob_expand_word_list`: by then the quote characters are
//! gone and every quoted character is CTLESC-protected (subst.c:4773
//! `quote_string`), so quoting protects only its own characters — the
//! unquoted `*.bash` segment still pathname-expands. Rubash fed the raw
//! token (quotes inline) to the globber, whose word-level guard rejected
//! any word starting with a quote, storing the pattern text literally and
//! killing every oh-my-bash theme (REPORT.md C-1).
//!
//! These tests pin the whole class (both the executor `arr=(...)` path and
//! the `declare -a arr=(...)` storage path), not just the reporter's case.

use std::fs;
use std::process::Command;

fn fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_BIN_EXE_rubash"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("probe148-tests")
        .join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("dir")).unwrap();
    fs::write(dir.join("dir/a.txt"), "").unwrap();
    fs::write(dir.join("dir/b.txt"), "").unwrap();
    fs::write(dir.join("star.txt"), "").unwrap();
    dir
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reporter's exact shape: quoted prefix (or quoted parameter) plus an
/// unquoted glob must pathname-expand (GNU stores the matches).
#[test]
fn quoted_prefix_plus_unquoted_glob_expands() {
    let dir = fixture("quoted-prefix");
    let (stdout, stderr, _) = rubash_in(
        &dir,
        r#"arr=("dir"/*.txt); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<dir/a.txt><dir/b.txt>\n", "stderr: {stderr}");
}

#[test]
fn quoted_parameter_plus_unquoted_glob_expands() {
    let dir = fixture("quoted-param");
    let (stdout, stderr, _) = rubash_in(
        &dir,
        r#"D=dir; arr=("$D"/*.txt); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<dir/a.txt><dir/b.txt>\n", "stderr: {stderr}");
}

#[test]
fn single_quoted_prefix_plus_unquoted_glob_expands() {
    let dir = fixture("sq-prefix");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"arr=('dir'/*.txt); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<dir/a.txt><dir/b.txt>\n");
}

#[test]
fn mid_word_quote_does_not_disable_glob() {
    let dir = fixture("mid-quote");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"arr=(dir"/"*.txt); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<dir/a.txt><dir/b.txt>\n");
}

/// The oh-my-bash `_omb_util_glob_expand` shape: function body runs
/// `eval -- "$1=($2)"` with nullglob + extglob on.
#[test]
fn omb_glob_expand_eval_shape() {
    let dir = fixture("omb-shape");
    let script = r#"
shopt -s nullglob extglob
_omb_util_glob_expand() {
  local __set=$- __shopt __gignore=$GLOBIGNORE
  shopt -u failglob; shopt -s nullglob; shopt -s extglob
  set +f; GLOBIGNORE=
  eval -- "$1=($2)"
}
_omb_util_glob_expand files '{"$D","$D"}/*.{txt,sh}'
printf '<%s>' "${files[@]}"; echo
"#;
    let (stdout, stderr, _) = rubash_in(&dir, &format!("D=dir\n{script}"));
    assert_eq!(
        stdout, "<dir/a.txt><dir/b.txt><dir/a.txt><dir/b.txt>\n",
        "stderr: {stderr}"
    );
}

/// A quoted glob metacharacter is data: no expansion, no CTLESC carrier in
/// the stored element (the storage side of GNU dequote_string,
/// subst.c:4807).
#[test]
fn quoted_metacharacter_stored_without_carrier() {
    let dir = fixture("quoted-meta");
    let (stdout, _, _) = rubash_in(&dir, r#"arr=("dir"/"*"); printf '<%s>' "${arr[@]}"; echo"#);
    assert_eq!(stdout, "<dir/*>\n");
    let (stdout, _, _) = rubash_in(&dir, r#"arr=([0]="*x"); printf '<%s>' "${arr[@]}"; echo"#);
    assert_eq!(stdout, "<*x>\n");
}

/// An unquoted backslash still escapes the metacharacter (control for the
/// re-encoding: unquoted text stays verbatim).
#[test]
fn unquoted_backslash_escape_stays_literal() {
    let dir = fixture("backslash");
    let (stdout, _, _) = rubash_in(&dir, r#"arr=(dir/\*.txt); printf '<%s>' "${arr[@]}"; echo"#);
    assert_eq!(stdout, "<dir/*.txt>\n");
}

/// nullglob removes an unmatched mixed-quoting word (GNU drops it; the old
/// code stored the literal pattern because the word was rejected before
/// the pattern check).
#[test]
fn nullglob_drops_unmatched_mixed_quoting_word() {
    let dir = fixture("nullglob-drop");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"shopt -s nullglob; arr=("dir"/no*match); echo "n=${#arr[@]}""#,
    );
    assert_eq!(stdout, "n=0\n");
}

/// Without nullglob the unmatched word is stored dequoted.
#[test]
fn unmatched_word_without_nullglob_stored_dequoted() {
    let dir = fixture("nomatch-literal");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"shopt -u nullglob; arr=("dir"/no*match); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<dir/no*match>\n");
}

/// failglob reports the dequoted word (GNU: `no match: dir/zzz*`), never a
/// CTLESC carrier, and the assignment aborts.
#[test]
fn failglob_reports_dequoted_pattern() {
    let dir = fixture("failglob");
    let (stdout, stderr, _) = rubash_in(&dir, r#"shopt -s failglob; arr=("dir"/zzz*); echo after"#);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("no match: dir/zzz*"),
        "stderr was: {stderr:?}"
    );
    let _ = (stdout,); // silence unused when assert passes
}

/// The eval route is the same code path as the direct assignment (eval
/// re-parses through the real parser — trap_exec.rs execute_eval_source).
/// Run from a script FILE: the `-c` form would put the inner `"` through
/// the Windows argv boundary, which is a separate concern.
#[test]
fn eval_array_assignment_globs() {
    let dir = fixture("eval-route");
    let script = dir.join("eval-probe.sh");
    fs::write(
        &script,
        "shopt -s nullglob\neval 'arr=(\"dir\"/*.txt)'\nprintf '<%s>' \"${arr[@]}\"\necho\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&script)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "<dir/a.txt><dir/b.txt>\n", "stderr: {stderr}");
}

/// The `declare -a arr=(...)` storage path must behave identically
/// (builtins/declare/storage/array.rs mirrors the executor copy).
#[test]
fn declare_array_assignment_globs() {
    let dir = fixture("declare-route");
    let (stdout, stderr, _) = rubash_in(
        &dir,
        r#"shopt -s nullglob; declare -a arr=("dir"/*.txt); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<dir/a.txt><dir/b.txt>\n", "stderr: {stderr}");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"declare -a b=([0]="*y"); printf '<%s>' "${b[@]}"; echo"#,
    );
    assert_eq!(stdout, "<*y>\n");
}

/// Control: a fully quoted element is never globbed (fast path preserved).
#[test]
fn fully_quoted_element_stays_literal() {
    let dir = fixture("fully-quoted");
    let (stdout, _, _) = rubash_in(&dir, r#"arr=("*"); printf '<%s>' "${arr[@]}"; echo"#);
    assert_eq!(stdout, "<*>\n");
}

/// Guard against over-reach: quote characters in command substitution
/// output are DATA, not quote operators (GNU never re-parses comsub
/// output), so the unquoted `*` there still globs against the literal
/// quote characters and, matching nothing, stays literal.
#[test]
fn command_substitution_output_quotes_are_data() {
    let dir = fixture("comsub-data");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"x=$(printf '"a"*'); y=($x); printf '<%s>' "${y[@]}"; echo"#,
    );
    assert_eq!(stdout, "<\"a\"*>\n");
}

/// Field-split products (unquoted `$var` inside the parens) keep globbing
/// with data metacharacters active: `x='*'; arr=($x)` stores the matches.
#[test]
fn unquoted_parameter_field_split_still_globs() {
    let dir = fixture("field-split");
    let (stdout, _, _) = rubash_in(
        &dir,
        r#"x='*.txt'; arr=($x); printf '<%s>' "${arr[@]}"; echo"#,
    );
    assert_eq!(stdout, "<star.txt>\n");
}
