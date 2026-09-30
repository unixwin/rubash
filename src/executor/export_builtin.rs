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
        // GNU export.def shares declare.def's operand handling: an
        // `export name[sub]=value` subscript resolves under the same
        // ExpandedOnce (W_ASSIGNMENT -> ASS_NOEXPAND) rules.
        let args = match self.rewrite_declare_operand_subscripts(
            &cmd.words[1..],
            &cmd.word_metadata,
            "export",
        ) {
            Ok(args) => args,
            Err(()) => return Ok(1),
        };
        // Same GNU variables.c:2920-2937 make_variable_value rule as the
        // readonly path: an operand whose target is integer-attributed
        // evaluates the RHS arithmetic (`export i=3+4` binds 7). An evalexp
        // failure follows variables.c:2938-2944: print `export: <evalerror>`,
        // drop the failing operand and its successors, arm the DISCARD
        // abort, and fail the command.
        let (args, arith_eval_failure) = self.evaluate_integer_attribute_assignment_args(&args);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(message) = &arith_eval_failure {
            if !message.is_empty() {
                writeln!(stderr, "{}export: {message}", self.diagnostic_prefix())?;
            }
            self.raise_evalerror_abort();
        }
        let status = crate::builtins::setattr::export_with_io(
            args.iter().map(String::as_str),
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        let status = if arith_eval_failure.is_some() {
            status.max(1)
        } else {
            status
        };
        if status == 0 {
            self.sync_setattr_typed_assignments(cmd.words[1..].iter().map(String::as_str));
            // Check for locale environment changes (LC_ALL, LC_CTYPE, LANG)
            for word in &cmd.words[1..] {
                if word.starts_with("LC_ALL=")
                    || word.starts_with("LC_CTYPE=")
                    || word.starts_with("LANG=")
                {
                    crate::locale::check_setlocale_warning();
                    // GNU variables.c sv_lang/sv_lc* call setlocale on the
                    // assignment; sync the process env so the dynamic
                    // locale readers (locale::is_utf8 - the dollar-quote
                    // backslash-u locale gate, rubash#353) see the change.
                    crate::locale::sync_process_locale(&self.shell_state.env_vars);
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
            if is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, base)
                || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base)
                || is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base)
            {
                continue;
            }
            match self.shell_state.env_vars.get(base) {
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
            let mut names = marked_env_names(&self.shell_state.env_vars, EXPORTED_FUNCTIONS);
            names.sort();
            for name in names {
                if let Some(body) = self.shell_state.functions.get(&name) {
                    self.write_function_definition(&name, &body.commands, true, stdout)?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for name in &args[index..] {
            if !self.shell_state.functions.contains_key(name) {
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
                unmark_env_name(&mut self.shell_state.env_vars, EXPORTED_FUNCTIONS, name);
            } else {
                mark_env_name(&mut self.shell_state.env_vars, EXPORTED_FUNCTIONS, name);
            }
        }

        Ok(status)
    }
}
