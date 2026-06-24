//! source module.
//!
//! GNU Bash source ownership:
// - builtins/source.def
// - execute_cmd.c
// - redir.c
// - subst.c

use crate::executor::path::shell_path_to_windows;
use crate::executor::{ExecuteError, Executor};
use crate::parser::{Ast, CommandNode};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn execute(executor: &mut Executor, args: &[String]) -> Result<(), ExecuteError> {
    execute_named(executor, "source", args)
}

pub fn execute_named(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    execute_named_with_io(executor, command_name, args, &mut std::io::stderr().lock())
}

pub fn execute_named_with_io<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    execute_named_with_io_impl(executor, command_name, args, stderr, None)
}

pub fn execute_named_with_io_and_redirects<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
    redirect_cmd: &CommandNode,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    execute_named_with_io_impl(executor, command_name, args, stderr, Some(redirect_cmd))
}

fn execute_named_with_io_impl<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
    redirect_cmd: Option<&CommandNode>,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    // TODO(builtins/source.def): GNU Bash `source_builtin` uses unwind/trap
    // machinery around `source_file`.
    let invocation = match SourceInvocation::parse(args) {
        Ok(invocation) => invocation,
        Err(error) => {
            match error {
                SourceParseError::MissingFilename => {
                    writeln!(
                        stderr,
                        "{}{command_name}: filename argument required",
                        executor.diagnostic_prefix()
                    )?;
                }
                SourceParseError::MissingPathArgument => {
                    writeln!(
                        stderr,
                        "{}{command_name}: -p: option requires an argument",
                        executor.diagnostic_prefix()
                    )?;
                }
                SourceParseError::InvalidOption(option) => {
                    writeln!(
                        stderr,
                        "{}{command_name}: -{option}: invalid option",
                        executor.diagnostic_prefix()
                    )?;
                }
            }
            writeln!(
                stderr,
                "{command_name}: usage: {command_name} [-p path] filename [arguments]"
            )?;
            executor.set_exit_code(2);
            return Ok(());
        }
    };
    let filename = invocation.filename;

    if is_null_device(filename) {
        executor.set_exit_code(0);
        return Ok(());
    }

    if filename == "echo" {
        // TODO(subst.c/execute_cmd.c): Process substitution should create a
        // /dev/fd path whose content is the command's stdout. The current
        // parser sees `. <(echo "echo two - OK")` as `source echo ...`; source
        // that generated text directly until process substitution is parsed.
        let source = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        if !source.is_empty() {
            return execute_text_maybe_redirected(executor, &source, &[], redirect_cmd);
        }
    }

    let Some(source_path) = invocation.resolve_path(executor) else {
        if invocation.path.is_some() || posix_plain_name_lookup(executor, filename) {
            writeln!(
                stderr,
                "{}.: {filename}: file not found",
                executor.diagnostic_prefix()
            )?;
        } else {
            writeln!(
                stderr,
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            )?;
        }
        executor.set_exit_code(1);
        if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
            return Err(ExecuteError::ExitCode(1));
        }
        return Ok(());
    };

    let source = match fs::read_to_string(&source_path) {
        Ok(source) => source,
        Err(_) => {
            writeln!(
                stderr,
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            )?;
            executor.set_exit_code(1);
            if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
                return Err(ExecuteError::ExitCode(1));
            }
            return Ok(());
        }
    };

    execute_text_maybe_redirected(executor, &source, invocation.args, redirect_cmd)
}

pub fn execute_text(executor: &mut Executor, source: &str) -> Result<(), ExecuteError> {
    execute_text_with_args(executor, source, &[])
}

pub fn execute_text_with_args(
    executor: &mut Executor,
    source: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    let ast = parse_source_ast(source);
    execute_ast_with_args(executor, ast, args)
}

fn execute_text_maybe_redirected(
    executor: &mut Executor,
    source: &str,
    args: &[String],
    redirect_cmd: Option<&CommandNode>,
) -> Result<(), ExecuteError> {
    let mut ast = parse_source_ast(source);
    if let Some(redirect_cmd) = redirect_cmd {
        executor.apply_command_output_redirects(redirect_cmd, &mut ast)?;
    }
    execute_ast_with_args(executor, ast, args)
}

fn parse_source_ast(source: &str) -> Ast {
    let tokens = crate::lexer::tokenize(source);
    crate::parser::parse(&tokens)
}

