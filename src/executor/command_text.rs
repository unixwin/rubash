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
        && cmd.arithmetic_command.is_none()
        && cmd.if_command.is_none()
        && cmd.loop_command.is_none()
        && cmd.conditional_command.is_none()
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
    body.iter().any(command_or_compound_has_heredoc)
}

pub(in crate::executor) fn function_definition_command_is_printable(command: &CommandNode) -> bool {
    !command.words.is_empty()
        || command.arithmetic_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.conditional_command.is_some()
}

fn command_or_compound_has_heredoc(command: &CommandNode) -> bool {
    command.heredoc.is_some()
        || command.if_command.as_ref().is_some_and(|if_command| {
            if_command
                .condition
                .iter()
                .any(command_or_compound_has_heredoc)
                || if_command
                    .then_body
                    .iter()
                    .any(command_or_compound_has_heredoc)
                || if_command.elif_branches.iter().any(|branch| {
                    branch.condition.iter().any(command_or_compound_has_heredoc)
                        || branch.body.iter().any(command_or_compound_has_heredoc)
                })
                || if_command
                    .else_body
                    .as_ref()
                    .is_some_and(|body| body.iter().any(command_or_compound_has_heredoc))
        })
        || command.loop_command.as_ref().is_some_and(|loop_command| {
            loop_command
                .condition
                .iter()
                .any(command_or_compound_has_heredoc)
                || loop_command
                    .body
                    .iter()
                    .any(command_or_compound_has_heredoc)
        })
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

pub(in crate::executor) fn bash_command_sequence_text(commands: &[CommandNode]) -> String {
    commands
        .iter()
        .map(bash_command_source_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

pub(in crate::executor) fn bash_command_source_text(cmd: &CommandNode) -> String {
    let mut text = if let Some(for_command) = &cmd.for_command {
        for_command_source_text(for_command)
    } else if let Some(arithmetic_command) = &cmd.arithmetic_command {
        arithmetic_command_source_text(arithmetic_command)
    } else if let Some(if_command) = &cmd.if_command {
        if_command_source_text(if_command)
    } else if let Some(loop_command) = &cmd.loop_command {
        loop_command_source_text(loop_command)
    } else if let Some(conditional_command) = &cmd.conditional_command {
        conditional_command_source_text(conditional_command)
    } else if let Some(select_command) = &cmd.select_command {
        select_command_source_text(select_command)
    } else if let Some(case_command) = &cmd.case_command {
        case_command_source_text(case_command)
    } else if let Some(coproc_command) = &cmd.coproc_command {
        coproc_command_source_text(coproc_command)
    } else if let Some(body) = &cmd.brace_group {
        format!("{{ {}; }}", bash_command_sequence_text(body))
    } else {
        bash_command_text(cmd)
    };
    append_source_redirects(&mut text, cmd);
    text
}

fn for_command_source_text(for_command: &ForCommand) -> String {
    let body = bash_command_sequence_text(&for_command.body);
    if let Some(arithmetic) = &for_command.arithmetic {
        return format!(
            "for (( {}; {}; {} )); do {}; done",
            arithmetic.init, arithmetic.test, arithmetic.update, body
        );
    }

    if for_command.default_positional {
        format!("for {}; do {}; done", for_command.variable, body)
    } else {
        format!(
            "for {} in {}; do {}; done",
            for_command.variable,
            for_command.words.join(" "),
            body
        )
    }
}

fn arithmetic_command_source_text(arithmetic_command: &ArithmeticCommand) -> String {
    format!("(( {} ))", arithmetic_command.expression)
}

fn if_command_source_text(if_command: &IfCommand) -> String {
    let mut text = format!(
        "if {}; then {}",
        bash_command_sequence_text(&if_command.condition),
        bash_command_sequence_text(&if_command.then_body)
    );
    for branch in &if_command.elif_branches {
        text.push_str(&format!(
            "; elif {}; then {}",
            bash_command_sequence_text(&branch.condition),
            bash_command_sequence_text(&branch.body)
        ));
    }
    if let Some(body) = &if_command.else_body {
        text.push_str(&format!("; else {}", bash_command_sequence_text(body)));
    }
    text.push_str("; fi");
    text
}

fn loop_command_source_text(loop_command: &LoopCommand) -> String {
    format!(
        "{} {}; do {}; done",
        if loop_command.until { "until" } else { "while" },
        bash_command_sequence_text(&loop_command.condition),
        bash_command_sequence_text(&loop_command.body)
    )
}

fn conditional_command_source_text(conditional_command: &ConditionalCommand) -> String {
    format!("[[ {}", conditional_command.args.join(" "))
}

fn select_command_source_text(select_command: &SelectCommand) -> String {
    let body = bash_command_sequence_text(&select_command.body);
    if select_command.default_positional {
        format!("select {}; do {}; done", select_command.variable, body)
    } else {
        format!(
            "select {} in {}; do {}; done",
            select_command.variable,
            select_command.words.join(" "),
            body
        )
    }
}

fn case_command_source_text(case_command: &CaseCommand) -> String {
    let mut text = format!("case {} in", case_command.word);
    for clause in &case_command.clauses {
        text.push(' ');
        text.push_str(&clause.patterns.join("|"));
        text.push_str(") ");
        text.push_str(&bash_command_sequence_text(&clause.body));
        text.push(' ');
        text.push_str(match clause.terminator {
            CaseTerminator::Break => ";;",
            CaseTerminator::FallThrough => ";&",
            CaseTerminator::TestNext => ";;&",
        });
    }
    text.push_str(" esac");
    text
}

fn coproc_command_source_text(coproc_command: &crate::parser::CoprocCommand) -> String {
    let mut text = String::from("coproc");
    if let Some(name) = &coproc_command.name {
        text.push(' ');
        text.push_str(name);
    }
    text.push(' ');
    if let Some(body) = &coproc_command.body {
        text.push_str("{ ");
        text.push_str(&bash_command_sequence_text(body));
        text.push_str("; }");
    } else {
        text.push_str(&coproc_command.words.join(" "));
    }
    text
}

fn append_source_redirects(text: &mut String, cmd: &CommandNode) {
    append_function_redirect(text, cmd.redirect_in.as_ref(), "<");
    append_function_redirect(
        text,
        cmd.redirect_out.as_ref(),
        cmd.redirect_out
            .as_ref()
            .filter(|redirect| redirect.clobber)
            .map(|_| ">|")
            .unwrap_or(">"),
    );
    append_function_redirect(text, cmd.append.as_ref(), ">>");
    append_function_redirect(text, cmd.redirect_err.as_ref(), "2>");
    append_function_redirect(text, cmd.redirect_err_append.as_ref(), "2>>");
    if let Some(here_string) = &cmd.here_string {
        text.push_str(" <<< ");
        text.push_str(here_string);
    }
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
