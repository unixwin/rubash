use super::flow::{command_tail, command_tail_from, normalize_inline_compound_commands};
use super::if_alias::{
    command_tail_starts_if, control_words, find_if_branch_command, find_matching_fi,
    find_word_command,
};
use crate::executor::{ExecuteError, Executor};
use crate::parser::{Ast, CommandNode};

pub fn execute_simple_if(
    executor: &mut Executor,
    ast: &Ast,
    index: usize,
) -> Result<Option<usize>, ExecuteError> {
    execute_simple_if_inner(executor, ast, index, false, false)
}

fn execute_simple_if_inner(
    executor: &mut Executor,
    ast: &Ast,
    index: usize,
    output_redirects_applied: bool,
    input_redirects_applied: bool,
) -> Result<Option<usize>, ExecuteError> {
    // TODO(parse.y/execute_cmd.c/test.def/expr.c): This recognizes narrow
    // source-test `if` forms until the parser has IF_COM and arithmetic
    // command nodes. Bash parses these as compound commands with test or
    // arithmetic evaluation and compound-list control flow.
    let Some(command) = ast.commands.get(index) else {
        return Ok(None);
    };
    let Some(command_words) = control_words(executor, command, &["if", "elif"]) else {
        return Ok(None);
    };
    let keyword = command_words[0].as_str();

    let inline_then = command_words.iter().position(|word| word == "then");
    let Some(then_index) = inline_then
        .map(|_| index)
        .or_else(|| find_word_command(executor, ast, index + 1, "then"))
    else {
        return Ok(None);
    };
    let body_start = if inline_then.is_some() {
        index + 1
    } else {
        then_index + 1
    };
    let branch_scan_start = if inline_then.is_none()
        && ast
            .commands
            .get(then_index)
            .is_some_and(|command| command_tail_starts_if(executor, command, 1))
    {
        then_index
    } else {
        body_start
    };
    let Some(fi_index) = find_matching_fi(executor, ast, branch_scan_start) else {
        return Ok(None);
    };

    let fi_command = ast.commands.get(fi_index).expect("fi index is valid");
    if !output_redirects_applied && command_has_output_redirects(fi_command) {
        let mut redirected = Ast {
            commands: ast.commands[index..=fi_index].to_vec(),
        };
        executor.apply_command_output_redirects(fi_command, &mut redirected)?;
        execute_simple_if_inner(executor, &redirected, 0, true, input_redirects_applied)?;
        return Ok(Some(fi_index + 1));
    }

    if !input_redirects_applied && command_has_input_redirects(fi_command) {
        executor.with_command_input_redirects(fi_command, |executor| {
            execute_simple_if_inner(executor, ast, index, output_redirects_applied, true)
        })?;
        return Ok(Some(fi_index + 1));
    }

    let elif_index = find_if_branch_command(executor, ast, branch_scan_start, fi_index, "elif");
    let else_index = find_if_branch_command(executor, ast, branch_scan_start, fi_index, "else");

    let condition_words;
    let words = if keyword == "elif" {
        condition_words = {
            let mut words = inline_then
                .map(|then_pos| command_words[..then_pos].to_vec())
                .unwrap_or_else(|| command_words.to_vec());
            words[0] = "if".to_string();
            words
        };
        &condition_words
    } else {
        condition_words = inline_then
            .map(|then_pos| command_words[..then_pos].to_vec())
            .unwrap_or_else(|| command_words.to_vec());
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
        let mut condition_command = command.clone();
        condition_command.words = words.to_vec();
        execute_command_if_condition(executor, &condition_command)?
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
            commands: normalize_inline_compound_commands(body_commands),
        };
        executor.execute_ast(&body)?;
        return Ok(Some(fi_index + 1));
    }

    if let Some(elif_index) = elif_index {
        return execute_simple_if_inner(
            executor,
            ast,
            elif_index,
            output_redirects_applied,
            input_redirects_applied,
        );
    }

    if let Some(else_index) = else_index {
        let mut body_commands = Vec::new();
        if let Some(command) = command_tail(ast.commands.get(else_index)) {
            body_commands.push(command);
        }
        body_commands.extend(ast.commands[else_index + 1..fi_index].iter().cloned());
        let body = Ast {
            commands: normalize_inline_compound_commands(body_commands),
        };
        executor.execute_ast(&body)?;
        return Ok(Some(fi_index + 1));
    }

    executor.set_exit_code(0);
    Ok(Some(fi_index + 1))
}

fn command_has_output_redirects(command: &CommandNode) -> bool {
    command.redirect_out.is_some()
        || command.append.is_some()
        || command.redirect_err.is_some()
        || command.redirect_err_append.is_some()
}

fn command_has_input_redirects(command: &CommandNode) -> bool {
    command.redirect_in.is_some()
        || command.here_string.is_some()
        || command
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.fd.is_none() && redirect.body.is_some())
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
    if first.and_or().is_none()
        && then_index > index + 1
        && is_arithmetic_condition_words(&first.words)
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
    command: &CommandNode,
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
