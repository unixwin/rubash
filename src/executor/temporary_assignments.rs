use super::*;

impl Executor {
    /// Apply assignments from a command containing no command word. GNU Bash
    /// keeps these assignments in the current shell scope; they are not the
    /// temporary environment used by `name=value command`.
    pub(in crate::executor) fn apply_permanent_assignments(
        &mut self,
        assignments: &[(String, String)],
    ) {
        for (name, value) in assignments {
            let expanded_value = self.expand_assignment_value(value);
            self.apply_shell_assignment(name, expanded_value);
        }
    }

    pub(in crate::executor) fn apply_temporary_assignments(
        &mut self,
        assignments: &[(String, String)],
    ) -> Vec<(String, Option<String>, Option<crate::shell::Variable>)> {
        // TODO(execute_cmd.c/variables.c): Bash applies assignment words with
        // different persistence rules for special builtins, functions, POSIX
        // mode, and external command environments. For upstream builtins tests,
        // make prefix assignments visible while the command runs, then restore
        // the previous shell variable values (both the legacy env_vars value
        // and the typed shell_state.variables owner, so parameter expansion
        // does not keep seeing a leaked temporary value).
        let mut previous = Vec::new();
        if !assignments.is_empty() {
            previous.push((
                EXPORTED_VARS.to_string(),
                self.env_vars.get(EXPORTED_VARS).cloned(),
                self.shell_state.variables.get(EXPORTED_VARS).cloned(),
            ));
        }
        for (name, value) in assignments {
            let expanded_value = self.expand_assignment_value(value);
            let (base_name, _) = assignment_name_and_append(name);
            previous.push((
                base_name.to_string(),
                self.env_vars.get(base_name).cloned(),
                self.shell_state.variables.get(base_name).cloned(),
            ));
            // GNU variables.c bind_variable (ASS_NAMEREF path): a temporary
            // assignment to a nameref writes the referenced variable, so the
            // restore must also capture the target's previous value or the
            // referenced variable keeps the temporary value after the command.
            let resolved_target: Option<String> = match self.nameref_resolution(base_name) {
                NamerefResolution::Target(ref target) if *target != *base_name => Some(target.clone()),
                _ => None,
            };
            if let Some(ref target) = resolved_target {
                previous.push((
                    target.clone(),
                    self.env_vars.get(target).cloned(),
                    self.shell_state.variables.get(target).cloned(),
                ));
            }
            self.apply_shell_assignment(name, expanded_value);
            // GNU variables.c bind_variable (ASS_NAMEREF tempenv path): the
            // temporary assignment lands on the referenced variable and it is
            // that variable which is exported for the command, never the
            // nameref itself (nameref14.sub: `ref=xxx typeset -p ref var`
            // prints `declare -x var` while ref stays unexported).
            let export_target = resolved_target
                .clone()
                .unwrap_or_else(|| base_name.to_string());
            self.mark_exported(&export_target);
        }
        previous
    }


