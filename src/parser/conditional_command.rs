use super::*;
use crate::lexer::Token;

pub(super) fn parse_conditional_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    if tokens.get(start)?.value != "[[" {
        return None;
    }

    let end = matching_conditional_end(tokens, start)?;
    let arg_parts = collect_conditional_args(tokens, start + 1, end);
    let args = arg_parts
        .iter()
        .map(|(arg, _)| arg.clone())
        .collect::<Vec<_>>();
    let arg_metadata = arg_parts
        .iter()
        .enumerate()
        .map(|(index, (arg, raw))| build_word_metadata(index, arg, raw))
        .collect::<Vec<_>>();
    let expression_args = arg_parts
        .last()
        .is_some_and(|(arg, _)| arg == "]]")
        .then(|| &arg_parts[..arg_parts.len() - 1])
        .unwrap_or(arg_parts.as_slice());
    let expression = conditional_expression(expression_args);

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.words.push("[[".to_string());
    command.words.extend(args.clone());
    command.conditional_command = Some(Box::new(ConditionalCommand {
        open_delimiter: tokens[start].value.clone(),
        open_delimiter_metadata: token_metadata(&tokens[start]),
        args,
        arg_metadata,
        close_delimiter: tokens[end].value.clone(),
        close_delimiter_metadata: token_metadata(&tokens[end]),
        expression,
    }));

    Some(finish_compound_command(command, tokens, end + 1))
}

fn collect_conditional_args(
    tokens: &[Token],
    mut index: usize,
    end: usize,
) -> Vec<(String, String)> {
    let mut args = Vec::new();
    while index <= end {
        if index == end {
            args.push((tokens[index].value.clone(), tokens[index].raw.clone()));
            break;
        }

        if let Some((word, next_i)) = collect_compound_word_value(tokens, index) {
            let raw = if next_i == index + 1 {
                tokens[index].raw.clone()
            } else {
                word.clone()
            };
            args.push((word, raw));
            index = next_i;
            continue;
        }

        args.push((tokens[index].value.clone(), tokens[index].raw.clone()));
        index += 1;
    }
    args
}

fn matching_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start + 1..tokens.len()).find(|&index| tokens[index].raw == "]]")
}

fn conditional_expression(args: &[(String, String)]) -> ConditionalExpression {
    if args.is_empty() {
        return conditional_leaf(ConditionalExpressionKind::Empty, None, args);
    }

    if let Some(inner) = conditional_outer_group(args) {
        return ConditionalExpression {
            kind: ConditionalExpressionKind::Group,
            open_delimiter: Some("(".to_string()),
            open_delimiter_metadata: Some(delimiter_metadata("(")),
            operator: None,
            operands: Vec::new(),
            pattern_operand: None,
            children: vec![conditional_expression(inner)],
            close_delimiter: Some(")".to_string()),
            close_delimiter_metadata: Some(delimiter_metadata(")")),
        };
    }

    if let Some(index) = top_level_operator(args, "||") {
        return conditional_logical_expression(args, index);
    }
    if let Some(index) = top_level_operator(args, "&&") {
        return conditional_logical_expression(args, index);
    }

    if args[0].0 == "!" {
        return ConditionalExpression {
            kind: ConditionalExpressionKind::Negation,
            open_delimiter: None,
            open_delimiter_metadata: None,
            operator: Some("!".to_string()),
            operands: Vec::new(),
            pattern_operand: None,
            children: vec![conditional_expression(&args[1..])],
            close_delimiter: None,
            close_delimiter_metadata: None,
        };
    }

    if args.len() == 2 && is_conditional_unary_operator(&args[0].0) {
        return conditional_leaf(
            ConditionalExpressionKind::Unary,
            Some(args[0].0.clone()),
            &args[1..2],
        );
    }

    if args.len() == 3 && is_conditional_binary_operator(&args[1].0) {
        return conditional_leaf(
            ConditionalExpressionKind::Binary,
            Some(args[1].0.clone()),
            &[args[0].clone(), args[2].clone()],
        );
    }

    if args.len() > 3
        && is_conditional_binary_operator(&args[1].0)
        && conditional_rhs_fragments_can_join(&args[2..])
    {
        let joined_rhs = (
            args[2..]
                .iter()
                .map(|(arg, _)| arg.as_str())
                .collect::<String>(),
            args[2..]
                .iter()
                .map(|(_, raw)| raw.as_str())
                .collect::<String>(),
        );
        return conditional_leaf(
            ConditionalExpressionKind::Binary,
            Some(args[1].0.clone()),
            &[args[0].clone(), joined_rhs],
        );
    }

    if args.len() == 1 {
        return conditional_leaf(ConditionalExpressionKind::Word, None, &args[0..1]);
    }

    conditional_leaf(ConditionalExpressionKind::Unknown, None, args)
}

