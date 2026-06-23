//! Rubash - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use rubash::executor::{ExecuteError, Executor};
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut executor = Executor::new();

    if args.len() > 1 {
        let code = run_args(&mut executor, &args[1..]);
        std::process::exit(code);
    }

    if io::stdin().is_terminal() {
        run_repl(&mut executor);
    } else {
        std::process::exit(run_stdin_script(&mut executor));
    }
}

fn print_usage() {
    println!("Usage: rubash [-c command] [script]");
}

fn run_args(executor: &mut Executor, args: &[String]) -> i32 {
    // TODO(shell.c): GNU Bash has a full option parser and shell-name handling.
    // This narrow parser supports the `-c` and `-o posix -c` forms used by
    // upstream alias tests.
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" if args.get(index + 1).map(String::as_str) == Some("posix") => {
                executor.set_env("__RUBASH_POSIX_MODE", "1");
                index += 2;
            }
            "-c" => {
                if let Some(command) = args.get(index + 1) {
                    executor.set_env("BASH_EXECUTION_STRING", command);
                    if let Some(command_name) = args.get(index + 2) {
                        executor.set_env("__RUBASH_SCRIPT_NAME", command_name);
                        executor.set_positional_params(args[index + 3..].to_vec());
                    }
                    return run_command_string(executor, command);
                }
                eprintln!("rubash: -c: option requires an argument");
                return 2;
            }
            "--help" | "-h" => {
                print_usage();
                return 0;
            }
            script => return run_script_file(executor, script),
        }
    }

    0
}

fn run_command_string(executor: &mut Executor, command: &str) -> i32 {
    let status = run_source(executor, command, false);
    finish_shell(executor, status, false)
}

fn run_script_file(executor: &mut Executor, script: &str) -> i32 {
    let path = Path::new(script);
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!("rubash: {}: {}", script, e);
            return 1;
        }
    };

    executor.set_env("__RUBASH_SCRIPT_NAME", script);
    let status = run_source(executor, &contents, false);
    finish_shell(executor, status, false)
}

fn run_repl(executor: &mut Executor) {
    println!("Rubash - A Rust implementation of GNU Bash");
    println!("Type 'exit' to quit.\n");

    let stdin = io::stdin();
    let mut input = String::new();

    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        input.clear();
        match stdin.lock().read_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let input = input.trim();
        if input == "exit" || input == "quit" {
            println!("Goodbye!");
            break;
        }

        run_line(executor, input, true);
    }
}

fn run_stdin_script(executor: &mut Executor) -> i32 {
    // TODO(shell.c/input.c): Bash reads commands from redirected stdin without
    // prompting, while commands launched from that stream inherit the same
    // input. Keep this line-oriented until parse.y owns incremental input.
    let mut input = String::new();

    loop {
        input.clear();
        match read_unbuffered_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        run_line(executor, &input, false);
    }

    let status = executor.last_exit_code();
    finish_shell(executor, status, false)
}

fn read_unbuffered_line(output: &mut String) -> io::Result<usize> {
    // TODO(input.c): This intentionally avoids BufRead prefetching so a child
    // shell script can inherit unread bytes from the same redirected stdin.
    let mut stdin = io::stdin().lock();
    let mut bytes = [0_u8; 1];
    let mut read = 0;
    loop {
        match stdin.read(&mut bytes)? {
            0 => break,
            count => {
                read += count;
                output.push(bytes[0] as char);
                if bytes[0] == b'\n' {
                    break;
                }
            }
        }
    }
    Ok(read)
}

fn run_line(executor: &mut Executor, input: &str, interactive: bool) -> i32 {
    let input = input.trim();
    if input.is_empty() {
        return executor.last_exit_code();
    }

    run_source(executor, input, interactive)
}

fn run_source(executor: &mut Executor, input: &str, interactive: bool) -> i32 {
    // TODO(shell.c/eval.c/parse.y): GNU Bash parses complete command streams,
    // including pending here-documents, rather than executing script files one
    // physical line at a time. This keeps batch input whole; interactive mode
    // still feeds one line at a time from the REPL.
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    match executor.execute_ast(&ast) {
        Ok(()) => executor.last_exit_code(),
        Err(ExecuteError::ExitCode(code)) => code,
        Err(e) => {
            if interactive {
                eprintln!("Error: {}", e);
            } else {
                eprintln!("{}", e);
            }
            1
        }
    }
}

fn finish_shell(executor: &mut Executor, status: i32, interactive: bool) -> i32 {
    match executor.run_exit_trap_with_status(status) {
        Ok(code) => code,
        Err(ExecuteError::ExitCode(code)) => code,
        Err(e) => {
            if interactive {
                eprintln!("Error: {}", e);
            } else {
                eprintln!("{}", e);
            }
            1
        }
    }
}
