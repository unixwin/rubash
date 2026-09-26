use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
}

pub(super) fn push_command_word(cmd: &mut CommandNode, token: &Token) {
    let word_index = cmd.words.len();
    record_command_substitutions_for_word(cmd, word_index, &token.value);
    record_arithmetic_expansions_for_word(cmd, word_index, &token.value);
    record_parameter_expansions_for_word(cmd, word_index, &token.value);
    record_brace_expansions_for_word(cmd, word_index, &token.value, &token.raw);
    record_extglob_patterns_for_word(cmd, word_index, &token.value, &token.raw);
    record_tilde_expansions_for_word(cmd, word_index, &token.value, &token.raw);
    record_pathname_patterns_for_word(cmd, word_index, &token.value, &token.raw);
    record_word_quotes_for_word(cmd, word_index, &token.raw);
    let prior_words_are_array_assignments = cmd.words.is_empty()
        || (!cmd.array_element_assignments.is_empty()
            && cmd.array_element_assignments.len() == cmd.words.len());
    if prior_words_are_array_assignments {
        // NOTE: an escaped quote inside an array-element subscript
        // (`a[\" \"]=v`) is NOT a parse-time error: the literal `"` survives
        // word expansion as subscript data and fails inside
        // array_expand_index's evalexp with `expr: arithmetic syntax error:
        // operand expected (error token is "expr")` plus the DISCARD abort
        // (verified GNU 5.3). Routing it through the real subscript evaluator
        // also preserves the associative key case (`foo["bar\"bie"]` is
        // valid data — assoc6.sub:44).
        record_array_element_assignment_for_word(cmd, word_index, &token.value, &token.raw);
    }
    cmd.word_metadata
        .push(build_word_metadata(word_index, &token.value, &token.raw));
    cmd.words.push(token.value.clone());
    cmd.word_kinds.push(token.kind.clone());
}

pub(super) fn build_word_metadata(word_index: usize, value: &str, raw: &str) -> WordMetadata {
    WordMetadata::new(word_index, value.to_string(), raw.to_string())
}

pub(super) fn collect_compound_word_value(
    tokens: &[Token],
    index: usize,
) -> Option<(String, usize)> {
    if let Some((process_substitution, next_i)) = process_substitution_word_target(tokens, index) {
        return Some((process_substitution.target, next_i + 1));
    }

    if let Some((process_substitution, next_i)) =
        output_process_substitution_word_target(tokens, index)
    {
        return Some((process_substitution.target, next_i + 1));
    }

    let token = tokens.get(index)?;
    if matches!(
        token.kind,
        TokenKind::Word
            | TokenKind::Variable
            | TokenKind::Assignment
            | TokenKind::CommandSubst
            | TokenKind::BraceExpand
    ) {
        return Some((token.value.clone(), index + 1));
    }

    None
}

pub(super) fn collect_compound_or_keyword_word_value(
    tokens: &[Token],
    index: usize,
) -> Option<(String, usize)> {
    collect_compound_word_value(tokens, index).or_else(|| {
        tokens
            .get(index)
            .filter(|token| token.kind == TokenKind::Keyword)
            .map(|token| (token.value.clone(), index + 1))
    })
}

pub(super) fn set_body_line(body: &mut [CommandNode], line: usize) {
    // TODO(parse.y): Bash preserves source locations through compound command
    // parsing. Rubash reparses inline function bodies from text today, so
    // recover the definition line for diagnostics such as readonly errors.
    for command in body {
        command.line = Some(line);
    }
}

pub(super) fn is_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == value)
}

/// Whether `token` IS the operator/terminator spelled `value` in the source —
/// not merely a word whose quote removal produced that text. GNU read_token
/// (parse.y:5305 read_token_word) consumes a quoted or escaped `)` / `;;` /
/// `)` as WORD TEXT that never becomes an operator token, so
/// `echo ')'` must print `)`. An operator token's raw spelling is exactly its
/// own text; any quoting (`')'`, `"')"`, `\;`) or other word content makes
/// raw differ from the de-quoted value. Fixes rubash#128.
pub(super) fn is_unquoted_operator(token: &Token, value: &str) -> bool {
    token.value == value && token.raw == value
}

