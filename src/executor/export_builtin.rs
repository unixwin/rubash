use super::*;

impl Executor {
    pub(in crate::executor) fn execute_export(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if export_args_request_functions(&cmd.words[1..]) {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status =
                self.execute_export_functions(&cmd.words[1..], &mut stdout, &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }
        self.mark_posix_function_export_touches(&cmd.words[1..]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::setattr::export_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &mut self.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        if status == 0 {
            self.sync_setattr_typed_assignments(cmd.words[1..].iter().map(String::as_str));
            // Check for locale environment changes (LC_ALL, LC_CTYPE, LANG)
            for word in &cmd.words[1..] {
                if word.starts_with("LC_ALL=")
                    || word.starts_with("LC_CTYPE=")
                    || word.starts_with("LANG=")
                {
                    crate::locale::check_setlocale_warning();
                    break;
                }
            }
        }
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    /// export/readonly mutate only the legacy env_vars map, but parameter
    /// expansion reads the typed shell_state.variables owner first, so a
    /// scalar that already exists there (e.g. HOME seeded at startup) keeps a
    /// stale value after an export NAME=value assignment. Mirror the assigned
    /// values into the typed owner for plain scalar names; arrays/assocs/
    /// namerefs are synced by their own paths.
    pub(in crate::executor) fn sync_setattr_typed_assignments<'a>(
        &mut self,
        args: impl Iterator<Item = &'a str>,
    ) {
        for arg in args {
            if arg == "--" {
                continue;
            }
            if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
                continue;
            }
            if !arg.contains('=') {
                continue;
            }
            let (raw_name, _) = arg.split_once('=').unwrap_or((arg, ""));
            let name = raw_name.strip_suffix('+').unwrap_or(raw_name);
            let (base, _) = assignment_name_and_append(name);
            if is_marked_var(&self.env_vars, ARRAY_VARS, base)
                || is_marked_var(&self.env_vars, ASSOC_VARS, base)
                || is_marked_var(&self.env_vars, NAMEREF_VARS, base)
            {
                continue;
            }
            match self.env_vars.get(base) {
                Some(value) => match self.shell_state.variables.get_mut(base) {
                    Some(variable) => {
                        variable.value = crate::shell::ShellValue::Scalar(value.clone());
                    }
                    None => {
                        let _ = self.shell_state.variables.set_scalar(base, value.clone());
                    }
                },
                None => {
                    self.shell_state.variables.remove(base);
                }
            }
        }
    }

    pub(in crate::executor) fn mark_posix_function_export_touches(&mut self, args: &[String]) {
        if self.function_depth == 0 || !self.posix_mode_enabled() {
            return;
        }
        let mut names_started = false;
        for arg in args {
            if arg == "--" {
                names_started = true;
                continue;
            }
            if !names_started && arg.starts_with('-') && arg != "-" {
                continue;
            }
            names_started = true;
            let Some(name) = local_assignment_name(arg) else {
                continue;
            };
            mark_env_name(&mut self.env_vars, POSIX_FUNCTION_EXPORT_TOUCHED, name);
        }
    }

    pub(in crate::executor) fn execute_export_functions<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> io::Result<i32>
    where
        W: Write,
        E: Write,
    {
        let mut unset = false;
        let mut print = false;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                break;
            }
            for option in arg[1..].chars() {
                match option {
                    'f' => {}
                    'n' => unset = true,
                    'p' => print = true,
                    other => {
                        writeln!(
                            stderr,
                            "{}export: -{other}: invalid option",
                            self.diagnostic_prefix()
                        )?;
                        writeln!(
                            stderr,
                            "export: usage: export [-fn] [name[=value] ...] or export -p"
                        )?;
                        return Ok(2);
                    }
                }
            }
            index += 1;
        }

        if print && index >= args.len() {
            let mut names = marked_env_names(&self.env_vars, EXPORTED_FUNCTIONS);
            names.sort();
            for name in names {
                if let Some(body) = self.functions.get(&name) {
                    self.write_function_definition(&name, &body.commands, true, stdout)?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for name in &args[index..] {
            if !self.functions.contains_key(name) {
                writeln!(
                    stderr,
                    "{}export: {name}: not a function",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            }
            if !unset && !is_exportable_function_name(name) {
                writeln!(
                    stderr,
                    "{}export: {name}: cannot export",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            }
            if unset {
                unmark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
            } else {
                mark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
            }
        }

        Ok(status)
    }
}
