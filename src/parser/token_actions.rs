use super::*;
use crate::lexer::{Token, TokenKind};

use super::parse_loop::{command_is_pending_inversion, offending_line_text, ParseState};

pub(super) enum TokenAction {
    Advance,
    Continue,
    Break,
}

/// GNU parse.y:532 `redirection: '>' WORD` (likewise `<', `>>', `&>',
/// `2>' and the LESSLESS/`<<<' WORD rules): a redirection operator takes a
/// WORD target — fd-dup digits and `(`/`<(` process substitutions are the
/// earlier branch targets. When the token that follows the operator is one
/// that can never begin a target (an operator, a case terminator, a
/// subshell paren, or end-of-line), yacc reports
/// `syntax error near unexpected token `<tok>'' — and `newline' when the
/// operator is last on its line or at EOF (rubash#220: `foo >>|a`,
/// `foo &>|b`, `foo &>>|c`, `echo <->`, `echo <5-10>`; `>>|`/`&>>|` are
/// mksh operators GNU splits into operator + `|' + word).
/// Word-shaped reserved words (`in', `then', `}') ARE legal targets in GNU
/// and are deliberately left to their existing paths (None).
pub(super) fn missing_redirect_target_node(
    tokens: &[Token],
    index: usize,
    diagnostic_text: Option<&str>,
    line_offset: usize,
) -> Option<CommandNode> {
    let offending = |name: &str, line: usize| {
        let mut command = CommandNode::new();
        command.line = Some(line);
        command.insert_assignment(
            "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
            format!(
                "{name}{}{line}",
                crate::executor::markers::PARSE_ERROR_FIELD_SEP
            ),
        );
        // GNU report_syntax_error (parse.y:6833) echoes the offending
        // input line via print_offending_line (parse.y:6814) in every
        // non-interactive `near unexpected token' report — the physical
        // line the OPERATOR sits on, taken from the source text because a
        // comment tail (parse.y:3630) never survives in token raws.
        command.insert_assignment(
            "__RUBASH_PARSE_SOURCE__".to_string(),
            offending_line_text(tokens, index, diagnostic_text, line_offset),
        );
        command
    };
    match tokens.get(index + 1) {
        None => Some(offending("newline", tokens[index].position)),
        Some(next) => match next.kind {
            TokenKind::Eof => Some(offending("newline", tokens[index].position)),
            // rubash#305: the operator is last on its delimiter line and a
            // here document was pending. GNU read_token's '\n' branch
            // (parse.y:3648-3654) gathers the bodies FIRST (3651) and only
            // then hands yacc the NEWLINE, so the offending token is
            // `newline' REPORTED at the post-gathering line (every body
            // line advanced line_number, make_cmd.c:580) while
            // print_offending_line still echoes the header line (the
            // primary reader's shell_input_line). The stream may hold
            // several bodies (`<<A <<B` gathers both); the line counter
            // ends at the LAST one's closing delimiter.
            TokenKind::HereDocBody => {
                let end_line = tokens[index + 1..]
                    .iter()
                    .take_while(|token| token.kind == TokenKind::HereDocBody)
                    .filter_map(|token| token.heredoc_end_line)
                    .max()
                    .unwrap_or(next.position);
                let mut command = offending("newline", end_line);
                // GNU gathers before yacc sees the NEWLINE, so an
                // EOF-unterminated gather has ALREADY warned
                // (make_cmd.c:626) by the time the syntax error prints —
                // carry the warnings on the error node for the executor
                // to emit first (same carrier as push_unclosed_paren_error).
                attach_pending_heredoc_eof_warnings(tokens, index, &mut command);
                Some(command)
            }
            TokenKind::Semicolon if next.line_break => {
                Some(offending("newline", tokens[index].position))
            }
            TokenKind::Semicolon
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::Background
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::RedirectIn
            | TokenKind::RedirectOut
            | TokenKind::Append
            | TokenKind::RedirectErr
            | TokenKind::RedirectErrAppend
            | TokenKind::HereDoc
            | TokenKind::HereString => Some(offending(&next.value, next.position)),
            TokenKind::Keyword if matches!(next.value.as_str(), "(" | ")") => {
                Some(offending(&next.value, next.position))
            }
            TokenKind::Word
                if next.raw == next.value && matches!(next.value.as_str(), ";;" | ";&" | ";;&") =>
            {
                Some(offending(&next.value, next.position))
            }
            _ => None,
        },
    }
}

/// rubash#305: attach `here-document at line N delimited by end-of-file`
/// warnings for the bodies following the dangling redirect operator at
/// `index` (see the HereDocBody arm above). GNU make_here_document warns
/// DURING gathering (make_cmd.c:626) when read_secondary_line returns
/// NULL, i.e. before read_token ever returns the NEWLINE whose rejection
/// produces the syntax error — so the warnings print first. Delimiter
/// pairing mirrors push_unclosed_paren_error: every `<<` token queues the
/// following word, every HereDocBody dequeues one; bodies gathered before
/// the operator already paired theirs.
fn attach_pending_heredoc_eof_warnings(tokens: &[Token], index: usize, command: &mut CommandNode) {
    let mut pending_delimiters: std::collections::VecDeque<String> =
        std::collections::VecDeque::new();
    for (position, token) in tokens.iter().enumerate().take(index + 1) {
        if token.kind == TokenKind::HereDoc {
            let delimiter = tokens
                .get(position + 1)
                .map(|next| next.value.clone())
                .unwrap_or_default();
            pending_delimiters.push_back(delimiter);
        }
    }
    let mut warn_index = 0usize;
    for token in tokens[index + 1..]
        .iter()
        .take_while(|token| token.kind == TokenKind::HereDocBody)
    {
        let delimiter = pending_delimiters.pop_front().unwrap_or_default();
        let body = token
            .value
            .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
            .unwrap_or(token.value.as_str());
        if !body.starts_with(crate::executor::markers::DATA_DOLLAR_STR) {
            continue;
        }
        let body_text = &body[crate::executor::markers::DATA_DOLLAR_STR.len()..];
        let gather_line = token.position;
        // An unterminated gather consumed the body lines it read and stops
        // at EOF on the last one (no closing-delimiter line).
        let warn_line = gather_line + body_text.lines().count();
        command.insert_assignment(
            format!("__RUBASH_PARSE_ERROR_HD_WARN_{warn_index}__"),
            format!(
                "{delimiter}{}{gather_line}{}{warn_line}",
                crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                crate::executor::markers::PARSE_ERROR_FIELD_SEP
            ),
        );
        warn_index += 1;
    }
}

