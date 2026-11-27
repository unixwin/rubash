//! Issue rubash#302 and rubash#304 regressions (wt11/lexmix lane).
//!
//! #302: a `function NAME()` header whose body `{` sits on the NEXT physical
//! line died with `syntax error near unexpected token '('` whenever the file
//! was read through the incremental group reader — the `source` builtin and
//! piped stdin. The completeness predicate
//! (`script_driver::stdin_source_is_function_signature`) knew the
//! `name()` form and the `function NAME` form but not `function NAME()`:
//! its parens-peel arm reduced the header to `function f` (not a single
//! WORD) and RETURNED false instead of falling through to the
//! keyword-form arm. GNU parse.y:1054-1061 `function_def` has four
//! productions — `WORD '(' ')' newline_list function_body`,
//! `FUNCTION WORD '(' ')' newline_list function_body`,
//! `FUNCTION WORD function_body`, and `FUNCTION WORD '\n' newline_list
//! function_body` — so the newline between `()` and the body is explicit
//! grammar. The reporter's extglob was incidental (the failure reproduces
//! with no `shopt` at all); the OMB svn completion hit it because
//! `svn.completion.sh` does `shopt -s extglob` at line 40 and defines
//! `function _svn_read_hashfile()` at line 45.
//!
//! #304: `echo $(echo hi <<< y)#tail` reported `command substitution:
//! unexpected EOF while looking for matching ')'` when the unquoted `$(`
//! started the word. Root cause: the expansion-layer comsub collector
//! (`executor::collect_command_substitution_source_ex`) skipped its
//! heredoc branch at the FIRST `<` of `<<<` (its lookahead saw the third
//! `<`), but the scan loop then re-entered the branch at the SECOND `<`,
//! whose shifted lookahead no longer saw a third `<`: `<<< y)` was read as
//! `<<` with delimiter `y` and the `)#tail` tail was swallowed as
//! heredoc-body data, never closing the substitution. The twin collector
//! `executor::copy_command_substitution_heredoc` had the same flaw (the
//! third `<` became the first delimiter byte). GNU lexes `<<<` as ONE
//! here-string operator before any heredoc gathering: parse.y:3692-3704
//! read_token `case '<'` — after `<<`, one more char is read and a third
//! `<` returns LESS_LESS_LESS (grammar parse.y:664
//! `LESS_LESS_LESS WORD`); a here-string never enters the LESS_LESS
//! heredoc path. `)` immediately followed by `#` is word text, not a
//! comment (parse.y:3630-3643: `#` introduces a comment only at a token
//! boundary, and the word already began with the substitution).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; the 30-case
//! matrix lives at target/issue-suites/results/issue304/matrix304b.sh).

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Run a script FILE (the incremental group reader path `source` shares)
/// in its own scratch directory and return (stdout, stderr, code).
fn rubash_file(script: &str, extra_args: &[&str]) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i302-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(extra_args)
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `source` a file from a driver script in the same directory.
fn rubash_source(sourced: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i302s-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("body.sh"), sourced).expect("write body.sh");
    std::fs::write(dir.join("driver.sh"), "source ./body.sh; echo rc=$?\n")
        .expect("write driver.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash source driver");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Pipe a script through stdin (the other incremental reader path).
fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---------------------------------------------------------------------------
// rubash#302: `function NAME()` header read through the group reader
// ---------------------------------------------------------------------------

/// The reporter's exact shape: `shopt -s extglob` then `function f()` with
/// the body brace on the next line, inside a SOURCEd file.
#[test]
fn sourced_extglob_then_function_paren_header() {
    let (stdout, stderr, _) = rubash_source("shopt -s extglob\nfunction f()\n{\n  :\n}\necho OK\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\nrc=0\n");
}

/// Same header without any shopt — proves the root cause is the header
/// form, not extglob (the issue title's attribution).
#[test]
fn sourced_function_paren_header_without_extglob() {
    let (stdout, stderr, _) = rubash_source("function f()\n{ :; }\nf\necho OK\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\nrc=0\n");
}

/// `function f ()` — spaced parens, body on the next line.
#[test]
fn sourced_function_spaced_paren_header() {
    let (stdout, stderr, _) = rubash_source("function f ()\n{ :; }\nf\necho OK\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\nrc=0\n");
}

/// Piped stdin takes the same incremental reader and must also keep the
/// group open across the header line.
#[test]
fn stdin_function_paren_header() {
    let (stdout, stderr, _) = rubash_stdin("shopt -s extglob\nfunction f()\n{\n  :\n}\necho OK\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\n");
}