pub(super) fn is_case_end_keyword(tokens: &[Token], index: usize) -> bool {
    is_keyword(tokens, index, "esac") && !case_pattern_starts_with_esac(tokens, index)
}

fn case_pattern_starts_with_esac(tokens: &[Token], index: usize) -> bool {
    // A clause terminator proves this is the case command's closing esac,
    // even when the following subshell close is adjacent (esac)).
    if index > 0 && is_case_clause_terminator_token(&tokens[index - 1]) {
        return false;
    }

    if !matches!(
        tokens.get(index + 1).map(|token| token.value.as_str()),
        Some(")" | "|")
    ) {
        return false;
    }

    let mut close = index + 1;
    while close < tokens.len() {
        if is_keyword(tokens, close, ")") {
            break;
        }
        if tokens[close].kind == TokenKind::Semicolon {
            return false;
        }
        close += 1;
    }
    if !is_keyword(tokens, close, ")") {
        return false;
    }

    let mut scan = close + 1;
    while scan < tokens.len() {
        if is_case_clause_terminator_token(&tokens[scan]) || is_keyword(tokens, scan, "esac") {
            return true;
        }
        if is_keyword(tokens, scan, ")") && command_boundary_keyword_allowed(tokens, scan) {
            return false;
        }
        scan += 1;
    }

    false
}

fn is_case_clause_terminator_token(token: &Token) -> bool {
    token.kind == TokenKind::Word && matches!(token.raw.as_str(), ";;" | ";&" | ";;&")
}

/// GNU parse.y for_command/select_command/while_command/until_command all
/// accept either "do list done" or the brace-group form ("{ list }") as the
/// loop body, so the matching terminator may be "done" or "}".
const LOOP_BODY_TERMINATOR: &str = "done-or-brace";

pub(super) fn update_compound_boundary_stack(
    tokens: &[Token],
    index: usize,
    stack: &mut Vec<&'static str>,
) {
    if let Some(expected) = stack.last().copied() {
        let expected_matches = if expected == "esac" {
            is_case_end_keyword(tokens, index)
        } else if expected == LOOP_BODY_TERMINATOR {
            is_keyword(tokens, index, "done") || is_boundary_keyword(tokens, index, "}")
        } else {
            is_keyword(tokens, index, expected)
        };
        if expected_matches {
            stack.pop();
            return;
        }
    }

    if !command_boundary_keyword_allowed(tokens, index) {
        return;
    }

    if is_keyword(tokens, index, "if") {
        stack.push("fi");
    } else if matches!(
        tokens.get(index).map(|token| token.value.as_str()),
        Some("for" | "select" | "while" | "until")
    ) {
        stack.push(LOOP_BODY_TERMINATOR);
    } else if is_keyword(tokens, index, "case") {
        stack.push("esac");
    }
}

pub(super) fn command_boundary_keyword_allowed(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return true;
    };

    if previous.kind == TokenKind::Keyword
        && previous.value.starts_with('{')
        && previous.value.ends_with('}')
        && previous.value.len() >= 2
    {
        return true;
    }

    // GNU parse.y reads reserved words at any position where a command is
    // complete: separators, `)`/`}`-style closers, and the `]]`/`))` that
    // end `[[ ]]`/`(( ))` (cond.tests:230 `if [[ str ]] then [[ str ]] fi`
    // — no `;` before `then`). A `]]`/`))` that never closed a matching
    // opener is a plain word argument (`if echo ]] then` is a GNU syntax
    // error, not a boundary), so verify the pairing backward.
    matches!(
        previous.kind,
        TokenKind::Semicolon
            | TokenKind::HereDocBody
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::Background
    ) || (previous.kind == TokenKind::Keyword
        && matches!(
            previous.value.as_str(),
            "{" | "(" | ")" | "then" | "do" | "else" | "elif" | "fi" | "done" | "esac" | "}"
        ))
        || (previous.kind == TokenKind::Word
            && matches!(previous.raw.as_str(), ";;" | ";&" | ";;&"))
        || compound_close_precedes(tokens, index)
}

