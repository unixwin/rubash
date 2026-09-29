use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_for_command(
    tokens: &[Token],
    start: usize,
    source: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports all `for_command`
    // grammar alternatives, nested compound lists, redirections on compound
    // commands and reserved-word parsing state. This maps common
    // `for name [in words]; do body; done` forms.
    if let Some((command, next_i)) = parse_arithmetic_for_command(tokens, start) {
        return Some((command, next_i));
    }
    // Once Bash has recognized the arithmetic-for opener, malformed headers
    // must remain syntax errors instead of falling through to ordinary for.
    let arithmetic_for_marker = tokens
        .get(start + 1)
        .is_some_and(|token| token.value == "((")
        || (is_keyword(tokens, start + 1, "(") && is_keyword(tokens, start + 2, "("));
    if arithmetic_for_marker {
        return None;
    }

    let variable_token = tokens.get(start + 1)?;
    // GNU keeps the loop variable name as the raw word text — quote removal
    // does not apply to the name position (`for f\1 in ...` reports
    // `f\1': not a valid identifier` against the raw spelling, errors.tests
    // line 41; `select $1` already follows this rule). Valid identifiers
    // never contain escapes, so raw == value for every accepted name.
    let variable = if variable_token.raw.is_empty() {
        variable_token.value.clone()
    } else {
        variable_token.raw.clone()
    };
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable | TokenKind::Keyword
    ) {
        return None;
    }
    // `for` loop variable may be a reserved word used as a name
    // (`for for in for; do ...`); reject only `in`/`do`/`done` that would
    // be ambiguous with the loop syntax. This mirrors GNU parse.y which
    // accepts any WORD as loop variable.
    if tokens.get(start + 1)?.kind == TokenKind::Keyword
        && matches!(tokens.get(start + 1)?.value.as_str(), "do" | "done" | "in")
    {
        return None;
    }
    let mut i = skip_newline_list(tokens, start + 2);
    let mut words = Vec::new();
    let mut word_metadata = Vec::new();
    let mut in_keyword = None;
    let mut in_keyword_metadata = None;
    let mut list_terminator = None;
    let mut list_terminator_metadata = None;
    let default_positional = if is_keyword(tokens, i, "in") {
        in_keyword = Some(tokens[i].value.clone());
        in_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));
        i += 1;
        while i < tokens.len() {
            if tokens[i].kind == TokenKind::Semicolon {
                if list_terminator.is_none() {
                    list_terminator = Some(tokens[i].value.clone());
                    list_terminator_metadata = Some(build_keyword_metadata(&tokens[i]));
                }
                i += 1;
                while tokens
                    .get(i)
                    .is_some_and(|token| token.kind == TokenKind::Semicolon)
                {
                    i += 1;
                }
                if is_keyword(tokens, i, "do") {
                    break;
                }
                if for_brace_body_start(tokens, i) {
                    break;
                }
                continue;
            }
            if for_brace_body_start(tokens, i) {
                return None;
            }
            if let Some((word, next_i)) = collect_compound_or_keyword_word_value(tokens, i) {
                let raw = if next_i == i + 1 {
                    tokens[i].raw.as_str()
                } else {
                    word.as_str()
                };
                word_metadata.push(build_word_metadata(words.len(), &word, raw));
                words.push(word);
                i = next_i;
                continue;
            }
            i += 1;
        }
        false
    } else {
        while tokens
            .get(i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            if list_terminator.is_none() {
                list_terminator = Some(tokens[i].value.clone());
                list_terminator_metadata = Some(build_keyword_metadata(&tokens[i]));
            }
            i += 1;
        }
        true
    };

    let (
        body,
        body_end,
        body_kind,
        do_keyword,
        do_keyword_metadata,
        end_keyword,
        end_keyword_metadata,
    ) = if let Some((body, next_i)) = parse_for_brace_body(tokens, i, source, source_line_offset) {
        (
            body,
            next_i,
            CommandBodyKind::BraceGroup,
            None,
            None,
            None,
            None,
        )
    } else {
        if !is_keyword(tokens, i, "do") {
            return None;
        }
        let do_keyword = Some(tokens[i].value.clone());
        let do_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));
        i += 1;

        let body_start = i;
        let mut stack = Vec::new();
        while i < tokens.len() {
            if stack.is_empty()
                && command_boundary_keyword_allowed(tokens, i)
                && is_keyword(tokens, i, "done")
            {
                break;
            }
            update_compound_boundary_stack(tokens, i, &mut stack);
            i += 1;
        }

        if !is_keyword(tokens, i, "done") {
            return None;
        }
        // GNU parse.y:893-935: `DO compound_list DONE` — the loop body
        // cannot be empty (`for i in 1; do done` => near `done',
        // rubash#221).
        if let Some(error) = empty_compound_body_error_node(tokens, body_start, i) {
            return Some((error, i + 1));
        }
        let end_keyword = Some(tokens[i].value.clone());
        let end_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));

        (
            parse_for_body_commands(&tokens[body_start..i], source, source_line_offset),
            i + 1,
            CommandBodyKind::DoDone,
            do_keyword,
            do_keyword_metadata,
            end_keyword,
            end_keyword_metadata,
        )
    };
    let (body_open_delimiter, body_close_delimiter) =
        command_body_delimiters(body_kind, do_keyword.as_deref(), end_keyword.as_deref());
    let (body_open_delimiter_metadata, body_close_delimiter_metadata) =
        command_body_delimiter_metadata(
            body_kind,
            do_keyword_metadata.as_ref(),
            end_keyword_metadata.as_ref(),
            body_open_delimiter.as_deref(),
            body_close_delimiter.as_deref(),
        );
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(Box::new(ForCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata: build_keyword_metadata(&tokens[start]),
        variable: variable.clone(),
        variable_metadata: Box::new(build_word_metadata(0, &variable, &variable_token.raw)),
        in_keyword,
        in_keyword_metadata,
        words,
        word_metadata,
        default_positional,
        list_terminator,
        list_terminator_metadata,
        arithmetic: None,
        body_kind,
        body_open_delimiter,
        body_open_delimiter_metadata,
        body_close_delimiter,
        body_close_delimiter_metadata,
        do_keyword,
        do_keyword_metadata,
        end_keyword,
        end_keyword_metadata,
        body,
    }));
    Some(finish_compound_command(command, tokens, body_end))
}

