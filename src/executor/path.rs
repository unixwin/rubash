//! path module.
//!
//! GNU Bash source ownership:
// - findcmd.c
// - findcmd.h

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn find_user_command(name: &str, env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    if has_path_separator(name) {
        let candidate = shell_path_to_windows(name, env_vars);
        return candidate.is_file().then_some(candidate);
    }

    for dir in split_path(env_vars.get("PATH").map(String::as_str).unwrap_or_default()) {
        let candidate = shell_path_to_windows(&dir, env_vars).join(name);
        if let Some(found) = executable_candidate(&candidate) {
            return Some(found);
        }
    }

    None
}

pub fn find_shell(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if cfg!(windows) {
        return find_windows_git_bash_from_env(env_vars)
            .or_else(find_windows_git_bash)
            .or_else(|| find_native_shell_on_path(env_vars));
    }

    ["sh", "bash"]
        .into_iter()
        .find_map(|name| find_user_command(name, env_vars))
}

pub fn should_run_with_shell(path: &Path) -> bool {
    if cfg!(windows) {
        !matches!(
            path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase),
            Some(ext) if matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd")
        )
    } else {
        false
    }
}

fn executable_candidate(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }

    if cfg!(windows) {
        for ext in executable_extensions() {
            let candidate = path.with_extension(ext);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn executable_extensions() -> Vec<String> {
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

fn find_windows_git_bash() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }

    [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn find_windows_git_bash_from_env(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    for key in ["CLAUDE_CODE_GIT_BASH_PATH", "GIT_BASH_PATH", "SHELL"] {
        let Some(value) = env_vars.get(key) else {
            continue;
        };
        let path = PathBuf::from(value);
        if is_native_windows_shell(&path) {
            return Some(path);
        }
    }

    let path = env_vars.get("PATH")?;
    split_path(path)
        .into_iter()
        .map(PathBuf::from)
        .flat_map(|dir| [dir.join("bash.exe"), dir.join("sh.exe")])
        .find(|path| is_native_windows_shell(path))
}

fn find_native_shell_on_path(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    ["sh", "bash"]
        .into_iter()
        .filter_map(|name| find_user_command(name, env_vars))
        .find(|path| is_native_windows_shell(path))
}

fn is_native_windows_shell(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

fn split_path(path: &str) -> Vec<String> {
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

fn has_path_separator(name: &str) -> bool {
    name.contains('/') || name.contains('\\')
}

pub(crate) fn shell_path_to_windows(path: &str, env_vars: &HashMap<String, String>) -> PathBuf {
    if !cfg!(windows) {
        return PathBuf::from(path);
    }

    let normalized = path.replace('\\', "/");

    if normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b'/'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        let drive = normalized.as_bytes()[1] as char;
        return PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &normalized[3..]).replace('/', "\\"),
        );
    }

    if let Some(rest) = normalized.strip_prefix("/usr/bin/") {
        if let Some(root) = git_root(env_vars) {
            return root.join("usr").join("bin").join(rest);
        }
    }

    if let Some(rest) = normalized.strip_prefix("/bin/") {
        if let Some(root) = git_root(env_vars) {
            return root.join("usr").join("bin").join(rest);
        }
    }

    if normalized == "/tmp" {
        if let Some(tmpdir) = env_vars.get("TMPDIR") {
            return PathBuf::from(tmpdir);
        }
    }

    if let Some(rest) = normalized.strip_prefix("/tmp/") {
        if let Some(tmpdir) = env_vars.get("TMPDIR") {
            return PathBuf::from(tmpdir).join(rest);
        }
    }

    PathBuf::from(path)
}

fn git_root(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    let exepath = env_vars.get("EXEPATH")?;
    let bin = Path::new(exepath);
    bin.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_shell_prefers_native_exe_from_env() {
        let native_exe = std::env::current_exe().unwrap();
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "CLAUDE_CODE_GIT_BASH_PATH".to_string(),
            native_exe.to_string_lossy().to_string(),
        );
        env_vars.insert(
            "PATH".to_string(),
            r"D:\tmp\guard-bin;D:\Git\bin".to_string(),
        );

        assert_eq!(find_shell(&env_vars), Some(native_exe));
    }
}
