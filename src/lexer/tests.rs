use super::*;

#[test]
fn test_tokenize_simple() {
    let tokens = tokenize("ls -la");
    assert!(tokens.len() >= 2);
    assert_eq!(tokens[0].value, "ls");
    assert_eq!(tokens[1].value, "-la");
}

#[test]
fn test_tokenize_empty() {
    assert!(tokenize("").is_empty());
}

#[test]
fn test_empty_quoted_heredoc_delimiter_reads_until_eof() {
    let tokens = tokenize("cat <<''\nhi\nthere\n''");

    assert!(tokens.iter().any(|token| token.kind == TokenKind::HereDoc));
    let body = tokens
        .iter()
        .find(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str());
    assert_eq!(body, Some("\x1e\x1fhi\nthere\n''\n"));
}

#[test]
fn test_command_substitution_here_string_does_not_swallow_following_heredoc() {
    let tokens = tokenize("echo $(\ncat <<< \"comsub here-string\"\n)\ncat <<''\nhi\nthere\n''");

    let bodies = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bodies, vec!["\x1e\x1fhi\nthere\n''\n"]);
}

#[test]
fn test_nested_braced_parameter_stays_in_one_word() {
    let tokens = tokenize("echo ${outer:-${inner:-fallback}} ${array[${idx:-0}]}");

    assert_eq!(tokens[1].value, "${outer:-${inner:-fallback}}");
    assert_eq!(tokens[2].value, "${array[${idx:-0}]}");
    assert!(tokens
        .iter()
        .all(|token| token.value != "}" || token.kind == TokenKind::HereDocBody));
}

#[test]
fn test_braced_parameter_single_quotes_do_not_swallow_closing_brace() {
    let tokens = tokenize("echo ${IFS+'bar} ${v/$'\\''/x}");

    assert_eq!(tokens[1].value, "${IFS+'bar}");
    assert_eq!(tokens[2].value, "${v/$'\\''/x}");
}

#[test]
fn test_comment_skip() {
    let tokens = tokenize("ls # comment");
    assert_eq!(tokens[0].value, "ls");
    assert!(tokens
        .iter()
        .skip(1)
        .all(|token| token.kind == TokenKind::Semicolon));
}
