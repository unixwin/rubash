use super::{command_boundary_keyword_allowed, is_case_end_keyword, parse, ProcessSubstitution};
use crate::executor::markers::DATA_DOLLAR;
use crate::lexer::{Token, TokenKind};
use std::collections::VecDeque;

pub(super) fn process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectIn {
        return None;
    }

    let mut index = redirect_index + 1;
    if !tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::RedirectIn)
        || !tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }
    index += 2;

    collect_process_substitution_target(tokens, index)
}

/// `<> <(:)` — the read-write operator (tokenized as RedirectOut with a
/// `<>`-suffixed value) followed by an INPUT process-substitution word.
/// GNU parse.y's redirection grammar takes any WORD after LESS_GREATER
/// (`LESS_GREATER WORD`, the `<>` production), and read_token lexes a
/// word-initial `<(` as a process substitution — make_redirection then
/// carries r_input_output with the substitution as the filename. The
/// pbb read_sleep idiom `read -rt 0.1 <> <(:)` (rubash#325) relies on it:
/// fd 0 binds read-write onto the empty pipe so `read -t` times out.
pub(super) fn read_write_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    let operator = tokens.get(redirect_index)?;
    if operator.kind != TokenKind::RedirectOut || !operator.value.ends_with("<>") {
        return None;
    }

    let mut index = redirect_index + 1;
    if !tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::RedirectIn)
        || !tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }
    index += 2;

    collect_process_substitution_target(tokens, index)
}

pub(super) fn process_substitution_word_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectIn
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_process_substitution_target(tokens, redirect_index + 2)
}

pub(super) fn any_process_substitution_word_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    process_substitution_word_target(tokens, redirect_index)
        .or_else(|| output_process_substitution_word_target(tokens, redirect_index))
}

pub(super) fn process_substitutions_in_word_with_raw(
    word: &str,
    raw: &str,
) -> Vec<ProcessSubstitution> {
    // Provably-empty admission (rubash#117 whitelist discipline): every
    // production this scan can find requires the trigger byte below, so its
    // absence proves the empty answer and skips the char collect + walk.
    // GNU anchor: parse.y:5305 read_token_word reads a word once; GNU runs
    // NO per-word expansion scans at parse time at all (subst.c analyzes at
    // execution) — this port's scans are the executor-facing metadata
    // source, and the gate only skips scans that cannot match.
    if !raw.contains('(') {
        return Vec::new();
    }

    if !raw_word_has_unquoted_process_substitution(raw) {
        return Vec::new();
    }

    let tokens = crate::lexer::tokenize(word);
    let mut substitutions = Vec::new();
    let mut index = 0usize;

    while index < tokens.len() {
        if let Some((substitution, next_index)) =
            any_process_substitution_word_target(&tokens, index)
        {
            substitutions.push(substitution);
            index = next_index + 1;
        } else {
            index += 1;
        }
    }

    // GNU subst.c:11349-11378 (expand_word_internal, cases '<'/'>'): an
    // unquoted `<('/`>(' ANYWHERE in the word — not just word-initial — is
    // extracted by extract_process_subst (subst.c:1311) and executed by
    // process_substitute (subst.c:6362), its `/dev/fd/N` result spliced into
    // the expanding word (`echo p<(echo x)q` -> `p/dev/fd/63q`,
    // rubash#339). The token-shape scan above finds only word-initial
    // substitutions; this raw-anchored scan owns the embedded spans (start
    // > 0, so a word-initial `<(` already found above is not duplicated).
    // The span text comes from the RAW word: the body keeps its own quoting
    // the same way GNU pulls the span out of the word string before quote
    // removal.
    substitutions.extend(embedded_process_substitutions_in_raw(raw));

    substitutions
}

