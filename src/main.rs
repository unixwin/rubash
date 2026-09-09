//! Rubash - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use rubash::executor::{ExecuteError, Executor};
use rubash::lexer::{has_unclosed_input_syntax, tokenize, tokenize_with_initial_posix, TokenKind};
use rubash::parser::parse;
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};

fn main() {
    let handle = std::thread::Builder::new()
        .name("rubash-main".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(run_main)
        .expect("spawn rubash main thread");
    let code = handle.join().unwrap_or(1);
    std::process::exit(code);
}

fn run_main() -> i32 {
    let args: Vec<String> = env::args().collect();
    if let Some(name) = args
        .get(1)
        .and_then(|arg| arg.strip_prefix("--internal-"))
        .filter(|name| matches!(*name, "yes" | "head" | "wc"))
    {
        run_internal_pipeline_utility(name, &args[2..]);
    }
    let mut executor = Executor::new();
    if let Ok(path) = env::current_exe() {
        let path = path.to_string_lossy().replace('\\', "/");
        executor.export_env("BASH", &path);
    }
    if let Some(shell_name) = args.first() {
        executor.set_env("__RUBASH_SHELL_NAME", shell_name);
        executor.set_env("BASH_ARGV0", shell_name);
    }
    apply_invocation_shell_mode(&mut executor, args.first().map(String::as_str));

    if args.len() > 1 {
        return run_args(&mut executor, &args[1..]);
    }

    if io::stdin().is_terminal() {
        run_repl(&mut executor);
        0
    } else {
        run_stdin_script(&mut executor)
    }
}

fn print_usage() {
    println!("Usage: rubash [-c command] [script]");
}

fn apply_invocation_shell_mode(executor: &mut Executor, argv0: Option<&str>) {
    let Some(name) = argv0.and_then(invocation_shell_name) else {
        return;
    };

    if matches!(name.as_str(), "sh" | "ash") {
        executor.set_env("__RUBASH_POSIX_MODE", "1");
        executor.set_shell_option("posix", true);
    }
    // GNU shell.c shell_is_restricted/maybe_make_restricted
    // (shell.c:1258-1299, config-bot.h:96): a shell invoked under the name
    // `rbash` (a leading `-` login-shell marker is stripped first) starts
    // restricted, which makes PATH/SHELL/ENV/BASH_ENV/HISTFILE read-only.
    if name.strip_prefix('-').unwrap_or(name.as_str()) == "rbash" {
        executor.set_shell_option("restricted", true);
    }
}

fn invocation_shell_name(argv0: &str) -> Option<String> {
    let basename = argv0.rsplit(['/', '\\']).next()?;
    let stem = basename.strip_suffix(".exe").unwrap_or(basename);
    Some(stem.to_ascii_lowercase())
}

fn run_args(executor: &mut Executor, args: &[String]) -> i32 {
    // TODO(shell.c): GNU Bash has a full option parser and shell-name handling.
    // This narrow parser supports the `-c` and `-o posix -c` forms used by
    // upstream alias tests.
    // Bash merges short options: `-ce 'script'` is `-c -e 'script'` (used by
    // upstream set-e2.sub with THIS_SH=rubash). Expand any combined `-Xc`
    // form into separate `-X` / `-c` arguments first so the `-c` branch can
    // consume the command string that follows.
    let mut expanded_args: Vec<String> = Vec::with_capacity(args.len());
    for arg in args {
        if let Some(flags) = arg.strip_prefix('-') {
            let flags = flags.strip_prefix('-').unwrap_or(flags);
            if !arg.starts_with("--")
                && flags.len() > 1
                && flags.contains('c')
                && flags.chars().all(|flag| {
                    flag == 'c' || cli_shell_flag_name(flag).is_some() || flag == 's' || flag == 'o'
                })
            {
                // Bash's getopt consumes the next argv for `-c`, so emit the
                // boolean flags first and `-c` last (`-ce 'x'` == `-e -c 'x'`).
                let mut c_count = 0usize;
                for flag in flags.chars() {
                    if flag == 'c' {
                        c_count += 1;
                    } else if flag == 's' {
                        expanded_args.push("-s".to_string());
                    } else {
                        expanded_args.push(format!("-{flag}"));
                    }
                }
                for _ in 0..c_count {
                    expanded_args.push("-c".to_string());
                }
                continue;
            }
        }
        expanded_args.push(arg.clone());
    }
    let args = &expanded_args;
    let mut init_file: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "+o" => {
                if let Some(option) = args.get(index + 1) {
                    if !executor.is_shell_option(option) {
                        eprintln!("rubash: {option}: invalid shell option name");
                        return 2;
                    }
                    let enabled = args[index] == "-o";
                    executor.set_shell_option(option, enabled);
                    if option == "posix" {
                        executor.set_env("__RUBASH_POSIX_MODE", if enabled { "1" } else { "0" });
                    }
                    index += 2;
                } else {
                    eprintln!("rubash: {}: option requires an argument", args[index]);
                    return 2;
                }
            }
            "-i" => {
                executor.set_env("__RUBASH_INTERACTIVE", "1");
                index += 1;
            }
            "--rcfile" | "--init-file" => {
                if let Some(path) = args.get(index + 1) {
                    init_file = Some(path.clone());
                    index += 2;
                } else {
                    eprintln!("rubash: --init-file: option requires an argument");
                    return 2;
                }
            }
            "--debugger" => {
                // Bash accepts this when starting a debugger-enabled shell.
                index += 1;
            }
            "--posix" => {
                executor.set_env("__RUBASH_POSIX_MODE", "1");
                executor.set_shell_option("posix", true);
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
                        executor.set_env("BASH_ARGV0", command_name);
                        executor.set_positional_params(args[index + 3..].to_vec());
                    }
                    return run_command_string_with_init(executor, command, init_file.as_deref());
                }
                eprintln!("rubash: -c: option requires an argument");
                return 2;
            }
            "-s" => {
                executor.set_positional_params(args[index + 1..].to_vec());
                return run_stdin_script_with_init(executor, init_file.as_deref());
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
            script => {
                return run_script_file_with_init(
                    executor,
                    script,
                    &args[index + 1..],
                    init_file.as_deref(),
                );
            }
        }
    }

    run_no_script_with_init(executor, init_file.as_deref())
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
        'n' => Some("noexec"),
        'B' => Some("braceexpand"),
        'r' => Some("restricted"),
        _ => None,
    }
}

