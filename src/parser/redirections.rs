use super::*;
use crate::lexer::{Token, TokenKind};
use crate::executor::markers::STORAGE_WORD_PREFIX_STR;

pub(super) fn collect_trailing_redirections(
    tokens: &[Token],
    index: &mut usize,
    command: &mut CommandNode,
) {
    loop {
        let Some(token) = tokens.get(*index) else {
            break;
        };
        if matches!(
            token.kind,
            TokenKind::Word | TokenKind::BraceExpand | TokenKind::Keyword
        ) && redirect_fd_var_prefix(tokens, *index + 1).is_some()
            && tokens.get(*index + 1).is_some_and(|next| {
                matches!(
                    next.kind,
                    TokenKind::RedirectIn | TokenKind::RedirectOut | TokenKind::Append
                )
            })
        {
            *index += 1;
            continue;
        }

        if token.kind == TokenKind::HereDocBody {
            if fill_pending_heredoc_body(command, &token.value, token.position) {
                *index += 1;
                continue;
            }
            break;
        }

        if token.kind == TokenKind::RedirectIn {
            if let Some((mut process_substitution, next_i)) =
                process_substitution_redirect_target(tokens, *index)
            {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                let fd_var = redirect_fd_var_prefix(tokens, *index);
                process_substitution.redirect_fd = fd;
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                let redirect =
                    redirect_node_with_fd_var(&token.value, fd, fd_var, &target, false, false);
                command.redirects.push(redirect.clone());
                // A `{var}` redirect allocates a fresh descriptor >= 10
                // (GNU redir.c redir_varassign) — it never binds fd 0, so it
                // must not mirror into redirect_in.
                if redirect.fd_var.is_none() && redirect.fd.unwrap_or(0) == 0 {
                    command.redirect_in = Some(redirect);
                }
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::RedirectOut {
            if let Some((process_substitution, next_i)) =
                combined_process_substitution_redirect_target(tokens, *index)
            {
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                assign_output_redirect(command, &token.value, &target, None, None);
                *index = next_i + 1;
                continue;
            }

            if let Some((mut process_substitution, next_i)) =
                output_process_substitution_redirect_target(tokens, *index)
            {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                let fd_var = redirect_fd_var_prefix(tokens, *index);
                process_substitution.redirect_fd = fd;
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                command.redirect_out = Some(redirect_node_with_fd_var(
                    &token.value,
                    fd,
                    fd_var,
                    &target,
                    false,
                    false,
                ));
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::Append {
            if let Some((process_substitution, next_i)) =
                combined_process_substitution_redirect_target(tokens, *index)
            {
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                assign_append_redirect(command, &token.value, &target, None, None);
                *index = next_i + 1;
                continue;
            }
        }

        if matches!(
            token.kind,
            TokenKind::RedirectErr | TokenKind::RedirectErrAppend
        ) {
            if let Some((mut process_substitution, next_i)) =
                stderr_process_substitution_redirect_target(tokens, *index)
            {
                process_substitution.redirect_fd = Some(2);
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                if token.kind == TokenKind::RedirectErrAppend {
                    let redirect = redirect_node(&token.value, Some(2), &target, true, false);
                    command.redirects.push(redirect.clone());
                    command.redirect_err_append = Some(redirect);
                } else {
                    let redirect =
                        redirect_node(&token.value, Some(2), &target, false, token.value == "2>|");
                    command.redirects.push(redirect.clone());
                    command.redirect_err = Some(redirect);
                }
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::HereString {
            if let Some((process_substitution, next_i)) =
                any_process_substitution_word_target(tokens, *index + 1)
            {
                assign_here_string_process_substitution(
                    command,
                    &token.value,
                    process_substitution,
                    redirect_fd_var_prefix(tokens, *index),
                );
                *index = next_i + 1;
                continue;
            }
        }

        let Some(target) = redirect_target_token(tokens, *index) else {
            break;
        };

        match token.kind {
            TokenKind::RedirectIn => {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                let (dup_value, dup_raw) = dup_close_target(command, &token.value, target);
                let redirect = redirect_node_with_fd_var_raw(
                    &token.value,
                    fd,
                    redirect_fd_var_prefix(tokens, *index),
                    &input_redirect_target(&token.value, &dup_value),
                    &input_redirect_target(&token.value, &dup_raw),
                    false,
                    false,
                );
                command.redirects.push(redirect.clone());
                if redirect.fd_var.is_none() && redirect.fd.unwrap_or(0) == 0 {
                    command.redirect_in = Some(redirect);
                }
            }
            TokenKind::RedirectOut => {
                if token.value.ends_with("<>") {
                    let fd = redirect_operator_fd(&token.value)
                        .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                    let redirect = redirect_node_with_fd_var_raw(
                        &token.value,
                        fd,
                        redirect_fd_var_prefix(tokens, *index),
                        &target.value,
                        &target.raw,
                        true,
                        false,
                    );
                    command.redirects.push(redirect.clone());
                    if redirect.fd_var.is_none() && redirect.fd.unwrap_or(0) == 0 {
                        command.redirect_in = Some(redirect);
                    }
                } else {
                    let (dup_value, dup_raw) = dup_close_target(command, &token.value, target);
                    assign_output_redirect_raw(
                        command,
                        &token.value,
                        &dup_value,
                        &dup_raw,
                        None,
                        redirect_fd_var_prefix(tokens, *index),
                    );
                }
            }
            TokenKind::Append => {
                assign_append_redirect_raw(
                    command,
                    &token.value,
                    &target.value,
                    &target.raw,
                    None,
                    redirect_fd_var_prefix(tokens, *index),
                );
            }
            TokenKind::RedirectErr => {
                assign_redirect_err_target(tokens, *index, command);
            }
            TokenKind::RedirectErrAppend => {
                assign_redirect_err_append_target(tokens, *index, command);
            }
            TokenKind::HereString => {
                assign_here_string_redirect_raw(
                    command,
                    &token.value,
                    &target.value,
                    &target.raw,
                    redirect_fd_var_prefix(tokens, *index),
                );
            }
            TokenKind::HereDoc => {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                command.redirects.push(redirect_node_with_fd_var_raw(
                    &token.value,
                    fd,
                    redirect_fd_var_prefix(tokens, *index),
                    &target.value,
                    &target.raw,
                    false,
                    false,
                ));
                command.heredoc_redirects.push(heredoc_redirect(
                    &token.value,
                    target,
                    fd,
                    redirect_fd_var_prefix(tokens, *index),
                ));
                if fd.is_none() {
                    // GNU make_cmd.c make_here_document stores
                    // `here_doc_eof = string_quote_removal(word)` — the
                    // DEQUOTED delimiter; it is used both for body-line
                    // matching and for the `delimited by end-of-file
                    // (wanted `%s')` warning, so drop CTLESC pairs here.
                    command.heredoc_delimiter = Some(
                        target
                            .value
                            .replace(crate::executor::markers::CTLESC, ""),
                    );
                }
                *index += 2;
                continue;
            }
            _ => break,
        }

        *index += 2;
    }
}

pub(super) fn take_heredoc_fd_prefix(cmd: &mut CommandNode) -> Option<u32> {
    take_redirect_fd_prefix(cmd)
}

pub(super) fn assign_here_string_redirect(
    command: &mut CommandNode,
    operator: &str,
    target: &str,
    fd_var: Option<String>,
) {
    assign_here_string_redirect_raw(command, operator, target, target, fd_var);
}

pub(super) fn assign_here_string_redirect_raw(
    command: &mut CommandNode,
    operator: &str,
    target: &str,
    raw_target: &str,
    fd_var: Option<String>,
) {
    let fd = redirect_operator_fd(operator);
    command.redirects.push(redirect_node_with_fd_var_raw(
        operator,
        fd,
        fd_var.clone(),
        target,
        raw_target,
        false,
        false,
    ));
    // GNU parse.y: `{var}<<<word` is a REDIR_WORD herestring — the word
    // binds a freshly allocated descriptor (redir.c redir_varassign), not
    // fd 0, so it must not take the `here_string` stdin mirror.
    if fd.is_some() || fd_var.is_some() {
        command.heredoc_redirects.push(HereDocRedirect {
            fd,
            fd_var: fd_var.clone(),
            operator: operator.to_string(),
            operator_metadata: Box::new(build_word_metadata(0, operator, operator)),
            delimiter: "<<<".to_string(),
            delimiter_metadata: Box::new(build_word_metadata(0, "<<<", "<<<")),
            strip_tabs: false,
            quoted_delimiter: false,
            here_string: true,
            body: Some(format!("{STORAGE_WORD_PREFIX_STR}{target}")),
            body_carrier: None,
            gather_line: None,
        });
    } else {
        command.here_string = Some(encode_stdin_body_enq(target));
    }
}

pub(super) fn assign_here_string_process_substitution(
    command: &mut CommandNode,
    operator: &str,
    mut process_substitution: ProcessSubstitution,
    fd_var: Option<String>,
) {
    let fd = redirect_operator_fd(operator);
    process_substitution.redirect_fd = fd;
    let target = process_substitution.target.clone();
    command.process_substitutions.push(process_substitution);
    assign_here_string_redirect(command, operator, &target, fd_var);
}

/// GNU parse.y:3802-3807: after `<&`/`>&` (optionally fd-prefixed) a `-`
/// is its own close token — `exec <&-1` is `exec <&-` (close fd 0) plus
/// operand `1` (`exec: 1: not found`), never a dup of fd "-1". When the
/// target word starts with `-` and has more characters, returns the `-`
/// close target and pushes the remainder onto the command's word list.
pub(super) fn dup_close_target(
    command: &mut CommandNode,
    operator: &str,
    target: &Token,
) -> (String, String) {
    if !(operator.ends_with("<&") || operator.ends_with(">&")) {
        return (target.value.clone(), target.raw.clone());
    }
    let Some(rest) = target.value.strip_prefix('-').filter(|rest| !rest.is_empty()) else {
        return (target.value.clone(), target.raw.clone());
    };
    let raw_rest = target
        .raw
        .strip_prefix('-')
        .filter(|raw| !raw.is_empty())
        .unwrap_or(rest);
    let word = Token::new_with_raw(TokenKind::Word, rest, raw_rest, target.position + 1);
    push_command_word(command, &word);
    ("-".to_string(), "-".to_string())
}

pub(super) fn redirect_target_token(tokens: &[Token], index: usize) -> Option<&Token> {
    tokens
        .get(index + 1)
        .filter(|target| is_redirect_target_token(target))
}

pub(super) fn is_redirect_target_token(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Word
            | TokenKind::Variable
            | TokenKind::Assignment
            | TokenKind::CommandSubst
            | TokenKind::BraceExpand
            | TokenKind::HereDocBody
    )
}

pub(super) fn take_adjacent_redirect_fd_prefix(
    cmd: &mut CommandNode,
    tokens: &[Token],
    redirect_index: usize,
) -> Option<u32> {
    let previous = redirect_index
        .checked_sub(1)
        .and_then(|index| tokens.get(index))?;
    let redirect = tokens.get(redirect_index)?;
    if previous.column + previous.value.len() != redirect.column {
        return None;
    }
    take_redirect_fd_prefix(cmd)
}

pub(super) fn redirect_fd_var_prefix(tokens: &[Token], redirect_index: usize) -> Option<String> {
    let redirect = tokens.get(redirect_index)?;
    let previous = redirect_index
        .checked_sub(1)
        .and_then(|index| tokens.get(index))?;
    // GNU read_token_word (parse.y:5821-5843): `{varname}` becomes a
    // REDIR_WORD only when the `}' is immediately followed by `<' or `>'
    // (the bare redirection operator — a digit-prefixed `{fd}2>f` is the
    // word `{fd}2` plus `>f`, and a spaced `{fd} <f` keeps `{fd}` an
    // ordinary word).  `character` there is the byte right after the word;
    // the token columns reproduce exactly that adjacency test.
    if !matches!(redirect.value.chars().next(), Some('<' | '>'))
        || redirect.column != previous.column + previous.raw.len()
    {
        return None;
    }
    // GNU checks the raw token text (`valid_identifier(token+1)`), so a
    // quoted name like `{"fd"}<f` stays an ordinary word.
    let name = previous.raw.strip_prefix('{')?.strip_suffix('}')?;
    if is_shell_identifier(name) {
        return Some(name.to_string());
    }
    // bash's REDIR_VARASSIGN also accepts array elements: {fd[0]}>&1.
    // Same grammar as the executor's dynamic_fd_var_name word path.
    let (array_name, index) = name.split_once('[')?;
    let index = index.strip_suffix(']')?;
    if is_shell_identifier(array_name)
        && !index.is_empty()
        && index.chars().all(|ch| ch.is_ascii_digit())
    {
        Some(name.to_string())
    } else {
        None
    }
}

fn is_shell_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    matches!(first, '_' | 'a'..='z' | 'A'..='Z')
        && chars.all(|ch| matches!(ch, '_' | 'a'..='z' | 'A'..='Z' | '0'..='9'))
}

pub(super) fn redirect_operator_fd(operator: &str) -> Option<u32> {
    let digits = operator
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

pub(super) fn redirect_node(
    operator: &str,
    fd: Option<u32>,
    target: &str,
    append: bool,
    clobber: bool,
) -> Redirect {
    redirect_node_with_raw(operator, fd, target, target, append, clobber)
}

pub(super) fn redirect_node_with_raw(
    operator: &str,
    fd: Option<u32>,
    target: &str,
    raw_target: &str,
    append: bool,
    clobber: bool,
) -> Redirect {
    // GNU make_redirection (make_cmd.c) stores the redirector fd on the
    // REDIRECT itself; a digit prefix fused into the operator token
    // (`3>&1`, `10>f`) is the same fd. Normalize it into `fd` so every
    // consumer sees the effective source descriptor instead of peeking at
    // operator text. `>&word`/`&>file` carry no digits and stay None.
    let fd = fd.or_else(|| redirect_operator_fd(operator));
    Redirect {
        fd,
        fd_var: None,
        operator: operator.to_string(),
        operator_metadata: Box::new(build_word_metadata(0, operator, operator)),
        kind: redirect_kind(operator, target),
        target: target.to_string(),
        target_metadata: Box::new(build_word_metadata(0, target, raw_target)),
        append,
        clobber,
    }
}

pub(super) fn redirect_node_with_fd_var(
    operator: &str,
    fd: Option<u32>,
    fd_var: Option<String>,
    target: &str,
    append: bool,
    clobber: bool,
) -> Redirect {
    let mut redirect = redirect_node(operator, fd, target, append, clobber);
    redirect.fd_var = fd_var;
    redirect
}

pub(super) fn redirect_node_with_fd_var_raw(
    operator: &str,
    fd: Option<u32>,
    fd_var: Option<String>,
    target: &str,
    raw_target: &str,
    append: bool,
    clobber: bool,
) -> Redirect {
    let mut redirect = redirect_node_with_raw(operator, fd, target, raw_target, append, clobber);
    redirect.fd_var = fd_var;
    redirect
}

pub(super) fn redirect_kind(operator: &str, target: &str) -> RedirectKind {
    if operator.ends_with("<&") {
        return if target == "&-" {
            RedirectKind::CloseInput
        } else {
            RedirectKind::DuplicateInput
        };
    }
    if operator.ends_with(">&") {
        return if target == "&-" {
            RedirectKind::CloseOutput
        } else {
            RedirectKind::DuplicateOutput
        };
    }
    if operator.ends_with("<>") {
        return RedirectKind::ReadWrite;
    }
    if operator == "&>" {
        return RedirectKind::CombinedOutput;
    }
    if operator == "&>>" {
        return RedirectKind::CombinedAppend;
    }
    if operator.ends_with("<<<") {
        return RedirectKind::HereString;
    }
    if operator.ends_with("<<-") || operator.ends_with("<<") {
        return RedirectKind::HereDoc;
    }
    if operator.ends_with(">>") {
        return RedirectKind::Append;
    }
    if operator.ends_with(">|") {
        return RedirectKind::ClobberOutput;
    }
    if operator.ends_with('>') {
        return RedirectKind::Output;
    }
    if operator.ends_with('<') {
        return RedirectKind::Input;
    }
    RedirectKind::Unknown
}

pub(super) fn redirect_target(operator: &str, target: &str) -> String {
    if operator.ends_with(">&") {
        format!("&{target}")
    } else {
        target.to_string()
    }
}

pub(super) fn input_redirect_target(operator: &str, target: &str) -> String {
    if operator.ends_with("<&") {
        format!("&{target}")
    } else {
        target.to_string()
    }
}

pub(super) fn take_redirect_fd_prefix(cmd: &mut CommandNode) -> Option<u32> {
    let fd = cmd
        .words
        .last()
        .filter(|word| !word.is_empty() && word.chars().all(|ch| ch.is_ascii_digit()))?
        .parse::<u32>()
        .ok()?;
    cmd.words.pop();
    cmd.word_kinds.pop();
    Some(fd)
}

pub(super) fn assign_heredoc_body(
    current_cmd: &mut CommandNode,
    ast: &mut Ast,
    body: String,
    gather_line: usize,
) {
    for command in ast.commands.iter_mut() {
        if fill_pending_heredoc_body_recursive(command, &body, gather_line) {
            return;
        }
    }
    if fill_pending_heredoc_body_recursive(current_cmd, &body, gather_line) {
        return;
    }
    current_cmd.heredoc = Some(encode_stdin_body_enq(&body));
}

fn fill_pending_heredoc_body_recursive(
    cmd: &mut CommandNode,
    body: &str,
    gather_line: usize,
) -> bool {
    if fill_pending_heredoc_body(cmd, body, gather_line) {
        return true;
    }

    if let Some(pipeline) = &mut cmd.pipeline_command {
        if fill_pending_heredoc_body_in_commands(&mut pipeline.stages, body, gather_line) {
            return true;
        }
    }
    if let Some(list) = &mut cmd.and_or_list {
        if fill_pending_heredoc_body_in_commands(&mut list.commands, body, gather_line) {
            return true;
        }
    }
    if let Some(time) = &mut cmd.time_command {
        if fill_pending_heredoc_body_recursive(&mut time.command, body, gather_line) {
            return true;
        }
    }
    if let Some(background) = &mut cmd.background_command {
        if fill_pending_heredoc_body_recursive(&mut background.command, body, gather_line) {
            return true;
        }
    }
    if let Some(inverted) = &mut cmd.inverted_command {
        if fill_pending_heredoc_body_recursive(&mut inverted.command, body, gather_line) {
            return true;
        }
    }
    if let Some(for_command) = &mut cmd.for_command {
        if fill_pending_heredoc_body_in_commands(&mut for_command.body, body, gather_line) {
            return true;
        }
    }
    if let Some(if_command) = &mut cmd.if_command {
        if fill_pending_heredoc_body_in_commands(&mut if_command.condition, body, gather_line)
            || fill_pending_heredoc_body_in_commands(&mut if_command.then_body, body, gather_line)
            || if_command.elif_branches.iter_mut().any(|branch| {
                fill_pending_heredoc_body_in_commands(&mut branch.condition, body, gather_line)
                    || fill_pending_heredoc_body_in_commands(&mut branch.body, body, gather_line)
            })
            || if_command.else_body.as_mut().is_some_and(|commands| {
                fill_pending_heredoc_body_in_commands(commands, body, gather_line)
            })
        {
            return true;
        }
    }
    if let Some(loop_command) = &mut cmd.loop_command {
        if fill_pending_heredoc_body_in_commands(&mut loop_command.condition, body, gather_line)
            || fill_pending_heredoc_body_in_commands(&mut loop_command.body, body, gather_line)
        {
            return true;
        }
    }
    if let Some(subshell) = &mut cmd.subshell_command {
        if fill_pending_heredoc_body_in_commands(&mut subshell.body, body, gather_line) {
            return true;
        }
    }
    if let Some(case_command) = &mut cmd.case_command {
        if case_command.clauses.iter_mut().any(|clause| {
            fill_pending_heredoc_body_in_commands(&mut clause.body, body, gather_line)
        }) {
            return true;
        }
    }
    if let Some(select_command) = &mut cmd.select_command {
        if fill_pending_heredoc_body_in_commands(&mut select_command.body, body, gather_line) {
            return true;
        }
    }
    if let Some(function) = &mut cmd.function_command {
        if fill_pending_heredoc_body_in_commands(&mut function.body, body, gather_line) {
            return true;
        }
    }
    if let Some(brace_group) = &mut cmd.brace_group {
        if fill_pending_heredoc_body_in_commands(&mut brace_group.body, body, gather_line) {
            return true;
        }
    }
    if let Some(coproc) = &mut cmd.coproc_command {
        if coproc.body.as_mut().is_some_and(|commands| {
            fill_pending_heredoc_body_in_commands(commands, body, gather_line)
        }) {
            return true;
        }
    }

    false
}

fn fill_pending_heredoc_body_in_commands(
    commands: &mut [CommandNode],
    body: &str,
    gather_line: usize,
) -> bool {
    commands
        .iter_mut()
        .any(|command| fill_pending_heredoc_body_recursive(command, body, gather_line))
}

/// A literal ENQ (0x05) byte in collected stdin-body text is
/// indistinguishable from the executor's PREEXPANDED_STDIN_BODY sentinel
/// (execution_misc.rs): a heredoc body starting with a raw 0x05 would look
/// already-expanded and skip expansion entirely. Encode literal ENQ chars
/// as raw-byte marker pairs; the marker decodes back to 0x05 at the byte
/// boundary (substitution_metadata::shell_text_to_raw_bytes). Same for a
/// here-string word, which expand_here_string_mut probes with the same
/// sentinel check.
fn encode_stdin_body_enq(text: &str) -> String {
    if !text.contains('\u{5}') {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\u{5}' {
            output.push(
                char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
                    .expect("sentinel is valid"),
            );
            output.push(
                char::from_u32(
                    crate::executor::substitution_metadata::RAW_BYTE_MARKER_FIRST + 0x05,
                )
                .expect("marker char is valid"),
            );
        } else {
            output.push(ch);
        }
    }
    output
}

pub(super) fn fill_pending_heredoc_body(
    cmd: &mut CommandNode,
    body: &str,
    gather_line: usize,
) -> bool {
    let Some(redirect) = cmd
        .heredoc_redirects
        .iter_mut()
        .find(|redirect| redirect.body.is_none())
    else {
        return false;
    };

    let body = encode_stdin_body_enq(body);
    redirect.body = Some(body.clone());
    redirect.gather_line = Some(gather_line);
    if redirect.fd.is_none() {
        cmd.heredoc = Some(body);
        // GNU stores the dequoted delimiter (`here_doc_eof = redir_word`),
        // so CTLESC pairs must not reach the warning text either.
        cmd.heredoc_delimiter = Some(
            redirect
                .delimiter
                .replace(crate::executor::markers::CTLESC, ""),
        );
        cmd.heredoc_gather_line = Some(gather_line);
    }
    true
}

pub(super) fn heredoc_redirect(
    operator: &str,
    delimiter: &Token,
    fd: Option<u32>,
    fd_var: Option<String>,
) -> HereDocRedirect {
    HereDocRedirect {
        fd,
        fd_var,
        operator: operator.to_string(),
        operator_metadata: Box::new(build_word_metadata(0, operator, operator)),
        delimiter: delimiter.value.clone(),
        delimiter_metadata: Box::new(build_word_metadata(0, &delimiter.value, &delimiter.raw)),
        strip_tabs: operator.ends_with("<<-"),
        quoted_delimiter: delimiter
            .raw
            .chars()
            .any(|ch| matches!(ch, '\'' | '"' | '\\')),
        here_string: false,
        body: None,
        body_carrier: None,
        gather_line: None,
    }
}
