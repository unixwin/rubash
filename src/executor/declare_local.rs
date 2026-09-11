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
        // GNU declare.def declares -t on functions: set/clear the trace
        // attribute (trace_p(var) in execute_cmd.c). A traced function
        // inherits the DEBUG and RETURN traps even with functrace off.
        let set_trace = args
            .iter()
            .any(|arg| arg.starts_with('-') && arg.contains('t') && arg.contains('f'));
        let clear_trace = args
            .iter()
            .any(|arg| arg.starts_with('+') && arg.contains('t') && arg.contains('f'));
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
                    self.write_function_definition(name, &body.commands, exported_only, stdout)?;
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
            if set_trace {
                mark_env_name(&mut self.env_vars, FUNC_TRACE_FUNCTIONS, name);
                if !print {
                    continue;
                }
            }
            if clear_trace {
                unmark_env_name(&mut self.env_vars, FUNC_TRACE_FUNCTIONS, name);
                if !print {
                    continue;
                }
            }
            if function_names_only {
                if exported_only {
                    writeln!(stdout, "declare -fx {name}")?;
                } else {
                    self.write_function_name(name, stdout)?;
                }
            } else {
                self.write_function_definition(
                    name,
                    &body.commands,
                    exported_only && is_exported,
                    stdout,
                )?;
            }
        }
        Ok(status)
    }

    fn write_function_name<W>(&self, name: &str, stdout: &mut W) -> io::Result<()>
    where
        W: Write,
    {
        if crate::builtins::shopt::option_enabled(&self.env_vars, "extdebug") {
            if let Some(location) = self.function_definition_locations.get(name) {
                return writeln!(stdout, "{} {} {}", name, location.line, location.source);
            }
        }
        writeln!(stdout, "{name}")
    }

    pub(in crate::executor) fn execute_declare(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        self.sync_dynamic_assoc_vars();
        // GNU materializes the DIRSTACK cell only when the command names
        // DIRSTACK itself; a bare list-all `declare -a` must keep the cell
        // empty (variables.c get_dirstack runs on named access only).
        if declare_words_name_dirstack(&cmd.words[1..]) {
            self.sync_dirstack_cell();
        }
        let mut args = self.expand_declare_assignment_args(&cmd.words[1..]);
        if declare_args_request_integer(&args) {
            args = self.evaluate_declare_integer_assignment_args(&args);
        }
        if self.function_depth > 0
            && !declare_args_force_global(&args)
            && !declare_args_request_print(&args)
        {
            let prefix_assignment_names = cmd
                .assignment_keys()
                .map(|name| assignment_name_and_append(name).0.to_string())
                .collect::<Vec<_>>();
            let pre_existing: Vec<String> = self
                .local_var_scopes
                .last()
                .map(|scope| scope.keys().cloned().collect())
                .unwrap_or_default();
            self.save_local_names(&args);
            if !local_args_request_inherit(&args) {
                self.initialize_non_inherited_locals(
                    &args,
                    &prefix_assignment_names,
                    &pre_existing,
                );
            }
        }
        let global_local_values = self.begin_global_declare_for_local_names(&args);
        let posix_function_export_unsets = self.posix_function_declare_unset_export_names(&args);
        let command_name = cmd.words.first().map(String::as_str).unwrap_or("declare");
        // GNU builtins/declare.def:704-806: a declare/typeset assignment whose
        // name is a nameref (and which does not itself carry -n) writes the
        // referenced variable, not the nameref. Capture the resolved targets
        // before the builtin runs so the typed owner can mirror them below.
        let (nameref_flag, _unset_nameref_flag) =
            crate::builtins::declare::declare_nameref_flags(&args);
        let nameref_assign_targets = if nameref_flag {
            // -n keeps the assignment on the nameref cell itself; only +n and
            // plain declarations follow the chain to the referenced variable.
            Vec::new()
        } else {
            crate::builtins::declare::nameref_assignment_targets(&args, &self.env_vars)
        };

        let result = (|| -> Result<i32, ExecuteError> {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status = crate::builtins::declare::execute_with_io_named_in_context(
                command_name,
                &args,
                &mut self.env_vars,
                &mut stdout,
                &mut stderr,
                self.function_depth > 0,
            )?;
            let stderr = if self.stdout_capture.is_some()
                && declare_args_request_print(&args)
                && !args.iter().any(|arg| {
                    (arg.starts_with('-') || arg.starts_with('+'))
                        && (arg.contains('f') || arg.contains('F'))
                })
                && status != 0
            {
                Vec::new()
            } else {
                stderr
            };
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            Ok(status)
        })();
        if result.as_ref().is_ok_and(|status| *status == 0) {
            // Mirror the through-the-nameref assignments into the typed owner:
            // parameter expansion reads shell_state.variables first, so a
            // stale typed target would shadow the new value written to
            // env_vars (probe: typeset +n foo=other then echo $bar).
            for (_, target) in &nameref_assign_targets {
                if let Some(value) = self.env_vars.get(target).cloned() {
                    match self.shell_state.variables.get_mut(target) {
                        Some(variable) => {
                            variable.value = crate::shell::ShellValue::Scalar(value);
                        }
                        None => {
                            let _ = self.shell_state.variables.set_scalar(target, value);
                        }
                    }
                }
            }
            crate::builtins::declare::sync_typed_assignments(
                &args,
                &self.env_vars,
                &mut self.shell_state.variables,
            );
            crate::builtins::declare::sync_typed_attributes(
                &args,
                &self.env_vars,
                &mut self.shell_state.variables,
            );
            self.apply_posix_function_declare_unset_export(posix_function_export_unsets);
            if crate::builtins::set::shell_option_enabled(&self.env_vars, "allexport") {
                // set -a (allexport): a typeset/declare assignment exports the
                // variable (variables.c do_export / set -a semantics), even
                // when the declare invocation itself carries no -x flag.
                for arg in &args {
                    if arg.contains('=') && !(arg.starts_with('-') || arg.starts_with('+')) {
                        let (raw_name, _) = arg.split_once('=').unwrap_or((arg, ""));
                        let (base, _) = assignment_name_and_append(raw_name);
                        self.mark_exported(base);
                    }
                }
            }
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
            // GNU local / local -p with no name arguments prints ONLY the
            // variables declared local to the current function frame (sorted,
            // declare -- form) -- never the whole variable table. Rewrite
            // the args so the shared declare printer renders just those names.
            if local_names(&args).is_empty() {
                let local_names: Vec<String> = self
                    .local_var_scopes
                    .last()
                    .map(|scope| {
                        scope
                            .keys()
                            .filter(|name| !name.starts_with("__RUBASH_"))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                let local_names = {
                    let mut names = local_names;
                    names.sort();
                    names
                };
                if local_names.is_empty() {
                    // No locals declared in this frame: GNU prints nothing.
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(0);
                }
                args.clear();
                args.push("-p".to_string());
                args.extend(local_names);
            }
            if !declare_args_request_print(&args) {
                let prefix_assignment_names = cmd
                    .assignment_keys()
                    .map(|name| assignment_name_and_append(name).0.to_string())
                    .collect::<Vec<_>>();
                let pre_existing: Vec<String> = self
                    .local_var_scopes
                    .last()
                    .map(|scope| scope.keys().cloned().collect())
                    .unwrap_or_default();
                self.save_local_names(&args);
                if !local_args_request_inherit(&args) {
                    self.initialize_non_inherited_locals(
                        &args,
                        &prefix_assignment_names,
                        &pre_existing,
                    );
                }
            }
            self.write_local_compound_readonly_assignment_errors(&args, &mut stderr)?;
            // GNU declare.def:565 uses variable_context to decide between the
            // global-scope self-reference error and the function-scope
            // circular-reference warning; local always runs in a function, so
            // `local -n a=$1` with a=$1 warns and continues.
            let status = crate::builtins::declare::execute_with_io_named_in_context(
                "local",
                &args,
                &mut self.env_vars,
                &mut stdout,
                &mut stderr,
                true,
            )?;
            if status == 0 {
                // Plain scalar locals must shadow the outer value in the typed
                // owner as well: parameter expansion reads shell_state.variables
                // first, so `local OPTERR=1` inside a function has to replace the
                // stale global there (getopts5.sub: getop must print OPTERR=1,
                // and the frame restore in restore_function_locals puts the
                // saved outer value back).
                for arg in &args {
                    let Some((raw_name, _)) = arg.split_once('=') else {
                        continue;
                    };
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
            status
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

    pub(in crate::executor) fn initialize_non_inherited_locals(
        &mut self,
        args: &[String],
        preserve_names: &[String],
        pre_existing: &[String],
    ) {
        if crate::builtins::shopt::option_enabled(&self.env_vars, "localvar_inherit") {
            return;
        }
        for name in local_names(args) {
            if preserve_names.iter().any(|preserve| preserve == &name) {
                continue;
            }
            // GNU builtins/declare.def:659-668: re-declaring a variable that
            // is already local at the SAME variable context keeps it (var =
            // refvar), so a valueless re-declaration of an existing
            // same-frame nameref preserves its cell (nameref12/nameref13.sub)
            // instead of resetting a fresh empty local. pre_existing lists
            // the frame snapshot BEFORE this command saved its own names, so
            // a first-time `declare -a a` still resets the fresh local
            // (assoc.tests: f: declare -a a prints an empty local).
            if pre_existing.iter().any(|existing| existing == &name) {
                continue;
            }
            if is_marked_var(&self.env_vars, EXPORTED_VARS, &name) {
                if let Some(value) = self.env_vars.get(&name).cloned() {
                    set_local_export_env_value(&mut self.env_vars, &name, value);
                }
            }
            self.env_vars.remove(&name);
            // A fresh local shadows the outer variable in the typed owner too:
            // parameter expansion reads shell_state.variables first, so a
            // stale global scalar would keep leaking through (bash: `local X`
            // makes ${X-unset} report unset until the frame returns).
            self.shell_state.variables.remove(&name);
            set_var_attrs(&mut self.env_vars, &name, VarAttrs::default());
        }
    }
}

/// True when a declare/typeset/local command line explicitly names DIRSTACK
/// (as an operand or assignment target). GNU variables.c get_dirstack runs
/// the dynamic getter -- materializing the stored array cell -- only on
/// named access; a bare list-all `declare -a` shows the last materialized
/// cell, which stays empty when DIRSTACK was never named.
fn declare_words_name_dirstack(words: &[String]) -> bool {
    let mut options_ended = false;
    for word in words {
        if !options_ended {
            if word == "--" {
                options_ended = true;
                continue;
            }
            if word.starts_with('-') || word.starts_with('+') {
                continue;
            }
        }
        let base = word.split('=').next().unwrap_or(word);
        let base = base.split('[').next().unwrap_or(base);
        if base == "DIRSTACK" {
            return true;
        }
    }
    false
}
