//! rubash#388: with `shopt -s expand_aliases` in effect, a
//! `function NAME<TAB># comment` head stopped parsing as a function
//! definition — the body executed inline and stderr showed
//! `line 2: function: command not found` where GNU 5.3.0 defines the
//! functions silently (this broke third_party/bash/examples/functions/
//! dirstack wholesale).
//!
//! GNU spec:
//! - parse.y:1056-1061 `function_def: FUNCTION WORD function_body` — the
//!   parser awaits the body across newlines; a comment after the head is
//!   discarded by the reader before it can become a token.
//! - parse.y:3630 + 3937-3940: a `#` that BEGINS a word (retind == 0 or
//!   after a shellblank) comments through end of line; a `#` glued into a
//!   word (`d#x`) is name text.
//!
//! Root cause: scripts whose text mentions `expand_aliases` take the
//! grouped script driver (script_uses_aliases), whose group-completeness
//! battery checks `stdin_source_is_function_signature` on the RAW pending
//! text — a trailing word-initial `#` comment defeated the keyword-form
//! head validation (`function d\t# note` failed the whitespace-free name
//! check), so the head line was declared a COMPLETE command and the body
//! parsed separately. The two signature predicates now strip every
//! word-initial unquoted comment SPAN (through EOL, the newline kept)
//! before head-shape validation. Spans are cut per line, never to end of
//! text, so a complete definition whose head carries a comment still
//! closes its group at the balanced `}` (verified: with pipe stdin the
//! trailing `read` then consumes the next script line like GNU, where a
//! to-end-of-text cut glued the whole script and left `read` at EOF).
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes (target/probe-wt37b/ and
//! target/issue-suites/results/wt37-gapfix1/issue388-matrix/, 2026-10-02).

use std::process::Command;

fn rubash(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (output.stdout, output.stderr, output.status.code())
}

fn assert_defines_and_runs(head: &str, call: &str, expected: &[u8]) {
    let script = format!("shopt -s expand_aliases\n{head}\n{call}\necho after\n");
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(code, Some(0), "rc for {script:?}");
    assert_eq!(stdout, expected, "stdout for {script:?}");
    assert!(
        stderr.is_empty(),
        "stderr for {script:?}: {:?}",
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn function_head_tab_comment_defines_under_expand_aliases() {
    // The #388 reproducer shape.
    assert_defines_and_runs(
        "function d\t# display the directory stack\n{\n\techo ran-d\n}",
        "d",
        b"ran-d\nafter\n",
    );
}

#[test]
fn function_head_space_and_quoted_comment_text() {
    // Quote characters inside the comment are inert.
    assert_defines_and_runs(
        "function e # comment with 'quote and $dollar\n{ echo ran-e; }",
        "e",
        b"ran-e\nafter\n",
    );
}

#[test]
fn function_head_comment_after_parens() {
    assert_defines_and_runs(
        "function f()\t# comment after parens\n{ echo ran-f; }",
        "f",
        b"ran-f\nafter\n",
    );
}

#[test]
fn full_line_comment_between_head_and_body() {
    assert_defines_and_runs(
        "function i # head comment\n# full-line comment between head and body\n{\n\techo ran-i\n}",
        "i",
        b"ran-i\nafter\n",
    );
}

#[test]
fn comment_line_before_head_and_no_shopt() {
    assert_defines_and_runs(
        "function j2 # head comment\n{ echo ran-j2; }",
        "j2",
        b"ran-j2\nafter\n",
    );
    // Same head without any shopt involvement.
    let (stdout, stderr, code) = rubash("function k # c\n{ echo ran-k; }\nk\necho after\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ran-k\nafter\n");
    assert!(stderr.is_empty(), "{stderr:?}");
}

#[test]
fn hash_glued_into_name_is_not_a_comment() {
    // parse.y:3937-3940: `#` mid-word is name text — the head is still a
    // head (GNU: defines `d#x`; the call below uses the exact spelling).
    let (stdout, stderr, code) =
        rubash("shopt -s expand_aliases\nfunction d#x\n{ echo ran-dx; }\nd#x\necho after\n");
    assert_eq!(code, Some(0), "{stderr:?}");
    assert_eq!(stdout, b"ran-dx\nafter\n");
    assert!(stderr.is_empty(), "{stderr:?}");
}

#[test]
fn complete_definition_with_head_comment_is_one_command() {
    // The comment must not make a COMPLETE `head + body` group look
    // incomplete: strip cuts only the comment's line, the body braces
    // then fail the head-only shape.
    let (stdout, stderr, code) = rubash(
        "shopt -s expand_aliases\nfunction m # note\n{ echo ran-m; }\n\
         echo mid\nfunction n # note\n{ echo ran-n; }\nn\necho after\n",
    );
    assert_eq!(code, Some(0), "{stderr:?}");
    assert_eq!(stdout, b"mid\nran-n\nafter\n");
    assert!(stderr.is_empty(), "{stderr:?}");
}

#[test]
fn complete_definition_with_head_comment_closes_the_group() {
    // GNU 5.3.0, pipe stdin (`cat g1.sh | bash -s`): EMPTY stdout, rc 0 —
    // the group closes at the balanced `}`, so the trailing `read`
    // consumes the last script line as data. A group glued open by the
    // head comment would execute `read` with stdin at EOF and print
    // `x=[]` (verified divergence of the to-end-of-text cut variant).
    use std::io::Write;
    use std::process::{Output, Stdio};
    let script = b"shopt -s expand_aliases\nfunction d # c\n{ :; }\nread x\necho \"x=[$x]\"\n";
    let run = |bin: &str| -> Output {
        let mut child = Command::new(bin)
            .arg("-s")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn shell");
        child
            .stdin
            .take()
            .expect("stdin pipe")
            .write_all(script)
            .expect("feed script");
        child.wait_with_output().expect("collect output")
    };
    let output = run(env!("CARGO_BIN_EXE_rubash"));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        b"",
        "stdout leaked: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        output.stderr,
        b"",
        "stderr leaked: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn comment_glued_after_parens_is_still_a_comment() {
    // read_token fetches `)` as its own token, so a `#` right after it is
    // read at token start and comments (verified on GNU 5.3.0:
    // `f()# c` + body defines f and runs it).
    assert_defines_and_runs(
        "i2()# glued after paren\n{ echo ran-i2; }",
        "i2",
        b"ran-i2\nafter\n",
    );
}

#[test]
fn metachars_inside_comment_text_do_not_end_the_head() {
    // The comment span runs to EOL; `(` `|` `;` inside its text are inert.
    assert_defines_and_runs(
        "function p # (note | with ; metachars)\n{ echo ran-p; }",
        "p",
        b"ran-p\nafter\n",
    );
}

#[test]
fn escaped_hash_in_name_is_not_a_comment() {
    // `\#` is name text (LEX_PASSNEXT), matching GNU 5.3.0 byte-for-byte:
    // the head parses as a definition attempt whose WORD `q\#r` is then
    // rejected by the executor (`not a valid identifier`), and the later
    // call resolves the escapes to `q#r` (command not found).
    let (stdout, stderr, code) =
        rubash("shopt -s expand_aliases\nfunction q\\#r\n{ echo ran-qr; }\nq\\#r\necho after\n");
    let stderr = String::from_utf8_lossy(&stderr);
    assert_eq!(code, Some(0), "{stderr:?}");
    assert_eq!(stdout, b"after\n");
    assert!(
        stderr.contains("`q\\#r': not a valid identifier"),
        "got {stderr:?}"
    );
    assert!(stderr.contains("q#r: command not found"), "got {stderr:?}");
}
