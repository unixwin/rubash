use super::parse_loop::offending_line_text;
use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_coproc_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // Parse `coproc [NAME] command [args...]` or `coproc [NAME] { body; }` or `coproc [NAME] ( body )`
    let mut i = start + 1; // skip `coproc`

    // Determine if next token is a name followed by a compound command,
    // or if the next token is itself the command.
    let mut name: Option<(String, String)> = None;
    let lookahead = tokens.get(i);
    if let Some(lookahead) = lookahead {
        let is_brace = is_brace_group_token(lookahead) || is_keyword(tokens, i, "{");
        let is_subshell = is_keyword(tokens, i, "(");
        let is_compound = is_brace || is_subshell || is_coproc_shell_command_start(tokens, i);

        if !is_compound {
            // This might be a name. Check if the token *after* it is a brace group or subshell.
            let next_after = tokens.get(i + 1);
            let next_is_compound = next_after.is_some_and(|t| is_brace_group_token(t))
                || is_keyword(tokens, i + 1, "{")
                || is_keyword(tokens, i + 1, "(")
                || is_coproc_shell_command_start(tokens, i + 1);
            // GNU parse.y:1125-1175 coproc grammar: `COPROC WORD
            // shell_command` — WORD is the coproc NAME only when what
            // follows is a compound/shell command. Before a SIMPLE
            // command there is no name production at all: `coproc CP
            // echo hi` is the unnamed form running the simple command
            // `CP echo hi` (the child reports `CP: command not found`
            // and the fds bind as COPROC). A builtin-name allowlist
            // here stole the first word of the simple command as a
            // name (rubash#1153).
            if !lookahead.value.starts_with('-')
                && !is_keyword(tokens, i, "!")
                && next_is_compound
            {
                name = Some((lookahead.value.clone(), lookahead.raw.clone()));
                i += 1; // consume the name
            }
            // Otherwise: no name, the token is part of the simple command
        }
    }

    // rubash#1652: GNU parse.y:1125-1175 coproc grammar is `COPROC
    // [WORD] shell_command`, and `!` is a pipeline modifier, not a
    // shell_command — so `coproc ! ...` is a syntax error, never a
    // name and never a simple-command word. Without this, the `!`
    // fell into the simple-command word list and the coproc ran
    // with the negation silently applied inside the child.
    // Note the name gate above excludes the `!` keyword too:
    // `coproc ! { ...; }` must hit this error, not the runtime
    // "not a valid identifier" path (a QUOTED "!" is a real word
    // and still takes the name path, matching GNU's identifier
    // error for `coproc "!" { ...; }`).
    if is_keyword(tokens, i, "!") {
        return Some(coproc_bang_error(tokens, start, i));
    }

    // Parse the body
    if let Some(token) = tokens.get(i) {
        // Brace group body (single token from lexer)
        if is_brace_group_token(token) {
            let body = parse_inline_brace_body(token);
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.logical_line = tokens.get(start).map(|t| t.logical_line);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                Vec::new(),
                CoprocBodyKind::BraceGroup,
                Some(("{".to_string(), "}".to_string())),
                Some(body),
            ));
            return Some(finish_coproc_command(command, tokens, i + 1));
        }

        if is_keyword(tokens, i, "{") {
            let (body, close_i) = parse_split_brace_body(tokens, i)?;
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.logical_line = tokens.get(start).map(|t| t.logical_line);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                Vec::new(),
                CoprocBodyKind::BraceGroup,
                Some((tokens[i].value.clone(), tokens[close_i].value.clone())),
                Some(body),
            ));
            return Some(finish_coproc_command(command, tokens, close_i + 1));
        }

        if let Some((body_command, body_end)) = parse_coproc_compound_body(tokens, i) {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.logical_line = tokens.get(start).map(|t| t.logical_line);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                Vec::new(),
                CoprocBodyKind::CompoundCommand,
                None,
                Some(vec![body_command]),
            ));
            return Some(finish_coproc_command(command, tokens, body_end));
        }

        if let Some((body, body_end)) = parse_coproc_command_sequence_body(tokens, i) {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.logical_line = tokens.get(start).map(|t| t.logical_line);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                Vec::new(),
                CoprocBodyKind::CommandSequence,
                None,
                Some(body),
            ));
            return Some(finish_coproc_command(command, tokens, body_end));
        }

        // Subshell body: ( ... )
        if is_keyword(tokens, i, "(") {
            i += 1; // consume `(`
            let body_start = i;
            let mut depth = 1usize;
            let mut case_depth = 0usize;
            while i < tokens.len() {
                let boundary = i == body_start || command_boundary_keyword_allowed(tokens, i);
                if boundary && is_keyword(tokens, i, "case") {
                    case_depth += 1;
                } else if (boundary || follows_case_in_keyword(tokens, i))
                    && is_case_end_keyword(tokens, i)
                {
                    // `esac` right after the case's `in` closes the empty
                    // case (parse.y:3428-3441 special_case_tokens) so
                    // `coproc ( case x in esac )` ends at its own `)`
                    // (rubash#336).
                    case_depth = case_depth.saturating_sub(1);
                } else if case_depth == 0 && is_keyword(tokens, i, "(") {
                    depth += 1;
                } else if case_depth == 0 && is_keyword(tokens, i, ")") {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                i += 1;
            }
            if i < tokens.len() {
                let body = parse(&tokens[body_start..i]).commands;
                let mut command = CommandNode::new();
                command.line = tokens.get(start).map(|t| t.position);
                command.logical_line = tokens.get(start).map(|t| t.logical_line);
                command.coproc_command = Some(coproc_command(
                    name,
                    Vec::new(),
                    Vec::new(),
                    CoprocBodyKind::Subshell,
                    Some(("(".to_string(), tokens[i].value.clone())),
                    Some(body),
                ));
                return Some(finish_coproc_command(command, tokens, i + 1));
            }
        }
    }

    // Simple command: collect remaining tokens as words
    let mut words = Vec::new();
    // Explicit element type: the rubash#1983 index-1 check below
    // reads word_metadata[0].raw before the first push, which
    // leaves the element type ambiguous to inference (E0282).
    let mut word_metadata: Vec<WordMetadata> = Vec::new();
    while i < tokens.len() {
        // rubash#1652 (continued): GNU also rejects `!` as the
        // SECOND word of the coproc's simple command (`coproc a !
        // ...`, `coproc NAME ! ...`): after COPROC WORD the parser
        // commits to the named form and demands a shell_command,
        // so a `!` there is a syntax error — while a `!` after two
        // or more words is a plain word (`coproc a b ! c` parses).
        // The body-position check above covers index 0; this
        // covers index 1. (A quoted "!" is a real word and never
        // reaches this arm: is_keyword only matches the bare
        // keyword token.)
        //
        // rubash#1983: the index-1 rejection does NOT apply when
        // the one collected word is an assignment word. GNU's
        // lexer returns an assignment-shaped first word as
        // ASSIGNMENT_WORD, and parse.y:5899
        // reserved_word_acceptable has no ASSIGNMENT_WORD in its
        // accept set, so the following `!` never becomes BANG at
        // all — it lexes as a plain WORD and lands as the child
        // command's NAME (`coproc X=1 ! echo hi` runs a child that
        // reports `!: command not found`, script rc 0; verified
        // against GNU bash 5.3.20, incl. the `a[1]=x` / `a+=1`
        // shapes; a non-name first word like `9a=1` or `a-b=1`
        // stays a WORD and the `!` still errors). The raw spelling
        // decides: a quoted name (`"X"=1`) is not an assignment
        // word in GNU either.
        if words.len() == 1
            && !coproc_word_is_assignment(&word_metadata[0].raw)
            && is_keyword(tokens, i, "!")
        {
            return Some(coproc_bang_error(tokens, start, i));
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
        } else {
            break;
        }
    }

    if words.is_empty() {
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|t| t.position);
    command.logical_line = tokens.get(start).map(|t| t.logical_line);
    command.coproc_command = Some(coproc_command(
        name,
        words,
        word_metadata,
        CoprocBodyKind::SimpleCommand,
        None,
        None,
    ));
    Some(finish_coproc_command(command, tokens, i))
}

