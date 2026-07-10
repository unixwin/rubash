use super::super::*;
use std::fs;

#[test]
fn test_kill_translates_int_and_term_signals() {
    let output_path = "target/rubash-kill-common-signals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "kill -l 2 > {output_path}; kill -l 130 >> {output_path}; \
         kill -l TERM >> {output_path}; kill -l 15 >> {output_path}; \
         kill -l 143 >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "INT\nINT\n15\nTERM\nTERM\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_translates_quit_and_kill_signals() {
    let output_path = "target/rubash-kill-quit-kill-signals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "kill -l 3 > {output_path}; kill -l 131 >> {output_path}; \
         kill -l QUIT >> {output_path}; kill -l 9 >> {output_path}; \
         kill -l 137 >> {output_path}; kill -l KILL >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "QUIT\nQUIT\n3\nKILL\nKILL\n9\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_translates_more_common_signals() {
    let output_path = "target/rubash-kill-more-common-signals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "kill -l 6 > {output_path}; kill -l 134 >> {output_path}; \
         kill -l SIGABRT >> {output_path}; kill -l PIPE >> {output_path}; \
         kill -l 141 >> {output_path}; kill -l ALRM >> {output_path}; \
         kill -l 142 >> {output_path}; kill -l SIGSEGV >> {output_path}; \
         kill -l 139 >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ABRT\nABRT\n6\n13\nPIPE\n14\nALRM\n11\nSEGV\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_translates_realtime_signals() {
    let output_path = "target/rubash-kill-realtime-signals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "kill -l 32 > {output_path}; kill -l 160 >> {output_path}; \
         kill -l SIGRTMIN >> {output_path}; kill -l RTMIN+1 >> {output_path}; \
         kill -l SIGRTMIN+1 >> {output_path}; kill -l 49 >> {output_path}; \
         kill -l RTMAX-1 >> {output_path}; kill -l SIGRTMAX >> {output_path}; \
         kill -l 64 >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "RTMIN\nRTMIN\n32\n33\n33\nRTMAX-15\n63\n64\nRTMAX\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_lists_common_signals() {
    let output_path = "target/rubash-kill-list-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("kill -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("1) SIGHUP"));
    assert!(output.contains("3) SIGQUIT"));
    assert!(output.contains("9) SIGKILL"));
    assert!(output.contains("15) SIGTERM"));
    assert!(output.contains("31) SIGUSR2"));
    assert!(output.contains("32) SIGRTMIN"));
    assert!(output.contains("64) SIGRTMAX"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_redirects_stderr() {
    let error_path = "target/rubash-kill-stderr-output.txt";
    let status_path = "target/rubash-kill-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("kill -l NO_SUCH_SIGNAL 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("kill: NO_SUCH_SIGNAL: invalid signal specification"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_kill_appends_stderr() {
    let error_path = "target/rubash-kill-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("kill -l NO_SUCH_SIGNAL 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("kill: NO_SUCH_SIGNAL: invalid signal specification"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_ulimit_redirects_output() {
    let output_path = "target/rubash-ulimit-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("ulimit -n > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1024\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_ulimit_appends_output() {
    let output_path = "target/rubash-ulimit-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("ulimit -n >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "before\n1024\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_ulimit_all_lists_resource_limits() {
    let output_path = "target/rubash-ulimit-all-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("ulimit -n 2048; ulimit -a > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("core file size"));
    assert!(output.contains("(blocks, -f) unlimited"));
    assert!(output.contains("open files"));
    assert!(output.contains("(-n) 2048"));
    assert!(output.contains("virtual memory"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_ulimit_redirects_stderr() {
    let error_path = "target/rubash-ulimit-stderr-output.txt";
    let status_path = "target/rubash-ulimit-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("ulimit -g 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("ulimit: -g: invalid option"));
    assert!(error.contains("ulimit: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}
