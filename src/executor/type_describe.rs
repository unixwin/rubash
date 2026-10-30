use super::*;

impl Executor {
    // rubash#289d: the raw-stdout `describe_name` (println! describe) was
    // removed with its only caller — describes must go through
    // describe_name_with_io so output reaches the buffered single-truth
    // path (comsub capture, redirects, fd table).

    pub(in crate::executor) fn describe_name_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        if !force_path {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.shell_state.aliases.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            writeln!(stdout, "{name} is aliased to `{}'", alias.value)?;
                        }
                        TypeDescribeMode::Reusable => {
                            writeln!(stdout, "alias {name}='{}'", alias.value)?
                        }
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "alias")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    return Ok(true);
                }
            }

            // GNU describe_command order: keywords, posix special builtins
            // before functions, then functions, then other builtins.
            if is_shell_keyword(name) {
                match mode {
                    TypeDescribeMode::Verbose => writeln!(stdout, "{name} is a shell keyword")?,
                    TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "keyword")?,
                    TypeDescribeMode::PathOnly => {}
                }
                return Ok(true);
            }

            if self.posix_mode_enabled()
                && self.is_enabled_shell_builtin_name(name)
                && super::command_registry::is_special_builtin(name)
            {
                match mode {
                    TypeDescribeMode::Verbose => {
                        writeln!(stdout, "{name} is a special shell builtin")?
                    }
                    TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                return Ok(true);
            }

            if !skip_functions {
                if let Some(body) = self.shell_state.functions.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            self.write_function_description(name, &body.commands, stdout)?
                        }
                        TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "function")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    return Ok(true);
                }
            }

            if self.is_enabled_shell_builtin_name(name) {
                match mode {
                    TypeDescribeMode::Verbose => writeln!(stdout, "{name} is a shell builtin")?,
                    TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                return Ok(true);
            }

            // #2036 S2 disclosure: a live in-process claim (emulation
            // layer / fast path) is reported as what it is, not as the
            // PATH file it replaces.
            if self.describe_emulated_claim_with_io(name, mode, stdout)? {
                return Ok(true);
            }
        }

        if let Some(path) = self.command_path(name, force_path) {
            match mode {
                TypeDescribeMode::Verbose => {
                    // niubash#567 annotation: the resolved-path line
                    // is the one face that may carry the host route
                    // provider's suffix (see route_annotation_for).
                    // No provider, or a name it does not claim:
                    // the line is byte-identical to the bare form.
                    let suffix = self
                        .route_annotation_for(name)
                        .map(|annotation| format!(" — {annotation}"))
                        .unwrap_or_default();
                    if crate::builtins::hash::hashed_path(&self.shell_state.env_vars, name)
                        .is_some()
                    {
                        writeln!(stdout, "{name} is hashed ({path}){suffix}")?;
                    } else {
                        writeln!(stdout, "{name} is {path}{suffix}")?;
                    }
                }
                TypeDescribeMode::Reusable | TypeDescribeMode::PathOnly => {
                    writeln!(stdout, "{path}")?
                }
                TypeDescribeMode::TypeOnly => writeln!(stdout, "file")?,
            }
            return Ok(true);
        }

        Ok(false)
    }

    pub(in crate::executor) fn describe_name_all_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        let mut found = false;

        // GNU type.def describe_command: with CDESC_ALL the alias/keyword/
        // function/builtin blocks run whenever !CDESC_FORCE_PATH, regardless
        // of CDESC_PATH_ONLY. Under `type -ap` they print nothing (the
        // PATH_ONLY arm is silent) but still set found=1, so the overall
        // exit status stays 0 even when no disk file matches.
        if !force_path {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.shell_state.aliases.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            writeln!(stdout, "{name} is aliased to `{}'", alias.value)?;
                        }
                        TypeDescribeMode::Reusable => {
                            writeln!(stdout, "alias {name}='{}'", alias.value)?
                        }
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "alias")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    found = true;
                }
            }

            // type -a (all != 0): describe_command keeps scanning after each
            // match. Order: keyword, posix special builtin (which then
            // suppresses the duplicate plain-builtin block via skipbuiltin),
            // function, builtin.
            let mut skip_builtin = false;
            if is_shell_keyword(name) {
                match mode {
                    TypeDescribeMode::Verbose => writeln!(stdout, "{name} is a shell keyword")?,
                    TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "keyword")?,
                    TypeDescribeMode::PathOnly => {}
                }
                found = true;
            }

            if self.posix_mode_enabled()
                && self.is_enabled_shell_builtin_name(name)
                && super::command_registry::is_special_builtin(name)
            {
                match mode {
                    TypeDescribeMode::Verbose => {
                        writeln!(stdout, "{name} is a special shell builtin")?
                    }
                    TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                skip_builtin = true;
                found = true;
            }

            if !skip_functions {
                if let Some(body) = self.shell_state.functions.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            self.write_function_description(name, &body.commands, stdout)?
                        }
                        TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "function")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    found = true;
                }
            }

            if !skip_builtin && self.is_enabled_shell_builtin_name(name) {
                match mode {
                    TypeDescribeMode::Verbose => writeln!(stdout, "{name} is a shell builtin")?,
                    TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                found = true;
            }

            // #2036 S2 disclosure (see describe_emulated_claim_with_io):
            // the claim line takes the builtin line's position; the PATH
            // enumeration below still runs, as it does for builtins.
            if self.describe_emulated_claim_with_io(name, mode, stdout)? {
                found = true;
            }
        }

        let mut first_path = true;
        for path in self.command_paths(name, force_path) {
            match mode {
                TypeDescribeMode::Verbose => {
                    // GNU type.def: the `is hashed (...)` wording lives only
                    // in the phash_search SHORTDESC branch, which CDESC_ALL
                    // never reaches (`all == 0 || CDESC_FORCE_PATH` guard, and
                    // -P clears CDESC_SHORTDESC). `type -a` always prints the
                    // plain `name is path` form.
                    // niubash#567: the route annotation rides the
                    // FIRST enumerated path only — that is the
                    // resolution dispatch would run; later PATH
                    // entries are unannotated enumerations.
                    let suffix = if first_path {
                        self.route_annotation_for(name)
                            .map(|annotation| format!(" — {annotation}"))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    writeln!(stdout, "{name} is {path}{suffix}")?;
                }
                TypeDescribeMode::Reusable | TypeDescribeMode::PathOnly => {
                    writeln!(stdout, "{path}")?
                }
                TypeDescribeMode::TypeOnly => writeln!(stdout, "file")?,
            }
            first_path = false;
            found = true;
        }

        Ok(found)
    }

    /// #2036 S2 disclosure ruling: when `name` carries a live
    /// simple-command claim whose registry projection is
    /// `Introspection::Emulated`, print the emulation answer for
    /// `mode` and return true; otherwise print nothing and return
    /// false. The answer forms mirror the builtin forms one class up:
    /// Verbose names the replaced body ("X is a shell emulation of
    /// <path>"), TypeOnly is the single token `emulation`, Reusable
    /// (`command -v`) is the bare name — invoking the name reproduces
    /// the in-process behavior, invoking the path would not — and
    /// PathOnly is silent-but-found, like a builtin under `type -p`.
    ///
    /// The liveness test is the dispatch test itself, face by face,
    /// so the report cannot claim a body dispatch would not run: the
    /// emulation layer needs its host flag on top of the shared
    /// stand-down predicate, the fast paths consult the predicate
    /// alone, and a Windows-only row claims nothing off Windows.
    /// Liveness implies the stand-down lookup resolved, so the path
    /// the sentence names always exists.
    /// niubash#567 discoverability ruling — checkable, never
    /// noisy: the registered route provider's annotation for
    /// `name`, consulted ONLY by the Verbose describe faces above.
    /// The annotation is a description suffix, not a replacement
    /// identity: unlike the emulation disclosure, the command
    /// still IS the resolved file (the provider routes calls to
    /// it in-process, with the file as the standing fallback), so
    /// `command -v`, `hash`, and `type -t` keep reporting the
    /// external identity byte-for-byte in every provider state,
    /// and with no provider registered — or a name the provider
    /// does not claim — every describe byte is unchanged.
    fn route_annotation_for(&self, name: &str) -> Option<String> {
        let slot = self.external_route_provider.as_ref()?;
        let slot = slot.try_borrow().ok()?;
        slot.0.route_annotation(name)
    }

    fn describe_emulated_claim_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        use super::command_registry::{CfgGate, ClaimClass, Introspection};
        let Some(row) = super::command_registry::registry_row(name) else {
            return Ok(false);
        };
        if row.introspection != Introspection::Emulated {
            return Ok(false);
        }
        if row.cfg == CfgGate::WindowsOnly && !cfg!(windows) {
            return Ok(false);
        }
        let live = match row.class {
            ClaimClass::EmulatedExternal => {
                self.external_file_builtins_enabled && !self.emulated_external_stands_down(name)
            }
            ClaimClass::HiddenFastpath | ClaimClass::Fastpath => {
                !self.emulated_external_stands_down(name)
            }
            _ => false,
        };
        if !live {
            return Ok(false);
        }
        let Some(path) = self.command_path_without_bridge(name, false) else {
            return Ok(false);
        };
        match mode {
            TypeDescribeMode::Verbose => writeln!(stdout, "{name} is a shell emulation of {path}")?,
            TypeDescribeMode::Reusable => writeln!(stdout, "{name}")?,
            TypeDescribeMode::TypeOnly => writeln!(stdout, "emulation")?,
            TypeDescribeMode::PathOnly => {}
        }
        Ok(true)
    }
}
