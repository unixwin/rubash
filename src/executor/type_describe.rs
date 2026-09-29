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
                && is_posix_special_builtin(name)
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
        }

        if let Some(path) = self.command_path(name, force_path) {
            match mode {
                TypeDescribeMode::Verbose => {
                    if crate::builtins::hash::hashed_path(&self.shell_state.env_vars, name)
                        .is_some()
                    {
                        writeln!(stdout, "{name} is hashed ({path})")?;
                    } else {
                        writeln!(stdout, "{name} is {path}")?;
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
                && is_posix_special_builtin(name)
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
        }

        for path in self.command_paths(name, force_path) {
            match mode {
                TypeDescribeMode::Verbose => {
                    // GNU type.def: the `is hashed (...)` wording lives only
                    // in the phash_search SHORTDESC branch, which CDESC_ALL
                    // never reaches (`all == 0 || CDESC_FORCE_PATH` guard, and
                    // -P clears CDESC_SHORTDESC). `type -a` always prints the
                    // plain `name is path` form.
                    writeln!(stdout, "{name} is {path}")?;
                }
                TypeDescribeMode::Reusable | TypeDescribeMode::PathOnly => {
                    writeln!(stdout, "{path}")?
                }
                TypeDescribeMode::TypeOnly => writeln!(stdout, "file")?,
            }
            found = true;
        }

        Ok(found)
    }
}
