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
fn test_compgen_wordlist_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-wordlist-output.txt";
    let status_path = "target/rubash-compgen-wordlist-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compgen -W 'alpha beta gamma' b > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_wordlist_prefix_suffix_and_no_match_status() {
    let output_path = "target/rubash-compgen-prefix-suffix-output.txt";
    let status_path = "target/rubash-compgen-prefix-suffix-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P pre- -S -suf -W 'alpha beta' a > {output_path}; echo first:$? > {status_path}; \
         compgen -W 'alpha beta' z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "pre-alpha-suf\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_v_assigns_matches_to_indexed_array() {
    let output_path = "target/rubash-compgen-v-output.txt";
    let status_path = "target/rubash-compgen-v-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -V matches -W 'alpha amber beta' a > {output_path}; \
         echo $? > {status_path}; \
         printf '<%s>|<%s>|<%s>\\n' \"${{matches[0]}}\" \"${{matches[1]}}\" \"${{#matches[@]}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alpha>|<amber>|<2>\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_compact_v_assigns_matches_to_indexed_array() {
    let output_path = "target/rubash-compgen-compact-v-output.txt";
    let status_path = "target/rubash-compgen-compact-v-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -Vmatches -W 'red blue brown' b > {output_path}; \
         echo $? > {status_path}; \
         printf '%s:%s:%s\\n' \"${{matches[0]}}\" \"${{matches[1]}}\" \"${{#matches[@]}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "blue:brown:2\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_alias_flag_filters_alias_names() {
    let output_path = "target/rubash-compgen-alias-output.txt";
    let status_path = "target/rubash-compgen-alias-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "alias rubash_cg_alpha='echo a' rubash_cg_beta='echo b'; \
         compgen -a rubash_cg_ > {output_path}; echo first:$? > {status_path}; \
         compgen -a rubash_cg_z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash_cg_alpha\nrubash_cg_beta\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_alias_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-alias-action-output.txt";
    let status_path = "target/rubash-compgen-alias-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "alias rubash_cga_alpha='echo a' rubash_cga_beta='echo b'; \
         compgen -P alias: -S :end -A alias -X '*beta' rubash_cga_ > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias:rubash_cga_alpha:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_arrayvar_action_filters_array_variables() {
    let output_path = "target/rubash-compgen-arrayvar-output.txt";
    let status_path = "target/rubash-compgen-arrayvar-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "rubash_cgav_indexed=(a); declare -A rubash_cgav_assoc=([k]=v); rubash_cgav_scalar=s; \
         compgen -A arrayvar rubash_cgav_ > {output_path}; echo first:$? > {status_path}; \
         compgen -A arrayvar rubash_cgav_z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash_cgav_assoc\nrubash_cgav_indexed\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_arrayvar_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-arrayvar-action-output.txt";
    let status_path = "target/rubash-compgen-arrayvar-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "rubash_cgava_indexed=(a); declare -A rubash_cgava_assoc=([k]=v); \
         compgen -P array: -S :end -A arrayvar -X '*assoc' rubash_cgava_ > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "array:rubash_cgava_indexed:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_binding_action_filters_readline_function_names() {
    let output_path = "target/rubash-compgen-binding-output.txt";
    let status_path = "target/rubash-compgen-binding-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -A binding beginning-of- > {output_path}; echo first:$? > {status_path}; \
         compgen -A binding no-such-binding >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "beginning-of-history\nbeginning-of-line\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_binding_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-binding-action-output.txt";
    let status_path = "target/rubash-compgen-binding-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P bind: -S :end -A binding -X '*word' backward- > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "bind:backward-byte:end\nbind:backward-char:end\nbind:backward-delete-char:end\nbind:backward-kill-line:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_filter_pattern_excludes_matching_candidates() {
    let output_path = "target/rubash-compgen-filter-output.txt";
    let status_path = "target/rubash-compgen-filter-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input =
        format!("compgen -W 'alpha beta bar' -X 'b*' > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_filter_pattern_inversion_keeps_matching_candidates() {
    let output_path = "target/rubash-compgen-filter-invert-output.txt";
    let status_path = "target/rubash-compgen-filter-invert-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P pre- -S -suf -W 'alpha beta bar' -X '!b*' > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "pre-beta-suf\npre-bar-suf\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_filter_pattern_all_filtered_keeps_generation_status() {
    let output_path = "target/rubash-compgen-filter-empty-output.txt";
    let status_path = "target/rubash-compgen-filter-empty-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input =
        format!("compgen -W 'alpha beta' -X 'a*' a > {output_path}; echo $? > {status_path}");
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K035, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_glob_pattern_outputs_path_matches() {
    let dir_path = "target/rubash-compgen-glob";
    let output_path = "target/rubash-compgen-glob-output.txt";
    let status_path = "target/rubash-compgen-glob-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/alpha.txt"), "").unwrap();
    fs::write(format!("{dir_path}/beta.txt"), "").unwrap();
    fs::write(format!("{dir_path}/gamma.log"), "").unwrap();
    let input = format!(
        "compgen -G '{dir_path}/*.txt' > {output_path}; echo first:$? > {status_path}; \
         compgen -G '{dir_path}/*.zzz' >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("{dir_path}/alpha.txt\n{dir_path}/beta.txt\n")
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_glob_pattern_uses_prefix_and_suffix() {
    let dir_path = "target/rubash-compgen-glob-wrap";
    let output_path = "target/rubash-compgen-glob-wrap-output.txt";
    let status_path = "target/rubash-compgen-glob-wrap-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/one.rs"), "").unwrap();
    let input = format!(
        "compgen -P pre- -S -suf -G '{dir_path}/*.rs' > {output_path}; echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("pre-{dir_path}/one.rs-suf\n")
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_directory_flag_outputs_directory_matches() {
    let dir_path = "target/rubash-compgen-directory";
    let output_path = "target/rubash-compgen-directory-output.txt";
    let status_path = "target/rubash-compgen-directory-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(format!("{dir_path}/alpha")).unwrap();
    fs::create_dir_all(format!("{dir_path}/beta")).unwrap();
    fs::write(format!("{dir_path}/apple.txt"), "").unwrap();
    let input = format!(
        "compgen -d {dir_path}/a > {output_path}; echo first:$? > {status_path}; \
         compgen -d {dir_path}/z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("{dir_path}/alpha\n")
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_directory_action_uses_prefix_suffix_and_filter() {
    let dir_path = "target/rubash-compgen-directory-action";
    let output_path = "target/rubash-compgen-directory-action-output.txt";
    let status_path = "target/rubash-compgen-directory-action-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(format!("{dir_path}/alpha")).unwrap();
    fs::create_dir_all(format!("{dir_path}/amber")).unwrap();
    fs::create_dir_all(format!("{dir_path}/beta")).unwrap();
    let input = format!(
        "compgen -P dir: -S :end -A directory -X '*ber' {dir_path}/a > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("dir:{dir_path}/alpha:end\n")
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_file_flag_outputs_file_and_directory_matches() {
    let dir_path = "target/rubash-compgen-file";
    let output_path = "target/rubash-compgen-file-output.txt";
    let status_path = "target/rubash-compgen-file-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(format!("{dir_path}/alpha")).unwrap();
    fs::create_dir_all(format!("{dir_path}/beta")).unwrap();
    fs::write(format!("{dir_path}/apple.txt"), "").unwrap();
    fs::write(format!("{dir_path}/apricot.log"), "").unwrap();
    let input = format!(
        "compgen -f {dir_path}/a > {output_path}; echo first:$? > {status_path}; \
         compgen -f {dir_path}/z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("{dir_path}/alpha\n{dir_path}/apple.txt\n{dir_path}/apricot.log\n")
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_file_action_uses_prefix_suffix_and_filter() {
    let dir_path = "target/rubash-compgen-file-action";
    let output_path = "target/rubash-compgen-file-action-output.txt";
    let status_path = "target/rubash-compgen-file-action-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(format!("{dir_path}/alpha")).unwrap();
    fs::write(format!("{dir_path}/apple.txt"), "").unwrap();
    fs::write(format!("{dir_path}/amber.txt"), "").unwrap();
    let input = format!(
        "compgen -P file: -S :end -A file -X '*.txt' {dir_path}/a > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("file:{dir_path}/alpha:end\n")
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_builtin_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-builtin-output.txt";
    let status_path = "target/rubash-compgen-builtin-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P pre- -S -suf -A builtin pr > {output_path}; echo first:$? > {status_path}; \
         compgen -A builtin zz >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "pre-printf-suf\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_builtin_flag_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-builtin-flag-output.txt";
    let status_path = "target/rubash-compgen-builtin-flag-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P pre- -S -suf -b pr > {output_path}; echo first:$? > {status_path}; \
         compgen -b zz >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "pre-printf-suf\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_command_flag_combines_shell_and_path_commands() {
    let dir_path = "target/rubash-compgen-command-bin";
    let output_path = "target/rubash-compgen-command-output.txt";
    let status_path = "target/rubash-compgen-command-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/rubash_cgc_path"), "").unwrap();
    let input = format!(
        "PATH={dir_path}; alias rubash_cgc_alias='echo a'; rubash_cgc_func() {{ :; }}; \
         compgen -c rubash_cgc_ > {output_path}; echo first:$? > {status_path}; \
         compgen -c rubash_cgc_z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash_cgc_alias\nrubash_cgc_func\nrubash_cgc_path\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_command_action_uses_prefix_suffix_and_filter() {
    let dir_path = "target/rubash-compgen-command-action-bin";
    let output_path = "target/rubash-compgen-command-action-output.txt";
    let status_path = "target/rubash-compgen-command-action-status.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/rubash_cgca_path"), "").unwrap();
    let input = format!(
        "PATH={dir_path}; alias rubash_cgca_alias='echo a'; rubash_cgca_func() {{ :; }}; \
         compgen -P cmd: -S :end -A command -X '*func' rubash_cgca_ > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "cmd:rubash_cgca_alias:end\ncmd:rubash_cgca_path:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_enabled_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-enabled-output.txt";
    let status_path = "target/rubash-compgen-enabled-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P enabled: -A enabled pr > {output_path}; echo first:$? > {status_path}; \
         compgen -A enabled zz >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "enabled:printf\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_enabled_action_omits_disabled_builtins() {
    let output_path = "target/rubash-compgen-enabled-disabled-output.txt";
    let status_path = "target/rubash-compgen-enabled-disabled-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "enable -n printf; \
         compgen -A enabled pr > {output_path}; echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_disabled_action_filters_disabled_builtins() {
    let output_path = "target/rubash-compgen-disabled-output.txt";
    let status_path = "target/rubash-compgen-disabled-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "enable -n printf pwd; \
         compgen -P disabled: -S :end -A disabled -X '*pwd' p > {output_path}; echo first:$? > {status_path}; \
         compgen -A disabled z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "disabled:printf:end\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_variable_flag_filters_shell_variables() {
    let output_path = "target/rubash-compgen-variable-output.txt";
    let status_path = "target/rubash-compgen-variable-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "RUBASH_COMPGEN_VAR_ALPHA=1; RUBASH_COMPGEN_VAR_BETA=2; \
         compgen -v RUBASH_COMPGEN_VAR_ > {output_path}; echo first:$? > {status_path}; \
         compgen -v RUBASH_COMPGEN_VAR_Z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "RUBASH_COMPGEN_VAR_ALPHA\nRUBASH_COMPGEN_VAR_BETA\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_variable_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-variable-action-output.txt";
    let status_path = "target/rubash-compgen-variable-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "RUBASH_COMPGEN_ACTION_ALPHA=1; RUBASH_COMPGEN_ACTION_BETA=2; \
         compgen -P var: -S :end -A variable -X '*BETA' RUBASH_COMPGEN_ACTION_ > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "var:RUBASH_COMPGEN_ACTION_ALPHA:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_export_action_filters_exported_variables() {
    let output_path = "target/rubash-compgen-export-output.txt";
    let status_path = "target/rubash-compgen-export-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "export RUBASH_COMPGEN_EXPORT_ALPHA=1 RUBASH_COMPGEN_EXPORT_BETA=2; \
         RUBASH_COMPGEN_EXPORT_SCALAR=3; \
         compgen -A export RUBASH_COMPGEN_EXPORT_ > {output_path}; echo first:$? > {status_path}; \
         compgen -A export RUBASH_COMPGEN_EXPORT_Z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "RUBASH_COMPGEN_EXPORT_ALPHA\nRUBASH_COMPGEN_EXPORT_BETA\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_export_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-export-action-output.txt";
    let status_path = "target/rubash-compgen-export-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "export RUBASH_COMPGEN_EXPORT_ACTION_ALPHA=1 RUBASH_COMPGEN_EXPORT_ACTION_BETA=2; \
         compgen -P export: -S :end -A export -X '*BETA' RUBASH_COMPGEN_EXPORT_ACTION_ > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "export:RUBASH_COMPGEN_EXPORT_ACTION_ALPHA:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_readonly_action_filters_readonly_variables() {
    let output_path = "target/rubash-compgen-readonly-output.txt";
    let status_path = "target/rubash-compgen-readonly-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "readonly RUBASH_COMPGEN_READONLY_ALPHA=1 RUBASH_COMPGEN_READONLY_BETA=2; \
         RUBASH_COMPGEN_READONLY_SCALAR=3; \
         compgen -A readonly RUBASH_COMPGEN_READONLY_ > {output_path}; echo first:$? > {status_path}; \
         compgen -A readonly RUBASH_COMPGEN_READONLY_Z >> {output_path}; echo second:$? >> {status_path}"
    );
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K036, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    // GNU 5.3.0 has no `readonly` compgen action: both shells reject it with
    // `compgen: readonly: invalid action name` (rc 2); the redirections
    // still create the (empty) output file (probe wt37-374 K036).
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:2\nsecond:2\n"
    );
    assert_eq!(
        cli_err
            .matches("compgen: readonly: invalid action name")
            .count(),
        2
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_readonly_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-readonly-action-output.txt";
    let status_path = "target/rubash-compgen-readonly-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "readonly RUBASH_COMPGEN_READONLY_ACTION_ALPHA=1 RUBASH_COMPGEN_READONLY_ACTION_BETA=2; \
         compgen -P readonly: -S :end -A readonly -X '*BETA' RUBASH_COMPGEN_READONLY_ACTION_ > {output_path}; \
         echo $? > {status_path}"
    );
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K037, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    // GNU 5.3.0 has no `readonly` compgen action: both shells reject it with
    // `compgen: readonly: invalid action name` (rc 2); the redirection still
    // creates the (empty) output file (probe wt37-374 K037).
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    assert!(cli_err.contains("compgen: readonly: invalid action name"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_export_flag_filters_exported_variables() {
    let output_path = "target/rubash-compgen-export-flag-output.txt";
    let status_path = "target/rubash-compgen-export-flag-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "export RUBASH_COMPGEN_EXPORT_FLAG_ALPHA=1 RUBASH_COMPGEN_EXPORT_FLAG_BETA=2; \
         RUBASH_COMPGEN_EXPORT_FLAG_LOCAL=3; \
         compgen -e RUBASH_COMPGEN_EXPORT_FLAG_ > {output_path}; echo first:$? > {status_path}; \
         compgen -e RUBASH_COMPGEN_EXPORT_FLAG_Z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "RUBASH_COMPGEN_EXPORT_FLAG_ALPHA\nRUBASH_COMPGEN_EXPORT_FLAG_BETA\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_keyword_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-keyword-output.txt";
    let status_path = "target/rubash-compgen-keyword-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compgen -P kw: -A keyword wh > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "kw:while\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_keyword_flag_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-keyword-flag-output.txt";
    let status_path = "target/rubash-compgen-keyword-flag-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compgen -k wh > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "while\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_helptopic_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-helptopic-output.txt";
    let status_path = "target/rubash-compgen-helptopic-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P help: -A helptopic pr > {output_path}; echo first:$? > {status_path}; \
         compgen -A helptopic no_such_topic >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "help:printf\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_hostname_action_filters_host_variables() {
    let output_path = "target/rubash-compgen-hostname-output.txt";
    let status_path = "target/rubash-compgen-hostname-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "HOSTNAME=rubash-host-alpha; COMPUTERNAME=rubash-host-beta; \
         compgen -A hostname rubash-host- > {output_path}; echo first:$? > {status_path}; \
         compgen -A hostname no-such-host >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash-host-alpha\nrubash-host-beta\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_hostname_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-hostname-action-output.txt";
    let status_path = "target/rubash-compgen-hostname-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "HOSTNAME=rubash-host-action-alpha; COMPUTERNAME=rubash-host-action-beta; \
         compgen -P host: -S :end -A hostname -X '*beta' rubash-host-action- > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "host:rubash-host-action-alpha:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_function_action_filters_function_names() {
    let output_path = "target/rubash-compgen-function-output.txt";
    let status_path = "target/rubash-compgen-function-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "rubash_cgf_alpha() {{ :; }}; rubash_cgf_beta() {{ :; }}; \
         compgen -A function rubash_cgf_ > {output_path}; echo first:$? > {status_path}; \
         compgen -A function rubash_cgf_z >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash_cgf_alpha\nrubash_cgf_beta\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_function_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-function-action-output.txt";
    let status_path = "target/rubash-compgen-function-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "rubash_cgfa_alpha() {{ :; }}; rubash_cgfa_beta() {{ :; }}; \
         compgen -P fn: -S :end -A function -X '*beta' rubash_cgfa_ > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "fn:rubash_cgfa_alpha:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_group_action_filters_group_variables() {
    let output_path = "target/rubash-compgen-group-output.txt";
    let status_path = "target/rubash-compgen-group-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "GROUP=rubash-group-alpha; GROUPNAME=rubash-group-beta; USERDOMAIN=rubash-group-gamma; \
         compgen -A group rubash-group- > {output_path}; echo first:$? > {status_path}; \
         compgen -A group no-such-group >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash-group-alpha\nrubash-group-beta\nrubash-group-gamma\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_group_flag_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-group-flag-output.txt";
    let status_path = "target/rubash-compgen-group-flag-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "GROUP=rubash-group-flag-alpha; GROUPNAME=rubash-group-flag-beta; \
         compgen -P group: -S :end -g -X '*beta' rubash-group-flag- > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "group:rubash-group-flag-alpha:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_service_action_filters_service_variables() {
    let output_path = "target/rubash-compgen-service-output.txt";
    let status_path = "target/rubash-compgen-service-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "SERVICE=rubash-service-alpha; SERVICENAME=rubash-service-beta; \
         compgen -A service rubash-service- > {output_path}; echo first:$? > {status_path}; \
         compgen -A service no-such-service >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash-service-alpha\nrubash-service-beta\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_service_flag_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-service-flag-output.txt";
    let status_path = "target/rubash-compgen-service-flag-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "SERVICE=rubash-service-flag-alpha; SERVICENAME=rubash-service-flag-beta; \
         compgen -P svc: -S :end -s -X '*beta' rubash-service-flag- > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "svc:rubash-service-flag-alpha:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_job_flag_filters_background_jobs() {
    let output_path = "target/rubash-compgen-job-output.txt";
    let status_path = "target/rubash-compgen-job-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "true & first=$!; false & second=$!; \
         compgen -j t > {output_path}; echo first:$? > {status_path}; \
         compgen -j z >> {output_path}; echo second:$? >> {status_path}; \
         disown \"$first\"; disown \"$second\""
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "true\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_job_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-job-action-output.txt";
    let status_path = "target/rubash-compgen-job-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "true & first=$!; false & second=$!; \
         compgen -P job: -S :end -A job -X '*false' > {output_path}; echo $? > {status_path}; \
         disown \"$first\"; disown \"$second\""
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "job:true:end\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_running_action_filters_background_jobs() {
    let output_path = "target/rubash-compgen-running-output.txt";
    let status_path = "target/rubash-compgen-running-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "true & first=$!; false & second=$!; \
         compgen -A running t > {output_path}; echo first:$? > {status_path}; \
         compgen -A running z >> {output_path}; echo second:$? >> {status_path}; \
         disown \"$first\"; disown \"$second\""
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "true\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_running_action_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-running-action-output.txt";
    let status_path = "target/rubash-compgen-running-action-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "true & first=$!; false & second=$!; \
         compgen -P run: -S :end -A running -X '*false' > {output_path}; echo $? > {status_path}; \
         disown \"$first\"; disown \"$second\""
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "run:true:end\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_stopped_action_succeeds_without_stopped_jobs() {
    let output_path = "target/rubash-compgen-stopped-output.txt";
    let status_path = "target/rubash-compgen-stopped-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "true & pid=$!; \
         compgen -A stopped > {output_path}; echo $? > {status_path}; \
         disown \"$pid\""
    );
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K038, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_user_action_filters_user_variables() {
    let output_path = "target/rubash-compgen-user-output.txt";
    let status_path = "target/rubash-compgen-user-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "USER=rubash-user-alpha; LOGNAME=rubash-user-beta; USERNAME=rubash-user-gamma; \
         compgen -A user rubash-user- > {output_path}; echo first:$? > {status_path}; \
         compgen -A user no-such-user >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash-user-alpha\nrubash-user-beta\nrubash-user-gamma\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_user_flag_uses_prefix_suffix_and_filter() {
    let output_path = "target/rubash-compgen-user-flag-output.txt";
    let status_path = "target/rubash-compgen-user-flag-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "USER=rubash-user-flag-alpha; LOGNAME=rubash-user-flag-beta; \
         compgen -P user: -S :end -u -X '*beta' rubash-user-flag- > {output_path}; \
         echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "user:rubash-user-flag-alpha:end\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_signal_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-signal-output.txt";
    let status_path = "target/rubash-compgen-signal-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -A signal SIGT > {output_path}; echo first:$? > {status_path}; \
         compgen -P sig: -A signal SIGTERM >> {output_path}; echo second:$? >> {status_path}; \
         compgen -A signal NO_SUCH >> {output_path}; echo third:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "SIGTRAP\nSIGTERM\nSIGTSTP\nSIGTTIN\nSIGTTOU\nsig:SIGTERM\n"
    );
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:0\nthird:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_shopt_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-shopt-output.txt";
    let status_path = "target/rubash-compgen-shopt-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("compgen -A shopt null > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "nullglob\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_compgen_setopt_action_filters_prefix_matches() {
    let output_path = "target/rubash-compgen-setopt-output.txt";
    let status_path = "target/rubash-compgen-setopt-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "compgen -P opt: -A setopt pipe > {output_path}; echo first:$? > {status_path}; \
         compgen -A setopt no_such >> {output_path}; echo second:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "opt:pipefail\n");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "first:0\nsecond:1\n"
    );
    let _ = fs::remove_file(output_path);
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
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K039, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    // GNU 5.3.0 `type` reprints function bodies in the multi-line form
    // (probe wt37-374 K039, byte-identical at the CLI).
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains(
        "f is a function\nf () \n{ \n    for x in a;\n    do\n        echo $x;\n    done\n}\n"
    ));
    assert!(output.contains("s is a function\ns () \n{ \n    select y in b;\n    do\n        echo $y;\n        break;\n    done\n}\n"));
    assert!(output.contains("c is a function\nc () \n{ \n    case $1 in \n        a)\n            echo alpha\n        ;;\n        *)\n            echo other\n        ;;\n    esac\n}\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_prints_nested_function_definition_body() {
    let output_path = "target/rubash-type-nested-function-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("outer() {{ inner() {{ echo nested; }}; inner; }}; type outer > {output_path}");
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K040, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    // GNU 5.3.0 `type` reprints the nested definition verbatim in the
    // multi-line form (probe wt37-374 K040, byte-identical at the CLI).
    let output = fs::read_to_string(output_path).unwrap();
    assert_eq!(
        output,
        "outer is a function\nouter () \n{ \n    function inner () \n    { \n        echo nested\n    };\n    inner\n}\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_prints_dynamic_fd_variables_with_bash_separators() {
    let output_path = "target/rubash-type-dynamic-fd-variable-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ exec {{fd0}}<&0; exec {{fd1}}>&1; exec {{stdin}}<&$fd0-; exec {{stdout}}>&$fd1-; }}; type f > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "f is a function\nf () \n{ \n    exec {fd0}<&0;\n    exec {fd1}>&1;\n    exec {stdin}<&$fd0-;\n    exec {stdout}>&$fd1-\n}\n"
    );
    let _ = fs::remove_file(output_path);
}
#[test]
fn test_type_prints_canonical_fd_duplication_and_combined_redirects() {
    let output_path = "target/rubash-type-fd-render-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ echo input<&0; echo output>&1; echo named<&$fd; echo combined &>out; echo append &>>out; }}; type f > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "f is a function\nf () \n{ \n    echo input 0<&0;\n    echo output 1>&1;\n    echo named <&$fd;\n    echo combined &> out;\n    echo append &>> out\n}\n"
    );
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
