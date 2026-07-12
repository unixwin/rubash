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
    let args = tokens[start + 1..=end]
        .iter()
        .map(|token| token.value.clone())
        .collect::<Vec<_>>();
    let expression_args = args
        .strip_suffix(&["]]".to_string()])
        .unwrap_or(args.as_slice());
    let expression = conditional_expression(expression_args);

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.words.push("[[".to_string());
    command.words.extend(args.clone());
    command.conditional_command = Some(Box::new(ConditionalCommand {
        open_delimiter: tokens[start].value.clone(),
        args,
        close_delimiter: tokens[end].value.clone(),
        expression,
    }));

    Some(finish_compound_command(command, tokens, end + 1))
}

fn matching_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start + 1..tokens.len()).find(|&index| tokens[index].value == "]]")
}

fn conditional_expression(args: &[String]) -> ConditionalExpression {
    if args.is_empty() {
        return conditional_leaf(ConditionalExpressionKind::Empty, None, args);
    }

    if let Some(inner) = conditional_outer_group(args) {
        return ConditionalExpression {
            kind: ConditionalExpressionKind::Group,
            open_delimiter: Some("(".to_string()),
            operator: None,
            operands: Vec::new(),
            children: vec![conditional_expression(inner)],
            close_delimiter: Some(")".to_string()),
        };
    }

    if let Some(index) = top_level_operator(args, "||") {
        return conditional_logical_expression(args, index);
    }
    if let Some(index) = top_level_operator(args, "&&") {
        return conditional_logical_expression(args, index);
    }

    if args[0] == "!" {
        return ConditionalExpression {
            kind: ConditionalExpressionKind::Negation,
            open_delimiter: None,
            operator: Some("!".to_string()),
            operands: Vec::new(),
            children: vec![conditional_expression(&args[1..])],
            close_delimiter: None,
        };
    }

    match args {
        [op, operand] if is_conditional_unary_operator(op) => conditional_leaf(
            ConditionalExpressionKind::Unary,
            Some(op.clone()),
            std::slice::from_ref(operand),
        ),
        [left, op, right] if is_conditional_binary_operator(op) => conditional_leaf(
            ConditionalExpressionKind::Binary,
            Some(op.clone()),
            &[left.clone(), right.clone()],
        ),
        [word] => conditional_leaf(
            ConditionalExpressionKind::Word,
            None,
            std::slice::from_ref(word),
        ),
        _ => conditional_leaf(ConditionalExpressionKind::Unknown, None, args),
    }
}

fn conditional_logical_expression(args: &[String], index: usize) -> ConditionalExpression {
    ConditionalExpression {
        kind: ConditionalExpressionKind::Logical,
        open_delimiter: None,
        operator: Some(args[index].clone()),
        operands: Vec::new(),
        children: vec![
            conditional_expression(&args[..index]),
            conditional_expression(&args[index + 1..]),
        ],
        close_delimiter: None,
    }
}

fn conditional_leaf(
    kind: ConditionalExpressionKind,
    operator: Option<String>,
    operands: &[String],
) -> ConditionalExpression {
    ConditionalExpression {
        kind,
        open_delimiter: None,
        operator,
        operands: operands.to_vec(),
        children: Vec::new(),
        close_delimiter: None,
    }
}

fn top_level_operator(args: &[String], operator: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "(" => depth += 1,
            ")" => depth = depth.saturating_sub(1),
            _ if depth == 0 && arg == operator => return Some(index),
            _ => {}
        }
    }
    None
}

fn conditional_outer_group(args: &[String]) -> Option<&[String]> {
    if args.first().map(String::as_str) != Some("(") || args.last().map(String::as_str) != Some(")")
    {
        return None;
    }

    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        match arg.as_str() {
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
            | "-h"
            | "-k"
            | "-L"
            | "-n"
            | "-O"
            | "-o"
            | "-p"
            | "-r"
            | "-S"
            | "-s"
            | "-t"
            | "-u"
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
