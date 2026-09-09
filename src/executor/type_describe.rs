use super::*;

impl Executor {
    pub(in crate::executor) fn describe_name(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        functions_only: bool,
    ) -> bool {
        if functions_only {
            if let Some(body) = self.functions.get(name) {
                match mode {
                    TypeDescribeMode::Verbose => {
                        self.print_function_description(name, &body.commands)
                    }
                    TypeDescribeMode::Reusable => println!("{name}"),
                    TypeDescribeMode::TypeOnly => println!("function"),
                    TypeDescribeMode::PathOnly => {}
                }
                return true;
            }
            return false;
        }
        if !force_path {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.aliases.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            println!("{name} is aliased to `{}'", alias.value);
                        }
                        TypeDescribeMode::Reusable => println!("alias {name}='{}'", alias.value),
                        TypeDescribeMode::TypeOnly => println!("alias"),
                        TypeDescribeMode::PathOnly => {}
                    }
                    return true;
                }
            }

            // GNU describe_command order (builtins/type.def:240-330):
            // reserved words, then -- in posix mode -- special builtins
            // BEFORE functions (execute_cmd.c:4659 "Posix.2 says special
            // builtins are found before functions"), then functions, then
            // other builtins.
            if is_shell_keyword(name) {
                match mode {
                    TypeDescribeMode::Verbose => println!("{name} is a shell keyword"),
                    TypeDescribeMode::Reusable => println!("{name}"),
                    TypeDescribeMode::TypeOnly => println!("keyword"),
                    TypeDescribeMode::PathOnly => {}
                }
                return true;
            }

            if self.posix_mode_enabled()
                && self.is_enabled_shell_builtin_name(name)
                && is_posix_special_builtin(name)
            {
                match mode {
                    TypeDescribeMode::Verbose => println!("{name} is a special shell builtin"),
                    TypeDescribeMode::Reusable => println!("{name}"),
                    TypeDescribeMode::TypeOnly => println!("builtin"),
                    TypeDescribeMode::PathOnly => {}
                }
                return true;
            }

            if !skip_functions {
                if let Some(body) = self.functions.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            self.print_function_description(name, &body.commands)
                        }
                        TypeDescribeMode::Reusable => println!("{name}"),
                        TypeDescribeMode::TypeOnly => println!("function"),
                        TypeDescribeMode::PathOnly => {}
                    }
                    return true;
                }
            }

            if self.is_enabled_shell_builtin_name(name) {
                match mode {
                    TypeDescribeMode::Verbose => println!("{name} is a shell builtin"),
                    TypeDescribeMode::Reusable => println!("{name}"),
                    TypeDescribeMode::TypeOnly => println!("builtin"),
                    TypeDescribeMode::PathOnly => {}
                }
                return true;
            }
        }

        if let Some(path) = self.command_path(name, force_path) {
            match mode {
                TypeDescribeMode::Verbose => {
                    if crate::builtins::hash::hashed_path(&self.env_vars, name).is_some() {
                        println!("{name} is hashed ({path})");
                    } else {
                        println!("{name} is {path}");
                    }
                }
                TypeDescribeMode::Reusable | TypeDescribeMode::PathOnly => println!("{path}"),
                TypeDescribeMode::TypeOnly => println!("file"),
            }
            return true;
        }

        false
    }

    pub(in crate::executor) fn describe_name_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        functions_only: bool,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        if functions_only {
            if let Some(body) = self.functions.get(name) {
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
            return Ok(false);
        }
        if !force_path {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.aliases.get(name) {
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
                if let Some(body) = self.functions.get(name) {
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
                    if crate::builtins::hash::hashed_path(&self.env_vars, name).is_some() {
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

    pub(in crate::executor) fn describe_name_all(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
    ) -> Result<bool, ExecuteError> {
        let mut stdout = std::io::stdout().lock();
        self.describe_name_all_with_io(name, mode, force_path, skip_functions, false, &mut stdout)
    }

    pub(in crate::executor) fn describe_name_all_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        functions_only: bool,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        if functions_only {
            if let Some(body) = self.functions.get(name) {
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
            return Ok(false);
        }
        let mut found = false;

        if !force_path && mode != TypeDescribeMode::PathOnly {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.aliases.get(name) {
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
                if let Some(body) = self.functions.get(name) {
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
                    if !force_path
                        && crate::builtins::hash::hashed_path(&self.env_vars, name).is_some()
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
            found = true;
        }

        Ok(found)
    }
}