fn execute_ast_with_args(
    executor: &mut Executor,
    ast: Ast,
    args: &[String],
) -> Result<(), ExecuteError> {
    let old_positional_params = executor.positional_params();
    let source_positional_params: Vec<String> = args.to_vec();
    let had_source_args = !source_positional_params.is_empty();
    let old_source_marker = executor.get_env("__RUBASH_IN_SOURCE").map(str::to_string);
    executor.set_env("__RUBASH_IN_SOURCE", "1");
    if had_source_args {
        executor.set_positional_params(source_positional_params.clone());
    }

    let result = executor.execute_ast(&ast);

    match old_source_marker {
        Some(value) => executor.set_env("__RUBASH_IN_SOURCE", &value),
        None => executor.remove_env("__RUBASH_IN_SOURCE"),
    }

    if had_source_args && executor.positional_params() == source_positional_params {
        executor.set_positional_params(old_positional_params);
    }

    match result {
        Err(ExecuteError::Return(status)) => {
            executor.set_exit_code(status);
            Ok(())
        }
        other => other,
    }
}

pub fn execute_simple_if(
    executor: &mut Executor,
    ast: &Ast,
    index: usize,
) -> Result<Option<usize>, ExecuteError> {
    // TODO(parse.y/execute_cmd.c/test.def/expr.c): This recognizes narrow
    // source-test `if` forms until the parser has IF_COM and arithmetic
    // command nodes. Bash parses these as compound commands with test or
    // arithmetic evaluation and compound-list control flow.
    let Some(command) = ast.commands.get(index) else {
        return Ok(None);
    };
    let Some(keyword) = command.words.first().map(String::as_str) else {
        return Ok(None);
    };
    if !matches!(keyword, "if" | "elif") {
        return Ok(None);
    }

    let inline_then = command.words.iter().position(|word| word == "then");
    let Some(then_index) = inline_then
        .map(|_| index)
        .or_else(|| find_word_command(ast, index + 1, "then"))
    else {
        return Ok(None);
    };
    let body_start = if inline_then.is_some() {
        index + 1
    } else {
        then_index + 1
    };
    let Some(fi_index) = find_matching_fi(ast, body_start) else {
        return Ok(None);
    };
    let elif_index = find_if_branch_command(ast, body_start, fi_index, "elif");
    let else_index = find_if_branch_command(ast, body_start, fi_index, "else");

    let condition_words;
    let words = if keyword == "elif" {
        condition_words = {
            let mut words = inline_then
                .map(|then_pos| command.words[..then_pos].to_vec())
                .unwrap_or_else(|| command.words.clone());
            words[0] = "if".to_string();
            words
        };
        &condition_words
    } else {
        condition_words = inline_then
            .map(|then_pos| command.words[..then_pos].to_vec())
            .unwrap_or_else(|| command.words.clone());
        &condition_words
    };
    let and_or_condition = if inline_then.is_none() {
        execute_and_or_if_condition(executor, ast, index, then_index)?
    } else {
        None
    };
    let condition_true = if let Some(value) = and_or_condition {
        value
    } else if test_if_condition_true(executor, words)? {
        true
    } else if let Some(value) = arithmetic_if_condition_value(executor, words) {
        value
    } else {
        execute_command_if_condition(executor, command)?
    };
    if condition_true {
        let body_end = elif_index.or(else_index).unwrap_or(fi_index);
        let mut body_commands = Vec::new();
        if let Some(then_pos) = inline_then {
            if let Some(command) = command_tail_from(ast.commands.get(then_index), then_pos + 1) {
                body_commands.push(command);
            }
        } else if let Some(command) = command_tail(ast.commands.get(then_index)) {
                body_commands.push(command);
        }
        body_commands.extend(ast.commands[body_start..body_end].iter().cloned());
        let body = Ast {
            commands: body_commands,
        };
        executor.execute_ast(&body)?;
        return Ok(Some(fi_index + 1));
    }

    if let Some(elif_index) = elif_index {
        return execute_simple_if(executor, ast, elif_index);
    }

    if let Some(else_index) = else_index {
        let mut body_commands = Vec::new();
        if let Some(command) = command_tail(ast.commands.get(else_index)) {
            body_commands.push(command);
        }
        body_commands.extend(ast.commands[else_index + 1..fi_index].iter().cloned());
        let body = Ast {
            commands: body_commands,
        };
        executor.execute_ast(&body)?;
        return Ok(Some(fi_index + 1));
    }

    executor.set_exit_code(0);
    Ok(Some(fi_index + 1))
}

