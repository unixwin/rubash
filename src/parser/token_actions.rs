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
            push_command_word(&mut state.current_cmd, token);
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
                    record_brace_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        None,
                    );
                    record_extglob_patterns_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
                        None,
                    );
                    record_tilde_expansions_for_assignment(
                        &mut state.current_cmd,
                        assignment_name,
                        &var_value,
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
                        record_brace_expansions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            Some(word_index),
                        );
                        record_extglob_patterns_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
                            Some(word_index),
                        );
                        record_tilde_expansions_for_assignment(
                            &mut state.current_cmd,
                            assignment_name.strip_suffix('+').unwrap_or(assignment_name),
                            value,
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
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
        }
        TokenKind::RedirectIn => {
            if command_is_open_conditional(&state.current_cmd) {
                push_command_word(&mut state.current_cmd, token);
            } else {
                note_command_line(&mut state.current_cmd, token);
                let fd = redirect_operator_fd(&token.value).or_else(|| {
                    take_adjacent_redirect_fd_prefix(&mut state.current_cmd, tokens, *i)
                });
                if let Some((mut process_substitution, next_i)) =
                    process_substitution_redirect_target(tokens, *i)
                {
                    process_substitution.redirect_fd = fd;
                    let target = process_substitution.target.clone();
                    state
                        .current_cmd
                        .process_substitutions
                        .push(process_substitution);
                    state.current_cmd.redirect_in =
                        Some(redirect_node(&token.value, fd, &target, false, false));
                    *i = next_i;
                } else if let Some((mut process_substitution, next_i)) =
                    process_substitution_word_target(tokens, *i)
                {
                    process_substitution.word_index = Some(state.current_cmd.words.len());
                    let target = process_substitution.target.clone();
                    state
                        .current_cmd
                        .process_substitutions
                        .push(process_substitution);
                    state.current_cmd.words.push(target);
                    state.current_cmd.word_kinds.push(TokenKind::Word);
                    *i = next_i;
                } else if *i + 1 < tokens.len() && is_redirect_target_token(&tokens[*i + 1]) {
                    state.current_cmd.redirect_in = Some(redirect_node(
                        &token.value,
                        fd,
                        &input_redirect_target(&token.value, &tokens[*i + 1].value),
                        false,
                        false,
                    ));
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
                state.current_cmd.redirect_err = Some(redirect_node(
                    &token.value,
                    Some(2),
                    &target,
                    false,
                    token.value == "2>|",
                ));
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
                state.current_cmd.redirect_err_append =
                    Some(redirect_node(&token.value, Some(2), &target, true, false));
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
                state.current_cmd.heredoc_redirects.push(heredoc_redirect(
                    &token.value,
                    delimiter_token,
                    fd,
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
                assign_here_string_redirect(
                    &mut state.current_cmd,
                    &token.value,
                    &tokens[*i + 1].value,
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
