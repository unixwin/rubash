use super::*;

pub(in crate::executor) fn import_exported_functions_from_env(
    env_vars: &HashMap<String, String>,
) -> HashMap<String, Vec<CommandNode>> {
    let mut functions = HashMap::new();
    for (env_name, value) in env_vars {
        let Some(name) = imported_function_name(env_name) else {
            continue;
        };
        let Some(body) = parse_exported_function_body(value) else {
            continue;
        };
        functions.insert(name.to_string(), body);
    }
    functions
}

pub(in crate::executor) fn imported_function_name(env_name: &str) -> Option<&str> {
    let name = env_name.strip_prefix("BASH_FUNC_")?.strip_suffix("%%")?;
    if is_imported_function_name(name) {
        Some(name)
    } else {
        None
    }
}

pub(in crate::executor) fn is_imported_function_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('=')
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

pub(in crate::executor) fn is_exportable_function_name(name: &str) -> bool {
    is_imported_function_name(name) && !name.contains('/') && !name.contains('\\')
}

pub(in crate::executor) fn parse_exported_function_body(value: &str) -> Option<Vec<CommandNode>> {
    let value = value.trim();
    let rest = value.strip_prefix("()")?.trim_start();
    if !rest.starts_with('{') || !rest.ends_with('}') {
        return None;
    }
    let body = rest[1..rest.len() - 1].trim();
    let tokens = crate::lexer::tokenize(body);
    Some(crate::parser::parse(&tokens).commands)
}

pub(in crate::executor) fn exported_function_env_name(name: &str) -> String {
    format!("BASH_FUNC_{name}%%")
}

pub(in crate::executor) fn exported_function_env_value(body: &[CommandNode]) -> String {
    let commands: Vec<String> = body
        .iter()
        .filter_map(exported_function_command_text)
        .collect();
    if commands.is_empty() {
        "() { :; }".to_string()
    } else {
        let mut output = String::from("() {");
        for command in commands {
            output.push('\n');
            output.push_str(&command);
        }
        output.push_str("\n}");
        output
    }
}

pub(in crate::executor) fn exported_function_command_text(command: &CommandNode) -> Option<String> {
    if command.words.is_empty() && command.assignments.is_empty() {
        return None;
    }
    if command.words.is_empty() {
        return Some(function_assignment_text(command));
    }

    let mut line = command.words.join(" ");
    if let Some(delimiter) = &command.heredoc_delimiter {
        line.push_str(" <<");
        line.push_str(delimiter);
    }
    append_exported_redirect(&mut line, command.redirect_in.as_ref(), "<");
    append_exported_redirect(
        &mut line,
        command.redirect_out.as_ref(),
        command
            .redirect_out
            .as_ref()
            .filter(|redirect| redirect.clobber)
            .map(|_| ">|")
            .unwrap_or(">"),
    );
    append_exported_redirect(&mut line, command.append.as_ref(), ">>");
    append_exported_redirect(
        &mut line,
        command.redirect_err.as_ref(),
        command
            .redirect_err
            .as_ref()
            .filter(|redirect| redirect.clobber)
            .map(|_| "2>|")
            .unwrap_or("2>"),
    );
    append_exported_redirect(&mut line, command.redirect_err_append.as_ref(), "2>>");
    if let (Some(body), Some(delimiter)) = (&command.heredoc, &command.heredoc_delimiter) {
        let body = body.strip_prefix('\x1e').unwrap_or(body);
        line.push('\n');
        line.push_str(body);
        line.push_str(delimiter);
        return Some(line);
    }

    if let Some(here_string) = &command.here_string {
        Some(format!("{} <<< {}", command.words.join(" "), here_string))
    } else {
        Some(line)
    }
}

pub(in crate::executor) fn function_assignment_text(command: &CommandNode) -> String {
    let mut assignments = command.assignments.iter().collect::<Vec<_>>();
    assignments.sort_by(|(left, _), (right, _)| left.cmp(right));
    assignments
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(in crate::executor) fn append_exported_redirect(
    line: &mut String,
    redirect: Option<&Redirect>,
    op: &str,
) {
    if let Some(redirect) = redirect {
        line.push(' ');
        line.push_str(op);
        line.push(' ');
        line.push_str(&redirect.target);
    }
}

pub(in crate::executor) fn export_args_request_functions(args: &[String]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if !arg.starts_with('-') || arg == "-" {
            return false;
        }
        if arg[1..].contains('f') {
            return true;
        }
    }
    false
}

pub(in crate::executor) fn readonly_args_request_functions(args: &[String]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if !arg.starts_with('-') || arg == "-" {
            return false;
        }
        if arg[1..].contains('f') {
            return true;
        }
    }
    false
}
