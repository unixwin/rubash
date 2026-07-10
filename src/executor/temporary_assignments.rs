use super::*;

impl Executor {
    pub(in crate::executor) fn apply_temporary_assignments(
        &mut self,
        assignments: &HashMap<String, String>,
    ) -> Vec<(String, Option<String>)> {
        // TODO(execute_cmd.c/variables.c): Bash applies assignment words with
        // different persistence rules for special builtins, functions, POSIX
        // mode, and external command environments. For upstream builtins tests,
        // make prefix assignments visible while the command runs, then restore
        // the previous shell variable values.
        let mut previous = Vec::new();
        if !assignments.is_empty() {
            previous.push((
                EXPORTED_VARS.to_string(),
                self.env_vars.get(EXPORTED_VARS).cloned(),
            ));
        }
        for (name, value) in assignments {
            let expanded_value = self.expand_assignment_value(value);
            let (base_name, _) = assignment_name_and_append(name);
            previous.push((base_name.to_string(), self.env_vars.get(base_name).cloned()));
            self.apply_shell_assignment(name, expanded_value);
            self.mark_exported(base_name);
        }
        previous
    }

    pub(in crate::executor) fn apply_shell_assignment(
        &mut self,
        name: &str,
        value: String,
    ) -> bool {
        // TODO(variables.c/arrayfunc.c): Bash stores append assignment state
        // separately on WORD_DESC/ASSIGNMENT_WORD. This narrow path handles
        // scalar `name+=value` until SHELL_VAR attributes and arrays own it.
        let (base_name, append) = assignment_name_and_append(name);
        let target_name = match self.nameref_resolution(base_name) {
            NamerefResolution::Target(target) => target,
            NamerefResolution::Circular => {
                eprintln!(
                    "{}warning: {}: circular name reference",
                    self.diagnostic_prefix(),
                    base_name
                );
                self.exit_code = 1;
                return false;
            }
            NamerefResolution::NotNameref => base_name.to_string(),
        };
        let base_name = target_name.as_str();
        if is_marked_var(&self.env_vars, "__RUBASH_READONLY_VARS", base_name) {
            eprintln!(
                "{}{}: readonly variable",
                self.diagnostic_prefix(),
                base_name
            );
            self.exit_code = 1;
            return false;
        }
        if base_name == "OPTIND" && !append {
            self.env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        }
        if base_name == "SECONDS" && !append {
            let assigned = value.trim().parse::<i64>().unwrap_or(0);
            let start = self
                .env_vars
                .get(SHELL_START_EPOCH)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_else(current_epoch_seconds);
            let elapsed = current_epoch_seconds() - start;
            self.env_vars
                .insert(SECONDS_OFFSET.to_string(), (assigned - elapsed).to_string());
            set_process_env(base_name, assigned.to_string());
            return true;
        }
        if base_name == "RANDOM" && !append {
            self.random_state
                .set(value.trim().parse::<u32>().unwrap_or(0));
            set_process_env(base_name, value);
            return true;
        }
        if base_name == "BASHPID" && !append {
            return true;
        }
        if base_name == "BASH_SUBSHELL" && !append {
            return true;
        }
        if base_name == "FUNCNAME" && !append {
            return true;
        }
        if base_name == "LINENO" && !append {
            return true;
        }
        if base_name == "BASH_COMMAND" && !append {
            return true;
        }
        if is_noassign_bash_array(base_name) && !append {
            return true;
        }
        let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
        let value = value
            .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
            .unwrap_or(&value)
            .to_string();
        let value = if append {
            let current = self.env_vars.get(base_name).cloned().unwrap_or_default();
            if is_marked_var(&self.env_vars, ASSOC_VARS, base_name) {
                if value.starts_with('(') && value.ends_with(')') {
                    append_assoc_value(&current, &value)
                } else {
                    append_assoc_scalar_value(&current, &value)
                }
            } else if is_array_storage(&current)
                || is_marked_var(&self.env_vars, ARRAY_VARS, base_name)
            {
                append_array_value(
                    &current,
                    &value,
                    is_marked_var(&self.env_vars, INTEGER_VARS, base_name),
                    self.env_vars.get("IFS").map(String::as_str),
                )
            } else if is_marked_var(&self.env_vars, INTEGER_VARS, base_name) {
                let current = self.eval_integer_assignment_value(&current);
                let value = self.eval_integer_assignment_value(&value);
                (current + value).to_string()
            } else {
                append_scalar_value(&current, &value)
            }
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && is_marked_var(&self.env_vars, ASSOC_VARS, base_name)
        {
            append_assoc_value("()", &value)
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && is_marked_var(&self.env_vars, INTEGER_VARS, base_name)
        {
            self.eval_integer_assignment_value(&value).to_string()
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !is_marked_var(&self.env_vars, ASSOC_VARS, base_name)
        {
            append_array_value(
                "()",
                &value,
                is_marked_var(&self.env_vars, INTEGER_VARS, base_name),
                self.env_vars.get("IFS").map(String::as_str),
            )
        } else if is_marked_var(&self.env_vars, INTEGER_VARS, base_name) {
            self.eval_integer_assignment_value(&value).to_string()
        } else {
            value
        };
        let value = self.apply_case_assignment_attributes(base_name, value);
        if value.starts_with('\x1d') && !is_marked_var(&self.env_vars, ASSOC_VARS, base_name) {
            mark_env_name(&mut self.env_vars, ARRAY_VARS, base_name);
        }
        unmark_env_name(&mut self.env_vars, DECLARED_UNSET_VARS, base_name);
        self.env_vars.insert(base_name.to_string(), value.clone());
        set_process_env(base_name, value);
        true
    }
}
