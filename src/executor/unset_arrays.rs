use super::*;

impl Executor {
    pub(in crate::executor) fn execute_unset(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return self.execute_unset_with_stderr(&cmd.words[1..], &mut std::io::sink());
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return self.execute_unset_with_stderr(&cmd.words[1..], &mut file);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return self.execute_unset_with_stderr(&cmd.words[1..], &mut file);
        }

        self.execute_unset_with_stderr(&cmd.words[1..], &mut std::io::stderr().lock())
    }

    pub(in crate::executor) fn execute_unset_with_stderr<W>(
        &mut self,
        args: &[String],
        stderr: &mut W,
    ) -> Result<i32, ExecuteError>
    where
        W: Write,
    {
        // TODO(builtins/set.def/variables.c/execute_cmd.c): `unset` searches
        // variables and functions with nuanced attributes. Keep function table
        // and variable table behavior aligned for builtins6.sub.
        if unset_args_need_builtin_diagnostics(args) {
            return crate::builtins::set::unset_with_stderr(
                args.iter().map(String::as_str),
                &mut self.env_vars,
                stderr,
            )
            .map_err(ExecuteError::from);
        }

        let function_only = args.iter().any(|arg| arg == "-f");
        let variable_only = args.iter().any(|arg| arg == "-v");
        let names: Vec<String> = args
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .cloned()
            .collect();

        let mut function_status = 0;
        if !variable_only {
            for name in &names {
                if marked_env_names(&self.env_vars, READONLY_FUNCTIONS)
                    .iter()
                    .any(|readonly| readonly == name)
                {
                    writeln!(
                        stderr,
                        "{}unset: {name}: cannot unset: readonly function",
                        self.diagnostic_prefix()
                    )?;
                    function_status = 1;
                    continue;
                }
                self.functions.remove(name);
                self.function_definition_redirects.remove(name);
                unmark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
            }
        }

        if function_only {
            return Ok(function_status);
        }

        let mut variable_args: Vec<String> = args
            .iter()
            .filter(|arg| arg.starts_with('-') && arg.as_str() != "-f")
            .cloned()
            .collect();
        for name in names {
            if self.unset_array_element(&name) {
                continue;
            }
            if self.unset_outer_local_variable(&name) {
                continue;
            }
            variable_args.push(name);
        }

        let variable_status = crate::builtins::set::unset_with_stderr(
            variable_args.iter().map(String::as_str),
            &mut self.env_vars,
            stderr,
        )
        .map_err(ExecuteError::from)?;
        Ok(if function_status != 0 {
            function_status
        } else {
            variable_status
        })
    }

    pub(in crate::executor) fn unset_outer_local_variable(&mut self, name: &str) -> bool {
        if is_marked_var(&self.env_vars, READONLY_VARS, name) {
            return false;
        }
        let Some(current_scope_index) = self.local_var_scopes.len().checked_sub(1) else {
            return false;
        };
        let Some(scope_index) = self.visible_local_scope_index(name) else {
            return false;
        };
        if scope_index >= current_scope_index {
            return false;
        }
        let previous = self.local_var_scopes[scope_index].remove(name);
        let attrs = self.local_attr_scopes[scope_index]
            .remove(name)
            .unwrap_or_default();
        restore_optional_shell_var(&mut self.env_vars, name, previous.flatten());
        set_var_attrs(&mut self.env_vars, name, attrs);
        true
    }

    pub(in crate::executor) fn unset_array_element(&mut self, name: &str) -> bool {
        let Some((array_name, subscript)) = parse_array_subscript(name) else {
            return false;
        };
        if array_name == "BASH_ALIASES" {
            let key = subscript.trim_matches('\'').trim_matches('"');
            self.aliases.remove(key);
            self.sync_dynamic_assoc_vars();
            return true;
        }
        if array_name == "BASH_CMDS" {
            let key = subscript.trim_matches('\'').trim_matches('"');
            crate::builtins::hash::remove_hashed_path(&mut self.env_vars, key);
            self.sync_dynamic_assoc_vars();
            return true;
        }
        let Some(current) = self.env_vars.get(array_name).cloned() else {
            return false;
        };

        if is_marked_var(&self.env_vars, ASSOC_VARS, array_name) {
            let key = subscript.trim_matches('\'').trim_matches('"');
            let mut entries = assoc_entries(&current);
            entries.retain(|(entry_key, _)| entry_key != key);
            self.env_vars
                .insert(array_name.to_string(), format_assoc_storage(entries));
            return true;
        }

        if is_marked_array_var(&self.env_vars, array_name) || is_array_storage(&current) {
            let subscript = self.expand_arithmetic_special_parameters(subscript);
            let Some(index) = eval_conditional_arith_value(&subscript, &self.env_vars) else {
                return false;
            };
            let Some(index) = resolve_indexed_array_subscript(&current, index) else {
                return false;
            };
            let mut entries = indexed_array_entries(&current);
            entries.remove(&index);
            self.env_vars.insert(
                array_name.to_string(),
                format_indexed_array_storage(entries),
            );
            return true;
        }

        false
    }
}
