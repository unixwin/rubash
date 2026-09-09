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

/// Bash compatibility version advertised through BASH_VERSION / BASH_VERSINFO.
/// Rubash targets GNU Bash 5.3 semantics (owner directive 2026-09-09, the
/// compiled /usr/local/bin/bash 5.3.0 baseline); scripts that feature-detect on
/// the version must see a compatible value, like other Bash-compatible shells.
const BASH_COMPAT_VERSION: &str = "5.3.0";

pub(in crate::executor) fn bash_version_value() -> String {
    format!("{}(1)-release", BASH_COMPAT_VERSION)
}

pub(in crate::executor) fn bash_path_value() -> String {
    std::env::current_exe()
        .map(|path| shell_display_path(&path.to_string_lossy().replace('\\', "/")))
        .unwrap_or_else(|_| "rubash".to_string())
}

pub(in crate::executor) fn shell_path_value() -> String {
    bash_path_value()
}

pub(in crate::executor) fn bash_versinfo_values() -> Vec<String> {
    let mut parts = BASH_COMPAT_VERSION.split('.');
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
        // Rubash emulates a root Unix session: the WSL GNU Bash 5.2.21
        // comparison baseline runs as root (UID/EUID 0), and `\$` prompt
        // decoding plus $UID/$EUID parity depend on the same identity.
        .unwrap_or_else(|| "0".to_string())
}

pub(in crate::executor) fn euid_value() -> String {
    std::env::var("EUID").unwrap_or_else(|_| uid_value())
}

pub(in crate::executor) fn ppid_value() -> String {
    std::env::var("PPID")
        .ok()
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))
        .or_else(|| parent_process_id().map(|pid| pid.to_string()))
        .unwrap_or_else(|| std::process::id().to_string())
}

