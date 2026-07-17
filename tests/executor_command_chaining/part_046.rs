use super::super::*;
use std::fs;

#[test]
fn test_trap_missing_signal_spec_returns_usage() {
    let output_path = "target/rubash-trap-missing-signal-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 512; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_l_redirects_signal_list() {
    let output_path = "target/rubash-trap-l-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with(" 1) SIGHUP"));
    assert!(output.contains("15) SIGTERM"));
    assert!(output.contains("64) SIGRTMAX"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_lp_lists_signals_and_returns_success() {
    let output_path = "target/rubash-trap-lp-status-output.txt";
    let list_path = "target/rubash-trap-lp-list-output.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(list_path);
    let input = format!("trap -lp > {list_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(list_path);
}

#[test]
fn test_read_r_reads_here_string_without_backslash_escape() {
    let output_path = "target/rubash-read-r-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -r line <<< 'alpha\\beta'; echo $line > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_p_consumes_prompt_argument() {
    let output_path = "target/rubash-read-p-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -p prompt value <<< alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sp_consumes_prompt_argument() {
    let output_path = "target/rubash-read-sp-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -sp prompt value <<< alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sp_compact_prompt() {
    let output_path = "target/rubash-read-sp-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -spprompt value <<< alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rp_consumes_prompt_and_reads_raw() {
    let output_path = "target/rubash-read-rp-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rp prompt value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rp_compact_prompt_reads_raw() {
    let output_path = "target/rubash-read-rp-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rpprompt value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rsp_consumes_prompt_and_reads_raw() {
    let output_path = "target/rubash-read-rsp-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rsp prompt value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_srp_compact_prompt_reads_raw() {
    let output_path = "target/rubash-read-srp-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -srpprompt value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_i_consumes_initial_text_argument() {
    let output_path = "target/rubash-read-i-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -i seed value <<< alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_si_consumes_initial_text() {
    let output_path = "target/rubash-read-si-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -si seed value <<< alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_si_compact_initial_text() {
    let output_path = "target/rubash-read-si-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -siseed value <<< alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_ri_consumes_initial_text_and_reads_raw() {
    let output_path = "target/rubash-read-ri-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -ri seed value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_ri_compact_initial_text_reads_raw() {
    let output_path = "target/rubash-read-ri-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -riseed value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rsi_consumes_initial_text_and_reads_raw() {
    let output_path = "target/rubash-read-rsi-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rsi seed value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sri_compact_initial_text_reads_raw() {
    let output_path = "target/rubash-read-sri-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -sriseed value <<< 'alpha\\beta'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_without_r_processes_backslash_escape() {
    let output_path = "target/rubash-read-backslash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read line <<< 'alpha\\beta'; echo $line > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alphabeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_multiple_names_assigns_remainder_to_last() {
    let output_path = "target/rubash-read-multiple-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read first rest <<< 'alpha beta gamma'; echo $first:$rest > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alpha:beta gamma\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_multiple_names_respects_backslash_escaped_ifs() {
    let output_path = "target/rubash-read-escaped-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read first rest <<< 'alpha\\ beta gamma'; printf '<%s><%s>\\n' \"$first\" \"$rest\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alpha beta><gamma>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_r_multiple_names_treats_backslash_as_literal() {
    let output_path = "target/rubash-read-r-multiple-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "read -r first rest <<< 'alpha\\ beta gamma'; printf '<%s><%s>\\n' \"$first\" \"$rest\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alpha\\><beta gamma>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_silent_raw_treats_backslash_as_literal() {
    let output_path = "target/rubash-read-sr-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -sr value <<< 'alpha\\ beta'; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<alpha\\ beta>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_multiple_names_respects_backslash_escaped_custom_ifs() {
    let output_path = "target/rubash-read-escaped-custom-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "IFS=, read first rest <<< 'alpha\\,beta,gamma'; printf '<%s><%s>\\n' \"$first\" \"$rest\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alpha,beta><gamma>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_multiple_names_uses_custom_ifs() {
    let output_path = "target/rubash-read-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("IFS=, read first rest <<< 'alpha,beta,gamma'; echo $first:$rest > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alpha:beta,gamma\n"
    );
    let _ = fs::remove_file(output_path);
}
