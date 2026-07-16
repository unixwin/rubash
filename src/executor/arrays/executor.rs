use super::*;
use crate::executor::assoc_keys;

impl Executor {
    pub(in crate::executor) fn indexed_array_stack(&self, name: &str) -> Vec<String> {
        self.env_vars
            .get(name)
            .map(|value| array_values(value))
            .unwrap_or_default()
    }

    pub(in crate::executor) fn array_assignment_transform(&self, name: &str) -> String {
        let Some(value) = self.env_vars.get(name) else {
            return String::new();
        };

        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            let entries = assoc_entries(value);
            if entries.is_empty() {
                return format!("declare -A {name}");
            }
            let rendered = entries
                .into_iter()
                .map(|(key, value)| {
                    format!("[{}]={}", quote_assoc_key(&key), quote_array_value(&value))
                })
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -A {name}=({rendered} )");
        }

        if is_marked_array_var(&self.env_vars, name) || is_array_storage(value) {
            let rendered = indexed_array_entries(value)
                .into_iter()
                .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -a {name}=({rendered})");
        }

        String::new()
    }

    pub(in crate::executor) fn array_element_parameter_value(
        &self,
        expression: &str,
    ) -> Option<String> {
        let (array_name, key) = parse_array_subscript(expression)?;
        let storage_name = self.resolved_variable_name(array_name)?;
        let storage = self.parameter_array_storage(array_name)?;
        if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
            let key = self.assoc_subscript_key(key);
            return assoc_value_at(&storage, &key);
        }
        let key = strip_matching_quotes(&self.expand_embedded_parameters(key)).to_string();
        eval_conditional_arith_value(&key, &self.env_vars)
            .and_then(|index| resolve_indexed_array_subscript(&storage, index))
            .and_then(|index| array_value_at(&storage, index))
    }

    pub(in crate::executor) fn array_length(&self, name: &str) -> usize {
        if name == "GROUPS" {
            return self.groups_words().len();
        }
        self.parameter_array_storage(name)
            .map(|value| array_values(&value).len())
            .unwrap_or(0)
    }

    pub(in crate::executor) fn array_at_word_values(&self, word: &str) -> Option<Vec<String>> {
        let quoted_array_word =
            (word.starts_with('"') && word.ends_with('"')) || word.starts_with('\x1d');
        let word = word
            .strip_prefix('"')
            .and_then(|word| word.strip_suffix('"'))
            .unwrap_or(word);
        let word = word.strip_prefix('\x1d').unwrap_or(word);
        if quoted_array_word {
            if let Some(prefix) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("@}"))
            {
                let mut names = self
                    .env_vars
                    .keys()
                    .map(String::as_str)
                    .filter(|name| name.starts_with(prefix))
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                names.sort_unstable();
                return Some(names);
            }
            if let Some(name) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("[@]}"))
            {
                if is_noassign_bash_array(name)
                    || matches!(name, "BASH_ALIASES" | "BASH_CMDS" | "BASH_VERSINFO")
                {
                    return None;
                }
                let storage_name = self.resolved_variable_name(name)?;
                let storage = self.parameter_array_storage(name)?;
                if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
                    return Some(assoc_keys(&storage));
                }
                return Some(array_indices(&storage));
            }
        }
        let name = word.strip_prefix("${")?.strip_suffix("[@]}")?;
        if is_noassign_bash_array(name)
            || matches!(name, "BASH_ALIASES" | "BASH_CMDS" | "BASH_VERSINFO")
        {
            return None;
        }
        self.parameter_array_storage(name)
            .map(|value| array_values(&value))
    }
}