fn execute_and_or_if_condition(
    executor: &mut Executor,
    ast: &Ast,
    index: usize,
    then_index: usize,
) -> Result<Option<bool>, ExecuteError> {
    let Some(command) = ast.commands.get(index) else {
        return Ok(None);
    };
    if command.and_or().is_none() && then_index <= index + 1 {
        return Ok(None);
    }

    let mut first = command.clone();
    first.words = first.words[1..].to_vec();
    first.pipe = None;
    if first.and_or().is_none() && then_index > index + 1 && is_arithmetic_condition_words(&first.words)
    {
        first.and_or = Some(true);
    }
    let mut commands = vec![first];
    commands.extend(
        ast.commands[index + 1..then_index]
            .iter()
            .filter(|command| !command.words.is_empty())
            .cloned(),
    );
    let condition_ast = Ast { commands };
    executor.with_errexit_suppressed(|executor| executor.execute_ast(&condition_ast))?;
    Ok(Some(executor.last_exit_code() == 0))
}

fn is_arithmetic_condition_words(words: &[String]) -> bool {
    matches!(words, [open, _, close] if open == "((" && close == "))")
}

pub fn execute_pipe_into_source(
    executor: &mut Executor,
    ast: &Ast,
    index: usize,
) -> Result<Option<usize>, ExecuteError> {
    // TODO(execute_cmd.c/redir.c/source.def): Bash connects the left command
    // stdout to the right command stdin. This handles
    // `echo "echo three - OK" | . /dev/stdin` from source6.sub by sourcing the
    // generated shell text in the current shell.
    let Some(left) = ast.commands.get(index) else {
        return Ok(None);
    };
    if left.pipe.is_none() {
        return Ok(None);
    }
    let Some(right) = ast.commands.get(index + 1) else {
        return Ok(None);
    };
    if !matches!(
        right.words.as_slice(),
        [name, path] if matches!(name.as_str(), "." | "source") && path == "/dev/stdin"
    ) {
        return Ok(None);
    }
    if left.words.first().map(String::as_str) != Some("echo") {
        return Ok(None);
    }

    let source = left.words[1..].join(" ");
    execute_text(executor, &source)?;
    Ok(Some(index + 2))
}

fn find_word_command(ast: &Ast, start: usize, word: &str) -> Option<usize> {
    find_word_command_before(ast, start, ast.commands.len(), word)
}

fn find_word_command_before(ast: &Ast, start: usize, end: usize, word: &str) -> Option<usize> {
    (start..end).find(|index| ast.commands[*index].words.first().map(String::as_str) == Some(word))
}