/// Install a `syntax error near unexpected token' node as the parse result
/// and abort the rest of the input, exactly as GNU's parser does (yyerror
/// then jumps to top level): commands already parsed from the same
/// physical line never run (`foo > | cat' runs nothing).
/// `pop_line` is the line the parse unit fails on — the OPERATOR's line,
/// not the reported error line: with a pending here document the report
/// line is the post-gathering line (parse.y:3651 + make_cmd.c:580), but
/// the never-to-run list is the whole logical line that carried the
/// operator (`echo before; echo x <<EOF>#c' runs NOTHING — the
/// newline-terminated list is one parse unit, rubash#305).
fn abort_parse_at_syntax_error(state: &mut ParseState, error: CommandNode, pop_line: usize) {
    while state
        .ast
        .commands
        .last()
        .is_some_and(|command| command.line == Some(pop_line))
    {
        state.ast.commands.pop();
    }
    state.current_cmd = error;
}

pub(super) fn handle_token(tokens: &[Token], i: &mut usize, state: &mut ParseState) -> TokenAction {
    let token = &tokens[*i];
    match token.kind {
        TokenKind::Word
        | TokenKind::Variable
        | TokenKind::CommandSubst
        | TokenKind::BraceExpand => {
            state.current_cmd.subshell |= state.in_subshell;
            note_command_line(&mut state.current_cmd, token);
            // GNU read_token_word (parse.y:5821-5843): a `{varname}` word
            // immediately followed by a redirection operator is a
            // REDIR_WORD claimed by the redirect (fd_var), never a command
            // word — `{fd}</dev/null exec` runs `exec`, not `{fd}`.
            if !command_is_open_conditional(&state.current_cmd)
                && tokens.get(*i + 1).is_some_and(|next| {
                    matches!(
                        next.kind,
                        TokenKind::RedirectIn
                            | TokenKind::RedirectOut
                            | TokenKind::Append
                            | TokenKind::HereDoc
                            | TokenKind::HereString
                    )
                })
                && redirect_fd_var_prefix(tokens, *i + 1).is_some()
            {
                return TokenAction::Advance;
            }
            if token.value.starts_with('{')
                && token.value.contains(';')
                && !token.value.contains('}')
            {
                state.current_cmd.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    "unexpected end of file".to_string(),
                );
            }
            if let Some((value, raw, next_i)) =
                collect_adjacent_process_substitution_word(tokens, *i)
            {
                push_synthetic_process_substitution_word(&mut state.current_cmd, &value, &raw);
                *i = next_i;
            } else if state.current_cmd.words.is_empty() {
                if let Some((value, raw, next_i)) =
                    collect_split_array_element_assignment_word(tokens, *i)
                {
                    push_synthetic_process_substitution_word(&mut state.current_cmd, &value, &raw);
                    *i = next_i;
                } else {
                    push_command_word(&mut state.current_cmd, token);
                }
            } else if token.raw.contains("=(") && token.raw.ends_with(')') {
                // GNU parse.y read_token_word: `=(` only opens a compound
                // assignment when the `=` follows a valid assignment LHS --
                // `$(( a=(1+2) ))` keeps `=(` inside the arithmetic word and
                // must never reach the position check.
                let compound_candidate = token
                    .raw
                    .split_once('=')
                    .is_some_and(|(lhs, _)| valid_compound_assignment_lhs(lhs));
                if compound_candidate && !compound_assignment_position_ok(&state.current_cmd.words)
                {
                    return reject_compound_assignment_position(tokens, i, state, token);
                }
                // Atomic compound operand after a command word (declare -a
                // e=(...) ): the lexer keeps name=(...) whole and de-quotes
                // the value, which would destroy element quote grouping
                // before the declare builtin sees it. Preserve the raw RHS
                // behind the compound marker like the Assignment path does.
                if let Some((lhs, rhs)) = token.raw.split_once('=') {
                    if valid_compound_assignment_lhs(lhs) {
                        let word = format!(
                            "{lhs}={}{}",
                            crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                            rhs
                        );
                        let word_index = state.current_cmd.words.len();
                        state
                            .current_cmd
                            .word_metadata
                            .push(build_word_metadata(word_index, &word, &token.raw));
                        state.current_cmd.words.push(word);
                        state.current_cmd.word_kinds.push(TokenKind::Word);
                    } else {
                        push_command_word(&mut state.current_cmd, token);
                    }
                } else {
                    push_command_word(&mut state.current_cmd, token);
                }
            } else {
                push_command_word(&mut state.current_cmd, token);
            }
        }
        TokenKind::Assignment => {
            state.current_cmd.subshell |= state.in_subshell;
            note_command_line(&mut state.current_cmd, token);
            if state.current_cmd.words.is_empty()
                && tokens
                    .get(*i + 1)
                    .is_some_and(|next| next.kind == TokenKind::Keyword && next.value == "(")
                && !token.raw.ends_with("=(")
            {
                // `name= ( ... )` is not a compound assignment. Bash rejects
                // the separated `(` during parsing instead of executing the
                // following words as a command.
                state.current_cmd.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    "unexpected token `('".to_string(),
                );
            }
            if let Some(pos) = token.value.find('=') {
                if state.current_cmd.words.is_empty() {
                    let var_name = token.value[..pos].to_string();
                    let mut var_value = token.value[pos + 1..].to_string();
                    let mut raw_assignment_value = token
                        .raw
                        .split_once('=')
                        .map(|(_, raw)| raw.to_string())
                        .unwrap_or_else(|| var_value.clone());
                    // Bash only treats `name=(...)` as a compound array
                    // assignment when the `(` is adjacent to `=`. The lexer
                    // marks that with a trailing `(` on the raw; `a= (1 2)`
                    // (space) stays a plain assignment and is a syntax error.
                    if var_value.is_empty() && token.raw.ends_with("=(") {
                        if let Some((compound_value, next_i)) =
                            collect_compound_assignment(tokens, *i)
                        {
                            if let Some(compound_assignment) = compound_assignment_from_word(
                                &token.value,
                                compound_value.clone(),
                                None,
                            ) {
                                state
                                    .current_cmd
                                    .compound_assignments
                                    .push(compound_assignment);
                            }
                            var_value = format!(
                                "{}{}",
                                crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                                compound_value
                            );
                            *i = next_i;
                        } else {
                            // The adjacent `(` of a compound array assignment
                            // was never closed before EOF (parse.y:
                            // "unexpected EOF while looking for matching `)'",
                            // reported with status 1 and no source echo).
                            state.current_cmd.insert_assignment(
                                "__RUBASH_PARSE_ERROR_EOF_PAREN__".to_string(),
                                "unexpected EOF while looking for matching `)'".to_string(),
                            );
                        }
                    } else if token.raw.ends_with(')') && token.raw.contains("=(") {
                        // Atomic name=(...) captured whole by the lexer.
                        // next_token consumes the first name character
                        // before skip_word runs, so compound_assignment_start
                        // sees "b=" for a two-character name and keeps the
                        // word atomic while a one-character name splits and
                        // takes the raw-preserving collector above. Preserve
                        // the raw RHS verbatim behind the compound marker
                        // exactly like the Word path does; token.value has
                        // already de-quoted the element grouping the
                        // assoc/array pair parser needs (assoc.tests
                        // wheat=([six]=6 [foo bar]="qux qix")).
                        if let Some((lhs, rhs)) = token.raw.split_once('=') {
                            if valid_compound_assignment_lhs(lhs) && rhs.starts_with('(') {
                                if let Some(op) = find_unquoted_ctrl_op(rhs) {
                                    state.current_cmd.insert_assignment(
                                        "__RUBASH_COMPOUND_SYNTAX_ERROR__".to_string(),
                                        format!("unexpected token `{op}'"),
                                    );
                                    state.current_cmd.insert_assignment(
                                        "__RUBASH_PARSE_SOURCE__".to_string(),
                                        token.raw.clone(),
                                    );
                                } else {
                                    var_value = format!(
                                        "{}{}",
                                        crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                                        rhs
                                    );
                                }
                            }
                        }
                    }
                    if let Some((value, raw, next_i)) =
                        collect_adjacent_assignment_process_substitution(tokens, *i, token)
                    {
                        var_value.push_str(&value);
                        raw_assignment_value.push_str(&raw);
                        *i = next_i;
                    }
                    let assignment_name = var_name.strip_suffix('+').unwrap_or(&var_name);
                    record_command_substitutions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        None,
                    );
                    record_arithmetic_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        None,
                    );
                    record_parameter_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        None,
                    );
                    record_brace_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        &raw_assignment_value,
                        None,
                    );
                    record_extglob_patterns_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        &raw_assignment_value,
                        None,
                    );
                    record_tilde_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        &raw_assignment_value,
                        None,
                    );
                    if let Some((_, raw_value)) = token.raw.split_once('=') {
                        record_word_quotes_for_assignment(
                            &mut state.current_cmd,
                            assignment_name,
                            raw_value,
                            None,
                        );
                    }
                    state.current_cmd.insert_assignment_with_raw(
                        var_name,
                        var_value,
                        raw_assignment_value,
                    );
                } else {
                    let mut word = token.value.clone();
                    let raw_word = token.raw.clone();
                    let mut atomic_compound_attached = false;
                    if raw_word.ends_with(')') && raw_word.contains("=(") {
                        let compound_candidate = raw_word
                            .split_once('=')
                            .is_some_and(|(lhs, _)| valid_compound_assignment_lhs(lhs));
                        if compound_candidate
                            && !compound_assignment_position_ok(&state.current_cmd.words)
                        {
                            return reject_compound_assignment_position(tokens, i, state, token);
                        }
                        // Atomic compound (the lexer keeps name=(...) whole
                        // through whitespace and metacharacters, GNU
                        // read_token_word): mark it and preserve the raw
                        // right-hand side verbatim so element quote grouping
                        // survives into the array storage, exactly like the
                        // split-form path below.
                        if let Some((lhs, rhs)) = raw_word.split_once('=') {
                            if valid_compound_assignment_lhs(lhs) {
                                word = format!(
                                    "{lhs}={}",
                                    crate::executor::types::COMPOUND_ASSIGNMENT_MARKER
                                );
                                word.push_str(rhs);
                                atomic_compound_attached = true;
                            }
                        }
                        // Split-form compound operand after a command word
                        // (declare -a e=( ... )): the lexer ends the word at
                        // the open paren; reassemble the compound through the
                        // quote-preserving collector so element quote
                        // grouping survives into the declare builtin, exactly
                        // like the first-word assignment path above.
                        //
                        // Only for genuinely split words: for an atomic
                        // token the raw RHS above is already verbatim, and
                        // the collector's quote re-joining would re-escape
                        // the element grouping the pair parser needs
                        // (assoc.tests: myarray=(["a]=test1;#a"]="123")).
                        if !atomic_compound_attached {
                            if let Some((compound_value, next_i)) =
                                collect_compound_assignment(tokens, *i)
                            {
                                if let Some((lhs, _)) = raw_word.split_once('=') {
                                    word = format!(
                                        "{lhs}={}{}",
                                        crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                                        compound_value
                                    );
                                    *i = next_i;
                                }
                            }
                        }
                    } else if raw_word.ends_with(")'")
                        && raw_word.contains("='(")
                        && state.current_cmd.words.first().is_some_and(|command| {
                            matches!(command.as_str(), "declare" | "typeset" | "local")
                        })
                        && state.current_cmd.words.iter().take(4).any(|argument| {
                            (argument.starts_with('-') || argument.starts_with('+'))
                                && argument.chars().any(|flag| flag == 'a' || flag == 'A')
                        })
                    {
                        // Whole-single-quoted compound operand
                        // (declare -a d='(...)'): mark the value so the
                        // executor keeps it verbatim; the token value already
                        // has the inner element quotes preserved.
                        if let Some((lhs, raw_rhs)) = raw_word.split_once('=') {
                            if valid_compound_assignment_lhs(lhs)
                                && raw_rhs.starts_with("'(")
                                && raw_rhs.ends_with(")'")
                            {
                                // token.value carries the single-quoted body
                                // with the lexer's carriers intact (\x1f for
                                // `$`, \x18 for `"`, ...), so word expansion
                                // leaves it verbatim. GNU defers the compound
                                // expansion to declare_builtin ->
                                // expand_compound_array_assignment
                                // (arrayfunc.c:557) — after earlier operands
                                // have bound — so `declare -a a=('x') d='($a)'
                                // must still see the unexpanded `$a` here. The
                                // \x03 lead-in inside the parens marks the
                                // carriers as deferred SYNTAX (the builtin
                                // decodes them back to real chars and expands)
                                // rather than escape-produced data.
                                let inner = token
                                    .value
                                    .split_once('=')
                                    .map(|(_, value)| value)
                                    .unwrap_or_else(|| &raw_rhs[1..raw_rhs.len() - 1]);
                                // The protected value may lead with the
                                // sq-protection tag (\x1c); keep it and mark
                                // the compound body after it.
                                let (tag, body) = inner
                                    .strip_prefix('\u{1c}')
                                    .map(|body| ("\u{1c}", body))
                                    .unwrap_or(("", inner));
                                let deferred = body
                                    .strip_prefix('(')
                                    .map(|rest| format!("{tag}(\u{3}{rest}"))
                                    .unwrap_or_else(|| inner.to_string());
                                word = format!(
                                    "{lhs}={}{}",
                                    crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                                    deferred
                                );
                            }
                        }
                    } else if word.ends_with('=') {
                        if let Some((compound_value, next_i)) =
                            collect_compound_assignment(tokens, *i)
                        {
                            let lhs = token.value.strip_suffix('=').unwrap_or(&token.value);
                            let compound_candidate = valid_compound_assignment_lhs(lhs);
                            if compound_candidate
                                && !compound_assignment_position_ok(&state.current_cmd.words)
                            {
                                return reject_compound_assignment_position(
                                    tokens, i, state, token,
                                );
                            }
                            if let Some(compound_assignment) = compound_assignment_from_word(
                                &token.value,
                                compound_value.clone(),
                                Some(state.current_cmd.words.len()),
                            ) {
                                state
                                    .current_cmd
                                    .compound_assignments
                                    .push(compound_assignment);
                            }
                            word.push_str(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER);
                            word.push_str(&compound_value);
                            *i = next_i;
                        }
                    }
                    let word_index = state.current_cmd.words.len();
                    if let Some((assignment_name, value)) = word.split_once('=') {
                        record_command_substitutions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            Some(word_index),
                        );
                        record_arithmetic_expansions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            Some(word_index),
                        );
                        record_parameter_expansions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            Some(word_index),
                        );
                        let raw_assignment_value = raw_word
                            .split_once('=')
                            .map(|(_, raw)| raw)
                            .unwrap_or(value);
                        record_brace_expansions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            raw_assignment_value,
                            Some(word_index),
                        );
                        record_extglob_patterns_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            raw_assignment_value,
                            Some(word_index),
                        );
                        record_tilde_expansions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            raw_assignment_value,
                            Some(word_index),
                        );
                        if let Some((raw_assignment_name, raw_value)) = raw_word.split_once('=') {
                            record_word_quotes_for_assignment(
                                &mut state.current_cmd,
                                raw_assignment_name
                                    .strip_suffix('+')
                                    .unwrap_or(raw_assignment_name),
                                raw_value,
                                Some(word_index),
                            );
                        }
                    }
                    state
                        .current_cmd
                        .word_metadata
                        .push(build_word_metadata(word_index, &word, &raw_word));
                    state.current_cmd.words.push(word);
                    state.current_cmd.word_kinds.push(TokenKind::Word);
                }
            }
        }
        TokenKind::Pipe | TokenKind::PipeErr => {
            if command_is_open_conditional(&state.current_cmd) {
                push_command_word(&mut state.current_cmd, token);
            } else if command_is_empty(&state.current_cmd) {
                // GNU parse.y:1471 `pipeline: pipeline '|' newline_list
                // pipeline` — `|` only ever follows a pipeline, never sits
                // in command-start position. `foo &| bar`, `| foo`,
                // `foo | | bar` and the case-body `a ;| b` are all
                // `syntax error near unexpected token `|'' (rubash#220;
                // `&|` and `;|` are mksh operators GNU rejects).
                let mut error = CommandNode::new();
                error.line = Some(token.position);
                error.insert_assignment(
                    "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                    format!(
                        "|{}{}",
                        crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                        token.position
                    ),
                );
                abort_parse_at_syntax_error(state, error, tokens[*i].position);
                return TokenAction::Break;
            } else {
                // Save current command with pipe flag
                state.current_cmd.subshell |= state.in_subshell;
                state.current_cmd.pipe = Some(if token.kind == TokenKind::PipeErr {
                    2
                } else {
                    1
                });
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
            }
        }
        TokenKind::Semicolon => {
            // Command separator
            state.current_cmd.subshell |= state.in_subshell;
            if !command_is_empty(&state.current_cmd) {
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
            }
        }
        TokenKind::RedirectIn => {
            if command_is_open_conditional(&state.current_cmd) {
                push_command_word(&mut state.current_cmd, token);
            } else {
                note_command_line(&mut state.current_cmd, token);
                let fd = redirect_operator_fd(&token.value).or_else(|| {
                    take_adjacent_redirect_fd_prefix(&mut state.current_cmd, tokens, *i)
                });
                let fd_var = redirect_fd_var_prefix(tokens, *i);
                if let Some((mut process_substitution, next_i)) =
                    process_substitution_redirect_target(tokens, *i)
                {
                    process_substitution.redirect_fd = fd;
                    let target = process_substitution.target.clone();
                    state
                        .current_cmd
                        .process_substitutions
                        .push(process_substitution);
                    let redirect =
                        redirect_node_with_fd_var(&token.value, fd, fd_var, &target, false, false);
                    state.current_cmd.redirects.push(redirect.clone());
                    // `{var}` redirects allocate a fresh descriptor (GNU
                    // redir.c redir_varassign) — never the fd-0 mirror.
                    if redirect.fd_var.is_none() && redirect.fd.unwrap_or(0) == 0 {
                        state.current_cmd.redirect_in = Some(redirect);
                    }
                    *i = next_i;
                } else if let Some((mut process_substitution, next_i)) =
                    process_substitution_word_target(tokens, *i)
                {
                    let (suffix, suffix_raw, joined_i) =
                        collect_process_substitution_suffix(tokens, next_i);
                    if process_substitution_is_adjacent_to_previous_word(tokens, *i)
                        && !state.current_cmd.words.is_empty()
                    {
                        let value = format!(
                            "{}{}{}",
                            state.current_cmd.words.last().cloned().unwrap_or_default(),
                            process_substitution.target,
                            suffix
                        );
                        let raw = format!(
                            "{}{}{}",
                            state
                                .current_cmd
                                .word_metadata
                                .last()
                                .map(|metadata| metadata.raw.as_str())
                                .unwrap_or_else(|| state
                                    .current_cmd
                                    .words
                                    .last()
                                    .map(String::as_str)
                                    .unwrap_or_default()),
                            process_substitution.target,
                            suffix_raw
                        );
                        replace_last_process_substitution_word(
                            &mut state.current_cmd,
                            &value,
                            &raw,
                        );
                        *i = joined_i;
                    } else if let Some((value, raw, joined_i)) =
                        collect_adjacent_process_substitution_word(tokens, *i)
                    {
                        push_synthetic_process_substitution_word(
                            &mut state.current_cmd,
                            &value,
                            &raw,
                        );
                        *i = joined_i;
                    } else {
                        process_substitution.word_index = Some(state.current_cmd.words.len());
                        let target = process_substitution.target.clone();
                        state
                            .current_cmd
                            .process_substitutions
                            .push(process_substitution);
                        state.current_cmd.words.push(target);
                        state.current_cmd.word_kinds.push(TokenKind::Word);
                        *i = next_i;
                    }
                } else if *i + 1 < tokens.len() && is_redirect_target_token(&tokens[*i + 1]) {
                    let (dup_value, dup_raw) =
                        dup_close_target(&mut state.current_cmd, &token.value, &tokens[*i + 1]);
                    let redirect = redirect_node_with_fd_var_raw(
                        &token.value,
                        fd,
                        fd_var,
                        &input_redirect_target(&token.value, &dup_value),
                        &input_redirect_target(&token.value, &dup_raw),
                        false,
                        false,
                    );
                    state.current_cmd.redirects.push(redirect.clone());
                    if redirect.fd_var.is_none() && redirect.fd.unwrap_or(0) == 0 {
                        state.current_cmd.redirect_in = Some(redirect);
                    }
                    *i += 1;
                } else if let Some(error) = missing_redirect_target_node(
                    tokens,
                    *i,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ) {
                    abort_parse_at_syntax_error(state, error, tokens[*i].position);
                    return TokenAction::Break;
                }
            }
        }
        TokenKind::RedirectOut => {
            if command_is_open_conditional(&state.current_cmd) {
                push_command_word(&mut state.current_cmd, token);
            } else {
                note_command_line(&mut state.current_cmd, token);
                if let Some(next_i) = assign_redirect_out_target(tokens, *i, &mut state.current_cmd)
                {
                    *i = next_i;
                } else if let Some(error) = missing_redirect_target_node(
                    tokens,
                    *i,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ) {
                    abort_parse_at_syntax_error(state, error, tokens[*i].position);
                    return TokenAction::Break;
                }
            }
        }
        TokenKind::Append => {
            note_command_line(&mut state.current_cmd, token);
            if let Some(next_i) = assign_append_target(tokens, *i, &mut state.current_cmd) {
                *i = next_i;
            } else if let Some(error) = missing_redirect_target_node(
                tokens,
                *i,
                state.diagnostic_text.as_deref(),
                state.source_line_offset,
            ) {
                abort_parse_at_syntax_error(state, error, tokens[*i].position);
                return TokenAction::Break;
            }
        }
        TokenKind::RedirectErr => {
            note_command_line(&mut state.current_cmd, token);
            if let Some((mut process_substitution, next_i)) =
                stderr_process_substitution_redirect_target(tokens, *i)
            {
                process_substitution.redirect_fd = Some(2);
                let target = process_substitution.target.clone();
                state
                    .current_cmd
                    .process_substitutions
                    .push(process_substitution);
                let redirect =
                    redirect_node(&token.value, Some(2), &target, false, token.value == "2>|");
                state.current_cmd.redirects.push(redirect.clone());
                state.current_cmd.redirect_err = Some(redirect);
                *i = next_i;
            } else if let Some(next_i) =
                assign_redirect_err_target(tokens, *i, &mut state.current_cmd)
            {
                *i = next_i;
            } else if let Some(error) = missing_redirect_target_node(
                tokens,
                *i,
                state.diagnostic_text.as_deref(),
                state.source_line_offset,
            ) {
                abort_parse_at_syntax_error(state, error, tokens[*i].position);
                return TokenAction::Break;
            }
        }
        TokenKind::RedirectErrAppend => {
            note_command_line(&mut state.current_cmd, token);
            if let Some((mut process_substitution, next_i)) =
                stderr_process_substitution_redirect_target(tokens, *i)
            {
                process_substitution.redirect_fd = Some(2);
                let target = process_substitution.target.clone();
                state
                    .current_cmd
                    .process_substitutions
                    .push(process_substitution);
                let redirect = redirect_node(&token.value, Some(2), &target, true, false);
                state.current_cmd.redirects.push(redirect.clone());
                state.current_cmd.redirect_err_append = Some(redirect);
                *i = next_i;
            } else if let Some(next_i) =
                assign_redirect_err_append_target(tokens, *i, &mut state.current_cmd)
            {
                *i = next_i;
            } else if let Some(error) = missing_redirect_target_node(
                tokens,
                *i,
                state.diagnostic_text.as_deref(),
                state.source_line_offset,
            ) {
                abort_parse_at_syntax_error(state, error, tokens[*i].position);
                return TokenAction::Break;
            }
        }
        TokenKind::HereDoc => {
            note_command_line(&mut state.current_cmd, token);
            // GNU parse.y:561 `redirection: ... LESSLESS WORD` — the
            // delimiter is a WORD. Word-shaped reserved words (`esac`,
            // `in`, ...) read as WORDs in target position (GNU probe:
            // `cat << esac` parses), so the shared target predicate admits
            // them; `('/`)' operator keywords stay rejected. Operator tokens
            // are syntax errors (`cat << ;;` => near `;;', rubash#220).
            if *i + 1 < tokens.len() && is_redirect_target_token(&tokens[*i + 1]) {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_heredoc_fd_prefix(&mut state.current_cmd));
                let delimiter_token = &tokens[*i + 1];
                let delimiter = delimiter_token.value.clone();
                state
                    .current_cmd
                    .redirects
                    .push(redirect_node_with_fd_var_raw(
                        &token.value,
                        fd,
                        redirect_fd_var_prefix(tokens, *i),
                        &delimiter,
                        &delimiter_token.raw,
                        false,
                        false,
                    ));
                state.current_cmd.heredoc_redirects.push(heredoc_redirect(
                    &token.value,
                    delimiter_token,
                    fd,
                    redirect_fd_var_prefix(tokens, *i),
                ));
                if fd.is_none() {
                    // GNU stores `here_doc_eof` dequoted (make_cmd.c
                    // string_quote_removal); CTLESC pairs must not leak
                    // into the `wanted `%s'` warning text.
                    state.current_cmd.heredoc_delimiter =
                        Some(delimiter.replace(crate::executor::markers::CTLESC, ""));
                }
                *i += 1;
            } else if let Some(error) = missing_redirect_target_node(
                tokens,
                *i,
                state.diagnostic_text.as_deref(),
                state.source_line_offset,
            ) {
                abort_parse_at_syntax_error(state, error, tokens[*i].position);
                return TokenAction::Break;
            }
        }
        TokenKind::HereString => {
            note_command_line(&mut state.current_cmd, token);
            if let Some((process_substitution, next_i)) =
                any_process_substitution_word_target(tokens, *i + 1)
            {
                assign_here_string_process_substitution(
                    &mut state.current_cmd,
                    &token.value,
                    process_substitution,
                    redirect_fd_var_prefix(tokens, *i),
                );
                *i = next_i;
            } else if *i + 1 < tokens.len() && is_redirect_target_token(&tokens[*i + 1]) {
                // GNU parse.y:664 `redirection: LESS_LESS_LESS WORD` — the
                // herestring operand is a WORD, so word-shaped reserved words
                // (`in`, `then`, `{`, ...) are operand text, not keywords
                // (CHECK_FOR_RESERVED_WORD parse.y:3170 +
                // reserved_word_acceptable parse.y:5899 — a redirect operator
                // is not an accepting position). Shared predicate with the
                // other redirect-target sites (issue #248).
                assign_here_string_redirect_raw(
                    &mut state.current_cmd,
                    &token.value,
                    &tokens[*i + 1].value,
                    &tokens[*i + 1].raw,
                    redirect_fd_var_prefix(tokens, *i),
                );
                *i += 1;
            } else if let Some(error) = missing_redirect_target_node(
                tokens,
                *i,
                state.diagnostic_text.as_deref(),
                state.source_line_offset,
            ) {
                abort_parse_at_syntax_error(state, error, tokens[*i].position);
                return TokenAction::Break;
            }
        }
        TokenKind::HereDocBody => {
            note_command_line(&mut state.current_cmd, token);
            assign_heredoc_body(
                &mut state.current_cmd,
                &mut state.ast,
                token.value.clone(),
                token.position,
            );
        }
        TokenKind::And | TokenKind::Or => {
            if command_is_open_conditional(&state.current_cmd) {
                push_command_word(&mut state.current_cmd, token);
            } else if command_is_empty(&state.current_cmd) {
                state.current_cmd.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    format!("unexpected token `{}`", token.value),
                );
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
            } else {
                // TODO(parse.y/execute_cmd.c): This preserves the AND-OR
                // list connector on simple commands. Full Bash grammar needs
                // a list AST with compound commands and proper precedence.
                state.current_cmd.subshell |= state.in_subshell;
                state.current_cmd.and_or = Some(token.kind == TokenKind::And);
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
            }
        }
        TokenKind::Background => {
            // TODO(parse.y/jobs.c): Bash starts the preceding pipeline
            // asynchronously and returns immediately. Until job control is
            // represented, keep `&` as a command terminator so redirections
            // apply to the command instead of treating `&` as an argument.
            state.current_cmd.subshell |= state.in_subshell;
            state.current_cmd.background = true;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
        }
        TokenKind::Keyword => {
            if command_is_open_conditional(&state.current_cmd)
                && matches!(token.value.as_str(), "(" | ")")
            {
                push_command_word(&mut state.current_cmd, token);
                *i += 1;
                return TokenAction::Continue;
            }

            if token.value == "!"
                && (command_is_empty(&state.current_cmd)
                    || command_is_pending_inversion(&state.current_cmd))
            {
                // GNU parse.y:1410-1413 `pipeline_command: BANG
                // pipeline_command' is right-recursive and toggles
                // CMD_INVERT_RETURN (`$2->flags ^= CMD_INVERT_RETURN'), so
                // any run of BANG prefixes may precede a pipeline — and the
                // pipeline_command underneath can itself be a compound
                // (`! ! case x in ... esac' is legal). Accept the toggle
                // while the current command holds ONLY a pending inversion,
                // so a second `!' toggles back off instead of degrading
                // into a `!' word argument (which then steals command
                // position from a following `case' — rubash#257).
                state.current_cmd.inverted = !state.current_cmd.inverted;
                note_command_line(&mut state.current_cmd, token);
                *i += 1;
                return TokenAction::Continue;
            }

            if command_is_empty(&state.current_cmd)
                && matches!(
                    token.value.as_str(),
                    "then" | "do" | "else" | "elif" | "fi" | "done" | "esac"
                )
            {
                note_command_line(&mut state.current_cmd, token);
                state.current_cmd.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    format!("unexpected token `{}`", token.value),
                );
                if let Some(source) = parse_error_source_line(
                    tokens,
                    *i,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ) {
                    state
                        .current_cmd
                        .insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
                }
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
                *i += 1;
                return TokenAction::Continue;
            }

            if token.value == "(" && command_is_empty(&state.current_cmd) {
                state.in_subshell = true;
                *i += 1;
                return TokenAction::Continue;
            }

            if token.value == "}" && command_is_empty(&state.current_cmd) {
                // GNU parse.y: `}' is a reserved word, and at command position
                // with no open brace group it is a fatal syntax error. A
                // leftover `}' is exactly what parse_matched_pair's
                // P_FIRSTCLOSE|P_DOLBRACE scan leaves behind when a bare `{`
                // does not nest inside `${...}` (parse.y:3974-3980), e.g. the
                // trailing `; }' of `xx=${ f() { x; }; }' (comsub2.tests:45).
                // GNU parses a complete command list before executing any of
                // it, so the syntax error suppresses the commands already
                // parsed on this line (probe: `echo ok; }` must not print
                // `ok`); commands on earlier lines still ran.
                let error_line = token.position;
                while state
                    .ast
                    .commands
                    .last()
                    .is_some_and(|command| command.line == Some(error_line))
                {
                    state.ast.commands.pop();
                }
                note_command_line(&mut state.current_cmd, token);
                state.current_cmd.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    format!("unexpected token `{}`", token.value),
                );
                if let Some(source) = parse_error_source_line(
                    tokens,
                    *i,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ) {
                    state
                        .current_cmd
                        .insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
                }
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
                *i += 1;
                return TokenAction::Continue;
            }

            if token.value == "(" && !command_is_empty(&state.current_cmd) {
                // GNU parse.y:1054 `function_def: WORD '(' ')' newline_list
                // function_body'. The LALR parser shifts a `(' after EXACTLY
                // ONE WORD at command position as a function-head candidate —
                // a pending `!' inversion still allows the shift (GNU
                // `! +(a|b)' errors at `a'), while an assignment or
                // redirection prefix already committed the parse to
                // simple_command (GNU `x=1 y=2 +(a|b)' and `>f (a)' error at
                // the `(' itself). Only `)' may follow the shifted `('; any
                // other token is the yacc error. With two or more words the
                // grammar has no `(' production at all — the error names the
                // `(' itself. yyerror aborts the rest of the input
                // (parse.y:6724 → report_syntax_error parse.y:6833), so the
                // FIRST error is the reported one: `echo +(a|b)' with the
                // extglob gate closed names `(', not the later stray `)'
                // (rubash#332). A line break or end of input right after the
                // shifted `(' is GNU's virtual `newline' token.
                let shifted_function_head = lone_word_function_head_candidate(&state.current_cmd);
                let offender = if shifted_function_head {
                    match tokens.get(*i + 1) {
                        Some(next)
                            if !(next.kind == TokenKind::Keyword && next.value == ")")
                                && next.kind != TokenKind::Eof =>
                        {
                            *i + 1
                        }
                        _ => *i,
                    }
                } else {
                    *i
                };
                let name_override = match tokens.get(offender) {
                    Some(next) if next.line_break || next.kind == TokenKind::Eof => Some("newline"),
                    None => Some("newline"),
                    _ => None,
                };
                super::parse_loop::push_unexpected_token_error_named(
                    state,
                    tokens,
                    offender,
                    name_override,
                );
                return TokenAction::Break;
            }

            if token.value == ")" && state.in_subshell {
                if command_is_empty(&state.current_cmd) {
                    if let Some(command) = state.ast.commands.last_mut() {
                        command.subshell_end = true;
                    }
                } else {
                    state.current_cmd.subshell = true;
                    state.current_cmd.subshell_end = true;
                }
                state.in_subshell = false;
                *i += 1;
                // Collect trailing redirections after ) like brace groups do.
                if command_is_empty(&state.current_cmd) {
                    if let Some(command) = state.ast.commands.last_mut() {
                        collect_trailing_redirections(tokens, &mut *i, command);
                    }
                } else {
                    collect_trailing_redirections(tokens, &mut *i, &mut state.current_cmd);
                }
                return TokenAction::Continue;
            }

            // TODO(parse.y): Reserved words are only reserved in specific
            // parser states. If an ordinary command has already started,
            // keep the token text so alias expansion can reparse it later.
            if matches!(token.value.as_str(), "{" | "}") && !command_is_empty(&state.current_cmd) {
                note_command_line(&mut state.current_cmd, token);
                push_command_word(&mut state.current_cmd, token);
                return TokenAction::Advance;
            }

            if !matches!(token.value.as_str(), "(" | ")" | "{" | "}") {
                note_command_line(&mut state.current_cmd, token);
                push_command_word(&mut state.current_cmd, token);
            }
        }
        TokenKind::Eof => {
            return TokenAction::Break;
        }
    }
    TokenAction::Advance
}

