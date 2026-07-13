use super::super::*;
use std::fs;

#[test]
fn test_pipeline_feeds_external_stage_stdin() {
    let output_path = target_test_path("rubash-pipeline-external-output.txt");
    #[cfg(windows)]
    let script_path = target_test_path("rubash-pipeline-filter.cmd");
    #[cfg(not(windows))]
    let script_path = target_test_path("rubash-pipeline-filter.sh");
    let shell_output_path = shell_test_path(&output_path);
    let shell_script_path = shell_test_path(&script_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
    #[cfg(windows)]
    fs::write(
        &script_path,
        "@echo off\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -Command \"$input | Where-Object { $_ -eq 'b' } | ForEach-Object { 'external:' + $_ }\"\r\n",
    )
    .unwrap();
    #[cfg(not(windows))]
    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  if [ \"$line\" = b ]; then\n    printf 'external:%s\\n' \"$line\"\n  fi\ndone\n",
    )
    .unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!("printf 'a\\nb\\n' | {shell_script_path} > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].pipe.is_some());
    assert_eq!(pipeline.stages[1].words, [shell_script_path.as_str()]);
    assert!(pipeline.stages[1].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path)
            .unwrap()
            .replace("\r\n", "\n"),
        "external:b\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(script_path);
}

#[test]
fn test_external_command_appends_stdout() {
    let bin_dir = "target/rubash-external-append-bin";
    let script_path = format!("{bin_dir}/emit");
    let output_path = "target/rubash-external-append-output.txt";
    let _ = fs::remove_dir_all(bin_dir);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-append\n").unwrap();
    let input = format!("emit > {output_path}; emit >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_path = std::env::var("PATH").ok();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);
    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "external-append\nexternal-append\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn test_external_command_appends_stderr() {
    let bin_dir = "target/rubash-external-stderr-append-bin";
    let script_path = format!("{bin_dir}/emiterr");
    let output_path = "target/rubash-external-stderr-append-output.txt";
    let _ = fs::remove_dir_all(bin_dir);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-error >&2\n").unwrap();
    let input = format!("emiterr 2> {output_path}; emiterr 2>> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_path = std::env::var("PATH").ok();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);
    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "external-error\nexternal-error\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn test_redirects_without_spaces_around_operator() {
    let output_path = "target/rubash-nospace-redirect-output.txt";
    let error_path = "target/rubash-nospace-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "echo alpha>{output_path}; echo beta>>{output_path}; no_such_nospace_cmd 2>{error_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_nospace_cmd: command not found"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_redirection_only_command_touches_output_files() {
    let output_path = "target/rubash-redirection-only-output.txt";
    let append_path = "target/rubash-redirection-only-append.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(append_path);
    fs::write(output_path, "seed\n").unwrap();
    let input = format!("> {output_path}; >> {append_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(append_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(append_path);
}

#[test]
fn test_mapfile_t_reads_here_string_into_array() {
    let output_path = "target/rubash-mapfile-t-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "mapfile -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_reads_redirected_default_parameter_path() {
    let input_path = target_test_path("rubash-mapfile-default-redirect-input.txt");
    let output_path = target_test_path("rubash-mapfile-default-redirect-output.txt");
    let shell_input_path = shell_test_path(&input_path);
    let shell_output_path = shell_test_path(&output_path);
    fs::write(&input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "mapfile -t arr < \"${{1:-{shell_input_path}}}\"; echo ${{#arr[@]}} ${{arr[@]}} > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_without_input_sets_empty_array_at_eof() {
    let output_path = "target/rubash-mapfile-eof-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr MAPFILE; mapfile arr; echo named:$?:${{#arr[@]}} > {output_path}; \
         mapfile; echo default:$?:${{#MAPFILE[@]}} >> {output_path}; declare -p arr MAPFILE >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "named:0:0\ndefault:0:0\ndeclare -a arr=()\ndeclare -a MAPFILE=()\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_option_errors_match_bash_statuses() {
    let output_path = "target/rubash-mapfile-option-status-output.txt";
    let error_path = "target/rubash-mapfile-option-error-output.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "mapfile -Z arr 2> {error_path}; echo invalid:$? > {output_path}; \
         mapfile -n 2>> {error_path}; echo missing:$? >> {output_path}; \
         readarray -O nope arr 2>> {error_path}; echo origin:$? >> {output_path}; \
         mapfile -c 0 -C cb arr 2>> {error_path}; echo quantum:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "invalid:2\nmissing:2\norigin:1\nquantum:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("mapfile: -Z: invalid option"));
    assert!(error.contains("mapfile: -n: option requires an argument"));
    assert!(error.contains("readarray: nope: invalid array origin"));
    assert!(error.contains("mapfile: 0: invalid callback quantum"));
    assert!(error.contains("mapfile: usage: mapfile"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}
