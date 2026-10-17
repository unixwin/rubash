use super::super::*;
use std::fs;

#[test]
fn test_trap_missing_signal_spec_returns_usage() {
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
fn test_trap_signal_list_can_be_piped() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-trap-l-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap -l | head -2 > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(fs::read_to_string(output_path).unwrap().lines().count(), 2);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_lp_lists_signals_and_returns_success() {
    let _cwd_guard = CwdGuard::take();
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
fn test_trap_p_uppercase_prints_action_only() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-trap-action-only-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("trap 'echo action' TERM; trap -P TERM > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "echo action\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_p_uppercase_requires_signal_and_is_mutually_exclusive() {
    let _cwd_guard = CwdGuard::take();
    for (command, expected_status) in [("trap -P", "2\n"), ("trap -p -P TERM", "2\n")] {
        let output_path = format!(
            "target/rubash-trap-p-upper-status-{}.txt",
            expected_status.len()
        );
        let _ = fs::remove_file(&output_path);
        let input = format!("{command}; echo $? > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&output_path).unwrap(), expected_status);
        let _ = fs::remove_file(output_path);
    }
}

#[test]
fn test_read_r_reads_here_string_without_backslash_escape() {
    let _cwd_guard = CwdGuard::take();
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
fn test_read_e_is_accepted() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-e-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -e value <<< alpha; echo $?:$value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_p_consumes_prompt_argument() {
    let _cwd_guard = CwdGuard::take();
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
fn test_read_combined_ep_consumes_prompt_argument() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-ep-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -ep prompt value <<< alpha; echo $value > {output_path}");
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
fn test_read_combined_ep_compact_prompt() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-ep-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -epprompt value <<< alpha; echo $value > {output_path}");
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
fn test_read_combined_rep_consumes_prompt_and_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-rep-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rep prompt value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_erp_compact_prompt_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-erp-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -erpprompt value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_resp_consumes_prompt_and_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-resp-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -resp prompt value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_serp_compact_prompt_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-serp-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -serpprompt value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_sp_consumes_prompt_argument() {
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
fn test_read_combined_ei_consumes_initial_text() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-ei-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -ei seed value <<< alpha; echo $value > {output_path}");
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
fn test_read_combined_ei_compact_initial_text() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-ei-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -eiseed value <<< alpha; echo $value > {output_path}");
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
fn test_read_combined_rei_consumes_initial_text_and_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-rei-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rei seed value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_eri_compact_initial_text_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-eri-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -eriseed value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_resi_consumes_initial_text_and_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-resi-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -resi seed value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_seri_compact_initial_text_reads_raw() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-read-seri-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -seriseed value <<< 'alpha\\beta'; echo $value > {output_path}");
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
fn test_read_combined_ri_consumes_initial_text_and_reads_raw() {
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
    let _cwd_guard = CwdGuard::take();
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
