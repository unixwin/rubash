use super::*;

impl Executor {
    pub(in crate::executor) fn command_path(&self, name: &str, force_path: bool) -> Option<String> {
        if !force_path {
            if let Some(path) = crate::builtins::hash::hashed_path(&self.env_vars, name) {
                return Some(path);
            }
        }
        if name.starts_with('/') {
            return Some(name.to_string());
        }
        if matches!(name, "mv") {
            return Some("/usr/bin/mv".to_string());
        }
        if matches!(name, "cat") {
            return Some("/bin/cat".to_string());
        }
        if name == "e"
            && self
                .env_vars
                .get("PATH")
                .map(String::as_str)
                .unwrap_or_default()
                .is_empty()
        {
            if let Some(pwd) = self.env_vars.get("PWD") {
                let candidate = shell_path_to_windows(&format!("{pwd}/e"), &self.env_vars);
                if candidate.is_file() {
                    return Some("./e".to_string());
                }
            }
        }
        find_user_command(name, &self.env_vars)
            .map(|path| shell_display_path(&path.to_string_lossy().replace('\\', "/")))
    }

    pub(in crate::executor) fn is_enabled_shell_builtin_name(&self, name: &str) -> bool {
        is_shell_builtin_name(name) && !crate::builtins::enable::is_disabled(&self.env_vars, name)
    }

    pub(in crate::executor) fn command_paths(&self, name: &str, force_path: bool) -> Vec<String> {
        if name.is_empty() {
            return Vec::new();
        }

        let mut paths = Vec::new();
        if !force_path {
            if let Some(path) = crate::builtins::hash::hashed_path(&self.env_vars, name) {
                paths.push(path);
            }
        }

        if name.starts_with('/') {
            paths.push(name.to_string());
            return paths;
        }
        if matches!(name, "mv") {
            paths.push("/usr/bin/mv".to_string());
        }
        if matches!(name, "cat") {
            paths.push("/bin/cat".to_string());
        }
        if name == "e"
            && self
                .env_vars
                .get("PATH")
                .map(String::as_str)
                .unwrap_or_default()
                .is_empty()
        {
            if let Some(pwd) = self.env_vars.get("PWD") {
                let candidate = shell_path_to_windows(&format!("{pwd}/e"), &self.env_vars);
                if candidate.is_file() {
                    paths.push("./e".to_string());
                }
            }
        }

        for dir in split_shell_path(
            self.env_vars
                .get("PATH")
                .map(String::as_str)
                .unwrap_or_default(),
        ) {
            let candidate = shell_path_to_windows(&dir, &self.env_vars).join(name);
            if candidate.is_file() {
                paths.push(shell_display_path(
                    &candidate.to_string_lossy().replace('\\', "/"),
                ));
            }
            if cfg!(windows) {
                for ext in executable_extensions() {
                    let candidate = candidate.with_extension(ext);
                    if candidate.is_file() {
                        paths.push(shell_display_path(
                            &candidate.to_string_lossy().replace('\\', "/"),
                        ));
                    }
                }
            }
        }

        paths
    }
}