fn collect_adjacent_process_substitution_word(
    tokens: &[Token],
    start: usize,
) -> Option<(String, String, usize)> {
    let mut index = start;
    let mut end = tokens.get(start)?.column;
    let mut value = String::new();
    let mut raw = String::new();
    let mut saw_process_substitution = false;

    while let Some(token) = tokens.get(index) {
        if !value.is_empty() && token.column != end {
            break;
        }

        if let Some((process_substitution, next_i)) =
            any_process_substitution_word_target(tokens, index)
        {
            value.push_str(&process_substitution.target);
            raw.push_str(&process_substitution.target);
            saw_process_substitution = true;
            let close = tokens.get(next_i)?;
            end = close.column + close.raw.len();
            index = next_i + 1;
            continue;
        }

        if adjacent_word_token(token) {
            value.push_str(&token.value);
            raw.push_str(&token.raw);
            end = token.column + token.raw.len();
            index += 1;
            continue;
        }

        break;
    }

    (saw_process_substitution && index > start).then_some((value, raw, index - 1))
}

fn collect_adjacent_assignment_process_substitution<'a>(
    tokens: &'a [Token],
    current: usize,
    assignment: &Token,
) -> Option<(String, String, usize)> {
    let next = tokens.get(current + 1)?;
    if next.column != assignment.column + assignment.raw.len() {
        return None;
    }
    collect_adjacent_process_substitution_word(tokens, current + 1)
}

