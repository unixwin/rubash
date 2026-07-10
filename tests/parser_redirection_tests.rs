use rubash::lexer::tokenize;
use rubash::parser::parse;

#[test]
fn test_output_redirect() {
    let input = "echo hello > file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_out.is_some());
    assert!(!ast.commands[0].redirect_out.as_ref().unwrap().clobber);
}

#[test]
fn test_output_redirect_without_space_after_word() {
    let input = "echo hello>file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["echo", "hello"]);
    assert_eq!(
        ast.commands[0].redirect_out.as_ref().unwrap().target,
        "file.txt"
    );
}

#[test]
fn test_clobber_output_redirect() {
    let input = "echo hello >| file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_out.as_ref().unwrap().clobber);
}

#[test]
fn test_input_redirect() {
    let input = "cat < input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_in.is_some());
}

#[test]
fn test_input_redirect_without_space() {
    let input = "cat<input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["cat"]);
    assert_eq!(
        ast.commands[0].redirect_in.as_ref().unwrap().target,
        "input.txt"
    );
}

#[test]
fn test_append_redirect() {
    let input = "echo hello >> file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].append.is_some());
}

#[test]
fn test_clobber_stderr_redirect() {
    let input = "echo hello 2>| err.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_err.as_ref().unwrap().clobber);
}

#[test]
fn test_here_string_redirect() {
    let input = "read x <<<\"alpha\"";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].here_string.as_deref(), Some("alpha"));
}
