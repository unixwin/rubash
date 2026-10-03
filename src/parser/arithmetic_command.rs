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
        command.logical_line = tokens.get(start).map(|token| token.logical_line);
        // GNU parse.y:4976-4982 parse_arith_cmd keeps the matched-pair body
        // verbatim; the expression is whitespace-insensitive (expr.c), so
        // trim only the surrounding blanks for the structured word.
        set_arithmetic_command_words(&mut command, inner.trim().to_string(), raw_inner);
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
            command.logical_line = tokens.get(start).map(|token| token.logical_line);
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
            command.logical_line = tokens.get(start).map(|token| token.logical_line);
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
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
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
    // rubash#413: the verdict is GNU's CHARACTER-level parse_matched_pair
    // scan (parse.y:4970 parse_arith_cmd → parse.y:3906), which counts every
    // unquoted `(`/`)` in the raw text — including parens the word layer
    // folded inside a `name=(...)` compound-assignment body. A token-level
    // paren count diverges exactly there: `(( x=([))] ))` folds
    // `x=([))] )` into one assignment word, hides the `)` that GNU's scan
    // counts (closing the group right before `]`), and misreads the
    // construct as arithmetic. Rebuild the character stream from the token
    // raws (the same reconstruction arithmetic_raw_slice performs) and scan
    // it with parse_matched_pair's P_ARITH rules: backslashes escape, quote
    // spans and `$(`/`${`/`$[` units are opaque (parse.y:4138-4186
    // parse_dollar_word), every other paren shifts the depth.
    let mut text = String::new();
    let body_start = if combined_open {
        // The combined `((...))` token's own raw already ends at `))`; the
        // verdict only matters for the split `(` `(` form, but keep the
        // combined shape sound: scan inside its raw body.
        if let Some(raw) = tokens.get(start).map(|token| token.raw.as_str()) {
            text.push_str(raw.strip_prefix("((").unwrap_or(raw));
        }
        start + 1
    } else {
        start + 2
    };
    for token in tokens.iter().skip(body_start) {
        text.push_str(&token.leading_ws);
        if token.kind == TokenKind::Semicolon && token.line_break {
            // A physical line break folded into a `;` token is a newline in
            // the char stream (parse_matched_pair reads across lines).
            text.push('\n');
        } else {
            text.push_str(&token.raw);
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 1usize;
    let mut scan = 0usize;
    while scan < chars.len() {
        match chars[scan] {
            '\\' => scan += 1,
            '\'' => {
                scan += 1;
                while scan < chars.len() && chars[scan] != '\'' {
                    scan += 1;
                }
            }
            '"' => {
                scan += 1;
                while scan < chars.len() {
                    if chars[scan] == '\\' {
                        scan += 2;
                        continue;
                    }
                    if chars[scan] == '"' {
                        break;
                    }
                    scan += 1;
                }
            }
            '`' => {
                scan += 1;
                while scan < chars.len() {
                    if chars[scan] == '\\' {
                        scan += 2;
                        continue;
                    }
                    if chars[scan] == '`' {
                        break;
                    }
                    scan += 1;
                }
            }
            '$' if matches!(chars.get(scan + 1), Some('(' | '{' | '[')) => {
                let (open, close) = match chars[scan + 1] {
                    '(' => ('(', ')'),
                    '{' => ('{', '}'),
                    _ => ('[', ']'),
                };
                // Nested dollar-word: consumed as a unit by parse_dollar_word
                // (parse.y:4146-4186) — its interior never shifts this
                // group's depth. If it never closes, neither does the group.
                match crate::lexer::dollar_word_group_len(&chars, scan + 1, open, close) {
                    Some(end) => scan = end,
                    None => return false,
                }
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    // parse.y:4976: the character immediately after the
                    // matched group decides — `)` means arithmetic, anything
                    // else the nested-subshell reinterpretation.
                    return chars.get(scan + 1) == Some(&')');
                }
            }
            _ => {}
        }
        scan += 1;
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
        if token.kind == TokenKind::Semicolon && token.line_break {
            // GNU parse.y:3459 read_token / parse_dparen (parse.y:3517+)
            // consume the arithmetic body through parse_matched_pair across
            // physical lines, and expr.c's lexer treats '\n' as whitespace —
            // a newline inside `(( ... ))` is never a command separator.
            // The line-oriented tokenizer folds the physical line break into
            // a `;` token; restore the newline in the verbatim capture so a
            // cross-line `if (( a > maj\n|| ... ))` still evaluates and the
            // diagnostic shows the newline, not `;` (rubash#174). A literal
            // `;` (no line_break) is data and stays.
            raw.push('\n');
        } else {
            raw.push_str(&token.raw);
        }
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
/// unmatched `(`/`)` grouping here so malformed input gets Bash's parse
/// status (2), instead of being treated as a valid command that merely
/// evaluates to an arithmetic error (status 1). Brackets are NOT part of
/// this check: GNU's P_ARITH matched-pair scan counts parens only
/// (parse.y:4970 via parse_matched_pair — a `[` inside `((...))` is plain
/// data; no P_ARRAYSUB scan runs), so `((a[b))` parses and the EVALUATOR
/// reports the operand/subscript error with status 1 (probes k1/k2,
/// rubash#390 q1: `((x=[y))` → `((: x=[y: arithmetic syntax error:
/// operand expected (error token is "[y")` rc 1).
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
            ')' if stack.pop() != Some('(') => return false,
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
    // rubash#131: a syntax error inside a compound body fails the whole
    // compound in GNU's grammar; surface the inner marker on the compound
    // itself so the reader reports it at this command (see
    // support.rs propagate_subtree_parse_error).
    super::support::propagate_subtree_parse_error(&mut command);
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
