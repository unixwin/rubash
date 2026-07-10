use super::*;

#[test]
fn test_escaped_character() {
    let input = "echo \\n";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 2);
}

#[test]
fn test_consecutive_operators() {
    let input = "ls || echo error";
    let tokens = tokenize(input);
    // ls, ||, echo, error = 4 tokens
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[1].kind, TokenKind::Or);
}

#[test]
fn test_and_operator() {
    let input = "ls && echo success";
    let tokens = tokenize(input);
    // ls, &&, echo, success = 4 tokens
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[1].kind, TokenKind::And);
}
