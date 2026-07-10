use super::super::*;
use std::fs;

#[test]
fn test_bash_command_expands_to_current_command() {
    let output_path = "target/rubash-bash-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo $BASH_COMMAND > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("echo $BASH_COMMAND > {output_path}\n")
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_command_assignment_does_not_override_dynamic_value() {
    let output_path = "target/rubash-bash-command-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("BASH_COMMAND=ignored; echo $BASH_COMMAND > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("echo $BASH_COMMAND > {output_path}\n")
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_version_and_versinfo_are_initialized() {
    let output_path = "target/rubash-bash-version-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf '%s\\n%s\\n%s\\n%s\\n%s:%s:%s\\n' \"$BASH\" \"$BASH_VERSION\" \"${{BASH_VERSINFO[@]}}\" \"$HOSTNAME\" \"$HOSTTYPE\" \"$OSTYPE\" \"$MACHTYPE\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines: Vec<&str> = output.lines().collect();
    assert!(!lines[0].is_empty());
    assert!(lines[1].starts_with(env!("CARGO_PKG_VERSION")));
    assert!(lines[1].ends_with("(1)-release"));
    let version_words = env!("CARGO_PKG_VERSION").replace('.', " ");
    assert!(lines[2].starts_with(&format!("{version_words} 1 release ")));
    assert_eq!(lines[2].split_whitespace().count(), 6);
    assert!(!lines[3].is_empty());
    assert_eq!(lines[4].split(':').count(), 3);
    assert!(!lines[4].contains("::"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_versinfo_assignment_reports_readonly() {
    let output_path = "target/rubash-bash-versinfo-readonly-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("BASH_VERSINFO[0]=9; echo $? ${{BASH_VERSINFO[0]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_call_stack_arrays_are_initialized() {
    let output_path = "target/rubash-bash-call-stack-arrays-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("declare -p BASH_ARGC BASH_ARGV BASH_LINENO BASH_SOURCE > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./call-stack.tests");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("declare -a BASH_ARGC=()\n"));
    assert!(output.contains("declare -a BASH_ARGV=()\n"));
    assert!(output.contains("declare -a BASH_LINENO=([0]=\"0\")\n"));
    assert!(output.contains("declare -a BASH_SOURCE=([0]=\"./call-stack.tests\")\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_call_stack_arrays_ignore_assignment() {
    let output_path = "target/rubash-bash-call-stack-noassign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "BASH_ARGC=(xxx); echo argc:$?:${{#BASH_ARGC[@]}} > {output_path}; \
         declare BASH_ARGV[1]=foo; echo argv:$?:${{#BASH_ARGV[@]}} >> {output_path}; \
         BASH_SOURCE[0]=other; echo source:$?:${{BASH_SOURCE[0]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./call-stack.tests");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "argc:0:0\nargv:0:0\nsource:0:./call-stack.tests\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_call_stack_arrays_cannot_be_unset() {
    let output_path = "target/rubash-bash-call-stack-unset-output.txt";
    let error_path = "target/rubash-bash-call-stack-unset-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "unset -v BASH_LINENO BASH_SOURCE 2> {error_path}; \
         echo status:$? > {output_path}; \
         echo values:${{BASH_LINENO[0]}}:${{BASH_SOURCE[0]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./call-stack-unset.case");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "status:1\nvalues:0:./call-stack-unset.case\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("unset: BASH_LINENO: cannot unset"));
    assert!(error.contains("unset: BASH_SOURCE: cannot unset"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_uid_and_euid_are_readonly_nonzero_ids() {
    let output_path = "target/rubash-uid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $UID:$EUID > {output_path}; if (( UID == 0 )); then echo root >> {output_path}; else echo user >> {output_path}; fi; test -R UID; echo readonly:$? >> {output_path}; UID=0; echo assign:$?:$UID >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines: Vec<&str> = output.lines().collect();
    let (uid, euid) = lines[0].split_once(':').unwrap();
    assert!(!uid.is_empty());
    assert!(!euid.is_empty());
    assert_eq!(lines[1], if uid == "0" { "root" } else { "user" });
    assert_eq!(lines[2], "readonly:0");
    assert_eq!(lines[3], format!("assign:1:{uid}"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_ppid_is_readonly_numeric_id() {
    let output_path = "target/rubash-ppid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $PPID > {output_path}; \
         test -R PPID; echo readonly:$? >> {output_path}; \
         PPID=1; echo assign:$?:$PPID >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines: Vec<&str> = output.lines().collect();
    assert!(lines[0].chars().all(|ch| ch.is_ascii_digit()));
    assert_eq!(lines[1], "readonly:0");
    assert_eq!(lines[2], format!("assign:1:{}", lines[0]));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_funcname_expands_inside_function() {
    let output_path = "target/rubash-funcname-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("show_name() {{ printf '%s\\n' \"$FUNCNAME\" > {output_path}; }}; show_name");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "show_name\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_funcname_tracks_nested_function_stack() {
    let output_path = "target/rubash-funcname-stack-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "outer() {{ inner; }}; \
         inner() {{ printf '%s|%s|%s|%s|%s\\n' \"$FUNCNAME\" \"${{FUNCNAME[0]}}\" \"${{FUNCNAME[1]}}\" \"${{FUNCNAME[@]}}\" \"${{#FUNCNAME[@]}}\" > {output_path}; }}; \
         outer"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "inner|inner|outer|inner outer|2\n"
    );
    let _ = fs::remove_file(output_path);
}
