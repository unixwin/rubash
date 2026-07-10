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
fn test_comment_skip() {
    let tokens = tokenize("ls # comment");
    assert_eq!(tokens[0].value, "ls");
    assert!(tokens
        .iter()
        .skip(1)
        .all(|token| token.kind == TokenKind::Semicolon));
}
