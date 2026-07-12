use super::*;

pub(in crate::executor) fn is_shell_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    is_shell_name_start(first) && chars.all(is_shell_name_char)
}

pub(in crate::executor) fn is_shell_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

pub(in crate::executor) fn is_shell_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

pub(in crate::executor) fn is_special_parameter_name(name: &str) -> bool {
    matches!(name, "#" | "?" | "$" | "!" | "-" | "0")
}

pub(in crate::executor) fn bash_version_value() -> String {
    format!("{}(1)-release", env!("CARGO_PKG_VERSION"))
}

pub(in crate::executor) fn bash_path_value() -> String {
    std::env::current_exe()
        .map(|path| shell_display_path(&path.to_string_lossy().replace('\\', "/")))
        .unwrap_or_else(|_| "rubash".to_string())
}

pub(in crate::executor) fn bash_versinfo_values() -> Vec<String> {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    vec![
        parts.next().unwrap_or("0").to_string(),
        parts.next().unwrap_or("0").to_string(),
        parts.next().unwrap_or("0").to_string(),
        "1".to_string(),
        "release".to_string(),
        machtype_value(),
    ]
}

pub(in crate::executor) fn hosttype_value() -> String {
    std::env::consts::ARCH.to_string()
}

pub(in crate::executor) fn hostname_value() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "localhost".to_string())
}

pub(in crate::executor) fn ostype_value() -> String {
    if cfg!(windows) {
        "msys".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

pub(in crate::executor) fn machtype_value() -> String {
    if cfg!(windows) {
        format!("{}-pc-msys", std::env::consts::ARCH)
    } else if cfg!(target_env = "gnu") {
        format!("{}-pc-{}-gnu", std::env::consts::ARCH, std::env::consts::OS)
    } else {
        format!(
            "{}-pc-{}-{}",
            std::env::consts::ARCH,
            std::env::consts::OS,
            std::env::consts::FAMILY
        )
    }
}

pub(in crate::executor) fn uid_value() -> String {
    std::env::var("UID")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "1000".to_string())
}

pub(in crate::executor) fn euid_value() -> String {
    std::env::var("EUID").unwrap_or_else(|_| uid_value())
}

pub(in crate::executor) fn ppid_value() -> String {
    std::env::var("PPID")
        .ok()
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or_else(|| std::process::id().to_string())
}

pub(in crate::executor) fn declare_args_request_integer(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg.starts_with('-')
            && arg != "-"
            && !arg.starts_with("--")
            && arg[1..].chars().any(|option| option == 'i')
    })
}

pub(in crate::executor) fn is_reserved_word(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "while"
            | "do"
            | "done"
            | "until"
            | "for"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "select"
            | "time"
            | "coproc"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::executor) enum LoopControlError {
    TooManyArguments,
    OutOfRange(String),
    NotNumeric(String),
}

pub(in crate::executor) fn loop_control_level(args: &[String]) -> Result<usize, LoopControlError> {
    let mut args = args.iter().map(String::as_str);
    let first = match args.next() {
        Some("--") => args.next(),
        other => other,
    };

    let Some(value) = first else {
        return Ok(1);
    };
    if args.next().is_some() {
        return Err(LoopControlError::TooManyArguments);
    }
    if value.starts_with('-') {
        return Err(LoopControlError::OutOfRange(value.to_string()));
    }

    let number = value.strip_prefix('+').unwrap_or(value);
    match number.parse::<usize>() {
        Ok(level) if level > 0 => Ok(level),
        Ok(_) => Err(LoopControlError::OutOfRange(value.to_string())),
        Err(_) => Err(LoopControlError::NotNumeric(value.to_string())),
    }
}

pub(in crate::executor) fn invert_exit_status(status: i32) -> i32 {
    i32::from(status == 0)
}

pub(in crate::executor) fn short_set_flag_option(flag: char) -> Option<&'static str> {
    match flag {
        'a' => Some("allexport"),
        'b' => Some("notify"),
        'B' => Some("braceexpand"),
        'E' => Some("errtrace"),
        'h' => Some("hashall"),
        'H' => Some("histexpand"),
        'k' => Some("keyword"),
        'P' => Some("physical"),
        'p' => Some("privileged"),
        't' => Some("onecmd"),
        'T' => Some("functrace"),
        'v' => Some("verbose"),
        _ => None,
    }
}

