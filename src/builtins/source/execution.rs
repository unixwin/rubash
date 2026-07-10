use crate::executor::{ExecuteError, Executor};
use crate::parser::{Ast, CommandNode};

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

pub(super) fn execute_text_maybe_redirected(
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