fn collect_split_array_element_assignment_word(
    tokens: &[Token],
    start: usize,
) -> Option<(String, String, usize)> {
    let first = tokens.get(start)?;
    if !adjacent_word_token(first) || !first.value.contains('[') || first.value.contains(']') {
        return None;
    }

    let mut value = first.value.clone();
    let mut raw = first.raw.clone();
    let mut end = first.column + first.raw.len();
    let mut index = start + 1;
    while let Some(token) = tokens.get(index) {
        if token.line_break || !adjacent_word_token(token) {
            break;
        }
        let gap = token.column.saturating_sub(end);
        if gap == 0 {
            value.push(' ');
            raw.push(' ');
        } else {
            value.push_str(&" ".repeat(gap));
            raw.push_str(&" ".repeat(gap));
        }
        value.push_str(&token.value);
        raw.push_str(&token.raw);
        end = token.column + token.raw.len();
        if array_element_assignment_from_word(&value, &raw).is_some() {
            return Some((value, raw, index));
        }
        index += 1;
    }

    None
}

/// The physical input line of `tokens[index]` for a `syntax error near
/// unexpected token' node. GNU parse.y:6813-6826 print_offending_line echoes
/// the current `shell_input_line` verbatim (only trailing newlines stripped,
/// leading whitespace kept), so when the original text is available slice
/// that line by the token's script line; the token join below is the
/// no-source fallback and cannot recover the original spacing (rubash#285:
/// `{ :; } }' used to echo a reconstructed `:; }').
fn parse_error_source_line(
    tokens: &[Token],
    index: usize,
    diagnostic_text: Option<&str>,
    source_line_offset: usize,
) -> Option<String> {
    tokens.get(index)?;
    if let Some(text) = diagnostic_text {
        if let Some(line) = tokens[index]
            .position
            .checked_sub(source_line_offset)
            .and_then(|line_in_text| super::parse_loop::source_line_by_number(text, line_in_text))
        {
            return Some(line);
        }
    }
    let mut start = index;
    while start > 0 {
        let previous = &tokens[start - 1];
        if previous.line_break || previous.kind == TokenKind::HereDocBody {
            break;
        }
        start -= 1;
    }

    let mut source = String::new();
    let mut previous_kind = None;
    for token in tokens.iter().skip(start) {
        if token.kind == TokenKind::HereDocBody || (!source.is_empty() && token.line_break) {
            break;
        }
        append_parse_source_token(&mut source, previous_kind.as_ref(), token);
        previous_kind = Some(token.kind.clone());
        if token.line_break {
            break;
        }
    }

    let source = source.trim().to_string();
    (!source.is_empty()).then_some(source)
}

