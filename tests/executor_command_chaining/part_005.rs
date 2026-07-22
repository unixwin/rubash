use super::super::*;
use std::fs;

#[test]
fn test_random_assignment_reseeds_sequence() {
    let output_path = "target/rubash-random-seed-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RANDOM=42; first=$RANDOM; RANDOM=42; second=$RANDOM; printf '%s:%s\\n' \"$first\" \"$second\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let (first, second) = output.trim_end().split_once(':').unwrap();
    assert_eq!(first, second);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_random_expands_inside_arithmetic_expressions() {
    let output_path = "target/rubash-random-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RANDOM=42; first=$((RANDOM)); RANDOM=42; second=$((RANDOM)); printf '%s:%s\\n' \"$first\" \"$second\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let (first, second) = output.trim_end().split_once(':').unwrap();
    assert_eq!(first, second);
    assert!(first.parse::<u32>().unwrap() <= 32767);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_random_advances_inside_arithmetic_command() {
    let output_path = "target/rubash-random-arithmetic-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RANDOM=42; (( first=RANDOM, second=RANDOM )); printf '%s:%s\\n' \"$first\" \"$second\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let (first, second) = output.trim_end().split_once(':').unwrap();
    assert_ne!(first, second);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_srandom_expands_inside_arithmetic_expressions() {
    let output_path = "target/rubash-srandom-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s\\n' \"$((SRANDOM))\" \"$((SRANDOM))\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let values = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(|line| line.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_ne!(values[0], values[1]);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bashpid_expands_to_current_shell_pid() {
    let output_path = "target/rubash-bashpid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s:%s\\n' \"$BASHPID\" \"$$\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let (bashpid, shell_pid) = output.trim_end().split_once(':').unwrap();
    assert_eq!(bashpid, shell_pid);
    assert_eq!(bashpid.parse::<u32>().unwrap(), std::process::id());
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bashpid_assignment_does_not_override_dynamic_value() {
    let output_path = "target/rubash-bashpid-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("BASHPID=1; printf '%s\\n' \"$BASHPID\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap().trim_end(),
        std::process::id().to_string()
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bashpid_changes_in_command_substitution() {
    let output_path = "target/rubash-bashpid-command-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf '%s:%s:%s:%s\\n' \"$$\" \"$BASHPID\" \"$(echo $BASHPID)\" \"$( (echo $BASHPID) )\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let parts: Vec<_> = output.trim_end().split(':').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], parts[1]);
    assert_ne!(parts[0], parts[2]);
    assert_ne!(parts[0], parts[3]);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_current_shell_command_substitution_captures_stdout_and_keeps_side_effects() {
    let output_path = "target/rubash-current-shell-command-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=old; text=${{ value=new; echo alpha; echo; }}; \
         printf 'text=<%s> value=<%s> status:%s\\n' \"$text\" \"$value\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "text=<alpha> value=<new> status:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_current_shell_reply_command_substitution_uses_reply_without_capturing_stdout() {
    let output_path = "target/rubash-current-shell-reply-command-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset REPLY; text=${{| REPLY=result; value=kept; }}; \
         printf 'text=<%s> reply=<%s> value=<%s>\\n' \"$text\" \"$REPLY\" \"$value\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "text=<result> reply=<result> value=<kept>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_adjacent_current_shell_reply_substitutions_update_reply_left_to_right() {
    let output_path = "target/rubash-adjacent-current-shell-reply-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "REPLY=outside; echo ${{| REPLY=inside1; }}-${{| REPLY=inside2; }}-$REPLY > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "inside1-inside2-outside\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_current_shell_reply_substitution_expands_inside_command_substitutions() {
    let output_path = "target/rubash-nested-current-shell-reply-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $(echo combined ${{| REPLY=comsubs; }}) > {output_path}; \
         echo ${{ echo $(echo combined ${{| REPLY=comsubs; }}); }} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "combined comsubs\ncombined comsubs\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_command_substitution_strips_trailing_newlines() {
    let output_path = "target/rubash-printf-command-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(printf 'a\\n\\n'); printf 'trail:<%s> len:%s\\n' \"$v\" \"${{#v}}\" > {output_path}; \
         w=$(printf 'a\\nb\\n'); printf 'mid:<%s> len:%s\\n' \"$w\" \"${{#w}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "trail:<a> len:1\nmid:<a\nb> len:3\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_command_substitution_captures_stdout() {
    let output_path = "target/rubash-compound-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(for value in a b; do echo $value; done); printf 'v=<%s> len:%s\\n' \"$v\" \"${{#v}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<a\nb> len:3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_list_substitution_captures_stdout() {
    let output_path = "target/rubash-list-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(echo first; echo second); printf 'v=<%s> len:%s\\n' \"$v\" \"${{#v}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v=<first\nsecond> len:12\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_substitution_captures_stdout() {
    let output_path = "target/rubash-case-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(case beta in alpha) printf alpha ;; beta) printf beta ;; esac); \
         printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v=<beta> status:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_substitution_allows_clause_without_terminator_after_if() {
    let output_path = "target/rubash-case-command-subst-if-no-terminator-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(case x in x) if ((1)); then printf ok; fi esac); \
         printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v=<ok> status:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_substitution_allows_nested_case_without_outer_terminator() {
    let output_path = "target/rubash-case-command-subst-nested-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(case x in x) case y in y) printf a;; esac esac); \
         printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<a> status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_substitution_keeps_reserved_patterns_before_for_body() {
    let output_path = "target/rubash-case-command-subst-reserved-patterns-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(case k in else|done|time|esac) for f in 1 2 3; do printf x; done esac); \
         printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<> status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_substitution_keeps_esac_after_comment() {
    let output_path = "target/rubash-case-command-subst-comment-output.txt";
    let error_path = "target/rubash-case-command-subst-comment-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        ": $(case a in a) printf ok ;; # comment\nesac) 2> {error_path}; \
         printf 'status:%s\\n' \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:0\n");
    assert_eq!(fs::read_to_string(error_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_and_or_command_substitution_captures_stdout() {
    let output_path = "target/rubash-and-or-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(false || printf fallback); printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v=<fallback> status:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_multi_command_substitution_keeps_assignments_local() {
    let output_path = "target/rubash-multi-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(x=inner; printf \"$x\"); printf 'v=<%s> x=<%s> status:%s\\n' \"$v\" \"$x\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v=<inner> x=<> status:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_pipeline_command_substitution_captures_stdout() {
    let output_path = "target/rubash-function-pipeline-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "inner() {{ echo 123 | grep 123; }}; \
         outer=\"$(inner)\"; \
         printf 'outer=<%s>\\n' \"$outer\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "outer=<123>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_substitution_captures_timed_stdout() {
    let output_path = "target/rubash-time-command-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("v=$(time -p echo hi); printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v=<hi> status:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_substitution_inverts_status() {
    let output_path = "target/rubash-time-command-substitution-invert-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("v=$(time ! false); printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<> status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_adjacent_command_substitutions_stay_in_one_word() {
    let output_path = "target/rubash-adjacent-command-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $(printf left):$(printf right) > {output_path}; echo `printf tick`:`printf tock` >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "left:right\ntick:tock\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_substitution_captures_heredoc_pipeline() {
    let output_path = "target/rubash-comsub-heredoc-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "TEST=$(cat <<EOF | sort -u\nabc\ngeh\ndef\nabc\nEOF\n); echo $TEST > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc def geh\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_backtick_command_substitution_captures_heredoc() {
    let output_path = "target/rubash-backtick-comsub-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("foo=`cat <<EOF\nhi\nEOF`\necho \"$foo\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_substitution_captures_heredoc_with_parentheses() {
    let output_path = "target/rubash-comsub-heredoc-parens-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "text=$(cat <<EOF\nthese balanced parens ( ) are not a problem\nEOF\n); echo $text > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "these balanced parens ( ) are not a problem\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_substitution_heredoc_delimiter_closes_before_paren() {
    let output_path = "target/rubash-comsub-heredoc-delim-before-paren-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("text=$(cat <<EOF\nhere is the text\nEOF)\necho = $text = > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "= here is the text =\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_substitution_captures_sequential_heredocs() {
    let output_path = "target/rubash-comsub-sequential-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo $(cat <<A; cat <<B\none\nA\ntwo\nB\n) > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one two\n");
    let _ = fs::remove_file(output_path);
}
