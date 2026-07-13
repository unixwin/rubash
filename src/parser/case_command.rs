use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_case_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports extglob patterns, nested
    // compound lists, and redirections on the compound command. This covers the
    // common `case word in pattern) list terminator` shape.
    let word = tokens.get(start + 1)?.value.clone();
    let mut i = start + 2;
    while i < tokens.len() && !is_keyword(tokens, i, "in") {
        i += 1;
    }
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    let in_keyword = tokens[i].value.clone();
    i += 1;

    let mut clauses = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, "esac") {
        while i < tokens.len() && tokens[i].kind == TokenKind::Semicolon {
            i += 1;
        }
        if is_keyword(tokens, i, "esac") {
            break;
        }

        let pattern_open_delimiter = if is_keyword(tokens, i, "(") {
            let delimiter = Some(tokens[i].value.clone());
            i += 1;
            delimiter
        } else {
            None
        };

        let mut patterns = Vec::new();
        let mut pattern_separators = Vec::new();
        let mut current_pattern = String::new();
        let mut in_extglob = 0i32;
        while i < tokens.len() {
            // Check if this is a ) that ends the case pattern (not inside extglob)
            if is_keyword(tokens, i, ")") && in_extglob == 0 {
                patterns.push(mark_case_pattern_literal_backslashes(&current_pattern));
                current_pattern.clear();
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
                    } else {
                        current_pattern.push_str(text);
                    }
                }
                TokenKind::Variable => {
                    current_pattern.push_str(&tokens[i].value);
                }
                // Handle `!(` as extglob negation pattern
                TokenKind::Keyword
                    if tokens[i].value == "!"
                        && i + 1 < tokens.len()
                        && is_keyword(tokens, i + 1, "(") =>
                {
                    let extglob = collect_extglob_pattern_from_bang(tokens, &mut i);
                    current_pattern.push_str(&extglob);
                }
                TokenKind::Keyword if tokens[i].value == "(" => {
                    current_pattern.push('(');
                    in_extglob += 1;
                }
                TokenKind::Keyword if tokens[i].value == ")" => {
                    current_pattern.push(')');
                    in_extglob -= 1;
                    if in_extglob < 0 {
                        in_extglob = 0;
                    }
                }
                TokenKind::Pipe => {
                    // Pipe separates case patterns (not inside extglob)
                    if in_extglob == 0 {
                        patterns.push(mark_case_pattern_literal_backslashes(&current_pattern));
                        pattern_separators.push(tokens[i].value.clone());
                        current_pattern.clear();
                    } else {
                        current_pattern.push('|');
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
        i += 1;

        let body_start = i;
        i = case_body_end(tokens, i);
        let body = parse(&tokens[body_start..i]).commands;
        let terminator_text = case_terminator(tokens, i).map(|_| tokens[i].value.clone());
        let terminator = case_terminator(tokens, i).unwrap_or(CaseTerminator::Break);
        let clause_index = clauses.len();
        let pattern_nodes = case_pattern_nodes(&patterns, clause_index);
        clauses.push(CaseClause {
            pattern_open_delimiter,
            patterns,
            pattern_separators,
            pattern_close_delimiter,
            pattern_nodes,
            body,
            terminator,
            terminator_text,
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
        word,
        in_keyword,
        clauses,
        end_keyword: tokens[i].value.clone(),
    }));
    Some(finish_compound_command(command, tokens, i + 1))
}

pub(super) fn mark_case_pattern_literal_backslashes(pattern: &str) -> String {
    pattern.replace('\\', "\x18")
}

fn case_pattern_nodes(patterns: &[String], clause_index: usize) -> Vec<CasePattern> {
    patterns
        .iter()
        .enumerate()
        .map(|(pattern_index, pattern)| {
            CasePattern::new(pattern.clone(), clause_index, pattern_index)
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
    let mut nested_case_depth = 0usize;
    let mut nested_compound_depth = 0usize;

    while index < tokens.len() {
        if is_keyword(tokens, index, "case") {
            nested_case_depth += 1;
            index += 1;
            continue;
        }

        if is_keyword(tokens, index, "esac") {
            if nested_case_depth == 0 {
                break;
            }
            nested_case_depth -= 1;
            index += 1;
            continue;
        }

        if is_keyword(tokens, index, "if")
            || is_keyword(tokens, index, "for")
            || is_keyword(tokens, index, "select")
            || is_keyword(tokens, index, "while")
            || is_keyword(tokens, index, "until")
        {
            nested_compound_depth += 1;
            index += 1;
            continue;
        }

        if is_keyword(tokens, index, "fi") || is_keyword(tokens, index, "done") {
            nested_compound_depth = nested_compound_depth.saturating_sub(1);
            index += 1;
            continue;
        }

        if nested_case_depth == 0 && nested_compound_depth == 0 && is_case_terminator(tokens, index)
        {
            break;
        }

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

    match token.value.as_str() {
        ";;" => Some(CaseTerminator::Break),
        ";&" => Some(CaseTerminator::FallThrough),
        ";;&" => Some(CaseTerminator::TestNext),
        _ => None,
    }
}
