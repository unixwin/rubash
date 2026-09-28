use super::parse_loop::unclosed_brace_eof_node;
use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_brace_group_command(
    tokens: &[Token],
    start: usize,
    source: Option<&str>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    let token = tokens.get(start)?;
    if token.kind == TokenKind::Keyword
        && token.value.starts_with('{')
        && token.value.ends_with('}')
        && token.value.len() >= 2
    {
        let inner_source = token.value.trim_start_matches('{').trim_end_matches('}');
        // GNU parse.y:1196 group_command: `'{' compound_list '}'` —
        // compound_list must end in a completed command (parse.y:1262-1279:
        // `list1 '\n' | '&' | ';'`, or a closed compound). The cheap tail
        // checks replicate brace_group_source_has_completed_command's
        // prefix; anything else asks the already-tokenized body's last
        // token (rubash#176: the old gate re-tokenized the whole body per
        // nesting level — and recursed into every folded child — making a
        // depth-D single-line chain O(D²·N); a folded child group is itself
        // a completed compound command, and its own recursive parse below
        // enforces its body's completion at its own level, exactly where
        // GNU's grammar checks it).
        // rubash#131: the body re-parse uses the gate of the pass that
        // folded this group (Token::extglob_gate), not the current global
        // (a later line's `shopt' may have flipped it after this line was
        // read; GNU gates at read time, parse.y:5466).
        let saved_extglob = crate::lexer::parse_extended_glob();
        crate::lexer::set_parse_extended_glob(token.extglob_gate);
        let body_tokens =
            crate::lexer::tokenize_with_initial_posix_and_line(inner_source, false, token.position);
        crate::lexer::set_parse_extended_glob(saved_extglob);
        let inner_tail = inner_source.trim_end_matches([' ', '\t']);
        // GNU parse.y:1264-1290 list grammar: after a completed `list1' only
        // `;' / `&' / a newline may follow before `}'. A body whose last
        // significant token is a dangling connector (`{ x &&\n}') is NOT
        // terminated — in GNU the `}' token then arrives where the grammar
        // demands a pipeline, so the yacc error names `}' (verified vs WSL
        // GNU 5.3.0: `{ x &&\n}` -> line 2 `syntax error near unexpected
        // token `}'' echoing `}').
        let tail_dangles = body_tokens.last().is_some_and(|last| {
            matches!(
                last.kind,
                TokenKind::And | TokenKind::Or | TokenKind::Pipe | TokenKind::PipeErr
            )
        });
        let completed = if inner_tail.is_empty() || tail_dangles {
            false
        } else if inner_tail.ends_with(';') || inner_tail.ends_with('\n') {
            true
        } else {
            body_tokens
                .last()
                .is_some_and(token_completes_brace_group_command)
        };
        if !completed {
            let mut command = CommandNode::new();
            // GNU parse.y:1264-1290 compound_list — the group body's last
            // command must be terminated. GNU reports the yacc error at the
            // closing `}' token (`{ x && }' -> `syntax error near unexpected
            // token `}'' at the `}' line) and print_offending_line
            // (parse.y:6813-6826) echoes that physical input line verbatim —
            // rubash#285: no echo at all used to print. The folded token
            // spans to its final `}', so the closing line is the open brace's
            // line plus the newlines before that `}'.
            let close_line = token.position
                + token.raw[..token.raw.rfind('}').unwrap_or(0)]
                    .matches('\n')
                    .count();
            command.line = Some(close_line);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                "unexpected token `}'".to_string(),
            );
            let source_line = source.and_then(|text| {
                close_line
                    .checked_sub(source_line_offset)
                    .and_then(|line_in_text| {
                        super::parse_loop::source_line_by_number(text, line_in_text)
                    })
            });
            if let Some(line) = source_line {
                command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), line);
            }
            return Some((command, start + 1));
        }
        // GNU parse.y keeps the in-place line counter: body commands inside
        // a collapsed `{ ...; }` token report real script lines. Pass the
        // inner source untrimmed so the tokenizer's newline counting keeps
        // positions anchored at the `{` token's line — trimming leading
        // newlines here would shift every body command up.
        let mut command = CommandNode::new();
        command.line = Some(token.position);
        command.brace_group = Some(Box::new(BraceGroupCommand {
            open_delimiter: "{".to_string(),
            open_delimiter_metadata: delimiter_metadata("{"),
            close_delimiter: "}".to_string(),
            close_delimiter_metadata: delimiter_metadata("}"),
            body: super::parse_loop::parse_body_with_diagnostics(
                &body_tokens,
                source,
                source_line_offset,
            ),
        }));
        return Some(finish_compound_command(command, tokens, start + 1));
    }

    // GNU parse.y:6890-6901: an unclosed `{` at EOF reports
    // "unexpected end of file from `X' command on line N". The collapsed
    // `{ ...` keyword token exists only when the group swallowed the rest
    // of the input, so a `{`-prefixed value with no closing `}` is always
    // unterminated.
    if token.kind == TokenKind::Keyword
        && token.value.starts_with('{')
        && token.value.trim() != "{"
        && !token.value.trim_end().ends_with('}')
    {
        let command = unclosed_brace_eof_node(tokens, start);
        return Some((command, tokens.len()));
    }

    if !is_keyword(tokens, start, "{") {
        return None;
    }

    let Some(i) = matching_brace_group_end(tokens, start) else {
        let command = unclosed_brace_eof_node(tokens, start);
        return Some((command, tokens.len()));
    };

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.brace_group = Some(Box::new(BraceGroupCommand {
        open_delimiter: "{".to_string(),
        open_delimiter_metadata: token_metadata(&tokens[start]),
        close_delimiter: "}".to_string(),
        close_delimiter_metadata: token_metadata(&tokens[i]),
        body: super::parse_loop::parse_body_with_diagnostics(
            &tokens[start + 1..i],
            source,
            source_line_offset,
        ),
    }));
    Some(finish_compound_command(command, tokens, i + 1))
}

fn token_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}
