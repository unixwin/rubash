use super::*;

impl Executor {
    pub(in crate::executor) fn dynamic_parameter_value(&self, name: &str) -> Option<String> {
        match name {
            "EPOCHSECONDS" => Some(current_epoch_seconds().to_string()),
            "EPOCHREALTIME" => {
                let micros = current_epoch_micros();
                Some(format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000))
            }
            "SECONDS" => {
                let start = self
                    .env_vars
                    .get(SHELL_START_EPOCH)
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or_else(current_epoch_seconds);
                let offset = self
                    .env_vars
                    .get(SECONDS_OFFSET)
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0);
                Some(
                    (current_epoch_seconds() - start + offset)
                        .max(0)
                        .to_string(),
                )
            }
            "RANDOM" => Some(self.next_random_value().to_string()),
            "SRANDOM" => Some(self.next_srandom_value().to_string()),
            "BASHPID" => Some(self.bashpid_value().to_string()),
            "BASH_SUBSHELL" => Some(self.subshell_depth.get().to_string()),
            "BASH_ARGV0" => Some(self.script_name_value()),
            "FUNCNAME" => Some(self.funcname_stack().first().cloned().unwrap_or_default()),
            "GROUPS" => self.group_value_at(0),
            "LINENO" => Some(
                self.env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .cloned()
                    .unwrap_or_else(|| "1".to_string()),
            ),
            "BASH_COMMAND" => Some(
                self.env_vars
                    .get("__RUBASH_CURRENT_COMMAND")
                    .cloned()
                    .unwrap_or_default(),
            ),
            "SHELLOPTS" => Some(crate::builtins::set::shellopts_value(&self.env_vars)),
            "BASHOPTS" => Some(crate::builtins::shopt::bashopts_value(&self.env_vars)),
            "PIPESTATUS" => self
                .env_vars
                .get("PIPESTATUS")
                .and_then(|value| array_value_at(value, 0))
                .or_else(|| Some("0".to_string())),
            _ => None,
        }
    }

    pub(in crate::executor) fn last_background_pid_value(&self) -> String {
        self.last_background_pid
            .map(|pid| pid.to_string())
            .unwrap_or_default()
    }

    pub(in crate::executor) fn dynamic_parameter_is_set(&self, name: &str) -> bool {
        matches!(
            name,
            "EPOCHSECONDS"
                | "EPOCHREALTIME"
                | "SECONDS"
                | "RANDOM"
                | "SRANDOM"
                | "BASHPID"
                | "BASH_SUBSHELL"
                | "BASH_ARGV0"
                | "FUNCNAME"
                | "GROUPS"
                | "LINENO"
                | "BASH_COMMAND"
                | "SHELLOPTS"
                | "BASHOPTS"
                | "PIPESTATUS"
        )
    }

    pub(in crate::executor) fn parameter_array_storage(&self, name: &str) -> Option<String> {
        let name = self.resolved_variable_name(name)?;
        let name = name.as_str();
        if name == "DIRSTACK" {
            return Some(self.dirstack_storage());
        }
        if name == "BASH_ALIASES" {
            return Some(self.bash_aliases_storage());
        }
        if name == "BASH_CMDS" {
            return Some(self.bash_cmds_storage());
        }
        self.env_vars.get(name).cloned()
    }

    pub(in crate::executor) fn is_assoc_parameter_array(&self, name: &str) -> bool {
        self.resolved_variable_name(name)
            .as_deref()
            .is_some_and(|name| is_marked_var(&self.env_vars, ASSOC_VARS, name))
    }

    pub(in crate::executor) fn dirstack_storage(&self) -> String {
        format_indexed_array_storage(
            crate::builtins::pushd::load_stack(&self.env_vars)
                .into_iter()
                .enumerate()
                .collect(),
        )
    }

    pub(in crate::executor) fn bashpid_value(&self) -> u32 {
        let pid = std::process::id();
        let depth = self.subshell_depth.get();
        if depth == 0 {
            pid
        } else {
            pid.saturating_add(u32::try_from(depth).unwrap_or(u32::MAX))
        }
    }

    pub(in crate::executor) fn bash_aliases_storage(&self) -> String {
        let mut entries: Vec<_> = self
            .aliases
            .iter()
            .map(|(name, alias)| (name.clone(), alias.value.clone()))
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        format_assoc_storage(entries)
    }

    pub(in crate::executor) fn bash_cmds_storage(&self) -> String {
        format_assoc_storage(crate::builtins::hash::hashed_entries(&self.env_vars))
    }

    pub(in crate::executor) fn sync_dynamic_assoc_vars(&mut self) {
        self.env_vars
            .insert("DIRSTACK".to_string(), self.dirstack_storage());
        mark_env_name(&mut self.env_vars, ARRAY_VARS, "DIRSTACK");
        self.env_vars
            .insert("BASH_ALIASES".to_string(), self.bash_aliases_storage());
        mark_env_name(&mut self.env_vars, ASSOC_VARS, "BASH_ALIASES");
        self.env_vars
            .insert("BASH_CMDS".to_string(), self.bash_cmds_storage());
        mark_env_name(&mut self.env_vars, ASSOC_VARS, "BASH_CMDS");
    }

    pub(in crate::executor) fn funcname_stack(&self) -> Vec<String> {
        self.env_vars
            .get("FUNCNAME")
            .map(|value| array_values(value))
            .unwrap_or_default()
    }

    pub(in crate::executor) fn restore_indexed_array(&mut self, name: &str, value: Option<String>) {
        match value {
            Some(value) => {
                self.env_vars.insert(name.to_string(), value);
            }
            None => {
                self.env_vars.insert(name.to_string(), String::new());
            }
        }
        mark_env_name(&mut self.env_vars, ARRAY_VARS, name);
    }

    pub(in crate::executor) fn current_bash_source(&self) -> String {
        self.env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .cloned()
            .unwrap_or_default()
    }

    pub(in crate::executor) fn next_random_value(&self) -> u32 {
        next_random_from_state(&self.random_state)
    }

    pub(in crate::executor) fn next_srandom_value(&self) -> u32 {
        next_srandom_from_state(&self.random_state)
    }

    pub(in crate::executor) fn script_name_value(&self) -> String {
        self.env_vars
            .get("BASH_ARGV0")
            .or_else(|| self.env_vars.get("__RUBASH_SCRIPT_NAME"))
            .cloned()
            .unwrap_or_else(|| "rubash".to_string())
    }

    pub(in crate::executor) fn groups_words(&self) -> Vec<String> {
        vec!["0".to_string()]
    }

    pub(in crate::executor) fn group_value_at(&self, index: usize) -> Option<String> {
        self.groups_words().get(index).cloned()
    }

    pub(in crate::executor) fn expand_declare_assignment_args(
        &mut self,
        args: &[String],
    ) -> Vec<String> {
        // TODO(builtins/declare.def/subst.c): `declare` and `typeset` perform
        // assignment-word RHS expansion before the builtin applies attributes.
        // General word expansion has already handled parameters and unquoted
        // tilde prefixes, so this bridge only removes Rubash's temporary quote
        // marker before declare.rs mirrors declare.def's bookkeeping.
        let mut expanded_args = Vec::new();
        for arg in args {
            let Some((name, value)) = split_assignment_word(arg) else {
                expanded_args.push(arg.clone());
                continue;
            };
            expanded_args.push(format!("{name}={}", self.expand_assignment_value(value)));
        }
        expanded_args
    }

    pub(in crate::executor) fn evaluate_declare_integer_assignment_args(
        &self,
        args: &[String],
    ) -> Vec<String> {
        args.iter()
            .map(|arg| {
                let Some((name, value)) = split_assignment_word(arg) else {
                    return arg.clone();
                };
                if value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                    || value.starts_with('(') && value.ends_with(')')
                {
                    return arg.clone();
                }
                format!("{name}={}", self.eval_integer_assignment_value(value))
            })
            .collect()
    }
}
