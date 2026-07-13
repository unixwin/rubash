use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn assign_redirect_out_target(
    tokens: &[Token],
    index: usize,
    command: &mut CommandNode,
) -> Option<usize> {
    if let Some((process_substitution, next_i)) =
        combined_process_substitution_redirect_target(tokens, index)
    {
        let target = process_substitution.target.clone();
        command.process_substitutions.push(process_substitution);
        assign_output_redirect(command, &tokens[index].value, &target, None, None);
        return Some(next_i);
    }

    if let Some((mut process_substitution, next_i)) =
        output_process_substitution_redirect_target(tokens, index)
    {
        let fd = redirect_operator_fd(&tokens[index].value)
            .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, index));
        let fd_var = redirect_fd_var_prefix(tokens, index);
        process_substitution.redirect_fd = fd;
        let target = process_substitution.target.clone();
        command.process_substitutions.push(process_substitution);
        let redirect =
            redirect_node_with_fd_var(&tokens[index].value, fd, fd_var, &target, false, false);
        command.redirects.push(redirect.clone());
        command.redirect_out = Some(redirect);
        return Some(next_i);
    }

    if let Some((mut process_substitution, next_i)) =
        output_process_substitution_word_target(tokens, index)
    {
        process_substitution.word_index = Some(command.words.len());
        let target = process_substitution.target.clone();
        command.process_substitutions.push(process_substitution);
        command.words.push(target);
        command.word_kinds.push(TokenKind::Word);
        return Some(next_i);
    }

    let target = redirect_target_token(tokens, index)?;
    let fd = redirect_operator_fd(&tokens[index].value)
        .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, index));
    let fd_var = redirect_fd_var_prefix(tokens, index);
    if tokens[index].value.ends_with("<>") {
        let redirect =
            redirect_node_with_fd_var(&tokens[index].value, fd, fd_var, &target.value, true, false);
        command.redirects.push(redirect.clone());
        command.redirect_in = Some(redirect);
        return Some(index + 1);
    }
    assign_output_redirect(command, &tokens[index].value, &target.value, fd, fd_var);
    Some(index + 1)
}

pub(super) fn assign_append_target(
    tokens: &[Token],
    index: usize,
    command: &mut CommandNode,
) -> Option<usize> {
    if let Some((process_substitution, next_i)) =
        combined_process_substitution_redirect_target(tokens, index)
    {
        let target = process_substitution.target.clone();
        command.process_substitutions.push(process_substitution);
        assign_append_redirect(command, &tokens[index].value, &target, None, None);
        return Some(next_i);
    }

    if let Some((mut process_substitution, next_i)) =
        append_process_substitution_redirect_target(tokens, index)
    {
        let fd = redirect_operator_fd(&tokens[index].value)
            .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, index));
        let fd_var = redirect_fd_var_prefix(tokens, index);
        process_substitution.redirect_fd = fd;
        let target = process_substitution.target.clone();
        command.process_substitutions.push(process_substitution);
        let redirect =
            redirect_node_with_fd_var(&tokens[index].value, fd, fd_var, &target, true, false);
        command.redirects.push(redirect.clone());
        command.append = Some(redirect);
        return Some(next_i);
    }

    let target = redirect_target_token(tokens, index)?;
    let fd = redirect_operator_fd(&tokens[index].value)
        .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, index));
    assign_append_redirect(
        command,
        &tokens[index].value,
        &target.value,
        fd,
        redirect_fd_var_prefix(tokens, index),
    );
    Some(index + 1)
}

pub(super) fn assign_redirect_err_target(
    tokens: &[Token],
    index: usize,
    command: &mut CommandNode,
) -> Option<usize> {
    let target = redirect_target_token(tokens, index)?;
    let target_value = redirect_target(&tokens[index].value, &target.value);
    command.redirects.push(redirect_node(
        &tokens[index].value,
        Some(2),
        &target_value,
        false,
        tokens[index].value == "2>|",
    ));
    if target_value == "&1" {
        if let Some(redirect) = command
            .append
            .clone()
            .or_else(|| command.redirect_out.clone())
        {
            command.redirect_err_append = Some(redirect_node(
                &tokens[index].value,
                Some(2),
                &redirect.target,
                true,
                false,
            ));
            return Some(index + 1);
        }
    }

    command.redirect_err = Some(redirect_node(
        &tokens[index].value,
        Some(2),
        &target_value,
        false,
        tokens[index].value == "2>|",
    ));
    Some(index + 1)
}

pub(super) fn assign_redirect_err_append_target(
    tokens: &[Token],
    index: usize,
    command: &mut CommandNode,
) -> Option<usize> {
    let target = redirect_target_token(tokens, index)?;
    let redirect = redirect_node(&tokens[index].value, Some(2), &target.value, true, false);
    command.redirects.push(redirect.clone());
    command.redirect_err_append = Some(redirect);
    Some(index + 1)
}

pub(super) fn assign_output_redirect(
    command: &mut CommandNode,
    operator: &str,
    target: &str,
    fd: Option<u32>,
    fd_var: Option<String>,
) {
    let redirect = redirect_node_with_fd_var(
        operator,
        fd,
        fd_var,
        &redirect_target(operator, target),
        false,
        operator == ">|",
    );
    command.redirects.push(redirect.clone());
    command.redirect_out = Some(redirect);
    if operator == "&>" {
        command.redirect_err_append = Some(redirect_node(operator, Some(2), target, true, false));
    }
}

pub(super) fn assign_append_redirect(
    command: &mut CommandNode,
    operator: &str,
    target: &str,
    fd: Option<u32>,
    fd_var: Option<String>,
) {
    let redirect = redirect_node_with_fd_var(operator, fd, fd_var, target, true, false);
    command.redirects.push(redirect.clone());
    command.append = Some(redirect);
    if operator == "&>>" {
        command.redirect_err_append = Some(redirect_node(operator, Some(2), target, true, false));
    }
}
