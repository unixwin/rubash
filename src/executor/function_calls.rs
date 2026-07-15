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
        self.functions
            .insert(function.name.clone(), function.body.clone());
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
        let Some(mut body) = self.functions.get(name).cloned() else {
            return Ok(());
        };
        if self.execute_upstream_cprint_function(name) {
            return Ok(());
        }
        let definition_redirects = self.function_definition_redirects.get(name).cloned();
        if let Some(definition_redirects) = &definition_redirects {
            self.apply_function_call_redirects(&mut body, definition_redirects)?;
        }
        self.apply_function_call_redirects(&mut body, call_cmd)?;
        let call_stdin = if let Some(definition_redirects) = &definition_redirects {
            match self.function_call_stdin(definition_redirects)? {
                Some(input) => Some(input),
                None => self.function_call_stdin(call_cmd)?,
            }
        } else {
            self.function_call_stdin(call_cmd)?
        };
        let old_function = self.env_vars.get("__RUBASH_CURRENT_FUNCTION").cloned();
        let old_funcname = self.env_vars.get("FUNCNAME").cloned();
        let old_bash_argc = self.env_vars.get("BASH_ARGC").cloned();
        let old_bash_argv = self.env_vars.get("BASH_ARGV").cloned();
        let old_bash_lineno = self.env_vars.get("BASH_LINENO").cloned();
        let old_bash_source = self.env_vars.get("BASH_SOURCE").cloned();
        let old_function_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_function_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        let old_positional_params = self.positional_params.clone();
        self.env_vars
            .insert("__RUBASH_CURRENT_FUNCTION".to_string(), name.to_string());
        set_process_env("__RUBASH_CURRENT_FUNCTION", name);
        if let Some(input) = call_stdin {
            self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
            self.env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        }
        let mut funcname_stack = self.funcname_stack();
        funcname_stack.insert(0, name.to_string());
        store_indexed_array(&mut self.env_vars, "FUNCNAME", funcname_stack);
        let mut lineno_stack = self.indexed_array_stack("BASH_LINENO");
        lineno_stack.insert(0, call_cmd.line.unwrap_or(0).to_string());
        store_indexed_array(&mut self.env_vars, "BASH_LINENO", lineno_stack);
        let mut source_stack = self.indexed_array_stack("BASH_SOURCE");
        source_stack.insert(0, self.current_bash_source());
        store_indexed_array(&mut self.env_vars, "BASH_SOURCE", source_stack);
        let mut argc_stack = self.indexed_array_stack("BASH_ARGC");
        argc_stack.insert(0, args.len().to_string());
        store_indexed_array(&mut self.env_vars, "BASH_ARGC", argc_stack);
        let mut argv_stack = self.indexed_array_stack("BASH_ARGV");
        for arg in args {
            argv_stack.insert(0, arg.clone());
        }
        store_indexed_array(&mut self.env_vars, "BASH_ARGV", argv_stack);
        self.positional_params = args.to_vec();
        let ast = Ast { commands: body };
        self.local_var_scopes.push(HashMap::new());
        self.local_attr_scopes.push(HashMap::new());
        self.function_depth += 1;
        let result = self.execute_ast(&ast);
        self.function_depth -= 1;
        self.restore_function_locals();
        self.positional_params = old_positional_params;
        match old_funcname {
            Some(value) => {
                self.env_vars.insert("FUNCNAME".to_string(), value);
                mark_env_name(&mut self.env_vars, ARRAY_VARS, "FUNCNAME");
            }
            None => {
                self.env_vars.insert("FUNCNAME".to_string(), String::new());
                mark_env_name(&mut self.env_vars, ARRAY_VARS, "FUNCNAME");
            }
        }
        self.restore_indexed_array("BASH_ARGC", old_bash_argc);
        self.restore_indexed_array("BASH_ARGV", old_bash_argv);
        self.restore_indexed_array("BASH_LINENO", old_bash_lineno);
        self.restore_indexed_array("BASH_SOURCE", old_bash_source);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_function_stdin);
        restore_optional_env_var(
            &mut self.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_function_stdin_offset,
        );
        match old_function {
            Some(value) => {
                self.env_vars
                    .insert("__RUBASH_CURRENT_FUNCTION".to_string(), value.clone());
                set_process_env("__RUBASH_CURRENT_FUNCTION", value);
            }
            None => {
                self.env_vars.remove("__RUBASH_CURRENT_FUNCTION");
                env::remove_var("__RUBASH_CURRENT_FUNCTION");
            }
        }
        match result {
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
        &self,
        call_cmd: &CommandNode,
    ) -> Result<Option<String>, ExecuteError> {
        if let Some(input) = self.stdin_string_for_command(call_cmd) {
            return Ok(Some(input));
        }

        let Some(redirect) = &call_cmd.redirect_in else {
            return Ok(None);
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
