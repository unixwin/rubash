use super::super::*;
use std::fs;

#[test]
fn test_compgen_empty_state_redirects_no_output() {
    let output_path = "target/rubash-compgen-output.txt";
    let status_path = "target/rubash-compgen-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compgen > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_invalid_option_reports_usage() {
    let error_path = "target/rubash-compgen-error.txt";
    let status_path = "target/rubash-compgen-error-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compgen -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("compgen: -x: invalid option\n"));
    assert!(error.contains("compgen: usage: compgen "));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compopt_outside_completion_function_fails() {
    let error_path = "target/rubash-compopt-error.txt";
    let status_path = "target/rubash-compopt-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compopt 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("compopt: not currently executing completion function\n"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_builtin_compopt_invalid_option_reports_usage() {
    let error_path = "target/rubash-builtin-compopt-error.txt";
    let status_path = "target/rubash-builtin-compopt-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("builtin compopt -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("compopt: -x: invalid option\n"));
    assert!(error.contains("compopt: usage: compopt "));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_eval_redirects_loop_body_without_retruncating() {
    let output_path = "target/rubash-eval-loop-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("eval 'for x in a b; do echo $x; done' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_redirects_output() {
    let output_path = "target/rubash-type-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("type -t echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "builtin\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_appends_output() {
    let output_path = "target/rubash-type-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("type -t echo >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\nbuiltin\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_prints_function_heredoc_body() {
    let output_path = "target/rubash-type-function-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("f()\n{{\ncat <<EOF > /dev/null\nbody\nEOF\naa=1\n}}\ntype f > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "f is a function\nf () \n{ \n    cat <<EOF > /dev/null\nbody\nEOF\n\n    aa=1\n}\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_terminates_plain_commands_before_function_heredoc() {
    let output_path = "target/rubash-type-function-heredoc-terminator-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("f()\n{{\necho\ncat <<EOF\nbody\nEOF\n}}\ntype f > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "f is a function\nf () \n{ \n    echo;\n    cat <<EOF\nbody\nEOF\n\n}\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_prints_compound_function_bodies() {
    let output_path = "target/rubash-type-compound-function-bodies-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() for x in a; {{ echo $x; }}; \
         s() select y in b; {{ echo $y; break; }}; \
         c() case $1 in a) echo alpha ;; *) echo other ;; esac; \
         type f s c > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("    for x in a; { echo $x; }"));
    assert!(output.contains("    select y in b; { echo $y; break; }"));
    assert!(output.contains("    case $1 in a) echo alpha ;; *) echo other ;; esac"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_pf_prints_condition_heredocs() {
    let output_path = "target/rubash-declare-function-condition-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "foo()\n{{\necho begin\nif cat << HERE\ncontents\nHERE\nthen\n    echo 1 2\n    echo 3 4\nfi\n}}\n\
         declare -pf foo > {output_path}\n\
         foo()\n{{\necho begin\nwhile read var << HERE\ncontents\nHERE\ndo\n    echo 1 2\n    echo 3 4\ndone\n}}\n\
         declare -pf foo >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("    if cat <<HERE\ncontents\nHERE\n    then\n"));
    assert!(output.contains("        echo 1 2;\n        echo 3 4;\n    fi\n"));
    assert!(output.contains("    while read var <<HERE\ncontents\nHERE\n    do\n"));
    assert!(output.contains("        echo 1 2;\n        echo 3 4;\n    done\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_heredoc_reads_following_lines_and_nested_alias_body() {
    let first_output_path = "target/rubash-alias-heredoc-following-output.txt";
    let nested_output_path = "target/rubash-alias-heredoc-nested-output.txt";
    let _ = fs::remove_file(first_output_path);
    let _ = fs::remove_file(nested_output_path);
    let input = format!(
        "shopt -s expand_aliases\n\
         alias 'headplus=cat > {first_output_path} <<EOF\nhello'\n\
         headplus\nworld\nEOF\n\
         alias head='cat > {nested_output_path} <<\\END' body='head\nhere-document\nEND'\n\
         body"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(first_output_path).unwrap(),
        "hello\nworld\n"
    );
    assert_eq!(
        fs::read_to_string(nested_output_path).unwrap(),
        "here-document\n"
    );
    let _ = fs::remove_file(first_output_path);
    let _ = fs::remove_file(nested_output_path);
}
