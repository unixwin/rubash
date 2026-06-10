//! bashrs - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use bashrs::lexer::tokenize;
use bashrs::parser::parse;
use bashrs::executor::Executor;

fn main() {
    println!("bashrs - A Rust implementation of GNU Bash");
    println!("Type 'exit' to quit.\n");

    let mut executor = Executor::new();

    loop {
        print!("$ ");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();

        let mut input = String::new();
        match std::io::stdin().read_line(&mut input) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input == "exit" || input == "quit" {
            println!("Goodbye!");
            break;
        }

        // Tokenize
        let tokens = tokenize(input);

        // Parse
        let ast = parse(&tokens);

        // Execute
        if let Err(e) = executor.execute_ast(&ast) {
            eprintln!("Error: {}", e);
        }
    }
}