/// Whether the token at `index - 1` is a `]]` or `))` that actually
/// closed a matching `[[`/`((` opener (GNU: the closer ends the compound
/// command, so the next token sits at a command boundary). Scans backward
/// counting closer/opener balance on raw spelling so quoted `']]'` and
/// word-argument `]]` never count.
fn compound_close_precedes(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return false;
    };
    let (closer, opener) = match previous.raw.as_str() {
        "]]" => ("]]", "[["),
        "))" => ("))", "(("),
        _ => return false,
    };
    let mut depth = 0i32;
    for token in tokens[..index].iter().rev() {
        if token.raw == closer {
            depth += 1;
        } else if token.raw == opener {
            depth -= 1;
            if depth == 0 {
                return true;
            }
        }
    }
    false
}

pub(super) fn is_boundary_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    command_boundary_keyword_allowed(tokens, index) && is_keyword(tokens, index, value)
}

pub(super) fn brace_group_source_has_completed_command(source: &str) -> bool {
    let terminator_source = source.trim_end_matches([' ', '\t']);
    if terminator_source.is_empty() {
        return false;
    }
    if terminator_source.ends_with(';') || terminator_source.ends_with('\n') {
        return true;
    }

    crate::lexer::tokenize(terminator_source)
        .last()
        .is_some_and(token_completes_brace_group_command)
}

pub(super) fn token_completes_brace_group_command(token: &Token) -> bool {
    // GNU parse.y `compound_list: newline_list list0` and
    // `list0: list1 '\n' newline_list | list1 '&' newline_list |
    // list1 ';' newline_list` (parse.y:1262-1279): a `{ }` group closes
    // after a command terminated by `;', a newline, or `&'. `&&'/`||'
    // (And/Or) are connectors, not terminators — `{ a && }` stays an
    // error as in GNU.
    if matches!(token.kind, TokenKind::Semicolon | TokenKind::Background) {
        return true;
    }
    if token.kind != TokenKind::Keyword {
        return false;
    }
    if matches!(token.value.as_str(), "}" | ")" | "fi" | "done" | "esac") {
        return true;
    }
    if token.value.starts_with('{') && token.value.ends_with('}') && token.value.len() >= 2 {
        let inner_source = token.value.trim_start_matches('{').trim_end_matches('}');
        return brace_group_source_has_completed_command(inner_source);
    }
    false
}