/// Quote-aware extraction of every UNQUOTED `<(`/`>(` span at raw offset > 0
/// (word-initial spans belong to the token-shape scan in
/// process_substitutions_in_word_with_raw). The paren matcher honors the
/// same quote/escape/nesting rules as GNU parse_matched_pair (parse.y:3877)
/// reached from the read_token_word shellexp arm (parse.y:5490-5524).
fn embedded_process_substitutions_in_raw(raw: &str) -> Vec<ProcessSubstitution> {
    let chars: Vec<char> = raw.chars().collect();
    let mut substitutions = Vec::new();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single
            && !double
            && matches!(ch, '<' | '>')
            && chars.get(index + 1) == Some(&'(')
            && index > 0
        {
            if let Some(end) = embedded_procsub_close(&chars, index + 2) {
                let source: String = chars[index + 2..end].iter().collect();
                let operator = if ch == '>' { ">" } else { "<" };
                let target: String = chars[index..=end].iter().collect();
                substitutions.push(ProcessSubstitution {
                    target,
                    open_delimiter_metadata: delimiter_metadata(&format!("{operator}(")),
                    open_delimiter: format!("{operator}("),
                    operator: operator.to_string(),
                    operator_metadata: delimiter_metadata(operator),
                    source: source.clone(),
                    close_delimiter_metadata: delimiter_metadata(")"),
                    close_delimiter: ")".to_string(),
                    commands: parse(&crate::lexer::tokenize(&source)).commands,
                    output: ch == '>',
                    word_index: None,
                    redirect_fd: None,
                });
                index = end + 1;
                continue;
            }
        }
        index += 1;
    }
    substitutions
}

/// Index of the `)` closing a process-substitution group whose `(` is at
/// `open` (just past the introducer). Mirrors the nesting/quote rules of
/// skip_procsub_paren (parser/token_actions.rs) — GNU parse_matched_pair.
fn embedded_procsub_close(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() && depth > 0 {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

/// Whether the raw word text contains a process-substitution opener that is
/// unquoted at its position. Mirrors GNU subst.c:11349-11378
/// (`expand_word_internal` cases '<' and '>'): a `<(`/`>(` inside single or
/// double quotes (or a here-document) is data — `add_character` — while an
/// unquoted one is extracted by `extract_process_subst` (subst.c:1311) and
/// executed by `process_substitute` (subst.c:6362).
pub(crate) fn raw_word_has_unquoted_process_substitution(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double && matches!(ch, '<' | '>') && chars.get(index + 1) == Some(&'(') {
            return true;
        }
        index += 1;
    }
    false
}

pub(super) fn output_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectOut
        || !tokens
            .get(redirect_index)?
            .value
            .strip_suffix('>')
            .is_some_and(|prefix| prefix.chars().all(|ch| ch.is_ascii_digit()))
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn combined_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    let redirect = tokens.get(redirect_index)?;
    if !matches!(redirect.kind, TokenKind::RedirectOut | TokenKind::Append)
        || !matches!(redirect.value.as_str(), "&>" | "&>>")
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn append_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::Append
        || !tokens
            .get(redirect_index)?
            .value
            .strip_suffix(">>")
            .is_some_and(|prefix| prefix.chars().all(|ch| ch.is_ascii_digit()))
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn stderr_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if !matches!(
        tokens.get(redirect_index)?.kind,
        TokenKind::RedirectErr | TokenKind::RedirectErrAppend
    ) || !tokens
        .get(redirect_index + 1)
        .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

/// `2>& <(...)` / `2>>& <(...)` — the stderr dup operator taking an INPUT
/// process-substitution WORD. GNU read_token (parse.y:3794-3796) reads the
/// `<(`+`(` as a procsub WORD operand, and r_duplicating_output_word
/// (redir.c:839-843) rejects the non-digit expansion as AMBIGUOUS_REDIRECT
/// reporting the literal word (`cat 2>& <(echo x)` ->
/// `<(echo x): ambiguous redirect`, rc=1; rubash#340). The parse keeps the
/// operator and the substitution as ONE redirect so the dup validator owns
/// that contract instead of the `<(` leaking into the command words.
pub(super) fn input_process_substitution_after_err_redirect(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if !matches!(
        tokens.get(redirect_index)?.kind,
        TokenKind::RedirectErr | TokenKind::RedirectErrAppend
    ) || !tokens
        .get(redirect_index + 1)
        .is_some_and(|token| token.kind == TokenKind::RedirectIn && token.value == "<")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn output_process_substitution_word_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    let redirect = tokens.get(redirect_index)?;
    let open = tokens.get(redirect_index + 1)?;
    if redirect.kind != TokenKind::RedirectOut
        || redirect.value != ">"
        || open.kind != TokenKind::Keyword
        || open.value != "("
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 2)
}

