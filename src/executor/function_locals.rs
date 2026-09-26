use super::*;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};

impl Executor {
    pub(in crate::executor) fn save_local_names(&mut self, args: &[String]) {
        let mut names = Vec::new();
        // Typed values are captured alongside the legacy local scope below.
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

        let Some(scope) = self.shell_state.local_var_scopes.last_mut() else {
            return;
        };
        let Some(attr_scope_index) = self.shell_state.local_attr_scopes.len().checked_sub(1) else {
            return;
        };
        let Some(typed_scope) = self.shell_state.local_typed_scopes.last_mut() else {
            return;
        };
        for name in names {
            if scope.contains_key(&name) {
                continue;
            }
            scope.insert(name.clone(), self.shell_state.env_vars.get(&name).cloned());
            let attrs = capture_var_attrs(&self.shell_state.env_vars, &name);
            self.shell_state.local_attr_scopes[attr_scope_index].insert(name.clone(), attrs);
            typed_scope.insert(name.clone(), self.shell_state.variables.get(&name).cloned());
            // getopts' intra-word scan position is frame state in GNU Bash:
            // declaring OPTIND local resets the scan for this frame, and the
            // caller's position comes back when the frame's locals restore
            // (getopts8.sub: x,y,a,b,c,z, not an x,y,c cycle).
            if name == "OPTIND" && !scope.contains_key("__RUBASH_GETOPTS_OFFSET") {
                scope.insert(
                    "__RUBASH_GETOPTS_OFFSET".to_string(),
                    self.shell_state
                        .env_vars
                        .get("__RUBASH_GETOPTS_OFFSET")
                        .cloned(),
                );
                self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OFFSET");
            }
        }
    }

    /// GNU make_local_variable bookkeeping for one name: snapshot the env
    /// value, attribute set, and typed slot into the current frame so the
    /// frame restore returns them to the caller. No-op outside a function or
    /// when the name is already local at this frame.
    pub(in crate::executor) fn save_frame_local_name(&mut self, name: &str) {
        if self
            .shell_state
            .local_var_scopes
            .last()
            .is_none_or(|scope| scope.contains_key(name))
        {
            return;
        }
        let env_value = self.shell_state.env_vars.get(name).cloned();
        let attrs = capture_var_attrs(&self.shell_state.env_vars, name);
        let typed = self.shell_state.variables.get(name).cloned();
        if let Some(scope) = self.shell_state.local_var_scopes.last_mut() {
            scope.insert(name.to_string(), env_value);
        }
        if let Some(attr_scope) = self.shell_state.local_attr_scopes.last_mut() {
            attr_scope.insert(name.to_string(), attrs);
        }
        if let Some(typed_scope) = self.shell_state.local_typed_scopes.last_mut() {
            typed_scope.insert(name.to_string(), typed);
        }
        if name == "OPTIND" {
            if let Some(scope) = self.shell_state.local_var_scopes.last_mut() {
                if !scope.contains_key("__RUBASH_GETOPTS_OFFSET") {
                    let saved = self
                        .shell_state
                        .env_vars
                        .get("__RUBASH_GETOPTS_OFFSET")
                        .cloned();
                    scope.insert("__RUBASH_GETOPTS_OFFSET".to_string(), saved);
                }
            }
            self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        }
    }

