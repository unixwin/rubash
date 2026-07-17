use super::super::*;
use std::fs;

#[test]
fn test_select_command_redirects_body_stdout() {
    let output_path = "target/rubash-select-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in alpha beta; do echo chosen:$item; break; done <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_command_accepts_brace_group_body() {
    let output_path = "target/rubash-select-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in alpha beta; {{ echo chosen:$item; break; }} <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_command_redirect_creates_file_for_empty_word_list() {
    let output_path = "target/rubash-select-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item in; do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_command_input_redirect_feeds_choice() {
    let input_path = "target/rubash-select-command-input.txt";
    let output_path = "target/rubash-select-command-input-output.txt";
    fs::write(input_path, "2\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in alpha beta; do echo chosen:$item; break; done < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_without_in_uses_positional_params() {
    let output_path = "target/rubash-select-default-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item; do echo chosen:$item; break; done <<< '2' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let select = ast.commands[0].select_command.as_ref().unwrap();
    assert!(select.default_positional);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string(), "beta".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_brace_body_without_in_uses_positional_params() {
    let output_path = "target/rubash-select-brace-body-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item; {{ echo chosen:$item; break; }} <<< '2' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let select = ast.commands[0].select_command.as_ref().unwrap();
    assert!(select.default_positional);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string(), "beta".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_explicit_empty_in_does_not_use_positional_params() {
    let output_path = "target/rubash-select-empty-in-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item in; do echo bad; done <<< '1' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let select = ast.commands[0].select_command.as_ref().unwrap();
    assert!(!select.default_positional);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_select_command_executes_choice() {
    let output_path = "target/rubash-alias-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias s=select; \
         s item in alpha beta; do echo chosen:$item; break; done <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_select_accepts_brace_group_body() {
    let output_path = "target/rubash-alias-select-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias s=select; \
         s item in alpha beta; {{ echo chosen:$item; break; }} <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_select_without_in_uses_positional_params() {
    let output_path = "target/rubash-alias-select-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias s=select; \
         s item; do echo chosen:$item; break; done <<< '1' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_pipeline_modifier_executes_full_pipeline() {
    let output_path = "target/rubash-time-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p echo alpha | wc -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.stages[0].words[0], "echo");
    assert!(pipeline.stages[0].pipe.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_pipeline_modifier_preserves_pipeline_status() {
    let output_path = "target/rubash-time-pipeline-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time echo alpha | grep beta > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].pipe.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_pipeline_modifier_keeps_raw_word_metadata() {
    let output_path = "target/rubash-time-pipeline-raw-metadata-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p printf '<%s>\\n' a{{b\\,c,d}} | cat > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.stages[0].words[0], "printf");
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<ab,c>\n<ad>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_redirects_timed_command_stdout() {
    let output_path = "target/rubash-time-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p echo hi > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert_eq!(time_command.command.words, ["echo", "hi"]);
    assert!(time_command.command.redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_heredoc_feeds_timed_command() {
    let output_path = "target/rubash-time-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p cat <<EOF > {output_path}\nalpha\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert_eq!(time_command.command.words, ["cat"]);
    assert_eq!(
        time_command.command.heredoc_delimiter.as_deref(),
        Some("EOF")
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_inverts_timed_status_with_redirect() {
    let output_path = "target/rubash-time-command-invert-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time ! false > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.inverted);
    assert_eq!(time_command.command.words, ["false"]);
    assert!(time_command.command.redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_null_command_resets_status() {
    let output_path = "target/rubash-time-null-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("false; time -p --; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[1].time_command.as_ref().unwrap();
    assert!(time_command.command.words.is_empty());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_builtin_can_invoke_time_keyword_bridge() {
    let output_path = "target/rubash-command-time-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command time -p false; echo false:$? > {output_path}; \
         command time -p true; echo true:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "false:1\ntrue:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_outer_inversion_wraps_time_simple_command() {
    let output_path = "target/rubash-inverted-time-simple-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("! time false; echo status:$? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].inverted_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_outer_inversion_wraps_time_pipeline_command() {
    let output_path = "target/rubash-inverted-time-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("! time echo alpha | grep beta; echo status:$? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].inverted_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_for_command() {
    let output_path = "target/rubash-time-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p for x in a b; do echo $x >> {output_path}; done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.posix_format);
    assert!(time_command.command.for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_inversion_prefix_executes_for_command() {
    let output_path = "target/rubash-time-inverted-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p ! for x in a; do echo $x >> {output_path}; done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.posix_format);
    assert!(time_command.inverted);
    assert!(time_command.command.for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nstatus:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_if_command_sequence() {
    let output_path = "target/rubash-time-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p if true; then echo yes > {output_path}; else echo no > {output_path}; fi; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.command.if_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_if_condition_keeps_raw_word_metadata() {
    let output_path = "target/rubash-time-if-raw-metadata-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p if printf '<%s>\\n' a{{b\\,c,d}} > {output_path}; then :; fi; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.command.if_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<ab,c>\n<ad>\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_inversion_prefix_executes_if_command_sequence() {
    let output_path = "target/rubash-time-inverted-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p ! if true; then echo yes > {output_path}; else echo no > {output_path}; fi; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.inverted);
    assert!(time_command.command.if_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\nstatus:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_while_command_sequence() {
    let output_path = "target/rubash-time-while-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; time -p while [[ $n -lt 2 ]]; do echo $n >> {output_path}; (( n++ )); done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[1].time_command.as_ref().unwrap();
    assert!(time_command.command.loop_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_until_command_sequence() {
    let output_path = "target/rubash-time-until-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; time -p until [[ $n -ge 2 ]]; do echo $n >> {output_path}; (( n++ )); done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[1].time_command.as_ref().unwrap();
    assert!(time_command.command.loop_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_case_command() {
    let output_path = "target/rubash-time-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p case beta in alpha) echo alpha > {output_path} ;; beta) echo beta > {output_path} ;; esac; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.command.case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_select_command() {
    let output_path = "target/rubash-time-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p select item in alpha beta; do echo chosen:$item > {output_path}; break; done <<< '2'; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.command.select_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "chosen:beta\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_arithmetic_command() {
    let output_path = "target/rubash-time-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p (( 1 )); echo true:$? > {output_path}; \
         time -p (( 0 )); echo false:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].time_command.is_some());
    assert!(ast.commands[2].time_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "true:0\nfalse:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_conditional_command() {
    let output_path = "target/rubash-time-conditional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=ok; time -p [[ $value == ok ]]; echo true:$? > {output_path}; \
         time -p [[ $value == bad ]]; echo false:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[1].time_command.is_some());
    assert!(ast.commands[3].time_command.is_some());
    assert!(ast.commands[1]
        .time_command
        .as_ref()
        .unwrap()
        .command
        .conditional_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "true:0\nfalse:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_brace_group() {
    let output_path = "target/rubash-time-brace-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p {{ echo one > {output_path}; echo two >> {output_path}; }}; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.command.brace_group.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "one\ntwo\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_brace_group_pipeline_feeds_next_stage() {
    let output_path = "target/rubash-time-brace-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time {{ echo one; }} | wc -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].time_command.is_some());
    assert_eq!(pipeline.stages[1].words, ["wc", "-l"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_inversion_prefix_executes_brace_group() {
    let output_path = "target/rubash-time-inverted-brace-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p ! {{ true; }}; echo status:$? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.inverted);
    assert!(time_command.command.brace_group.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_subshell_group() {
    let output_path = "target/rubash-time-subshell-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=outer; time -p ( value=inner; echo $value > {output_path} ); echo $value >> {output_path}; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[1].time_command.as_ref().unwrap();
    let body = &time_command.command.subshell_command.as_ref().unwrap().body;
    assert_eq!(body[0].assignments.get("value"), Some(&"inner".to_string()));
    assert_eq!(body[1].words, ["echo", "$value"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "inner\nouter\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_function_definition() {
    let output_path = "target/rubash-time-function-definition-output.txt";
    let error_path = "target/rubash-time-function-definition-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "time function first {{ echo first > {output_path}; }} 2> {error_path}; first; \
         time second() {{ echo second >> {output_path}; }} 2>> {error_path}; second"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "first\nsecond\n");
    assert!(ast.commands[0]
        .time_command
        .as_ref()
        .unwrap()
        .command
        .function_command
        .is_some());
    assert!(ast.commands[2]
        .time_command
        .as_ref()
        .unwrap()
        .command
        .function_command
        .is_some());
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_time_prefix_executes_coproc_command() {
    let status_path = "target/rubash-time-coproc-status.txt";
    let _ = fs::remove_file(status_path);
    let input =
        format!("time -p coproc TIMEDC {{ :; }}; echo pid:${{TIMEDC_PID:+set}} > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_time_inversion_prefix_executes_function_definition() {
    let output_path = "target/rubash-time-inverted-function-definition-output.txt";
    let error_path = "target/rubash-time-inverted-function-definition-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "time -p ! function timed_func {{ echo timed > {output_path}; }} 2> {error_path}; timed_func"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();

    assert!(time_command.posix_format);
    assert!(time_command.inverted);
    assert!(time_command.command.function_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "timed\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_time_inversion_prefix_executes_subshell_group() {
    let output_path = "target/rubash-time-inverted-subshell-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p ! ( true ); echo status:$? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let time_command = ast.commands[0].time_command.as_ref().unwrap();
    assert!(time_command.inverted);
    assert!(time_command.command.subshell_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_time_executes_brace_group() {
    let output_path = "target/rubash-alias-time-brace-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p {{ echo alias-time > {output_path}; }}; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias-time\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_time_preserves_brace_group_redirects() {
    let output_path = "target/rubash-alias-time-brace-redirect-output.txt";
    let status_path = "target/rubash-alias-time-brace-redirect-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p {{ echo redirected; }} > {output_path}; echo status:$? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "redirected\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_alias_introduced_time_executes_if_sequence() {
    let output_path = "target/rubash-alias-time-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p if true; then echo alias-if > {output_path}; fi; \
         echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias-if\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_time_executes_for_sequence_with_redirect() {
    let output_path = "target/rubash-alias-time-for-redirect-output.txt";
    let status_path = "target/rubash-alias-time-for-redirect-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p for value in a b; do echo $value; done > {output_path}; \
         echo status:$? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_alias_introduced_time_executes_case_sequence() {
    let output_path = "target/rubash-alias-time-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p case beta in alpha) echo alpha ;; \
         beta) echo beta ;; esac > {output_path}; \
         echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_time_executes_arithmetic_command() {
    let output_path = "target/rubash-alias-time-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p (( 1 )); echo true:$? > {output_path}; \
         t -p (( 0 )); echo false:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "true:0\nfalse:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_time_executes_coproc_command() {
    let status_path = "target/rubash-alias-time-coproc-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!(
        "shopt -s expand_aliases; alias t=time; \
         t -p coproc ATIMEDC {{ :; }}; echo pid:${{ATIMEDC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");
    let _ = fs::remove_file(status_path);
}
