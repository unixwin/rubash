//! Rubash - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use rubash::executor::{ExecuteError, Executor};
use rubash::lexer::{tokenize, TokenKind};
use rubash::parser::parse;
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;

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
            "--posix" => {
                executor.set_env("__RUBASH_POSIX_MODE", "1");
                index += 1;
            }
            "--login" | "--noprofile" | "--norc" | "-l" => {
                index += 1;
            }
            "-O" | "+O" => {
                if let Some(option) = args.get(index + 1) {
                    if !executor.set_shopt_option(option, args[index] == "-O") {
                        eprintln!("rubash: {option}: invalid shell option name");
                        return 2;
                    }
                    index += 2;
                } else {
                    eprintln!("rubash: {}: option requires an argument", args[index]);
                    return 2;
                }
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
            "-s" => {
                executor.set_positional_params(args[index + 1..].to_vec());
                return run_stdin_script(executor);
            }
            "--" => {
                index += 1;
            }
            "--help" | "-h" => {
                print_usage();
                return 0;
            }
            option if apply_cli_shell_flags(executor, option) => {
                index += 1;
            }
            script => return run_script_file(executor, script, &args[index + 1..]),
        }
    }

    0
}

fn apply_cli_shell_flags(executor: &mut Executor, option: &str) -> bool {
    let (enabled, flags) = if let Some(flags) = option.strip_prefix('-') {
        (true, flags)
    } else if let Some(flags) = option.strip_prefix('+') {
        (false, flags)
    } else {
        return false;
    };
    if flags.is_empty() || flags.contains('c') || flags.contains('o') || flags.contains('s') {
        return false;
    }
    for flag in flags.chars() {
        let Some(name) = cli_shell_flag_name(flag) else {
            return false;
        };
        executor.set_shell_option(name, enabled);
    }
    true
}

fn cli_shell_flag_name(flag: char) -> Option<&'static str> {
    match flag {
        'e' => Some("errexit"),
        'u' => Some("nounset"),
        'x' => Some("xtrace"),
        'C' => Some("noclobber"),
        'f' => Some("noglob"),
        'h' => Some("hashall"),
        'B' => Some("braceexpand"),
        _ => None,
    }
}

fn run_command_string(executor: &mut Executor, command: &str) -> i32 {
    executor.inherit_process_stdin();
    let status = run_source(executor, command, false);
    finish_shell(executor, status, false)
}

fn run_script_file(executor: &mut Executor, script: &str, args: &[String]) -> i32 {
    let path = script_arg_to_path(script);
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!("rubash: {}: {}", script, e);
            return 1;
        }
    };

    executor.set_env("__RUBASH_SCRIPT_NAME", script);
    executor.inherit_process_stdin();
    executor.set_positional_params(args.to_vec());
    let status = run_source(executor, &contents, false);
    finish_shell(executor, status, false)
}

fn script_arg_to_path(script: &str) -> PathBuf {
    if !cfg!(windows) {
        return PathBuf::from(script);
    }

    let normalized = script.replace('\\', "/");
    if normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b'/'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        let drive = normalized.as_bytes()[1] as char;
        return PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &normalized[3..]).replace('/', "\\"),
        );
    }

    if normalized == "/tmp" {
        if let Ok(tmpdir) = env::var("TMPDIR") {
            return PathBuf::from(tmpdir);
        }
    } else if let Some(rest) = normalized.strip_prefix("/tmp/") {
        if let Ok(tmpdir) = env::var("TMPDIR") {
            return PathBuf::from(tmpdir).join(rest);
        }
    }

    PathBuf::from(script)
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
    // input. Keep ordinary input line-oriented, but gather obvious compound
    // commands until their closing reserved word arrives.
    executor.inherit_process_stdin();
    let mut input = String::new();
    let mut pending = String::new();

    loop {
        input.clear();
        match read_unbuffered_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        pending.push_str(&input);
        if stdin_source_needs_more(&pending) {
            continue;
        }

        run_line(executor, &pending, false);
        pending.clear();
    }

    if !pending.trim().is_empty() {
        run_line(executor, &pending, false);
    }

    let status = executor.last_exit_code();
    finish_shell(executor, status, false)
}

fn stdin_source_needs_more(source: &str) -> bool {
    if stdin_source_is_function_signature(source) {
        return true;
    }
    if stdin_source_has_unclosed_function_body(source) {
        return true;
    }

    let tokens = tokenize(source);
    let mut stack = Vec::new();
    for token in tokens {
        if token.kind != TokenKind::Keyword {
            continue;
        }
        match token.value.as_str() {
            "case" => stack.push("esac"),
            "if" => stack.push("fi"),
            "for" | "select" | "while" | "until" => stack.push("done"),
            "esac" | "fi" | "done" if stack.last() == Some(&token.value.as_str()) => {
                stack.pop();
            }
            _ => {}
        }
    }
    !stack.is_empty()
}

fn stdin_source_is_function_signature(source: &str) -> bool {
    let trimmed = source.trim();
    if let Some(name) = trimmed.strip_suffix("()") {
        return is_stdin_function_name(name.trim());
    }

    trimmed
        .strip_prefix("function ")
        .map(str::trim)
        .is_some_and(is_stdin_function_name)
}

fn stdin_source_has_unclosed_function_body(source: &str) -> bool {
    stdin_source_has_unclosed_function_delimited_body(source, '{')
        || stdin_source_has_unclosed_function_delimited_body(source, '(')
}

fn stdin_source_has_unclosed_function_delimited_body(source: &str, delimiter: char) -> bool {
    let Some(open_delimiter) = first_unquoted_function_body_delimiter(source, delimiter) else {
        return false;
    };
    if unquoted_delimiter_depth(&source[open_delimiter..], delimiter) == 0 {
        return false;
    }

    let signature = source[..open_delimiter].trim_end();
    if let Some(name) = signature.strip_suffix("()") {
        return is_stdin_function_name(name.trim_end());
    }

    signature
        .strip_prefix("function ")
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(is_stdin_function_name)
}

fn is_stdin_function_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn first_unquoted_char(source: &str, target: char) -> Option<usize> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            continue;
        }
        if !single && !double && ch == target {
            return Some(index);
        }
    }
    None
}

fn first_unquoted_function_body_delimiter(source: &str, target: char) -> Option<usize> {
    let mut search_from = 0usize;
    while let Some(relative_index) = first_unquoted_char(&source[search_from..], target) {
        let index = search_from + relative_index;
        if target == '('
            && source[index + target.len_utf8()..]
                .trim_start()
                .starts_with(')')
        {
            search_from = index + target.len_utf8();
            continue;
        }
        return Some(index);
    }
    None
}

fn unquoted_delimiter_depth(source: &str, open: char) -> usize {
    let close = match open {
        '{' => '}',
        '(' => ')',
        _ => return 0,
    };
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            continue;
        }
        if single || double {
            continue;
        }
        match ch {
            ch if ch == open => depth += 1,
            ch if ch == close => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth
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
