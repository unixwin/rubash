use super::{
    command_boundary_keyword_allowed, follows_case_in_keyword, is_case_end_keyword, parse,
    redirect_fd_var_prefix, ProcessSubstitution,
};
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

/// End index (just past the closer) of the `$`-introduced group whose
/// `$` sits at `dollar` in `chars` — `$(`/`${`/`$[` — or None when the
/// `$` introduces no group or the group never closes. GNU
/// expand_word_internal consumes these units whole in its `$` arm
/// (subst.c:10915 extract_arithmetic_subst for `$((`, the comsub and
/// brace arms likewise), so a `<`/`>` inside one is data for the inner
/// construct and never reaches the `<`/`>` process-substitution arms
/// (subst.c:11365-11378) at this word's level. The balanced skip is
/// the lexer's own parse_matched_pair walker (dollar_word_group_len),
/// the same rules the P_ARITH span scan uses (rubash#2648:
/// `$(( 4>(2+3) ? 1 : 32))` executed `2+3` as a command).
fn dollar_group_end(chars: &[char], dollar: usize) -> Option<usize> {
    let (open, close) = match chars.get(dollar + 1) {
        Some('(') => ('(', ')'),
        Some('{') => ('{', '}'),
        Some('[') => ('[', ']'),
        _ => return None,
    };
    crate::lexer::dollar_word_group_len(chars, dollar + 1, open, close)
}

