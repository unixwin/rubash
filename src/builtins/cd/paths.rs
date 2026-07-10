use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub(super) fn starts_with_dot_component(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::CurDir | Component::ParentDir)
    )
}

pub(super) fn current_logical_pwd(env_vars: &HashMap<String, String>) -> PathBuf {
    if let Some(pwd) = shell_var(env_vars, "PWD") {
        if cfg!(windows) && pwd.starts_with('/') {
            return PathBuf::from(pwd);
        }

        let path = PathBuf::from(pwd);
        if path.is_absolute() {
            return path;
        }
    }

    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub(super) fn logical_destination(old_pwd: &Path, target: &Path) -> PathBuf {
    let combined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        old_pwd.join(target)
    };

    normalize_logical_path(&combined)
}

pub(super) fn logical_destination_display(old_pwd: &Path, target: &Path) -> String {
    if !cfg!(windows) {
        return shell_display_path(&logical_destination(old_pwd, target));
    }

    let target_display = path_display_text(target);
    if target_display.starts_with('/') {
        return normalize_logical_display(&target_display);
    }

    let old_display = path_display_text(old_pwd);
    normalize_logical_display(&format!("{old_display}/{target_display}"))
}

pub(super) fn shell_var(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    env_vars
        .get(name)
        .cloned()
        .or_else(|| env::var(name).ok())
        .filter(|value| !value.is_empty())
}

pub(super) fn filesystem_path_for_display(
    dir: &str,
    env_vars: &HashMap<String, String>,
) -> PathBuf {
    // TODO(general.c/pathnames.h): keep Bash-visible /tmp paths logical on Windows.
    if cfg!(windows) {
        if dir.len() >= 3
            && dir.as_bytes()[0] == b'/'
            && dir.as_bytes()[2] == b'/'
            && dir.as_bytes()[1].is_ascii_alphabetic()
        {
            let drive = dir.as_bytes()[1] as char;
            return PathBuf::from(
                format!("{}:\\{}", drive.to_ascii_uppercase(), &dir[3..]).replace('/', "\\"),
            );
        }

        if dir == "/tmp" {
            if let Some(tmpdir) = shell_var(env_vars, "TMPDIR") {
                return PathBuf::from(tmpdir);
            }
        }
        if let Some(rest) = dir.strip_prefix("/tmp/") {
            if let Some(tmpdir) = shell_var(env_vars, "TMPDIR") {
                return PathBuf::from(tmpdir).join(rest);
            }
        }
    }

    PathBuf::from(dir)
}

pub(super) fn set_shell_env(env_vars: &mut HashMap<String, String>, name: &str, value: String) {
    env_vars.insert(name.to_string(), value.clone());
    env::set_var(name, OsString::from(value));
}

pub(super) fn shell_display_path(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows)
        && value.len() >= 3
        && value.as_bytes()[1] == b':'
        && value.as_bytes()[2] == b'/'
    {
        let drive = value.as_bytes()[0] as char;
        value = format!("/{}{}", drive.to_ascii_lowercase(), &value[2..]);
    }
    if value.is_empty() {
        "/".to_string()
    } else {
        value
    }
}

fn normalize_logical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        normalized
    }
}

fn normalize_logical_display(path: &str) -> String {
    let mut parts = Vec::new();
    let absolute = path.starts_with('/');

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    let normalized = parts.join("/");
    if absolute {
        format!("/{normalized}")
    } else if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized
    }
}

fn path_display_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
