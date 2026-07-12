use super::{parse, CommandNode, CommandSubstitutionNode};

pub(super) fn record_command_substitutions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    let substitutions = command_substitutions_in_word(word)
        .into_iter()
        .map(|mut substitution| {
            substitution.word_index = Some(word_index);
            substitution
        });
    command.command_substitutions.extend(substitutions);
}

pub(super) fn record_command_substitutions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let substitutions = command_substitutions_in_word(value)
        .into_iter()
        .map(|mut substitution| {
            substitution.assignment_name = Some(assignment_name.to_string());
            substitution.word_index = word_index;
            substitution
        });
    command.command_substitutions.extend(substitutions);
}

fn command_substitutions_in_word(word: &str) -> Vec<CommandSubstitutionNode> {
    let chars = word.chars().collect::<Vec<_>>();
    let mut substitutions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$'
            && chars.get(index + 1) == Some(&'(')
            && chars.get(index + 2) != Some(&'(')
        {
            if let Some((substitution, next_index)) = dollar_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '`' {
            if let Some((substitution, next_index)) = backtick_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    substitutions
}

fn dollar_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let mut index = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let text = chars[start..=index].iter().collect();
                    let source = chars[start + 2..index].iter().collect();
                    return Some((command_substitution_node(text, source, false), index + 1));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn backtick_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let mut index = start + 1;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '`' {
            let text = chars[start..=index].iter().collect();
            let source = chars[start + 1..index].iter().collect();
            return Some((command_substitution_node(text, source, true), index + 1));
        }
        index += 1;
    }
    None
}

fn command_substitution_node(
    text: String,
    source: String,
    backtick: bool,
) -> CommandSubstitutionNode {
    let tokens = crate::lexer::tokenize(&source);
    let commands = parse(&tokens).commands;
    let (open_delimiter, close_delimiter) = if backtick {
        ("`".to_string(), "`".to_string())
    } else {
        ("$(".to_string(), ")".to_string())
    };
    CommandSubstitutionNode {
        text,
        open_delimiter,
        source,
        close_delimiter,
        commands,
        backtick,
        word_index: None,
        assignment_name: None,
    }
}
