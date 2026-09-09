use super::*;

pub(in crate::executor) fn import_exported_functions_from_env(
    env_vars: &HashMap<String, String>,
) -> (
    HashMap<String, FunctionBody>,
    HashMap<String, crate::executor::FunctionDefInfo>,
) {
    let mut functions = HashMap::new();
    let mut def_infos = HashMap::new();
    for (env_name, value) in env_vars {
        let Some(name) = imported_function_name(env_name) else {
            continue;
        };
        let Some((body, def_redirects)) = parse_exported_function_body(value) else {
            continue;
        };
        functions.insert(name.to_string(), Rc::new(Ast { commands: body }));
        // The exportstr carries the function-definition redirections after
        // the closing brace (`() { ... } 1>&2`), the way GNU's
        // named_function_string(FUNC_EXTERNAL) renders them, so a re-imported
        // function still prints them.
        def_infos.insert(
            name.to_string(),
            crate::executor::FunctionDefInfo {
                body_kind: Some(crate::parser::FunctionBodyKind::BraceGroup),
                def_redirects,
            },
        );
    }
    (functions, def_infos)
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

pub(in crate::executor) fn parse_exported_function_body(
    value: &str,
) -> Option<(Vec<CommandNode>, Vec<crate::parser::Redirect>)> {
    let value = value.trim();
    let rest = value.strip_prefix("()")?.trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    // Split the closing brace of the definition from any trailing
    // function-definition redirections (`} 1>&2`). GNU exportstr values are
    // the printed definition, so the redirections come after the brace.
    let (body_and_brace, def_redirects) = match rest.rfind('}') {
        Some(close) if close + 1 == rest.len() => (rest, Vec::new()),
        Some(close) => {
            let trailing = rest[close + 1..].trim();
            let tokens = crate::lexer::tokenize(&format!(": {trailing}"));
            let parsed = crate::parser::parse(&tokens);
            let redirects = parsed
                .commands
                .first()
                .map(crate::parser::ast_print::collected_redirects)
                .unwrap_or_default();
            (&rest[..close + 1], redirects)
        }
        None => return None,
    };
    if !body_and_brace.ends_with('}') {
        return None;
    }
    let body = body_and_brace[1..body_and_brace.len() - 1].trim();
    let tokens = crate::lexer::tokenize(body);
    Some((crate::parser::parse(&tokens).commands, def_redirects))
}

pub(in crate::executor) fn exported_function_env_name(name: &str) -> String {
    format!("BASH_FUNC_{name}%%")
}

pub(in crate::executor) fn exported_function_env_value(
    body: &[CommandNode],
    def_redirects: &[crate::parser::Redirect],
) -> String {
    let commands: Vec<String> = body
        .iter()
        .filter_map(exported_function_command_text)
        .collect();
    if commands.is_empty() && def_redirects.is_empty() {
        "() { :; }".to_string()
    } else if commands.is_empty() {
        format!(
            "() {{ :; }}{}",
            redirect_suffix_text(def_redirects)
        )
    } else {
        let mut output = String::from("() {");
        for (index, command) in commands.iter().enumerate() {
            output.push('\n');
            output.push_str(command);
            let command_has_heredoc = body
                .get(index)
                .is_some_and(|command| command.heredoc.is_some());
            if index + 1 < commands.len() && !command_has_heredoc && !command.ends_with(';') {
                output.push(';');
            }
        }
        output.push_str("\n}");
        output.push_str(&redirect_suffix_text(def_redirects));
        output
    }
}

/// Function-definition redirections rendered after the closing brace of an
/// exportstr value (print_cmd.c prints them via print_redirection_list).
fn redirect_suffix_text(def_redirects: &[crate::parser::Redirect]) -> String {
    if def_redirects.is_empty() {
        return String::new();
    }
    format!(
        " {}",
        crate::parser::ast_print::redirect_list_text(def_redirects)
    )
}

pub(in crate::executor) fn exported_function_command_text(command: &CommandNode) -> Option<String> {
    if function_definition_command_uses_source_text(command) {
        let line = bash_command_source_text(command);
        if !line.trim().is_empty() {
            return Some(line);
        }
    }

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
    let combined = command
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
            command.redirect_err_append.as_ref().filter(|redirect| {
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
        append_exported_redirect(&mut line, Some(redirect), op);
    } else {
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
    }
    if let (Some(body), Some(delimiter)) = (&command.heredoc, &command.heredoc_delimiter) {
        let body = body
            .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
            .unwrap_or(body);
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
        if redirect.target.starts_with('&') {
            if redirect.fd.is_none()
                && matches!(op, "<" | ">")
                && redirect.target[1..].chars().all(|ch| ch.is_ascii_digit())
            {
                line.push_str(if op == "<" { "0" } else { "1" });
            }
            line.push_str(op);
            line.push_str(&redirect.target);
        } else {
            line.push_str(op);
            line.push(' ');
            line.push_str(&redirect.target);
        }
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
