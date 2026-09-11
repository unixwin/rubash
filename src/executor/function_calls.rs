use super::*;

impl Executor {
    pub(in crate::executor) fn define_function(
        &mut self,
        cmd: &CommandNode,
        function: &FunctionCommand,
    ) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c): Bash stores a COMMAND tree plus source
        // metadata and function attributes. Keep the parsed body in a small
        // function table until the command representation is complete.
        // GNU execute_cmd.c::execute_intern_function -> general.c::
        // valid_function_word: a function name that contains `$', was quoted
        // or escaped, or is a `<(...)'/`>(...)' process-substitution-like
        // word is rejected with err_invalidid (non-fatal, rc=1, in default
        // mode). Under POSIX mode, special-builtin names are rejected as
        // "is a special builtin" and non-identifiers with err_invalidid;
        // both are fatal (EX_BADUSAGE=2, aborting the current subshell or
        // script via jump_to_top_level(ERREXIT)).
        let name_raw = function.name_metadata.raw.clone();
        let name_quoted = name_raw != function.name
            && (name_raw.contains('\'') || name_raw.contains('"') || name_raw.contains('\\'));
        let procsubst_like = name_raw.starts_with("<(") || name_raw.starts_with(">(");
        let invalid_identifier = function.name.contains('$') || name_quoted || procsubst_like;
        let posix_mode = self.posix_mode_enabled();
        let name_error_line = function.body_end_line.or(cmd.line);
        let name_error_prefix = |executor: &Self| {
            name_error_line.map_or_else(
                || executor.diagnostic_prefix(),
                |line| executor.diagnostic_prefix_for_line(line),
            )
        };
        if invalid_identifier {
            eprintln!(
                "{}`{}': not a valid identifier",
                name_error_prefix(self),
                name_raw
            );
            if posix_mode {
                self.exit_code = 2;
                return Err(ExecuteError::FatalFunctionError(2));
            }
            self.exit_code = 1;
            return Ok(());
        }
        if posix_mode && is_posix_special_builtin(&function.name) {
            eprintln!(
                "{}`{}': is a special builtin",
                name_error_prefix(self),
                function.name
            );
            self.exit_code = 2;
            return Err(ExecuteError::FatalFunctionError(2));
        }
        // GNU 5.3 builds with POSIX_RESTRICT_FUNCNAME undefined
        // (config-top.h:211), so posix mode does NOT reject non-identifier
        // function names (execute_cmd.c execute_intern_function only sets
        // pflags&1 under that ifdef). `!! () { fc -s "$@"; }` under
        // `set -o posix` defines the function (func.tests func5.sub).
        if marked_env_names(&self.env_vars, READONLY_FUNCTIONS)
            .iter()
            .any(|name| name == &function.name)
        {
            eprintln!(
                "{}{}: readonly function",
                self.diagnostic_prefix(),
                function.name
            );
            self.exit_code = 1;
            return Ok(());
        }
        self.functions.insert(
            function.name.clone(),
            Rc::new(Ast {
                commands: function.body.clone(),
            }),
        );
        if let Some(line) = cmd.line {
            self.function_definition_locations.insert(
                function.name.clone(),
                FunctionDefinitionLocation {
                    line,
                    source: self.current_bash_source(),
                    body_open_line: function.body_open_line,
                },
            );
        } else {
            self.function_definition_locations.remove(&function.name);
        }
        if command_has_input_or_output_redirects(cmd) {
            let mut redirects = CommandNode::new();
            redirects.redirect_in = cmd.redirect_in.clone();
            redirects.redirect_out = cmd.redirect_out.clone();
            redirects.append = cmd.append.clone();
            redirects.redirect_err = cmd.redirect_err.clone();
            redirects.redirect_err_append = cmd.redirect_err_append.clone();
            redirects.heredoc = cmd.heredoc.clone();
            redirects.here_string = cmd.here_string.clone();
            self.function_definition_redirects
                .insert(function.name.clone(), redirects);
        } else {
            self.function_definition_redirects.remove(&function.name);
        }
        // Print/roundtrip metadata: body kind plus the definition-level
        // redirect list (the generic `redirects` field collects `} >&2`
        // style trailing redirections at parse time).
        self.function_def_infos.insert(
            function.name.clone(),
            FunctionDefInfo {
                body_kind: Some(function.body_kind),
                def_redirects: crate::parser::ast_print::collected_redirects(cmd),
            },
        );
        self.exit_code = 0;
        Ok(())
    }

    pub(in crate::executor) fn function_name_for_command_word(&self, word: &str) -> Option<String> {
        if self.functions.contains_key(word) {
            return Some(word.to_string());
        }
        let unescaped = word.replace("\\=", "=");
        if unescaped != word && self.functions.contains_key(&unescaped) {
            Some(unescaped)
        } else {
            None
        }
    }

    pub(in crate::executor) fn execute_function(
        &mut self,
        name: &str,
        args: &[String],
        call_cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let Some(body) = self.functions.get(name).cloned() else {
            return Ok(());
        };
        // GNU Bash execute_cmd.c:5200 (execute_function) plus
        // variables.c:5944 (sv_funcnest): the nesting limit comes only from a
        // numeric $FUNCNEST > 0. An unset FUNCNEST (funcnest_max = 0), a zero
        // value, or a non-numeric value imposes no limit at all; there is no
        // built-in default cap (func4.sub recurses to completion when unset).
        // The chosen limit is reported in the diagnostic.
        let funcnest: Option<usize> = self
            .env_vars
            .get("FUNCNEST")
            .and_then(|value| value.trim().parse::<usize>().ok());
        let nesting_limit: Option<usize> = match funcnest {
            Some(limit) if limit > 0 => Some(limit),
            _ => None,
        };
        if let Some(nesting_limit) = nesting_limit {
            if self.function_depth >= nesting_limit {
                eprintln!(
                    "{}{}: maximum function nesting level exceeded ({})",
                    self.diagnostic_prefix(),
                    name,
                    nesting_limit
                );
                self.exit_code = 1;
                return Ok(());
            }
        }
        if self.execute_upstream_cprint_function(name) {
            return Ok(());
        }
        let definition_redirects = self.function_definition_redirects.get(name).cloned();
        let body_needs_redirects = definition_redirects
            .as_ref()
            .is_some_and(function_redirects_affect_body)
            || function_redirects_affect_body(call_cmd);
        let redirected_body = if body_needs_redirects {
            let mut commands = body.commands.clone();
            if let Some(definition_redirects) = &definition_redirects {
                self.apply_function_call_redirects(&mut commands, definition_redirects)?;
            }
            self.apply_function_call_redirects(&mut commands, call_cmd)?;
            Some(Ast { commands })
        } else {
            None
        };
        let body_ast = redirected_body.as_ref().unwrap_or_else(|| body.as_ref());
        let call_stdin = if let Some(definition_redirects) = &definition_redirects {
            match self.function_call_stdin(definition_redirects)? {
                Some(input) => Some(input),
                None => self.function_call_stdin(call_cmd)?,
            }
        } else {
            self.function_call_stdin(call_cmd)?
        };
        let (old_function, old_function_stdin, old_function_stdin_offset, old_positional_params) = {
            let old_function = self.env_vars.get("__RUBASH_CURRENT_FUNCTION").cloned();
            let old_function_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
            let old_function_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
            let old_positional_params = self.positional_params.clone();
            self.env_vars
                .insert("__RUBASH_CURRENT_FUNCTION".to_string(), name.to_string());
            if let Some(input) = call_stdin {
                self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
                self.env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            }
            self.function_name_stack.insert(0, name.to_string());
            let call_line = self
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .cloned()
                .or_else(|| call_cmd.line.map(|line| line.to_string()))
                .unwrap_or_else(|| "0".to_string());
            // GNU execute_function:5311-5317 pushes the call line onto
            // BASH_LINENO (array_push) instead of overwriting the bottom
            // frame: inside fn called at line N, BASH_LINENO=(N, "0"), so
            // ${BASH_LINENO[1]} is "0" ("main()'s file is the same as the first caller",
            // dbg-support.tests) and `caller` sees the full caller chain
            // (probe: BASH_LINENO[1]=[] vs GNU [0]).
            self.bash_lineno_stack.insert(0, call_line);
            let source = self.current_bash_source();
            self.bash_source_stack.insert(
                0,
                if source.is_empty() {
                    "environment".to_string()
                } else {
                    source
                },
            );
            self.bash_argc_stack.insert(0, args.len().to_string());
            for arg in args {
                self.bash_argv_stack.insert(0, arg.clone());
            }
            self.set_positional_params(args.to_vec());
            (
                old_function,
                old_function_stdin,
                old_function_stdin_offset,
                old_positional_params,
            )
        };
        self.local_var_scopes.push(HashMap::new());
        self.local_attr_scopes.push(HashMap::new());
        self.local_typed_scopes.push(HashMap::new());
        self.function_depth += 1;
        let old_debug_trap_function_line = self.debug_trap_function_line;
        if self.debug_trap_running {
            self.debug_trap_function_line = body.commands.first().and_then(|command| command.line);
        }
        // GNU execute_cmd.c:5351 sets line_number = function_line_number =
        // tc->line (the body-open line) at entry, and 5383-5387 runs the
        // DEBUG trap there ("so we can trap at the start of a function's
        // execution rather than the execution of the body's first command").
        // The fire only happens when the function inherits the DEBUG trap
        // (5270: trace attribute or functrace); otherwise
        // restore_default_signal(DEBUG_TRAP) removed it. run_debug_trap's own
        // in-progress guard keeps the DEBUG trap handler function itself from
        // firing (sigmodes[DEBUG_TRAP] & SIG_INPROGRESS).
        let functrace = crate::builtins::set::shell_option_enabled(&self.env_vars, "functrace");
        let function_traced = functrace || self.function_has_trace_attribute(name);
        // GNU execute_cmd.c:5269-5278: save the inherited DEBUG action and
        // remove it for the body unless the function inherits the trap; the
        // body may still set a new DEBUG trap, which then fires for the
        // remaining body commands (trap.tests: "func[29] funcdebug").
        let saved_debug_action = crate::builtins::trap::get_trap_action(&self.env_vars, "DEBUG");
        if saved_debug_action.is_some() && !function_traced {
            crate::builtins::trap::clear_debug_trap(&mut self.env_vars);
        }
        let definition_line = self
            .function_definition_locations
            .get(name)
            .map(|location| location.line);
        if function_traced {
            let body_open_line = call_cmd
                .function_command
                .as_ref()
                .and_then(|function| function.body_open_line)
                .or(self
                    .function_definition_locations
                    .get(name)
                    .and_then(|location| location.body_open_line))
                .or(definition_line);
            if let Some(line) = body_open_line {
                self.env_vars
                    .insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
            }
            let command_text = crate::executor::command_text::bash_command_source_text(call_cmd);
            self.run_debug_trap(&command_text)?;
        }
        let result = self.execute_ast_inner(body_ast);
        // GNU execute_cmd.c uw_maybe_set_debug_trap: at function exit the
        // saved DEBUG action is restored only when the body did not set a
        // new one, so a trap set inside the function persists after return
        // (trap.tests listing shows the funcdebug action after func).
        if let Some(action) = saved_debug_action {
            if !function_traced {
                crate::builtins::trap::maybe_restore_debug_trap(&mut self.env_vars, action);
            }
        }
        self.debug_trap_function_line = old_debug_trap_function_line;
        // GNU restores line_number to the function definition line when the
        // body group finishes (the group's execute_command_internal unwinds
        // line_number to the value set at 5351), so the RETURN trap action
        // and the DEBUG fire for its command see the definition line
        // (dbg-support.tests: "debug lineno: 30 fn1" and
        // "return lineno: 30 fn1" at fn1's exit).
        if result.is_ok() {
            if let Some(line) = definition_line {
                self.env_vars
                    .insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
            }
        }
        self.run_function_return_trap()?;
        {
            self.function_depth -= 1;
            self.restore_function_locals();
            self.set_positional_params(old_positional_params);
            if !self.function_name_stack.is_empty() {
                self.function_name_stack.remove(0);
            }
            if !self.bash_lineno_stack.is_empty() {
                self.bash_lineno_stack.remove(0);
            }
            if !self.bash_source_stack.is_empty() {
                self.bash_source_stack.remove(0);
            }
            if !self.bash_argc_stack.is_empty() {
                self.bash_argc_stack.remove(0);
            }
            for _ in args {
                if !self.bash_argv_stack.is_empty() {
                    self.bash_argv_stack.remove(0);
                }
            }
            restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_function_stdin);
            restore_optional_env_var(
                &mut self.env_vars,
                FUNCTION_STDIN_OFFSET,
                old_function_stdin_offset,
            );
            match old_function {
                Some(value) => {
                    self.env_vars
                        .insert("__RUBASH_CURRENT_FUNCTION".to_string(), value);
                }
                None => {
                    self.env_vars.remove("__RUBASH_CURRENT_FUNCTION");
                }
            }
        }
        match result {
            // GNU Bash 5.2 (probes f3/f4, 2026-08-24): a word-expansion
            // failure ends only this function invocation; the caller sees
            // the failure status and keeps its own remaining list running.
            Err(ExecuteError::ExpansionFailure(status))
                if !self.inside_compound_condition.get() =>
            {
                self.exit_code = status;
                Ok(())
            }
            Err(ExecuteError::Return(status)) => {
                self.exit_code = status;
                Ok(())
            }
            other => other,
        }
    }

    pub(in crate::executor) fn apply_function_call_redirects(
        &self,
        body: &mut [CommandNode],
        call_cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &call_cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            self.create_redirect_output(&target, redirect.clobber)?;
            let append_redirect = Redirect {
                operator: ">>".to_string(),
                operator_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    ">>".to_string(),
                    ">>".to_string(),
                )),
                kind: crate::parser::RedirectKind::Append,
                append: true,
                ..redirect.clone()
            };
            for command in body.iter_mut() {
                if command.redirect_out.is_none() && command.append.is_none() {
                    command.append = Some(append_redirect.clone());
                }
            }
        } else if let Some(redirect) = &call_cmd.append {
            for command in body.iter_mut() {
                if command.redirect_out.is_none() && command.append.is_none() {
                    command.append = Some(redirect.clone());
                }
            }
        }

        if let Some(redirect) = &call_cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                operator: "2>>".to_string(),
                operator_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    "2>>".to_string(),
                    "2>>".to_string(),
                )),
                kind: crate::parser::RedirectKind::Append,
                append: true,
                ..redirect.clone()
            };
            for command in body.iter_mut() {
                if command.redirect_err.is_none() && command.redirect_err_append.is_none() {
                    command.redirect_err_append = Some(append_redirect.clone());
                }
            }
        } else if let Some(redirect) = &call_cmd.redirect_err_append {
            for command in body.iter_mut() {
                if command.redirect_err.is_none() && command.redirect_err_append.is_none() {
                    command.redirect_err_append = Some(redirect.clone());
                }
            }
        }

        Ok(())
    }

    pub(in crate::executor) fn function_call_stdin(
        &mut self,
        call_cmd: &CommandNode,
    ) -> Result<Option<String>, ExecuteError> {
        if let Some(input) = self.stdin_string_for_command_mut(call_cmd) {
            return Ok(Some(input));
        }

        let Some(redirect) = &call_cmd.redirect_in else {
            // A shell script read through `< file` still has the unread
            // portion of that virtual stdin available to commands it invokes.
            // The nested shell must consume it instead of the host process
            // stdin (for example, input-line.sh/input-line.sub).
            if let Some(input) = self.env_vars.get(FUNCTION_STDIN) {
                let offset = self
                    .env_vars
                    .get(FUNCTION_STDIN_OFFSET)
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                return Ok(Some(input.get(offset..).unwrap_or_default().to_string()));
            }
            return Ok(self.virtual_fd_stdin_remaining(0));
        };
        if redirect.fd.unwrap_or(0) != 0 {
            return Ok(None);
        }
        let target = self.expand_word(&redirect.target);
        if is_closed_redirect_target(&target) {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(shell_path_to_windows(
            &target,
            &self.env_vars,
        ))?))
    }
}

fn function_redirects_affect_body(command: &CommandNode) -> bool {
    command.redirect_out.is_some()
        || command.append.is_some()
        || command.redirect_err.is_some()
        || command.redirect_err_append.is_some()
}
