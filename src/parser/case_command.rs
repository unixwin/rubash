use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_case_command(
    tokens: &[Token],
    start: usize,
    source: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports nested compound lists and
    // redirections on the compound command. This covers the common
    // `case word in pattern) list terminator` shape.
    let (word, raw_word, mut i) = collect_case_word(tokens, start + 1)?;
    // GNU parse.y case_command (grammar: CASE WORD newline_list IN):
    // after the subject word the grammar admits only a newline_list
    // before IN, and yyerror (parse.y:6727 -> report_syntax_error,
    // parse.y:6833) names the token that actually arrived where IN was
    // expected. The old walk skipped arbitrary tokens hunting for a
    // later `in', so the failure surfaced at the resync `esac' with no
    // source echo (rubash#2740, #2679 c24): `case in esac' names
    // `esac', `case abc abc)' names the second `abc', `case x; in'
    // names `;'. End of input in this state is the yyerror EOF path
    // (parse.y:6890-6901): "unexpected end of file from `case'
    // command on line N".
    while i < tokens.len() && tokens[i].kind == TokenKind::Semicolon && tokens[i].line_break {
        i += 1;
    }
    if !is_keyword(tokens, i, "in") {
        match tokens.get(i) {
            None => {
                let command = super::parse_loop::unclosed_keyword_eof_node(
                    tokens,
                    start,
                    "case",
                    source.as_deref(),
                    source_line_offset,
                );
                return Some((command, tokens.len()));
            }
            Some(_) => {
                return Some(case_near_error_node(tokens, start, i));
            }
        }
    }
    let in_keyword = tokens[i].value.clone();
    let in_keyword_metadata = build_keyword_metadata(&tokens[i]);
    i += 1;

    // GNU parse.y:1037 grammar: `case WORD in newline_list ...' — the
    // newline_list between `in` and the first pattern (or the empty case's
    // `esac`) is legal, so the empty-case decision must look PAST it:
    // `case x in\nesac) echo hi;;\nesac` is the same stray-`)` error as the
    // one-line form, reported at the `)`'s physical line (rubash#381;
    // verified vs WSL GNU 5.3.0: `syntax error near unexpected token `)'`
    // citing the `esac) echo hi;;` line, not the final `esac` line).
    let mut case_head = i;
    while case_head < tokens.len()
        && tokens[case_head].kind == TokenKind::Semicolon
        && tokens[case_head].line_break
    {
        case_head += 1;
    }

    // GNU parse.y: the token right after `in` (past the newline_list) is
    // read in the pattern-list state where a bare `esac` terminates an
    // EMPTY case list — `case x in esac` is a complete case command with
    // zero clauses (verified against GNU 5.2.21: it runs the following
    // commands and falls through). The `)` that often follows (`case x in
    // esac)`) is then a separate syntax error reported at the paren. If
    // `esac` is immediately followed by `)`, `|` or `(` it is *not* an
    // empty case but a syntax error (the `)` would be a stray pattern
    // delimiter). Returning None makes the whole `case` fail to parse, so
    // `eval` reports the syntax error and does not execute the following
    // `echo`.
    if is_keyword(tokens, case_head, "esac") {
        // GNU parse.y:3177-3187 CHECK_FOR_RESERVED_WORD + 3433-3441
        // special_case_tokens: the `esac' after `in' (+ the grammar's
        // newline_list, parse.y:1037) is the ESAC of the EMPTY case — the
        // case command ends HERE (esacs_needed_count--, PST_CASEPAT
        // cleared). Whatever follows belongs to the OUTER grammar, which
        // owns the diagnostic: a `)' at top level is the stray-`)' syntax
        // error at that token, inside `( ... )` the subshell absorbs it and
        // the error surfaces at the NEXT command-start token, and `| fi)'
        // names the reserved word (rubash#390 q3/q4 — the pre-emptive
        // "error at the `)` right after esac" guard produced GNU's
        // top-level diagnostic even when an enclosing construct legally
        // consumed the `)`).
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.logical_line = tokens.get(start).map(|token| token.logical_line);
        command.case_command = Some(Box::new(CaseCommand {
            keyword: tokens[start].value.clone(),
            keyword_metadata: build_keyword_metadata(&tokens[start]),
            word_metadata: build_word_metadata(0, &word, &raw_word),
            word,
            in_keyword,
            in_keyword_metadata,
            clauses: Vec::new(),
            end_keyword: tokens[case_head].value.clone(),
            end_keyword_metadata: build_keyword_metadata(&tokens[case_head]),
        }));
        return Some(finish_compound_command(command, tokens, case_head + 1));
    }

    let mut clauses = Vec::new();
    while i < tokens.len() {
        while i < tokens.len() && tokens[i].kind == TokenKind::Semicolon {
            i += 1;
        }
        // A clause terminator's `;;' re-enters the pattern-list state (GNU
        // parse.y:3710 sets PST_CASEPAT), so the empty-case rule applies
        // BETWEEN clauses too: `esac' closes the case (parse.y:3185-3187 —
        // not preceded by `|'/`(' — esacs_needed_count--, PST_CASEPAT
        // cleared) and the outer grammar owns the diagnostic, exactly like
        // the after-`in' form (rubash#381 rejection, rubash#390 naming:
        // `case x in a) :;; esac ) echo' names the stray `)' at top level,
        // `( case x in a) :;; esac ) echo hi' lets the subshell absorb the
        // `)' and names `echo').
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
                    // GNU parse.y:1225-1236 pattern_list: a pattern holds
                    // at least one word — a `)' where a pattern word was
                    // expected (the empty alternative in `a|)', or a
                    // clause opening with `)') is named by yyerror like
                    // any other unexpected token (rubash#2743, #2679 c25:
                    // `case in in)' names `)').
                    return Some(case_near_error_node(tokens, start, i));
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
                command.logical_line = tokens.get(start).map(|token| token.logical_line);
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
            // GNU parse.y:1225-1236 pattern_list: a pattern is ONE word —
            // glued fragments of it arrive as consecutive tokens with no
            // intervening whitespace — followed by `|' or `)'. Anything
            // else in pattern state is the token yyerror names
            // (rubash#2743, #2679 c23): a whitespace-separated second word
            // (`a echo' names `echo'), a clause-terminator token (`a;;'
            // names `;;' — an operator token even when glued), or a list
            // operator (the `&' after a clause's `;;' is re-met in this
            // same pattern state and names `&').
            if in_extglob == 0 {
                let token = &tokens[i];
                let clause_terminator_word = token.kind == TokenKind::Word
                    && matches!(token.raw.as_str(), ";;" | ";&" | ";;&");
                let list_operator = matches!(
                    token.kind,
                    TokenKind::And | TokenKind::Or | TokenKind::PipeErr | TokenKind::Background
                );
                let second_word = current_pattern_has_token
                    && !token.leading_ws.is_empty()
                    && matches!(
                        token.kind,
                        TokenKind::Word
                            | TokenKind::Variable
                            | TokenKind::Assignment
                            | TokenKind::CommandSubst
                            | TokenKind::BraceExpand
                            | TokenKind::Keyword
                    );
                if clause_terminator_word || list_operator || second_word {
                    return Some(case_near_error_node(tokens, start, i));
                }
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
                    // Check if this word ends with an extglob operator before (.
                    // rubash#131: the reassembly mirrors GNU read_token_word's
                    // pattern-group consumption (parse.y:5466), which only
                    // runs while the extglob shopt is on; gated off, the
                    // `(' stays an ordinary token and the pattern loop below
                    // reports it (GNU: `syntax error near unexpected token
                    // `('').
                    if i + 1 < tokens.len()
                        && is_keyword(tokens, i + 1, "(")
                        && ends_with_extglob_operator(text)
                        && crate::lexer::parse_extended_glob()
                        && !tokens[i].extglob_split
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
                // Handle `!(` as extglob negation pattern (rubash#131:
                // same parse-time extglob gate as the reassembly above).
                TokenKind::Keyword
                    if tokens[i].value == "!"
                        && i + 1 < tokens.len()
                        && is_keyword(tokens, i + 1, "(")
                        && crate::lexer::parse_extended_glob()
                        && !tokens[i].extglob_split =>
                {
                    let extglob = collect_extglob_pattern_from_bang(tokens, &mut i);
                    current_pattern.push_str(&extglob);
                    current_raw_pattern.push_str(&extglob);
                }
                TokenKind::Keyword if tokens[i].value == "(" => {
                    // Error when the gate is closed now, or when the
                    // preceding word was split at this `(' while the gate
                    // was closed at ITS read time (rubash#131: the shopt
                    // state may have flipped between that line and this
                    // parse; GNU decides at read time, parse.y:5466).
                    let split_while_gated = i > 0 && tokens[i - 1].extglob_split;
                    if !crate::lexer::parse_extended_glob() || split_while_gated {
                        // rubash#131: with the extglob shopt off, GNU's
                        // read_token_word never absorbs this `(' into the
                        // pattern word (parse.y:5466 gate), the grammar then
                        // rejects it in pattern position, and yacc reports
                        // `syntax error near unexpected token `(' with the
                        // offending line echoed (verified GNU 5.3.0:
                        // `case ab in a?(b)) echo m;; *) echo n;; esac').
                        let mut command = CommandNode::new();
                        command.line = tokens.get(start).map(|token| token.position);
                        command.logical_line = tokens.get(start).map(|token| token.logical_line);
                        command.insert_assignment(
                            "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                            format!(
                                "({}{}",
                                crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                                tokens[i].position,
                            ),
                        );
                        command.insert_assignment(
                            "__RUBASH_PARSE_SOURCE__".to_string(),
                            pattern_error_line_text(tokens, i),
                        );
                        // Consume only through this case's `esac`, not
                        // the rest of the input: commands after the case
                        // are separate units (GNU's reader reaches them
                        // independently), and the executor may recover
                        // this node by reparsing it after a runtime
                        // extglob flip (rubash#1093).
                        return Some(finish_compound_command(
                            command,
                            tokens,
                            case_command_end_index(tokens, start),
                        ));
                    }
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
                        // rubash#2744: a `|' with no pattern word before
                        // it — leading (`|a)') or doubled (`a||b)') —
                        // starts no alternative: GNU's pattern_list
                        // (parse.y:1225-1236) admits `|' only BETWEEN
                        // words, and reports it as the unexpected token.
                        if !current_pattern_has_token {
                            return Some(case_pattern_error_node(tokens, start, i, "|"));
                        }
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
                // rubash#2744: `||' (and `&&') in pattern position is
                // the shell operator token, not pattern text — GNU
                // reports it (`case a in a||b)' names `||').
                TokenKind::Or | TokenKind::And if in_extglob == 0 => {
                    let blame = tokens[i].value.clone();
                    return Some(case_pattern_error_node(tokens, start, i, &blame));
                }
                // rubash#2687: an unquoted `<`/`>' in pattern position
                // is a redirection operator to GNU, not a literal —
                // `case "a<" in a<*)' is a syntax error at the `<'. The
                // token used to fall into the catch-all and be silently
                // dropped, so the pattern quietly lost the character
                // and matched as `a*'. A `<(`/`>(' pair is the one
                // exception: a process substitution is a WORD in
                // pattern position and is collected verbatim above the
                // error (it expands like any other pattern word).
                TokenKind::RedirectIn | TokenKind::RedirectOut if in_extglob == 0 => {
                    if let Some((text, end)) = collect_pattern_procsub(tokens, i) {
                        current_pattern.push_str(&text);
                        current_raw_pattern.push_str(&text);
                        i = end;
                    } else {
                        let blame = tokens[i].value.clone();
                        return Some(case_pattern_error_node(tokens, start, i, &blame));
                    }
                }
                TokenKind::Append
                | TokenKind::RedirectErr
                | TokenKind::RedirectErrAppend
                | TokenKind::HereDoc
                | TokenKind::HereString
                    if in_extglob == 0 =>
                {
                    let blame = tokens[i].value.clone();
                    return Some(case_pattern_error_node(tokens, start, i, &blame));
                }
                _ => {}
            }
            i += 1;
        }
        if i >= tokens.len() {
            // The pattern state ran into end of input (`case x in (a',
            // `case x in a'): GNU's reader presents EOF in this state as
            // its `newline' token (the EOF token's report name for a
            // string/file input), at the last input line — the resync
            // fallback used to let the main loop reinterpret the `(a'
            // opener as a subshell and name `(' (rubash#2743).
            let last = tokens.len().saturating_sub(1);
            return Some(case_near_error_node_text(
                tokens,
                start,
                "newline",
                tokens.get(last).map_or(1, |token| token.position),
                last,
            ));
        }
        if !is_keyword(tokens, i, ")") {
            return None;
        }
        let pattern_close_delimiter = tokens[i].value.clone();
        let pattern_close_delimiter_metadata = build_keyword_metadata(&tokens[i]);
        i += 1;

        // GNU parse.y:1227-1236 pattern_list: after the pattern's
        // closing `)' the grammar admits only a compound_list or a
        // newline_list (the bodyless alternative); compound_list
        // (parse.y:1262) is `newline_list list0', and no list can begin
        // with `)'. A second `)' — `x))', `(x))', `x|y))' — is a
        // syntax error at that token (rubash#1641; verified vs GNU
        // 5.3.20: `syntax error near unexpected token `)'', rc 2, and
        // the clause never executes). Report it through the same
        // pattern-position error channel as the extglob `(' above, and
        // consume only through this case's `esac` so commands after
        // the case stay separate units.
        let mut body_head = i;
        while body_head < tokens.len()
            && tokens[body_head].kind == TokenKind::Semicolon
            && tokens[body_head].line_break
        {
            body_head += 1;
        }
        if is_keyword(tokens, body_head, ")") {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            command.logical_line = tokens.get(start).map(|token| token.logical_line);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                format!(
                    "){}{}",
                    crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                    tokens[body_head].position
                ),
            );
            command.insert_assignment(
                "__RUBASH_PARSE_SOURCE__".to_string(),
                pattern_error_line_text(tokens, body_head),
            );
            return Some(finish_compound_command(
                command,
                tokens,
                case_command_end_index(tokens, start),
            ));
        }

        let body_start = i;
        let (body_end, stray_close) = case_body_end(tokens, i);
        if let Some(stray) = stray_close {
            // The body scan ran into a token the clause grammar cannot
            // admit with no valid terminator in between (rubash#1656):
            // GNU parse.y:1237-1256 case_clause_sequence admits only
            // SEMI_SEMI/SEMI_AND/SEMI_SEMI_AND or ESAC after a clause's
            // compound_list, so this is a syntax error at that token,
            // reported via the same channel as rubash#1641 above.
            // Verified against GNU bash 5.3.20 (rc 2) for both spelled
            // seeds `a) echo A;\;& b) ...' and `a) echo A;\;\& b) ...'.
            // The offender is usually the next clause's pattern close
            // `)', but a list operator at command position inside the
            // body trips the grammar earlier and is named instead
            // (rubash#3031: `1) a;| 2) ...' names the `|').
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            command.logical_line = tokens.get(start).map(|token| token.logical_line);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                format!(
                    "{}{}{}",
                    case_offender_text(&tokens[stray]),
                    crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                    tokens[stray].position
                ),
            );
            command.insert_assignment(
                "__RUBASH_PARSE_SOURCE__".to_string(),
                pattern_error_line_text(tokens, stray),
            );
            return Some(finish_compound_command(
                command,
                tokens,
                case_command_end_index(tokens, start),
            ));
        }
        i = body_end;
        let body = super::parse_loop::parse_body_with_diagnostics(
            &tokens[body_start..i],
            source,
            source_line_offset,
        );
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

        // GNU parse.y:3641-3646: the newline token that ends the `;;' line
        // triggers gather_here_documents BEFORE the next token is read, so
        // a here-document opened in this clause (`1) cat <<EOF ;;` with the
        // body on the following lines) is gathered BETWEEN clauses: its
        // body text never becomes a token of the next clause's pattern, and
        // the delimiter line's newline is just more newline_list before the
        // next pattern/esac (parse.y:1225 pattern_list). Attach the gathered
        // body to this clause's pending heredoc and skip it
        // (rubash#222: t0383).
        while i < tokens.len() {
            match tokens[i].kind {
                TokenKind::HereDocBody => {
                    let body = tokens[i].value.clone();
                    let gather_line = tokens[i].position;
                    let end_line = tokens[i].heredoc_end_line;
                    if let Some(clause) = clauses.last_mut() {
                        let mut ast = Ast {
                            commands: std::mem::take(&mut clause.body),
                        };
                        super::redirections::assign_heredoc_body(
                            &mut CommandNode::new(),
                            &mut ast,
                            &mut [],
                            &mut [],
                            body,
                            gather_line,
                            end_line,
                        );
                        clause.body = ast.commands;
                    }
                    i += 1;
                }
                // newline_list between clauses (line-break separators
                // around the gathered body).
                TokenKind::Semicolon if tokens[i].line_break => i += 1,
                _ => break,
            }
        }
    }

    if !is_keyword(tokens, i, "esac") {
        // A token that arrived where ESAC was expected is the offender
        // (the same mismatched-closer rule parse_if_command applies to
        // `done' for `fi'); end of input keeps the caller's EOF path
        // (the compound-EOF report for an unterminated case).
        if i < tokens.len() {
            return Some(case_near_error_node(tokens, start, i));
        }
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
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

/// Index just past the `esac` that closes the case command starting at
/// `start`, counting nested `case`/`esac` keyword pairs. Falls back to
/// the end of input when no closing `esac` exists (genuinely unterminated
/// case), preserving the historical consume-everything behavior there.
fn case_command_end_index(tokens: &[Token], start: usize) -> usize {
    let mut depth = 0usize;
    let mut index = start;
    while index < tokens.len() {
        // rubash#1721: only structural `case'/`esac' count — a
        // keyword-spelled argument word inside a clause body
        // (`echo esac') is not the case's own keyword (GNU parse.y:
        // 3168 CHECK_FOR_RESERVED_WORD).
        if tokens[index].structural_keyword() && is_keyword(tokens, index, "case") {
            depth += 1;
        } else if tokens[index].structural_keyword() && is_keyword(tokens, index, "esac") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return index + 1;
            }
        }
        index += 1;
    }
    tokens.len()
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
    let mut first = true;
    for token in &tokens[start..] {
        if token.position != line {
            break;
        }
        if token.kind == TokenKind::Semicolon && token.line_break {
            continue;
        }
        // HereDocBody tokens carry no source text of this physical
        // line either: a real body lives on the following lines, and
        // the synthetic end-of-input body is a lone marker byte
        // (rubash#2687: blaming a `<<' in pattern position echoed a
        // trailing \x1f from that token).
        if token.kind == TokenKind::HereDocBody {
            continue;
        }
        // GNU y.error echoes the physical line verbatim (parse.y
        // report_syntax_error -> the saved shell_input_line), so keep the
        // original inter-token whitespace captured in `leading_ws` instead
        // of normalizing to single spaces (rubash#131 reproducer:
        // `  -?([a-z])+([0-9])) echo m ;;' keeps its two-space indent).
        if first {
            text.push_str(&token.leading_ws);
            first = false;
        } else if !token.leading_ws.is_empty() {
            text.push_str(&token.leading_ws);
        } else if let Some(end) = prev_end {
            if token.column > end {
                text.push(' ');
            }
        }
        text.push_str(&token.raw);
        prev_end = Some(token.column + token.raw.len());
    }
    text
}

