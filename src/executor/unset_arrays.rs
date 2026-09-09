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
                self.function_def_infos.remove(name);
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
            // GNU builtins/set.def:927-935 + 990-1010 (unset_builtin): a
            // subscripted name whose base is a nameref unbinds the referenced
            // array's element (unset n[0] with n->v removes v[0]); a plain
            // nameref whose cell is an array reference unbinds that element
            // while keeping the nameref itself.
            if self.unset_through_nameref(&name) {
                continue;
            }
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
        // GNU builtins/set.def:995-1000: unsetting through a nameref removes
        // the referenced variable and keeps the nameref; drop the referenced
        // variable from the typed owner too so parameter expansion does not
        // see a stale value (unset foo with foo->bar must clear bar).
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            if is_marked_var(&self.env_vars, NAMEREF_VARS, name) {
                if let Some(cell) = self
                    .env_vars
                    .get(name)
                    .filter(|cell| is_shell_name(cell))
                    .cloned()
                {
                    self.shell_state.variables.remove(cell.as_str());
                }
            }
        }
        // Also remove from shell_state.variables so that shell_variable_value
        // does not return a stale value for an unset variable (the builtin
        // only cleans env_vars).  Without this, ${var-word} after
        // `var=""; unset var` still sees the old empty value.
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            self.shell_state.variables.remove(name);
        }
        Ok(if function_status != 0 {
            function_status
        } else {
            variable_status
        })
    }

    /// GNU builtins/set.def:990-1010 (unset_builtin): `unset -v` of a
    /// nameref whose cell is an array reference unbinds the referenced
    /// element and keeps the nameref; a subscripted name whose base is a
    /// nameref resolves the subscript against the referenced array
    /// (nameref3/nameref15.sub). Returns true when this call performed the
    /// unbind and the ordinary variable path must be skipped.
    pub(in crate::executor) fn unset_through_nameref(&mut self, name: &str) -> bool {
        if is_marked_var(&self.env_vars, NAMEREF_VARS, name) {
            let cell = self.env_vars.get(name).cloned().unwrap_or_default();
            if parse_array_subscript(&cell).is_some() {
                return self.unset_array_element(&cell);
            }
            return false;
        }
        let Some((base, subscript)) = parse_array_subscript(name) else {
            return false;
        };
        if !is_marked_var(&self.env_vars, NAMEREF_VARS, base) {
            return false;
        }
        let Some(cell) = self.env_vars.get(base).filter(|cell| is_shell_name(cell)) else {
            return false;
        };
        let cell = cell.clone();
        self.unset_array_element(&format!("{cell}[{subscript}]"))
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
            // GNU unbind_array_element (arrayfunc.c:1180-1200): with the
            // default compat level (> 51), `unset arr[*]` / `unset arr[@]`
            // FLUSHES every element (behavior 2) instead of unsetting the
            // variable or treating * as an index; the variable itself stays
            // declared as an empty array (array.tests: `unset e[*]` then
            // `declare -a e=()`).
            if subscript == "*" || subscript == "@" {
                self.env_vars.insert(
                    array_name.to_string(),
                    format_indexed_array_storage(Default::default()),
                );
                return true;
            }
            let subscript = self.expand_arithmetic_special_parameters(subscript);
            let Some(index) = self.eval_arithmetic_expansion_value(&subscript) else {
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

        // GNU arrayfunc.c:1218-1231 unbind_array_element scalar branch: for
        // a non-array variable the subscript is evaluated arithmetically and
        // subscript 0 IS the variable itself, so the whole variable is
        // unbound (array.tests: unset 'v[0]' on a scalar removes v). Any
        // other subscript returns -2, which unset.def reports as "not an
        // array variable" -- reached by returning false here, as does an
        // @/* subscript (arrayfunc.c:1163-1164).
        if subscript == "*" || subscript == "@" {
            return false;
        }
        let subscript = self.expand_arithmetic_special_parameters(subscript);
        if self.eval_arithmetic_expansion_value(&subscript) == Some(0) {
            if is_marked_var(&self.env_vars, READONLY_VARS, array_name) {
                return false;
            }
            self.env_vars.remove(array_name);
            std::env::remove_var(array_name);
            self.shell_state.variables.remove(array_name);
            for key in [
                EXPORTED_VARS,
                READONLY_VARS,
                ARRAY_VARS,
                ASSOC_VARS,
                INTEGER_VARS,
                UPPERCASE_VARS,
                LOWERCASE_VARS,
                NAMEREF_VARS,
                DECLARED_UNSET_VARS,
            ] {
                unmark_env_name(&mut self.env_vars, key, array_name);
            }
            return true;
        }

        false
    }
}
