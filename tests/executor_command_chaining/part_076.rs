use super::super::*;
use std::fs;

#[test]
fn test_for_command_redirects_body_stdout() {
    let output_path = "target/rubash-for-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in alpha beta; do echo $item; done > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alpha\nbeta\ndone\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_expands_brace_range_words() {
    let output_path = "target/rubash-for-brace-range-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in {{1..3}}; do echo $item >> {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n2\n3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_expands_stepped_and_padded_brace_ranges() {
    let output_path = "target/rubash-for-stepped-brace-range-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in {{01..03}} {{5..1..2}} {{a..e..2}}; do echo $item >> {output_path}; done"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "01\n02\n03\n5\n3\n1\na\nc\ne\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_expands_brace_list_words() {
    let output_path = "target/rubash-for-brace-list-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in pre{{a,b}}; do echo $item >> {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "prea\npreb\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_simple_command_brace_expansion_keeps_escaped_commas() {
    let output_path = "target/rubash-brace-escaped-comma-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '<%s>\\n' a{{b\\,c,d}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<ab,c>\n<ad>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_brace_expansion_keeps_escaped_commas() {
    let output_path = "target/rubash-for-brace-escaped-comma-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("for item in a{{b\\,c,d}}; do printf '<%s>\\n' \"$item\" >> {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<ab,c>\n<ad>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_accepts_brace_group_body() {
    let output_path = "target/rubash-for-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in alpha beta; {{ echo $item; }} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-for-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in; do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-arithmetic-for-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for (( i = 0; i < 0; i++ )); do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .for_command
        .as_ref()
        .and_then(|for_command| for_command.arithmetic.as_ref())
        .is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_accepts_brace_group_body() {
    let output_path = "target/rubash-arithmetic-for-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for (( i = 0; i < 3; i++ )); {{ echo $i; }} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .for_command
        .as_ref()
        .and_then(|for_command| for_command.arithmetic.as_ref())
        .is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_pipes_body_stdout() {
    let output_path = "target/rubash-arithmetic-for-pipe-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for (( i = 0; i < 2; i++ )); do echo $i; done | wc -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].pipeline_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_input_redirect_feeds_body_reads() {
    let input_path = "target/rubash-for-command-input.txt";
    let output_path = "target/rubash-for-command-input-output.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in first second; do read value; echo $item:$value; done < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "first:alpha\nsecond:beta\n"
    );
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_here_string_feeds_body_read() {
    let output_path = "target/rubash-for-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in one; do read value; echo $item:$value; done <<< alpha > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_keeps_brace_group_body_command() {
    let output_path = "target/rubash-for-brace-group-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in one; do {{ echo $item; }} done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let body = &ast.commands[0].for_command.as_ref().unwrap().body;
    assert!(body.iter().any(|command| command.brace_group.is_some()));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_keeps_nested_select_body() {
    let output_path = "target/rubash-for-nested-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in outer; do select choice in inner; do echo $item:$choice; break; done <<< 1; done > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let body = &ast.commands[0].for_command.as_ref().unwrap().body;
    assert!(body.iter().any(|command| command.select_command.is_some()));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "outer:inner\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_keeps_nested_select_body() {
    let output_path = "target/rubash-arithmetic-for-nested-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for (( i = 0; i < 1; i++ )); do select choice in inner; do echo $i:$choice; break; done <<< 1; done > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let body = &ast.commands[0].for_command.as_ref().unwrap().body;
    assert!(body.iter().any(|command| command.select_command.is_some()));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:inner\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_input_redirect_feeds_body_read() {
    let input_path = "target/rubash-arithmetic-for-command-input.txt";
    let output_path = "target/rubash-arithmetic-for-command-input-output.txt";
    fs::write(input_path, "alpha\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for (( i = 0; i < 1; i++ )); do read value; echo $i:$value; done < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .for_command
        .as_ref()
        .and_then(|for_command| for_command.arithmetic.as_ref())
        .is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:alpha\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_for_keeps_nested_alias_while_body() {
    let output_path = "target/rubash-alias-for-nested-while-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias f=for; alias w=while; \
         f item in a b; do n=0; w test $n -lt 1; do echo $item:$n >> {output_path}; (( ++n )); done; done"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a:0\nb:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_for_accepts_brace_group_body() {
    let output_path = "target/rubash-alias-for-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias f=for; \
         f item in alpha beta; {{ echo $item; }} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}
