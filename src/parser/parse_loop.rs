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
    state.ast.commands = fold_inverted_commands(state.ast.commands);
    state.ast.commands = fold_and_or_list_commands(state.ast.commands);
    state.ast.commands = fold_background_commands(state.ast.commands);
    state.ast
}

fn fold_inverted_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command.inverted {
                return command;
            }

            command.inverted = false;
            let line = command.line;
            let and_or = command.and_or.take();
            let background_flag = command.background;
            command.background = false;
            let mut inverted = CommandNode::new();
            inverted.line = line;
            inverted.and_or = and_or;
            inverted.background = background_flag;
            inverted.inverted_command = Some(InvertedCommand {
                operator: "!".to_string(),
                command: Box::new(command),
            });
            inverted
        })
        .collect()
}

fn fold_background_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command.background {
                return command;
            }

            command.background = false;
            let line = command.line;
            let mut background = CommandNode::new();
            background.line = line;
            background.background_command = Some(BackgroundCommand {
                operator: "&".to_string(),
                command: Box::new(command),
            });
            background
        })
        .collect()
}

fn fold_and_or_list_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut folded = Vec::new();
    let mut index = 0;
    while index < commands.len() {
        let command = commands[index].clone();
        if command.and_or.is_none() {
            folded.push(command);
            index += 1;
            continue;
        }

        let mut list_commands = vec![command];
        let mut connectors = Vec::new();
        let mut operators = Vec::new();
        index += 1;
        while let Some(connector) = list_commands.last().and_then(|command| command.and_or) {
            connectors.push(connector);
            operators.push(if connector { "&&" } else { "||" }.to_string());
            while commands.get(index).is_some_and(command_is_empty) {
                index += 1;
            }
            let Some(next) = commands.get(index).cloned() else {
                break;
            };
            list_commands.push(next);
            index += 1;
        }

        if connectors.is_empty() || list_commands.len() != connectors.len() + 1 {
            folded.extend(list_commands);
            continue;
        }

        let first = list_commands
            .first()
            .expect("and-or list has a first command");
        let last = list_commands
            .last()
            .expect("and-or list has a last command");
        let mut list = CommandNode::new();
        list.line = first.line;
        list.background = last.background;
        list.and_or_list = Some(AndOrListCommand {
            commands: list_commands,
            connectors,
            operators,
        });
        folded.push(list);
    }
    folded
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
        let mut operators = Vec::new();
        index += 1;
        while let Some(command) = commands.get(index) {
            if let Some(pipe) = stages.last().and_then(|stage| stage.pipe) {
                operators.push(if pipe == 2 { "|&" } else { "|" }.to_string());
            }
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
        pipeline.pipeline_command = Some(PipelineCommand { stages, operators });
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
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((time_cmd, next_i)) = parse_time_prefixed_compound_command(tokens, i) {
            push_compound_command(state, time_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "if"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((if_cmd, next_i)) = parse_if_command(tokens, i) {
            push_compound_command(state, if_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && matches!(token.value.as_str(), "while" | "until")
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((loop_cmd, next_i)) = parse_loop_command(tokens, i) {
            push_compound_command(state, loop_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "for"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((for_cmd, next_i)) = parse_for_command(tokens, i) {
            push_compound_command(state, for_cmd);
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
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((case_cmd, next_i)) = parse_case_command(tokens, i) {
            push_compound_command(state, case_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "select"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((select_cmd, next_i)) = parse_select_command(tokens, i) {
            push_compound_command(state, select_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "coproc"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((coproc_cmd, next_i)) = parse_coproc_command(tokens, i) {
            push_compound_command(state, coproc_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            push_compound_command(state, arith_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd) && token.value == "[[" {
        if let Some((conditional_cmd, next_i)) = parse_conditional_command(tokens, i) {
            push_compound_command(state, conditional_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd)
        && token.kind == TokenKind::Keyword
        && token.value == "("
    {
        if let Some((subshell_cmd, next_i)) = parse_subshell_command(tokens, i) {
            push_compound_command(state, subshell_cmd);
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

    if command_allows_compound_start(&state.current_cmd) {
        if let Some((brace_cmd, next_i)) = parse_brace_group_command(tokens, i) {
            push_compound_command(state, brace_cmd);
            return Some(next_i);
        }
    }

    None
}

fn command_allows_compound_start(command: &CommandNode) -> bool {
    command_is_empty(command) || command_is_pending_inversion(command)
}

fn command_is_pending_inversion(command: &CommandNode) -> bool {
    if !command.inverted {
        return false;
    }
    let mut without_inversion = command.clone();
    without_inversion.inverted = false;
    command_is_empty(&without_inversion)
}

fn push_compound_command(state: &mut ParseState, mut command: CommandNode) {
    if command_is_pending_inversion(&state.current_cmd) {
        command.inverted = !command.inverted;
        command.line = command.line.or(state.current_cmd.line);
    }
    state.ast.commands.push(command);
    state.current_cmd = CommandNode::new();
}

fn parse_time_prefixed_compound_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    tokens.get(start)?;
    let mut posix_format = false;
    let mut inverted = false;
    let mut prefix_words = Vec::new();
    let mut i = start + 1;
    while tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "-p" | "--" | "!"))
    {
        prefix_words.push(tokens[i].value.clone());
        match tokens[i].value.as_str() {
            "-p" => posix_format = true,
            "!" => inverted = !inverted,
            _ => {}
        }
        i += 1;
    }

    let (mut command, next_i) = if is_keyword(tokens, i, "for") {
        parse_for_command(tokens, i)?
    } else if is_keyword(tokens, i, "if") {
        parse_if_command(tokens, i)?
    } else if tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "while" | "until"))
    {
        parse_loop_command(tokens, i)?
    } else if is_keyword(tokens, i, "case") {
        parse_case_command(tokens, i)?
    } else if is_keyword(tokens, i, "select") {
        parse_select_command(tokens, i)?
    } else if is_keyword(tokens, i, "coproc") {
        parse_coproc_command(tokens, i)?
    } else if tokens.get(i).is_some_and(|token| token.value == "[[") {
        parse_conditional_command(tokens, i)?
    } else if is_keyword(tokens, i, "{")
        || tokens.get(i).is_some_and(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
    {
        parse_brace_group_command(tokens, i)?
    } else if let Some(parsed) = parse_arithmetic_command(tokens, i) {
        parsed
    } else if is_keyword(tokens, i, "(") {
        parse_subshell_command(tokens, i)?
    } else {
        return None;
    };

    let and_or = command.and_or.take();
    let mut timed = CommandNode::new();
    timed.line = tokens.get(start).map(|token| token.position);
    timed.and_or = and_or;
    timed.time_command = Some(TimeCommand {
        keyword: tokens[start].value.clone(),
        prefix_words,
        command: Box::new(command),
        posix_format,
        inverted,
    });
    Some((timed, next_i))
}
