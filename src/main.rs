//! Rubash - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use rubash::executor::{ExecuteError, Executor};
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut executor = Executor::new();

    if args.len() > 1 {
        let code = run_args(&mut executor, &args[1..]);
        std::process::exit(code);
    }

    run_repl(&mut executor);
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
    run_source(executor, command, false)
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
    run_source(executor, &contents, false)
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
