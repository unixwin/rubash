use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_arithmetic_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let first = tokens.get(start)?.value.as_str();

    if let Some(inner) = first
        .strip_prefix("((")
        .and_then(|value| value.strip_suffix("))"))
    {
        let raw_inner = tokens[start]
            .raw
            .strip_prefix("((")
            .and_then(|value| value.strip_suffix("))"))
            .map(str::to_string);
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        set_arithmetic_command_words(&mut command, inner.to_string(), raw_inner);
        return Some(finish_arithmetic_command(command, tokens, start + 1));
    }

    let mut i;
    let open_end;
    let mut parts = Vec::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    if first == "((" {
        if !dparen_lexically_arithmetic(tokens, start) {
            return None;
        }
        i = start + 1;
        open_end = start + 1;
    } else if is_keyword(tokens, start, "(")
        && is_keyword(tokens, start + 1, "(")
        && tokens[start + 1].column == tokens[start].column + tokens[start].raw.len()
    {
        if !dparen_lexically_arithmetic(tokens, start) {
            return None;
        }
        i = start + 2;
        open_end = start + 2;
    } else {
        return None;
    }

    while i + 1 < tokens.len() {
        if paren_depth == 0 && bracket_depth == 0 && tokens[i].value == "))" {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            let raw = arithmetic_raw_slice(tokens, open_end, Some(i));
            set_arithmetic_command_words(&mut command, parts.join(" "), Some(raw));
            return Some(finish_arithmetic_command(command, tokens, i + 1));
        }

        if paren_depth == 0
            && bracket_depth == 0
            && is_keyword(tokens, i, ")")
            && is_keyword(tokens, i + 1, ")")
        {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            let raw = arithmetic_raw_slice(tokens, open_end, Some(i));
            set_arithmetic_command_words(&mut command, parts.join(" "), Some(raw));
            return Some(finish_arithmetic_command(command, tokens, i + 2));
        }

        if tokens[i].value == "[" {
            bracket_depth += 1;
            parts.push(arithmetic_token_value(&tokens[i]));
            i += 1;
            continue;
        }

        if tokens[i].value == "]" && bracket_depth > 0 {
            bracket_depth -= 1;
            parts.push(arithmetic_token_value(&tokens[i]));
            i += 1;
            continue;
        }

        if bracket_depth == 0 && is_keyword(tokens, i, "(") {
            paren_depth += 1;
            parts.push(arithmetic_token_value(&tokens[i]));
            i += 1;
            continue;
        }

        if bracket_depth == 0 && is_keyword(tokens, i, ")") && paren_depth > 0 {
            paren_depth -= 1;
            parts.push(arithmetic_token_value(&tokens[i]));
            i += 1;
            continue;
        }

        if let Some(combined) = arithmetic_combined_operator(&tokens[i], tokens.get(i + 1)) {
            parts.push(combined);
            i += 2;
            continue;
        }

        if tokens[i].kind == TokenKind::Semicolon {
            i += 1;
            continue;
        }

        parts.push(arithmetic_token_value(&tokens[i]));
        i += 1;
    }

    if parts.is_empty() {
        return None;
    }

    while i < tokens.len() && tokens[i].kind != TokenKind::Semicolon {
        parts.push(arithmetic_token_value(&tokens[i]));
        i += 1;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    let raw = arithmetic_raw_slice(tokens, open_end, None);
    set_arithmetic_command_words(&mut command, parts.join(" "), Some(raw));
    Some(finish_arithmetic_command(command, tokens, i))
}

// GNU parse.y parse_dparen: after `((`, bash matches the balanced parenthesis
// group opened by the second `(` (parse_matched_pair with P_ARITH) and then
// reads the next character.  The construct is an arithmetic command only when
// that character is `)`; otherwise the double parenthesis is a nested
// subshell and bash re-lexes from the outer `(`.  `start` addresses either a
// combined `((` token or an adjacent `(` `(` pair.
pub(super) fn dparen_lexically_arithmetic(tokens: &[Token], start: usize) -> bool {
    let combined_open = tokens.get(start).is_some_and(|token| token.value == "((");
    let mut depth = 1usize;
    let mut i = if combined_open { start + 1 } else { start + 2 };
    while i < tokens.len() {
        if super::is_unquoted_operator(&tokens[i], "(") {
            depth += 1;
        } else if super::is_unquoted_operator(&tokens[i], ")") {
            depth -= 1;
            if depth == 0 {
                return tokens
                    .get(i + 1)
                    .is_some_and(|token| super::is_unquoted_operator(token, ")"));
            }
        } else {
            match tokens[i].value.as_str() {
                "))" if depth == 1 => {
                    // The token closes this group and the following `)` is the
                    // next character: arithmetic, like GNU reading `))` here.
                    return true;
                }
                _ => {}
            }
        }
        i += 1;
    }
    false
}