/// Real parent process id. Windows does not export a PPID environment
/// variable, so walk the process table; Unix uses getppid(2).
fn parent_process_id() -> Option<u32> {
    #[cfg(windows)]
    {
        windows_parent_process_id()
    }

    #[cfg(unix)]
    {
        u32::try_from(unsafe { libc::getppid() }).ok()
    }

    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_parent_process_id() -> Option<u32> {
    use std::ffi::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut PROCESSENTRY32W) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    let current = std::process::id();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut parent = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32_process_id == current {
                    parent = Some(entry.th32_parent_process_id);
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        parent
    }
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

pub(in crate::executor) fn is_posix_special_builtin(name: &str) -> bool {
    matches!(
        name,
        "." | ":"
            | "break"
            | "continue"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "times"
            | "trap"
            | "unset"
    )
}

/// GNU general.c::valid_identifier: `[A-Za-z_][A-Za-z0-9_]*`. The POSIX
/// function-name restriction it supports is compiled out of the 5.3 baseline
/// (config-top.h leaves POSIX_RESTRICT_FUNCNAME undefined, so
/// execute_intern_function never sets pflags&1 and `!! () { ...; }` is a
/// legal POSIX-mode definition); the helper stays for the documented ifdef.
#[allow(dead_code)]
pub(in crate::executor) fn valid_function_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
        'r' => Some("restricted"),
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
            if command.redirects.iter().any(|existing| {
                matches!(
                    existing.kind,
                    crate::parser::RedirectKind::Output
                        | crate::parser::RedirectKind::Append
                        | crate::parser::RedirectKind::ClobberOutput
                )
            }) {
                command.redirects.push(redirect.clone());
            } else {
                command.redirects.insert(0, redirect.clone());
            }
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
        if let Some(time_command) = &mut command.time_command {
            apply_stdout_append_redirect(
                std::slice::from_mut(time_command.command.as_mut()),
                redirect,
            );
        }
        if let Some(background_command) = &mut command.background_command {
            apply_stdout_append_redirect(
                std::slice::from_mut(background_command.command.as_mut()),
                redirect,
            );
        }
        if let Some(inverted_command) = &mut command.inverted_command {
            apply_stdout_append_redirect(
                std::slice::from_mut(inverted_command.command.as_mut()),
                redirect,
            );
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
    let redirect = if matches!(redirect.kind, crate::parser::RedirectKind::DuplicateOutput)
        && redirect_target_fd(&redirect.target).is_none()
    {
        let mut normalized = redirect.clone();
        normalized.operator = "2>>".to_string();
        normalized.kind = crate::parser::RedirectKind::Append;
        normalized.append = true;
        normalized.clobber = false;
        normalized
    } else {
        redirect.clone()
    };

    for command in commands {
        let inherits_stderr =
            command.redirect_err.is_none() && command.redirect_err_append.is_none();
        if inherits_stderr {
            command.redirect_err_append = Some(redirect.clone());
            let has_stdout_redirect = command.redirects.iter().any(|existing| {
                matches!(
                    existing.kind,
                    crate::parser::RedirectKind::Output
                        | crate::parser::RedirectKind::Append
                        | crate::parser::RedirectKind::ClobberOutput
                ) && existing.fd.unwrap_or(1) == 1
            });
            if has_stdout_redirect {
                command.redirects.push(redirect.clone());
            } else {
                command.redirects.insert(0, redirect.clone());
            }
            apply_inherited_stderr_to_stdout_fd_copy(command, &redirect);
        }
        if let Some(for_command) = &mut command.for_command {
            apply_stderr_append_redirect(&mut for_command.body, &redirect);
        }
        if let Some(pipeline_command) = &mut command.pipeline_command {
            apply_stderr_append_redirect(&mut pipeline_command.stages, &redirect);
        }
        if let Some(and_or_list) = &mut command.and_or_list {
            apply_stderr_append_redirect(&mut and_or_list.commands, &redirect);
        }
        if let Some(time_command) = &mut command.time_command {
            apply_stderr_append_redirect(
                std::slice::from_mut(time_command.command.as_mut()),
                &redirect,
            );
        }
        if let Some(background_command) = &mut command.background_command {
            apply_stderr_append_redirect(
                std::slice::from_mut(background_command.command.as_mut()),
                &redirect,
            );
        }
        if let Some(inverted_command) = &mut command.inverted_command {
            apply_stderr_append_redirect(
                std::slice::from_mut(inverted_command.command.as_mut()),
                &redirect,
            );
        }
        if let Some(if_command) = &mut command.if_command {
            apply_stderr_append_redirect(&mut if_command.condition, &redirect);
            apply_stderr_append_redirect(&mut if_command.then_body, &redirect);
            for branch in &mut if_command.elif_branches {
                apply_stderr_append_redirect(&mut branch.condition, &redirect);
                apply_stderr_append_redirect(&mut branch.body, &redirect);
            }
            if let Some(body) = &mut if_command.else_body {
                apply_stderr_append_redirect(body, &redirect);
            }
        }
        if let Some(loop_command) = &mut command.loop_command {
            apply_stderr_append_redirect(&mut loop_command.condition, &redirect);
            apply_stderr_append_redirect(&mut loop_command.body, &redirect);
        }
        if let Some(case_command) = &mut command.case_command {
            for clause in &mut case_command.clauses {
                apply_stderr_append_redirect(&mut clause.body, &redirect);
            }
        }
        if let Some(brace_group) = &mut command.brace_group {
            apply_stderr_append_redirect(&mut brace_group.body, &redirect);
        }
    }
}

pub(in crate::executor) fn split_shell_path(path: &str) -> Vec<String> {
    if cfg!(windows) {
        split_windows_shell_path(path)
    } else {
        path.split(':')
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()
    }
}

fn split_windows_shell_path(path: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let bytes = path.as_bytes();
    let mut entry_start = 0;

    for (index, byte) in bytes.iter().enumerate() {
        let is_separator = match byte {
            b';' => true,
            b':' => !is_windows_drive_colon(bytes, entry_start, index),
            _ => false,
        };

        if is_separator {
            push_path_entry(&mut entries, path, entry_start, index);
            entry_start = index + 1;
        }
    }

    push_path_entry(&mut entries, path, entry_start, path.len());
    entries
}

fn push_path_entry(entries: &mut Vec<String>, path: &str, start: usize, end: usize) {
    if start < end {
        entries.push(path[start..end].to_string());
    }
}

fn is_windows_drive_colon(bytes: &[u8], entry_start: usize, colon: usize) -> bool {
    colon == entry_start + 1
        && bytes[entry_start].is_ascii_alphabetic()
        && bytes
            .get(colon + 1)
            .is_some_and(|next| matches!(next, b'\\' | b'/'))
}

#[cfg(test)]
mod split_shell_path_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_splits_bash_style_prefix_before_native_path() {
        assert_eq!(
            split_shell_path("/c/tools/cloc:/c/tools/winuxcmd;C:/Windows"),
            vec![
                "/c/tools/cloc".to_string(),
                "/c/tools/winuxcmd".to_string(),
                "C:/Windows".to_string(),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_preserves_drive_colons_while_accepting_colon_separators() {
        assert_eq!(
            split_shell_path("C:/Windows;D:\\Tools:/c/bin"),
            vec![
                "C:/Windows".to_string(),
                "D:\\Tools".to_string(),
                "/c/bin".to_string(),
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_splits_only_on_colons() {
        assert_eq!(
            split_shell_path("/usr/bin:/bin;/literal-semicolon"),
            vec![
                "/usr/bin".to_string(),
                "/bin;/literal-semicolon".to_string()
            ]
        );
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