fn conditional_rhs_fragments_can_join(rhs: &[(String, String)]) -> bool {
    rhs.len() > 1
        && rhs
            .iter()
            .any(|(arg, _)| matches!(arg.as_str(), "(" | ")" | "|") || arg.contains('('))
}

fn conditional_logical_expression(
    args: &[(String, String)],
    index: usize,
) -> ConditionalExpression {
    ConditionalExpression {
        kind: ConditionalExpressionKind::Logical,
        open_delimiter: None,
        open_delimiter_metadata: None,
        operator: Some(args[index].0.clone()),
        operands: Vec::new(),
        pattern_operand: None,
        children: vec![
            conditional_expression(&args[..index]),
            conditional_expression(&args[index + 1..]),
        ],
        close_delimiter: None,
        close_delimiter_metadata: None,
    }
}

fn conditional_leaf(
    kind: ConditionalExpressionKind,
    operator: Option<String>,
    operands: &[(String, String)],
) -> ConditionalExpression {
    ConditionalExpression {
        kind,
        open_delimiter: None,
        open_delimiter_metadata: None,
        pattern_operand: conditional_pattern_operand(operator.as_deref(), operands),
        operator,
        operands: operands.iter().map(|(arg, _)| arg.clone()).collect(),
        children: Vec::new(),
        close_delimiter: None,
        close_delimiter_metadata: None,
    }
}

fn token_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

fn conditional_pattern_operand(
    operator: Option<&str>,
    operands: &[(String, String)],
) -> Option<ConditionalPatternOperand> {
    let rhs = operands.get(1)?;
    let kind = match operator? {
        "=" | "==" | "!=" => ConditionalPatternKind::Glob,
        "=~" => ConditionalPatternKind::Regex,
        _ => return None,
    };
    Some(ConditionalPatternOperand::new_with_raw(
        rhs.0.clone(),
        rhs.1.clone(),
        kind,
    ))
}

fn top_level_operator(args: &[(String, String)], operator: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        match arg.0.as_str() {
            "(" => depth += 1,
            ")" => depth = depth.saturating_sub(1),
            _ if depth == 0 && arg.0 == operator => return Some(index),
            _ => {}
        }
    }
    None
}

fn conditional_outer_group(args: &[(String, String)]) -> Option<&[(String, String)]> {
    if args.first().map(|(arg, _)| arg.as_str()) != Some("(")
        || args.last().map(|(arg, _)| arg.as_str()) != Some(")")
    {
        return None;
    }

    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        match arg.0.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index != args.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }

    (depth == 0).then_some(&args[1..args.len() - 1])
}

fn is_conditional_unary_operator(op: &str) -> bool {
    matches!(
        op,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-G"
            | "-h"
            | "-k"
            | "-L"
            | "-n"
            | "-O"
            | "-o"
            | "-p"
            | "-R"
            | "-r"
            | "-S"
            | "-s"
            | "-t"
            | "-u"
            | "-N"
            | "-v"
            | "-w"
            | "-x"
            | "-z"
    )
}

fn is_conditional_binary_operator(op: &str) -> bool {
    matches!(
        op,
        "=" | "=="
            | "!="
            | "=~"
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-ef"
            | "-nt"
            | "-ot"
    )
}
