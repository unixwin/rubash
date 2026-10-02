use super::super::*;
use std::fs;

#[test]
fn test_case_command_redirects_clause_stdout() {
    let output_path = "target/rubash-case-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case x in x) echo matched ;; *) echo missed ;; esac > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "matched\ndone\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_redirect_creates_file_without_match() {
    let output_path = "target/rubash-case-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("case z in x) echo missed ;; esac > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_last_clause_can_omit_terminator() {
    let output_path = "target/rubash-case-last-clause-no-terminator-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("case x in x) echo matched; esac > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let case_command = ast.commands[0].case_command.as_ref().unwrap();

    assert_eq!(case_command.clauses[0].terminator_text, None);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "matched\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_empty_case_command_redirect_creates_file() {
    let output_path = "target/rubash-empty-case-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("case z in esac > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0]
        .case_command
        .as_ref()
        .unwrap()
        .clauses
        .is_empty());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_input_redirect_feeds_clause_body() {
    let input_path = "target/rubash-case-command-input.txt";
    let output_path = "target/rubash-case-command-input-output.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case x in x) read first; read second; echo $first/$second ;; esac < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_here_string_feeds_clause_body() {
    let output_path = "target/rubash-case-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("case x in x) read value; echo got:$value ;; esac <<< alpha > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_materializes_input_process_substitution_word() {
    let output_path = "target/rubash-case-input-process-substitution-word-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case <(printf alpha) in *process-subst*|/dev/fd/*) echo matched > {output_path} ;; *) echo missed > {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "matched\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_quoted_patterns_treat_globs_as_literals() {
    let output_path = "target/rubash-case-quoted-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case literal in \"l*\") echo quoted > {output_path} ;; l*) echo pattern > {output_path} ;; esac; \
         pat='l*'; case literal in \"$pat\") echo quoted-var >> {output_path} ;; l*) echo pattern-var >> {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "pattern\npattern-var\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_command_redirects_clause_stdout() {
    let output_path = "target/rubash-alias-case-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c=case\n\
         c x in x) echo matched ;; esac > {output_path}; echo done >> {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "matched\ndone\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_here_string_feeds_clause_body() {
    let output_path = "target/rubash-alias-case-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c=case\n\
         c x in x) cat ;; esac <<< alpha > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    // wt33 (#374): `cat <<< alpha` writes `alpha\n` (CLI/GNU behavior).
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_keeps_multiple_clause_commands() {
    let output_path = "target/rubash-alias-case-multiple-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c=case\n\
         c x in x) read value; echo got:$value ;; esac <<< alpha > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_keeps_multiple_clauses() {
    let output_path = "target/rubash-alias-case-multiple-clauses-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c=case\n\
         c y in x) echo x ;; y) echo y ;; *) echo star ;; esac > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "y\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_keeps_pattern_alternates() {
    let output_path = "target/rubash-alias-case-pattern-alternates-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c=case\n\
         c y in x|y) echo yes ;; *) echo no ;; esac > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_keeps_extglob_pattern_alternates() {
    let output_path = "target/rubash-alias-case-extglob-alternates-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases extglob\nalias c=case\n\
         c foobar in @(foo|bar)) echo no ;; @(foo|foobar)) echo yes ;; *) echo star ;; esac > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_keeps_single_extglob_pattern() {
    let output_path = "target/rubash-alias-case-single-extglob-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases extglob\nalias c=case\n\
         c foo in @(foo)) echo yes ;; *) echo no ;; esac > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_case_keeps_nested_case_body() {
    let output_path = "target/rubash-alias-case-nested-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c=case\n\
         c x in x) case y in y) echo inner >> {output_path} ;; esac; \
         echo outer >> {output_path} ;; esac"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "inner\nouter\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_case_prefix_keeps_nested_case_body() {
    let output_path = "target/rubash-alias-case-prefix-nested-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias c='case x in'\n\
         c x) case y in y) echo inner >> {output_path} ;; esac; \
         echo outer >> {output_path} ;; esac"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "inner\nouter\n");
    let _ = fs::remove_file(output_path);
}