pub(in crate::executor) fn apply_stdout_append_redirect(
    commands: &mut [CommandNode],
    redirect: &Redirect,
) {
    for command in commands {
        if command.redirect_out.is_none() && command.append.is_none() {
            command.append = Some(redirect.clone());
        }
        if let Some(for_command) = &mut command.for_command {
            apply_stdout_append_redirect(&mut for_command.body, redirect);
        }
        if let Some(pipeline_command) = &mut command.pipeline_command {
            apply_stdout_append_redirect(&mut pipeline_command.stages, redirect);
        }
        if let Some(and_or_list) = &mut command.and_or_list {
            apply_stdout_append_redirect(&mut and_or_list.commands, redirect);
        }
        if let Some(if_command) = &mut command.if_command {
            apply_stdout_append_redirect(&mut if_command.condition, redirect);
            apply_stdout_append_redirect(&mut if_command.then_body, redirect);
            for branch in &mut if_command.elif_branches {
                apply_stdout_append_redirect(&mut branch.condition, redirect);
                apply_stdout_append_redirect(&mut branch.body, redirect);
            }
            if let Some(body) = &mut if_command.else_body {
                apply_stdout_append_redirect(body, redirect);
            }
        }
        if let Some(loop_command) = &mut command.loop_command {
            apply_stdout_append_redirect(&mut loop_command.condition, redirect);
            apply_stdout_append_redirect(&mut loop_command.body, redirect);
        }
        if let Some(case_command) = &mut command.case_command {
            for clause in &mut case_command.clauses {
                apply_stdout_append_redirect(&mut clause.body, redirect);
            }
        }
        if let Some(brace_group) = &mut command.brace_group {
            apply_stdout_append_redirect(&mut brace_group.body, redirect);
        }
    }
}

pub(in crate::executor) fn apply_stderr_append_redirect(
    commands: &mut [CommandNode],
    redirect: &Redirect,
) {
    for command in commands {
        let inherits_stderr =
            command.redirect_err.is_none() && command.redirect_err_append.is_none();
        if inherits_stderr {
            command.redirect_err_append = Some(redirect.clone());
            apply_inherited_stderr_to_stdout_fd_copy(command, redirect);
        }
        if let Some(for_command) = &mut command.for_command {
            apply_stderr_append_redirect(&mut for_command.body, redirect);
        }
        if let Some(pipeline_command) = &mut command.pipeline_command {
            apply_stderr_append_redirect(&mut pipeline_command.stages, redirect);
        }
        if let Some(and_or_list) = &mut command.and_or_list {
            apply_stderr_append_redirect(&mut and_or_list.commands, redirect);
        }
        if let Some(if_command) = &mut command.if_command {
            apply_stderr_append_redirect(&mut if_command.condition, redirect);
            apply_stderr_append_redirect(&mut if_command.then_body, redirect);
            for branch in &mut if_command.elif_branches {
                apply_stderr_append_redirect(&mut branch.condition, redirect);
                apply_stderr_append_redirect(&mut branch.body, redirect);
            }
            if let Some(body) = &mut if_command.else_body {
                apply_stderr_append_redirect(body, redirect);
            }
        }
        if let Some(loop_command) = &mut command.loop_command {
            apply_stderr_append_redirect(&mut loop_command.condition, redirect);
            apply_stderr_append_redirect(&mut loop_command.body, redirect);
        }
        if let Some(case_command) = &mut command.case_command {
            for clause in &mut case_command.clauses {
                apply_stderr_append_redirect(&mut clause.body, redirect);
            }
        }
        if let Some(brace_group) = &mut command.brace_group {
            apply_stderr_append_redirect(&mut brace_group.body, redirect);
        }
    }
}

pub(in crate::executor) fn split_shell_path(path: &str) -> Vec<String> {
    if path.contains(';') {
        path.split(';')
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        path.split(':')
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()
    }
}

pub(in crate::executor) fn executable_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .map(|value| {
            value
                .split(';')
                .filter_map(|ext| ext.trim().trim_start_matches('.').split_whitespace().next())
                .filter(|ext| !ext.is_empty())
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_else(|| vec!["exe".into(), "com".into(), "bat".into(), "cmd".into()])
}

pub(in crate::executor) fn normalize_type_option(option: &str) -> &str {
    match option {
        "-type" | "--type" => "-t",
        "-path" | "--path" => "-p",
        "-all" | "--all" => "-a",
        other => other,
    }
}

pub(in crate::executor) fn parse_command_describe_args(
    args: &[String],
) -> Option<(TypeDescribeMode, bool, usize)> {
    let mut mode = None;
    let mut use_standard_path = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'p' => use_standard_path = true,
                'v' => mode = Some(TypeDescribeMode::Reusable),
                'V' => mode = Some(TypeDescribeMode::Verbose),
                _ => return None,
            }
        }
        index += 1;
    }

    mode.map(|mode| (mode, use_standard_path, index))
}