pub(super) fn collect_process_substitution_target(
    tokens: &[Token],
    source_start: usize,
) -> Option<(ProcessSubstitution, usize)> {
    collect_process_substitution_target_with_prefix(tokens, source_start, false)
}

fn collect_output_process_substitution_target(
    tokens: &[Token],
    source_start: usize,
) -> Option<(ProcessSubstitution, usize)> {
    collect_process_substitution_target_with_prefix(tokens, source_start, true)
}

fn collect_process_substitution_target_with_prefix(
    tokens: &[Token],
    source_start: usize,
    output: bool,
) -> Option<(ProcessSubstitution, usize)> {
    let mut index = source_start;
    let source_start = index;
    let mut depth = 1usize;
    let mut case_depth = 0usize;
    while index < tokens.len() {
        let boundary = index == source_start || command_boundary_keyword_allowed(tokens, index);
        if boundary && tokens[index].kind == TokenKind::Keyword && tokens[index].value == "case" {
            case_depth += 1;
        } else if boundary && is_case_end_keyword(tokens, index) {
            case_depth = case_depth.saturating_sub(1);
        } else if case_depth == 0
            && tokens[index].kind == TokenKind::Keyword
            && tokens[index].value == "("
        {
            depth += 1;
        } else if case_depth == 0
            && tokens[index].kind == TokenKind::Keyword
            && tokens[index].value == ")"
        {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        index += 1;
    }
    if index >= tokens.len() {
        return None;
    }

    let source = process_substitution_source(&tokens[source_start..index]);
    let operator = if output { ">" } else { "<" };
    let prefix = format!("{operator}(");
    let commands = parse(&crate::lexer::tokenize(&source)).commands;
    Some((
        ProcessSubstitution {
            target: format!("{prefix}{source})"),
            open_delimiter_metadata: delimiter_metadata(&prefix),
            open_delimiter: prefix,
            operator: operator.to_string(),
            operator_metadata: delimiter_metadata(operator),
            source,
            close_delimiter_metadata: delimiter_metadata(")"),
            close_delimiter: ")".to_string(),
            commands,
            output,
            word_index: None,
            redirect_fd: None,
        },
        index,
    ))
}

fn process_substitution_source(tokens: &[Token]) -> String {
    let mut source = String::new();
    let mut pending_heredoc_delimiters: VecDeque<String> = VecDeque::new();
    let mut skip_next_semicolon = false;

    for (index, token) in tokens.iter().enumerate() {
        if skip_next_semicolon && token.kind == TokenKind::Semicolon {
            skip_next_semicolon = false;
            continue;
        }
        skip_next_semicolon = false;

        if token.kind == TokenKind::HereDocBody {
            if !source.ends_with('\n') {
                source.push('\n');
            }
            let body = token
                .value
                .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
                .unwrap_or(&token.value);
            source.push_str(body.strip_prefix(DATA_DOLLAR).unwrap_or(body));
            if !source.ends_with('\n') {
                source.push('\n');
            }
            if let Some(delimiter) = pending_heredoc_delimiters.pop_front() {
                source.push_str(&delimiter);
                source.push('\n');
            }
            skip_next_semicolon = true;
            continue;
        }

        if !source.is_empty() && !source.ends_with('\n') {
            source.push(' ');
        }
        source.push_str(&token.raw);

        if token.kind == TokenKind::HereDoc {
            if let Some(delimiter) = tokens.get(index + 1) {
                pending_heredoc_delimiters.push_back(delimiter.value.clone());
            }
        }
    }

    source
}

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::new(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}
