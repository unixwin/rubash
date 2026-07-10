use super::*;

pub(in crate::executor) fn unset_args_need_builtin_diagnostics(args: &[String]) -> bool {
    let mut functions = false;
    let mut variables = false;

    for arg in args {
        if arg == "--" || arg == "-" {
            break;
        }
        if !arg.starts_with('-') {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'f' => functions = true,
                'v' => variables = true,
                'n' => {}
                _ => return true,
            }
        }
    }

    functions && variables
}

pub(in crate::executor) fn command_has_no_effect(cmd: &CommandNode) -> bool {
    cmd.assignments.is_empty()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.here_string.is_none()
        && cmd.pipe.is_none()
        && cmd.and_or.is_none()
        && !cmd.background
        && !cmd.inverted
        && !cmd.subshell
        && !cmd.subshell_end
        && cmd.for_command.is_none()
        && cmd.case_command.is_none()
        && cmd.function_command.is_none()
}

pub(in crate::executor) fn normalize_leading_assignment_words(cmd: &mut CommandNode) {
    let mut count = 0;
    while let Some(word) = cmd.words.get(count) {
        let Some((name, value)) = split_assignment_word(word) else {
            break;
        };
        cmd.assignments.insert(name.to_string(), value.to_string());
        count += 1;
    }
    if count > 0 {
        cmd.words.drain(0..count);
    }
}

pub(in crate::executor) fn command_has_redirect(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
}

pub(in crate::executor) fn function_body_needs_command_terminators(body: &[CommandNode]) -> bool {
    body.iter().any(|command| command.heredoc.is_some())
}

pub(in crate::executor) fn function_definition_command_omits_terminator(
    command: &CommandNode,
) -> bool {
    command.heredoc.is_some()
        || matches!(
            command.words.first().map(String::as_str),
            Some("then" | "do" | "else" | "elif" | "fi" | "done")
        )
}

pub(in crate::executor) fn function_definition_command_closes_block(command: &CommandNode) -> bool {
    matches!(
        command.words.first().map(String::as_str),
        Some("else" | "elif" | "fi" | "done")
    )
}

pub(in crate::executor) fn function_definition_command_opens_nested_body(
    command: &CommandNode,
) -> bool {
    matches!(
        command.words.first().map(String::as_str),
        Some("then" | "do" | "else" | "elif")
    )
}

pub(in crate::executor) fn write_function_definition_heredoc_body<W>(
    command: &CommandNode,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let (Some(body), Some(delimiter)) = (&command.heredoc, &command.heredoc_delimiter) else {
        return Ok(());
    };
    let body = body.strip_prefix('\x1e').unwrap_or(body);
    write!(stdout, "{body}")?;
    writeln!(stdout, "{delimiter}")?;
    Ok(())
}

pub(in crate::executor) fn append_function_redirect(
    line: &mut String,
    redirect: Option<&crate::parser::Redirect>,
    op: &str,
) {
    if let Some(redirect) = redirect {
        line.push(' ');
        line.push_str(op);
        line.push(' ');
        line.push_str(&redirect.target);
    }
}

pub(in crate::executor) fn fd_stdin_key(fd: u32) -> String {
    format!("{FD_STDIN_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_stdin_offset_key(fd: u32) -> String {
    format!("{FD_STDIN_OFFSET_PREFIX}{fd}")
}

pub(in crate::executor) fn command_has_output_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
}

pub(in crate::executor) fn command_has_input_or_output_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.heredoc.is_some()
        || cmd.here_string.is_some()
        || command_has_output_redirects(cmd)
}

pub(in crate::executor) fn bash_command_text(cmd: &CommandNode) -> String {
    let mut parts = Vec::new();
    for (name, value) in &cmd.assignments {
        parts.push(format!("{name}={value}"));
    }
    parts.extend(cmd.words.iter().cloned());

    if let Some(redirect) = &cmd.redirect_in {
        parts.push(format_redirect("<", redirect));
    }
    if let Some(redirect) = &cmd.redirect_out {
        parts.push(format_redirect(
            if redirect.clobber { ">|" } else { ">" },
            redirect,
        ));
    }
    if let Some(redirect) = &cmd.append {
        parts.push(format_redirect(">>", redirect));
    }
    if let Some(redirect) = &cmd.redirect_err {
        parts.push(format_redirect("2>", redirect));
    }
    if let Some(redirect) = &cmd.redirect_err_append {
        parts.push(format_redirect("2>>", redirect));
    }
    if let Some(here_string) = &cmd.here_string {
        parts.push(format!("<<< {here_string}"));
    }

    parts.join(" ")
}

pub(in crate::executor) fn function_here_string_text(
    value: &str,
    multi_command_body: bool,
) -> String {
    if value.contains('$') {
        return format!("\"{}\"", value.replace('"', "\\\""));
    }

    if value.contains(char::is_whitespace) || value.contains('"') {
        return shell_single_quote_assignment_value(value);
    }

    if multi_command_body {
        return format!("\"{}\"", value.replace('"', "\\\""));
    }

    value.to_string()
}
