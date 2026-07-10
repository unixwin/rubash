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
            if !is_shell_name(target) {
                return NamerefResolution::NotNameref;
            }
            if !is_marked_var(&self.env_vars, NAMEREF_VARS, target) {
                return NamerefResolution::Target(target.clone());
            }
            current = target;
        }
        NamerefResolution::Circular
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
                return None;
            }
            NamerefResolution::NotNameref => name.to_string(),
        };
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

        matches!(command, "export" | "declare" | "typeset" | "readonly")
            || (command == "eval" && cmd.assignments.keys().any(|name| name.ends_with('+')))
            || (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1")
                && matches!(command, "." | "source" | "eval" | ":" | "return"))
    }

    pub(in crate::executor) fn posix_mode_enabled(&self) -> bool {
        self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1")
    }

    pub(in crate::executor) fn applied_temporary_assignment_values(
        &self,
        assignments: &HashMap<String, String>,
    ) -> HashMap<String, Option<String>> {
        assignments
            .keys()
            .map(|name| {
                let (base_name, _) = assignment_name_and_append(name);
                (base_name.to_string(), self.env_vars.get(base_name).cloned())
            })
            .collect()
    }

    pub(in crate::executor) fn restore_function_temporary_assignments(
        &mut self,
        previous: Vec<(String, Option<String>)>,
        applied: HashMap<String, Option<String>>,
    ) {
        for (name, value) in previous.into_iter().rev() {
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
                env::remove_var(name);
            }
        }
    }

    pub(in crate::executor) fn restore_temporary_assignments(
        &mut self,
        previous: Vec<(String, Option<String>)>,
    ) {
        for (name, value) in previous.into_iter().rev() {
            if let Some(value) = value {
                self.env_vars.insert(name.clone(), value.clone());
                set_process_env(&name, value);
            } else {
                self.env_vars.remove(&name);
                env::remove_var(name);
            }
        }
    }
}