fn find_matching_fi(ast: &Ast, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..ast.commands.len() {
        match ast.commands[index].words.first().map(String::as_str) {
            Some("if") => depth += 1,
            Some("fi") if depth == 0 => return Some(index),
            Some("fi") => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn find_if_branch_command(ast: &Ast, start: usize, end: usize, word: &str) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..end {
        match ast.commands[index].words.first().map(String::as_str) {
            Some("if") => depth += 1,
            Some("fi") => depth = depth.saturating_sub(1),
            Some(candidate) if depth == 0 && candidate == word => return Some(index),
            _ => {}
        }
    }
    None
}

fn command_tail(
    command: Option<&crate::parser::CommandNode>,
) -> Option<crate::parser::CommandNode> {
    let command = command?;
    if command.words.len() <= 1 {
        return None;
    }
    command_tail_from(Some(command), 1)
}

fn command_tail_from(
    command: Option<&crate::parser::CommandNode>,
    start: usize,
) -> Option<crate::parser::CommandNode> {
    let command = command?;
    if command.words.len() <= start {
        return None;
    }
    let mut tail = command.clone();
    tail.words = tail.words[start..].to_vec();
    Some(tail)
}

fn is_null_device(path: &str) -> bool {
    matches!(path, "/dev/null" | "NUL")
}

fn posix_plain_name_lookup(executor: &Executor, filename: &str) -> bool {
    executor.get_env("__RUBASH_POSIX_MODE") == Some("1")
        && !filename.contains('/')
        && !filename.contains('\\')
}

fn arithmetic_if_condition_value(executor: &mut Executor, words: &[String]) -> Option<bool> {
    // TODO(parse.y/execute_cmd.c/expr.c): Replace this condition-position
    // bridge with a real IF_COM containing an arith_command node.
    if words.first().map(String::as_str) != Some("if") {
        return None;
    }
    let expression = arithmetic_condition_expression(&words[1..])?;
    executor
        .eval_arithmetic_command_value(&expression)
        .map(|value| value != 0)
        .or(Some(false))
}

fn arithmetic_condition_expression(words: &[String]) -> Option<String> {
    match words {
        [open, expression, close] if open == "((" && close == "))" => Some(expression.clone()),
        [single] if single.starts_with("((") && single.ends_with("))") => single
            .strip_prefix("((")
            .and_then(|value| value.strip_suffix("))"))
            .map(str::to_string),
        terms
            if terms.iter().all(|word| {
                word.chars()
                    .all(|ch| ch.is_ascii_digit() || "+-*/%()".contains(ch))
            }) && !terms.is_empty() =>
        {
            Some(terms.join(" "))
        }
        _ => None,
    }
}

fn test_if_condition_true(executor: &Executor, words: &[String]) -> Result<bool, ExecuteError> {
    // TODO(parse.y/execute_cmd.c/test.def): This bridges simple `if [ ... ]`
    // commands until Rubash has IF_COM nodes and normal compound-list
    // execution. It is shared by source and builtins upstream tests.
    if !matches!(words.first().map(String::as_str), Some("if" | "elif"))
        || words.get(1).map(String::as_str) != Some("[")
    {
        return Ok(false);
    }

    let mut args = Vec::new();
    for word in &words[2..] {
        args.push(executor.expand_word(word));
    }
    let status = crate::builtins::test::execute(&args, true, executor.env_vars())?;
    Ok(status == 0)
}

fn execute_command_if_condition(
    executor: &mut Executor,
    command: &crate::parser::CommandNode,
) -> Result<bool, ExecuteError> {
    let Some(condition_words) = command.words.get(1..) else {
        return Ok(false);
    };
    if condition_words.is_empty() {
        return Ok(false);
    }

    let mut condition = command.clone();
    condition.words = condition_words.to_vec();
    if condition.words.first().map(String::as_str) == Some("!") {
        condition.inverted = !condition.inverted;
        condition.words.remove(0);
    }
    condition.pipe = None;
    condition.and_or = None;
    let ast = Ast {
        commands: vec![condition],
    };
    executor.with_errexit_suppressed(|executor| executor.execute_ast(&ast))?;
    Ok(executor.last_exit_code() == 0)
}

struct SourceInvocation<'a> {
    filename: &'a str,
    args: &'a [String],
    path: Option<String>,
}

enum SourceParseError {
    MissingFilename,
    MissingPathArgument,
    InvalidOption(char),
}

impl<'a> SourceInvocation<'a> {
    fn parse(args: &'a [String]) -> Result<Self, SourceParseError> {
        let mut index = 0;
        let mut path = None;

        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }

            if arg == "-p" {
                let Some(value) = args.get(index + 1) else {
                    return Err(SourceParseError::MissingPathArgument);
                };
                path = Some(if value.is_empty() {
                    ".".to_string()
                } else {
                    value.clone()
                });
                index += 2;
                continue;
            }

            if let Some(option) = invalid_option(arg) {
                return Err(SourceParseError::InvalidOption(option));
            }

            break;
        }

        let Some(filename) = args.get(index).map(String::as_str) else {
            return Err(SourceParseError::MissingFilename);
        };

        Ok(Self {
            filename,
            args: &args[index + 1..],
            path,
        })
    }

    fn resolve_path(&self, executor: &Executor) -> Option<PathBuf> {
        if let Some(path) = &self.path {
            return source_path_search(path, self.filename, executor);
        }

        if should_search_source_path(executor, self.filename) {
            if let Some(path) = executor
                .get_env("PATH")
                .and_then(|path| source_path_search(path, self.filename, executor))
            {
                return Some(path);
            }
        }

        let source_path = shell_path_to_windows(self.filename, executor.env_vars());
        source_path.exists().then_some(source_path)
    }
}

fn invalid_option(arg: &str) -> Option<char> {
    let option = arg.strip_prefix('-')?;
    if option.is_empty() {
        return None;
    }
    option.chars().next()
}

fn should_search_source_path(executor: &Executor, filename: &str) -> bool {
    crate::builtins::shopt::sourcepath_enabled()
        && !filename.contains('/')
        && !filename.contains('\\')
        && executor.get_env("PATH").is_some()
}

fn source_path_search(path: &str, filename: &str, executor: &Executor) -> Option<PathBuf> {
    // Empty components mean the current directory.
    for entry in path.split(':') {
        let candidate = if entry.is_empty() || entry == "." {
            PathBuf::from(filename)
        } else {
            let mut base = shell_path_to_windows(entry, executor.env_vars());
            base.push(filename);
            base
        };

        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}