pub(super) fn matching_brace_group_end(tokens: &[Token], start: usize) -> Option<usize> {
    // The lexer keeps trailing blanks inside a '{' keyword token when the
    // brace is followed by blanks before the physical newline ("{\t"), so
    // recognize the opening brace the same way parse_function_command's
    // value.trim() == "{" gate does. An exact is_keyword(tokens, start,
    // "{") here rejects "{\t", the function-body scan returned None, and
    // the caller's last-'}' fallback silently absorbed the rest of the
    // script into a dead function body (GNU parse.y reads the reserved
    // word '{' and treats the blanks as a token separator).
    if !tokens
        .get(start)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value.trim() == "{")
    {
        return None;
    }

    let mut depth = 1usize;
    let mut stack = Vec::new();
    let mut index = start + 1;
    while index < tokens.len() {
        update_compound_boundary_stack(tokens, index, &mut stack);
        if !stack.is_empty() {
            index += 1;
            continue;
        }

        if is_boundary_keyword(tokens, index, "{") || word_command_open_brace(tokens, index) {
            depth += 1;
        } else if is_boundary_keyword(tokens, index, "}") {
            // GNU parse.y requires a completed command before a brace-group
            // close. `{ command }` is malformed; `{ command; }`, a physical
            // newline, and a completed compound command before `}` are valid.
            let has_completed_command = index > start + 1
                && tokens
                    .get(index - 1)
                    .is_some_and(token_completes_brace_group_command);
            if !has_completed_command {
                return None;
            }
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

/// GNU's grammar puts `{` in command position after constructs whose
/// rubash token stream shows it following a WORD or a non-boundary
/// keyword, so `command_boundary_keyword_allowed` alone misses it:
/// `coproc [NAME] {` (parse.y:1125-1174 `coproc: COPROC [WORD]
/// shell_command`), `function NAME {` (function_def `FUNCTION WORD
/// command`), and the pipeline prefixes `!`, `time`, `time -p`,
/// `time --` (parse.y `time_pipeline`, `BANG`). Without this, the `{` is
/// invisible to the depth scan and that group's `}` is mistaken for the
/// enclosing brace's close (type4.sub `mkcoprocs`: `coproc a { cat
/// <<EOF1 ... }` aborted the whole function parse).
/// The walk leftwards consumes only genuine command prefixes, so
/// `echo coproc a {` and `foo bar {` keep `{` as a plain word.
fn word_command_open_brace(tokens: &[Token], index: usize) -> bool {
    if !is_keyword(tokens, index, "{") {
        return false;
    }
    let mut j = index;
    let mut saw_prefix = false;
    while j > 0 {
        let prev = &tokens[j - 1];
        match prev.kind {
            TokenKind::Keyword => match prev.value.as_str() {
                // BANG negation, TIME, and `coproc {`/`function {`
                // (the latter malformed in GNU but harmless to accept).
                "!" | "time" | "coproc" | "function" => {
                    saw_prefix = true;
                    j -= 1;
                }
                _ => return false,
            },
            TokenKind::Word => match prev.value.as_str() {
                // `time -p` / `time --` options (TIMEOPT/TIMEIGN).
                "-p" | "--" => j -= 1,
                _ => {
                    // A WORD directly before `{` is a command argument
                    // (`echo a {`) unless it is a coproc/function name,
                    // which must directly follow its keyword.
                    if j >= 2
                        && matches!(tokens[j - 2].kind, TokenKind::Keyword)
                        && matches!(tokens[j - 2].value.as_str(), "coproc" | "function")
                    {
                        saw_prefix = true;
                        j -= 2;
                        continue;
                    }
                    return false;
                }
            },
            // A separator or other non-word token: `{` opens a group only
            // when the walk consumed at least one command prefix.
            _ => return saw_prefix,
        }
    }
    saw_prefix
}

pub(super) fn command_is_empty(cmd: &CommandNode) -> bool {
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.compound_assignments.is_empty()
        && cmd.array_element_assignments.is_empty()
        && cmd.process_substitutions.is_empty()
        && cmd.command_substitutions.is_empty()
        && cmd.arithmetic_expansions.is_empty()
        && cmd.parameter_expansions.is_empty()
        && cmd.brace_expansions.is_empty()
        && cmd.extglob_patterns.is_empty()
        && cmd.tilde_expansions.is_empty()
        && cmd.pathname_patterns.is_empty()
        && cmd.word_quotes.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.redirects.is_empty()
        && cmd.pipe.is_none()
        && !cmd.background
        && cmd.and_or.is_none()
        && !cmd.inverted
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.time_command.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.for_command.is_none()
        && cmd.arithmetic_command.is_none()
        && cmd.if_command.is_none()
        && cmd.loop_command.is_none()
        && cmd.conditional_command.is_none()
        && cmd.subshell_command.is_none()
        && cmd.case_command.is_none()
        && cmd.select_command.is_none()
        && cmd.function_command.is_none()
        && cmd.brace_group.is_none()
        && cmd.coproc_command.is_none()
}

pub(super) fn command_is_open_conditional(cmd: &CommandNode) -> bool {
    (cmd.words.first().map(String::as_str) == Some("[[")
        || (matches!(cmd.words.first().map(String::as_str), Some("if" | "elif"))
            && cmd.words.get(1).map(String::as_str) == Some("[[")))
        && !cmd.words.iter().any(|word| word == "]]")
}

pub(super) fn command_accepts_embedded_arithmetic_command(cmd: &CommandNode) -> bool {
    matches!(
        cmd.words.first().map(String::as_str),
        Some("if" | "elif" | "while" | "until" | "do" | "then" | "else")
    ) && cmd.words.len() == 1
}

pub(super) fn is_function_name(name: &str) -> bool {
    if name.is_empty() || name.contains('=') {
        return false;
    }

    !name
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

pub(super) fn is_function_keyword_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

/// GNU's function_def grammar accepts any single WORD as a candidate function
/// name; a word whose raw text differs from its dequoted value was quoted or
/// escaped (W_QUOTED), which the executor later rejects via err_invalidid.
/// The parser must still parse `'a b c' () { ...; }' as a function definition
/// so the executor can report the name error (parse.y: function_def).
pub(super) fn is_quoted_function_name(name: &str, name_raw: &str) -> bool {
    !name_raw.is_empty() && name_raw != name
}