/// The parse-error node GNU reports for a `!` where the coproc
/// grammar admits no pipeline modifier (body position, or second
/// word of the simple command). The executor reports it as
/// `syntax error near unexpected token `!'` and aborts with
/// status 2. The rest of the offending construct (up to the next
/// command boundary) is swallowed so parsing resumes cleanly at
/// the next command.
fn coproc_bang_error(tokens: &[Token], start: usize, bang: usize) -> (CommandNode, usize) {
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|t| t.position);
    command.logical_line = tokens.get(start).map(|t| t.logical_line);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
        format!(
            "!{}{}",
            crate::executor::markers::PARSE_ERROR_FIELD_SEP,
            tokens[bang].position,
        ),
    );
    command.insert_assignment(
        "__RUBASH_PARSE_SOURCE__".to_string(),
        offending_line_text(tokens, bang, None, 0),
    );
    let mut end = bang;
    while end < tokens.len() && tokens[end].kind != TokenKind::Semicolon {
        end += 1;
    }
    finish_coproc_command(command, tokens, end)
}

fn finish_coproc_command(
    command: CommandNode,
    tokens: &[Token],
    index: usize,
) -> (CommandNode, usize) {
    let (command, mut next_i) = finish_compound_command(command, tokens, index);
    while tokens
        .get(next_i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        next_i += 1;
    }
    (command, next_i)
}