fn arithmetic_token_value(token: &Token) -> String {
    // Arithmetic parsing removes shell quotes from token values. Preserve
    // single quotes in the command expression so the evaluator can reject
    // `(( '1' ))` like Bash instead of silently treating it as 1.
    if token.raw.contains(['\'', '"']) {
        token.raw.clone()
    } else {
        token.value.clone()
    }
}

/// Verbatim source text between the arithmetic command delimiters: each
/// inner token contributes its leading whitespace plus its raw text, and
/// the whitespace before the closing delimiter completes the slice. This
/// mirrors parse.y::parse_arith_cmd, which captures the bytes between
/// `((` and `))` untouched, so trailing blanks before `))` survive into
/// arithmetic diagnostics exactly like GNU bash.
fn arithmetic_raw_slice(tokens: &[Token], open_end: usize, close_index: Option<usize>) -> String {
    let end = close_index.unwrap_or(tokens.len()).min(tokens.len());
    let mut raw = String::new();
    for token in &tokens[open_end.min(end)..end] {
        raw.push_str(&token.leading_ws);
        raw.push_str(&token.raw);
    }
    if let Some(closer) = tokens.get(end) {
        raw.push_str(&closer.leading_ws);
    }
    raw
}

fn set_arithmetic_command_words(
    command: &mut CommandNode,
    expression: String,
    raw_expression: Option<String>,
) {
    let delimiters_balanced = arithmetic_delimiters_balanced(&expression);
    command.words.push("((".to_string());
    command.words.push(expression.clone());
    command.words.push("))".to_string());
    let mut arithmetic = arithmetic_command(expression);
    arithmetic.raw_expression = raw_expression.filter(|raw| !raw.is_empty());
    command.arithmetic_command = Some(arithmetic);
    if !delimiters_balanced {
        command.insert_assignment(
            "__RUBASH_PARSE_ERROR__".to_string(),
            "unexpected EOF while looking for matching `)'".to_string(),
        );
    }
}

/// Arithmetic commands are parsed before arithmetic evaluation. Reject an
/// unmatched grouping delimiter here so malformed input gets Bash's parse
/// status (2), instead of being treated as a valid command that merely
/// evaluates to an arithmetic error (status 1).
fn arithmetic_delimiters_balanced(expression: &str) -> bool {
    let mut stack = Vec::new();
    let mut escaped = false;
    let mut quote = None;

    for ch in expression.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('"') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }

        match ch {
            '(' => stack.push(ch),
            '[' => stack.push(ch),
            ')' if stack.pop() != Some('(') => return false,
            ']' if stack.pop() != Some('[') => return false,
            _ => {}
        }
    }

    quote.is_none() && stack.is_empty()
}

