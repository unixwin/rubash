use super::parse_loop::unclosed_brace_eof_node;
use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_brace_group_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let token = tokens.get(start)?;
    if token.kind == TokenKind::Keyword
        && token.value.starts_with('{')
        && token.value.ends_with('}')
        && token.value.len() >= 2
    {
        let inner_source = token.value.trim_start_matches('{').trim_end_matches('}');
        if !brace_group_source_has_completed_command(inner_source) {
            let mut command = CommandNode::new();
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                "unexpected token `}'".to_string(),
            );
            return Some((command, start + 1));
        }
        // GNU parse.y keeps the in-place line counter: body commands inside
        // a collapsed `{ ...; }` token report real script lines. Pass the
        // inner source untrimmed so the tokenizer's newline counting keeps
        // positions anchored at the `{` token's line — trimming leading
        // newlines here would shift every body command up.
        let body_tokens =
            crate::lexer::tokenize_with_initial_posix_and_line(inner_source, false, token.position);
        let mut command = CommandNode::new();
        command.line = Some(token.position);
        command.brace_group = Some(Box::new(BraceGroupCommand {
            open_delimiter: "{".to_string(),
            open_delimiter_metadata: delimiter_metadata("{"),
            close_delimiter: "}".to_string(),
            close_delimiter_metadata: delimiter_metadata("}"),
            body: parse(&body_tokens).commands,
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
        body: parse(&tokens[start + 1..i]).commands,
    }));
    Some(finish_compound_command(command, tokens, i + 1))
}

fn token_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}