pub(super) fn command_body_delimiters(
    body_kind: CommandBodyKind,
    do_keyword: Option<&str>,
    end_keyword: Option<&str>,
) -> (Option<String>, Option<String>) {
    match body_kind {
        CommandBodyKind::BraceGroup => (Some("{".to_string()), Some("}".to_string())),
        CommandBodyKind::DoDone => (
            do_keyword.map(str::to_string),
            end_keyword.map(str::to_string),
        ),
    }
}

pub(super) fn command_body_delimiter_metadata(
    body_kind: CommandBodyKind,
    do_keyword_metadata: Option<&Box<WordMetadata>>,
    end_keyword_metadata: Option<&Box<WordMetadata>>,
    body_open_delimiter: Option<&str>,
    body_close_delimiter: Option<&str>,
) -> (Option<Box<WordMetadata>>, Option<Box<WordMetadata>>) {
    match body_kind {
        CommandBodyKind::DoDone => (do_keyword_metadata.cloned(), end_keyword_metadata.cloned()),
        CommandBodyKind::BraceGroup => (
            body_open_delimiter.map(synthetic_delimiter_metadata),
            body_close_delimiter.map(synthetic_delimiter_metadata),
        ),
    }
}

fn synthetic_delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

fn parse_for_body_commands(
    tokens: &[Token],
    source: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Vec<CommandNode> {
    super::parse_loop::parse_body_with_diagnostics(tokens, source, source_line_offset)
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn build_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn skip_newline_list(tokens: &[Token], mut index: usize) -> usize {
    while tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Semicolon && token.line_break)
    {
        index += 1;
    }
    index
}

fn for_brace_body_start(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| {
        (token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
            && token.value.len() >= 2)
            || is_keyword(tokens, index, "{")
    })
}

fn parse_for_brace_body(
    tokens: &[Token],
    index: usize,
    source: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(Vec<CommandNode>, usize)> {
    let token = tokens.get(index)?;
    if token.kind == TokenKind::Keyword
        && token.value.starts_with('{')
        && token.value.ends_with('}')
        && token.value.len() >= 2
    {
        let inner = token
            .value
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();
        let body_tokens = crate::lexer::tokenize(inner);
        return Some((
            parse_for_body_commands(&body_tokens, source, source_line_offset),
            index + 1,
        ));
    }

    if !is_keyword(tokens, index, "{") {
        return None;
    }

    let i = matching_brace_group_end(tokens, index)?;

    Some((
        parse_for_body_commands(&tokens[index + 1..i], source, source_line_offset),
        i + 1,
    ))
}
