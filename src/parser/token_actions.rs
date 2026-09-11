use super::*;
use crate::lexer::{Token, TokenKind};

use super::parse_loop::ParseState;

pub(super) enum TokenAction {
    Advance,
    Continue,
    Break,
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
                                var_value = format!(
                                    "{}{}",
                                    crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                                    rhs
                                );
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
                    state.current_cmd.insert_assignment(var_name, var_value);
                } else {
                    let mut word = token.value.clone();
                    let raw_word = token.raw.clone();
                    let mut atomic_compound_attached = false;
                    if raw_word.ends_with(')') && raw_word.contains("=(") {
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
                                // Strip only the outer single quotes; the
                                // inner double quotes are the element
                                // grouping the storage parser needs.
                                let inner = &raw_rhs[1..raw_rhs.len() - 1];
                                word = format!(
                                    "{lhs}={}{}",
                                    crate::executor::types::COMPOUND_ASSIGNMENT_MARKER,
                                    inner
                                );
                            }
                        }
                    } else if word.ends_with('=') {
                        if let Some((compound_value, next_i)) =
                            collect_compound_assignment(tokens, *i)
                        {
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
                    state.current_cmd.redirect_in = Some(redirect);
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
                    let redirect = redirect_node_with_fd_var_raw(
                        &token.value,
                        fd,
                        fd_var,
                        &input_redirect_target(&token.value, &tokens[*i + 1].value),
                        &input_redirect_target(&token.value, &tokens[*i + 1].raw),
                        false,
                        false,
                    );
                    state.current_cmd.redirects.push(redirect.clone());
                    state.current_cmd.redirect_in = Some(redirect);
                    *i += 1;
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
                }
            }
        }
        TokenKind::Append => {
            note_command_line(&mut state.current_cmd, token);
            if let Some(next_i) = assign_append_target(tokens, *i, &mut state.current_cmd) {
                *i = next_i;
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
            }
        }
        TokenKind::HereDoc => {
            note_command_line(&mut state.current_cmd, token);
            if *i + 1 < tokens.len() {
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
                    state.current_cmd.heredoc_delimiter = Some(delimiter);
                }
                *i += 1;
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
            } else if *i + 1 < tokens.len()
                && matches!(
                    tokens[*i + 1].kind,
                    TokenKind::Word
                        | TokenKind::Variable
                        | TokenKind::CommandSubst
                        | TokenKind::Assignment
                )
            {
                assign_here_string_redirect_raw(
                    &mut state.current_cmd,
                    &token.value,
                    &tokens[*i + 1].value,
                    &tokens[*i + 1].raw,
                    redirect_fd_var_prefix(tokens, *i),
                );
                *i += 1;
            }
        }
        TokenKind::HereDocBody => {
            note_command_line(&mut state.current_cmd, token);
            assign_heredoc_body(&mut state.current_cmd, &mut state.ast, token.value.clone());
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

            if token.value == "!" && command_is_empty(&state.current_cmd) {
                // TODO(parse.y/execute_cmd.c): Bash represents `!` as a
                // pipeline/list inversion flag. Keep it on the next simple
                // command until the parser has a real pipeline state.ast.
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
                if let Some(source) = parse_error_source_line(tokens, *i) {
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
                if let Some(source) = parse_error_source_line(tokens, *i) {
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
                state.current_cmd.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    "unexpected token `('".to_string(),
                );
                if let Some(source) = parse_error_source_line(tokens, *i) {
                    state
                        .current_cmd
                        .insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
                }
                *i += 1;
                return TokenAction::Continue;
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

fn parse_error_source_line(tokens: &[Token], index: usize) -> Option<String> {
    tokens.get(index)?;
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

/// Name-side validator for atomic compound assignments: optional
/// `name[subscript]` head, then identifier rules (GNU arrayfunc.c).
fn valid_compound_assignment_lhs(lhs: &str) -> bool {
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