/// End index (just past the closing backquote) of the backquote command
/// substitution whose opening backquote sits at `tick` in `chars` — or
/// None when the span never closes. GNU expand_word_internal consumes
/// a backquote unit whole in its own switch arm (subst.c:11572: the
/// body is pulled out by string_extract and executed by
/// command_substitute, subst.c:7143), so a `<`/`>` inside one is data
/// for the inner command text and never reaches the `<`/`>`
/// process-substitution arms (subst.c:11364-11378) at this word's
/// level — the same opacity the `$` arm gives `$(`/`${`/`$[`
/// (dollar_group_end, rubash#3132), extended to the backquote group
/// (rubash#3337). The span walk is the lexer's own backquote walker
/// (skip_backtick_corrected), the same rules the tokenizer uses.
/// Unlike the `$` groups, a backquote span also opens inside double
/// quotes (the backquote arm is not suppressed by Q_DOUBLE_QUOTES —
/// only the `<`/`>` arms are), so the skip arms gate on `!single`
/// only; skipping there also keeps a `"` in the body from corrupting
/// the walker's quote state.
fn backquote_group_end(chars: &[char], tick: usize) -> Option<usize> {
    crate::lexer::skip_backtick_corrected(chars, tick)
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
        if !single && !double && ch == '$' {
            // A `$(`/`${`/`$[` unit is opaque at this word's level
            // (dollar_group_end): its interior belongs to the inner
            // construct's own scan.
            if let Some(end) = dollar_group_end(&chars, index) {
                index = end;
                continue;
            }
        }
        if !single && ch == '`' {
            // A backquote unit is opaque at this word's level
            // (backquote_group_end): its interior is the inner
            // command text, whose own execution materializes any
            // process substitution inside it.
            if let Some(end) = backquote_group_end(&chars, index) {
                index = end;
                continue;
            }
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
        if !single && ch == '`' {
            // A backquote unit is opaque at this span's level
            // (backquote_group_end, the lexer's own backquote walker
            // from PR #3351): its interior — an unmatched `(` or a
            // quote that would corrupt this walker's quote state —
            // belongs to the inner command text, never to this
            // span's paren depth (GNU parse.y:3877
            // parse_matched_pair consumes nested quote units whole
            // via the shellquote arm; subst.c:1311
            // extract_process_subst -> xparse_dolparen (parse.y:4686)
            // re-parses the body with the real parser, rubash#3354).
            // The gate is `!single` only, like the sibling scans in
            // this file: a backquote group stays active inside
            // double quotes. An unclosed group falls through to the
            // plain walk below.
            if let Some(end) = backquote_group_end(&chars, index) {
                index = end;
                continue;
            }
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

/// Byte range of the next UNQUOTED `<(`/`>(` span in `text` at or after
/// byte offset `from` whose operator matches `operator` and whose body
/// re-prints to `source` under `process_substitution_source` — i.e. the
/// span the metadata entry (operator, source) describes. GNU
/// extract_process_subst (subst.c:1311) pulls the span VERBATIM out of
/// the word string; the metadata target is a token-join RE-PRINT
/// (`>&2` comes back as `>& 2`) that need not occur in the word at all,
/// so a caller splicing by the target string silently misses the span
/// (rubash#1561). The re-print comparison is the identity check: a span
/// whose body tokenizes to a different re-print is a different
/// substitution and is passed over. Quote/escape tracking mirrors
/// `embedded_process_substitutions_in_raw` (parse.y:3877
/// parse_matched_pair rules via `embedded_procsub_close`); the walk
/// starts at the top of the word so the quote state at `from` is exact.
pub(crate) fn verbatim_process_substitution_span(
    text: &str,
    from: usize,
    operator: &str,
    source: &str,
) -> Option<(usize, usize)> {
    let want = operator.chars().next()?;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let plain: Vec<char> = chars.iter().map(|(_, ch)| *ch).collect();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let (byte, ch) = chars[index];
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
        if !single && !double && ch == '$' {
            // Same opacity rule as the extraction scan
            // (dollar_group_end): a span inside a `$(`/`${`/`$[` unit
            // belongs to the inner construct, not to this word.
            if let Some(end) = dollar_group_end(&plain, index) {
                index = end;
                continue;
            }
        }
        if !single && ch == '`' {
            // Same opacity rule as the extraction scan
            // (backquote_group_end): a span inside a backquote unit
            // belongs to the inner command text, not to this word.
            if let Some(end) = backquote_group_end(&plain, index) {
                index = end;
                continue;
            }
        }
        if !single
            && !double
            && ch == want
            && byte >= from
            && chars.get(index + 1).is_some_and(|(_, next)| *next == '(')
        {
            if let Some(end) = embedded_procsub_close(&plain, index + 2) {
                let body: String = plain[index + 2..end].iter().collect();
                if process_substitution_source(&crate::lexer::tokenize(&body)) == source {
                    let end_byte = chars[end].0 + 1;
                    return Some((byte, end_byte));
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
    // Whitelist admission (rubash#117 discipline): the DFA below only
    // reports `<(`/`>(` outside quotes. Without either byte pair no arm
    // can fire, so skip the char-collect + scan. A present pair runs the
    // full quote-aware scan unchanged.
    if !raw.contains("<(") && !raw.contains(">(") {
        return false;
    }
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
        if !single && !double && ch == '$' {
            // A `$(`/`${`/`$[` unit is opaque at this word's level
            // (dollar_group_end): a `<(`/`>(` inside one — e.g. the
            // comparison in `$(( 4>(2+3) ))` — is not this word's
            // process substitution.
            if let Some(end) = dollar_group_end(&chars, index) {
                index = end;
                continue;
            }
        }
        if !single && ch == '`' {
            // A backquote unit is opaque at this word's level
            // (backquote_group_end): a `<(`/`>(` inside one — e.g. a
            // process substitution in a backquoted pipeline — is not
            // this word's process substitution.
            if let Some(end) = backquote_group_end(&chars, index) {
                index = end;
                continue;
            }
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
        } else if (boundary || follows_case_in_keyword(tokens, index))
            && is_case_end_keyword(tokens, index)
        {
            // `esac` right after the case's `in` closes the empty case
            // (parse.y:3428-3441 special_case_tokens) so `<(case x in
            // esac)` ends at its own `)` (rubash#336).
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

    // Whether a line-break separator at `from` is one the canonical
    // reprint joins with the `;' spelling rather than a verbatim
    // newline: the end of the body (a trailing separator separates
    // nothing) and the closer/continuation keywords (esac / fi / done /
    // then / else / elif / do / `}' / `)'). The #1024 newline fold below
    // is owed only BETWEEN two commands; before a closer there is no
    // second command, so those positions keep the `;' spelling the
    // #373/#381/#390 adjudication pins (`case esac in ; esac`,
    // `echo bye ;` — rubash#1859). `in` is deliberately absent from the
    // list: `case x ; in` is a GNU syntax error (oracle-verified), so a
    // break before `in` keeps the verbatim newline.
    let next_keeps_semicolon_spelling = |from: usize| -> bool {
        match tokens[from + 1..]
            .iter()
            .find(|t| t.kind != TokenKind::Semicolon)
        {
            None => true,
            Some(t) => {
                t.kind == TokenKind::Keyword
                    && matches!(
                        t.value.as_str(),
                        "esac" | "fi" | "done" | "then" | "else" | "elif" | "do" | "}" | ")"
                    )
            }
        }
    };

    for (index, token) in tokens.iter().enumerate() {
        if skip_next_semicolon && token.kind == TokenKind::Semicolon {
            skip_next_semicolon = false;
            continue;
        }
        skip_next_semicolon = false;

        // rubash#564: a physical newline JOINED into the logical line by the
        // reader reaches the flat pass as a Semicolon with line_break=true.
        // GNU parse_comsub keeps the body verbatim (parse.y:4632 print_comsub
        // reprints the substitution with the body's own newlines), and the
        // source re-parses when the substitution executes — where a `;' is
        // only valid BETWEEN complete commands, never in the positions GNU
        // holds a bare newline: at the body's START (`cat <(\necho hi\n)`
        // produced a leading `;' → "unexpected token `;'") or doubled after
        // another `;'/`;;' (`;;\nesac' → ";; ;"). Fold ONLY those stray
        // positions to the verbatim `\n`; mid-body and trailing separators
        // keep the `;' spelling the #373/#381/#390 adjudication pins
        // (`case esac in ; esac`, `echo bye ;` — GNU-verified wt33/wt39
        // bytes).
        if token.kind == TokenKind::Semicolon
            && token.line_break
            && (source.is_empty() || source.ends_with(';') || source.ends_with('\n'))
        {
            if !source.is_empty()
                && !source.ends_with('\n')
                && !next_keeps_semicolon_spelling(index)
            {
                source.push('\n');
            }
            continue;
        }

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

        // rubash#1024: a bare physical newline mid-body is a canonical line
        // break (parse.y:4632 print_comsub reprints the body with its own
        // newlines), so it re-parses as `\n` rather than `;`: body
        // diagnostics then number from the enclosing command's line instead
        // of collapsing to line 1. An explicit `;` before the newline
        // (`a;\nb`) keeps the `;` spelling above — the canonical model
        // numbers that shape like GNU without a second separator.
        // rubash#1859: "mid-body" means between two commands — a break
        // before a closer/continuation keyword or at the body's end is
        // not between commands and keeps the `;` spelling (see
        // next_keeps_semicolon_spelling).
        if token.kind == TokenKind::Semicolon
            && token.line_break
            && !source.is_empty()
            && !source.ends_with(';')
            && !source.ends_with('\n')
            && !next_keeps_semicolon_spelling(index)
        {
            source.push('\n');
            continue;
        }

        // rubash#2522: the re-join must not invent whitespace across the
        // one boundary whose spacing is grammar-semantic: a `{name}` word
        // directly followed by a redirection operator is ONE redirector
        // (REDIR_WORD) — GNU read_token_word returns REDIR_WORD only when
        // the '<'/'>' immediately follows the '}' (parse.y:5826-5837), and
        // GNU's own printer re-prints every REDIR_VARASSIGN spelling glued
        // (print_cmd.c:1237-1297: `{%s}>&%d` at :1244, `{%s}>&%s` at :1260,
        // `{%s}>&-` at :1297). Operator→target spacing is re-parse-neutral
        // (`>& -` is still r_close_this), but `{w} >& 9` re-lexes as the
        // WORD `{w}` plus a plain fd-1 `>&9` dup. The procsub producer
        // executes this re-print as its body (rubash respawns `rubash -c`,
        // where GNU's fork gets the VERBATIM string, subst.c:6362
        // process_substitute → parse_and_execute), so the inserted space
        // made the producer's external child drop the varfd descriptor and
        // wire its fd 1 to the dup source. redirect_fd_var_prefix is the
        // parser's own column-adjacency test for exactly this boundary —
        // reuse it rather than restating the rule.
        let glued_var_redirector = redirect_fd_var_prefix(tokens, index).is_some();
        if !source.is_empty() && !source.ends_with('\n') && !glued_var_redirector {
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
