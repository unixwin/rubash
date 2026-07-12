use super::super::*;
use std::fs;

#[test]
fn test_unquoted_command_substitution_word_splits_with_adjacent_text() {
    let output_path = "target/rubash-comsub-word-split-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "one=one; four=four; five='fi ve'; \
         printf '[%s]\\n' $one`echo two three`$four > {output_path}; \
         printf '[%s]\\n' `echo two three`$five >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "[onetwo]\n[threefour]\n[two]\n[threefi]\n[ve]\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_cat_command_substitution_reads_files_and_strips_trailing_newlines() {
    let input_path = "target/rubash-cat-command-substitution-input.txt";
    let output_path = "target/rubash-cat-command-substitution-output.txt";
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    fs::write(input_path, "a\nb\n\n").unwrap();
    let input = format!(
        "v=$(cat {input_path}); printf 'v=<%s> len:%s\\n' \"$v\" \"${{#v}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<a\nb> len:3\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_file_command_substitution_strips_trailing_newlines() {
    let input_path = "target/rubash-readfile-command-substitution-input.txt";
    let output_path = "target/rubash-readfile-command-substitution-output.txt";
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    fs::write(input_path, "a\nb\n\n").unwrap();
    let input = format!(
        "v=$(< {input_path}); printf 'v=<%s> len:%s\\n' \"$v\" \"${{#v}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<a\nb> len:3\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_file_command_substitution_expands_glob() {
    let input_path = "target/rubash-readfile-command-substitution-glob-input.txt";
    let output_path = "target/rubash-readfile-command-substitution-glob-output.txt";
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    fs::write(input_path, "globbed\n").unwrap();
    let input = format!(
        "v=$(< target/rubash-readfile-command-substitution-glob-*); \
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
        "v=<globbed> status:0\n"
    );
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_file_command_substitution_missing_file_sets_status() {
    let missing_path = "target/rubash-readfile-command-substitution-missing.txt";
    let output_path = "target/rubash-readfile-command-substitution-missing-output.txt";
    let _ = fs::remove_file(missing_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=$(< {missing_path}); printf 'v=<%s> status:%s\\n' \"$v\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<> status:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_external_command_substitution_captures_stdout() {
    let bin_dir = "target/rubash-command-substitution-bin";
    let helper_path = format!("{bin_dir}/rubash-comsub-helper");
    let output_path = "target/rubash-external-command-substitution-output.txt";
    let _ = fs::create_dir_all(bin_dir);
    let _ = fs::remove_file(&helper_path);
    let _ = fs::remove_file(output_path);
    write_executable(&helper_path, "#!/usr/bin/env bash\nprintf 'a\\nb\\n\\n'\n").unwrap();
    let input = format!(
        "v=$(rubash-comsub-helper); printf 'v=<%s> len:%s\\n' \"$v\" \"${{#v}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "v=<a\nb> len:3\n");
    let _ = fs::remove_file(helper_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mktemp_t_command_substitution_succeeds() {
    let output_path = target_test_path("rubash-mktemp-t-command-substitution-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "tmp=$(mktemp -t cb.XXXXXX) || exit 1\n\
         test -f \"$tmp\"\n\
         echo status:$?:$tmp > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("status:0:"));
    assert!(output.contains("cb."));
    let temp_path = output.trim_end().trim_start_matches("status:0:");
    let _ = fs::remove_file(shell_output_path_to_host(temp_path));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mktemp_d_command_substitution_creates_directory() {
    let output_path = target_test_path("rubash-mktemp-d-command-substitution-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "tmp=$(mktemp -d) || exit 1\n\
         test -d \"$tmp\"\n\
         echo status:$?:$tmp > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("status:0:"));
    assert!(output.contains("rubash-mktemp."));
    let temp_path = output.trim_end().trim_start_matches("status:0:");
    let _ = fs::remove_dir_all(shell_output_path_to_host(temp_path));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_cp_copies_file_to_mktemp_directory() {
    let source_path = target_test_path("rubash-cp-source.txt");
    let output_path = target_test_path("rubash-cp-mktemp-directory-output.txt");
    let shell_source_path = shell_test_path(&source_path);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&source_path);
    let _ = fs::remove_file(&output_path);
    fs::write(&source_path, "copied\n").unwrap();
    let input = format!(
        "tmp=$(mktemp -d) || exit 1\n\
         cp {shell_source_path} \"$tmp\"\n\
         test -f \"$tmp/rubash-cp-source.txt\"\n\
         echo status:$?:$tmp > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("status:0:"));
    let temp_path = output.trim_end().trim_start_matches("status:0:");
    let _ = fs::remove_dir_all(shell_output_path_to_host(temp_path));
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_multiline_brace_group_continues_until_closing_brace() {
    let output_path = target_test_path("rubash-multiline-brace-group-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "{{ first=alpha &&\n\
           second=beta\n\
         }} || exit 1\n\
         echo \"$first/$second\" > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(output_path);
}
