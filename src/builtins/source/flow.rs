use crate::parser::{ArithmeticForCommand, Ast, CommandNode, ForCommand};

pub(super) fn find_word_command(ast: &Ast, start: usize, word: &str) -> Option<usize> {
    find_word_command_before(ast, start, ast.commands.len(), word)
}

fn find_word_command_before(ast: &Ast, start: usize, end: usize, word: &str) -> Option<usize> {
    (start..end).find(|index| ast.commands[*index].words.first().map(String::as_str) == Some(word))
}

pub(super) fn find_matching_fi(ast: &Ast, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..ast.commands.len() {
        if command_starts_if(&ast.commands[index]) {
            depth += 1;
            continue;
        }
        match ast.commands[index].words.first().map(String::as_str) {
            Some("fi") if depth == 0 => return Some(index),
            Some("fi") => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

pub(super) fn find_if_branch_command(
    ast: &Ast,
    start: usize,
    end: usize,
    word: &str,
) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..end {
        if command_starts_if(&ast.commands[index]) {
            depth += 1;
            continue;
        }
        match ast.commands[index].words.first().map(String::as_str) {
            Some("fi") => depth = depth.saturating_sub(1),
            Some(candidate) if depth == 0 && candidate == word => return Some(index),
            _ => {}
        }
    }
    None
}

pub(super) fn command_starts_if(command: &CommandNode) -> bool {
    matches!(command.words.as_slice(), [first, ..] if first == "if")
        || matches!(
            command.words.as_slice(),
            [first, second, ..]
                if matches!(first.as_str(), "then" | "else" | "do") && second == "if"
        )
}

pub(super) fn command_tail_starts_if(command: &CommandNode, start: usize) -> bool {
    command.words.get(start).is_some_and(|word| word == "if")
}

pub(super) fn command_tail(command: Option<&CommandNode>) -> Option<CommandNode> {
    let command = command?;
    if command.words.len() <= 1 {
        return None;
    }
    command_tail_from(Some(command), 1)
}

pub(super) fn command_tail_from(
    command: Option<&CommandNode>,
    start: usize,
) -> Option<CommandNode> {
    let command = command?;
    if command.words.len() <= start {
        return None;
    }
    let mut tail = command.clone();
    tail.words = tail.words[start..].to_vec();
    Some(tail)
}

pub(super) fn normalize_inline_compound_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut normalized = Vec::new();
    let mut index = 0usize;
    while index < commands.len() {
        if let Some((command, next_index)) = inline_arithmetic_for_command(&commands, index) {
            normalized.push(command);
            index = next_index;
            continue;
        }

        normalized.push(commands[index].clone());
        index += 1;
    }
    normalized
}

fn inline_arithmetic_for_command(
    commands: &[CommandNode],
    index: usize,
) -> Option<(CommandNode, usize)> {
    let command = commands.get(index)?;
    if command.words.first().map(String::as_str) != Some("for") {
        return None;
    }
    if command.words.len() != 4 || command.words.get(2).map(String::as_str) != Some(";;") {
        return None;
    }

    let do_index = index + 1;
    let do_command = commands.get(do_index)?;
    if do_command.words.first().map(String::as_str) != Some("do") {
        return None;
    }
    let done_index = find_matching_done(commands, do_index + 1)?;

    let mut body = Vec::new();
    if let Some(command) = command_tail(commands.get(do_index)) {
        body.push(command);
    }
    body.extend(commands[do_index + 1..done_index].iter().cloned());

    let mut for_node = command.clone();
    for_node.words.clear();
    for_node.word_kinds.clear();
    for_node.for_command = Some(ForCommand {
        variable: String::new(),
        words: Vec::new(),
        default_positional: false,
        arithmetic: Some(ArithmeticForCommand {
            init: command.words[1].clone(),
            test: String::new(),
            update: command.words[3].clone(),
        }),
        body: normalize_inline_compound_commands(body),
    });
    Some((for_node, done_index + 1))
}

fn find_matching_done(commands: &[CommandNode], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..commands.len() {
        match commands[index].words.first().map(String::as_str) {
            Some("for" | "while" | "until") => depth += 1,
            Some("done") if depth == 0 => return Some(index),
            Some("done") => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}
