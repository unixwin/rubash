use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::executor::path::shell_path_to_windows;

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
    // Windows absolute paths start with a drive letter (`C:/...`), not `/`;
    // `Path::is_absolute` covers both native and slash-drive forms.
    let normalized = if target.is_absolute() || target_display.starts_with('/') {
        normalize_logical_display(&target_display)
    } else {
        let old_display = path_display_text(old_pwd);
        normalize_logical_display(&format!("{old_display}/{target_display}"))
    };
    windows_slash_drive_display_to_native(&normalized).unwrap_or(normalized)
}

pub(super) fn shell_var(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    // HOME and OLDPWD are shell variables managed by the shell itself
    // (unset removes them from env_vars). On Windows, env::var still
    // returns the original process value after unset because
    // apply_required_windows_child_environment re-adds HOME from
    // USERPROFILE. So for HOME and OLDPWD, only check env_vars.
    if name == "HOME" || name == "OLDPWD" {
        return env_vars
            .get(name)
            .cloned()
            .filter(|value| !value.is_empty());
    }
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
    shell_path_to_windows(dir, env_vars)
}

pub(super) fn set_shell_env(env_vars: &mut HashMap<String, String>, name: &str, value: String) {
    env_vars.insert(name.to_string(), value.clone());
    env::set_var(name, OsString::from(value));
}

pub(super) fn shell_display_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        "/".to_string()
    } else {
        windows_slash_drive_display_to_native(&value).unwrap_or(value)
    }
}

/// OLDPWD carries the previous *logical* PWD value verbatim (builtins/cd.def:
/// `set_working_directory` stores the old `$PWD` string). It must not be
/// routed through `shell_display_path`, whose slash-drive→native bridge
/// would rewrite `/c/Users/...` into `C:/Users/...` and desynchronize
/// `OLDPWD` from the value `$PWD` held.
pub(super) fn logical_pwd_var_display(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        "/".to_string()
    } else {
        value
    }
}

pub(super) use crate::executor::path::shell_pwd_display_path;

fn windows_slash_drive_display_to_native(path: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }

    let bytes = path.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        return Some(format!("{drive}:/"));
    }
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b'/' && bytes[1].is_ascii_alphabetic() {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        return Some(format!("{drive}:{}", &path[2..]));
    }

    None
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

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{logical_destination_display, shell_display_path};
    #[cfg(windows)]
    use std::path::Path;

    #[cfg(windows)]
    #[test]
    fn logical_destination_display_converts_slash_drive_to_native() {
        assert_eq!(
            logical_destination_display(Path::new("/c/Users/example"), Path::new("repo")),
            "C:/Users/example/repo"
        );
        assert_eq!(
            logical_destination_display(Path::new("/c/Users/example/repo"), Path::new("..")),
            "C:/Users/example"
        );
        assert_eq!(
            logical_destination_display(Path::new("C:/Users/example"), Path::new("repo")),
            "C:/Users/example/repo"
        );
    }

    #[cfg(windows)]
    #[test]
    fn shell_display_path_preserves_posix_bridge_but_not_slash_drive() {
        assert_eq!(
            shell_display_path(Path::new("/c/Users/example")),
            "C:/Users/example"
        );
        assert_eq!(shell_display_path(Path::new("/usr")), "/usr");
    }
}
