use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_case_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports nested compound lists and
    // redirections on the compound command. This covers the common
    // `case word in pattern) list terminator` shape.
    let (word, raw_word, mut i) = collect_case_word(tokens, start + 1)?;
    while i < tokens.len() && !is_keyword(tokens, i, "in") {
        // `do`/`then` are command-list reserved words, not material to skip
        // while looking for the case-word separator. Bash reports malformed
        // forms such as `case in do do) ...` during parsing.
        if is_keyword(tokens, i, "do") || is_keyword(tokens, i, "then") {
            return None;
        }
        i += 1;
    }
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    let in_keyword = tokens[i].value.clone();
    let in_keyword_metadata = build_keyword_metadata(&tokens[i]);
    i += 1;

    // GNU parse.y: the token right after `in` is read in the pattern-list
    // state where a bare `esac` terminates an EMPTY case list — `case x in
    // esac` is a complete case command with zero clauses (verified against
    // GNU 5.2.21: it runs the following commands and falls through). The
    // `)` that often follows (`case x in esac)`) is then a separate syntax
    // error reported at the paren. If `esac` is immediately followed by
    // `)`, `|` or `(` it is *not* an empty case but a syntax error (the
    // `)` would be a stray pattern delimiter). Returning None makes the
    // whole `case` fail to parse, so `eval` reports the syntax error and
    // does not execute the following `echo`.
    if is_keyword(tokens, i, "esac") {
        let follows_pattern_delim = tokens
            .get(i + 1)
            .is_some_and(|next| next.value == ")" || next.value == "|" || next.value == "(");
        if follows_pattern_delim {
            // `case x in esac)` and `case esac in esac)` are syntax errors:
            // the bare `esac` after `in` would be an empty case, but the
            // following `)`/`|` makes it a stray pattern delimiter. GNU
            // reports `syntax error near unexpected token ')'` and the
            // entire `case ... esac` fails (eval returns 2, no `echo` runs).
            // Build a parse-error command that spans to the final `esac`
            // so the `echo` is not executed as a separate command.
            let mut final_esac = i;
            for idx in (i..tokens.len()).rev() {
                if is_keyword(tokens, idx, "esac") {
                    final_esac = idx;
                    break;
                }
            }
            let source = raw_token_span(tokens, start, final_esac);
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                "unexpected token `)'".to_string(),
            );
            command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
            return Some(finish_compound_command(command, tokens, final_esac + 1));
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
            clauses: Vec::new(),
            end_keyword: tokens[i].value.clone(),
            end_keyword_metadata: build_keyword_metadata(&tokens[i]),
        }));
        return Some(finish_compound_command(command, tokens, i + 1));
    }

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
        let mut current_pattern_has_token = false;
        let mut in_extglob = 0i32;
        while i < tokens.len() {
            // Check if this is a ) that ends the case pattern (not inside extglob)
            if is_keyword(tokens, i, ")") && in_extglob == 0 {
                if !current_pattern_has_token {
                    return None;
                }
                // Bash does not perform quote removal on case patterns
                // (execute_cmd.c), so raw `\]` / `\"` / `\\` escapes survive.
                patterns.push(current_pattern.clone());
                raw_patterns.push(current_raw_pattern.clone());
                current_pattern.clear();
                current_raw_pattern.clear();
                break;
            }
            // parse.y:1225-1236 pattern_list: `newline_list' is legal only
            // before a clause's first pattern and after a bodyless clause's
            // `)'. Inside the pattern itself — between `|' alternatives, or
            // between the pattern word and its `)' — the grammar admits no
            // `;' or newline, and yacc reports `syntax error near unexpected
            // token `newline'' (`` `;' '' for a real semicolon) at that
            // line (`case a in a;b) ...', `case a in a|<newline> b) ...').
            if in_extglob == 0 && tokens[i].kind == TokenKind::Semicolon {
                let token_text = if tokens[i].line_break { "newline" } else { ";" };
                let mut command = CommandNode::new();
                command.line = tokens.get(start).map(|token| token.position);
                command.insert_assignment(
                    "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                    format!(
                        "{}{}{}",
                        token_text,
                        crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                        tokens[i].position
                    ),
                );
                command.insert_assignment(
                    "__RUBASH_PARSE_SOURCE__".to_string(),
                    pattern_error_line_text(tokens, i),
                );
                return Some(finish_compound_command(command, tokens, tokens.len()));
            }
            if tokens[i].kind != TokenKind::Pipe {
                current_pattern_has_token = true;
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
                        // Keep the quote-free value for the structured AST
                        // metadata, but retain raw text for unquoted escaped
                        // patterns.  The executor uses raw_text when quote
                        // semantics matter, so this does not discard `\]`
                        // or `\"` during matching.
                        let pattern_text =
                            if tokens[i].raw.chars().any(|ch| matches!(ch, '\'' | '"')) {
                                text
                            } else {
                                &tokens[i].raw
                            };
                        current_pattern.push_str(&strip_case_quote_markers(pattern_text));
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
                        patterns.push(current_pattern.clone());
                        raw_patterns.push(current_raw_pattern.clone());
                        pattern_separator_metadata.push(build_word_metadata(
                            pattern_separators.len(),
                            &tokens[i].value,
                            &tokens[i].raw,
                        ));
                        pattern_separators.push(tokens[i].value.clone());
                        current_pattern.clear();
                        current_raw_pattern.clear();
                        current_pattern_has_token = false;
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

fn raw_token_span(tokens: &[Token], start: usize, end: usize) -> String {
    let slice = &tokens[start..=end];
    let mut parts = Vec::with_capacity(slice.len() * 2);
    for (idx, token) in slice.iter().enumerate() {
        if idx > 0 {
            let previous = &slice[idx - 1];
            let previous_end = previous.column + previous.raw.len();
            if token.column > previous_end {
                parts.push(" ".repeat(token.column - previous_end));
            }
        }
        parts.push(token.raw.clone());
    }
    parts.concat()
}

pub(super) fn case_parse_error_message(_tokens: &[Token], _start: usize) -> &'static str {
    "unexpected token `esac'"
}

