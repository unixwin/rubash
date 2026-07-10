use super::*;

impl Executor {
    pub(in crate::executor) fn save_local_names(&mut self, args: &[String]) {
        let mut names = Vec::new();
        for arg in args {
            if arg == "--" {
                continue;
            }
            if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
                continue;
            }
            let Some(name) = local_assignment_name(arg) else {
                continue;
            };
            names.push(name.to_string());
        }

        let Some(scope) = self.local_var_scopes.last_mut() else {
            return;
        };
        let Some(attr_scope_index) = self.local_attr_scopes.len().checked_sub(1) else {
            return;
        };
        for name in names {
            if scope.contains_key(&name) {
                continue;
            }
            scope.insert(name.clone(), self.env_vars.get(&name).cloned());
            let attrs = capture_var_attrs(&self.env_vars, &name);
            self.local_attr_scopes[attr_scope_index].insert(name, attrs);
        }
    }

    pub(in crate::executor) fn save_assignment_local_names(
        &mut self,
        assignments: &HashMap<String, String>,
    ) {
        let names = assignments
            .keys()
            .map(|name| assignment_name_and_append(name).0.to_string())
            .collect::<Vec<_>>();

        let Some(scope) = self.local_var_scopes.last_mut() else {
            return;
        };
        let Some(attr_scope_index) = self.local_attr_scopes.len().checked_sub(1) else {
            return;
        };
        for name in names {
            if scope.contains_key(&name) {
                continue;
            }
            scope.insert(name.clone(), self.env_vars.get(&name).cloned());
            let attrs = capture_var_attrs(&self.env_vars, &name);
            self.local_attr_scopes[attr_scope_index].insert(name, attrs);
        }
    }

    pub(in crate::executor) fn posix_function_declare_prefix_assignments_are_local(
        &self,
        cmd: &CommandNode,
    ) -> bool {
        self.function_depth > 0
            && self.posix_mode_enabled()
            && !cmd.assignments.is_empty()
            && cmd
                .words
                .first()
                .is_some_and(|word| matches!(word.as_str(), "declare" | "typeset"))
            && !declare_args_force_global(&cmd.words[1..])
            && !declare_args_request_print(&cmd.words[1..])
    }

    pub(in crate::executor) fn posix_function_declare_unset_export_names(
        &self,
        args: &[String],
    ) -> Vec<(String, Option<String>, bool)> {
        if self.function_depth == 0
            || !self.posix_mode_enabled()
            || declare_args_force_global(args)
            || declare_args_request_print(args)
            || !declare_args_contain_option(args, 'x', false)
        {
            return Vec::new();
        }

        args.iter()
            .filter(|arg| {
                !((arg.starts_with('-') || arg.starts_with('+'))
                    && arg.as_str() != "-"
                    && arg.as_str() != "+")
            })
            .filter_map(|arg| local_assignment_name(arg))
            .map(|name| {
                (
                    name.to_string(),
                    self.env_vars.get(name).cloned(),
                    is_marked_var(&self.env_vars, EXPORTED_VARS, name),
                )
            })
            .collect()
    }

    pub(in crate::executor) fn apply_posix_function_declare_unset_export(
        &mut self,
        names: Vec<(String, Option<String>, bool)>,
    ) {
        for (name, old_value, was_exported) in names {
            if was_exported {
                if let Some(value) = old_value {
                    set_local_export_env_value(&mut self.env_vars, &name, value);
                }
            }
            self.env_vars.remove(&name);
            env::remove_var(&name);
            mark_env_name(&mut self.env_vars, DECLARED_UNSET_VARS, &name);
        }
    }

    pub(in crate::executor) fn restore_function_locals(&mut self) -> HashSet<String> {
        let Some(scope) = self.local_var_scopes.pop() else {
            return HashSet::new();
        };
        let attr_scope = self.local_attr_scopes.pop().unwrap_or_default();
        let mut names = HashSet::new();
        for (name, value) in scope {
            names.insert(name.clone());
            match value {
                Some(value) => {
                    self.env_vars.insert(name.clone(), value.clone());
                    set_process_env(&name, value);
                }
                None => {
                    self.env_vars.remove(&name);
                    env::remove_var(&name);
                }
            }
            set_var_attrs(
                &mut self.env_vars,
                &name,
                attr_scope.get(&name).copied().unwrap_or_default(),
            );
            remove_local_export_env_value(&mut self.env_vars, &name);
        }
        names
    }

    pub(in crate::executor) fn begin_global_declare_for_local_names(
        &mut self,
        args: &[String],
    ) -> Vec<SavedGlobalDeclareLocal> {
        if self.function_depth == 0 || !declare_args_force_global(args) {
            return Vec::new();
        }

        let mut saved_locals = Vec::new();
        let mut seen = HashSet::new();
        for arg in args {
            if arg == "--" {
                continue;
            }
            if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
                continue;
            }
            let Some(name) = local_assignment_name(arg) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                continue;
            }
            let Some(scope_index) = self.visible_local_scope_index(name) else {
                continue;
            };
            saved_locals.push(SavedGlobalDeclareLocal {
                name: name.to_string(),
                scope_index,
                local_value: self.env_vars.get(name).cloned(),
                local_attrs: capture_var_attrs(&self.env_vars, name),
            });
        }

        for saved in &saved_locals {
            let scope = &self.local_var_scopes[saved.scope_index];
            let attr_scope = &self.local_attr_scopes[saved.scope_index];
            restore_optional_shell_var(
                &mut self.env_vars,
                &saved.name,
                scope.get(&saved.name).cloned().flatten(),
            );
            set_var_attrs(
                &mut self.env_vars,
                &saved.name,
                attr_scope.get(&saved.name).copied().unwrap_or_default(),
            );
        }

        saved_locals
    }

    pub(in crate::executor) fn visible_local_scope_index(&self, name: &str) -> Option<usize> {
        self.local_var_scopes
            .iter()
            .rposition(|scope| scope.contains_key(name))
    }

    pub(in crate::executor) fn finish_global_declare_for_local_names(
        &mut self,
        saved_locals: Vec<SavedGlobalDeclareLocal>,
    ) {
        if saved_locals.is_empty() {
            return;
        }

        for saved in saved_locals {
            let Some(scope) = self.local_var_scopes.get_mut(saved.scope_index) else {
                continue;
            };
            scope.insert(saved.name.clone(), self.env_vars.get(&saved.name).cloned());
            let Some(attr_scope) = self.local_attr_scopes.get_mut(saved.scope_index) else {
                continue;
            };
            attr_scope.insert(
                saved.name.clone(),
                capture_var_attrs(&self.env_vars, &saved.name),
            );
            restore_optional_shell_var(&mut self.env_vars, &saved.name, saved.local_value);
            set_var_attrs(&mut self.env_vars, &saved.name, saved.local_attrs);
        }
    }
}
