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

/// rubash#704 R6.1 — printf numeric validation. GNU getintmax/getuintmax/
/// getdouble (printf.def:1427/1450/1472) hand the operand to
/// strtoimax/strtoumax/strtod, which skip only LEADING white space; the
/// accumulated prefix value is still printed while chk_converror
/// (printf.def:1388-1395) rejects whatever bytes remain and flips
/// conversion_error (exit 1).
mod numeric_validation_704 {
    use super::*;

    #[test]
    fn trailing_whitespace_is_not_part_of_the_number() {
        let (status, stdout, stderr, _) =
            run(&["%d|%i|%u|%x|%X|%o", "5 ", "5 ", "5 ", "5 ", "5 ", "5 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "5|5|5|5|5|5");
        assert_eq!(stderr.matches("printf: 5 : invalid number").count(), 6);

        let (status, stdout, stderr, _) = run(&["%d", "5\t"]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "5");
        assert!(stderr.contains("printf: 5\t: invalid number"));
    }

    #[test]
    fn float_conversions_keep_the_accumulated_prefix_value() {
        // getdouble -> strtod: `5 ` accumulates 5.0; the trailing space is a
        // chk_converror suffix. The pre-fix engine dropped the value to 0.
        let (status, stdout, stderr, _) = run(&["%f|%e|%g|%a", "5 ", "5 ", "5 ", "5 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        // What this facet owns is that the VALUE survives: it must not
        // collapse to 0x0p+0. The nibble-style `%a` (`0xap-1`) is the
        // glibc %La form owned by issue #879.
        assert_eq!(stdout, "5.000000|5.000000e+00|5|0xap-1");
        assert_eq!(stderr.matches("printf: 5 : invalid number").count(), 4);

        let (status, stdout, stderr, _) = run(&["%f", "5.5 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "5.500000");
        assert!(stderr.contains("printf: 5.5 : invalid number"));
    }

    #[test]
    fn leading_whitespace_still_converts_silently() {
        let (status, stdout, stderr, _) = run(&["%d|%f", " 5", " 5"]);
        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "5|5.000000");
        assert!(stderr.is_empty());

        // Both-side padding: the leading half is skipped, the trailing half
        // is the suffix the diagnostic names verbatim.
        let (status, stdout, stderr, _) = run(&["%d", "  5  "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "5");
        assert!(stderr.contains("printf:   5  : invalid number"));
    }

    #[test]
    fn suffix_wins_over_erange_in_the_diagnostic_choice() {
        // chk_converror checks `*ep != '\0'` BEFORE errno == ERANGE.
        let (status, stdout, stderr, _) = run(&["%d", "99999999999999999999x"]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "9223372036854775807");
        assert!(stderr.contains("99999999999999999999x: invalid number"));
        assert!(!stderr.contains("Numerical result out of range"));
    }

    #[test]
    fn signed_suffixes_keep_their_converted_value() {
        let (status, stdout, stderr, _) = run(&["%d|%d|%u|%d", "+5 ", "-5 ", "-1 ", "-010 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "5|-5|18446744073709551615|-8");
        assert!(stderr.contains("printf: +5 : invalid number"));
        assert!(stderr.contains("printf: -5 : invalid number"));
        assert!(stderr.contains("printf: -1 : invalid number"));
        assert!(stderr.contains("printf: -010 : invalid number"));
    }

    #[test]
    fn invalid_number_classification_follows_sh_invalidnum() {
        // builtins/common.c:214-225 sh_invalidnum(): leading `0` + digit is
        // an octal complaint, leading `0x` (lowercase x only) a hex
        // complaint, everything else a plain number — including `0Xzz`
        // (strtoumax consumed only the `0`) and `0 `.
        let (status, stdout, stderr, _) =
            run(&["%d|%d|%d|%d|%d", "055 ", "01z", "0Xzz", "0x10 ", "0 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "45|1|0|16|0");
        assert!(stderr.contains("printf: 055 : invalid octal number"));
        assert!(stderr.contains("01z: invalid octal number"));
        assert!(stderr.contains("0Xzz: invalid number"));
        assert!(stderr.contains("printf: 0x10 : invalid hex number"));
        assert!(stderr.contains("printf: 0 : invalid number"));
    }

    #[test]
    fn base_zero_binary_literals_convert_like_glibc() {
        // glibc 2.38+ strto* accepts the C23 0b/0B prefix in base 0.
        let (status, stdout, stderr, _) = run(&["%d|%d", "0b101", "0B101"]);
        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "5|5");
        assert!(stderr.is_empty());

        // A non-binary digit after 0b is a suffix on the octal `0`.
        let (status, stdout, stderr, _) = run(&["%d", "0b2"]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "0");
        assert!(stderr.contains("0b2: invalid number"));
    }

    #[test]
    fn whitespace_only_operand_reports_the_whole_operand() {
        let (status, stdout, stderr, _) = run(&["%d|%d", " ", "  "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "0|0");
        assert!(stderr.contains("printf:  : invalid number"));
        assert!(stderr.contains("printf:   : invalid number"));
    }

    #[test]
    fn lone_quote_is_a_nul_character_constant_without_error() {
        // printf.def:1519-1544 asciicode(): arg[1] of a quoted operand, with
        // no chk_converror — a lone quote reads the NUL at arg[1] (0).
        let (status, stdout, stderr, _) = run(&["%d|%d|%d", "'a", "'a ", "'"]);
        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "97|97|0");
        assert!(stderr.is_empty());
    }

    #[test]
    fn float_subject_sequences_parse_the_longest_valid_prefix() {
        let (status, stdout, stderr, _) =
            run(&["%f|%f|%f|%f|%f|%f", "5x", "5. ", "5e", "1e3 ", ".5 ", "ff"]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(
            stdout,
            "5.000000|5.000000|5.000000|1000.000000|0.500000|0.000000"
        );
        assert!(stderr.contains("5x: invalid number"));
        assert!(stderr.contains("printf: 5. : invalid number"));
        assert!(stderr.contains("5e: invalid number"));
        assert!(stderr.contains("printf: 1e3 : invalid number"));
        assert!(stderr.contains("printf: .5 : invalid number"));
        assert!(stderr.contains("ff: invalid number"));
    }

    #[test]
    fn hexadecimal_float_subjects_convert() {
        let (status, stdout, stderr, _) = run(&["%f|%f|%f|%f", "0x1", "0x1p3", "0x.8p1", "0xg"]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "1.000000|8.000000|1.000000|0.000000");
        assert!(stderr.contains("printf: 0xg: invalid hex number"));
    }

    #[test]
    fn nonfinite_operands_spell_like_glibc() {
        // glibc spells per conversion case: %f/%e/%g -> inf/nan, %F/%E ->
        // INF/NAN; the sign bit renders as a literal minus; `%+f` keeps the
        // plus.
        let (status, stdout, stderr, _) = run(&[
            "%f|%f|%F|%F|%e|%+f|%f",
            "inf",
            "nan",
            "inf",
            "nan ",
            "nan",
            "inf",
            "-nan",
        ]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "inf|nan|INF|NAN|nan|+inf|-nan");
        assert!(stderr.contains("printf: nan : invalid number"));
    }

    #[test]
    fn nan_parenthesised_sequence_is_part_of_the_subject() {
        let (status, stdout, stderr, _) = run(&["%f|%f", "nan(123)", "infinity"]);
        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "nan|inf");
        assert!(stderr.is_empty());
    }

    #[test]
    fn width_and_continuation_still_apply_to_suffixed_operands() {
        let (status, stdout, stderr, _) = run(&["%5d|\n", "5 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "    5|\n");
        assert!(stderr.contains("printf: 5 : invalid number"));

        // POSIX: processing continues after a conversion error — every
        // operand gets its own diagnostic.
        let (status, stdout, stderr, _) = run(&["%d:%d\n", "5 ", "6 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "5:6\n");
        assert!(stderr.contains("printf: 5 : invalid number"));
        assert!(stderr.contains("printf: 6 : invalid number"));
    }

    #[test]
    fn v_assignment_still_lands_with_a_conversion_error() {
        let (status, stdout, stderr, env_vars) = run(&["-v", "V", "%d\n", "5 "]);
        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stdout.is_empty());
        assert!(stderr.contains("printf: 5 : invalid number"));
        assert_eq!(env_vars.get("V"), Some(&"5\n".to_string()));
    }
}