    pub(in crate::executor) fn save_assignment_local_names(
        &mut self,
        assignments: &[(String, String)],
    ) {
        let names = assignments
            .iter()
            .map(|(name, _)| assignment_name_and_append(name).0.to_string())
            .collect::<Vec<_>>();

        let Some(scope) = self.shell_state.local_var_scopes.last_mut() else {
            return;
        };
        let Some(attr_scope_index) = self.shell_state.local_attr_scopes.len().checked_sub(1) else {
            return;
        };
        let Some(typed_scope) = self.shell_state.local_typed_scopes.last_mut() else {
            return;
        };
        for name in names {
            if scope.contains_key(&name) {
                continue;
            }
            scope.insert(name.clone(), self.shell_state.env_vars.get(&name).cloned());
            let attrs = capture_var_attrs(&self.shell_state.env_vars, &name);
            self.shell_state.local_attr_scopes[attr_scope_index].insert(name.clone(), attrs);
            typed_scope.insert(name.clone(), self.shell_state.variables.get(&name).cloned());
            // getopts' intra-word scan position is frame state in GNU Bash:
            // declaring OPTIND local resets the scan for this frame, and the
            // caller's position comes back when the frame's locals restore
            // (getopts8.sub: x,y,a,b,c,z, not an x,y,c cycle).
            if name == "OPTIND" && !scope.contains_key("__RUBASH_GETOPTS_OFFSET") {
                scope.insert(
                    "__RUBASH_GETOPTS_OFFSET".to_string(),
                    self.shell_state
                        .env_vars
                        .get("__RUBASH_GETOPTS_OFFSET")
                        .cloned(),
                );
                self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OFFSET");
            }
        }
    }

