use super::super::*;
use std::fs;

#[test]
fn test_alias_introduced_arithmetic_for_executes_loop() {
    let output_path = "target/rubash-alias-arithmetic-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias f=for\n\
         f (( i = 0; i < 3; i++ )); do echo $i; done > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_arithmetic_for_accepts_brace_group_body() {
    let output_path = "target/rubash-alias-arithmetic-for-brace-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias f=for\n\
         f (( i = 0; i < 2; i++ )); {{ echo $i; }} > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_arithmetic_for_accepts_empty_init() {
    let output_path = "target/rubash-alias-arithmetic-for-empty-init-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias f=for\ni=0\n\
         f (( ; i < 3; i++ )); do echo $i; done > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_arithmetic_for_accepts_empty_update() {
    let output_path = "target/rubash-alias-arithmetic-for-empty-update-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias f=for\n\
         f (( i = 0; i < 3; )); do echo $i; (( i++ )); done > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_compact_arithmetic_for_accepts_empty_test() {
    let output_path = "target/rubash-alias-compact-arithmetic-for-empty-test-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias f=for\n\
         f ((i=0;;i++)); do echo $i; if (( i == 2 )); then break; fi; done > {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}
