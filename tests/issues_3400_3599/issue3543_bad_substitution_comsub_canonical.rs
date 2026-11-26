//! rubash#3543 — residual of rubash#3404: a `bad substitution` diagnostic
//! that cites a parser-read word must show every live `$(...)` span in
//! its canonical print_comsub form (`>&2` -> `1>&2`), because GNU's
//! parse_comsub (parse.y:4451) replaces the body with the print_cmd.c
//! serialization at parse time (parse.y:4632; print_cmd.c:1150
//! print_redirection prints both fds of r_duplicating_output).
//!
//! The citation stays RAW for text GNU expands without parse_comsub:
//! here-document bodies (redir.c:348 heredoc_expand -> expand_string)
//! and prompt strings (parse.y:6689 expand_prompt_string, reached here
//! through the `${var@P}` transform). And a `$(` the lexer never
//! recognized — single-quoted or backslash-escaped — is literal word
//! text, not a command substitution, so it cites raw too.
//!
//! Expectations are the GNU bash 5.3.20 oracle's, probed from script
//! files (bad substitution is a non-fatal expansion error: the word
//! expands empty, the command reports status 1, and the script
//! continues).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3543-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's shape: `( : ${-x$(echo hi >&2)} )` reports the canonical
/// `1>&2` form. The diagnostic is non-fatal — the bad word expands
/// empty, `:` still runs inside the subshell, and the script continues
/// (GNU probe: subshell reports status 1, top-level `:` reports 1).
#[test]
fn issue_shape_and_top_level_cite_canonical_comsub() {
    let script = concat!(
        "( : ${-x$(echo hi >&2)} )\n",
        "echo \"sub=$?\"\n",
        ": ${-y$(echo yo >&3)}\n",
        "echo \"top=$?\"\n",
    );
    let (stdout, stderr, code) = rubash_file("shape", script);
    assert_eq!(stdout, "sub=1\ntop=1\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: ${-x$(echo hi 1>&2)}: bad substitution\n",
            "t.sh: line 3: ${-y$(echo yo 1>&3)}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// Runtime text GNU never ran parse_comsub on cites its `$(` spans raw:
/// an unquoted here-document body (redir.c:348 heredoc_expand) and a
/// prompt string expanded through `${var@P}` (parse.y:6689
/// expand_prompt_string). The heredoc citation keeps the body's
/// trailing newline, so the `: bad substitution` lands on the next
/// line — GNU prints it that way too.
#[test]
fn heredoc_body_and_prompt_string_stay_raw() {
    let script = concat!(
        "cat <<EOT >/dev/null\n",
        "${-z$(echo zo >&4)}\n",
        "EOT\n",
        "echo \"hd=$?\"\n",
        "v='${-x$(echo hi >&2)}'\n",
        "echo \"${v@P}\"\n",
        "echo \"p=$?\"\n",
    );
    let (stdout, stderr, code) = rubash_file("runtime", script);
    assert_eq!(stdout, "hd=1\n${-x$(echo hi >&2)}\np=0\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: ${-z$(echo zo >&4)}\n",
            ": bad substitution\n",
            "t.sh: line 6: ${-x$(echo hi >&2)}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// A `$(` the lexer did not recognize as a command substitution is
/// literal word text: the single-quoted span stays raw while the live
/// span canonicalizes, and a backslash-escaped `$(` stays raw inside a
/// double-quoted word. (GNU keeps the quote delimiters and the `\` in
/// the cited word; rubash's word transport already drops them before
/// this diagnostic — a pre-existing display gap outside #3543 — so
/// this asserts the canonical/raw invariant, not the delimiters.)
#[test]
fn quoted_and_escaped_comsubs_stay_raw() {
    let script = concat!(
        ": pre'$(echo sq >&2)'${-x$(echo hi >&2)}post\n",
        ": \"esc\\$(echo esc >&2)${-y$(echo yo >&3)}\"\n",
        "echo DONE\n",
    );
    let (stdout, stderr, code) = rubash_file("quoted", script);
    assert_eq!(stdout, "DONE\n");
    // Line 1: the `'...'` span is literal — `>&2` must NOT canonicalize;
    // the live `${-x$(...)}` span prints `1>&2`.
    assert!(
        stderr.contains("$(echo sq >&2)"),
        "quoted comsub must stay raw: {stderr}"
    );
    assert!(
        !stderr.contains("$(echo sq 1>&2)"),
        "quoted comsub must not canonicalize: {stderr}"
    );
    assert!(
        stderr.contains("${-x$(echo hi 1>&2)}post"),
        "live comsub must canonicalize: {stderr}"
    );
    // Line 2: `\$(` inside double quotes is an escaped dollar — the
    // span is literal `$(...)` text and stays raw.
    assert!(
        stderr.contains("$(echo esc >&2)"),
        "escaped comsub must stay raw: {stderr}"
    );
    assert!(
        !stderr.contains("$(echo esc 1>&2)"),
        "escaped comsub must not canonicalize: {stderr}"
    );
    assert!(
        stderr.contains("${-y$(echo yo 1>&3)}"),
        "live comsub must canonicalize: {stderr}"
    );
    assert_eq!(code, Some(0));
}
