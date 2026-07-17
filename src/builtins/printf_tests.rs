use super::*;
use std::collections::HashMap;

#[path = "printf_tests/float_numeric.rs"]
mod float_numeric;

fn run(args: &[&str]) -> (i32, String, String, HashMap<String, String>) {
    let mut env_vars = HashMap::new();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = execute_with_io(
        args.iter().copied(),
        &mut env_vars,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    (
        status,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
        env_vars,
    )
}

#[test]
fn prints_plain_and_escaped_format() {
    assert_eq!(run(&["a\\nb"]).1, "a\nb");
}

#[test]
fn format_string_escapes_match_bash() {
    assert_eq!(run(&["\\045\\x41\\u0042\\101"]).1, "%ABA");
    assert_eq!(run(&["4\\.2 one\\ctwo"]).1, "4\\.2 one\\ctwo");
    assert_eq!(run(&["\\0101"]).1, "A");
}

#[test]
fn invalid_format_characters_fail_like_bash() {
    let (status, stdout, stderr, _) = run(&["ab%Mcd\n"]);

    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "ab");
    assert!(stderr.contains("`M': invalid format character"));

    let (status, stdout, stderr, _) = run(&["%10"]);

    assert_eq!(status, EXECUTION_FAILURE);
    assert!(stdout.is_empty());
    assert!(stderr.contains("`%10': missing format character"));
}

#[test]
fn invalid_options_fail_but_double_dash_allows_dash_format() {
    let (status, stdout, stderr, _) = run(&["-x"]);

    assert_eq!(status, EX_USAGE);
    assert!(stdout.is_empty());
    assert!(stderr.contains("invalid option"));

    let (status, stdout, stderr, _) = run(&["--", "-x"]);

    assert_eq!(status, EXECUTION_SUCCESS);
    assert_eq!(stdout, "-x");
    assert!(stderr.is_empty());
}

#[test]
fn invalid_numeric_arguments_render_zero_and_fail() {
    let (status, stdout, stderr, _) = run(&[
        "%d|%o|%x|%.2f|%*s|%.*s",
        "z",
        "+",
        "GNU",
        "nope",
        "bad",
        "x",
        "bad",
        "abc",
    ]);

    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "0|0|0|0.00|x|");
    assert!(stderr.contains("z: invalid number"));
    assert!(stderr.contains("+: invalid number"));
    assert!(stderr.contains("GNU: invalid number"));
    assert!(stderr.contains("nope: invalid number"));
    assert_eq!(stderr.matches("bad: invalid number").count(), 2);
}

#[test]
fn numeric_errors_do_not_stop_reused_formats() {
    let (status, stdout, stderr, _) = run(&["%d ", "z", "1"]);

    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "0 1 ");
    assert!(stderr.contains("z: invalid number"));

    let (status, stdout, stderr, _) = run(&["%d", ""]);

    assert_eq!(status, EXECUTION_SUCCESS);
    assert_eq!(stdout, "0");
    assert!(stderr.is_empty());
}

#[test]
fn reuses_format_until_arguments_are_consumed() {
    assert_eq!(run(&["%s ", "a", "b"]).1, "a b ");
}

#[test]
fn supports_string_numeric_and_b_formats() {
    assert_eq!(
        run(&["%s:%03d:%x:%b", "x", "7", "15", "a\\nb"]).1,
        "x:007:f:a\nb"
    );
}

#[test]
fn assigns_output_with_v() {
    let (_status, stdout, _stderr, env_vars) = run(&["-v", "NAME", "%s", "value"]);

    assert!(stdout.is_empty());
    assert_eq!(env_vars.get("NAME"), Some(&"value".to_string()));
}

#[test]
fn assigns_output_with_compact_v() {
    let (status, stdout, stderr, env_vars) = run(&["-vNAME", "%s", "value"]);

    assert_eq!(status, EXECUTION_SUCCESS);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
    assert_eq!(env_vars.get("NAME"), Some(&"value".to_string()));
}

#[test]
fn compact_v_rejects_invalid_identifier() {
    let (status, stdout, stderr, env_vars) = run(&["-vBAD-NAME", "%s", "value"]);

    assert_eq!(status, EX_USAGE);
    assert!(stdout.is_empty());
    assert!(env_vars.is_empty());
    assert!(stderr.contains("BAD-NAME"));
    assert!(stderr.contains("not a valid identifier"));
}

#[test]
fn percent_n_assigns_character_count_without_output() {
    let (_status, stdout, _stderr, env_vars) = run(&["abc%n:%s", "COUNT", "done"]);

    assert_eq!(stdout, "abc:done");
    assert_eq!(env_vars.get("COUNT"), Some(&"3".to_string()));
}

#[test]
fn percent_n_works_with_v_assignment() {
    let (_status, stdout, _stderr, env_vars) = run(&["-v", "OUT", "ab%ncd", "COUNT"]);

    assert!(stdout.is_empty());
    assert_eq!(env_vars.get("OUT"), Some(&"abcd".to_string()));
    assert_eq!(env_vars.get("COUNT"), Some(&"2".to_string()));
}

#[test]
fn supports_dynamic_width_and_precision() {
    assert_eq!(run(&["<%*.*s>", "10", "4", "abcdef"]).1, "<      abcd>");
    assert_eq!(run(&["<%*s>", "-6", "ab"]).1, "<ab    >");
    assert_eq!(run(&["<%.*s>", "-1", "abcdef"]).1, "<abcdef>");
}

#[test]
fn percent_q_uses_backslash_quoting_for_printable_shell_metacharacters() {
    assert_eq!(
        run(&["<%q><%q><%q>", "a b", "this&that", "~"]).1,
        "<a\\ b><this\\&that><\\~>"
    );
}

#[test]
fn percent_q_and_upper_q_apply_precision_like_bash() {
    assert_eq!(run(&["<%.2q><%.2Q>", "a b", "a b"]).1, "<a\\><a\\ >");
}

#[test]
fn percent_b_decodes_numeric_escapes() {
    assert_eq!(
        run(&["%b", "\\01017 \\1017 \\x417 \\u0041"]).1,
        "A7 A7 A7 A"
    );
}

#[test]
fn numeric_formats_accept_bash_character_constants() {
    assert_eq!(
        run(&[
            "%d:%o:%x:%.2f:%d",
            "'string'",
            "\"string\"",
            "'string'",
            "'string'",
            "GNU"
        ])
        .1,
        "115:163:73:115.00:0"
    );
}

#[test]
fn alternate_integer_formats_add_bash_prefixes() {
    assert_eq!(
        run(&["%#o:%#x:%#X:%#o:%#x", "115", "115", "115", "0", "0"]).1,
        "0163:0x73:0X73:0:0"
    );
}

#[test]
fn signed_integer_formats_honor_sign_flags_and_zero_padding() {
    assert_eq!(
        run(&[
            "<%+d><% d><%+5d><%05d><%+05d>",
            "42",
            "42",
            "42",
            "-42",
            "42"
        ])
        .1,
        "<+42>< 42><  +42><-0042><+0042>"
    );
}
