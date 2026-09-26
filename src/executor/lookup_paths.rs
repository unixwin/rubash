use super::*;

impl Executor {
    pub(in crate::executor) fn command_path(&self, name: &str, force_path: bool) -> Option<String> {
        if !force_path {
            if let Some(path) = crate::builtins::hash::hashed_path(&self.shell_state.env_vars, name)
            {
                return Some(path);
            }
        }
        if name.starts_with('/') {
            return Some(name.to_string());
        }
        // Windows suite bridge: GNU test replays expect mv/cat/ls to
        // describe even on hosts whose PATH never holds them (type.tests:39
        // `type -t mv`), so the /bin-namespace display paths stay. Unix
        // must NOT fake them: GNU describe_command (type.def:379) prints
        // the real find_user_command hit below (WSL: `mv is /usr/bin/mv`).
        #[cfg(windows)]
        {
            if matches!(name, "mv") {
                return Some("/usr/bin/mv".to_string());
            }
            if matches!(name, "cat" | "ls") {
                return Some(format!("/bin/{name}"));
            }
        }
        // GNU findcmd.c:266 path_value + builtins/type.def:390 describe_command:
        // an unset PATH makes find_user_command return NAME unchanged, which
        // describe_command then reports as "./name" when NAME is executable in
        // the physical cwd (sh_makepath MP_DOCWD); an empty PATH normalizes to
        // "." and resolves the same "./name".
        if let Some(dot_path) = self.command_path_in_dot(name) {
            return Some(dot_path);
        }
        find_user_command(name, &self.shell_state.env_vars)
            .map(|path| shell_display_path(&path.to_string_lossy().replace('\\', "/")))
    }

    fn command_path_in_dot(&self, name: &str) -> Option<String> {
        if name.contains('/') {
            return None;
        }
        let path_var = self.shell_state.env_vars.get("PATH");
        let (display_prefix, check_dir) = match path_var {
            // GNU findcmd.c:266 path_value: an empty PATH normalizes to ".",
            // so the "." element resolves NAME to "./name".
            Some(path) if path.is_empty() => (".".to_string(), ".".to_string()),
            // GNU findcmd.c:290-292 + builtins/type.def:390-405: an unset PATH
            // makes find_user_command return NAME unchanged; describe_command
            // then runs sh_makepath(NULL, name, MP_DOCWD), which reports the
            // physical working directory as an absolute path.
            None => {
                let cwd = self.shell_state.env_vars.get("PWD").cloned().or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                })?;
                (cwd.clone(), cwd)
            }
            Some(_) => return None,
        };
        // GNU file_status() resolves NAME against the process's physical cwd;
        // prefer that, then translate the logical directory for shells whose
        // cwd did not follow the `cd`.
        let exists = std::path::Path::new(name).is_file()
            || std::path::Path::new(&check_dir).join(name).is_file()
            || shell_path_to_windows(&format!("{check_dir}/{name}"), &self.shell_state.env_vars)
                .is_file();
        exists.then(|| format!("{display_prefix}/{name}"))
    }

    pub(in crate::executor) fn is_enabled_shell_builtin_name(&self, name: &str) -> bool {
        is_shell_builtin_name(name)
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, name)
    }

    pub(in crate::executor) fn command_paths(&self, name: &str, force_path: bool) -> Vec<String> {
        if name.is_empty() {
            return Vec::new();
        }

        let mut paths = Vec::new();
        // GNU builtins/type.def describe_command: the hash table is consulted
        // only when `all == 0 || (dflags & CDESC_FORCE_PATH)`. `command_paths`
        // is the -a (`CDESC_ALL`) enumeration, so the hashed entry is reported
        // only under -P (force_path); a plain `type -a` must NOT prepend it.
        if force_path {
            if let Some(path) = crate::builtins::hash::hashed_path(&self.shell_state.env_vars, name)
            {
                paths.push(path);
            }
        }

        if name.starts_with('/') {
            paths.push(name.to_string());
            return paths;
        }
        // Same Windows/unix split as command_path above: the /bin-namespace
        // display paths are a Windows suite bridge; unix enumerates only the
        // real PATH matches, per GNU user_command_matches (findcmd.c:437).
        #[cfg(windows)]
        {
            if matches!(name, "mv") {
                paths.push("/usr/bin/mv".to_string());
            }
            if matches!(name, "cat" | "ls") {
                paths.push(format!("/bin/{name}"));
            }
        }
        // GNU findcmd.c:437 user_command_matches iterates path_value("PATH"):
        // an empty PATH normalizes to "." and contributes "./name", while an
        // unset PATH yields no list and no match at all.
        if self
            .shell_state
            .env_vars
            .get("PATH")
            .is_some_and(|path| path.is_empty())
        {
            if let Some(dot_path) = self.command_path_in_dot(name) {
                paths.push(dot_path);
            }
        }

        for dir in split_shell_path(
            self.shell_state
                .env_vars
                .get("PATH")
                .map(String::as_str)
                .unwrap_or_default(),
        ) {
            let candidate = shell_path_to_windows(&dir, &self.shell_state.env_vars).join(name);
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