fn append_parse_source_token(source: &mut String, previous: Option<&TokenKind>, token: &Token) {
    if token.raw.is_empty() {
        return;
    }
    if !source.is_empty() && parse_source_needs_space(previous, &token.kind) {
        source.push(' ');
    }
    source.push_str(&token.raw);
}

fn parse_source_needs_space(previous: Option<&TokenKind>, current: &TokenKind) -> bool {
    if previous.is_none() {
        return false;
    }
    if matches!(
        current,
        TokenKind::Semicolon | TokenKind::Background | TokenKind::Pipe | TokenKind::PipeErr
    ) {
        return false;
    }
    if matches!(
        previous,
        Some(
            TokenKind::HereDoc
                | TokenKind::HereString
                | TokenKind::RedirectIn
                | TokenKind::RedirectOut
                | TokenKind::Append
                | TokenKind::RedirectErr
                | TokenKind::RedirectErrAppend
        )
    ) {
        return false;
    }
    true
}

fn adjacent_word_token(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Word | TokenKind::Variable | TokenKind::CommandSubst | TokenKind::BraceExpand
    )
}

fn push_synthetic_process_substitution_word(cmd: &mut CommandNode, value: &str, raw: &str) {
    let token = Token::new_with_raw(TokenKind::Word, value, raw, 0);
    push_command_word(cmd, &token);
    if let Some(metadata) = cmd.word_metadata.last() {
        cmd.process_substitutions
            .extend(metadata.process_substitutions.iter().cloned());
    }
}