fn is_case_clause_terminator_token(token: &Token) -> bool {
    token.kind == TokenKind::Word && matches!(token.raw.as_str(), ";;" | ";&" | ";;&")
}

/// Physical input line of `tokens[index]` for a pattern-position error echo,
/// reconstructed from same-line token raws. Synthetic line-break separators
/// carry no source text and are skipped (GNU y.error echoes the line as
/// read, e.g. `y|' — not `y|;').
fn pattern_error_line_text(tokens: &[Token], index: usize) -> String {
    let line = tokens[index].position;
    let mut start = index;
    while start > 0 && tokens[start - 1].position == line {
        start -= 1;
    }
    let mut text = String::new();
    let mut prev_end: Option<usize> = None;
    for token in &tokens[start..] {
        if token.position != line {
            break;
        }
        if token.kind == TokenKind::Semicolon && token.line_break {
            continue;
        }
        if let Some(end) = prev_end {
            if token.column > end {
                text.push(' ');
            }
        }
        text.push_str(&token.raw);
        prev_end = Some(token.column + token.raw.len());
    }
    text
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
        // The lexer protects quoted glob operators with an internal marker so
        // matching can distinguish them from active pattern syntax. That
        // marker is not part of the parser's public structured text.
        return Some((strip_case_quote_markers(&word), raw, next_i));
    }

    tokens
        .get(index)
        .filter(|token| token.kind == TokenKind::Keyword)
        .map(|token| (token.value.clone(), token.raw.clone(), index + 1))
}

fn strip_case_quote_markers(value: &str) -> String {
    value.replace(crate::executor::markers::CTLESC, "")
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

        // A case clause owns a command list, but its top-level compound
        // terminators belong to the surrounding grammar. Bash parse.y rejects
        // these instead of executing them as ordinary commands inside the
        // clause (for example `case x in x) done ;; esac`).
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && matches!(
                tokens[index].value.as_str(),
                "done" | "fi" | "then" | "else" | "elif"
            )
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