    /// GNU bind_variable with a nameref cell naming an array element: the
    /// value (and its integer evaluation when either the nameref or the
    /// referenced array carries the integer attribute) is written to that
    /// element of the referenced array (nameref23.sub: declare -in b="a[0]";
    /// b+=1 increments a[0]).
    fn apply_nameref_array_element_assignment(
        &mut self,
        elem_base: &str,
        subscript: &str,
        value: &str,
        append: bool,
        integer: bool,
    ) -> bool {
        let current = self.env_vars.get(elem_base).cloned().unwrap_or_default();
        if is_marked_var(&self.env_vars, ASSOC_VARS, elem_base) {
            let key = self.assoc_subscript_key(subscript);
            let mut entries = assoc_entries(&current);
            let existing = entries
                .iter()
                .rev()
                .find_map(|(entry_key, entry_value)| {
                    (entry_key == &key).then_some(entry_value.clone())
                })
                .unwrap_or_default();
            let element = if append {
                if integer {
                    // Real evaluator: resolves shell variables like GNU's
                    // expr.c evaluation (flix=9 -> 9, not the storage-shape 0).
                    (self.eval_integer_assignment_value(&existing)
                        + self.eval_integer_assignment_value(value))
                        .to_string()
                } else {
                    append_scalar_value(&existing, value)
                }
            } else if integer {
                self.eval_integer_assignment_value(value).to_string()
            } else {
                value.to_string()
            };
            if let Some((_, entry_value)) = entries
                .iter_mut()
                .rev()
                .find(|(entry_key, _)| entry_key == &key)
            {
                *entry_value = element;
            } else {
                entries.push((key, element));
            }
            let new_value = format!(
                "({})",
                entries
                    .into_iter()
                    .map(|(key, value)| {
                        format!("[{}]={}", quote_assoc_key(&key), quote_assoc_storage_value(&value))
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            self.env_vars.insert(elem_base.to_string(), new_value);
            self.exit_code = 0;
            return true;
        }
        let Ok(index) = self
            .eval_integer_assignment_value(subscript)
            .to_string()
            .parse::<usize>()
        else {
            self.exit_code = 1;
            return false;
        };
        let mut entries = indexed_array_entries(&current);
        let current_element = entries.get(&index).cloned().unwrap_or_default();
        let element = if append {
            if integer {
                (self.eval_integer_assignment_value(&current_element)
                    + self.eval_integer_assignment_value(value))
                    .to_string()
            } else {
                append_scalar_value(&current_element, value)
            }
        } else if integer {
            self.eval_integer_assignment_value(value).to_string()
        } else {
            value.to_string()
        };
        entries.insert(index, element);
        self.env_vars
            .insert(elem_base.to_string(), format_indexed_array_storage(entries));
        self.exit_code = 0;
        true
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
                // GNU writes each diagnostic with one write(2). eprintln!
                // fragments the message into one syscall per format piece and
                // those pieces race with stdout under the WSL interop relay,
                // splitting the message across unrelated lines; emit one
                // pre-formatted buffer instead.
                let line = format!(
                    "{}warning: {}: circular name reference\n",
                    self.diagnostic_prefix(),
                    base_name
                );
                let _ = std::io::stderr().write_all(line.as_bytes());
                // GNU variables.c:2036-2046 find_variable_nameref + the
                // bind_variable maxloop path: inside a function a circula
                // nameref assignment writes the GLOBAL namesake of the name
                // that closed the loop, while the local nameref keeps its
                // cell (nameref8.sub f1, nameref15.sub xxx_func).
                let circular_value = if append {
                    let current = self.circular_fallback_value(base_name).unwrap_or_default();
                    format!("{current}{value}")
                } else {
                    value.clone()
                };
                self.assign_circular_fallback(base_name, circular_value);
                self.exit_code = 0;
                return true;
            }
            NamerefResolution::NotNameref => base_name.to_string(),
        };
        // GNU variables.c bind_variable_internal: an assignment to a nameref
        // whose cell is empty or not a valid name is rejected with
        // sh_invalidid and the nameref is left unchanged
        // (nameref12.sub: r=^ / r=% against a valueless or invalid cell).
        if is_marked_var(&self.env_vars, NAMEREF_VARS, base_name) {
            let cell = self.env_vars.get(base_name).cloned().unwrap_or_default();
            let cell_valid = is_shell_name(&cell) || parse_array_subscript(&cell).is_some();
            if !append && !cell_valid {
                let offender = if cell.is_empty() { value.as_str() } else { cell.as_str() };
                let line = format!(
                    "{}`{offender}': not a valid identifier\n",
                    self.diagnostic_prefix()
                );
                let _ = std::io::stderr().write_all(line.as_bytes());
                self.exit_code = 1;
                return false;
            }
        }
        let base_name = target_name.as_str();
        // GNU arrayfunc.c/variables.c: a nameref whose cell is an array
        // element (declare -in b="a[0]"; b+=1) binds through to that element
        // of the referenced array instead of creating a variable literally
        // named a[0] (nameref23.sub).
        if let Some((elem_base, subscript)) = base_name.split_once('[') {
            if let Some(subscript) = subscript.strip_suffix(']') {
                if is_marked_var(&self.env_vars, ARRAY_VARS, elem_base)
                    || is_marked_var(&self.env_vars, ASSOC_VARS, elem_base)
                {
                    if is_marked_var(&self.env_vars, "__RUBASH_READONLY_VARS", elem_base) {
                        let line = format!(
                            "{}{}: readonly variable\n",
                            self.diagnostic_prefix(),
                            elem_base
                        );
                        let _ = std::io::stderr().write_all(line.as_bytes());
                        self.exit_code = 1;
                        return false;
                    }
                    let integer = is_marked_var(&self.env_vars, INTEGER_VARS, base_name)
                        || is_marked_var(&self.env_vars, INTEGER_VARS, elem_base);
                    return self.apply_nameref_array_element_assignment(
                        elem_base, subscript, &value, append, integer,
                    );
                }
            }
        }
        if is_marked_var(&self.env_vars, "__RUBASH_READONLY_VARS", base_name) {
            let line = format!(
                "{}{}: readonly variable\n",
                self.diagnostic_prefix(),
                base_name
            );
            let _ = std::io::stderr().write_all(line.as_bytes());
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
        if base_name == "SRANDOM" && !append {
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
                    append_assoc_value(
                        &current,
                        &value,
                        is_marked_var(&self.env_vars, INTEGER_VARS, base_name),
                        &self.env_vars,
                    )
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
                    &self.env_vars,
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
            for bare in assoc_bare_elements(&value) {
                eprintln!(
                    "{}{}: {}: must use subscript when assigning associative array",
                    self.diagnostic_prefix(),
                    base_name,
                    bare
                );
            }
            append_assoc_value(
                "()",
                &value,
                is_marked_var(&self.env_vars, INTEGER_VARS, base_name),
                &self.env_vars,
            )
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !is_marked_var(&self.env_vars, ASSOC_VARS, base_name)
            && is_marked_var(&self.env_vars, INTEGER_VARS, base_name)
            && integer_compound_assignment_is_scalar(&value)
        {
            // Bash keeps `typeset -i x; x=(1+2)` scalar.  A compound
            // assignment becomes an array only when it contains indexed o
            // multiple elements; the single arithmetic expression is still
            // assigned through the integer attribute.
            self.eval_integer_assignment_value(&value[1..value.len() - 1])
                .to_string()
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !is_marked_var(&self.env_vars, ASSOC_VARS, base_name)
        {
            // variables.c/arrayfunc.c: a compound `name=(...)` assignment
            // always makes an array, even when the variable previously had
            // the integer attribute (`typeset -i x; x=([0]=7+11)` becomes an
            // integer array with x[0]=18, not a scalar arithmetic result).
            append_array_value(
                "()",
                &value,
                is_marked_var(&self.env_vars, INTEGER_VARS, base_name),
                self.env_vars.get("IFS").map(String::as_str),
                &self.env_vars,
            )
        } else if is_marked_var(&self.env_vars, INTEGER_VARS, base_name) {
            self.eval_integer_assignment_value(&value).to_string()
        } else {
            value
        };
        let value = self.apply_case_assignment_attributes(base_name, value);
        let protocol_scalar = self.pending_scalar_assignment;
        self.pending_scalar_assignment = false;
        if value.starts_with('\x1d')
            && !protocol_scalar
            && !is_marked_var(&self.env_vars, ASSOC_VARS, base_name)
        {
            mark_env_name(&mut self.env_vars, ARRAY_VARS, base_name);
        }
        unmark_env_name(&mut self.env_vars, DECLARED_UNSET_VARS, base_name);
        let is_array = compound_assignment
            || is_marked_var(&self.env_vars, ARRAY_VARS, base_name)
            || is_marked_var(&self.env_vars, ASSOC_VARS, base_name);
        if !is_array {
            if let Some(variable) = self.shell_state.variables.get_mut(base_name) {
                if let crate::shell::ShellValue::Scalar(current) = &mut variable.value {
                    *current = value.clone();
                }
            } else {
                let _ = self
                    .shell_state
                    .variables
                    .set_scalar(base_name, value.clone());
            }
        }
        self.env_vars.insert(base_name.to_string(), value.clone());
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "allexport") {
            self.mark_exported(base_name);
        }
        sync_shell_assignment_process_env(&self.env_vars, base_name, value);
        true
    }
}

fn integer_compound_assignment_is_scalar(value: &str) -> bool {
    let Some(inner) = value.strip_prefix('(').and_then(|v| v.strip_suffix(')')) else {
        return false;
    };
    !inner.is_empty() && !inner.chars().any(|ch| ch.is_whitespace()) && !inner.contains(['[', ']'])
}
