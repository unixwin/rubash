use crate::lexer::{Token, TokenKind};

use super::*;

pub(super) struct ParseState {
    pub(super) ast: Ast,
    pub(super) current_cmd: CommandNode,
    pub(super) in_subshell: bool,
}

/// Parse tokens into an AST
pub fn parse(tokens: &[Token]) -> Ast {
    let mut state = ParseState {
        ast: Ast {
            commands: Vec::new(),
        },
        current_cmd: CommandNode::new(),
        in_subshell: false,
    };

    let mut i = 0;
    while i < tokens.len() {
        if let Some(next_i) = try_parse_compound_start(tokens, i, &mut state) {
            i = next_i;
            continue;
        }

        match handle_token(tokens, &mut i, &mut state) {
            TokenAction::Advance => i += 1,
            TokenAction::Continue => continue,
            TokenAction::Break => break,
        }
    }

    if !command_is_empty(&state.current_cmd) {
        state.ast.commands.push(state.current_cmd);
    }

    state.ast.commands = fold_pipeline_commands(state.ast.commands);
    state.ast
}

fn fold_pipeline_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut folded = Vec::new();
    let mut index = 0;
    while index < commands.len() {
        let command = commands[index].clone();
        if command.pipe.is_none() {
            folded.push(command);
            index += 1;
            continue;
        }

        let mut stages = vec![command];
        index += 1;
        while let Some(command) = commands.get(index) {
            stages.push(command.clone());
            index += 1;
            if command.pipe.is_none() {
                break;
            }
        }

        if stages.len() == 1
            || stages.last().is_some_and(|command| command.pipe.is_some())
            || looks_like_case_pattern_alternate(&stages)
        {
            folded.extend(stages);
            continue;
        }

        let first = stages.first().expect("pipeline has a first stage");
        let last = stages.last().expect("pipeline has a last stage");
        let mut pipeline = CommandNode::new();
        pipeline.line = first.line;
        pipeline.inverted = first.inverted;
        pipeline.background = last.background;
        pipeline.and_or = last.and_or;
        if let Some(first_stage) = stages.first_mut() {
            first_stage.inverted = false;
        }
        pipeline.pipeline_command = Some(PipelineCommand { stages });
        folded.push(pipeline);
    }
    folded
}

fn looks_like_case_pattern_alternate(stages: &[CommandNode]) -> bool {
    let Some(first) = stages.first() else {
        return false;
    };
    if first.words.get(2).map(String::as_str) != Some("in") {
        return false;
    }
    first.words.len() >= 4 && stages.len() >= 2
}

fn try_parse_compound_start(tokens: &[Token], i: usize, state: &mut ParseState) -> Option<usize> {
    let token = &tokens[i];

    if token.kind == TokenKind::Keyword
        && token.value == "time"
        && command_is_empty(&state.current_cmd)
    {
        if let Some((time_cmd, next_i)) = parse_time_prefixed_compound_command(tokens, i) {
            state.ast.commands.push(time_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "if"
        && command_is_empty(&state.current_cmd)
    {
        if let Some((if_cmd, next_i)) = parse_if_command(tokens, i) {
            state.ast.commands.push(if_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && matches!(token.value.as_str(), "while" | "until")
        && command_is_empty(&state.current_cmd)
    {
        if let Some((loop_cmd, next_i)) = parse_loop_command(tokens, i) {
            state.ast.commands.push(loop_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "for"
        && command_is_empty(&state.current_cmd)
    {
        if let Some((for_cmd, next_i)) = parse_for_command(tokens, i) {
            state.ast.commands.push(for_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if ((token.kind == TokenKind::Word)
        || (token.kind == TokenKind::Keyword && token.value == "function"))
        && command_is_empty(&state.current_cmd)
    {
        if let Some((function_cmd, next_i)) = parse_function_command(tokens, i) {
            state.ast.commands.push(function_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "case"
        && command_is_empty(&state.current_cmd)
    {
        if let Some((case_cmd, next_i)) = parse_case_command(tokens, i) {
            state.ast.commands.push(case_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "select"
        && command_is_empty(&state.current_cmd)
    {
        if let Some((select_cmd, next_i)) = parse_select_command(tokens, i) {
            state.ast.commands.push(select_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "coproc"
        && command_is_empty(&state.current_cmd)
    {
        if let Some((coproc_cmd, next_i)) = parse_coproc_command(tokens, i) {
            state.ast.commands.push(coproc_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if command_is_empty(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            state.ast.commands.push(arith_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if command_is_empty(&state.current_cmd) && token.value == "[[" {
        if let Some((conditional_cmd, next_i)) = parse_conditional_command(tokens, i) {
            state.ast.commands.push(conditional_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if command_is_empty(&state.current_cmd)
        && token.kind == TokenKind::Keyword
        && token.value == "("
    {
        if let Some((subshell_cmd, next_i)) = parse_subshell_command(tokens, i) {
            state.ast.commands.push(subshell_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    if command_accepts_embedded_arithmetic_command(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            note_command_line(&mut state.current_cmd, token);
            state.current_cmd.words.extend(arith_cmd.words);
            state.current_cmd.and_or = arith_cmd.and_or;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(next_i);
        }
    }

    if command_is_empty(&state.current_cmd) {
        if let Some((brace_cmd, next_i)) = parse_brace_group_command(tokens, i) {
            state.ast.commands.push(brace_cmd);
            state.current_cmd = CommandNode::new();
            return Some(next_i);
        }
    }

    None
}

fn parse_time_prefixed_compound_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let mut words = vec![tokens.get(start)?.value.clone()];
    let mut i = start + 1;
    while tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "-p" | "--" | "!"))
    {
        words.push(tokens[i].value.clone());
        i += 1;
    }

    let (mut command, next_i) = if is_keyword(tokens, i, "for") {
        parse_for_command(tokens, i)?
    } else if is_keyword(tokens, i, "case") {
        parse_case_command(tokens, i)?
    } else if is_keyword(tokens, i, "select") {
        parse_select_command(tokens, i)?
    } else if is_keyword(tokens, i, "coproc") {
        parse_coproc_command(tokens, i)?
    } else if is_keyword(tokens, i, "{")
        || tokens.get(i).is_some_and(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
    {
        parse_brace_group_command(tokens, i)?
    } else if is_keyword(tokens, i, "(") && !is_keyword(tokens, i + 1, "(") {
        parse_subshell_command(tokens, i)?
    } else if tokens
        .get(i)
        .is_some_and(|token| token.value.starts_with("(("))
    {
        parse_arithmetic_command(tokens, i)?
    } else {
        return None;
    };

    command.words = words;
    Some((command, next_i))
}
