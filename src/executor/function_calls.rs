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
        if marked_env_names(&self.shell_state.env_vars, READONLY_FUNCTIONS)
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
        self.shell_state.functions.insert(
            function.name.clone(),
            Rc::new(Ast {
                commands: function.body.clone(),
            }),
        );
        if let Some(line) = cmd.line {
            self.shell_state.function_definition_locations.insert(
                function.name.clone(),
                FunctionDefinitionLocation {
                    line,
                    source: self.current_bash_source(),
                    body_open_line: function.body_open_line,
                },
            );
        } else {
            self.shell_state
                .function_definition_locations
                .remove(&function.name);
        }
        if command_has_input_or_output_redirects(cmd) {
            let mut redirects = CommandNode::new();
            redirects.redirect_in = cmd.redirect_in.clone();
            redirects.redirect_out = cmd.redirect_out.clone();
            redirects.append = cmd.append.clone();
            redirects.redirect_err = cmd.redirect_err.clone();
            redirects.redirect_err_append = cmd.redirect_err_append.clone();
            redirects.heredoc = cmd.heredoc.clone();
            redirects.heredoc_body = cmd.heredoc_body.clone();
            redirects.here_string = cmd.here_string.clone();
            redirects.here_string_carrier = cmd.here_string_carrier.clone();
            self.shell_state
                .function_definition_redirects
                .insert(function.name.clone(), redirects);
        } else {
            self.shell_state
                .function_definition_redirects
                .remove(&function.name);
        }
        // Print/roundtrip metadata: body kind plus the definition-level
        // redirect list (the generic `redirects` field collects `} >&2`
        // style trailing redirections at parse time).
        self.shell_state.function_def_infos.insert(
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
        if self.shell_state.functions.contains_key(word) {
            return Some(word.to_string());
        }
        let unescaped = word.replace("\\=", "=");
        if unescaped != word && self.shell_state.functions.contains_key(&unescaped) {
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
        let Some(body) = self.shell_state.functions.get(name).cloned() else {
            return Ok(());
        };
        // GNU Bash execute_cmd.c:5200 (execute_function) plus
        // variables.c:5944 (sv_funcnest): the nesting limit comes only from a
        // numeric $FUNCNEST > 0. An unset FUNCNEST (funcnest_max = 0), a zero
        // value, or a non-numeric value imposes no limit at all; there is no
        // built-in default cap (func4.sub recurses to completion when unset).
        // The chosen limit is reported in the diagnostic.
        let funcnest: Option<usize> = self
            .shell_state
            .env_vars
            .get("FUNCNEST")
            .and_then(|value| value.trim().parse::<usize>().ok());
        let nesting_limit: Option<usize> = match funcnest {
            Some(limit) if limit > 0 => Some(limit),
            _ => None,
        };
        if let Some(nesting_limit) = nesting_limit {
            if self.shell_state.function_depth >= nesting_limit {
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
        let definition_redirects = self
            .shell_state
            .function_definition_redirects
            .get(name)
            .cloned();
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
        // GNU shares fd 0 between caller and function: capture the parent's
        // FUNCTION_STDIN cursor before function_call_stdin carves the
        // remainder so the child's consumed prefix can fold back onto it.
        let parent_stdin_base = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut fd0_bindings: Vec<FunctionCallStdinBinding> = Vec::new();
        let (call_stdin, stdin_carved_from_parent) =
            if let Some(definition_redirects) = &definition_redirects {
                match self.function_call_stdin(definition_redirects)? {
                    (Some(input), carved, binding) => {
                        if let Some(binding) = binding {
                            fd0_bindings.push(binding);
                        }
                        (Some(input), carved)
                    }
                    (None, _, binding) => {
                        if let Some(binding) = binding {
                            fd0_bindings.push(binding);
                        }
                        let (input, carved, binding) = self.function_call_stdin(call_cmd)?;
                        if let Some(binding) = binding {
                            fd0_bindings.push(binding);
                        }
                        (input, carved)
                    }
                }
            } else {
                let (input, carved, binding) = self.function_call_stdin(call_cmd)?;
                if let Some(binding) = binding {
                    fd0_bindings.push(binding);
                }
                (input, carved)
            };
        // GNU redir.c do_redirections applies the call's redirects last;
        // when materialized FUNCTION_STDIN text serves the body, a device
        // bind installed by an earlier redirect would shadow it — undo.
        if call_stdin.is_some() {
            while let Some(binding) = fd0_bindings.pop() {
                self.restore_function_call_fd0(binding);
            }
        }
        let (old_function, old_function_stdin, old_function_stdin_offset, old_positional_params) = {
            let old_function = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_FUNCTION")
                .cloned();
            let old_function_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
            let old_function_stdin_offset = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .cloned();
            let old_positional_params = self.shell_state.positional_params.clone();
            self.shell_state
                .env_vars
                .insert("__RUBASH_CURRENT_FUNCTION".to_string(), name.to_string());
            if let Some(input) = call_stdin {
                self.shell_state
                    .env_vars
                    .insert(FUNCTION_STDIN.to_string(), input);
                self.shell_state
                    .env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            }
            self.shell_state
                .function_name_stack
                .insert(0, name.to_string());
            let call_line = self
                .shell_state
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
            self.shell_state.bash_lineno_stack.insert(0, call_line);
            let source = self.current_bash_source();
            self.shell_state.bash_source_stack.insert(
                0,
                if source.is_empty() {
                    "environment".to_string()
                } else {
                    source
                },
            );
            self.shell_state
                .bash_argc_stack
                .insert(0, args.len().to_string());
            for arg in args {
                self.shell_state.bash_argv_stack.insert(0, arg.clone());
            }
            self.set_positional_params(args.to_vec());
            (
                old_function,
                old_function_stdin,
                old_function_stdin_offset,
                old_positional_params,
            )
        };
        self.shell_state.local_var_scopes.push(HashMap::new());
        self.shell_state.local_attr_scopes.push(HashMap::new());
        self.shell_state.local_typed_scopes.push(HashMap::new());
        self.shell_state.function_depth += 1;
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
        let functrace =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "functrace");
        let function_traced = functrace || self.function_has_trace_attribute(name);
        // GNU execute_cmd.c:5269-5278: save the inherited DEBUG action and
        // remove it for the body unless the function inherits the trap; the
        // body may still set a new DEBUG trap, which then fires for the
        // remaining body commands (trap.tests: "func[29] funcdebug").
        let saved_debug_action =
            crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG");
        if saved_debug_action.is_some() && !function_traced {
            crate::builtins::trap::clear_debug_trap(&mut self.shell_state.env_vars);
        }
        let definition_line = self
            .shell_state
            .function_definition_locations
            .get(name)
            .map(|location| location.line);
        if function_traced {
            let body_open_line = call_cmd
                .function_command
                .as_ref()
                .and_then(|function| function.body_open_line)
                .or(self
                    .shell_state
                    .function_definition_locations
                    .get(name)
                    .and_then(|location| location.body_open_line))
                .or(definition_line);
            if let Some(line) = body_open_line {
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
            }
            // GNU's the_printed_command at the entry fire (execute_cmd.c:5387)
            // is still the call's simple-command text built from the PARSED
            // words — `f x "y z"` keeps its source quoting. call_cmd here has
            // already been through word expansion (expand_command_words drops
            // word_metadata), which loses that quoting. __RUBASH_LAST_COMMAND
            // is recorded by set_current_command from the pre-expansion node,
            // so it carries the same raw source text GNU prints.
            let command_text = self
                .shell_state
                .env_vars
                .get("__RUBASH_LAST_COMMAND")
                .cloned()
                .filter(|text| !text.is_empty() && !call_cmd.words.is_empty())
                .unwrap_or_else(|| {
                    crate::executor::command_text::bash_command_source_text(call_cmd)
                });
            self.run_debug_trap(&command_text)?;
        }
        // GNU execute_function (execute_cmd.c:5269+) applies the call's
        // redirections to real descriptors for the body's duration
        // (redir.c do_redirections, undone on return), so `f 3>&1` makes a
        // body's `1>&3` resolve fd 3 to the call's binding. The body's
        // ambient line_number is the function DEFINITION line
        // (execute_cmd.c:5351 line_number = function_line_number = tc->line).
        let result = self.with_compound_output_redirects(call_cmd, |executor| {
            executor.with_ambient_line(definition_line, |executor| {
                executor.execute_ast_inner(body_ast)
            })
        });
        // GNU execute_cmd.c:5269+ — the call's input redirections are
        // undone when the function returns.
        while let Some(binding) = fd0_bindings.pop() {
            self.restore_function_call_fd0(binding);
        }
        // GNU execute_cmd.c uw_maybe_set_debug_trap: at function exit the
        // saved DEBUG action is restored only when the body did not set a
        // new one, so a trap set inside the function persists after return
        // (trap.tests listing shows the funcdebug action after func).
        if let Some(action) = saved_debug_action {
            if !function_traced {
                crate::builtins::trap::maybe_restore_debug_trap(
                    &mut self.shell_state.env_vars,
                    action,
                );
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
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
            }
        }
        self.run_function_return_trap()?;
        {
            self.shell_state.function_depth -= 1;
            self.restore_function_locals();
            self.set_positional_params(old_positional_params);
            if !self.shell_state.function_name_stack.is_empty() {
                self.shell_state.function_name_stack.remove(0);
            }
            if !self.shell_state.bash_lineno_stack.is_empty() {
                self.shell_state.bash_lineno_stack.remove(0);
            }
            if !self.shell_state.bash_source_stack.is_empty() {
                self.shell_state.bash_source_stack.remove(0);
            }
            if !self.shell_state.bash_argc_stack.is_empty() {
                self.shell_state.bash_argc_stack.remove(0);
            }
            for _ in args {
                if !self.shell_state.bash_argv_stack.is_empty() {
                    self.shell_state.bash_argv_stack.remove(0);
                }
            }
            // Fold any deferred comsub write-back into the child's cursor
            // before it is read — the pending offset was recorded against
            // the child's FUNCTION_STDIN buffer, so it must apply while
            // that buffer is still installed.
            self.apply_comsub_stdin_writeback();
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                FUNCTION_STDIN,
                old_function_stdin,
            );
            if stdin_carved_from_parent {
                // The child's FUNCTION_STDIN_OFFSET is its cursor into the
                // carved remainder; fold it back into the parent's cursor so
                // input the function did not read stays readable after return
                // (GNU: shared fd 0 position).
                let child_offset = self
                    .shell_state
                    .env_vars
                    .get(FUNCTION_STDIN_OFFSET)
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (parent_stdin_base + child_offset).to_string(),
                );
            } else {
                restore_optional_env_var(
                    &mut self.shell_state.env_vars,
                    FUNCTION_STDIN_OFFSET,
                    old_function_stdin_offset,
                );
            }
            match old_function {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_CURRENT_FUNCTION".to_string(), value);
                }
                None => {
                    self.shell_state
                        .env_vars
                        .remove("__RUBASH_CURRENT_FUNCTION");
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
        // GNU execute_cmd.c execute_function applies the call's redirections
        // to the shell's descriptors around the body (redir.c
        // do_redirections, undone on return), so `f &> out` routes BOTH
        // streams (command.h:32-35 r_err_and_out/r_append_err_and_out,
        // applied at redir.c:899-900/1030). Mirror
        // apply_brace_group_redirects: propagate through the shared helpers
        // so each body command's ordered redirect list — not only the
        // convenience fields — carries the fd-2 leg. Setting
        // `redirect_err_append` alone created the target file but left the
        // ordered fd-state machine unaware of it, so `f 2>err` / `f &>f`
        // leaked the body's stderr to the console.
        // GNU redir.c do_redirections applies the call's redirects to the
        // descriptors named by each redirection, not to fd 1/2
        // unconditionally: `f 4>&-' closes fd 4 for the body's duration and
        // must not be replayed as a stdout append on every inner command
        // (redir7.sub `stuff 4>&-' re-serialized the injected `>>&-' into a
        // spawned `-c' child and failed to parse it).
        let is_stdout_write_redirect = |redirect: &Redirect| {
            redirect.fd.unwrap_or(1) == 1
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Output
                        | crate::parser::RedirectKind::Append
                        | crate::parser::RedirectKind::ClobberOutput
                        | crate::parser::RedirectKind::CombinedOutput
                        | crate::parser::RedirectKind::CombinedAppend
                )
        };
        let is_stderr_write_redirect = |redirect: &Redirect| {
            redirect.fd.unwrap_or(2) == 2
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Output
                        | crate::parser::RedirectKind::Append
                        | crate::parser::RedirectKind::ClobberOutput
                        | crate::parser::RedirectKind::CombinedOutput
                        | crate::parser::RedirectKind::CombinedAppend
                )
        };
        if let Some(redirect) = call_cmd
            .redirect_out
            .as_ref()
            .filter(|r| is_stdout_write_redirect(r))
        {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            apply_stdout_append_redirect(body, &append_redirect);
        }
        if let Some(redirect) = call_cmd
            .append
            .as_ref()
            .filter(|r| is_stdout_write_redirect(r))
        {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            apply_stdout_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = call_cmd
            .redirect_err
            .as_ref()
            .filter(|r| is_stderr_write_redirect(r))
        {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            apply_stderr_append_redirect(body, &append_redirect);
        }
        if let Some(redirect) = call_cmd
            .redirect_err_append
            .as_ref()
            .filter(|r| is_stderr_write_redirect(r))
        {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            // `&>`/`&>>` store their fd-2 leg as a Combined* kind. A body
            // command's own `>other` redirect must still win fd 1, so the
            // propagated leg claims fd 2 only (redir.c:899-900 splits
            // r_err_and_out into fd-1 open plus fd-2 dup).
            if matches!(
                append_redirect.kind,
                crate::parser::RedirectKind::CombinedOutput
                    | crate::parser::RedirectKind::CombinedAppend
            ) {
                append_redirect.kind = crate::parser::RedirectKind::Append;
                append_redirect.operator = "2>>".to_string();
                append_redirect.fd = Some(2);
                append_redirect.append = true;
            }
            apply_stderr_append_redirect(body, &append_redirect);
        }

        Ok(())
    }

    /// Returns the call's stdin plus whether it was carved from the caller's
    /// FUNCTION_STDIN remainder. GNU execute_function inherits fd 0
    /// unchanged: the function and its caller share one input cursor, so a
    /// carved call must fold the child's final offset back onto the parent's
    /// cursor on return rather than drain to EOF.
    pub(in crate::executor) fn function_call_stdin(
        &mut self,
        call_cmd: &CommandNode,
    ) -> Result<(Option<String>, bool, Option<FunctionCallStdinBinding>), ExecuteError> {
        self.apply_comsub_stdin_writeback();
        let carves_parent_stdin = call_cmd.redirect_in.is_none()
            && call_cmd.heredoc.is_none()
            && call_cmd.here_string.is_none()
            && self.virtual_fd_stdin_remaining(0).is_none()
            && self.function_stdin_remaining().is_some();
        if let Some(input) = self.stdin_string_for_command_mut(call_cmd) {
            return Ok((Some(input), carves_parent_stdin, None));
        }

        let Some(redirect) = &call_cmd.redirect_in else {
            // A shell script read through `< file` still has the unread
            // portion of that virtual stdin available to commands it invokes.
            // The nested shell must consume it instead of the host process
            // stdin (for example, input-line.sh/input-line.sub).
            if let Some(input) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
                let offset = self
                    .shell_state
                    .env_vars
                    .get(FUNCTION_STDIN_OFFSET)
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                return Ok((
                    Some(input.get(offset..).unwrap_or_default().to_string()),
                    true,
                    None,
                ));
            }
            return Ok((self.virtual_fd_stdin_remaining(0), false, None));
        };
        if redirect.fd.unwrap_or(0) != 0 {
            return Ok((None, false, None));
        }
        let target = self.expand_redirect_target(redirect);
        if is_closed_redirect_target(&target) {
            return Ok((None, false, None));
        }
        // GNU execute_cmd.c:5269-5278 (execute_function) applies the call's
        // redirections to real descriptors for the body's duration
        // (redir.c do_redirections, unwound on return). Endpoints with no
        // EOF — CON via /dev/tty, a fifo, an fd alias — cannot be slurped
        // into FUNCTION_STDIN: read_to_string blocks forever waiting for a
        // console EOF that never comes (test.tests `t -t 0 < /dev/tty`
        // hung). Bind fd 0 to the live endpoint instead so the body's
        // readers reach it through fd_table.
        if let Some(source_fd) = redirect_target_fd(&target) {
            if self.fd_table.is_open_for_read(source_fd)
                || self.fd_table.read_endpoint(source_fd).is_some()
            {
                let saved = self.fd_table.entries.get(&0).cloned();
                if self.fd_table.dup_input(0, source_fd).is_ok() {
                    return Ok((None, false, Some(FunctionCallStdinBinding { entry: saved })));
                }
            }
            return Ok((None, false, None));
        }
        let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
        if !std::fs::metadata(&path)
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            let file = FileFd::open_read(path.clone())?;
            let saved = self.fd_table.entries.get(&0).cloned();
            self.fd_table
                .open_input(0, FdReadEndpoint::File(file), false);
            return Ok((None, false, Some(FunctionCallStdinBinding { entry: saved })));
        }
        // A `<(cmd)` temp path is a draining stream, not a replayable
        // file — serve the shared remainder (subst.c:7143).
        let input = match self.procsub_stream_take(&path) {
            Some(bytes) => crate::executor::substitution_metadata::bytes_to_shell_text(&bytes),
            None => fs::read_to_string(&path)?,
        };
        Ok((Some(input), false, None))
    }

    fn restore_function_call_fd0(&mut self, binding: FunctionCallStdinBinding) {
        match binding.entry {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
    }
}

/// A live fd-0 binding installed for a function call whose `<` target is
/// not a slurpable file (device, fifo, fd alias). `entry` is the
/// displaced table entry — `None` when fd 0 was unbound — restored after
/// the body finishes (GNU execute_cmd.c undo_redirections).
pub(in crate::executor) struct FunctionCallStdinBinding {
    pub(in crate::executor) entry: Option<crate::executor::fd_table::FdEntry>,
}

fn function_redirects_affect_body(command: &CommandNode) -> bool {
    command.redirect_out.is_some()
        || command.append.is_some()
        || command.redirect_err.is_some()
        || command.redirect_err_append.is_some()
        || !command.redirects.is_empty()
}
