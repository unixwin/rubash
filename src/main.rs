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
        let code = match args[1].as_str() {
            "-c" => {
                if let Some(command) = args.get(2) {
                    run_command_string(&mut executor, command)
                } else {
                    eprintln!("rubash: -c: option requires an argument");
                    2
                }
            }
            "--help" | "-h" => {
                print_usage();
                0
            }
            script => run_script_file(&mut executor, script),
        };
        std::process::exit(code);
    }

    run_repl(&mut executor);
}

fn print_usage() {
    println!("Usage: rubash [-c command] [script]");
}

fn run_command_string(executor: &mut Executor, command: &str) -> i32 {
    let mut last_status = 0;
    for line in command.lines() {
        last_status = run_line(executor, line, false);
        if matches!(line.trim(), "exit" | "quit") {
            break;
        }
    }
    last_status
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

    let mut last_status = 0;
    for line in contents.lines() {
        last_status = run_line(executor, line, false);
        if matches!(line.trim(), "exit" | "quit") {
            break;
        }
    }
    last_status
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
