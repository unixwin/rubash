use rubash::lexer::{tokenize, TokenKind};

#[test]
fn test_pipe_operator() {
    let input = "ls | grep foo";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[1].kind, TokenKind::Pipe);
    assert_eq!(tokens[1].value, "|");
}

#[test]
fn test_semicolon() {
    let input = "ls; cd /";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[1].kind, TokenKind::Semicolon);
    assert_eq!(tokens[1].value, ";");
}

#[test]
fn test_redirect_output() {
    let input = "echo hello > file.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectOut);
}

#[test]
fn test_redirect_output_without_space_after_word() {
    let input = "echo hello>file.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[1].value, "hello");
    assert_eq!(tokens[2].kind, TokenKind::RedirectOut);
    assert_eq!(tokens[3].value, "file.txt");
}

#[test]
fn test_clobber_redirect_output() {
    let input = "echo hello >| file.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectOut);
    assert_eq!(tokens[2].value, ">|");
}

#[test]
fn test_redirect_input() {
    let input = "cat < input.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[1].kind, TokenKind::RedirectIn);
    assert_eq!(tokens[1].value, "<");
}

#[test]
fn test_input_redirect_fd_prefix_without_space() {
    let input = "cat 0<input.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[1].kind, TokenKind::RedirectIn);
    assert_eq!(tokens[1].value, "0<");
    assert_eq!(tokens[2].value, "input.txt");
}

#[test]
fn test_read_write_redirect() {
    let input = "cat <> input.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[1].kind, TokenKind::RedirectOut);
    assert_eq!(tokens[1].value, "<>");
}

#[test]
fn test_append_redirect() {
    let input = "echo hello >> file.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::Append);
}

#[test]
fn test_append_redirect_without_spaces() {
    let input = "echo hello>>file.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::Append);
    assert_eq!(tokens[3].value, "file.txt");
}

#[test]
fn test_redirect_stderr() {
    let input = "echo error 2> err.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectErr);
}

#[test]
fn test_redirect_stderr_without_space_before_target() {
    let input = "echo error 2>err.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectErr);
    assert_eq!(tokens[3].value, "err.txt");
}

#[test]
fn test_clobber_redirect_stderr() {
    let input = "echo error 2>| err.txt";
    let tokens = tokenize(input);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectErr);
    assert_eq!(tokens[2].value, "2>|");
}

#[test]
fn test_combined_stdout_stderr_redirect() {
    let tokens = tokenize("echo both &> out.txt");
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectOut);
    assert_eq!(tokens[2].value, "&>");
}

#[test]
fn test_combined_stdout_stderr_append_redirect() {
    let tokens = tokenize("echo both &>> out.txt");
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::Append);
    assert_eq!(tokens[2].value, "&>>");
}

#[test]
fn test_redirect_stderr_close_fd() {
    let tokens = tokenize("echo error 2>&-");
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectErr);
    assert_eq!(tokens[2].value, "2>&");
    assert_eq!(tokens[3].value, "-");
}

#[test]
fn test_redirect_stdout_close_fd_with_prefix() {
    let tokens = tokenize("echo hidden 1>&-");
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[2].kind, TokenKind::RedirectOut);
    assert_eq!(tokens[2].value, "1>&");
    assert_eq!(tokens[3].value, "-");
}
