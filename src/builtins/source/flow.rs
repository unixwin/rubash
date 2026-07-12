use crate::parser::{ArithmeticForCommand, CommandBodyKind, CommandNode, ForCommand};

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

pub(crate) fn normalize_inline_compound_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
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
        keyword: "for".to_string(),
        variable: String::new(),
        in_keyword: None,
        words: Vec::new(),
        default_positional: false,
        list_terminator: None,
        arithmetic: Some(ArithmeticForCommand {
            open_delimiter: "((".to_string(),
            init: command.words[1].clone(),
            separators: vec![";".to_string(), ";".to_string()],
            test: String::new(),
            update: command.words[3].clone(),
            close_delimiter: "))".to_string(),
        }),
        body_kind: CommandBodyKind::DoDone,
        do_keyword: Some("do".to_string()),
        end_keyword: Some("done".to_string()),
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
