use super::super::*;
use std::fs;

#[test]
fn test_set_posix_updates_visible_option_state() {
    let output_path = "target/rubash-set-posix-option-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o posix; type break > {output_path}; set -o >> {output_path}; \
         set +o posix; type break >> {output_path}; set -o >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("break is a special shell builtin\n"));
    assert!(output.contains("break is a shell builtin\n"));
    let posix_lines: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("posix"))
        .collect();
    assert_eq!(posix_lines, ["posix          \ton", "posix          \toff"]);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shellopts_assignment_reports_readonly() {
    let status_path = "target/rubash-shellopts-readonly-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("SHELLOPTS=ignored; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_bashopts_reflects_shopt_options() {
    let output_path = "target/rubash-bashopts-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $BASHOPTS > {output_path}; \
         shopt -s checkhash; echo $BASHOPTS >> {output_path}; \
         shopt -u checkwinsize; echo $BASHOPTS >> {output_path}; \
         shopt -u checkhash; \
         readonly -p >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines: Vec<&str> = output.lines().collect();
    assert!(lines[0].contains("checkwinsize"));
    assert!(lines[0].contains("cmdhist"));
    assert!(!lines[0].contains("checkhash"));
    assert!(lines[1].contains("checkhash"));
    assert!(lines[1].contains("checkwinsize"));
    assert!(lines[2].contains("checkhash"));
    assert!(!lines[2].contains("checkwinsize"));
    assert!(output.contains("declare -r BASHOPTS=\""));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bashopts_assignment_reports_readonly() {
    let status_path = "target/rubash-bashopts-readonly-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("BASHOPTS=$BASHOPTS; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_set_noclobber_updates_shell_flags() {
    let output_path = "target/rubash-set-noclobber-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -C; echo $- > {output_path}; set +C; echo $- >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let lines: Vec<String> = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines[0].contains('C'));
    assert!(!lines[1].contains('C'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_noglob_updates_shell_flags_and_option_tests() {
    let output_path = "target/rubash-set-noglob-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -f; echo $- > {output_path}; [[ -o noglob ]]; echo $? >> {output_path}; \
         set +f; echo $- >> {output_path}; [[ -o noglob ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let lines: Vec<String> = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines[0].contains('f'));
    assert_eq!(lines[1], "0");
    assert!(!lines[2].contains('f'));
    assert_eq!(lines[3], "1");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_noglob_with_positional_operands() {
    let output_path = "target/rubash-set-noglob-operands-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -f alpha beta; echo $# $1 $2 $- > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("2 alpha beta "));
    assert!(output.trim_end().ends_with('f'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_noexec_updates_shell_option() {
    let tokens = tokenize("set -n");
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(executor.get_env("__RUBASH_SETOPT_noexec"), Some("1"));
}

#[test]
fn test_set_noexec_skips_later_commands_and_redirections() {
    let output_path = "target/rubash-noexec-skips-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -n; echo should-not-run > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_additional_set_short_flags_update_shell_options() {
    let output_path = "target/rubash-set-extra-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $- > {output_path}; set -abPkv; echo $- >> {output_path}; \
         [[ -o allexport ]]; echo $? >> {output_path}; [[ -o notify ]]; echo $? >> {output_path}; \
         [[ -o physical ]]; echo $? >> {output_path}; [[ -o keyword ]]; echo $? >> {output_path}; \
         [[ -o verbose ]]; echo $? >> {output_path}; set +abPkvh; echo $- >> {output_path}; \
         [[ -o hashall ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let lines: Vec<String> = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines[0].contains('h'));
    for flag in ['a', 'b', 'P', 'k', 'v'] {
        assert!(lines[1].contains(flag));
    }
    assert_eq!(lines[2..7], ["0", "0", "0", "0", "0"].map(str::to_string));
    for flag in ['a', 'b', 'P', 'k', 'v', 'h'] {
        assert!(!lines[7].contains(flag));
    }
    assert_eq!(lines[8], "1");
    let _ = fs::remove_file(output_path);
}
