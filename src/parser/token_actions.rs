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
            if let Some((value, raw, next_i)) =
                collect_adjacent_process_substitution_word(tokens, *i)
            {
                push_synthetic_process_substitution_word(&mut state.current_cmd, &value, &raw);
                *i = next_i;
            } else {
                push_command_word(&mut state.current_cmd, token);
            }
        }
        TokenKind::Assignment => {
            state.current_cmd.subshell |= state.in_subshell;
            note_command_line(&mut state.current_cmd, token);
            if let Some(pos) = token.value.find('=') {
                if state.current_cmd.words.is_empty() {
                    let var_name = token.value[..pos].to_string();
                    let mut var_value = token.value[pos + 1..].to_string();
                    if var_value.is_empty() {
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
                            var_value = format!("\x1e{compound_value}");
                            *i = next_i;
                        }
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
                    let raw_assignment_value = token.raw.split_once('=').map(|(_, raw)| raw);
                    record_brace_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        raw_assignment_value.unwrap_or(&var_value),
                        None,
                    );
                    record_extglob_patterns_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        raw_assignment_value.unwrap_or(&var_value),
                        None,
                    );
                    record_tilde_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        raw_assignment_value.unwrap_or(&var_value),
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
                    state.current_cmd.assignments.insert(var_name, var_value);
                } else {
                    let mut word = token.value.clone();
                    let raw_word = token.raw.clone();
                    if word.ends_with('=') {
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
                            word.push('\x1e');
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

            if token.value == "(" && command_is_empty(&state.current_cmd) {
                state.in_subshell = true;
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
