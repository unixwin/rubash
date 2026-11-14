use super::*;

impl Executor {
    pub(in crate::executor) fn command_path(&self, name: &str, force_path: bool) -> Option<String> {
        self.command_path_with_bridge(name, force_path, true, true)
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
        self.command_path_with_bridge(name, force_path, false, false)
    }

    fn command_path_with_bridge(
        &self,
        name: &str,
        force_path: bool,
        allow_bridge: bool,
        describe_spelling: bool,
    ) -> Option<String> {
        if !force_path {
            if let Some(path) = crate::builtins::hash::hashed_path(&self.shell_state.env_vars, name)
            {
                return Some(path);
            }
        }
        // rubash#2717: a slash-bearing name is never PATH-walked at
        // the describe faces — GNU describe_command qualifies it by
        // file_status alone (FS_EXECABLE) and reports it verbatim.
        // The unconditional absolute-name shortcut and the lose-on
        // find_user_command fallthrough below both reported
        // non-executable files and missing paths as found.
        if crate::executor::path::has_path_separator(name) {
            return crate::executor::path::slash_name_is_describable(
                name,
                &self.shell_state.env_vars,
            )
            .then(|| name.to_string());
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
        let found = find_user_command(name, &self.shell_state.env_vars)?;
        // rubash#3147: the describe faces (describe_spelling) print
        // the walk's ONE constructed string — the as-written
        // `<elem>/<name>` join (findcmd.c:553 sh_makepath) that
        // describe_command takes from find_user_command and prints
        // verbatim (builtins/type.def:379, :425/:427). The anchored
        // resolution form (`/tmp/<fixture>/./rel1/foo`, `/./` residue
        // included) is the execution answer, not the display:
        // recorded_resolution_spelling applies the same
        // anchor-equality determination as the hash recording
        // (rubash#2696) — the join is the display exactly when
        // anchoring it lands on the file the walk produced, and any
        // other resolution keeps the anchored display form. The
        // execution-decision face (command_path_without_bridge)
        // always keeps the anchored form.
        if describe_spelling {
            Some(crate::executor::path::recorded_resolution_spelling(
                name,
                &found,
                &self.shell_state.env_vars,
            ))
        } else {
            Some(shell_display_path(
                &found.to_string_lossy().replace('\\', "/"),
            ))
        }
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

        // rubash#2717: same file_status qualification as
        // command_path — a slash-bearing name is reported (verbatim,
        // once) only when executable; the PATH enumeration below
        // never applies to it (GNU absolute_program).
        if crate::executor::path::has_path_separator(name) {
            if crate::executor::path::slash_name_is_describable(name, &self.shell_state.env_vars) {
                paths.push(name.to_string());
            }
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
        // GNU findcmd.c:477-481 user_command_matches iterates
        // path_value("PATH"): an unset PATH yields no list and no match
        // at all, while a set PATH (an empty one normalizes to "." in
        // path_value, findcmd.c:266-281) is walked element by element
        // by split_shell_path's extract_colon_unit walk. rubash#2943:
        // this enumeration previously ALSO pushed command_path_in_dot
        // for an empty PATH — the walk's own "." element already
        // contributes that "./name" — and ran the walk on "" for an
        // unset PATH, so both shapes reported a phantom cwd match.
        // (command_path_in_dot still serves the single-match describe
        // in command_path, where GNU's find_user_command faces for
        // unset/empty PATH are exactly its arms.)
        if let Some(path_value) = self.shell_state.env_vars.get("PATH") {
            for dir in split_shell_path(path_value) {
                let candidate = shell_path_to_windows(&dir, &self.shell_state.env_vars).join(name);
                // rubash#3147: the existence probe resolves against
                // the SESSION cwd, exactly like the single-match walk
                // (find_in_path_walk_unix anchors its candidate with
                // anchor_at_session_cwd — GNU file_status resolves
                // the constructed name against the shell's cwd, not
                // the process cwd). The unanchored probe lost every
                // relative-element match after a `cd`. The DISPLAYED
                // string stays the as-written join itself (the
                // candidate before anchoring), never the anchored
                // form.
                let anchored = crate::executor::path::anchor_at_session_cwd(
                    candidate.clone(),
                    &self.shell_state.env_vars,
                );
                if anchored.is_file() {
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
        }

        paths
    }
}
