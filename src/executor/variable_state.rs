use super::*;

impl Executor {
    pub(crate) fn alias_expansion_enabled(&self) -> bool {
        self.env_vars
            .get("__RUBASH_SHOPT_STATE")
            .is_some_and(|value| value.split('\x1f').any(|name| name == "expand_aliases"))
    }

    pub(in crate::executor) fn apply_case_assignment_attributes(
        &self,
        name: &str,
        value: String,
    ) -> String {
        if is_marked_var(&self.env_vars, UPPERCASE_VARS, name) {
            value.to_uppercase()
        } else if is_marked_var(&self.env_vars, LOWERCASE_VARS, name) {
            value.to_lowercase()
        } else if is_marked_var(&self.env_vars, CAPCASE_VARS, name) {
            // GNU capitalize: first character uppercased, rest lowercased
            // (variables.c capcase assignment, casemod.tests:99-103).
            let mut chars = value.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => value,
            }
        } else {
            value
        }
    }

    pub(in crate::executor) fn nameref_target_name(&self, name: &str) -> Option<String> {
        match self.nameref_resolution(name) {
            NamerefResolution::Target(target) => Some(target),
            NamerefResolution::Circular | NamerefResolution::NotNameref => None,
        }
    }

    pub(in crate::executor) fn resolved_variable_name(&self, name: &str) -> Option<String> {
        match self.nameref_resolution(name) {
            NamerefResolution::Target(target) => Some(target),
            NamerefResolution::Circular => None,
            NamerefResolution::NotNameref => Some(name.to_string()),
        }
    }

    pub(in crate::executor) fn nameref_resolution(&self, name: &str) -> NamerefResolution {
        let mut current = name;
        let mut seen = HashSet::new();
        for _ in 0..16 {
            if !seen.insert(current.to_string()) {
                return NamerefResolution::Circular;
            }
            if !is_marked_var(&self.env_vars, NAMEREF_VARS, current) {
                return NamerefResolution::NotNameref;
            }
            let Some(target) = self.env_vars.get(current) else {
                return NamerefResolution::NotNameref;
            };
            if !is_shell_name(target) && parse_array_subscript(target).is_none() {
                return NamerefResolution::NotNameref;
            }
            if !is_marked_var(&self.env_vars, NAMEREF_VARS, target) {
                return NamerefResolution::Target(target.clone());
            }
            current = target;
        }
        NamerefResolution::Circular
    }

    /// GNU variables.c:2011 find_variable_nameref: when a nameref chain
    /// resolves back onto itself, the fallback is the GLOBAL variable of the
    /// name that closed the loop, searched without following namerefs
    /// ("XXX - provisional change - circular refs go to global scope for
    /// resolution, without namerefs", variables.c:2036-2046). This only
    /// applies inside a function context (`variable_context && v->context`);
    /// at the global scope the chain resolution returns nothing.
    pub(in crate::executor) fn nameref_circular_fallback_name(
        &self,
        name: &str,
    ) -> Option<String> {
        if self.function_depth == 0 {
            return None;
        }
        let mut current = name;
        let mut seen = HashSet::new();
        for _ in 0..16 {
            if !seen.insert(current.to_string()) {
                return Some(current.to_string());
            }
            if !is_marked_var(&self.env_vars, NAMEREF_VARS, current) {
                return None;
            }
            let target = self.env_vars.get(current)?;
            if !is_shell_name(target) && parse_array_subscript(target).is_none() {
                return None;
            }
            current = target.as_str();
        }
        None
    }

    /// The value of the global namesake a circular nameref falls back to
    /// (GNU find_global_variable_noref): the typed owner only, because the
    /// legacy env_vars cell holds the local nameref's reference, never the
    /// shadowed global value.
    pub(in crate::executor) fn circular_fallback_value(&self, name: &str) -> Option<String> {
        let fallback = self.nameref_circular_fallback_name(name)?;
        match self.shell_state.variables.get(&fallback) {
            Some(crate::shell::Variable {
                value: crate::shell::ShellValue::Scalar(value),
                ..
            }) => Some(value.clone()),
            _ => None,
        }
    }

    /// GNU variables.c bind_variable: assigning through a circular nameref
    /// inside a function writes the global namesake (bind_global_variable on
    /// the maxloop path) while the local nameref keeps its cell. rubash
    /// models the global value in the typed owner plus the frame snapshots
    /// that restore it when the function returns.
    pub(in crate::executor) fn assign_circular_fallback(&mut self, name: &str, value: String) {
        let Some(fallback) = self.nameref_circular_fallback_name(name) else {
            return;
        };
        let mut variable = match self.shell_state.variables.get(&fallback) {
            Some(existing) => existing.clone(),
            None => crate::shell::Variable::scalar(String::new()),
        };
        variable.value = crate::shell::ShellValue::Scalar(value.clone());
        let _ = self
            .shell_state
            .variables
            .set(fallback.clone(), variable.clone());
        // Update the frame snapshots so the global value survives the
        // local-variable restore when the function returns.
        if let Some(typed_scope) = self.local_typed_scopes.last_mut() {
            if typed_scope.contains_key(&fallback) {
                typed_scope.insert(fallback.clone(), Some(variable));
            }
        }
        if let Some(scope) = self.local_var_scopes.last_mut() {
            if scope.contains_key(&fallback) {
                scope.insert(fallback.clone(), Some(value));
            }
        }
    }

    pub(in crate::executor) fn shell_variable_value(&self, name: &str) -> Option<String> {
        let name = match self.nameref_resolution(name) {
            NamerefResolution::Target(target) => target,
            NamerefResolution::Circular => {
                eprintln!(
                    "{}warning: {}: circular name reference",
                    self.diagnostic_prefix(),
                    name
                );
                // GNU variables.c:2036-2046: circular refs inside a function
                // resolve at the global scope without namerefs.
                return self.circular_fallback_value(name);
            }
            NamerefResolution::NotNameref => name.to_string(),
        };
        if let Some(value) = self.array_element_parameter_value(&name) {
            return Some(value);
        }
        if let Some(crate::shell::Variable {
            value: crate::shell::ShellValue::Scalar(value),
            ..
        }) = self.shell_state.variables.get(&name)
        {
            return Some(value.clone());
        }
        self.env_vars
            .get(&name)
            .and_then(|value| self.scalar_parameter_value(&name, value))
    }

    pub(in crate::executor) fn scalar_parameter_value(
        &self,
        name: &str,
        value: &str,
    ) -> Option<String> {
        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            return assoc_value_at(value, "0");
        }
        if is_marked_array_var(&self.env_vars, name) {
            return array_value_at(value, 0);
        }
        Some(value.to_string())
    }

    pub(in crate::executor) fn eval_integer_assignment_value(&self, value: &str) -> i128 {
        eval_conditional_arith_value(value, &self.env_vars).unwrap_or(0)
    }

    pub(in crate::executor) fn mark_exported(&mut self, name: &str) {
        let mut exported: Vec<String> = self
            .env_vars
            .get(EXPORTED_VARS)
            .map(|value| {
                value
                    .split('\x1f')
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        if !exported.iter().any(|exported_name| exported_name == name) {
            exported.push(name.to_string());
        }
        self.env_vars
            .insert(EXPORTED_VARS.to_string(), exported.join("\x1f"));
    }

    pub(in crate::executor) fn keeps_temporary_assignments(&self, cmd: &CommandNode) -> bool {
        // TODO(execute_cmd.c/variables.c): Bash has precise persistence rules
        // for assignment words before special builtins. This covers the POSIX
        // special-builtin and export cases exercised by upstream builtins.tests.
        let Some(command) = cmd.words.first().map(String::as_str) else {
            return false;
        };

        matches!(command, "export" | "readonly")
            || ((command == "declare" || command == "typeset")
                && Self::declare_applies_persistent_attribute(cmd))
            || (command == "eval" && cmd
.assignment_keys().any(|name| name.ends_with('+')))
            || (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1")
                && matches!(command, "." | "source" | "eval" | ":" | "return"))
    }

    // GNU declare.def:1045-1086: a variable found in the temporary environment
    // is converted into a real variable (keeping its value, dropping the
    // tempenv attribute) only when the declare applies the export or readonly
    // attribute — `var=value declare -x var` behaves like `var=value export
    // var`.  Attribute-free forms (`declare -p`, `declare -i`, plain
    // `declare name`) leave the assignment temporary, so it vanishes with the
    // command.
    fn declare_applies_persistent_attribute(cmd: &CommandNode) -> bool {
        cmd.words
            .iter()
            .skip(1)
            .take_while(|word| {
                let word = word.as_str();
                word.starts_with('-') && word.len() > 1 && word != "--"
            })
            .any(|word| word.contains('x') || word.contains('r'))
    }

    pub(in crate::executor) fn posix_mode_enabled(&self) -> bool {
        self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1")
    }

    pub(in crate::executor) fn applied_temporary_assignment_values(
        &self,
        assignments: &[(String, String)],
    ) -> HashMap<String, Option<String>> {
        assignments
            .iter()
            .map(|(name, _)| {
                let (base_name, _) = assignment_name_and_append(name);
                (base_name.to_string(), self.env_vars.get(base_name).cloned())
            })
            .collect()
    }

    pub(in crate::executor) fn restore_function_temporary_assignments(
        &mut self,
        previous: Vec<(String, Option<String>, Option<crate::shell::Variable>)>,
        applied: HashMap<String, Option<String>>,
    ) {
        for (name, value, typed_value) in previous.into_iter().rev() {
            if name != EXPORTED_VARS {
                if is_marked_var(&self.env_vars, POSIX_FUNCTION_EXPORT_TOUCHED, &name) {
                    continue;
                }
                let current = self.env_vars.get(&name).cloned();
                if applied
                    .get(&name)
                    .is_some_and(|applied_value| current != *applied_value)
                {
                    continue;
                }
            }
            if let Some(value) = value {
                self.env_vars.insert(name.clone(), value.clone());
                set_process_env(&name, value);
            } else {
                self.env_vars.remove(&name);
                env::remove_var(&name);
            }
            self.restore_typed_temporary_value(&name, typed_value);
        }
    }

    pub(in crate::executor) fn restore_temporary_assignments(
        &mut self,
        previous: Vec<(String, Option<String>, Option<crate::shell::Variable>)>,
    ) {
        for (name, value, typed_value) in previous.into_iter().rev() {
            if let Some(value) = value {
                self.env_vars.insert(name.clone(), value.clone());
                set_process_env(&name, value);
            } else {
                self.env_vars.remove(&name);
                env::remove_var(&name);
            }
            self.restore_typed_temporary_value(&name, typed_value);
        }
    }

    fn restore_typed_temporary_value(
        &mut self,
        name: &str,
        typed_value: Option<crate::shell::Variable>,
    ) {
        // Temporary assignment words must not outlive the command, even in the
        // typed owner that parameter expansion reads first. Remove then set so
        // a readonly flag gained mid-command cannot block the restore.
        self.shell_state.variables.remove(name);
        if let Some(variable) = typed_value {
            let _ = self.shell_state.variables.set(name.to_string(), variable);
        }
    }
}
