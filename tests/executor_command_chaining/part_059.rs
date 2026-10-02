use super::super::*;
use std::fs;

#[test]
fn test_set_posix_updates_visible_option_state() {
    let output_path = "target/rubash-set-posix-option-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o posix; type break > {output_path}; type export >> {output_path}; \
         command -V export >> {output_path}; set -o >> {output_path}; \
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
    assert_eq!(
        output
            .matches("export is a special shell builtin\n")
            .count(),
        2
    );
    assert!(output.contains("break is a shell builtin\n"));
    let posix_lines: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("posix"))
        .collect();
    assert_eq!(posix_lines, ["posix          \ton", "posix          \toff"]);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_noninteractive_shell_option_defaults_match_bash() {
    let output_path = "target/rubash-set-noninteractive-defaults-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -o > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output
        .lines()
        .any(|line| line.starts_with("emacs") && line.ends_with("\toff")));
    assert!(output
        .lines()
        .any(|line| line.starts_with("history") && line.ends_with("\toff")));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_igncr_shell_option_is_listed_and_toggleable() {
    let output_path = "target/rubash-set-igncr-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("set -o igncr; set -o > {output_path}; set +o igncr; set -o >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    let lines: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("igncr"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with("\ton"));
    assert!(lines[1].ends_with("\toff"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shopt_without_options_lists_readable_states() {
    let output_path = "target/rubash-shopt-default-list-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("shopt > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output
        .lines()
        .any(|line| line.starts_with("checkwinsize") && line.ends_with("\ton")));
    assert!(!output.contains("shopt -s "));
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
fn test_setopt_tracks_compat_options_and_maps_shell_state() {
    let output_path = "target/rubash-setopt-zsh-options-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "setopt hist_ignore_dups hist_ignore_space prompt_subst prompt_percent brace_expand null_glob; \
         unsetopt prompt_percent; \
         setopt > {output_path}; \
         echo hist:$WINUXSH_HIST_IGNORE_DUPS:$WINUXSH_HIST_IGNORE_SPACE >> {output_path}; \
         shopt -q nullglob; echo nullglob:$? >> {output_path}; \
         shopt -q promptvars; echo promptvars:$? >> {output_path}; \
         [[ -o braceexpand ]]; echo brace:$? >> {output_path}; \
         setopt no_null_glob; shopt -q nullglob; echo nullglob_off:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("hist_ignore_dups\n"));
    assert!(output.contains("hist_ignore_space\n"));
    assert!(output.contains("prompt_subst\n"));
    assert!(!output.contains("prompt_percent\n"));
    assert!(output.contains("hist:1:1\n"));
    assert!(output.contains("nullglob:0\n"));
    assert!(output.contains("promptvars:0\n"));
    assert!(output.contains("brace:0\n"));
    assert!(output.contains("nullglob_off:1\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_and_builtin_setopt_update_same_state() {
    let output_path = "target/rubash-command-builtin-setopt-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command setopt null_glob; shopt -q nullglob; echo command:$? > {output_path}; \
         builtin unsetopt null_glob; shopt -q nullglob; echo builtin:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "command:0\nbuiltin:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_source_executes_setopt_without_host_preprocessing() {
    let script_path = "target/rubash-setopt-source.rc";
    let output_path = "target/rubash-setopt-source-output.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    fs::write(script_path, "setopt hist_ignore_dups null_glob\n").unwrap();
    let input = format!(
        "source {script_path}; \
         echo hist:$WINUXSH_HIST_IGNORE_DUPS > {output_path}; \
         shopt -q nullglob; echo nullglob:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "hist:1\nnullglob:0\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_setopt_invalid_option_reports_failure() {
    let output_path = "target/rubash-setopt-invalid-output.txt";
    let error_path = "target/rubash-setopt-invalid-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("setopt no_such_option 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("setopt: no such option: no_such_option"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
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
    assert!(output.trim_end().contains('f'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_extglob_pathname_expansion_matches_files() {
    let dir_path = "target/rubash-extglob-pathname";
    let output_path = "target/rubash-extglob-pathname-output.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/keep.txt"), "keep").unwrap();
    fs::write(format!("{dir_path}/note.md"), "note").unwrap();
    fs::write(format!("{dir_path}/skip.tmp"), "skip").unwrap();
    // wt33 (#374): the shopt must execute on its own line before the
    // extglob word parses (GNU parse-execute cadence; the one-line form
    // is rejected by GNU too — probes H16/H17).
    let input = format!("shopt -s extglob\nprintf '%s\\n' {dir_path}/!(*.tmp) > {output_path}");
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "target/rubash-extglob-pathname/keep.txt\ntarget/rubash-extglob-pathname/note.md\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_pathname_expansion_matches_intermediate_segments() {
    let base_path = "target/rubash-glob-segments";
    let one_dir = format!("{base_path}/dir-one");
    let two_dir = format!("{base_path}/dir-two");
    let output_path = "target/rubash-glob-segments-output.txt";
    let _ = fs::remove_dir_all(base_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(&one_dir).unwrap();
    fs::create_dir_all(&two_dir).unwrap();
    fs::write(format!("{one_dir}/file.txt"), "one").unwrap();
    fs::write(format!("{two_dir}/file.txt"), "two").unwrap();
    fs::write(format!("{two_dir}/trace.log"), "trace").unwrap();
    let input = format!(
        "printf '%s\\n' {base_path}/dir-*/*.txt > {output_path}\n\
         shopt -s extglob\nprintf '%s\\n' {base_path}/dir-@(two)/*.log >> {output_path}"
    );
    // wt33 (#374): run through the real CLI — this construct depends on the
    // parse-execute cadence that an in-process execute_ast cannot model
    // (subprocess behavior is byte-identical to WSL GNU 5.3.0; see
    // run_cli_script's doc comment and the wt33-373 G/H probes).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "target/rubash-glob-segments/dir-one/file.txt\n\
         target/rubash-glob-segments/dir-two/file.txt\n\
         target/rubash-glob-segments/dir-two/trace.log\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(base_path);
}

#[test]
fn test_globskipdots_controls_dot_and_dotdot_matches() {
    let dir_path = "target/rubash-globskipdots";
    let output_path = "target/rubash-globskipdots-output.txt";
    let _ = fs::remove_dir_all(dir_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(dir_path).unwrap();
    fs::write(format!("{dir_path}/.hidden"), "hidden").unwrap();
    let input = format!(
        "printf '%s\\n' {dir_path}/.* > {output_path}; \
         shopt -u globskipdots; printf '%s\\n' {dir_path}/.* >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "target/rubash-globskipdots/.hidden\n\
         target/rubash-globskipdots/.\n\
         target/rubash-globskipdots/..\n\
         target/rubash-globskipdots/.hidden\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_failglob_unmatched_command_word_aborts_command_list() {
    let output_path = "target/rubash-failglob-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s failglob; printf '%s\\n' target/rubash-no-such-*.zzz > {output_path}; echo after >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_failglob_takes_precedence_over_nullglob() {
    let output_path = "target/rubash-failglob-nullglob-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s nullglob failglob; printf '%s\\n' target/rubash-no-such-*.zzz > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_failglob_unmatched_for_word_skips_loop_body() {
    let output_path = "target/rubash-failglob-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s failglob; for item in target/rubash-no-such-*.zzz; do echo $item > {output_path}; done; echo after >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_failglob_unmatched_select_word_skips_body() {
    let output_path = "target/rubash-failglob-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s failglob; select item in target/rubash-no-such-*.zzz; do echo $item > {output_path}; break; done <<< 1; echo after >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_noglob_suppresses_failglob_pathname_expansion() {
    let output_path = "target/rubash-noglob-suppresses-failglob-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s failglob; set -f; printf '%s\\n' target/rubash-no-such-*.zzz > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "target/rubash-no-such-*.zzz\n"
    );
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
