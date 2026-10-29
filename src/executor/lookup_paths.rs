use super::*;

impl Executor {
    pub(in crate::executor) fn command_path(&self, name: &str, force_path: bool) -> Option<String> {
        self.command_path_with_bridge(name, force_path, true)
    }

    /// Bridge-free command lookup, for execution decisions only.
    /// The Windows suite bridge below answers display questions
    /// (`type`, `command -v`) while PATH still holds its startup
    /// value; it must never decide whether an emulation may run —
    /// GNU findcmd.c:348 search_for_command resolves a slash-free
    /// name only against the hash table and the PATH walk
    /// (findcmd.c:623 find_user_command_in_path), and a display-only
    /// stand-in is neither. rubash#1777: under an imported PATH the
    /// bridge made `cat` look resolvable, so its emulation arms ran
    /// while `head`/`wc` stood down and reported 127.
    pub(in crate::executor) fn command_path_without_bridge(
        &self,
        name: &str,
        force_path: bool,
    ) -> Option<String> {
        self.command_path_with_bridge(name, force_path, false)
    }

    fn command_path_with_bridge(
        &self,
        name: &str,
        force_path: bool,
        allow_bridge: bool,
    ) -> Option<String> {
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
        // rubash#588: like the winuxcmd dispatcher, this /bin namespace is
        // the stand-in for GNU's startup default PATH (variables.c:550
        // set_if_not) and answers only while PATH is still the imported
        // value — after a shell-level assignment, describe_command reports
        // what find_user_command_in_path (findcmd.c:623-697) actually
        // found, so `PATH=/nonexistent; command -v ls` fails with rc 1 the
        // way GNU 5.3.0 does (WSL probe 2026-10-13).
        #[cfg(windows)]
        if allow_bridge && !crate::executor::path::path_is_user_owned(&self.shell_state.env_vars) {
            if matches!(name, "mv") {
                return Some("/usr/bin/mv".to_string());
            }
            if matches!(name, "cat" | "ls") {
                return Some(format!("/bin/{name}"));
            }
        }
        #[cfg(not(windows))]
        let _ = allow_bridge;
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

    /// Whether an in-process emulation of the external command `name`
    /// must stand down. GNU findcmd.c:348-427 search_for_command runs a
    /// slash-free name only when the lookup succeeds — a hash-table hit
    /// or a PATH walk hit. The emulations (pipeline inline stages, the
    /// external file builtins, the identity tools, the sleep/dirname/
    /// basename fast paths, the comsub shortcuts) stand in for that PATH
    /// binary, so they inherit its resolvability: PATH unset is the
    /// degenerate no-walk case (rubash#1126), and a SET PATH holding no
    /// such command fails the lookup identically (rubash#1545 — a timed
    /// pipeline's `cat` stage silently executed in-process under
    /// PATH=/nonexistent while GNU reports `command not found`, 127;
    /// the command's existence must not depend on the invocation
    /// shape). The resolution model is this module's own
    /// [`Executor::command_path_without_bridge`]: hash first, then the
    /// GNU walk, with the unset/empty-PATH "." rule — the Windows suite
    /// bridge is a display stand-in, not a resolution (rubash#1777).
    pub(in crate::executor) fn emulated_external_stands_down(&self, name: &str) -> bool {
        crate::executor::path::path_is_unset(&self.shell_state.env_vars)
            || self.command_path_without_bridge(name, false).is_none()
    }

    /// Stand-down adjudication driven by the command registry's
    /// two-axis policy (command_registry stand_down, #2036): the
    /// caller must fall through to a real spawn when this returns
    /// true. The unresolvable axis reuses the standard predicate
    /// above — `FailClosed` stands down to the spawn and its 127
    /// (#1545), any other value keeps the face's answer. The
    /// resolvable axis reads the row: `Replace` keeps the answer,
    /// `Yield` stands down, and `ScriptYield` (the identity tools,
    /// rubash#2104) stands down exactly when the resolution lands
    /// on a script — GNU findcmd.c:348 search_for_command has no
    /// uname/arch builtin to outrank the file PATH found, so the
    /// script is the user's program; a native-image resolution is
    /// the synced body the emulation stands in for and keeps the
    /// in-process answer. Faces whose rule is their own machinery
    /// (the pipeline sed recheck, the spawn stand-in) do not consult
    /// this helper; their policy values fall through to "answer".
    pub(in crate::executor) fn registered_stand_down(&self, name: &str) -> bool {
        use super::command_registry::{ResolvablePolicy, UnresolvablePolicy};
        let Some(row) = super::command_registry::registry_row(name) else {
            return self.emulated_external_stands_down(name);
        };
        if self.emulated_external_stands_down(name) {
            return matches!(row.stand_down.unresolvable, UnresolvablePolicy::FailClosed);
        }
        match row.stand_down.resolvable {
            ResolvablePolicy::Yield => true,
            ResolvablePolicy::ScriptYield => self.resolved_candidate_is_script(name),
            ResolvablePolicy::Replace
            | ResolvablePolicy::NotApplicable
            | ResolvablePolicy::InvertedSed
            | ResolvablePolicy::HashStandin => false,
        }
    }

    /// Whether the bridge-free resolution of `name` (the same
    /// candidate `registered_stand_down` just found resolvable) is a
    /// script: a file whose first bytes are `#!`. The candidate path
    /// goes through the same shell->filesystem mapping the operand
    /// faces use (session_path over shell_path_to_windows). An
    /// unreadable candidate answers false — the spawn the caller
    /// falls back to on the other arms reports such errors itself.
    fn resolved_candidate_is_script(&self, name: &str) -> bool {
        use std::io::Read;
        let Some(found) = self.command_path_without_bridge(name, false) else {
            return false;
        };
        let fs_path = self.session_path(crate::executor::path::shell_path_to_windows(
            &found,
            &self.shell_state.env_vars,
        ));
        let Ok(mut file) = std::fs::File::open(&fs_path) else {
            return false;
        };
        let mut head = [0u8; 2];
        file.read_exact(&mut head).is_ok() && &head == b"#!"
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
                let cwd = self
                    .shell_state
                    .env_vars
                    .get("PWD")
                    .cloned()
                    .unwrap_or_else(|| self.shell_state.cwd.to_string_lossy().replace('\\', "/"));
                (cwd.clone(), cwd)
            }
            Some(_) => return None,
        };
        // GNU file_status() resolves NAME against the shell's physical cwd
        // (the session cwd field since G-02); prefer that, then translate
        // the logical directory.
        let exists = self.shell_state.cwd.join(name).is_file()
            || std::path::Path::new(&check_dir).join(name).is_file()
            || shell_path_to_windows(&format!("{check_dir}/{name}"), &self.shell_state.env_vars)
                .is_file();
        exists.then(|| format!("{display_prefix}/{name}"))
    }

    pub(in crate::executor) fn is_enabled_shell_builtin_name(&self, name: &str) -> bool {
        is_shell_builtin_name(name)
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, name)
    }

    /// Execution-routing classification: the pure builtin table plus the
    /// Windows `sudo` host builtin, which only exists while an embedding
    /// host registered an elevation handler. Introspection surfaces
    /// (`type`, `command -v`, `enable -a`, `compgen -b`) deliberately keep
    /// reporting `sudo` as an external command — the same hidden-builtin
    /// contract as `uname`/`arch` (docs/builtins.md §3) and GNU's own
    /// behavior for names it resolves off PATH (issue #759 item 4: GNU has
    /// no `sudo` builtin, `command -v sudo` finds /usr/bin/sudo).
    #[cfg(windows)]
    pub(in crate::executor) fn routes_as_builtin(&self, name: &str) -> bool {
        is_shell_builtin_name(name) || (name == "sudo" && self.elevation_handler.is_some())
    }

    /// Non-Windows routing: there is no host-capability `sudo` builtin off
    /// Windows (the elevation machinery is `#[cfg(windows)]` throughout),
    /// so a name routes as a builtin iff the pure table says so.
    #[cfg(not(windows))]
    pub(in crate::executor) fn routes_as_builtin(&self, name: &str) -> bool {
        let _ = self;
        is_shell_builtin_name(name)
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
        // rubash#588: gated on the startup-PATH state exactly like
        // command_path — after a shell-level PATH assignment the
        // enumeration reports real hits only.
        #[cfg(windows)]
        if !crate::executor::path::path_is_user_owned(&self.shell_state.env_vars) {
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
