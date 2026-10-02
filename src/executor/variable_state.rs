use super::*;
use crate::executor::markers::DATA_DOLLAR;

impl Executor {
    /// shopt `expand_aliases` state — `pub` for the bin-side grouped-driver
    /// admission (rubash#414). GNU's non-interactive shell derives the
    /// parser's live flag as expand_aliases = posixly_correct
    /// (shell.c:1853 init_noninteractive); rubash folds that into the
    /// shopt flip itself (apply_posix_mode_transition), so this one
    /// predicate answers "aliases are live for the reader" in both modes.
    pub fn alias_expansion_enabled(&self) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_SHOPT_STATE")
            .is_some_and(|value| {
                value
                    .split(DATA_DOLLAR)
                    .any(|name| name == "expand_aliases")
            })
    }

    /// True while executing a command batch whose source text was already
    /// alias-expanded by the grouped script driver (lexer::alias_stream,
    /// GNU parse.y alias_expand_token + push_string). Executor-level word
    /// expansion must not expand those words a second time
    /// (`echo` -> `echo a` -> `echo a a`).
    pub(crate) fn alias_streamed(&self) -> bool {
        self.shell_state
            .env_vars
            .contains_key("__RUBASH_ALIAS_STREAMED")
    }

    /// An inner parse whose source is NOT pre-expanded (source files read
    /// incrementally like GNU's reader_loop) lifts the streamed marker for
    /// the duration and restores it afterwards.
    pub(crate) fn suspend_alias_streamed(&mut self) -> Option<String> {
        self.shell_state.env_vars.remove("__RUBASH_ALIAS_STREAMED")
    }

    /// Mark the current execution's source as already alias-expanded at the
    /// input-stream level (comsub_body_alias_splice / expand_group_aliases).
    /// Returns the previous marker for resume_alias_streamed.
    pub(crate) fn mark_alias_streamed(&mut self) -> Option<String> {
        self.shell_state
            .env_vars
            .insert("__RUBASH_ALIAS_STREAMED".to_string(), "1".to_string())
    }

    pub(crate) fn resume_alias_streamed(&mut self, saved: Option<String>) {
        match saved {
            Some(value) => {
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_ALIAS_STREAMED".to_string(), value);
            }
            None => {
                self.shell_state.env_vars.remove("__RUBASH_ALIAS_STREAMED");
            }
        }
    }

    pub(in crate::executor) fn apply_case_assignment_attributes(
        &self,
        name: &str,
        value: String,
    ) -> String {
        if is_marked_var(&self.shell_state.env_vars, UPPERCASE_VARS, name) {
            value.to_uppercase()
        } else if is_marked_var(&self.shell_state.env_vars, LOWERCASE_VARS, name) {
            value.to_lowercase()
        } else if is_marked_var(&self.shell_state.env_vars, CAPCASE_VARS, name) {
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
            NamerefResolution::Circular
            | NamerefResolution::MaxDepth
            | NamerefResolution::Unresolved
            | NamerefResolution::NotNameref => None,
        }
    }

    pub(in crate::executor) fn resolved_variable_name(&self, name: &str) -> Option<String> {
        match self.nameref_resolution(name) {
            NamerefResolution::Target(target) => Some(target),
            NamerefResolution::Circular | NamerefResolution::MaxDepth => None,
            // An unresolvable nameref cell still resolves the NAME to the
            // variable itself: find_variable_nameref_for_assignment
            // (variables.c:2210-2237) returns the nameref so `ref=x` binds
            // the cell and `unset ref` unbinds the nameref.
            NamerefResolution::Unresolved | NamerefResolution::NotNameref => Some(name.to_string()),
        }
    }

    pub(in crate::executor) fn nameref_resolution(&self, name: &str) -> NamerefResolution {
        // GNU variables.c:2011-2047 find_variable_nameref: level counts the
        // hops; level > NAMEREF_MAX (8, variables.h:181) reports
        // "maximum nameref depth exceeded" (MaxDepth here). A chain is only
        // "circular name reference" when the resolved variable is the
        // starting one (v == orig) or the one we just came from
        // (v == oldv); cycles that do not pass through orig keep looping
        // until the depth limit fires.
        let mut current = name.to_string();
        for level in 1..=9 {
            if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, &current) {
                return if current == name {
                    NamerefResolution::NotNameref
                } else {
                    NamerefResolution::Target(current)
                };
            }
            if level > 8 {
                return NamerefResolution::MaxDepth;
            }
            let Some(target) = self.shell_state.env_vars.get(&current) else {
                // Marked nameref with no cell entry at all.
                return NamerefResolution::Unresolved;
            };
            if target.is_empty()
                || (!is_shell_name(target) && parse_array_subscript(target).is_none())
            {
                // GNU variables.c:2023-2026 find_variable_nameref: an empty
                // or unresolvable cell returns NULL, so the nameref reads
                // as unset (e.g. `typeset -n ref` -> ${ref-unset} yields
                // "unset"), never as its own cell text.
                return NamerefResolution::Unresolved;
            }
            if target == name || target == &current {
                return NamerefResolution::Circular;
            }
            current = target.clone();
        }
        NamerefResolution::MaxDepth
    }

    /// GNU variables.c:2182 find_variable_nameref_for_create: returns the
    /// cell text of the LAST nameref in the chain when the chain stops on
    /// a missing/empty/invalid cell — the value `sh_invalidid` reports for
    /// `ref[k]=v` on an unassigned nameref (`': not a valid identifier`).
    pub(in crate::executor) fn last_nameref_cell(&self, name: &str) -> Option<String> {
        let mut current = name.to_string();
        for _ in 0..9 {
            if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, &current) {
                return None;
            }
            let cell = self
                .shell_state
                .env_vars
                .get(&current)
                .cloned()
                .unwrap_or_default();
            if cell.is_empty() || (!is_shell_name(&cell) && parse_array_subscript(&cell).is_none())
            {
                return Some(cell);
            }
            if cell == current || cell == name {
                return None;
            }
            current = cell;
        }
        None
    }

    /// GNU variables.c:2011 find_variable_nameref: when a nameref chain
    /// resolves back onto itself, the fallback is the GLOBAL variable of the
    /// name that closed the loop, searched without following namerefs
    /// ("XXX - provisional change - circular refs go to global scope for
    /// resolution, without namerefs", variables.c:2036-2046). This only
    /// applies inside a function context (`variable_context && v->context`);
    /// at the global scope the chain resolution returns nothing.
    pub(in crate::executor) fn nameref_circular_fallback_name(&self, name: &str) -> Option<String> {
        if self.shell_state.function_depth == 0 {
            return None;
        }
        let mut current = name;
        // Same level/circular accounting as nameref_resolution above:
        // NAMEREF_MAX=8 hops, circular only when the chain returns to the
        // start name or the variable just traversed (variables.c:2033).
        for _ in 0..8 {
            if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, current) {
                return None;
            }
            let target = self.shell_state.env_vars.get(current)?;
            if !is_shell_name(target) && parse_array_subscript(target).is_none() {
                return None;
            }
            if target == name || target == current {
                return Some(target.clone());
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
        // GNU find_global_variable_noref reads the binding at the GLOBAL
        // context. In rubash a shadowed global is the saved entry in the
        // OUTERMOST local scope that captured the name — a deeper scope's
        // snapshot holds an intervening local, and the live store slot is
        // the shadowing local itself (e.g. the `ref -> ref` cell, which is
        // why reading the live slot yields "ref" instead of the global).
        for typed_scope in &self.shell_state.local_typed_scopes {
            if let Some(saved) = typed_scope.get(&fallback) {
                return match saved {
                    Some(crate::shell::Variable {
                        value: crate::shell::ShellValue::Scalar(value),
                        ..
                    }) => Some(value.clone()),
                    _ => None,
                };
            }
        }
        // No scope shadowed the name: the live entry is the global binding.
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
    /// models the global binding as the OUTERMOST local-scope snapshot that
    /// captured the name (see circular_fallback_value); when no scope
    /// shadowed it the live store entry is the global one.
    pub(in crate::executor) fn assign_circular_fallback(
        &mut self,
        name: &str,
        value: String,
        append: bool,
    ) {
        let Some(fallback) = self.nameref_circular_fallback_name(name) else {
            return;
        };
        // GNU variables.c bind_global_variable -> bind_variable_internal:
        // the write goes through the variable's assign_func, so an array or
        // assoc namesake takes the value at element/key 0 rather than
        // collapsing to a scalar (nameref15.sub: local `a -> a`, `a=X`
        // leaves `declare -a a=([0]="X")`).
        // The marker sets are name-flat, so they describe the LOCAL shadow
        // (`local -n a` clears -a on `a`). The global namesake's attributes
        // are the saved VarAttrs in the outermost frame that localized the
        // name — the same frame holding its saved value.
        let saved_attrs = self
            .shell_state
            .local_var_scopes
            .iter()
            .position(|scope| scope.contains_key(&fallback))
            .and_then(|index| {
                self.shell_state
                    .local_attr_scopes
                    .get(index)
                    .and_then(|scope| scope.get(&fallback))
                    .copied()
                    .map(|attrs| (index, attrs))
            });
        if let Some((scope_index, attrs)) =
            saved_attrs.filter(|(_, attrs)| attrs.array || attrs.assoc)
        {
            let saved = self.shell_state.local_var_scopes[scope_index]
                .get(&fallback)
                .cloned()
                .flatten()
                .unwrap_or_default();
            let integer = attrs.integer;
            let updated = if attrs.assoc {
                let mut entries = crate::executor::assignment_helpers::assoc_entries(&saved);
                let existing = entries
                    .iter()
                    .rev()
                    .find_map(|(key, entry)| (key == "0").then_some(entry.clone()))
                    .unwrap_or_default();
                let element = if append && integer {
                    (self.eval_integer_assignment_value(&existing)
                        + self.eval_integer_assignment_value(&value))
                    .to_string()
                } else if append {
                    format!("{existing}{value}")
                } else if integer {
                    self.eval_integer_assignment_value(&value).to_string()
                } else {
                    value.clone()
                };
                match entries.iter_mut().rev().find(|(key, _)| key == "0") {
                    Some((_, entry)) => *entry = element,
                    None => entries.push(("0".to_string(), element)),
                }
                format!(
                    "({})",
                    entries
                        .into_iter()
                        .map(|(key, entry)| format!(
                            "[{}]={}",
                            crate::executor::assignment_helpers::quote_assoc_key(&key),
                            crate::executor::assignment_helpers::quote_assoc_storage_value(&entry)
                        ))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            } else {
                let mut entries = super::arrays::indexed_array_entries(&saved);
                let existing = entries.get(&0).cloned().unwrap_or_default();
                let element = if append && integer {
                    (self.eval_integer_assignment_value(&existing)
                        + self.eval_integer_assignment_value(&value))
                    .to_string()
                } else if append {
                    format!("{existing}{value}")
                } else if integer {
                    self.eval_integer_assignment_value(&value).to_string()
                } else {
                    value.clone()
                };
                entries.insert(0, element);
                super::arrays::format_indexed_array_storage(entries)
            };
            self.shell_state.local_var_scopes[scope_index].insert(fallback.clone(), Some(updated));
            return;
        }
        let mut variable = match self.circular_fallback_value(name) {
            Some(existing) => crate::shell::Variable::scalar(existing),
            None => crate::shell::Variable::scalar(String::new()),
        };
        let scalar = if append {
            let existing = self.circular_fallback_value(name).unwrap_or_default();
            format!("{existing}{value}")
        } else {
            value.clone()
        };
        variable.value = crate::shell::ShellValue::Scalar(scalar.clone());
        let mut wrote_snapshot = false;
        for typed_scope in &mut self.shell_state.local_typed_scopes {
            if typed_scope.contains_key(&fallback) {
                typed_scope.insert(fallback.clone(), Some(variable.clone()));
                wrote_snapshot = true;
                break;
            }
        }
        for scope in &mut self.shell_state.local_var_scopes {
            if scope.contains_key(&fallback) {
                scope.insert(fallback.clone(), Some(scalar));
                break;
            }
        }
        if !wrote_snapshot {
            let _ = self.shell_state.variables.set(fallback, variable);
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
            NamerefResolution::MaxDepth => {
                // GNU find_variable_nameref (variables.c:2022-2023): a chain
                // past NAMEREF_MAX resolves to nothing — the variable
                // expands unset after the depth warning.
                eprintln!(
                    "{}warning: {}: maximum nameref depth (8) exceeded",
                    self.diagnostic_prefix(),
                    name
                );
                return None;
            }
            // GNU: find_variable on an unresolvable nameref returns NULL —
            // the parameter is unset, not set-but-null (C3).
            NamerefResolution::Unresolved => return None,
            NamerefResolution::NotNameref => name.to_string(),
        };
        // GNU find_variable_nameref -> find_variable_internal resolves an
        // `arr[@]`/`arr[*]` cell to the array; scalar `${ref}` then reads
        // array_value's all-elements join (nameref18.sub `s=${ref}` yields
        // "1 2 3").
        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
        {
            if let Some(storage) = self.parameter_array_storage(array_name) {
                let values = if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
                    assoc_hash_ordered_values(
                        &storage,
                        assoc_nbuckets(&self.shell_state.env_vars, array_name),
                    )
                } else {
                    array_values(&storage)
                };
                return Some(values.join(&self.ifs_first_char_separator()));
            }
        }
        if let Some(value) = self.array_element_parameter_value(&name) {
            return Some(value);
        }
        // GNU semantics (variables.c array_value): once a variable has been
        // converted to an array (e.g. via `a[2]=bdef`), `${a}` is equivalent
        // to `${a[0]}`.  The scalar value in `shell_state.variables` is stale
        // and must not short-circuit the array lookup.  `unset a[0]` removes
        // element 0 from the array storage, so `${a}` must return empty.
        if is_marked_array_var(&self.shell_state.env_vars, &name) {
            return self
                .shell_state
                .env_vars
                .get(&name)
                .and_then(|value| self.scalar_parameter_value(&name, value));
        }
        // GNU builtins/declare.def:800-812: a valueless `declare
        // [-irux...] NAME` binds a NULL cell (variables.c:1912 var_isset is
        // `var->value != 0`) and sets att_invisible, so `${V+set}` /
        // `${V-word}` treat the name as unset until a real assignment binds
        // a value. The DECLARED_UNSET_VARS mark records that state; the
        // empty scalar materialized into the typed store by
        // sync_typed_attributes must not override it (rubash#409).
        if is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, &name)
            && !self.shell_state.env_vars.contains_key(&name)
        {
            return None;
        }
        if let Some(crate::shell::Variable {
            value: crate::shell::ShellValue::Scalar(value),
            ..
        }) = self.shell_state.variables.get(&name)
        {
            return Some(value.clone());
        }
        self.shell_state
            .env_vars
            .get(&name)
            .and_then(|value| self.scalar_parameter_value(&name, value))
    }

    pub(in crate::executor) fn scalar_parameter_value(
        &self,
        name: &str,
        value: &str,
    ) -> Option<String> {
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
            return assoc_value_at(value, "0");
        }
        if is_marked_array_var(&self.shell_state.env_vars, name) {
            return array_value_at(value, 0);
        }
        Some(value.to_string())
    }

    pub(in crate::executor) fn eval_integer_assignment_value(&self, value: &str) -> i128 {
        eval_conditional_arith_value(value, &self.shell_state.env_vars).unwrap_or(0)
    }

    pub(in crate::executor) fn mark_exported(&mut self, name: &str) {
        // VarTable keeps the structured att_exported bit and the serialized
        // EXPORTED_VARS list in sync in one call (variables.h:124-133).
        self.shell_state.env_vars.mark_name(EXPORTED_VARS, name);
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
            || (command == "eval" && cmd.assignment_keys().any(|name| name.ends_with('+')))
            || (self
                .shell_state
                .env_vars
                .get("__RUBASH_POSIX_MODE")
                .map(String::as_str)
                == Some("1")
                && (is_posix_special_builtin(command) || command == "source"))
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
        self.shell_state
            .env_vars
            .get("__RUBASH_POSIX_MODE")
            .map(String::as_str)
            == Some("1")
    }

    pub(in crate::executor) fn restore_temporary_assignments(
        &mut self,
        previous: Vec<(
            String,
            Option<String>,
            Option<crate::shell::Variable>,
            Option<VarAttrs>,
        )>,
    ) {
        if let Some(mark) = self.tempenv_marks.pop() {
            self.tempenv_names.truncate(mark);
        }
        // GNU variables.c:2615-2631 make_local_variable (was_tmpvar): a
        // declare/typeset/local operand bound by this command's prefix was
        // promoted to a frame local — its env binding must survive this
        // restore, and it keeps the tempvar's exported attribute.
        let promoted = std::mem::take(&mut self.tempenv_promoted_names);
        let mut deferred_attrs = Vec::new();
        for (name, value, typed_value, saved_attrs) in previous.into_iter().rev() {
            self.tempenv_previous.remove(&name);
            // GNU variables.c:4485-4525 push_posix_temp_var: a propagated
            // binding descended into the caller's context — the caller's
            // tempenv restore must not touch it, and it keeps the tempvar's
            // own attributes (v->attributes |= var->attributes).
            if let Some(attrs) = self
                .tempenv_propagated_names
                .iter()
                .find(|(propagated, _)| propagated == &name)
                .map(|(_, attrs)| attrs.clone())
            {
                deferred_attrs.push((name, attrs));
                continue;
            }
            if promoted.iter().any(|promoted_name| promoted_name == &name) {
                continue;
            }
            if crate::shell::var_table::VarAttrs::default()
                .flag(name.as_str())
                .is_some()
            {
                // A whole serialized attribute list (EXPORTED_VARS /
                // NAMEREF_VARS are tempenv entries themselves): restore the
                // string through VarTable so the structured attribute bits
                // follow it. The process-env sync the generic path does is
                // kept for parity with the pre-VarTable behavior (the
                // __RUBASH_* names ride the process environment to real
                // children).
                let process_value = value.clone();
                self.shell_state.env_vars.restore_attr_string(&name, value);
                if let Some(value) = process_value {
                    set_process_env(&name, value);
                } else {
                    env::remove_var(&name);
                }
                continue;
            }
            if let Some(value) = value {
                self.shell_state
                    .env_vars
                    .insert(name.clone(), value.clone());
                set_process_env(&name, value);
            } else {
                self.shell_state.env_vars.remove(&name);
                env::remove_var(&name);
            }
            self.restore_typed_temporary_value(&name, typed_value);
            // GNU variables.c pop_scope: the saved variable object — value
            // AND attributes — is reinstalled when the tempenv pops, so an
            // attribute gained mid-command (`a=7 f` where f runs
            // `readonly a`) does not leak onto the restored binding.
            if let Some(attrs) = saved_attrs {
                deferred_attrs.push((name, attrs));
            }
        }
        // The attribute-list env keys (__RUBASH_*_VARS) are tempenv entries
        // themselves whose whole-list restore ran above, so the per-name
        // attribute writes must go last or the list restore clobbers them.
        for (name, attrs) in deferred_attrs {
            set_var_attrs(&mut self.shell_state.env_vars, &name, attrs);
            // Only resync an existing typed cell — creating one for an unset
            // name would materialize `var=<unset>` bindings as `var=`.
            if self.shell_state.variables.get(&name).is_some() {
                crate::builtins::declare::sync_typed_attributes(
                    &[name],
                    &self.shell_state.env_vars,
                    &mut self.shell_state.variables,
                );
            }
        }
        for name in promoted {
            self.mark_exported(&name);
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
