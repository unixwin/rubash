use super::super::*;
use std::fs;

#[test]
fn test_break_outside_loop_returns_success() {
    let output_path = "target/rubash-break-outside-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("break not-a-number; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_break_outside_loop_redirects_stderr() {
    let output_path = "target/rubash-break-outside-redirect-output.txt";
    let error_path = "target/rubash-break-outside-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("break 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("break: only meaningful in a `for', `while', or `until' loop"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_break_zero_in_loop_returns_failure_without_breaking() {
    let output_path = "target/rubash-break-zero-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while true; do break 0; echo $? > {output_path}; break; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_break_zero_in_loop_redirects_stderr() {
    let output_path = "target/rubash-break-zero-redirect-output.txt";
    let error_path = "target/rubash-break-zero-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input =
        format!("while true; do break 0 2> {error_path}; echo $? > {output_path}; break; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("break: 0: loop count out of range"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_break_accepts_positive_signed_level() {
    let output_path = "target/rubash-break-plus-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("while true; do break +1; echo bad > {output_path}; done; echo ok > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_continue_zero_in_loop_returns_failure_without_continuing() {
    let output_path = "target/rubash-continue-zero-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while true; do continue 0; echo $? > {output_path}; break; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_continue_zero_in_loop_redirects_stderr() {
    let output_path = "target/rubash-continue-zero-redirect-output.txt";
    let error_path = "target/rubash-continue-zero-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input =
        format!("while true; do continue 0 2> {error_path}; echo $? > {output_path}; break; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("continue: 0: loop count out of range"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_break_two_exits_nested_loops() {
    let output_path = "target/rubash-break-two-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for outer in a b; do for inner in c d; do break 2; echo inner >> {output_path}; done; echo outer >> {output_path}; done; echo ok > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_until_true_skips_body() {
    let output_path = "target/rubash-until-true-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("until true; do echo bad > {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let loop_command = ast.commands[0].loop_command.as_ref().unwrap();
    assert!(loop_command.until);
    assert_eq!(loop_command.condition[0].words, ["true"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_case_question_mark_pattern_matches() {
    let output_path = "target/rubash-case-question-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case hello in h?llo) echo yes > {output_path} ;; *) echo no > {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_bracket_range_pattern_matches() {
    let output_path = "target/rubash-case-bracket-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case beta in [a-c]eta) echo yes > {output_path} ;; *) echo no > {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_posix_character_class_patterns_match() {
    let dir_path = "target/rubash-posix-class-patterns";
    let output_path = "target/rubash-posix-class-patterns-output.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/5.txt"), "digit").unwrap();
    fs::write(format!("{dir_path}/a.txt"), "alpha").unwrap();
    let input = format!(
        "case 5 in [[:digit:]]) echo case:yes > {output_path} ;; *) echo case:no > {output_path} ;; esac; \
         [[ A == [[:upper:]] ]]; echo upper:$? >> {output_path}; \
         [[ _ == [[:word:]] ]]; echo word:$? >> {output_path}; \
         printf '%s\\n' {dir_path}/[[:digit:]].txt >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "case:yes\nupper:0\nword:0\ntarget/rubash-posix-class-patterns/5.txt\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_case_backslash_patterns_preserve_literal_backslash() {
    let output_path = "target/rubash-case-backslash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "x='\\x'; \
         case $x in \\x) echo bad > {output_path} ;; \\\\x) echo literal > {output_path} ;; *) echo star > {output_path} ;; esac; \
         case x in \\\\x) echo bad >> {output_path} ;; \\x) echo plain >> {output_path} ;; *) echo star >> {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(
        ast.commands
            .iter()
            .filter(|command| command.case_command.is_some())
            .count(),
        2
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "literal\nplain\n");
    let _ = fs::remove_file(output_path);
}