    pub(in crate::executor) fn posix_function_declare_prefix_assignments_are_local(
        &self,
        cmd: &CommandNode,
    ) -> bool {
        self.shell_state.function_depth > 0
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
        if self.shell_state.function_depth == 0
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
                    self.shell_state.env_vars.get(name).cloned(),
                    is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name),
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
                    set_local_export_env_value(&mut self.shell_state.env_vars, &name, value);
                }
            }
            self.shell_state.env_vars.remove(&name);
            env::remove_var(&name);
            mark_env_name(&mut self.shell_state.env_vars, DECLARED_UNSET_VARS, &name);
        }
    }

    /// GNU set.def:330-352 get_current_options: a bitmap over every `set -o`
    /// option (letter flags and binary options alike). Serialized as
    /// `name=0|1` pairs for the `-` local's value.
    pub(in crate::executor) fn current_options_bitmap(&self) -> String {
        crate::builtins::set::shell_option_names()
            .map(|name| {
                format!(
                    "{name}={}",
                    crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, name)
                        as u8
                )
            })
            .collect::<Vec<_>>()
            .join(DATA_DOLLAR_STR)
    }

    /// GNU set.def:358-386 set_current_options: apply the bitmap saved by a
    /// `-` local — only options whose state differs are flipped, and the
    /// binary-option side effects run with them (set.def:388-399
    /// set_ignoreeof binds IGNOREEOF=10 / unbinds it).
    fn apply_options_bitmap(&mut self, bitmap: &str) {
        for entry in bitmap.split(DATA_DOLLAR) {
            let Some((name, state)) = entry.split_once('=') else {
                continue;
            };
            let enabled = state == "1";
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, name)
                == enabled
            {
                continue;
            }
            crate::builtins::set::set_shell_option(&mut self.shell_state.env_vars, name, enabled);
            if name == "ignoreeof" {
                // Same typed-owner mirror as the `set -o ignoreeof` path —
                // set.def:388-399 binds IGNOREEOF=10 or unbinds it.
                if enabled {
                    let _ = self
                        .shell_state
                        .variables
                        .set_scalar("IGNOREEOF", "10".to_string());
                } else {
                    self.shell_state.variables.remove("IGNOREEOF");
                }
            }
            if name == "posix" {
                self.shell_state.env_vars.insert(
                    "__RUBASH_POSIX_MODE".to_string(),
                    if enabled { "1" } else { "0" }.to_string(),
                );
            }
        }
    }

    pub(in crate::executor) fn restore_function_locals(&mut self) -> HashSet<String> {
        let Some(scope) = self.shell_state.local_var_scopes.pop() else {
            return HashSet::new();
        };
        // GNU variables.c:5271-5275 (push_posix_tempvar_internal, reached via
        // pop_var_context -> hash_flush -> push_func_var): a local `-`
        // restores the saved `set -o` options when its frame pops.
        if scope.contains_key("-") {
            if let Some(bitmap) = self.shell_state.env_vars.get("-").cloned() {
                self.apply_options_bitmap(&bitmap);
            }
        }
        let attr_scope = self.shell_state.local_attr_scopes.pop().unwrap_or_default();
        let typed_scope = self
            .shell_state
            .local_typed_scopes
            .pop()
            .unwrap_or_default();
        let mut names = HashSet::new();
        for (name, value) in scope {
            names.insert(name.clone());
            match value {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert(name.clone(), value.clone());
                    // Internal pseudo-variables (e.g. the getopts scan
                    // offset saved alongside a local OPTIND) must not
                    // leak into the child process environment.
                    if !name.starts_with("__RUBASH_") {
                        set_process_env(&name, value);
                    }
                }
                None => {
                    self.shell_state.env_vars.remove(&name);
                    env::remove_var(&name);
                }
            }
            set_var_attrs(
                &mut self.shell_state.env_vars,
                &name,
                attr_scope.get(&name).copied().unwrap_or_default(),
            );
            remove_local_export_env_value(&mut self.shell_state.env_vars, &name);
        }
        for (name, variable) in typed_scope {
            self.shell_state.variables.remove(&name);
            if let Some(variable) = variable {
                let _ = self.shell_state.variables.set(name, variable);
            }
        }
        names
    }

    pub(in crate::executor) fn begin_global_declare_for_local_names(
        &mut self,
        args: &[String],
    ) -> Vec<SavedGlobalDeclareLocal> {
        if self.shell_state.function_depth == 0 || !declare_args_force_global(args) {
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
                local_value: self.shell_state.env_vars.get(name).cloned(),
                local_attrs: capture_var_attrs(&self.shell_state.env_vars, name),
                local_typed: self.shell_state.variables.get(name).cloned(),
            });
        }

        for saved in &saved_locals {
            let scope = &self.shell_state.local_var_scopes[saved.scope_index];
            let attr_scope = &self.shell_state.local_attr_scopes[saved.scope_index];
            restore_optional_shell_var(
                &mut self.shell_state.env_vars,
                &saved.name,
                scope.get(&saved.name).cloned().flatten(),
            );
            set_var_attrs(
                &mut self.shell_state.env_vars,
                &saved.name,
                attr_scope.get(&saved.name).copied().unwrap_or_default(),
            );
            let global_typed = self.shell_state.local_typed_scopes[saved.scope_index]
                .get(&saved.name)
                .cloned()
                .flatten();
            self.shell_state.variables.remove(&saved.name);
            if let Some(variable) = global_typed {
                let _ = self.shell_state.variables.set(&saved.name, variable);
            }
        }

        saved_locals
    }

    pub(in crate::executor) fn visible_local_scope_index(&self, name: &str) -> Option<usize> {
        self.shell_state
            .local_var_scopes
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
            let Some(scope) = self.shell_state.local_var_scopes.get_mut(saved.scope_index) else {
                continue;
            };
            scope.insert(
                saved.name.clone(),
                self.shell_state.env_vars.get(&saved.name).cloned(),
            );
            let Some(attr_scope) = self
                .shell_state
                .local_attr_scopes
                .get_mut(saved.scope_index)
            else {
                continue;
            };
            attr_scope.insert(
                saved.name.clone(),
                capture_var_attrs(&self.shell_state.env_vars, &saved.name),
            );
            let typed_scope = self
                .shell_state
                .local_typed_scopes
                .get_mut(saved.scope_index);
            if let Some(typed_scope) = typed_scope {
                typed_scope.insert(
                    saved.name.clone(),
                    self.shell_state.variables.get(&saved.name).cloned(),
                );
            }
            self.shell_state.variables.remove(&saved.name);
            if let Some(variable) = saved.local_typed {
                let _ = self.shell_state.variables.set(&saved.name, variable);
            }
            restore_optional_shell_var(
                &mut self.shell_state.env_vars,
                &saved.name,
                saved.local_value,
            );
            set_var_attrs(
                &mut self.shell_state.env_vars,
                &saved.name,
                saved.local_attrs,
            );
        }
    }
}
