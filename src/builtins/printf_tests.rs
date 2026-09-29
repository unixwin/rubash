use super::*;

#[path = "printf_tests/float_numeric.rs"]
mod float_numeric;

fn run(args: &[&str]) -> (i32, String, String, crate::shell::var_table::VarTable) {
    let mut env_vars = crate::shell::var_table::VarTable::default();
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

fn run_bytes(args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
    let mut env_vars = crate::shell::var_table::VarTable::default();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = execute_with_io(
        args.iter().copied(),
        &mut env_vars,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    (status, stdout, stderr)
}

#[test]
fn integer_overflow_saturates_and_sets_failure_status() {
    let (status, stdout, stderr, _) = run(&[
        "%d|%d|%x",
        "9223372036854775808",
        "-9223372036854775809",
        "0xFFFFFFFFFFFFFFFF",
    ]);

    // GNU printf.def:826-827: printf_erange sets conversion_error, so the
    // builtin exits 1 even though it keeps processing and prints the
    // saturated values (`printf '%d' 9223372036854775808; echo $?` -> 1).
    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(
        stdout,
        "9223372036854775807|-9223372036854775808|ffffffffffffffff"
    );
    assert_eq!(stderr.matches("Numerical result out of range").count(), 2);
    assert!(!stderr.contains("warning:"));
}

#[test]
fn unsigned_formats_accept_uintmax_max_without_overflow() {
    let (status, stdout, stderr, _) = run(&[
        "%u|%o|%x|%X",
        "0xffffffffffffffff",
        "0xffffffffffffffff",
        "0xffffffffffffffff",
        "0xffffffffffffffff",
    ]);
    assert_eq!(status, EXECUTION_SUCCESS);
    assert_eq!(
        stdout,
        "18446744073709551615|1777777777777777777777|ffffffffffffffff|FFFFFFFFFFFFFFFF"
    );
    assert!(stderr.is_empty());
}

#[test]
fn unsigned_invalid_prefix_fails_and_preserves_value() {
    let (status, stdout, stderr, _) = run(&["%u|%x", "123oops", "oops"]);
    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "123|0");
    assert!(stderr.contains("123oops: invalid number"));
    assert!(stderr.contains("oops: invalid number"));
}

#[test]
fn unsigned_overflow_saturates_with_conversion_error_status() {
    let (status, stdout, stderr, _) =
        run(&["%u|%x", "18446744073709551616", "0x10000000000000000"]);
    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "18446744073709551615|ffffffffffffffff");
    assert_eq!(stderr.matches("Numerical result out of range").count(), 2);
    assert!(!stderr.contains("warning:"));
}

#[test]
fn invalid_prefixed_numbers_report_radix_diagnostics() {
    let (status, stdout, stderr, _) = run(&["%d|%d|%d", "09", "08", "0x1g"]);

    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "0|0|1");
    assert!(stderr.contains("09: invalid octal number"));
    assert!(stderr.contains("08: invalid octal number"));
    assert!(stderr.contains("0x1g: invalid hex number"));
}

#[test]
fn prints_plain_and_escaped_format() {
    assert_eq!(run(&["a\\nb"]).1, "a\nb");
}

#[test]
fn format_string_escapes_match_bash() {
    assert_eq!(run(&["\\045\\x41\\u0042\\101"]).1, "%ABA");
    assert_eq!(run(&["4\\.2 one\\ctwo"]).1, "4\\.2 one\\ctwo");
    // GNU printf.def tescape with sawc==NULL (format string): a leading-0
    // octal escape takes at most two MORE digits, so `\0101' is `\010'
    // (backspace) followed by a literal `1'; only %b (sawc != NULL) allows
    // the four-digit form. Verified: `printf '\01017' | od -An -tx1' on GNU
    // 5.3.0 prints `08 31 37'.
    assert_eq!(run(&["\\0101"]).1, "\u{8}1");
    assert_eq!(run(&["\\01017"]).1, "\u{8}17");
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

    // GNU getintmax: a PRESENT empty operand is `printf: : invalid number'
    // with exit 1 (strtoimax leaves ep==s -> chk_converror); only a MISSING
    // argument converts silently as 0.
    assert_eq!(status, EXECUTION_FAILURE);
    assert_eq!(stdout, "0");
    assert!(stderr.contains("printf: : invalid number"));
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
fn percent_q_uses_ansi_c_quoting_for_control_characters() {
    assert_eq!(
        run(&[
            "<%q><%q><%q><%q>",
            "a
 b",
            "a	 b",
            "ab",
            "ab"
        ])
        .1,
        "<$'a\\n b'><$'a\\t b'><$'a\\001b'><$'a\\177b'>"
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
fn percent_b_octal_escapes_wrap_to_raw_bytes() {
    let (_status, stdout, _stderr) = run_bytes(&["%b|%b|%b", "\\400", "\\777", "\\377"]);

    assert_eq!(stdout, b"\0|\xff|\xff");
}

#[test]
fn percent_b_strips_unknown_escape_backslash_like_bash() {
    let (status, stdout, stderr, _) = run(&["%b", "x\\qy"]);

    assert_eq!(status, EXECUTION_SUCCESS);
    // GNU printf.def default: unrecognized backslash escapes in %b are
    // passed through unaltered (backslash kept).
    assert_eq!(stdout, "x\\qy");
    assert!(stderr.is_empty());
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
