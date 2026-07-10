use super::run;

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