fn replace_last_process_substitution_word(cmd: &mut CommandNode, value: &str, raw: &str) {
    let Some(word_index) = cmd.words.len().checked_sub(1) else {
        return;
    };
    cmd.words[word_index] = value.to_string();
    if let Some(kind) = cmd.word_kinds.get_mut(word_index) {
        *kind = TokenKind::Word;
    }
    let metadata = build_word_metadata(word_index, value, raw);
    cmd.process_substitutions
        .extend(metadata.process_substitutions.iter().cloned());
    if let Some(slot) = cmd.word_metadata.get_mut(word_index) {
        *slot = metadata;
    } else {
        cmd.word_metadata.push(metadata);
    }
}

fn process_substitution_is_adjacent_to_previous_word(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
    else {
        return false;
    };
    adjacent_word_token(previous) && tokens[index].column == previous.column + previous.raw.len()
}

fn collect_process_substitution_suffix(
    tokens: &[Token],
    close_index: usize,
) -> (String, String, usize) {
    let Some(close) = tokens.get(close_index) else {
        return (String::new(), String::new(), close_index);
    };
    let mut value = String::new();
    let mut raw = String::new();
    let mut end = close.column + close.raw.len();
    let mut index = close_index + 1;

    while let Some(token) = tokens.get(index) {
        if token.column != end || !adjacent_word_token(token) {
            break;
        }
        value.push_str(&token.value);
        raw.push_str(&token.raw);
        end = token.column + token.raw.len();
        index += 1;
    }

    (value, raw, index.saturating_sub(1))
}

