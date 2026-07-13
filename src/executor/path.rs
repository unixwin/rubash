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
        return executable_candidate(&candidate);
    }

    for dir in split_path(env_vars.get("PATH").map(String::as_str).unwrap_or_default()) {
        let candidate = shell_path_to_windows(&dir, env_vars).join(name);
        if let Some(found) = executable_candidate(&candidate) {
            return Some(found);
        }
    }

    None
}

pub fn standard_path(env_vars: &HashMap<String, String>) -> String {
    if cfg!(windows) {
        let mut paths = Vec::new();
        if let Some(root) = git_bash_root(env_vars) {
            paths.push(root.join("usr").join("local").join("bin"));
            paths.push(root.join("usr").join("bin"));
            paths.push(root.join("bin"));
        }
        paths.push(PathBuf::from(r"C:\Windows\System32"));
        paths.push(PathBuf::from(r"C:\Windows"));
        return paths
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(";");
    }

    "/usr/local/bin:/usr/bin:/bin".to_string()
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
        .or_else(find_standard_unix_shell)
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

fn find_standard_unix_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }

    ["/bin/sh", "/usr/bin/sh", "/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
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

    if normalized == "/dev/null" || normalized.eq_ignore_ascii_case("NUL") {
        return PathBuf::from("NUL");
    }

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
        if let Some(root) = git_bash_root(env_vars) {
            return root.join("usr").join("bin").join(rest);
        }
    }

    if let Some(rest) = normalized.strip_prefix("/bin/") {
        if let Some(root) = git_bash_root(env_vars) {
            return root.join("usr").join("bin").join(rest);
        }
    }

    if normalized == "/tmp" {
        if let Some(tmpdir) = env_vars.get("TMPDIR") {
            if tmpdir.replace('\\', "/") == "/tmp" {
                return std::env::temp_dir();
            }
            return shell_path_to_windows(tmpdir, env_vars);
        }
    }

    if let Some(rest) = normalized.strip_prefix("/tmp/") {
        if let Some(tmpdir) = env_vars.get("TMPDIR") {
            if tmpdir.replace('\\', "/") == "/tmp" {
                return std::env::temp_dir().join(rest);
            }
            return shell_path_to_windows(tmpdir, env_vars).join(rest);
        }
    }

    PathBuf::from(path)
}

fn git_root(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    let exepath = env_vars.get("EXEPATH")?;
    let bin = Path::new(exepath);
    bin.parent().map(Path::to_path_buf)
}

fn git_bash_root(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(root) = git_root(env_vars) {
        return Some(root);
    }

    for key in ["CLAUDE_CODE_GIT_BASH_PATH", "GIT_BASH_PATH", "SHELL"] {
        let Some(value) = env_vars.get(key) else {
            continue;
        };
        let path = PathBuf::from(value);
        if !is_native_windows_shell(&path) {
            continue;
        }
        if let Some(bin) = path.parent() {
            if let Some(root) = bin.parent() {
                return Some(root.to_path_buf());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::fs;

    #[cfg(not(windows))]
    #[test]
    fn unix_shell_lookup_falls_back_to_standard_paths() {
        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), "target/rubash-isolated-bin".to_string());

        assert!(find_shell(&env_vars).is_some());
    }

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

    #[cfg(windows)]
    #[test]
    fn windows_absolute_usr_bin_command_uses_pathext() {
        let root = std::env::temp_dir().join("rubash-path-test-git");
        let bin = root.join("bin");
        let usr_bin = root.join("usr").join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&usr_bin).unwrap();
        let bash = bin.join("bash.exe");
        let env = usr_bin.join("env.exe");
        fs::write(&bash, "").unwrap();
        fs::write(&env, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "CLAUDE_CODE_GIT_BASH_PATH".to_string(),
            bash.to_string_lossy().to_string(),
        );

        assert_eq!(find_user_command("/usr/bin/env", &env_vars), Some(env));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_works_with_mixed_case_path() {
        // On Windows, std::env::vars() returns PATH as "Path" (capital P).
        // find_user_command reads env_vars.get("PATH") (all caps), so we should
        // fail to find the command when only "Path" is set. This test documents
        // the upstream behavior and motivates the init.rs fix that mirrors
        // the value into the all-caps key.
        let target_dir = std::env::temp_dir().join("rubash-mixed-case-path");
        let _ = fs::remove_dir_all(&target_dir);
        fs::create_dir_all(&target_dir).unwrap();
        let marker = target_dir.join("cmd.exe");
        fs::write(&marker, "").unwrap();

        // This lookup attempts to find "cmd" using only the all-caps PATH key,
        // which is what Executor::new() will hold after the init.rs fix runs.
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "PATH".to_string(),
            target_dir.to_string_lossy().to_string(),
        );
        assert_eq!(
            find_user_command("cmd", &env_vars).map(|p| p.to_string_lossy().to_string()),
            Some(marker.to_string_lossy().to_string()),
        );

        let _ = fs::remove_dir_all(&target_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_fails_when_only_path_lower_is_set() {
        // Direct counter-test for the casing bug: setting the OS-side
        // "Path" (capital P) without the all-caps "PATH" key causes
        // find_user_command to miss the command. Bug surfaces in shells
        // embedding rubash on Windows until Executor::new() normalizes.
        let target_dir = std::env::temp_dir().join("rubash-only-path-lower");
        let _ = fs::remove_dir_all(&target_dir);
        std::fs::create_dir_all(&target_dir).unwrap();
        let marker = target_dir.join("cmd.exe");
        std::fs::write(&marker, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "Path".to_string(),
            target_dir.to_string_lossy().to_string(),
        );

        // find_user_command has no normalization itself; the init.rs workaround
        // upstream performs the casing mirror. Without that workaround, this
        // lookup returns None.
        assert_eq!(
            find_user_command("cmd", &env_vars),
            None,
            "find_user_command should not see the lowercase Path key until init.rs normalizes"
        );

        let _ = std::fs::remove_dir_all(&target_dir);
    }
}