/// A function body spanning several lines keeps the group open until the
/// closing brace (stdin_source_has_unclosed_function_body keyword form).
#[test]
fn sourced_function_paren_header_multiline_body() {
    let (stdout, stderr, _) =
        rubash_source("function f()\n{\n  echo a\n  echo b\n}\nf\necho done\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "a\nb\ndone\nrc=0\n");
}

/// Complete one-line headers and the plain `name()` form are unaffected.
#[test]
fn complete_function_headers_still_parse() {
    let (stdout, stderr, _) = rubash_file("function f() { :; }\nf\necho OK\n", &[]);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\n");

    let (stdout, stderr, _) = rubash_file("f()\n{ :; }\nf\necho OK\n", &[]);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\n");

    let (stdout, stderr, _) = rubash_file("function f\n{ :; }\nf\necho OK\n", &[]);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\n");
}

/// Over-acceptance guard: a COMPLETE header-plus-body line is not a bare
/// signature — the group reader must not wait for more input (which would
/// misplace diagnostics and stall piped stdin).
#[test]
fn complete_function_line_is_not_a_signature() {
    let (stdout, stderr, _) = rubash_stdin("function f() { :; }\necho OK\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "OK\n");
    // `function f g` is not a function_def (parse.y:1058 wants the body
    // right after ONE WORD): submitting it must produce GNU's syntax
    // error, not an endless continuation.
    let (stdout, stderr, code) = rubash_file("function f g\n{ :; }\n", &[]);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("syntax error"),
        "expected syntax error, stderr: {stderr}"
    );
    assert_eq!(code, Some(2));
}

// ---------------------------------------------------------------------------
// rubash#304: `<<<` herestring inside a comsub body, `#` word text after `)`
// ---------------------------------------------------------------------------

/// The reporter's exact line: `)` immediately followed by `#` is word
/// text (the word began with the substitution), GNU prints `hi#tail`.
#[test]
fn comsum_herestring_hash_tail() {
    let (stdout, stderr, _) = rubash_c("echo $(echo hi <<< y)#tail");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "hi#tail\n");
}

/// Word-prefixed, quoted, assignment-RHS and spaced-comment variants.
#[test]
fn comsum_herestring_hash_tail_positions() {
    let (stdout, stderr, _) = rubash_c("echo foo$(echo hi <<< y)#tail");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "foohi#tail\n");

    let (stdout, stderr, _) = rubash_c("echo \"pre$(echo hi <<< y)#tail\"");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "prehi#tail\n");

    let (stdout, stderr, _) = rubash_c("x=$(cat <<< y)#tail; echo \"[$x]\"");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "[y#tail]\n");

    // Space before `#` makes it a comment (token boundary).
    let (stdout, stderr, _) = rubash_c("echo $(echo hi <<< y) #c");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "hi\n");
}

/// Herestring operand mid-word `#`, nested comsum, pipelines and multiple
/// substitutions per line — the collector must close at the FIRST `)`.
#[test]
fn comsum_herestone_tail_shapes() {
    let (stdout, stderr, _) = rubash_c("echo $(cat <<< y#tail)");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "y#tail\n");

    let (stdout, stderr, _) = rubash_c("echo $(cat <<< $(echo sub))#t");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "sub#t\n");

    let (stdout, stderr, _) = rubash_c("echo $(cat <<< y | tr a-z A-Z)#t");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "Y#t\n");

    let (stdout, stderr, _) = rubash_c("echo $(echo hi <<< y)#tail $(echo again <<< z)#t2");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "hi#tail again#t2\n");

    let (stdout, stderr, _) = rubash_c("echo a$(echo b <<< c)d$(echo e <<< f)g#h");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "abdeg#h\n");
}

/// A herestring operand that IS a quoted `#`-word stays data, and `#`
/// right after `<<<` still introduces a comment (GNU read_token lexes the
/// operator, so the operand position starts a fresh token).
#[test]
fn comsum_herestring_hash_operand_boundaries() {
    let (stdout, stderr, _) = rubash_c("echo $(cat <<< 'sq # not comment')#t");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "sq # not comment#t\n");

    // GNU: `#` directly after `<<<` is a comment, so the substitution
    // never closes -> syntax error near `)' (verified vs WSL 5.3.0).
    let (stdout, _, code) = rubash_c("echo $(echo hi <<< #tail)");
    assert!(stdout.is_empty());
    assert_eq!(code, Some(2));
}

/// The heredoc (LESS_LESS) path inside comsum bodies must be untouched:
/// `<<EOT ... EOT )#tail` still collects the body and the tail stays word
/// text.
#[test]
fn comsum_heredoc_tail_unaffected() {
    let (stdout, stderr, _) = rubash_c("echo $(echo hi <<'EOT'\nbody\nEOT\n)#after");
    assert!(stderr.is_empty());
    assert_eq!(stdout, "hi#after\n");
}