/// The name GNU's report_syntax_error (parse.y:6833) gives the token
/// that arrived where the case grammar expected something else: a
/// line-break separator reports as `newline', a hard `;' as `;', a word
/// in its source form (quotes intact — error_token_from_token returns
/// the WORD's read text), anything else by its token value.
fn case_offender_text(token: &Token) -> String {
    match token.kind {
        TokenKind::Semicolon => {
            if token.line_break {
                "newline".to_string()
            } else {
                ";".to_string()
            }
        }
        TokenKind::Word
        | TokenKind::Variable
        | TokenKind::Assignment
        | TokenKind::CommandSubst
        | TokenKind::BraceExpand => token.raw.clone(),
        _ => token.value.clone(),
    }
}

/// A `syntax error near unexpected token' node in this module's
/// established pattern-position encoding (NEAR spec + the physical
/// offending line), consuming the rest of the input like the sibling
/// pattern errors (`;' inside a pattern, rubash#1641's second `)').
fn case_near_error_node(
    tokens: &[Token],
    start: usize,
    offender_index: usize,
) -> (CommandNode, usize) {
    let token = &tokens[offender_index];
    case_near_error_node_text(
        tokens,
        start,
        &case_offender_text(token),
        token.position,
        offender_index,
    )
}

fn case_near_error_node_text(
    tokens: &[Token],
    start: usize,
    token_text: &str,
    line: usize,
    source_index: usize,
) -> (CommandNode, usize) {
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
        format!(
            "{}{}{}",
            token_text,
            crate::executor::markers::PARSE_ERROR_FIELD_SEP,
            line
        ),
    );
    command.insert_assignment(
        "__RUBASH_PARSE_SOURCE__".to_string(),
        pattern_error_line_text(tokens, source_index),
    );
    finish_compound_command(command, tokens, tokens.len())
}

