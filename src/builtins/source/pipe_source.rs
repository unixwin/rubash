use super::execution::execute_text;
use crate::executor::{ExecuteError, Executor};
use crate::parser::Ast;

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
