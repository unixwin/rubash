use super::super::*;
use std::fs;

#[test]
fn test_semicolon_separation() {
    let input = "echo a; echo b";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok());
    assert_eq!(ast.commands.len(), 2);
}

#[test]
fn test_empty_command_does_not_reset_exit_status() {
    let output_path = "target/rubash-empty-command-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("false; ; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("TMPDIR", &std::env::temp_dir().to_string_lossy());

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_underscore_tracks_last_command_argument() {
    let output_path = "target/rubash-underscore-last-arg-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf '%s\\n' alpha beta > {output_path}; \
         printf '%s\\n' \"$_\" >> {output_path}; \
         :; printf '%s\\n' \"$_\" >> {output_path}; \
         _=temporary printf '%s\\n' final >> {output_path}; \
         printf '%s\\n' \"$_\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("TMPDIR", &std::env::temp_dir().to_string_lossy());

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alpha\nbeta\nbeta\n:\nfinal\nfinal\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_assignment_like_argument_suppresses_tilde_expansion() {
    let output_path = "target/rubash-quoted-assignment-like-arg-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "HOME=/usr/xyz; \
         echo \"SHELL=~/bash\" > {output_path}; \
         echo SHELL=~/bash >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "SHELL=~/bash\nSHELL=/usr/xyz/bash\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_and_operator() {
    let input = "true && echo success";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok());
}

#[test]
fn test_or_operator() {
    let input = "false || echo fallback";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok());
}

#[test]
fn test_ansi_c_quoted_words_decode_as_single_arguments() {
    let output_path = "target/rubash-ansi-c-quoted-word-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '<%s>\\n' $'hello world!' $'hello\\' world' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<hello world!>\n<hello' world>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_locale_quoted_words_expand_like_double_quotes() {
    let output_path = "target/rubash-locale-quoted-word-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("v=VALUE; printf '<%s>\\n' $\"hello\" $\"$v\" $\"hello world\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<hello>\n<VALUE>\n<hello world>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_redirects_filtered_output() {
    let output_path = "target/rubash-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo hello | grep hello > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].pipe.is_some());
    assert_eq!(pipeline.stages[1].words, ["grep", "hello"]);
    assert!(pipeline.stages[1].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_counts_bytes_with_wc() {
    let output_path = "target/rubash-pipeline-wc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo hello | wc -c > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].pipe.is_some());
    assert_eq!(pipeline.stages[1].words, ["wc", "-c"]);
    assert!(pipeline.stages[1].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap().trim(), "6");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_filters_printf_output() {
    let output_path = "target/rubash-pipeline-printf-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf 'a\\nb\\n' | grep b > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].pipe.is_some());
    assert_eq!(pipeline.stages[0].words, ["printf", "a\\nb\\n"]);
    assert_eq!(pipeline.stages[1].words, ["grep", "b"]);
    assert!(pipeline.stages[1].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "b\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_grep_matches_start_anchor() {
    let output_path = "target/rubash-pipeline-grep-anchor-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf 'foo=abc\\nbar=foo\\n' | grep ^foo= > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "foo=abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_status_uses_last_command_by_default() {
    let output_path = "target/rubash-pipeline-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("false | true; echo $? > {output_path}");
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
fn test_pipestatus_tracks_simple_and_pipeline_statuses() {
    let output_path = "target/rubash-pipestatus-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $PIPESTATUS:${{PIPESTATUS[@]}}:${{#PIPESTATUS[@]}} > {output_path}; \
         false; echo $?:${{PIPESTATUS[@]}}:${{PIPESTATUS[0]}} >> {output_path}; \
         false | true | false; echo $? -- $PIPESTATUS -- ${{PIPESTATUS[@]}} -- ${{PIPESTATUS[0]}} - ${{PIPESTATUS[1]}} - ${{PIPESTATUS[2]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0:0:1\n1:1:1\n1 -- 1 -- 1 0 1 -- 1 - 0 - 1\n"
    );
    let _ = fs::remove_file(output_path);
}
