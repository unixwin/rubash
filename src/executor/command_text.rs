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
    // A command carrying words is executable — GNU execute_cmd.c dispatches
    // every word-bearing command through execute_simple_command (and a brace
    // group's body through execute_command), so `time { echo x; }` must run
    // the group body. This predicate only recognizes nodes with literally
    // nothing to execute.
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.redirects.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.here_string.is_none()
        && cmd.pipe.is_none()
        && cmd.and_or.is_none()
        && !cmd.background
        && !cmd.inverted
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.time_command.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && !cmd.subshell
        && !cmd.subshell_end
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
        && cmd.function_command.is_none()
        && cmd
            .brace_group
            .as_ref()
            .map_or(true, |bg| bg.body.iter().all(command_has_no_effect))
}

pub(in crate::executor) fn normalize_leading_assignment_words(cmd: &mut CommandNode) {
    let mut count = 0;
    while let Some(word) = cmd.words.get(count) {
        let Some((name, value)) = split_assignment_word(word) else {
            break;
        };
        cmd.insert_assignment(name.to_string(), value.to_string());
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
        || !cmd.redirects.is_empty()
}

pub(in crate::executor) fn function_definition_command_uses_source_text(
    command: &CommandNode,
) -> bool {
    command.words.is_empty()
        || command.pipeline_command.is_some()
        || command.and_or_list.is_some()
        || command.time_command.is_some()
        || command.background_command.is_some()
        || command.inverted_command.is_some()
        || command.arithmetic_command.is_some()
        || command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.conditional_command.is_some()
        || command.subshell_command.is_some()
        || command.case_command.is_some()
        || command.select_command.is_some()
        || command.brace_group.is_some()
        || command.coproc_command.is_some()
        || command.function_command.is_some()
}

pub(in crate::executor) fn append_function_redirect(
    line: &mut String,
    redirect: Option<&crate::parser::Redirect>,
    op: &str,
) {
    let Some(redirect) = redirect else {
        return;
    };

    // A dynamic varredir already contributes its fd word (for example
    // {fd}<&0); Bash keeps the operator adjacent to that word.
    if redirect.fd_var.is_none() {
        line.push(' ');
    }
    let target = redirect.target.as_str();
    if target.starts_with('&') {
        // Bash keeps fd duplication operators adjacent to their raw token.
        if redirect.fd.is_none()
            && redirect.fd_var.is_none()
            && matches!(op, "<" | ">")
            && target[1..].chars().all(|ch| ch.is_ascii_digit())
        {
            line.push_str(if op == "<" { "0" } else { "1" });
        }
        line.push_str(op);
        line.push_str(target);
    } else {
        line.push_str(op);
        line.push(' ');
        line.push_str(target);
    }
}

pub(in crate::executor) fn fd_stdin_key(fd: u32) -> String {
    format!("{FD_STDIN_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_stdin_offset_key(fd: u32) -> String {
    format!("{FD_STDIN_OFFSET_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_dynamic_input_key(fd: u32) -> String {
    format!("{FD_DYNAMIC_INPUT_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_output_key(fd: u32) -> String {
    format!("{FD_OUTPUT_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_output_process_substitution_key(fd: u32) -> String {
    format!("{FD_OUTPUT_PROCESS_SUBSTITUTION_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_closed_key(fd: u32) -> String {
    format!("{FD_CLOSED_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_terminal_key(fd: u32) -> String {
    format!("{FD_TERMINAL_PREFIX}{fd}")
}

pub(in crate::executor) fn command_has_output_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || cmd.redirects.iter().any(|redirect| redirect.is_output_side())
}

pub(in crate::executor) fn command_has_input_or_output_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.heredoc.is_some()
        || cmd.here_string.is_some()
        || cmd.redirects.iter().any(|redirect| redirect.is_input_side())
        || command_has_output_redirects(cmd)
}

pub(in crate::executor) fn command_references_bash_command(cmd: &CommandNode) -> bool {
    cmd.parameter_expansions.iter().any(|parameter| {
        parameter.name == "BASH_COMMAND"
            || parameter.text.contains("BASH_COMMAND")
            || parameter.text.contains("${!")
    }) || cmd.words.iter().any(|word| word.contains("BASH_COMMAND"))
        || cmd
            .word_metadata
            .iter()
            .any(|metadata| metadata.raw.contains("BASH_COMMAND") || metadata.raw.contains("${!"))
        || cmd
            .assignments
            .iter()
            .any(|(name, value)| name.contains("BASH_COMMAND") || value.contains("BASH_COMMAND"))
        || cmd
            .redirects
            .iter()
            .any(|redirect| redirect.target.contains("BASH_COMMAND"))
        || cmd
            .here_string
            .as_ref()
            .is_some_and(|value| value.contains("BASH_COMMAND"))
}

/// GNU `line_number` ownership (execute_cmd.c): these command kinds stamp
/// `line_number` from their own `->line` field while they execute —
/// cm_simple (:936), cm_subshell (:696), cm_for (:3001), cm_select
/// (:3513), cm_case (:3653), cm_arith (:3904), cm_cond (:4138),
/// cm_arith_for (:3236). Everything else — while/until, if, `{ }`,
/// coproc, function definitions, and the pipeline/&&/||/&/!/time wrapper
/// nodes — runs under the ambient `line_number` left by the reader or
/// the enclosing command.
pub(in crate::executor) fn command_sets_own_line(cmd: &CommandNode) -> bool {
    cmd.loop_command.is_none()
        && cmd.if_command.is_none()
        && cmd.brace_group.is_none()
        && cmd.coproc_command.is_none()
        && cmd.function_command.is_none()
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.time_command.is_none()
        && !command_is_time_prefixed_compound(cmd)
}

pub(in crate::executor) fn command_needs_process_line_env(cmd: &CommandNode) -> bool {
    cmd.words
        .iter()
        .any(|word| command_word_needs_process_line_env(word))
}

fn command_word_needs_process_line_env(word: &str) -> bool {
    matches!(
        word,
        "alias"
            | "unalias"
            | "declare"
            | "typeset"
            | "export"
            | "readonly"
            | "hash"
            | "help"
            | "kill"
            | "let"
            | "local"
            | "shopt"
            | "setopt"
            | "shift"
            | "ulimit"
            | "unset"
            | "unsetopt"
    )
}

pub(in crate::executor) fn bash_command_text(cmd: &CommandNode) -> String {
    let mut parts = Vec::new();
    for (name, value) in &cmd.assignments {
        // Assignment values may carry the lexer's private quoted-RHS marker.
        // BASH_COMMAND exposes shell source, never that expansion sentinel.
        let value = value.strip_prefix(crate::executor::markers::IFS_GLUE).unwrap_or(value);
        parts.push(format!("{name}={value}"));
    }
    let words = command_words_source_text_for_command(cmd);
    if !words.is_empty() {
        parts.push(words);
    }

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
        parts.push(err_append_source_text(cmd, redirect));
    }
    if let Some(here_string) = &cmd.here_string {
        parts.push(format!("<<< {here_string}"));
    }

    parts.join(" ")
}

/// `2>&1` after `>file` is mirrored into `redirect_err_append` as a
/// synthesized `2>>file` (parser/redirect_assign.rs assign_redirect_err_target)
/// so legacy consumers can route fd 2 to the resolved file — but the ordered
/// `redirects` list keeps the real `2>&1` dup. Reprints must use the real
/// entry (GNU print_cmd.c prints `r_instruction` literally): a serialized
/// `2>>file` re-opened by a `-c` child loses the shared open file
/// description (redir7.sub's `eval \`...\`` spawned `( sleep > /dev/null
/// 2>> /dev/null & )` instead of `2>&1`).
fn err_append_source_text(cmd: &CommandNode, redirect: &Redirect) -> String {
    if !cmd.redirects.contains(redirect) {
        if let Some(dup) = cmd.redirects.iter().find(|entry| {
            matches!(entry.kind, crate::parser::RedirectKind::DuplicateOutput)
                && entry.fd.unwrap_or(1) == 2
        }) {
            return format_redirect(&dup.operator, dup);
        }
    }
    format_redirect("2>>", redirect)
}

fn command_words_source_text_for_command(cmd: &CommandNode) -> String {
    let mut rendered = cmd
        .words
        .iter()
        .enumerate()
        .map(|(index, word)| command_word_source_text(index, word, &cmd.word_metadata))
        .collect::<Vec<_>>();

    for assignment in &cmd.compound_assignments {
        if let Some(index) = assignment.word_index {
            if let Some(word) = rendered.get_mut(index) {
                *word = compound_assignment_source_text(assignment);
            }
        }
    }

    rendered.join(" ")
}

pub(in crate::executor) fn command_words_source_text(
    words: &[String],
    metadata: &[WordMetadata],
) -> String {
    words
        .iter()
        .enumerate()
        .map(|(index, word)| command_word_source_text(index, word, metadata))
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_word_source_text(index: usize, word: &str, metadata: &[WordMetadata]) -> String {
    metadata
        .get(index)
        .filter(|metadata| metadata.value == *word && !metadata.raw.is_empty())
        .map(|metadata| metadata.raw.clone())
        .unwrap_or_else(|| shell_single_quote_assignment_value(word))
}

fn compound_assignment_source_text(assignment: &crate::parser::CompoundAssignment) -> String {
    format!(
        "{}{}{}",
        assignment.name, assignment.operator, assignment.value
    )
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
    } else if let Some(pipeline_command) = &cmd.pipeline_command {
        pipeline_command_source_text(pipeline_command)
    } else if let Some(and_or_list) = &cmd.and_or_list {
        and_or_list_source_text(and_or_list)
    } else if let Some(time_command) = &cmd.time_command {
        time_command_source_text(time_command)
    } else if let Some(background_command) = &cmd.background_command {
        background_command_source_text(background_command)
    } else if let Some(inverted_command) = &cmd.inverted_command {
        inverted_command_source_text(inverted_command)
    } else if let Some(arithmetic_command) = &cmd.arithmetic_command {
        arithmetic_command_source_text(arithmetic_command)
    } else if let Some(if_command) = &cmd.if_command {
        if_command_source_text(if_command)
    } else if let Some(loop_command) = &cmd.loop_command {
        loop_command_source_text(loop_command)
    } else if let Some(conditional_command) = &cmd.conditional_command {
        conditional_command_source_text(conditional_command)
    } else if let Some(subshell_command) = &cmd.subshell_command {
        subshell_command_source_text(subshell_command)
    } else if let Some(select_command) = &cmd.select_command {
        select_command_source_text(select_command)
    } else if let Some(case_command) = &cmd.case_command {
        case_command_source_text(case_command)
    } else if let Some(coproc_command) = &cmd.coproc_command {
        coproc_command_source_text(coproc_command)
    } else if let Some(function_command) = &cmd.function_command {
        function_command_source_text(function_command)
    } else if let Some(brace_group) = &cmd.brace_group {
        format!("{{ {}; }}", bash_command_sequence_text(&brace_group.body))
    } else {
        // bash_command_text already appends the command's redirections;
        // routing it through append_source_redirects below would render
        // each one twice (`2> /dev/null 2> /dev/null`).
        return bash_command_text(cmd);
    };
    append_source_redirects(&mut text, cmd);
    text
}

fn for_command_source_text(for_command: &ForCommand) -> String {
    let body = command_body_source_text(
        for_command.body_kind,
        for_command.body_open_delimiter.as_deref(),
        for_command.body_close_delimiter.as_deref(),
        &for_command.body,
    );
    if let Some(arithmetic) = &for_command.arithmetic {
        return format!(
            "for (( {}; {}; {} )); {}",
            arithmetic.init, arithmetic.test, arithmetic.update, body
        );
    }

    if for_command.default_positional {
        // GNU print_cmd.c:602 print_for_command_head reprints the implicit
        // `for i; do` form's map_list as the literal `"$@"` word.
        format!("for {} in \"$@\"; {}", for_command.variable, body)
    } else {
        format!(
            "for {} in {}; {}",
            for_command.variable,
            for_command.words.join(" "),
            body
        )
    }
}

fn pipeline_command_source_text(pipeline_command: &PipelineCommand) -> String {
    pipeline_command
        .stages
        .iter()
        .map(bash_command_source_text)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn and_or_list_source_text(and_or_list: &AndOrListCommand) -> String {
    let mut text = String::new();
    for (index, command) in and_or_list.commands.iter().enumerate() {
        if index > 0 {
            let connector = and_or_list
                .connectors
                .get(index - 1)
                .copied()
                .unwrap_or(true);
            text.push_str(if connector { " && " } else { " || " });
        }
        text.push_str(&bash_command_source_text(command));
    }
    text
}

fn time_command_source_text(time_command: &TimeCommand) -> String {
    let mut parts = vec!["time".to_string()];
    if time_command.posix_format {
        parts.push("-p".to_string());
    }
    if time_command.inverted {
        parts.push("!".to_string());
    }
    parts.push(bash_command_source_text(&time_command.command));
    parts.join(" ")
}

fn background_command_source_text(background_command: &BackgroundCommand) -> String {
    format!(
        "{} &",
        bash_command_source_text(&background_command.command)
    )
}

fn inverted_command_source_text(inverted_command: &InvertedCommand) -> String {
    format!("! {}", bash_command_source_text(&inverted_command.command))
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
        "{} {}; {} {}; {}",
        if loop_command.until { "until" } else { "while" },
        bash_command_sequence_text(&loop_command.condition),
        loop_command.body_open_delimiter,
        bash_command_sequence_text(&loop_command.body),
        loop_command.body_close_delimiter
    )
}

fn conditional_command_source_text(conditional_command: &ConditionalCommand) -> String {
    format!("[[ {}", conditional_command.args.join(" "))
}

fn subshell_command_source_text(subshell_command: &SubshellCommand) -> String {
    format!("( {} )", bash_command_sequence_text(&subshell_command.body))
}

fn select_command_source_text(select_command: &SelectCommand) -> String {
    let body = command_body_source_text(
        select_command.body_kind,
        select_command.body_open_delimiter.as_deref(),
        select_command.body_close_delimiter.as_deref(),
        &select_command.body,
    );
    if select_command.default_positional {
        // GNU print_cmd.c:656 print_select_command_head reprints the
        // implicit `select x; do` form's map_list as the literal `"$@"`.
        format!("select {} in \"$@\"; {}", select_command.variable, body)
    } else {
        format!(
            "select {} in {}; {}",
            select_command.variable,
            select_command.words.join(" "),
            body
        )
    }
}

fn command_body_source_text(
    body_kind: CommandBodyKind,
    open_delimiter: Option<&str>,
    close_delimiter: Option<&str>,
    body: &[CommandNode],
) -> String {
    let body = bash_command_sequence_text(body);
    match body_kind {
        CommandBodyKind::DoDone => format!(
            "{} {}; {}",
            open_delimiter.unwrap_or("do"),
            body,
            close_delimiter.unwrap_or("done")
        ),
        CommandBodyKind::BraceGroup => format!(
            "{} {}; {}",
            open_delimiter.unwrap_or("{"),
            body,
            close_delimiter.unwrap_or("}")
        ),
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
        text.push_str(&command_words_source_text(
            &coproc_command.words,
            &coproc_command.word_metadata,
        ));
    }
    text
}

fn function_command_source_text(function_command: &crate::parser::FunctionCommand) -> String {
    let mut text = if function_command.keyword {
        format!("function {}", function_command.name)
    } else {
        function_command.name.clone()
    };
    if function_command.has_parentheses {
        text.push_str("()");
    }

    let body = bash_command_sequence_text(&function_command.body);
    match function_command.body_kind {
        FunctionBodyKind::BraceGroup => format!(
            "{} {} {}; {}",
            text,
            function_command
                .body_open_delimiter
                .as_deref()
                .unwrap_or("{"),
            body,
            function_command
                .body_close_delimiter
                .as_deref()
                .unwrap_or("}")
        ),
        FunctionBodyKind::Subshell => format!(
            "{} {} {} {}",
            text,
            function_command
                .body_open_delimiter
                .as_deref()
                .unwrap_or("("),
            body,
            function_command
                .body_close_delimiter
                .as_deref()
                .unwrap_or(")")
        ),
        FunctionBodyKind::CommandSequence | FunctionBodyKind::CompoundCommand => {
            format!("{text} {body}")
        }
    }
}

pub(in crate::executor) fn append_source_redirects(text: &mut String, cmd: &CommandNode) {
    append_function_redirect(text, cmd.redirect_in.as_ref(), "<");
    let combined = cmd
        .redirect_out
        .as_ref()
        .filter(|redirect| {
            matches!(
                redirect.kind,
                crate::parser::RedirectKind::CombinedOutput
                    | crate::parser::RedirectKind::CombinedAppend
            )
        })
        .or_else(|| {
            cmd.redirect_err_append.as_ref().filter(|redirect| {
                matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::CombinedOutput
                        | crate::parser::RedirectKind::CombinedAppend
                )
            })
        });
    if let Some(redirect) = combined {
        let op = if redirect.kind == crate::parser::RedirectKind::CombinedAppend {
            "&>>"
        } else {
            "&>"
        };
        append_function_redirect(text, Some(redirect), op);
    } else {
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
        if let Some(redirect) = &cmd.redirect_err_append {
            text.push(' ');
            text.push_str(&err_append_source_text(cmd, redirect));
        }
    }
    // Numbered redirects (fd>=3) and `{var}` dynamic fds live only in the
    // ordered `redirects` list — the mirror fields above cannot represent
    // them. GNU print_cmd prints every redirection, so reprint them too or
    // a `-c` child loses `3>f` entirely.
    for redirect in &cmd.redirects {
        if redirect.is_list_only_redirect() {
            text.push(' ');
            text.push_str(&format_redirect(&redirect.operator, redirect));
        }
    }
    if let Some(here_string) = &cmd.here_string {
        text.push_str(" <<< ");
        text.push_str(here_string);
    }
}
