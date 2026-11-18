//! rubash#3502 follow-up — the bad-substitution pre-scan
//! (parameter_expansion_error_in_word_context) must exempt `${ cmd; }`
//! funsub bodies: GNU param_expand routes `${` + FUNSUB_CHAR
//! (parser.h:85) to function_substitute (subst.c:6917) before
//! parameter_brace_expand, so valid_brace_expansion_word
//! (subst.c:10053/7583) never judges the body as a parameter name.
//! #3855's whole-inner braced_name_is_bad_substitution check read
//! ` case x in x) esac; ` / ` echo 4; ` as illegal names and turned
//! every funsub form reaching the scan into `bad substitution`.
//!
//! Expectations verified against WSL GNU Bash 5.3.20 script-file
//! probes (scratch-j1/funsub-badsub-cmp.sh -> *.gnu.out/err).

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

#[test]
fn funsub_header_in_arithmetic_for_is_not_a_bad_substitution() {
    // The i1-audit reproducer: `${ case x in x) esac; }` is a funsub
    // whose body is command text; the loop runs and the script
    // continues (GNU exit 0, prints `after`).
    let (stdout, stderr, code) =
        rubash("for (( ${ case x in x) esac; };; )); do break; done; echo after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn funsub_assignment_rhs_expands() {
    let (stdout, _, code) = rubash("v=${ echo 4; }; echo \"[$v] rc=$?\"\n");
    assert_eq!(stdout, "[4] rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn funsub_inside_operator_default_word() {
    // `${v:-${ echo d; }}` with v unset: the `:-` word is expanded and
    // the funsub inside it runs (GNU prints `d`).
    let (stdout, _, code) = rubash("unset v; echo \"${v:-${ echo d; }}\"\n");
    assert_eq!(stdout, "d\n");
    assert_eq!(code, Some(0));
}

#[test]
fn funsub_inside_question_message_word_runs_then_reports_outer() {
    // GNU parameter_brace_expand_error expands the `:?` message word
    // first (subst.c:8224): the funsub inside it executes and its
    // output becomes the reported message — `v: hi`, fatal 127.
    let (stdout, stderr, code) = rubash("unset v; echo \"${v:?${ echo hi; }}\"; echo after\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("v: hi"), "stderr: {stderr}");
    assert!(!stderr.contains("bad substitution"), "stderr: {stderr}");
    assert_eq!(code, Some(127));
}

#[test]
fn embedded_funsub_mid_word_expands() {
    let (stdout, _, code) = rubash("x=pre${ echo mid; }post; echo \"[$x]\"\n");
    assert_eq!(stdout, "[premidpost]\n");
    assert_eq!(code, Some(0));
}

#[test]
fn funsub_inside_double_quotes_expands() {
    let (stdout, _, code) = rubash("echo \"${ echo q; }\"\n");
    assert_eq!(stdout, "q\n");
    assert_eq!(code, Some(0));
}

#[test]
fn funsub_body_brace_as_word_text_uses_command_position_close() {
    // `${ echo a}b; }`: the first `}` is word text (term==false), the
    // funsub closes at the second `}` — GNU captures `a}b`.
    let (stdout, _, code) = rubash("v=${ echo a}b; }; echo \"[$v] rc=$?\"\n");
    assert_eq!(stdout, "[a}b] rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn funsub_tab_and_newline_introducers_expand() {
    let (stdout, _, code) = rubash("v=${\techo t;\t}; echo \"[$v]\"\n");
    assert_eq!(stdout, "[t]\n");
    assert_eq!(code, Some(0));
    let (stdout, _, code) = rubash("v=${\necho n;\n}; echo \"[$v]\"\n");
    assert_eq!(stdout, "[n]\n");
    assert_eq!(code, Some(0));
}

#[test]
fn bad_substitution_still_fires_next_to_funsub() {
    // The exemption is per-span: a real bad substitution in the same
    // word still reports, naming the whole word (GNU cites the word
    // being expanded, subst.c:10278).
    let (stdout, stderr, code) = rubash("echo \"${ echo ok; }${-x}\"; echo after\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("${ echo ok; }${-x}: bad substitution"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
    let (_, stderr, _) = rubash("echo \"${-x}\"; echo after\n");
    assert!(
        stderr.contains("${-x}: bad substitution"),
        "stderr: {stderr}"
    );
    let (_, stderr, _) = rubash("echo \"${(M)y}\"; echo after\n");
    assert!(
        stderr.contains("${(M)y}: bad substitution"),
        "stderr: {stderr}"
    );
}