fn arithmetic_command(expression: String) -> ArithmeticCommand {
    let operators = arithmetic_operators(&expression);
    let variables = arithmetic_variables(&expression);
    ArithmeticCommand {
        open_delimiter: "((".to_string(),
        open_delimiter_metadata: delimiter_metadata("(("),
        expression,
        raw_expression: None,
        close_delimiter: "))".to_string(),
        close_delimiter_metadata: delimiter_metadata("))"),
        variables,
        has_assignment: operators
            .iter()
            .any(|operator| is_arithmetic_assignment_operator(&operator.text)),
        has_comparison: operators
            .iter()
            .any(|operator| is_arithmetic_comparison_operator(&operator.text)),
        has_logical: operators
            .iter()
            .any(|operator| matches!(operator.text.as_str(), "&&" | "||" | "!")),
        has_update: operators
            .iter()
            .any(|operator| matches!(operator.text.as_str(), "++" | "--")),
        operators,
    }
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(WordMetadata::new(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}

pub(super) fn arithmetic_operators(expression: &str) -> Vec<ArithmeticOperator> {
    const OPERATORS: &[&str] = &[
        "<<=", ">>=", "**=", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "&&",
        "||", "==", "!=", "<=", ">=", "<<", ">>", "**", "=", "<", ">", "&", "|", "^", "%", "/",
        "*", "+", "-", "!", "~", "?", ":", ",",
    ];

    let mut operators = Vec::new();
    let mut index = 0;
    while index < expression.len() {
        let rest = &expression[index..];
        if let Some(operator) = OPERATORS
            .iter()
            .find(|operator| rest.starts_with(**operator))
        {
            operators.push(ArithmeticOperator {
                text: (*operator).to_string(),
                index,
            });
            index += operator.len();
        } else {
            index += rest.chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    operators
}

pub(super) fn arithmetic_variables(expression: &str) -> Vec<String> {
    let chars = expression.char_indices().collect::<Vec<_>>();
    let mut variables = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, ch) = chars[i];
        if !is_arithmetic_identifier_start(ch) {
            i += 1;
            continue;
        }

        let mut end = start + ch.len_utf8();
        i += 1;
        while let Some((index, next)) = chars.get(i).copied() {
            if !is_arithmetic_identifier_continue(next) {
                break;
            }
            end = index + next.len_utf8();
            i += 1;
        }

        let name = expression[start..end].to_string();
        if !variables.contains(&name) {
            variables.push(name);
        }
    }
    variables
}

fn is_arithmetic_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_arithmetic_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

pub(super) fn is_arithmetic_assignment_operator(operator: &str) -> bool {
    matches!(
        operator,
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "**="
    )
}

pub(super) fn is_arithmetic_comparison_operator(operator: &str) -> bool {
    matches!(operator, "==" | "!=" | "<" | ">" | "<=" | ">=")
}

pub(super) fn finish_arithmetic_command(
    mut command: CommandNode,
    tokens: &[Token],
    mut index: usize,
) -> (CommandNode, usize) {
    collect_trailing_redirections(tokens, &mut index, &mut command);
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Pipe) => {
            command.pipe = Some(1);
            (command, index + 1)
        }
        Some(TokenKind::PipeErr) => {
            command.pipe = Some(2);
            (command, index + 1)
        }
        Some(TokenKind::And) => {
            command.and_or = Some(true);
            (command, index + 1)
        }
        Some(TokenKind::Or) => {
            command.and_or = Some(false);
            (command, index + 1)
        }
        Some(TokenKind::Background) => {
            command.background = true;
            (command, index + 1)
        }
        Some(TokenKind::Semicolon) => (command, index + 1),
        _ => (command, index),
    }
}

pub(super) fn finish_compound_command(
    mut command: CommandNode,
    tokens: &[Token],
    mut index: usize,
) -> (CommandNode, usize) {
    collect_trailing_redirections(tokens, &mut index, &mut command);
    // GNU sets each top-level command's ambient line_number from where its
    // parse ended — the last token consumed by the command itself (closing
    // keyword or trailing redirect target), before the list terminator.
    command.end_line = index
        .checked_sub(1)
        .and_then(|i| tokens.get(i))
        .map(|token| token.position)
        .or(command.line);
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Pipe) => {
            command.pipe = Some(1);
            (command, index + 1)
        }
        Some(TokenKind::PipeErr) => {
            command.pipe = Some(2);
            (command, index + 1)
        }
        Some(TokenKind::And) => {
            command.and_or = Some(true);
            (command, index + 1)
        }
        Some(TokenKind::Or) => {
            command.and_or = Some(false);
            (command, index + 1)
        }
        Some(TokenKind::Background) => {
            command.background = true;
            (command, index + 1)
        }
        Some(TokenKind::Semicolon) => (command, index + 1),
        _ => (command, index),
    }
}

pub(super) fn arithmetic_combined_operator(token: &Token, next: Option<&Token>) -> Option<String> {
    let op = token.value.as_str();
    if !matches!(op, ">" | "<" | "!" | "&" | "|" | "<<" | ">>") {
        return None;
    }

    let next = next?;
    if next.value == "=" {
        return Some(format!("{op}="));
    }

    next.value
        .strip_prefix('=')
        .map(|rhs| format!("{op}={rhs}"))
}