/// GNU parse.y:5791-5810 + builtins/mkbuiltins.c:157 assignment_builtins:
/// a `=(` inside a command word only lexes as a compound assignment where
/// an assignment statement is acceptable (PST_ASSIGNOK) — command position
/// (every preceding word is an assignment), after an assignment builtin
/// (alias/declare/export/local/readonly/typeset), or after eval/let.
/// Elsewhere the `(` is an unexpected token, e.g.
/// `printf "%s\n" -a a=(a 'b  c')` (array1.sub:1).
fn compound_assignment_position_ok(words: &[String]) -> bool {
    for word in words {
        let is_assignment_word = word
            .split_once('=')
            .is_some_and(|(lhs, _)| valid_compound_assignment_lhs(lhs));
        if !is_assignment_word {
            return matches!(
                word.as_str(),
                "alias" | "declare" | "export" | "local" | "readonly" | "typeset" | "eval" | "let"
            );
        }
    }
    true
}

/// Emit the GNU `syntax error near unexpected token `('` parse error for a
/// `=(` word in a position where assignments are not acceptable. GNU parses
/// the whole line before executing any of it (same as the `}` case above),
/// so commands already parsed on this line are suppressed.
fn reject_compound_assignment_position(
    tokens: &[Token],
    i: &mut usize,
    state: &mut ParseState,
    token: &Token,
) -> TokenAction {
    let error_line = token.position;
    while state
        .ast
        .commands
        .last()
        .is_some_and(|command| command.line == Some(error_line))
    {
        state.ast.commands.pop();
    }
    state.current_cmd.insert_assignment(
        "__RUBASH_PARSE_ERROR__".to_string(),
        "unexpected token `('".to_string(),
    );
    if let Some(source) = parse_error_source_line(
        tokens,
        *i,
        state.diagnostic_text.as_deref(),
        state.source_line_offset,
    ) {
        state
            .current_cmd
            .insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
    }
    state
        .ast
        .commands
        .push(std::mem::take(&mut state.current_cmd));
    *i += 1;
    TokenAction::Continue
}

