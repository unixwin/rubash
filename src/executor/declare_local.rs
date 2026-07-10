use super::*;

impl Executor {
    pub(in crate::executor) fn execute_declare_functions(
        &mut self,
        args: &[String],
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> io::Result<i32> {
        // TODO(builtins/declare.def/execute_cmd.c): Bash prints the stored
        // function COMMAND tree. Rubash currently stores only parsed command
        // bodies, so render the simple function form used by builtins6.sub.
        let names: Vec<&str> = args
            .iter()
            .filter(|arg| !arg.starts_with('-') && !arg.starts_with('+'))
            .map(String::as_str)
            .collect();
        let print_not_found = args.iter().any(|arg| arg == "-p");
        let function_names_only = args
            .iter()
            .any(|arg| arg.starts_with('-') && arg.contains('F'));
        let function_definition_mode = args
            .iter()
            .any(|arg| (arg.starts_with('-') || arg.starts_with('+')) && arg.contains('f'));
        let set_export = args
            .iter()
            .any(|arg| arg.starts_with('-') && arg.contains('x'));
        let clear_export = args
            .iter()
            .any(|arg| arg.starts_with('+') && arg.contains('x'));
        let set_export_attribute = set_export && function_definition_mode;
        let clear_export_attribute = clear_export && function_definition_mode;
        let exported_only = set_export;
        let readonly = args
            .iter()
            .any(|arg| arg.starts_with('-') && arg.contains('r'));
        let print = args
            .iter()
            .any(|arg| arg.starts_with('-') && arg.contains('p'));
        let exported_functions = marked_env_names(&self.env_vars, EXPORTED_FUNCTIONS);
        if names.is_empty() {
            let mut functions: Vec<_> = self.functions.iter().collect();
            functions.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (name, body) in functions {
                if exported_only && !exported_functions.iter().any(|exported| *exported == *name) {
                    continue;
                }
                if function_names_only {
                    if exported_only {
                        writeln!(stdout, "declare -fx {name}")?;
                    } else {
                        writeln!(stdout, "declare -f {name}")?;
                    }
                } else {
                    self.write_function_definition(name, body, exported_only, stdout)?;
                }
            }
            return Ok(0);
        }
        let mut status = 0;
        for name in names {
            let Some(body) = self.functions.get(name) else {
                if print_not_found {
                    writeln!(
                        stderr,
                        "{}declare: {name}: not found",
                        self.diagnostic_prefix()
                    )?;
                }
                status = 1;
                continue;
            };
            let is_exported = exported_functions.iter().any(|exported| exported == name);
            if exported_only && !is_exported && !set_export_attribute {
                continue;
            }
            if clear_export_attribute {
                unmark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
                if !print {
                    continue;
                }
            } else if set_export_attribute {
                mark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
                if !print && !function_names_only {
                    continue;
                }
            }
            if readonly {
                mark_env_name(&mut self.env_vars, READONLY_FUNCTIONS, name);
                if !print {
                    continue;
                }
            }
            if function_names_only {
                if exported_only {
                    writeln!(stdout, "declare -fx {name}")?;
                } else {
                    writeln!(stdout, "{name}")?;
                }
            } else {
                self.write_function_definition(name, body, exported_only && is_exported, stdout)?;
            }
        }
        Ok(status)
    }

    pub(in crate::executor) fn execute_declare(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        self.sync_dynamic_assoc_vars();
        let mut args = self.expand_declare_assignment_args(&cmd.words[1..]);
        if declare_args_request_integer(&args) {
            args = self.evaluate_declare_integer_assignment_args(&args);
        }
        if self.function_depth > 0
            && !declare_args_force_global(&args)
            && !declare_args_request_print(&args)
        {
            self.save_local_names(&args);
        }
        let global_local_values = self.begin_global_declare_for_local_names(&args);
        let posix_function_export_unsets = self.posix_function_declare_unset_export_names(&args);

        let result = (|| -> Result<i32, ExecuteError> {
            if let Some(redirect) = &cmd.redirect_out {
                let target = self.expand_word(&redirect.target);
                let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
                return Ok(crate::builtins::declare::execute_with_io(
                    &args,
                    &mut self.env_vars,
                    &mut file,
                    &mut std::io::stderr().lock(),
                )?);
            }

            if let Some(redirect) = &cmd.append {
                let target = self.expand_word(&redirect.target);
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                return Ok(crate::builtins::declare::execute_with_io(
                    &args,
                    &mut self.env_vars,
                    &mut file,
                    &mut std::io::stderr().lock(),
                )?);
            }

            if let Some(redirect) = &cmd.redirect_err {
                let target = self.expand_word(&redirect.target);
                if is_null_device(&target) {
                    return Ok(crate::builtins::declare::execute_with_io(
                        &args,
                        &mut self.env_vars,
                        &mut std::io::stdout().lock(),
                        &mut std::io::sink(),
                    )?);
                }
                let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
                return Ok(crate::builtins::declare::execute_with_io(
                    &args,
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut file,
                )?);
            }

            if let Some(redirect) = &cmd.redirect_err_append {
                let target = self.expand_word(&redirect.target);
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                return Ok(crate::builtins::declare::execute_with_io(
                    &args,
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut file,
                )?);
            }

            Ok(crate::builtins::declare::execute(
                &args,
                &mut self.env_vars,
            )?)
        })();
        if result.as_ref().is_ok_and(|status| *status == 0) {
            self.apply_posix_function_declare_unset_export(posix_function_export_unsets);
        }
        self.finish_global_declare_for_local_names(global_local_values);
        result
    }

    pub(in crate::executor) fn execute_declare_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if cmd.words[1..].iter().any(|word| {
            (word.starts_with('-') || word.starts_with('+'))
                && (word.contains('f') || word.contains('F'))
        }) {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            self.exit_code =
                self.execute_declare_functions(&cmd.words[1..], &mut stdout, &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(());
        }
        self.exit_code = self.execute_declare(cmd)?;
        Ok(())
    }

    pub(in crate::executor) fn execute_local(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = if self.function_depth == 0 {
            writeln!(
                stderr,
                "{}local: can only be used in a function",
                self.diagnostic_prefix()
            )?;
            1
        } else if let Err(option) = validate_local_options(&cmd.words[1..]) {
            writeln!(
                stderr,
                "{}local: -{option}: invalid option",
                self.diagnostic_prefix()
            )?;
            writeln!(stderr, "local: usage: local [option] name[=value] ...")?;
            2
        } else {
            let mut args = self.expand_declare_assignment_args(&cmd.words[1..]);
            if declare_args_request_integer(&args) {
                args = self.evaluate_declare_integer_assignment_args(&args);
            }
            if !declare_args_request_print(&args) {
                self.save_local_names(&args);
                self.initialize_non_inherited_locals(&args);
            }
            self.write_local_compound_readonly_assignment_errors(&args, &mut stderr)?;
            crate::builtins::declare::execute_with_io(
                &args,
                &mut self.env_vars,
                &mut stdout,
                &mut stderr,
            )?
        };
        let stderr = local_stderr_from_declare(stderr);
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn write_local_compound_readonly_assignment_errors<W>(
        &self,
        args: &[String],
        stderr: &mut W,
    ) -> io::Result<()>
    where
        W: Write,
    {
        for arg in args {
            let Some((name, value)) = split_assignment_word(arg) else {
                continue;
            };
            if !value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
                continue;
            }
            let (name, _) = assignment_name_and_append(name);
            if is_marked_var(&self.env_vars, READONLY_VARS, name) {
                writeln!(
                    stderr,
                    "{}{}: readonly variable",
                    self.diagnostic_prefix(),
                    name
                )?;
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn initialize_non_inherited_locals(&mut self, args: &[String]) {
        if crate::builtins::shopt::option_enabled(&self.env_vars, "localvar_inherit") {
            return;
        }
        for name in local_names_without_assignment(args) {
            if is_marked_var(&self.env_vars, EXPORTED_VARS, &name) {
                if let Some(value) = self.env_vars.get(&name).cloned() {
                    set_local_export_env_value(&mut self.env_vars, &name, value);
                }
            }
            self.env_vars.remove(&name);
            set_var_attrs(&mut self.env_vars, &name, VarAttrs::default());
        }
    }
}
