//! Issue rubash#463: a word-attached multi-line process substitution —
//! `word<(` NEWLINE ... NEWLINE `)` — was rejected with `syntax error near
//! unexpected token `)'` while GNU accepts it. The word-attached form is ONE
//! word in GNU: read_token's fall-through (parse.y:3793-3797) hands `word<(`
//! to read_token_word, whose shellexp arm (parse.y:5494-5524) consumes the
//! `(...)` body into the token under parse_matched_pair's LEX_GTLT
//! discipline (parse.y:4147-4153), where newlines are ordinary body
//! characters. The fix joins physical lines in the tokenizer's join gate
//! while the last word's `(`/`>(` body is still open, so the word scanner
//! re-lexes the whole multi-line word exactly like the single-line form.
//!
//! Oracle: GNU bash 5.2.37 (msys, D:\Git\usr\bin\bash.exe) `bash -n` rc=0 on
//! every accepted shape below; byte-identical stdout on the executed shapes.
//! Real-world evidence: h4l/json.bash examples/jb-cli.sh (the `jb
//! menu:json@<(...)' nesting example), rc=2 on master before the fix.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
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

fn rubash_n(script: &str) -> (String, String, Option<i32>) {
    let file = std::env::temp_dir().join(format!("issue463-{}.sh", std::process::id()));
    std::fs::write(&file, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg(&file)
        .output()
        .expect("run rubash -n");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_file(&file);
    result
}

/// The issue's minimal repro parses (`-n`) and executes with GNU-identical
/// stdout (`echo outer/dev/fd/63` — the substitution command's output goes
/// to the pipe, exactly like GNU).
#[test]
fn minimal_repro_parses_and_executes() {
    let script = "echo outer<(\necho inner\n)\n";
    let (_, stderr, code) = rubash_n(script);
    assert_eq!(code, Some(0), "rubash -n rejected the repro: {stderr}");
    assert!(stderr.is_empty(), "unexpected diagnostics: {stderr}");

    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0), "execution failed: {stderr}");
    assert_eq!(stdout, "outer/dev/fd/63\n", "GNU-identical stdout expected");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

/// The whole fix rides on the tokenizer's join gate: the logical line is
/// re-lexed as ONE word when the last word's `<(` body is still open —
/// `echo done` after the closing `)` must run as a separate command.
#[test]
fn multiline_attached_word_spans_lines_and_parsing_continues() {
    let script = "echo before<(\necho hidden\n)\necho after\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "before/dev/fd/63\nafter\n");
    assert!(stderr.is_empty(), "{stderr}");
}

/// The h4l/json.bash jb-cli nesting shape: word-attached via `@`, bodies
/// nested three deep, every body spanning lines.
#[test]
fn json_bash_jb_cli_nesting_shape_parses() {
    let script = concat!(
        "jb menu:json@<(\n",
        "  jb id=file value=File popup:json@<(\n",
        "    jb menuitem:json[]@<(\n",
        "      jb value=New onclick=\"CreateNewDoc()\"\n",
        "    )\n",
        "  )\n",
        ")\n",
    );
    let (_, stderr, code) = rubash_n(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert!(stderr.is_empty());
    // The `@(...)` words are unknown commands, but the PARSE must succeed:
    // GNU rc=0 with `command not found` statuses only.
    let (_, exec_stderr, exec_code) = rubash_file(script);
    assert_eq!(exec_code, Some(127), "exec stderr: {exec_stderr}");
}

/// Output process substitutions (`>(`) attached to a word span lines too.
#[test]
fn multiline_attached_output_procsub() {
    let script = "echo hello > >(cat\n)\necho done\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("hello"), "stdout: {stdout}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
}

/// Execute via a script FILE: the batch path is what json.bash needs, and
/// it sidesteps the pre-existing `-c`-only gap where the space-separated
/// multi-line procsub body mis-parses (present on master, tracked apart
/// from this issue).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let file = std::env::temp_dir().join(format!("issue463-exec-{}.sh", std::process::id()));
    std::fs::write(&file, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&file);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Inside a command substitution the attached form spans lines as well.
#[test]
fn attached_procsub_inside_command_substitution() {
    let script = "x=$(echo outer<(\necho inner\n))\necho \"$x\"\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "outer/dev/fd/63\n", "GNU-identical stdout expected");
    assert!(stderr.is_empty(), "{stderr}");
}

/// Single-line attached forms (all previously working shapes) stay intact.
#[test]
fn single_line_attached_forms_unchanged() {
    for script in [
        "echo outer<(echo inner)\n",
        "cat <(echo hi)\n",
        "echo <(echo hi)\n",
        "echo outer<(\necho inner\n)\n",
    ] {
        let (stdout, stderr, code) = rubash_file(script);
        assert_eq!(code, Some(0), "script {script:?}: {stderr}");
        assert!(!stdout.is_empty(), "script {script:?} produced no output");
    }
}

/// rubash#424 must not regress: the extglob redirect rejections in the
/// neighbouring lexer region keep their syntax errors (GNU-verified rc=2).
#[test]
fn issue424_extglob_redirect_rejections_intact() {
    // Backslash-escaped extglob introducer before a redirect target is a
    // parse error, not a literal filename redirect.
    let (_, stderr, code) = rubash_n("echo x >\\@(zz)x\n");
    assert_eq!(code, Some(2), "expected GNU parse rejection, got: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );

    // A bare `<`/`>` operator inside a compound assignment is still an
    // operator (GNU rejects `a=(<x)` — #339 keeps it an operator).
    let (_, stderr, code) = rubash_n("a=(<x)\n");
    assert_ne!(code, Some(0), "expected parse rejection, got: {stderr}");
    assert!(stderr.contains("syntax error"), "stderr: {stderr}");

    // A bare `<(` at word START (space-separated) keeps the parser's own
    // multi-line bridge: the join gate only holds for word-ATTACHED
    // introducers. Parse acceptance per the issue's boundary table
    // (execution of the space form has a pre-existing `-c`/script gap
    // unrelated to this fix).
    let (_, stderr, code) = rubash_n("cat <(\necho hi\n)\necho done\n");
    assert_eq!(code, Some(0), "expected parse acceptance, got: {stderr}");
}

/// rubash#468 (procsub /dev/fd fd passing) must not regress: multi-line
/// bodies still execute their commands exactly once, in order.
#[test]
fn multiline_body_executes_once_with_fd_output() {
    let script = "v=$(cat <(\necho body\n))\necho \"$v\"\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "body\n", "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

/// A quoted `<(` never opens a join: the quotes are word text.
#[test]
fn quoted_opener_does_not_join() {
    let script = "echo 'a<('\necho done\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "a<(\ndone\n", "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}