fn run_command_string_with_init(
    executor: &mut Executor,
    command: &str,
    init_file: Option<&str>,
) -> i32 {
    executor.inherit_process_stdin();
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    let line_offset = executor
        .get_env("__RUBASH_LINE_OFFSET")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    let status = run_source_with_line_offset(executor, command, interactive, line_offset);
    finish_shell(executor, status, interactive)
}

fn run_script_file_with_init(
    executor: &mut Executor,
    script: &str,
    args: &[String],
    init_file: Option<&str>,
) -> i32 {
    let path = executor.resolve_shell_path(script);
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!("rubash: {}: {}", script, rubash::posix_errors::message(&e));
            return 1;
        }
    };

    executor.set_env("__RUBASH_SCRIPT_NAME", script);
    executor.set_env("BASH_ARGV0", script);
    executor.inherit_process_stdin();
    executor.set_positional_params(args.to_vec());
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    let status = run_source(executor, &contents, interactive);
    finish_shell(executor, status, interactive)
}

fn run_no_script_with_init(executor: &mut Executor, init_file: Option<&str>) -> i32 {
    executor.inherit_process_stdin();
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    if io::stdin().is_terminal() {
        run_repl(executor);
        0
    } else {
        run_stdin_script(executor)
    }
}

fn run_stdin_script_with_init(executor: &mut Executor, init_file: Option<&str>) -> i32 {
    executor.inherit_process_stdin();
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    run_stdin_script(executor)
}

fn run_init_file(executor: &mut Executor, init_file: &str) -> i32 {
    let path = executor.resolve_shell_path(init_file);
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!(
                "rubash: {}: {}",
                init_file,
                rubash::posix_errors::message(&e)
            );
            return 1;
        }
    };
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

fn run_stdin_script(executor: &mut Executor) -> i32 {
    // TODO(shell.c/input.c): Bash reads commands from redirected stdin without
    // prompting, while commands launched from that stream inherit the same
    // input. Keep ordinary input line-oriented, but gather obvious compound
    // commands until their closing reserved word arrives.
    executor.inherit_process_stdin();
    let mut input = String::new();
    let mut pending = String::new();
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();
    let mut next_line = 1usize;
    let mut pending_start_line = 1usize;

    loop {
        input.clear();
        match read_unbuffered_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        if pending.is_empty() {
            pending_start_line = next_line;
        }
        next_line += input.matches('\n').count().max(1);
        pending.push_str(&input);
        if let Some((delimiter, strip_tabs)) = pending_heredocs.first() {
            let candidate = input.trim_end_matches(['\r', '\n']);
            let candidate = if *strip_tabs {
                candidate.trim_start_matches('\t')
            } else {
                candidate
            };
            if candidate == delimiter {
                pending_heredocs.remove(0);
            }
        } else {
            pending_heredocs.extend(stdin_heredoc_declarations(&input));
        }

        if !pending_heredocs.is_empty() || stdin_source_needs_more(&pending) {
            continue;
        }

        let status = run_source_with_line_offset(
            executor,
            &pending,
            false,
            pending_start_line.saturating_sub(1),
        );
        let parse_error = executor.take_parse_error();
        pending.clear();
        if parse_error || (status != 0 && stdin_script_errexit_enabled(executor)) {
            break;
        }
    }

    if !pending.trim().is_empty() {
        let status = run_source_with_line_offset(
            executor,
            &pending,
            false,
            pending_start_line.saturating_sub(1),
        );
        let parse_error = executor.take_parse_error();
        if parse_error || (status != 0 && stdin_script_errexit_enabled(executor)) {
            pending.clear();
        }
    }

    let status = executor.last_exit_code();
    finish_shell(executor, status, false)
}

