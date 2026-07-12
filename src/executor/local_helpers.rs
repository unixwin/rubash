use super::*;

pub(in crate::executor) fn validate_local_options(args: &[String]) -> Result<(), char> {
    for arg in args {
        if arg == "--" {
            return Ok(());
        }
        if (!arg.starts_with('-') && !arg.starts_with('+')) || arg == "-" || arg == "+" {
            return Ok(());
        }
        for option in arg[1..].chars() {
            match option {
                'a' | 'A' | 'f' | 'F' | 'g' | 'I' | 'i' | 'l' | 'n' | 'p' | 'r' | 't' | 'u'
                | 'x' => {}
                other => return Err(other),
            }
        }
    }
    Ok(())
}

pub(in crate::executor) fn local_args_request_inherit(args: &[String]) -> bool {
    declare_args_contain_option(args, 'I', true) || declare_args_contain_option(args, 'I', false)
}

pub(in crate::executor) fn declare_args_force_global(args: &[String]) -> bool {
    declare_args_contain_option(args, 'g', true)
}

pub(in crate::executor) fn declare_args_request_print(args: &[String]) -> bool {
    declare_args_contain_option(args, 'p', true)
}

pub(in crate::executor) fn declare_args_contain_option(
    args: &[String],
    option: char,
    set_attr: bool,
) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if (!arg.starts_with('-') && !arg.starts_with('+')) || arg == "-" || arg == "+" {
            return false;
        }
        if arg.starts_with('-') != set_attr {
            continue;
        }
        if arg[1..].chars().any(|current| current == option) {
            return true;
        }
    }
    false
}

pub(in crate::executor) fn local_assignment_name(arg: &str) -> Option<&str> {
    let name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
    let name = name.strip_suffix('+').unwrap_or(name);
    let name = name.split_once('[').map(|(name, _)| name).unwrap_or(name);
    let name = name
        .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
        .unwrap_or(name);
    if is_shell_name(name) {
        Some(name)
    } else {
        None
    }
}

pub(in crate::executor) fn local_names_without_assignment(args: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for arg in args {
        if arg == "--" {
            continue;
        }
        if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
            continue;
        }
        if arg.contains('=') {
            continue;
        }
        if let Some(name) = local_assignment_name(arg) {
            names.push(name.to_string());
        }
    }
    names
}

pub(in crate::executor) fn local_stderr_from_declare(stderr: Vec<u8>) -> Vec<u8> {
    String::from_utf8(stderr)
        .map(|text| text.replace("declare:", "local:").into_bytes())
        .unwrap_or_default()
}

pub(in crate::executor) fn restore_optional_env_var(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    value: Option<String>,
) {
    match value {
        Some(value) => {
            env_vars.insert(name.to_string(), value);
        }
        None => {
            env_vars.remove(name);
        }
    }
}

pub(in crate::executor) fn restore_optional_shell_var(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    value: Option<String>,
) {
    match value {
        Some(value) => {
            env_vars.insert(name.to_string(), value.clone());
            if is_valid_process_env(name, &value) {
                set_process_env(name, value);
            }
        }
        None => {
            env_vars.remove(name);
            env::remove_var(name);
        }
    }
}

pub(in crate::executor) fn capture_var_attrs(
    env_vars: &HashMap<String, String>,
    name: &str,
) -> VarAttrs {
    VarAttrs {
        exported: is_marked_var(env_vars, EXPORTED_VARS, name),
        readonly: is_marked_var(env_vars, READONLY_VARS, name),
        integer: is_marked_var(env_vars, INTEGER_VARS, name),
        uppercase: is_marked_var(env_vars, UPPERCASE_VARS, name),
        lowercase: is_marked_var(env_vars, LOWERCASE_VARS, name),
        nameref: is_marked_var(env_vars, NAMEREF_VARS, name),
        array: is_marked_var(env_vars, ARRAY_VARS, name),
        assoc: is_marked_var(env_vars, ASSOC_VARS, name),
    }
}

pub(in crate::executor) fn set_var_attrs(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    attrs: VarAttrs,
) {
    set_marked_var(env_vars, EXPORTED_VARS, name, attrs.exported);
    set_marked_var(env_vars, READONLY_VARS, name, attrs.readonly);
    set_marked_var(env_vars, INTEGER_VARS, name, attrs.integer);
    set_marked_var(env_vars, UPPERCASE_VARS, name, attrs.uppercase);
    set_marked_var(env_vars, LOWERCASE_VARS, name, attrs.lowercase);
    set_marked_var(env_vars, NAMEREF_VARS, name, attrs.nameref);
    set_marked_var(env_vars, ARRAY_VARS, name, attrs.array);
    set_marked_var(env_vars, ASSOC_VARS, name, attrs.assoc);
}

pub(in crate::executor) fn is_valid_process_env(name: &str, value: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0')
}

pub(in crate::executor) fn set_process_env(name: &str, value: impl AsRef<str>) {
    let value = value.as_ref();
    if name != "TMPDIR" && is_valid_process_env(name, value) {
        env::set_var(name, value);
    }
}

pub(in crate::executor) fn safe_temp_dir_string() -> String {
    for name in ["TMPDIR", "TEMP", "TMP"] {
        if let Ok(value) = env::var(name) {
            if !value.is_empty() && !value.contains('\0') {
                return value;
            }
        }
    }

    env::current_dir()
        .map(|path| path.join("target").to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string())
}

pub(in crate::executor) fn set_marked_var(
    env_vars: &mut HashMap<String, String>,
    key: &str,
    name: &str,
    marked: bool,
) {
    if marked {
        mark_env_name(env_vars, key, name);
    } else {
        unmark_env_name(env_vars, key, name);
    }
}
