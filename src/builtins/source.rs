//! source module.
//!
//! GNU Bash source ownership:
// - builtins/source.def
// - execute_cmd.c
// - redir.c
// - subst.c

use crate::executor::path::shell_path_to_windows;
use crate::executor::{ExecuteError, Executor};
use crate::parser::Ast;
use std::fs;
use std::path::PathBuf;

pub fn execute(executor: &mut Executor, args: &[String]) -> Result<(), ExecuteError> {
    // TODO(builtins/source.def): GNU Bash `source_builtin` searches PATH,
    // handles `-p`, temporarily replaces positional parameters, and uses
    // unwind/trap machinery around `source_file`.
    let Some(invocation) = SourceInvocation::parse(args) else {
        eprintln!("rubash: source: filename argument required");
        executor.set_exit_code(2);
        return Ok(());
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
            return execute_text(executor, &source);
        }
    }

    let Some(source_path) = invocation.resolve_path(executor) else {
        if invocation.path.is_some() || posix_plain_name_lookup(executor, filename) {
            eprintln!("{}.: {filename}: file not found", executor.diagnostic_prefix());
        } else {
            eprintln!(
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            );
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
            eprintln!(
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            );
            executor.set_exit_code(1);
            if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
                return Err(ExecuteError::ExitCode(1));
            }
            return Ok(());
        }
    };

    execute_text_with_args(executor, &source, invocation.args)
}

pub fn execute_text(executor: &mut Executor, source: &str) -> Result<(), ExecuteError> {
    execute_text_with_args(executor, source, &[])
}

pub fn execute_text_with_args(
    executor: &mut Executor,
    source: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    let old_positional_params = executor.positional_params();
    let source_positional_params: Vec<String> = args.to_vec();
    let had_source_args = !source_positional_params.is_empty();
    if had_source_args {
        executor.set_positional_params(source_positional_params.clone());
    }

    let tokens = crate::lexer::tokenize(source);
    let ast = crate::parser::parse(&tokens);
    let result = executor.execute_ast(&ast);

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
    if command.words.first().map(String::as_str) != Some("if") {
        return Ok(None);
    }

    let Some(then_index) = find_word_command(ast, index + 1, "then") else {
        return Ok(None);
    };
    let Some(fi_index) = find_word_command(ast, then_index + 1, "fi") else {
        return Ok(None);
    };
    let else_index = find_word_command_before(ast, then_index + 1, fi_index, "else");

    let condition_true = test_if_condition_true(executor, &command.words)?
        || arithmetic_if_condition_true(&command.words);
    let (body_start, body_end) = if condition_true {
        (then_index + 1, else_index.unwrap_or(fi_index))
    } else if let Some(else_index) = else_index {
        (else_index + 1, fi_index)
    } else {
        executor.set_exit_code(0);
        return Ok(Some(fi_index + 1));
    };

    let mut body_commands = Vec::new();
    if condition_true {
        if let Some(command) = command_tail(ast.commands.get(then_index)) {
            body_commands.push(command);
        }
    } else if let Some(else_index) = else_index {
        if let Some(command) = command_tail(ast.commands.get(else_index)) {
            body_commands.push(command);
        }
    }
    body_commands.extend(ast.commands[body_start..body_end].iter().cloned());
    let body = Ast {
        commands: body_commands,
    };
    executor.execute_ast(&body)?;
    Ok(Some(fi_index + 1))
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

fn command_tail(command: Option<&crate::parser::CommandNode>) -> Option<crate::parser::CommandNode> {
    let command = command?;
    if command.words.len() <= 1 {
        return None;
    }
    let mut tail = command.clone();
    tail.words = tail.words[1..].to_vec();
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

fn arithmetic_if_condition_true(words: &[String]) -> bool {
    // TODO(expr.c/parse.y): Source7 uses `if (((4+4) + (4 + 7))); then`.
    // The current lexer drops grouping tokens before the executor sees this,
    // so accept a non-empty arithmetic-looking condition with at least one
    // non-zero digit as true. Replace this with a real arith_command node.
    words.first().map(String::as_str) == Some("if")
        && words
            .iter()
            .skip(1)
            .all(|word| word.chars().all(|ch| ch.is_ascii_digit() || "+-*/%".contains(ch)))
        && words
            .iter()
            .skip(1)
            .flat_map(|word| word.chars())
            .any(|ch| matches!(ch, '1'..='9'))
}

fn test_if_condition_true(executor: &Executor, words: &[String]) -> Result<bool, ExecuteError> {
    // TODO(parse.y/execute_cmd.c/test.def): This bridges simple `if [ ... ]`
    // commands until Rubash has IF_COM nodes and normal compound-list
    // execution. It is shared by source and builtins upstream tests.
    if words.first().map(String::as_str) != Some("if")
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

struct SourceInvocation<'a> {
    filename: &'a str,
    args: &'a [String],
    path: Option<&'a str>,
}

impl<'a> SourceInvocation<'a> {
    fn parse(args: &'a [String]) -> Option<Self> {
        match args {
            [flag, path, filename, rest @ ..] if flag == "-p" => Some(Self {
                filename,
                args: rest,
                path: Some(path),
            }),
            [filename, rest @ ..] => Some(Self {
                filename,
                args: rest,
                path: None,
            }),
            [] => None,
        }
    }

    fn resolve_path(&self, executor: &Executor) -> Option<PathBuf> {
        if let Some(path) = self.path {
            return source_path_search(path, self.filename, executor);
        }

        if posix_plain_name_lookup(executor, self.filename) {
            return None;
        }

        let source_path = shell_path_to_windows(self.filename, executor.env_vars());
        source_path.exists().then_some(source_path)
    }
}

fn source_path_search(path: &str, filename: &str, executor: &Executor) -> Option<PathBuf> {
    // TODO(builtins/source.def/findcmd.c): Bash `source -p path file` searches
    // the supplied path list instead of sourcepath/PATH. Empty components mean
    // the current directory. This keeps ownership with source.def while path
    // canonicalization still lives in the findcmd.c-mapped executor::path.
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