fn build_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

/// The pattern-position syntax-error node (GNU parse.y pattern_list,
/// parse.y:1225-1236): blame `blame_text` at `blame_index`, echo the
/// offending line, and consume only through this case's `esac` so
/// commands after the case stay separate units. Same channel as the
/// extglob `(' and stray-`)' arms above (rubash#1641).
fn case_pattern_error_node(
    tokens: &[Token],
    start: usize,
    blame_index: usize,
    blame_text: &str,
) -> (CommandNode, usize) {
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
        format!(
            "{}{}{}",
            blame_text,
            crate::executor::markers::PARSE_ERROR_FIELD_SEP,
            tokens[blame_index].position
        ),
    );
    command.insert_assignment(
        "__RUBASH_PARSE_SOURCE__".to_string(),
        pattern_error_line_text(tokens, blame_index),
    );
    finish_compound_command(command, tokens, case_command_end_index(tokens, start))
}

/// Collect a process-substitution word starting at the redirect token
/// `tokens[index]` (`<`/`>` directly followed by a glued `('), as
/// verbatim pattern text. GNU reads `<(...)' as ONE word in pattern
/// position — it is expanded like any pattern word, never a
/// redirection (parse.y pattern_list admits WORD; the reader's
/// process-substitution scan makes the group part of the word).
/// Returns the verbatim text and the index of the closing `)' token.
fn collect_pattern_procsub(tokens: &[Token], index: usize) -> Option<(String, usize)> {
    let open = tokens.get(index + 1)?;
    if !is_keyword(tokens, index + 1, "(") || !open.leading_ws.is_empty() {
        return None;
    }
    let mut depth = 0usize;
    let mut end = None;
    for (offset, token) in tokens[index + 1..].iter().enumerate() {
        if is_keyword(tokens, index + 1 + offset, "(") {
            depth += 1;
        } else if is_keyword(tokens, index + 1 + offset, ")") {
            depth -= 1;
            if depth == 0 {
                end = Some(index + 1 + offset);
                break;
            }
        }
    }
    let end = end?;
    let mut text = tokens[index].raw.clone();
    for token in &tokens[index + 1..=end] {
        text.push_str(&token.leading_ws);
        text.push_str(&token.raw);
    }
    Some((text, end))
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

/// End of a clause body: the index where the body token span stops,
/// plus the index of a stray pattern-close `)` met on the way, if any
/// (rubash#1656). A body is a compound_list (parse.y:1262); at its top
/// level an unquoted `)` owns no construct — subshell/function parens
/// balance in `paren_depth`, nested compounds ride the boundary
/// stack, pattern regions ride the stack's case sentinels — so a `)`
/// that arrives with both empty can only be the NEXT clause's pattern
/// close, reached because the current clause's terminator is missing
/// or invalid (`a) echo A;\;& b) ...`: `\;&` is a word plus `&`, not
/// SEMI_AND). GNU parse.y:1237-1256 case_clause_sequence admits only
/// a terminator or ESAC there and reports a syntax error at that `)`.
pub(super) fn case_body_end(tokens: &[Token], mut index: usize) -> (usize, Option<usize>) {
    let mut stack = Vec::new();
    let mut paren_depth = 0usize;
    // rubash#3031: whether the walk sits at a command position of the
    // clause's compound_list (parse.y:1262) — at body start, or just
    // after a top-level separator/joiner. A list operator met there has
    // no command on its left, so it cannot continue the grammar: it is
    // the token GNU's yyerror names (`case i in 1) a;| 2) ...' names
    // the `|', not the next clause's pattern close the stray-`)` walk
    // below used to reach). Tracked only at the flat level: nested
    // compounds ride `stack`, subshell parens ride `paren_depth`, and
    // the token that OPENED such a construct already counts as the
    // command a following joiner continues.
    let mut at_command_start = true;
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

        let flat = stack.is_empty() && paren_depth == 0;
        if flat
            && at_command_start
            && matches!(
                tokens[index].kind,
                TokenKind::Pipe
                    | TokenKind::PipeErr
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Background
            )
        {
            return (index, Some(index));
        }

        if tokens[index].kind == TokenKind::Keyword && is_unquoted_operator(&tokens[index], ")") {
            if paren_depth == 0 && stack.is_empty() {
                return (index, Some(index));
            }
            paren_depth = paren_depth.saturating_sub(1);
        } else if tokens[index].kind == TokenKind::Keyword
            && is_unquoted_operator(&tokens[index], "(")
        {
            paren_depth += 1;
        }

        if flat {
            // Separators and joiners alike leave the list expecting a
            // command (a joiner with no following command is the same
            // error one token later); every other token is command
            // material and ends the position.
            at_command_start = matches!(
                tokens[index].kind,
                TokenKind::Semicolon
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Pipe
                    | TokenKind::PipeErr
                    | TokenKind::Background
            );
        }

        update_compound_boundary_stack(tokens, index, &mut stack);
        index += 1;
    }

    (index, None)
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
