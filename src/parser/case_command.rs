use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_case_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports nested compound lists and
    // redirections on the compound command. This covers the common
    // `case word in pattern) list terminator` shape.
    let (word, raw_word, mut i) = collect_case_word(tokens, start + 1)?;
    while i < tokens.len() && !is_keyword(tokens, i, "in") {
        i += 1;
    }
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    let in_keyword = tokens[i].value.clone();
    let in_keyword_metadata = build_keyword_metadata(&tokens[i]);
    i += 1;

    let mut clauses = Vec::new();
    while i < tokens.len() {
        while i < tokens.len() && tokens[i].kind == TokenKind::Semicolon {
            i += 1;
        }
        if is_case_end_keyword(tokens, i) {
            break;
        }

        let pattern_open_delimiter = if is_keyword(tokens, i, "(") {
            let delimiter = Some(tokens[i].value.clone());
            i += 1;
            delimiter
        } else {
            None
        };
        let pattern_open_delimiter_metadata = pattern_open_delimiter
            .as_ref()
            .map(|delimiter| Box::new(build_word_metadata(0, delimiter, delimiter)));

        let mut patterns = Vec::new();
        let mut raw_patterns = Vec::new();
        let mut pattern_separators = Vec::new();
        let mut pattern_separator_metadata = Vec::new();
        let mut current_pattern = String::new();
        let mut current_raw_pattern = String::new();
        let mut in_extglob = 0i32;
        while i < tokens.len() {
            // Check if this is a ) that ends the case pattern (not inside extglob)
            if is_keyword(tokens, i, ")") && in_extglob == 0 {
                patterns.push(mark_case_pattern_literal_backslashes(&current_pattern));
                raw_patterns.push(current_raw_pattern.clone());
                current_pattern.clear();
                current_raw_pattern.clear();
                break;
            }
            match tokens[i].kind {
                TokenKind::Word
                | TokenKind::Assignment
                | TokenKind::CommandSubst
                | TokenKind::BraceExpand => {
                    let text = &tokens[i].value;
                    // Check if this word ends with an extglob operator before (
                    if i + 1 < tokens.len()
                        && is_keyword(tokens, i + 1, "(")
                        && ends_with_extglob_operator(text)
                    {
                        // Collect the full extglob pattern
                        let extglob = collect_extglob_pattern(tokens, &mut i);
                        current_pattern.push_str(&extglob);
                        current_raw_pattern.push_str(&extglob);
                    } else {
                        current_pattern.push_str(text);
                        current_raw_pattern.push_str(&tokens[i].raw);
                    }
                }
                TokenKind::Variable => {
                    current_pattern.push_str(&tokens[i].value);
                    current_raw_pattern.push_str(&tokens[i].raw);
                }
                // Handle `!(` as extglob negation pattern
                TokenKind::Keyword
                    if tokens[i].value == "!"
                        && i + 1 < tokens.len()
                        && is_keyword(tokens, i + 1, "(") =>
                {
                    let extglob = collect_extglob_pattern_from_bang(tokens, &mut i);
                    current_pattern.push_str(&extglob);
                    current_raw_pattern.push_str(&extglob);
                }
                TokenKind::Keyword if tokens[i].value == "(" => {
                    current_pattern.push('(');
                    current_raw_pattern.push_str(&tokens[i].raw);
                    in_extglob += 1;
                }
                TokenKind::Keyword if tokens[i].value == ")" => {
                    current_pattern.push(')');
                    current_raw_pattern.push_str(&tokens[i].raw);
                    in_extglob -= 1;
                    if in_extglob < 0 {
                        in_extglob = 0;
                    }
                }
                TokenKind::Keyword => {
                    current_pattern.push_str(&tokens[i].value);
                    current_raw_pattern.push_str(&tokens[i].raw);
                }
                TokenKind::Pipe => {
                    // Pipe separates case patterns (not inside extglob)
                    if in_extglob == 0 {
                        patterns.push(mark_case_pattern_literal_backslashes(&current_pattern));
                        raw_patterns.push(current_raw_pattern.clone());
                        pattern_separator_metadata.push(build_word_metadata(
                            pattern_separators.len(),
                            &tokens[i].value,
                            &tokens[i].raw,
                        ));
                        pattern_separators.push(tokens[i].value.clone());
                        current_pattern.clear();
                        current_raw_pattern.clear();
                    } else {
                        current_pattern.push('|');
                        current_raw_pattern.push_str(&tokens[i].raw);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if !is_keyword(tokens, i, ")") {
            return None;
        }
        let pattern_close_delimiter = tokens[i].value.clone();
        let pattern_close_delimiter_metadata = build_keyword_metadata(&tokens[i]);
        i += 1;

        let body_start = i;
        i = case_body_end(tokens, i);
        let body = parse(&tokens[body_start..i]).commands;
        let terminator_text = case_terminator(tokens, i).map(|_| tokens[i].value.clone());
        let terminator_metadata =
            case_terminator(tokens, i).map(|_| build_keyword_metadata(&tokens[i]));
        let terminator = case_terminator(tokens, i).unwrap_or(CaseTerminator::Break);
        let clause_index = clauses.len();
        let pattern_nodes = case_pattern_nodes(&patterns, &raw_patterns, clause_index);
        clauses.push(CaseClause {
            pattern_open_delimiter,
            pattern_open_delimiter_metadata,
            patterns,
            pattern_separators,
            pattern_separator_metadata,
            pattern_close_delimiter,
            pattern_close_delimiter_metadata,
            pattern_nodes,
            body,
            terminator,
            terminator_text,
            terminator_metadata,
        });

        if is_case_terminator(tokens, i) {
            i += 1;
        }
    }

    if !is_keyword(tokens, i, "esac") {
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.case_command = Some(Box::new(CaseCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata: build_keyword_metadata(&tokens[start]),
        word_metadata: build_word_metadata(0, &word, &raw_word),
        word,
        in_keyword,
        in_keyword_metadata,
        clauses,
        end_keyword: tokens[i].value.clone(),
        end_keyword_metadata: build_keyword_metadata(&tokens[i]),
    }));
    Some(finish_compound_command(command, tokens, i + 1))
}

fn build_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn collect_case_word(tokens: &[Token], index: usize) -> Option<(String, String, usize)> {
    if let Some((word, next_i)) = collect_compound_word_value(tokens, index) {
        let raw = if next_i == index + 1 {
            tokens[index].raw.clone()
        } else {
            word.clone()
        };
        return Some((word, raw, next_i));
    }

    tokens
        .get(index)
        .filter(|token| token.kind == TokenKind::Keyword)
        .map(|token| (token.value.clone(), token.raw.clone(), index + 1))
}

pub(super) fn mark_case_pattern_literal_backslashes(pattern: &str) -> String {
    pattern.replace('\\', "\x18")
}

fn case_pattern_nodes(
    patterns: &[String],
    raw_patterns: &[String],
    clause_index: usize,
) -> Vec<CasePattern> {
    patterns
        .iter()
        .enumerate()
        .map(|(pattern_index, pattern)| {
            let raw_pattern = raw_patterns
                .get(pattern_index)
                .cloned()
                .unwrap_or_else(|| pattern.clone());
            CasePattern::new_with_raw(pattern.clone(), raw_pattern, clause_index, pattern_index)
        })
        .collect()
}

/// Check if a pattern string ends with an extglob operator character before (
pub(super) fn ends_with_extglob_operator(pattern: &str) -> bool {
    pattern
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '@' | '*' | '+' | '?' | '!'))
}

/// Collect a full extglob pattern from tokens, starting at the current position.
/// The current token should be a word ending with an extglob operator (e.g., "foo+"),
/// and the next token should be "(".
/// Returns the complete extglob pattern string.
/// After return, `i` points to the last token consumed (the closing ")").
pub(super) fn collect_extglob_pattern(tokens: &[Token], i: &mut usize) -> String {
    let mut pattern = tokens[*i].value.clone();
    *i += 1;

    // Consume the "("
    if *i < tokens.len() && is_keyword(tokens, *i, "(") {
        pattern.push('(');
        *i += 1;

        // Collect until matching ")"
        let mut depth = 1i32;
        while *i < tokens.len() && depth > 0 {
            match tokens[*i].kind {
                TokenKind::Keyword if tokens[*i].value == "(" => {
                    depth += 1;
                    pattern.push('(');
                }
                TokenKind::Keyword if tokens[*i].value == ")" => {
                    depth -= 1;
                    pattern.push(')');
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Pipe => {
                    pattern.push('|');
                }
                _ => {
                    pattern.push_str(&tokens[*i].value);
                }
            }
            *i += 1;
        }
    }

    pattern
}

/// Collect a full extglob pattern from tokens when the current token is `!` (Keyword).
/// The next token should be "(".
/// Returns the complete extglob pattern string (e.g., "!(a|b)").
/// After return, `i` points to the closing ")" token.
pub(super) fn collect_extglob_pattern_from_bang(tokens: &[Token], i: &mut usize) -> String {
    let mut pattern = "!".to_string();
    *i += 1; // skip the `!`

    // Consume the "("
    if *i < tokens.len() && is_keyword(tokens, *i, "(") {
        pattern.push('(');
        *i += 1;

        // Collect until matching ")"
        let mut depth = 1i32;
        while *i < tokens.len() && depth > 0 {
            match tokens[*i].kind {
                TokenKind::Keyword if tokens[*i].value == "(" => {
                    depth += 1;
                    pattern.push('(');
                }
                TokenKind::Keyword if tokens[*i].value == ")" => {
                    depth -= 1;
                    pattern.push(')');
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Pipe => {
                    pattern.push('|');
                }
                _ => {
                    pattern.push_str(&tokens[*i].value);
                }
            }
            *i += 1;
        }
    }

    pattern
}

pub(super) fn case_body_end(tokens: &[Token], mut index: usize) -> usize {
    let mut stack = Vec::new();
    while index < tokens.len() {
        if stack.is_empty()
            && (is_case_terminator(tokens, index)
                || (command_boundary_keyword_allowed(tokens, index)
                    && is_keyword(tokens, index, "esac")))
        {
            break;
        }

        update_compound_boundary_stack(tokens, index, &mut stack);
        index += 1;
    }

    index
}

pub(super) fn is_case_terminator(tokens: &[Token], index: usize) -> bool {
    case_terminator(tokens, index).is_some()
}

pub(super) fn case_terminator(tokens: &[Token], index: usize) -> Option<CaseTerminator> {
    let token = tokens.get(index)?;
    if token.kind != TokenKind::Word {
        return None;
    }

    match token.raw.as_str() {
        ";;" => Some(CaseTerminator::Break),
        ";&" => Some(CaseTerminator::FallThrough),
        ";;&" => Some(CaseTerminator::TestNext),
        _ => None,
    }
}
