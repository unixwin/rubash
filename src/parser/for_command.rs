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

    // A single `(' after the reserved word is not a loop variable: GNU's
    // for grammar expects a NAME (or the arith-for `((' handled above) and
    // reports `syntax error near unexpected token `('` at the paren (issue
    // #516 `for()'). Returning None drops `for' to the word path, where the
    // parser's word-followed-by-`(' gate owns the near-token error.
    if is_keyword(tokens, start + 1, "(") {
        return None;
    }
    let variable_token = tokens.get(start + 1)?;
    if !matches!(
        variable_token.kind,
        TokenKind::Word | TokenKind::Variable | TokenKind::Keyword
    ) {
        // GNU parse.y:893 (`FOR WORD newline_list DO ...`): the name
        // slot admits exactly a WORD, so any other token there is the
        // syntax-error token at its own position (rubash#2108 face A:
        // `for; do ...` blames `;', `for > f; do ...` blames `>').
        // Returning None degraded the head to the word path, which
        // walked on to `do' and blamed IT instead. Two shapes keep the
        // None fall-through: end of input (no token to blame — the
        // dangling-head diagnostic owns it) and a bare newline (GNU
        // blames `newline' there; that diagnostic owns the shape too).
        if variable_token.kind == TokenKind::Eof
            || (variable_token.kind == TokenKind::Semicolon && variable_token.line_break)
        {
            return None;
        }
        return Some(head_unexpected_token_node(
            tokens,
            start,
            start + 1,
            source,
            source_line_offset,
        ));
    }
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
    // `for` loop variable may be any reserved word used as a name: GNU
    // parse.y reads the slot after FOR as a plain WORD (reserved-word
    // recognition is positional), so `for in in a b`, `for do in a` and
    // `for done in a` are all valid loops (an invalid NAME like `{` is
    // an execution-time `not a valid identifier`, not a parse error).
    let mut i = skip_newline_list(tokens, start + 2);
    // GNU parse.y for grammar: after the name only `in`, a terminator
    // (`;`/newline, already skipped) or `do` may follow — anything else
    // is `syntax error near unexpected token` AT THAT TOKEN
    // (rubash#1140: `for in 1 2; do ...` blames `1', not the later
    // `do' the fall-through word path used to reach).
    if !is_keyword(tokens, i, "in") {
        if let Some(token) = tokens.get(i) {
            let legal_follower = token.kind == TokenKind::Semicolon
                || is_keyword(tokens, i, "do")
                || for_brace_body_start(tokens, i)
                || matches!(
                    token.kind,
                    TokenKind::Eof
                        | TokenKind::RedirectIn
                        | TokenKind::RedirectOut
                        | TokenKind::Append
                        | TokenKind::RedirectErr
                        | TokenKind::RedirectErrAppend
                        | TokenKind::HereDoc
                        | TokenKind::HereString
                );
            if !legal_follower {
                let mut command = CommandNode::new();
                command.line = tokens.get(start).map(|token| token.position);
                command.insert_assignment(
                    "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                    format!(
                        "{}{}{}",
                        token.value,
                        crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                        token.position
                    ),
                );
                command.insert_assignment(
                    "__RUBASH_PARSE_SOURCE__".to_string(),
                    super::parse_loop::offending_line_text(
                        tokens,
                        i,
                        source.map(|rc| &**rc),
                        source_line_offset,
                    ),
                );
                return Some(finish_compound_command(command, tokens, tokens.len()));
            }
        }
    }
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
                // GNU parse.y:921 (`FOR WORD newline_list IN word_list
                // list_terminator newline_list DO compound_list DONE`):
                // after the list terminator and the newline list, only
                // `do` (or the `{` body form above) may follow — any other
                // token is the syntax-error token (rubash#2087:
                // `for i in 1 2\ndone` blames `done`; `for i in 1 2;
                // echo x; do ...` blames `echo`). Collecting it as another
                // list word degrades the recognized head into an
                // executable simple command.
                if tokens.get(i).is_some() {
                    return Some(head_unexpected_token_node(
                        tokens,
                        start,
                        i,
                        source,
                        source_line_offset,
                    ));
                }
                continue;
            }
            if for_brace_body_start(tokens, i) {
                return None;
            }
            // GNU parse.y:1037-1054 wordlist admits only WORD tokens: a
            // bare `(' in the list is `syntax error near unexpected token
            // `(''. read_token_word absorbs the group into the word ONLY
            // while the extglob shopt is on (parse.y:5466), so with the
            // gate closed `for x in a @(b|c)' leaves the `(' as its own
            // token and this position must reject it (rubash#386) exactly
            // as simple-command position already does; a plain
            // `for x in a (b)' is the same grammar slot.
            if tokens[i].kind == TokenKind::Keyword
                && tokens[i].value == "("
                && tokens[i].raw == "("
            {
                let mut command = CommandNode::new();
                command.line = tokens.get(start).map(|token| token.position);
                command.insert_assignment(
                    "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                    format!(
                        "({}{}",
                        crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                        tokens[i].position
                    ),
                );
                command.insert_assignment(
                    "__RUBASH_PARSE_SOURCE__".to_string(),
                    super::parse_loop::offending_line_text(
                        tokens,
                        i,
                        source.map(|rc| &**rc),
                        source_line_offset,
                    ),
                );
                return Some(finish_compound_command(command, tokens, tokens.len()));
            }
            // GNU's lexer returns SEMI_SEMI for the case-terminator
            // family (parse.y: the word_list production admits WORD
            // tokens only), so an unquoted `;;'/`;&'/`;;&' in the list
            // is the syntax-error token (rubash#2087: `for i in 1 2;;
            // do ...' blames `;;'). This lexer emits the family as Word
            // tokens (raw == value is the unquoted operator spelling; a
            // quoted `';;'' stays a genuine list word), and the collect
            // below would swallow it as one.
            if tokens[i].kind == TokenKind::Word
                && tokens[i].raw == tokens[i].value
                && matches!(tokens[i].value.as_str(), ";;" | ";&" | ";;&")
            {
                return Some(head_unexpected_token_node(
                    tokens,
                    start,
                    i,
                    source,
                    source_line_offset,
                ));
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
            // GNU parse.y:526-529 (`word_list: WORD | word_list WORD`)
            // admits WORD tokens only, so an operator token the collect
            // above could not take as a word is the syntax-error token
            // at its own position (parse.y:921 for the `for`
            // production; rubash#2108 face B: `for i in 1 2 | cat`
            // blames `|', `for i in > f 1` blames `>'). The loop used
            // to step over this whole class silently — the operator
            // vanished, its target was collected as a list word, and a
            // bogus loop ran. The gate sits AFTER the collect, so the
            // word forms that begin with an operator-shaped token are
            // already taken: process substitutions (GNU lexes `<(...)`
            // inside read_token_word as word text) and quoted or
            // escaped operator spellings (`for i in '|' \> ';;'`
            // iterates over them — Word tokens, never operator kinds).
            if matches!(
                tokens[i].kind,
                TokenKind::Pipe
                    | TokenKind::PipeErr
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Background
                    | TokenKind::RedirectIn
                    | TokenKind::RedirectOut
                    | TokenKind::Append
                    | TokenKind::RedirectErr
                    | TokenKind::RedirectErrAppend
                    | TokenKind::HereDoc
                    | TokenKind::HereString
            ) {
                return Some(head_unexpected_token_node(
                    tokens,
                    start,
                    i,
                    source,
                    source_line_offset,
                ));
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
        // GNU parse.y:893-915: with no `in` list the head is `FOR WORD
        // newline_list DO` / `FOR WORD ';' newline_list DO` — after the
        // terminator run the next token must be `do` (or the `{` body
        // form handled below); anything else is the syntax-error token
        // (rubash#2087: `for i; echo x; do ...` blames `echo`;
        // `for i;\ndone` blames `done`). Returning None here degrades
        // the recognized head into an executable simple command.
        if tokens.get(i).is_some()
            && !is_keyword(tokens, i, "do")
            && !for_brace_body_start(tokens, i)
        {
            return Some(head_unexpected_token_node(
                tokens,
                start,
                i,
                source,
                source_line_offset,
            ));
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
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
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

/// The `syntax error near unexpected token' node for a token that
/// breaks the `for` head's grammar at its position (rubash#2087) — the
/// same `__RUBASH_PARSE_ERROR_NEAR__` encoding as the #1140 follower
/// gate above: the spec carries the offending token and ITS line, the
/// source assignment echoes its physical line.
fn head_unexpected_token_node(
    tokens: &[Token],
    start: usize,
    bad_index: usize,
    source: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> (CommandNode, usize) {
    let token = &tokens[bad_index];
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
        format!(
            "{}{}{}",
            near_blame_text(token),
            crate::executor::markers::PARSE_ERROR_FIELD_SEP,
            token.position
        ),
    );
    command.insert_assignment(
        "__RUBASH_PARSE_SOURCE__".to_string(),
        super::parse_loop::offending_line_text(
            tokens,
            bad_index,
            source.map(|rc| &**rc),
            source_line_offset,
        ),
    );
    finish_compound_command(command, tokens, tokens.len())
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
        // rubash#1544: folded-body re-lex under the fold pass's posix
        // mode, locked (see Token::posix_gate).
        let body_tokens = crate::lexer::tokenize_locked_body(inner, token.posix_gate);
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