fn coproc_command(
    name: Option<(String, String)>,
    words: Vec<String>,
    word_metadata: Vec<WordMetadata>,
    body_kind: CoprocBodyKind,
    body_delimiters: Option<(String, String)>,
    body: Option<Vec<CommandNode>>,
) -> Box<CoprocCommand> {
    let (body_open_delimiter, body_close_delimiter) = body_delimiters
        .map(|(open, close)| (Some(open), Some(close)))
        .unwrap_or((None, None));
    let body_open_delimiter_metadata = body_open_delimiter
        .as_deref()
        .map(synthetic_delimiter_metadata);
    let body_close_delimiter_metadata = body_close_delimiter
        .as_deref()
        .map(synthetic_delimiter_metadata);
    let (name, name_metadata) = name
        .map(|(value, raw)| {
            let metadata = build_word_metadata(0, &value, &raw);
            (Some(value), Some(Box::new(metadata)))
        })
        .unwrap_or((None, None));
    Box::new(CoprocCommand {
        keyword: "coproc".to_string(),
        keyword_metadata: synthetic_delimiter_metadata("coproc"),
        name,
        name_metadata,
        words,
        word_metadata,
        body_kind,
        body_open_delimiter,
        body_open_delimiter_metadata,
        body_close_delimiter,
        body_close_delimiter_metadata,
        body,
    })
}

fn synthetic_delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

/// GNU assignment-word shape at the head of a simple command: a name
/// (`[A-Za-z_][A-Za-z0-9_]*`, optionally with a `[...]` subscript)
/// followed by `=` or `+=`. This is the shape GNU's lexer returns as
/// ASSIGNMENT_WORD (parse.y read_token), after which reserved words
/// stop being recognized (parse.y:5899 reserved_word_acceptable) —
/// the rubash#1983 exemption for a `!` following the coproc's first
/// word. (Judgment shape first measured in the #1712 line's
/// 537e177c; subscript/`+=` arms oracle-verified for #1983.)
fn coproc_word_is_assignment(word: &str) -> bool {
    let bytes = word.as_bytes();
    let mut i = 0;
    if i >= bytes.len() || !(bytes[i] == b'_' || bytes[i].is_ascii_alphabetic()) {
        return false;
    }
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'[' {
        match word[i..].find(']') {
            Some(close) => i += close + 1,
            None => return false,
        }
    }
    if i < bytes.len() && bytes[i] == b'+' {
        i += 1;
    }
    i < bytes.len() && bytes[i] == b'='
}