/// Name-side validator for atomic compound assignments: optional
/// `name[subscript]` head, then identifier rules (GNU arrayfunc.c).
fn valid_compound_assignment_lhs(lhs: &str) -> bool {
    // GNU general.c:519 assignment(): `name+=` is an assignment word (`+` is
    // valid only immediately before `=`), so `name+=(list)` opens a compound
    // assignment exactly like `name=(list)` — compound_assignment_start
    // (lexer/word.rs) applies the same single-trailing-`+` strip.
    let lhs = lhs.strip_suffix('+').unwrap_or(lhs);
    let head = if lhs.ends_with(']') {
        let Some(open) = lhs.rfind('[') else {
            return false;
        };
        let subscript = &lhs[open + 1..lhs.len() - 1];
        if subscript.contains('[') || subscript.contains(']') {
            return false;
        }
        &lhs[..open]
    } else {
        lhs
    };
    let bytes = head.as_bytes();
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        && bytes.iter().any(|b| b.is_ascii_alphabetic() || *b == b'_')
}

/// Scan the raw text of a compound assignment value `( ... )` for an
/// unquoted control operator (&, |, ;, <, >) that GNU's
/// parse_compound_assignment rejects as a syntax error.
///
/// GNU parse_compound_assignment (parse.y:7140) uses read_token(READ)
/// to tokenize; `$(...)`, `${...}` and backtick command substitutions
/// are consumed as part of a WORD token, so control operators inside
/// them are NOT syntax errors at the compound-assignment level. This
/// scanner must skip those constructs the same way.
fn find_unquoted_ctrl_op(value: &str) -> Option<char> {
    let bytes = value.as_bytes();
    let mut i = 0;
    if i < bytes.len() && bytes[i] == b'(' {
        i += 1;
    }
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut backtick_depth = 0i32;
    // GNU parse.y:3629-3642 read_token: a `#' at a token start comments out
    // the rest of the line, and parse.y:7131-7135 allows newlines inside a
    // compound assignment — so a comment line in `name=( ... )' never
    // reaches the parser and its `&'/`<'/... text cannot be a syntax error.
    let mut word_start = true;
    while i < bytes.len() {
        let c = bytes[i];
        if escaped {
            escaped = false;
            word_start = false;
            i += 1;
            continue;
        }
        // Inside backticks, everything is literal until the closing `
        if backtick_depth > 0 {
            match c {
                b'\\' if !in_single => escaped = true,
                b'`' if !in_single && !in_double => backtick_depth -= 1,
                _ => {}
            }
            word_start = false;
            i += 1;
            continue;
        }
        match c {
            b'\\' if !in_single => escaped = true,
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double && word_start => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            // Skip $(...) command substitution — GNU read_token_word
            // consumes it as one unit; control operators inside are
            // part of the subshell, not the compound assignment.
            b'$' if !in_single && i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                i = skip_dollar_paren(bytes, i + 2);
                word_start = false;
                continue;
            }
            // Skip ${...} parameter expansion — same reasoning.
            b'$' if !in_single && i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                i = skip_dollar_brace(bytes, i + 2);
                word_start = false;
                continue;
            }
            // Skip backtick command substitution.
            b'`' if !in_single && !in_double => {
                backtick_depth += 1;
            }
            b'&' | b'|' | b';' | b'<' | b'>' if !in_single && !in_double => {
                return Some(c as char);
            }
            _ => {}
        }
        word_start = matches!(c, b' ' | b'\t' | b'\n' | b'\r');
        i += 1;
    }
    None
}

/// Skip from just after `$(` to the matching `)`, respecting nested
/// `$(...)`, `${...}`, quotes, and backticks.
fn skip_dollar_paren(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while i < bytes.len() && depth > 0 {
        let c = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match c {
            b'\\' if !in_single => escaped = true,
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'$' if !in_single && i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                depth += 1;
                i += 1;
            }
            b')' if !in_single && !in_double => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    i
}

/// Skip from just after `${` to the matching `}`, respecting nested
/// `${...}` and quotes.
fn skip_dollar_brace(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while i < bytes.len() && depth > 0 {
        let c = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match c {
            b'\\' if !in_single => escaped = true,
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'{' if !in_single && !in_double => depth += 1,
            b'}' if !in_single && !in_double => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    i
}
