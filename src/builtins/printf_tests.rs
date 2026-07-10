use super::*;
use std::collections::HashMap;

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

#[test]
fn float_formats_honor_sign_flags_and_alternate_decimal_point() {
    assert_eq!(
        run(&[
            "<%+010.0f><% 010.0f><%#4.0f><%#.0f><%+10.0f>",
            "123",
            "123",
            "123",
            "123",
            "123"
        ])
        .1,
        "<+000000123>< 000000123><123.><123.><      +123>"
    );
}

#[test]
fn float_formats_use_bash_default_precision_and_exponents() {
    assert_eq!(
        run(&["<%f><%F><%e><%E>", "4", "4", "4", "4"]).1,
        "<4.000000><4.000000><4.000000e+00><4.000000E+00>"
    );
}

#[test]
fn general_float_formats_match_bash_significant_digits() {
    assert_eq!(
        run(&[
            "<%.4g><%.4g><%.4g><%.4g><%.4G><%6.2g><%6.2G>",
            "12345",
            "0.00012345",
            "123.44",
            "0",
            "12345",
            "4.2",
            "4.2"
        ])
        .1,
        "<1.234e+04><0.0001234><123.4><0><1.234E+04><   4.2><   4.2>"
    );
}

#[test]
fn alternate_general_float_formats_keep_decimal_zeroes() {
    assert_eq!(
        run(&["<%#.0g><%#.4g><%#.4e><%#.0e>", "4", "123.44", "4", "4"]).1,
        "<4.><123.4><4.0000e+00><4.e+00>"
    );
}

#[test]
fn hex_float_formats_match_bash_precision_and_flags() {
    assert_eq!(
        run(&[
            "<%.0a><%.2a><%10.2a><%+.2a><% .2a><%.2A>",
            "4.2",
            "4.2",
            "4.2",
            "4.2",
            "4.2",
            "4.2"
        ])
        .1,
        "<0x1p+2><0x1.0dp+2>< 0x1.0dp+2><+0x1.0dp+2>< 0x1.0dp+2><0X1.0DP+2>"
    );
}

#[test]
fn hex_float_formats_handle_zero_integer_and_alternate_form() {
    assert_eq!(
        run(&["<%a><%a><%a><%#a>", "0", "-0", "1", "4"]).1,
        "<0x0p+0><-0x0p+0><0x1p+0><0x1.p+2>"
    );
}

#[test]
fn integer_formats_parse_bash_numeric_bases() {
    assert_eq!(
        run(&[
            "%d:%d:%d:%i:%u:%x:<%*s>",
            "0x1a",
            "032",
            "-010",
            "010",
            "0x10",
            "032",
            "010",
            "x"
        ])
        .1,
        "26:26:-8:8:16:1a:<       x>"
    );
}

#[test]
fn integer_formats_apply_precision_like_bash() {
    assert_eq!(
        run(&[
            "<%.5d><%8.5d><%08.5d><%.0d><%+.0d><% .0d><%#.5o><%#.5x><%#.0o>",
            "42",
            "42",
            "42",
            "0",
            "0",
            "0",
            "9",
            "26",
            "0"
        ])
        .1,
        "<00042><   00042><   00042><><+>< ><00011><0x0001a><0>"
    );
}

#[test]
fn percent_b_backslash_c_stops_all_output() {
    assert_eq!(run(&["<%b>tail\n", "a\\cB"]).1, "<a");
    assert_eq!(run(&["X%bY%sZ\n", "a\\c", "later"]).1, "Xa");
}
