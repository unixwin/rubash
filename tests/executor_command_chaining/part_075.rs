use super::super::*;
use std::fs;

#[test]
fn test_while_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-while-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while false; do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_while_command_redirects_body_stdout() {
    let output_path = "target/rubash-while-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "while true; do echo loop; break; done > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "loop\ndone\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_until_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-until-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("until true; do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_while_command_here_string_feeds_condition_read() {
    let output_path = "target/rubash-while-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while read value; do echo got:$value; done <<< alpha > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_until_command_here_string_feeds_body_read() {
    let output_path = "target/rubash-until-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "until false; do read value; echo got:$value; break; done <<< alpha > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_while_command_executes_loop() {
    let output_path = "target/rubash-alias-while-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias w=while\nn=0\n\
         w test $n -lt 2; do echo loop:$n >> {output_path}; (( n++ )); done"
    );
    // wt42 (2026-10-02 full-rebaseline): GNU 5.3.0 rejects the `;`
    // one-buffer form (`alias w=while; w …; do`) with `syntax error near
    // unexpected token do' — the whole list is parsed before the alias
    // command executes, so `w' never expands (parse.y:5761 read_token_word
    // -> 3249 alias_expand_token at READ time); 815802eb aligned rubash.
    // Newline-separated definitions execute before the use line is read;
    // run_cli_script output is byte-identical to WSL GNU 5.3.0.
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "loop:0\nloop:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_until_command_executes_loop() {
    let output_path = "target/rubash-alias-until-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias u=until\nn=0\n\
         u test $n -ge 2; do echo loop:$n >> {output_path}; (( n++ )); done"
    );
    // wt42 (2026-10-02 full-rebaseline): see the while-form comment above —
    // GNU expands the alias only when the definition executed before the
    // use line is READ; the `;` one-buffer form is rejected by both shells.
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "loop:0\nloop:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_while_keeps_nested_alias_for_body() {
    let output_path = "target/rubash-alias-while-nested-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias w=while\nalias f=for\nn=0\n\
         w test $n -lt 1; do f item in a b; do echo $item >> {output_path}; done; (( ++n )); done"
    );
    // wt42 (2026-10-02 full-rebaseline): see the while-form comment above —
    // GNU expands the alias only when the definition executed before the
    // use line is READ; the `;` one-buffer form is rejected by both shells.
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_while_prefix_with_do_skips_body() {
    let output_path = "target/rubash-alias-while-prefix-do-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias w='while false; do'\n\
         w echo bad > {output_path}; done; echo after > {output_path}"
    );
    // wt42 (2026-10-02 full-rebaseline): see the while-form comment above —
    // GNU expands the alias only when the definition executed before the
    // use line is READ; the `;` one-buffer form is rejected by both shells.
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "after\n");
    let _ = fs::remove_file(output_path);
}
