use rubash::lexer::{tokenize, TokenKind};

#[test]
fn test_single_quotes() {
    let input = "echo 'hello world'";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[1].value, "hello world");
}

#[test]
fn test_double_quotes() {
    let input = "echo \"hello world\"";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[1].value, "hello world");
    assert_eq!(tokens[1].raw, "\"hello world\"");
}

#[test]
fn test_empty_single_quotes() {
    let input = "''";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].value, "");
}

#[test]
fn test_empty_double_quotes() {
    let input = "\"\"";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].value, "");
}

#[test]
fn test_nested_quotes_in_double() {
    let input = "echo \"it's a 'test'\"";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 2);
    // wt33 (#373): single quotes inside a double-quoted word ride in
    // token.value as the \x17 DATA_SINGLE_QUOTE carrier; execution strips
    // them (probe wt33-373/run/F01_lexer_carrier_execution, byte-identical
    // to GNU 5.3.0: `printf '[%s]\n' "it's a 'test'"` -> [it's a 'test']).
    assert_eq!(tokens[1].value, "it\u{17}s a \u{17}test\u{17}");
}

#[test]
fn test_assignment_word_with_quoted_value() {
    let input = "alias foo='echo '";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[1].kind, TokenKind::Assignment);
    assert_eq!(tokens[1].value, "foo=\x1cecho ");
    assert_eq!(tokens[1].raw, "foo='echo '");
}

#[test]
fn test_multiline_single_quote_is_one_word() {
    let tokens = tokenize("echo 'foo\nbar'");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].value, "echo");
    assert_eq!(tokens[1].value, "foo\nbar");
}

#[test]
fn test_multiline_double_quote_is_one_word() {
    let tokens = tokenize("echo \"foo\nbar\"");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].value, "echo");
    assert_eq!(tokens[1].value, "foo\nbar");
}

#[test]
fn test_multiline_ansi_c_quote_with_escaped_single_quote_is_one_word() {
    let tokens = tokenize("echo $'foo\\'\nbar'");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].value, "echo");
    // wt33 (#373): $'...' quoting rides in token.value as the
    // \u{E010}/\u{E401} PUA carriers; execution decodes (probe
    // wt33-373/run/F02_ansi_c_multiline_word, byte-identical to GNU 5.3.0:
    // the multiline ansi-c word prints `foo'` / `bar`).
    assert_eq!(tokens[1].value, "foo\u{e010}\u{e401}\nbar");
    assert_eq!(tokens[1].raw, "$'foo\\'\nbar'");
}

#[test]
fn test_ansi_c_numeric_escapes_keep_following_non_digits() {
    let tokens = tokenize("printf $'\\x4Z' $'\\xZ' $'\\u0041Z'");

    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[1].value, "\x04Z");
    assert_eq!(tokens[2].value, "\\xZ");
    assert_eq!(tokens[3].value, "AZ");
}

#[test]
fn test_variable_with_adjacent_empty_quotes_uses_word_quote_removal() {
    let tokens = tokenize("recho $v''");

    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[1].kind, TokenKind::Word);
    // wt33 (#373): the empty-quote pair leaves a \x13 marker in token.value
    // (quote-removal bookkeeping); execution drops it (probe
    // wt33-373/run/F01 line 2-3: `v=VV; printf '[%s]\n' $v''` -> [VV]).
    assert_eq!(tokens[1].value, "$v\u{13}");
    assert_eq!(tokens[1].raw, "$v''");
}

#[test]
fn test_unquoted_trailing_backslash_at_eof_is_preserved() {
    let tokens = tokenize("echo escape\\");

    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[1].kind, TokenKind::Word);
    assert_eq!(tokens[1].value, "escape\\");
    assert_eq!(tokens[1].raw, "escape\\");
}

#[test]
fn test_command_substitution_preserves_inner_ansi_c_quotes() {
    let tokens = tokenize("echo \"$(printf $'foo\\'\nbar')\"");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].value, "echo");
    assert_eq!(tokens[1].value, "$(printf $'foo\\'\nbar')");
    assert_eq!(tokens[1].raw, "\"$(printf $'foo\\'\nbar')\"");
}

#[test]
fn test_command_substitution_ansi_c_quote_does_not_swallow_next_command() {
    let tokens = tokenize("echo $(printf $'foo\\'\nbar')\necho after");
    assert_eq!(tokens.len(), 5);
    assert_eq!(tokens[0].value, "echo");
    assert_eq!(tokens[1].value, "$(printf $'foo\\'\nbar')");
    assert_eq!(tokens[2].kind, TokenKind::Semicolon);
    assert_eq!(tokens[3].value, "echo");
    assert_eq!(tokens[4].value, "after");
}

#[test]
fn test_command_substitution_ansi_c_escaped_quote_mid_line_continues_correctly() {
    let tokens = tokenize("echo $(printf $'foo\\'x\nbar')\necho after");
    assert_eq!(tokens.len(), 5);
    assert_eq!(tokens[0].value, "echo");
    assert_eq!(tokens[1].value, "$(printf $'foo\\'x\nbar')");
    assert_eq!(tokens[2].kind, TokenKind::Semicolon);
    assert_eq!(tokens[3].value, "echo");
    assert_eq!(tokens[4].value, "after");
}

#[test]
fn test_pipeline_multiline_single_quote_is_one_word() {
    let tokens = tokenize("printf x | awk '\n/^}$/ { print $0 }\n/.*/ { next }\n'");
    assert_eq!(tokens.len(), 5);
    assert_eq!(tokens[0].value, "printf");
    assert_eq!(tokens[1].value, "x");
    assert_eq!(tokens[2].kind, TokenKind::Pipe);
    assert_eq!(tokens[3].value, "awk");
    assert_eq!(
        tokens[4].value,
        "\n/^}\x1f/ { print \x1f0 }\n/.*/ { next }\n"
    );
}