fn is_coproc_shell_command_start(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| {
        matches!(
            token.value.as_str(),
            "for" | "case" | "select" | "coproc" | "if" | "while" | "until" | "[["
        ) || token.value.starts_with("((")
            || parse_function_command(tokens, index).is_some()
    })
}

fn parse_coproc_command_sequence_body(
    tokens: &[Token],
    start: usize,
) -> Option<(Vec<CommandNode>, usize)> {
    let end = match tokens.get(start)?.value.as_str() {
        "[[" => matching_coproc_conditional_end(tokens, start)?,
        "if" => matching_coproc_if_end(tokens, start)?,
        "while" | "until" => matching_coproc_loop_end(tokens, start)?,
        _ => return None,
    };
    Some((parse(&tokens[start..=end]).commands, end + 1))
}

fn matching_coproc_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start..tokens.len()).find(|&index| tokens[index].raw == "]]")
}

fn matching_coproc_if_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        let boundary = index == start || command_boundary_keyword_allowed(tokens, index);
        if boundary && is_keyword(tokens, index, "if") {
            depth += 1;
        } else if boundary && is_keyword(tokens, index, "fi") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn matching_coproc_loop_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        let boundary = index == start || command_boundary_keyword_allowed(tokens, index);
        if boundary
            && (is_keyword(tokens, index, "for")
                || is_keyword(tokens, index, "while")
                || is_keyword(tokens, index, "until")
                || is_keyword(tokens, index, "select"))
        {
            depth += 1;
        } else if boundary && is_keyword(tokens, index, "done") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn parse_coproc_compound_body(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    if let Some(parsed) = parse_arithmetic_command(tokens, start) {
        return Some(parsed);
    }
    if let Some(parsed) = parse_function_command(tokens, start) {
        return Some(parsed);
    }

    // rubash#1653: there is deliberately NO "time" arm here. GNU
    // parse.y:1125-1175 gives coproc only `COPROC shell_command`,
    // `COPROC WORD shell_command` and `COPROC simple_command`
    // (plus redirection variants); `time` is a timespec prefix on
    // `pipeline_command` (parse.y:1416), not a shell_command starter,
    // so after `coproc` it is never the time keyword: it is the
    // coproc NAME when a compound command follows (handled by the
    // caller's next-token check), and otherwise the first word of
    // the simple-command body below (`coproc time echo hi` runs a
    // child that reports `time: command not found`, and
    // `coproc time echo hi | cat` leaves the `| cat` outside the
    // coproc, so the coproc lives in the pipeline subshell and the
    // parent never sees COPROC_PID).
    match tokens.get(start)?.value.as_str() {
        "for" => parse_for_command(tokens, start, None, 0),
        "if" => parse_if_command_standalone(tokens, start, None, 0),
        "while" | "until" => parse_loop_command(tokens, start, None, 0),
        "case" => parse_case_command(tokens, start, None, 0),
        "select" => parse_select_command(tokens, start, None, 0),
        "coproc" => parse_coproc_command(tokens, start),
        "[[" => parse_conditional_command(tokens, start, None, 0),
        _ => None,
    }
}

fn is_brace_group_token(token: &Token) -> bool {
    token.kind == TokenKind::BraceExpand
        || (token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
            && token.value.len() > 1)
}

fn parse_inline_brace_body(token: &Token) -> Vec<CommandNode> {
    let inner = token
        .value
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    // rubash#1544: folded-body re-lex under the fold pass's posix
    // mode, locked (see Token::posix_gate).
    let body_tokens = crate::lexer::tokenize_locked_body(inner, token.posix_gate);
    parse(&body_tokens).commands
}

fn parse_split_brace_body(tokens: &[Token], start: usize) -> Option<(Vec<CommandNode>, usize)> {
    if !is_keyword(tokens, start, "{") {
        return None;
    }

    let i = matching_brace_group_end(tokens, start)?;

    Some((parse(&tokens[start + 1..i]).commands, i))
}
