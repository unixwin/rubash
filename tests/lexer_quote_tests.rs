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
    assert_eq!(tokens[1].value, "it's a 'test'");
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
    assert_eq!(tokens[1].value, "foo'\nbar");
    assert_eq!(tokens[1].raw, "$'foo\\'\nbar'");
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