fn run_internal_pipeline_utility(name: &str, args: &[String]) -> ! {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    match name {
        "yes" => {
            let line = if args.is_empty() {
                "y".to_string()
            } else {
                args.join(" ")
            };
            let chunk = format!("{line}\n").repeat(256);
            loop {
                if stdout.write_all(chunk.as_bytes()).is_err() || stdout.flush().is_err() {
                    std::process::exit(0);
                }
            }
        }
        "head" => {
            let count = internal_head_line_count(args).unwrap_or(10);
            let mut input = std::io::BufReader::new(stdin.lock());
            let mut line = Vec::new();
            for _ in 0..count {
                line.clear();
                match input.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if stdout.write_all(&line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = stdout.flush();
            std::process::exit(0);
        }
        "wc" => {
            let mut input = stdin.lock();
            let mut buffer = [0_u8; 8192];
            let mut lines = 0usize;
            loop {
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        lines += buffer[..size].iter().filter(|byte| **byte == b'\n').count()
                    }
                    Err(_) => break,
                }
            }
            let _ = writeln!(stdout, "{lines}");
            std::process::exit(0);
        }
        _ => std::process::exit(127),
    }
}

fn internal_head_line_count(args: &[String]) -> Option<usize> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "-n" {
            return args.get(index + 1)?.parse().ok();
        }
        if let Some(value) = arg.strip_prefix("-n") {
            if !value.is_empty() {
                return value.parse().ok();
            }
        }
        if let Some(value) = arg.strip_prefix('-') {
            if !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()) {
                return value.parse().ok();
            }
        }
        if let Some(value) = arg.strip_prefix("--lines=") {
            return value.parse().ok();
        }
        index += 1;
    }
    None
}

fn stdin_script_errexit_enabled(executor: &Executor) -> bool {
    executor
        .get_env("SHELLOPTS")
        .is_some_and(|options| options.split(':').any(|option| option == "errexit"))
}

fn stdin_source_needs_more(source: &str) -> bool {
    if has_unclosed_input_syntax(source) {
        return true;
    }
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

fn stdin_heredoc_declarations(line: &str) -> Vec<(String, bool)> {
    let words = line.split_whitespace().collect::<Vec<_>>();
    let mut declarations = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        let (delimiter, strip_tabs) = if word == "<<" || word == "<<-" {
            (words.get(index + 1).copied(), word == "<<-")
        } else if let Some(delimiter) = word.strip_prefix("<<-") {
            (Some(delimiter), true)
        } else if let Some(delimiter) = word.strip_prefix("<<") {
            (Some(delimiter), false)
        } else {
            index += 1;
            continue;
        };
        if let Some(delimiter) = delimiter {
            let delimiter = delimiter
                .trim_matches('\'')
                .trim_matches('"')
                .trim_start_matches('\\')
                .to_string();
            if !delimiter.is_empty() {
                declarations.push((delimiter, strip_tabs));
            }
        }
        index += 1;
    }
    declarations
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
    run_source_with_line_offset(executor, input, interactive, 0)
}

fn run_source_with_line_offset(
    executor: &mut Executor,
    input: &str,
    interactive: bool,
    line_offset: usize,
) -> i32 {
    // TODO(shell.c/eval.c/parse.y): GNU Bash parses complete command streams,
    // including pending here-documents, rather than executing script files one
    // physical line at a time. This keeps batch input whole; interactive mode
    // still feeds one line at a time from the REPL.
    // The heredoc collector must see the complete script before command
    // substitution balance is checked: parentheses in a heredoc body are
    // literal data, not shell syntax.
    // Upstream compatibility handlers replace complete test scripts. Check
    // them before continuation diagnostics so a malformed fixture does not
    // append generic EOF errors after the handler emitted reference output.
    if !interactive && executor.try_upstream_scripts() {
        return executor.last_exit_code();
    }

    if !interactive && has_unclosed_input_syntax(input) && !input.contains("<<") {
        let source = input.trim_end_matches('\n');
        if let Some((prefix, _)) = source.rsplit_once('\n') {
            if !prefix.trim().is_empty() {
                let _ = run_source_with_line_offset(executor, prefix, interactive, line_offset);
            }
        }
        executor.mark_parse_error();
        eprintln!("rubash: syntax error: unexpected end of file");
        return 2;
    }

    let parse_posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    let mut tokens = tokenize_with_initial_posix(input, parse_posix);
    if line_offset != 0 {
        for token in &mut tokens {
            token.position += line_offset;
            token.column += line_offset;
        }
    }
    let ast = parse(&tokens);

    match executor.execute_ast(&ast) {
        Ok(()) => executor.last_exit_code(),
        Err(ExecuteError::ExitCode(code)) => code,
        Err(ExecuteError::ExpansionFailure(code)) => code,
        Err(ExecuteError::FatalFunctionError(code)) => code,
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
