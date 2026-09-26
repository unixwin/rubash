use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    /// Apply assignments from a command containing no command word. GNU Bash
    /// keeps these assignments in the current shell scope; they are not the
    /// temporary environment used by `name=value command`.
    pub(in crate::executor) fn apply_permanent_assignments(
        &mut self,
        assignments: &[(String, String)],
    ) {
        for (name, value) in assignments {
            if std::env::var("RUBASH_DEBUG_ASSIGN").is_ok() {
                eprintln!("ASSIGN {name}={value:?}");
            }
            let expanded_value = self.expand_assignment_value(name, value);
            // GNU subst.c:10404+ expand_word_error -> DISCARD: a failed
            // assignment word (failglob no-match, readonly violation, ...)
            // abandons the rest of this command's assignment list.
            if !self.apply_shell_assignment(name, expanded_value) {
                // GNU subst.c:10404+ expand_word_error -> DISCARD: the failed
                // assignment reports status 1 and the evalerror abort armed
                // inside the apply path bounds the discard to this command
                // list.
                self.exit_code = 1;
                break;
            }
            // GNU variables.c make_variable_value: an integer-attribute
            // assignment that fails arithmetic evaluation (e.g. `i=0#4`
            // with `declare -i i`) reports evalerror and propagates exit
            // status 1. apply_shell_assignment resets exit_code to 0 on
            // success, so promote the arithmetic_expansion_error flag here.
            if self.shell_state.arithmetic_expansion_error.get() {
                self.shell_state.arithmetic_expansion_error.set(false);
                self.exit_code = 1;
            }
        }
    }

    /// GNU failglob path for `name=(...)` / `name+=(...)`: subst.c
    /// glob_expand_word_list reports `no match: WORD` and the expansion
    /// error aborts the assignment (execute_cmd.c returns status 1 via
    /// jump_to_top_level DISCARD — remaining same-line commands do not
    /// run). arrayfunc.c assign_array_var_from_string already created the
    /// array before expand_compound_array_assignment ran, so a brand-new
    /// target is left bound as an empty indexed array (`a=(zzz-*)` then
    /// `declare -p a` prints `declare -a a=()`), while a target that
    /// already existed — including a declared-but-unset `declare -a f1` —
    /// keeps its prior state (niubash #121). Returning false propagates
    /// the failure through apply_shell_assignment's callers, which map it
    /// to ExpansionFailure(1) — the same DISCARD contract.
    fn fail_compound_array_assignment(&mut self, base_name: &str, pattern: &str) -> bool {
        self.report_failglob(pattern);
        if !self.shell_state.env_vars.contains_key(base_name)
            && !is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, base_name)
        {
            self.shell_state.env_vars.insert(
                base_name.to_string(),
                format_indexed_array_storage(BTreeMap::new()),
            );
            mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, base_name);
        }
        false
    }

    pub(in crate::executor) fn apply_temporary_assignments(
        &mut self,
        assignments: &[(String, String)],
    ) -> Vec<(
        String,
        Option<String>,
        Option<crate::shell::Variable>,
        Option<VarAttrs>,
    )> {
        // TODO(execute_cmd.c/variables.c): Bash applies assignment words with
        // different persistence rules for special builtins, functions, POSIX
        // mode, and external command environments. For upstream builtins tests,
        // make prefix assignments visible while the command runs, then restore
        // the previous shell variable values (both the legacy env_vars value
        // and the typed shell_state.variables owner, so parameter expansion
        // does not keep seeing a leaked temporary value).
        self.tempenv_marks.push(self.tempenv_names.len());
        // A promoted tempenv local survives the command-end restore, so any
        // names left promoted by a command whose restore was deferred
        // (keep_temporary_assignments) must not skip a later mark's restore.
        self.tempenv_promoted_names.clear();
        let mut previous = Vec::new();
        if !assignments.is_empty() {
            previous.push((
                EXPORTED_VARS.to_string(),
                self.shell_state.env_vars.get(EXPORTED_VARS).cloned(),
                self.shell_state.variables.get(EXPORTED_VARS).cloned(),
                None,
            ));
            previous.push((
                NAMEREF_VARS.to_string(),
                self.shell_state.env_vars.get(NAMEREF_VARS).cloned(),
                self.shell_state.variables.get(NAMEREF_VARS).cloned(),
                None,
            ));
        }
        // GNU findcmd.c:356-365: a PATH in the temporary command environment
        // (PATH=foo cmd) bypasses the hash table entirely. Rubash's lookup
        // cache keys on a PATH fingerprint, which already prevents temp-PATH
        // results from polluting the normal cache, but GNU also skips the
        // hash read so a stale remembered path is never returned for a
        // temp-PATH command. Tag the temp-PATH state here so find_user_command
        // can bypass the cache; the tag is cleared in restore.
        let has_temp_path = assignments.iter().any(|(name, _)| {
            let base = name.split('[').next().unwrap_or(name);
            base == "PATH"
        });
        if has_temp_path {
            previous.push((
                "__RUBASH_TEMP_PATH".to_string(),
                self.shell_state.env_vars.get("__RUBASH_TEMP_PATH").cloned(),
                self.shell_state
                    .variables
                    .get("__RUBASH_TEMP_PATH")
                    .cloned(),
                None,
            ));
            self.shell_state
                .env_vars
                .insert("__RUBASH_TEMP_PATH".to_string(), "1".to_string());
        }
        for (name, value) in assignments {
            let expanded_value = self.expand_assignment_value(name, value);
            let (base_name, _) = assignment_name_and_append(name);
            let saved_env = self.shell_state.env_vars.get(base_name).cloned();
            let saved_typed = self.shell_state.variables.get(base_name).cloned();
            let saved_attrs = capture_var_attrs(&self.shell_state.env_vars, base_name);
            self.tempenv_previous.insert(
                base_name.to_string(),
                (saved_env.clone(), saved_typed.clone(), saved_attrs.clone()),
            );
            previous.push((
                base_name.to_string(),
                saved_env,
                saved_typed,
                Some(saved_attrs),
            ));
            // GNU variables.c bind_variable (ASS_NAMEREF path): a temporary
            // assignment to a nameref writes the referenced variable, so the
            // restore must also capture the target's previous value or the
            // referenced variable keeps the temporary value after the command.
            let resolved_target: Option<String> = match self.nameref_resolution(base_name) {
                NamerefResolution::Target(ref target) if *target != *base_name => {
                    Some(target.clone())
                }
                _ => None,
            };
            if let Some(ref target) = resolved_target {
                previous.push((
                    target.clone(),
                    self.shell_state.env_vars.get(target).cloned(),
                    self.shell_state.variables.get(target).cloned(),
                    Some(capture_var_attrs(&self.shell_state.env_vars, target)),
                ));
            }
            // GNU variables.c:3564-3578 assign_in_env: when the name does not
            // resolve (find_variable NULL — e.g. a nameref with an empty or
            // invalid cell), the tempenv binding falls back to the literal
            // name as a plain exported variable; the nameref attribute does
            // not apply inside the temporary environment
            // (nameref11.sub: `declare -n r; r=/ f` shows `declare -x r="/"`
            // inside f and restores the empty nameref afterwards). A readonly
            // original still rejects the binding via ASSIGN_DISALLOWED.
            if resolved_target.is_none()
                && is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base_name)
            {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, base_name) {
                    let line = format!(
                        "{}{base_name}: readonly variable
",
                        self.assignment_diagnostic_prefix()
                    );
                    self.emit_assignment_diag(line);
                    continue;
                }
                self.shell_state
                    .env_vars
                    .insert(base_name.to_string(), expanded_value.clone());
                let _ = self.shell_state.variables.set(
                    base_name.to_string(),
                    crate::shell::Variable::scalar(expanded_value.clone()),
                );
                unmark_env_name(&mut self.shell_state.env_vars, NAMEREF_VARS, base_name);
                self.tempenv_names.push(base_name.to_string());
                self.mark_exported(base_name);
                continue;
            }
            // GNU variables.c assign_in_env: a compound `name=(...)` or
            // `name+=(...)` word in a command's temporary environment binds
            // the literal list text as a scalar — the element words are not
            // re-parsed and pathname expansion never runs on them, so
            // `a=(zzz-*) declare -p a` prints `declare -x a="(zzz-nomatch-*)"`
            // even under failglob (niubash #121). Route it through the same
            // exported-scalar binding the nameref fallback above uses.
            if let Some(compound) = expanded_value.strip_prefix(COMPOUND_ASSIGNMENT_MARKER) {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, base_name) {
                    let line = format!(
                        "{}{base_name}: readonly variable\n",
                        self.assignment_diagnostic_prefix()
                    );
                    self.emit_assignment_diag(line);
                    continue;
                }
                self.shell_state
                    .env_vars
                    .insert(base_name.to_string(), compound.to_string());
                let _ = self.shell_state.variables.set(
                    base_name.to_string(),
                    crate::shell::Variable::scalar(compound.to_string()),
                );
                self.tempenv_names.push(base_name.to_string());
                self.mark_exported(base_name);
                continue;
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
            self.tempenv_names.push(export_target.clone());
            self.mark_exported(&export_target);
        }
        previous
    }

    /// GNU bind_variable with a nameref cell naming an array element: the
    /// value (and its integer evaluation when either the nameref or the
    /// referenced array carries the integer attribute) is written to that
    /// element of the referenced array (nameref23.sub: declare -in b="a[0]";
    /// b+=1 increments a[0]).
    pub(in crate::executor) fn apply_nameref_array_element_assignment(
        &mut self,
        elem_base: &str,
        subscript: &str,
        value: &str,
        append: bool,
        integer: bool,
        subscript_from_operand: bool,
        operand_is_funcenv_nameref: bool,
    ) -> bool {
        // GNU arrayfunc.c:268-275 bind_array_variable ->
        // variables.c:2182 find_variable_nameref_for_create: an element
        // assignment whose OPERAND base is a nameref (`ref[0]=v`) creates
        // the array at the cell's name -- the cell must be a bare
        // identifier (valid_identifier, so `x[i]` cells are rejected too).
        // An empty or invalid cell fails sh_invalidid and leaves the
        // nameref untouched (nameref12.sub: `typeset -n ref; ref[0]=foo`
        // reports `': not a valid identifier` and keeps `declare -n ref`).
        if subscript_from_operand
            && is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, elem_base)
        {
            let cell = self
                .shell_state
                .env_vars
                .get(elem_base)
                .cloned()
                .unwrap_or_default();
            if !is_shell_name(&cell) {
                let line = format!(
                    "{}`{cell}': not a valid identifier
",
                    self.assignment_diagnostic_prefix()
                );
                self.emit_assignment_diag(line);
                self.exit_code = 1;
                return false;
            }
            return self.apply_nameref_array_element_assignment(
                &cell, subscript, value, append, integer, false, false,
            );
        }
        // GNU variables.c:3241-3280 bind_variable walks function contexts
        // first: a cell array-reference reached through a funcenv nameref
        // goes straight to assign_array_element, whose bind_array_variable
        // -> find_variable_nameref_for_create (variables.c:2182) requires
        // the last nameref's cell to be a bare identifier. `a[0]` is not,
        // so `f() { local -n a=a[0]; a=X; }` fails sh_invalidid and keeps
        // the local nameref (nameref15.sub:14). Only the global-table path
        // (bind_variable_internal, variables.c:3085) removes the attribute.
        if !subscript_from_operand
            && operand_is_funcenv_nameref
            && is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, elem_base)
        {
            let line = format!(
                "{}`{elem_base}[{subscript}]': not a valid identifier\n",
                self.assignment_diagnostic_prefix()
            );
            self.emit_assignment_diag(line);
            self.exit_code = 1;
            return false;
        }
        // GNU arrayfunc.c:464-475 find_or_make_array_variable: when the
        // variable being array-ified is itself a nameref, the attribute is
        // removed with a warning and the cell text is dropped -- it never
        // becomes element 0 (nameref15.sub: `typeset -n a=b b; b=a[1];
        // a=foo` leaves `declare -a a=([1]="foo")`, not a nameref or an
        // array holding "b").
        let current = if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, elem_base) {
            let line = format!(
                "{}warning: {elem_base}: removing nameref attribute
",
                self.assignment_diagnostic_prefix()
            );
            self.emit_assignment_diag(line);
            unmark_env_name(&mut self.shell_state.env_vars, NAMEREF_VARS, elem_base);
            String::new()
        } else {
            self.shell_state
                .env_vars
                .get(elem_base)
                .cloned()
                .unwrap_or_default()
        };
        // GNU SET_VFLAGS provenance (builtins/common.h:277-289): a subscript
        // arriving inside the builtin operand (`read a[$x]`) is ExpandedOnce
        // data — verbatim with array_expand_once, one deferred
        // expand_subscript_string pass without it. A subscript arriving via a
        // nameref CELL (`declare -n r='a[$x]'; r=v`) is raw stored text that
        // GNU expands at bind time (variables.c bind_variable ->
        // assign_array_element -> expand_array_index), so it stays Raw.
        let subscript_source = if subscript_from_operand {
            SubscriptSource::ExpandedOnce(subscript)
        } else {
            SubscriptSource::Raw(subscript)
        };
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, elem_base) {
            // Same \x1d bookkeeping trim assoc_subscript_key applies.
            let key = self
                .resolve_array_subscript(subscript_source)
                .trim_matches(STORAGE_WORD_PREFIX)
                .to_string();
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
                    // evalerror here DISCARDs like any make_variable_value
                    // failure (variables.c:2920-2952).
                    let Some(existing) = self.eval_integer_assignment_checked(&existing) else {
                        return false;
                    };
                    let Some(value) = self.eval_integer_assignment_checked(value) else {
                        return false;
                    };
                    (existing + value).to_string()
                } else {
                    append_scalar_value(&existing, value)
                }
            } else if integer {
                match self.eval_integer_assignment_checked(value) {
                    Some(result) => result.to_string(),
                    None => return false,
                }
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
                        format!(
                            "[{}]={}",
                            quote_assoc_key(&key),
                            quote_assoc_storage_value(&value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            self.shell_state
                .env_vars
                .insert(elem_base.to_string(), new_value);
            self.exit_code = 0;
            return true;
        }
        // arrayfunc.c array_expand_index -> evalexp under no_expand rules:
        // a surviving $name/$(...) in an ExpandedOnce operand fails
        // "operand expected" (diagnostic already printed) instead of
        // executing a second time.
        let index = match self.eval_indexed_subscript(subscript_source) {
            IndexedSubscript::Index(index) => {
                match resolve_indexed_array_subscript(&current, index) {
                    Some(index) => index,
                    // GNU assign_array_element_internal: an out-of-range
                    // negative index is a bad subscript (arrayfunc.c).
                    None => {
                        self.report_bad_array_subscript(&format!("{elem_base}[{index}]"));
                        self.exit_code = 1;
                        return false;
                    }
                }
            }
            IndexedSubscript::Empty => {
                self.report_bad_array_subscript(&format!("{elem_base}[]"));
                self.exit_code = 1;
                return false;
            }
            IndexedSubscript::Error => {
                self.exit_code = 1;
                return false;
            }
        };
        let mut entries = indexed_array_entries(&current);
        let current_element = entries.get(&index).cloned().unwrap_or_default();
        let element = if append {
            if integer {
                let Some(current_element) = self.eval_integer_assignment_checked(&current_element)
                else {
                    return false;
                };
                let Some(value) = self.eval_integer_assignment_checked(value) else {
                    return false;
                };
                (current_element + value).to_string()
            } else {
                append_scalar_value(&current_element, value)
            }
        } else if integer {
            match self.eval_integer_assignment_checked(value) {
                Some(result) => result.to_string(),
                None => return false,
            }
        } else {
            value.to_string()
        };
        entries.insert(index, element);
        self.shell_state
            .env_vars
            .insert(elem_base.to_string(), format_indexed_array_storage(entries));
        self.exit_code = 0;
        true
    }

    /// GNU variables.c:2920-2952 make_variable_value: an integer-attribute
    /// assignment evaluates the word through evalexp — for `name+=value` the
    /// CURRENT cell is evaluated first (ksh93 semantics), then the rhs. A
    /// failure runs evalerror and, without ASS_NOLONGJMP (only internal
    /// rebinds set it), jump_to_top_level(DISCARD): the variable keeps its
    /// old value and the rest of the command list is discarded
    /// (`declare -i x; x=4+; x+=7; echo after` prints the diagnostic, leaves
    /// x alone, and never reaches `echo after`). Returns None on failure
    /// after reporting and arming the evalerror abort.
    fn eval_integer_assignment_checked(&mut self, value: &str) -> Option<i128> {
        if let Some(result) = crate::executor::arithmetic::eval_conditional_arith_value(
            value,
            &self.shell_state.env_vars,
        ) {
            return Some(result);
        }
        let message =
            crate::executor::arithmetic::take_arith_eval_error().map(|record| record.render(true));
        let message = message.or_else(|| {
            crate::executor::arithmetic::arithmetic_error_message(
                value,
                false,
                &self.shell_state.env_vars,
            )
        });
        if let Some(message) = message {
            let line = format!("{}{}\n", self.assignment_diagnostic_prefix(), message);
            self.emit_assignment_diag(line);
        }
        // Do NOT set arithmetic_expansion_error here: every caller maps the
        // None return to `false`, so the flag has no promotion consumer and
        // would survive into the NEXT command's expand_assignment_value_result
        // (assignment_expansion.rs:295), silently aborting it as if its own
        // expansion had failed (`declare -i i; i=0#4\ni=3+` lost the `3+`
        // diagnostic). The DISCARD abort is already armed by
        // raise_evalerror_abort (GNU expr.c evalerror -> jump_to_top_level).
        self.raise_evalerror_abort();
        None
    }

    /// GNU builtins call bind_variable while `this_command_name` is the
    /// builtin's name, so assignment diagnostics carry the `name:` segment
    /// (`getopts: `?': not a valid identifier`, `declare: x: readonly
    /// variable`). Bare `name=value` commands have no command segment.
    pub(in crate::executor) fn apply_shell_assignment_command(
        &mut self,
        command: &str,
        name: &str,
        value: String,
    ) -> bool {
        let previous = self.assignment_command_name.replace(command.to_string());
        let result = self.apply_shell_assignment(name, value);
        self.assignment_command_name = previous;
        result
    }

    /// emit_assignment_diag routes through the builtin's buffered stderr when
    /// a caller opted in (GNU builtin_error ordering/redirection), otherwise
    /// writes process stderr directly like eprintln!.
    fn emit_assignment_diag(&mut self, line: String) {
        if self.buffer_assignment_diagnostics {
            self.pending_assignment_diagnostics
                .extend_from_slice(line.as_bytes());
        } else {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }

    fn assignment_diagnostic_prefix(&self) -> String {
        match &self.assignment_command_name {
            Some(command) => format!("{}{command}: ", self.diagnostic_prefix()),
            None => self.diagnostic_prefix(),
        }
    }

    pub(in crate::executor) fn apply_shell_assignment(
        &mut self,
        name: &str,
        value: String,
    ) -> bool {
        let result = self.apply_shell_assignment_inner(name, value);
        // GNU variables.c:5761 { "BASH_XTRACEFD", sv_xtracefd }: assigning
        // the variable retargets xtrace (or reports an invalid fd) through
        // the bind hook, independent of which builtin stored the value.
        let base = assignment_name_and_append(name).0;
        if result && base == "BASH_XTRACEFD" {
            self.apply_xtracefd_assignment();
        }
        result
    }

    fn apply_shell_assignment_inner(&mut self, name: &str, value: String) -> bool {
        // TODO(variables.c/arrayfunc.c): Bash stores append assignment state
        // separately on WORD_DESC/ASSIGNMENT_WORD. This narrow path handles
        // scalar `name+=value` until SHELL_VAR attributes and arrays own it.
        let (base_name, append) = assignment_name_and_append(name);
        // A subscript inside the operand itself (`read a[$x]`, `a[$x]=v`
        // reaching this path) is ExpandedOnce argv data; a subscript that
        // arrives via a nameref cell is raw stored text GNU expands at bind
        // time.
        let (target_name, subscript_from_operand) = match self.nameref_resolution(base_name) {
            NamerefResolution::Target(target) => (target, false),
            NamerefResolution::Circular => {
                // GNU bind_variable -> find_variable_nameref_context: the
                // within-context chain walk loops until NAMEREF_MAX, so the
                // WRITE diagnostic is "maximum nameref depth" — distinct
                // from the read path's "circular name reference"
                // (variables.c:3274, find_nameref_at_context maxloop). The
                // write then lands on bind_global_variable: the global
                // namesake of the loop-closing name, leaving the local
                // nameref cell intact (nameref15.sub: `r2+=X` with local
                // `r2 -> r2` writes global r2).
                if self.nameref_circular_fallback_name(base_name).is_some() {
                    let line = format!(
                        "{}warning: {}: maximum nameref depth (8) exceeded\n",
                        self.assignment_diagnostic_prefix(),
                        base_name
                    );
                    self.emit_assignment_diag(line);
                    self.assign_circular_fallback(base_name, value.clone(), append);
                    return true;
                }
                // At global scope find_variable_nameref's circular branch
                // has no context fallback (variables.c:2039), so the read
                // warning fires and the assignment fails.
                let line = format!(
                    "{}warning: {}: circular name reference\n",
                    self.assignment_diagnostic_prefix(),
                    base_name
                );
                self.emit_assignment_diag(line);
                return false;
            }
            NamerefResolution::MaxDepth => {
                // GNU variables.c:2220 find_variable_nameref_for_assignment:
                // depth overflow returns INVALID_NAMEREF_VALUE after the
                // internal_warning, so the assignment fails with status 1.
                let line = format!(
                    "{}warning: {}: maximum nameref depth (8) exceeded\n",
                    self.assignment_diagnostic_prefix(),
                    base_name
                );
                self.emit_assignment_diag(line);
                return false;
            }
            // Unresolved cell: resolve to the nameref itself; the
            // empty-cell binding block below assigns the new target. Like
            // NotNameref, any subscript here came from the operand.
            NamerefResolution::Unresolved | NamerefResolution::NotNameref => {
                (base_name.to_string(), true)
            }
        };
        // GNU arrayfunc.c:454 find_or_make_array_variable ->
        // variables.c:2182 find_variable_nameref_for_create requires a bare
        // identifier (valid_identifier, not valid_nameref_value(...,1)):
        // a compound `ref=(...)`/`ref+=(...)` whose nameref cell is an array
        // reference like `XXX[0]` fails sh_invalidid, while scalar `ref=v`
        // forwards to the element (assign_array_element, variables.c:3267).
        if value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
            && parse_array_subscript(&target_name).is_some()
        {
            let line = format!(
                "{}`{target_name}': not a valid identifier
",
                self.assignment_diagnostic_prefix()
            );
            self.emit_assignment_diag(line);
            self.exit_code = 1;
            return false;
        }
        // GNU variables.c bind_variable_internal: when a nameref has an
        // empty cell (valueless, created by `declare -n name` without a
        // value), an assignment with a valid shell name or array subscript
        // sets the nameref target; an invalid value is rejected with
        // sh_invalidid.  A nameref whose cell is already invalid (not
        // empty, not a valid name) is left unchanged on any assignment
        // (nameref12.sub: r=^ against an invalid cell).
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base_name) {
            let cell = self
                .shell_state
                .env_vars
                .get(base_name)
                .cloned()
                .unwrap_or_default();
            let cell_valid = is_shell_name(&cell) || parse_array_subscript(&cell).is_some();
            if !append && !cell_valid {
                // Distinguish valueless (empty) from already-invalid cells.
                let value_valid = is_shell_name(value.as_str())
                    || parse_array_subscript(value.as_str()).is_some();
                if cell.is_empty() && value_valid {
                    // Valueless nameref: set the target to the new value.
                    self.shell_state
                        .env_vars
                        .insert(base_name.to_string(), value.clone());
                    self.exit_code = 0;
                    return true;
                }
                let offender = if cell.is_empty() {
                    value.as_str()
                } else {
                    cell.as_str()
                };
                let line = format!(
                    "{}`{offender}': not a valid identifier\n",
                    self.assignment_diagnostic_prefix()
                );
                self.emit_assignment_diag(line);
                self.exit_code = 1;
                return false;
            }
        }
        // GNU variables.c:3241 bind_variable: a SCALAR operand whose name
        // is a nameref in a live function context resolves through
        // find_variable_nameref_context, whose array-reference cell then
        // goes straight to assign_array_element (no attribute strip --
        // unlike the global bind_variable_internal path).
        let operand_is_funcenv_nameref =
            is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base_name)
                && self
                    .shell_state
                    .local_var_scopes
                    .iter()
                    .any(|scope| scope.contains_key(base_name));
        let base_name = target_name.as_str();
        // GNU arrayfunc.c/variables.c: a nameref whose cell is an array
        // element (declare -in b="a[0]"; b+=1) binds through to that element
        // of the referenced array instead of creating a variable literally
        // named a[0] (nameref23.sub).
        // element (declare -in b="a[0]"; b+=1) binds through to that element
        // of the referenced array instead of creating a variable literally
        // named a[0] (nameref23.sub).
        if let Some((elem_base, subscript)) = base_name.split_once('[') {
            if let Some(subscript) = subscript.strip_suffix(']') {
                // GNU builtins/common.c:949 builtin_bind_variable: any valid
                // array reference (name[subscript]) goes through
                // assign_array_element -> find_or_make_array_variable,
                // which creates the array on demand. This applies to `read
                // x[1]` (read.def:1151 bind_read_variable) and direct
                // `x[1]=value` assignments alike, even when the variable is
                // not previously declared as an array (array.tests:80).
                if is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, elem_base)
                    || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, elem_base)
                {
                    if is_marked_var(
                        &self.shell_state.env_vars,
                        "__RUBASH_READONLY_VARS",
                        elem_base,
                    ) {
                        let line = format!(
                            "{}{}: readonly variable\n",
                            self.diagnostic_prefix(),
                            elem_base
                        );
                        self.emit_assignment_diag(line);
                        self.exit_code = 1;
                        return false;
                    }
                    let integer =
                        is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name)
                            || is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, elem_base);
                    return self.apply_nameref_array_element_assignment(
                        elem_base,
                        subscript,
                        &value,
                        append,
                        integer,
                        subscript_from_operand,
                        operand_is_funcenv_nameref,
                    );
                }
                // GNU arrayfunc.c:334-360 assign_array_element: the element
                // reference must be name[subscript] with a non-empty,
                // non-@/* subscript and a valid base -- otherwise
                // err_badarraysub (`var[@]': bad array subscript for a
                // nameref cell like `var[@]`, nameref15.sub:78) or
                // sh_invalidid on the cell text (`a-b[0]').
                if !is_shell_name(elem_base) {
                    let line = format!(
                        "{}`{}': not a valid identifier
",
                        self.assignment_diagnostic_prefix(),
                        base_name
                    );
                    self.emit_assignment_diag(line);
                    self.exit_code = 1;
                    return false;
                }
                if subscript.is_empty() || subscript == "@" || subscript == "*" {
                    let line = format!(
                        "{}{}: bad array subscript
",
                        self.assignment_diagnostic_prefix(),
                        base_name
                    );
                    self.emit_assignment_diag(line);
                    self.exit_code = 1;
                    return false;
                }
                {
                    if is_marked_var(
                        &self.shell_state.env_vars,
                        "__RUBASH_READONLY_VARS",
                        elem_base,
                    ) {
                        // GNU error.c:453 err_readonly -> report_error: the
                        // bind-layer diagnostic is `name: readonly variable`
                        // with no builtin-name segment (read.def:1155
                        // bind_read_variable -> variables.c bind_variable).
                        let line = format!(
                            "{}{}: readonly variable\n",
                            self.diagnostic_prefix(),
                            elem_base
                        );
                        self.emit_assignment_diag(line);
                        self.exit_code = 1;
                        return false;
                    }
                    let integer =
                        is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name)
                            || is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, elem_base);
                    return self.apply_nameref_array_element_assignment(
                        elem_base,
                        subscript,
                        &value,
                        append,
                        integer,
                        subscript_from_operand,
                        operand_is_funcenv_nameref,
                    );
                }
            }
        }
        if is_marked_var(
            &self.shell_state.env_vars,
            "__RUBASH_READONLY_VARS",
            base_name,
        ) {
            let line = format!(
                "{}{}: readonly variable\n",
                self.diagnostic_prefix(),
                base_name
            );
            self.emit_assignment_diag(line);
            self.exit_code = 1;
            return false;
        }
        if base_name == "OPTIND" && !append {
            self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        }
        // GNU variables.c:6205-6217 sv_ignoreeof (the IGNOREEOF/ignoreeof
        // special-variable hook): assigning the variable turns the
        // ignoreeof option on — the option is "the variable is set", so
        // even `IGNOREEOF=` enables it.
        if matches!(base_name, "IGNOREEOF" | "ignoreeof") && !append {
            crate::builtins::set::sync_shell_option_flag(
                &mut self.shell_state.env_vars,
                "ignoreeof",
                true,
            );
        }
        if base_name == "SECONDS" && !append {
            let assigned = value.trim().parse::<i64>().unwrap_or(0);
            let start = self
                .shell_state
                .env_vars
                .get(SHELL_START_EPOCH)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_else(current_epoch_seconds);
            let elapsed = current_epoch_seconds() - start;
            self.shell_state
                .env_vars
                .insert(SECONDS_OFFSET.to_string(), (assigned - elapsed).to_string());
            set_process_env(base_name, assigned.to_string());
            return true;
        }
        if base_name == "RANDOM" && !append {
            // GNU variables.c:1393-1408 assign_random: a non-numeric value
            // fails valid_number and returns without reseeding; a numeric
            // seed runs sbrand — rseed = seed, last_random_value = 0.
            if let Ok(seed) = value.trim().parse::<i64>() {
                self.shell_state.random_state.rseed.set(seed as u32);
                self.shell_state.random_state.last_value.set(0);
            }
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
        // GNU assign_array_var_from_string (arrayfunc.c:910-924): each
        // `[sub]=` element inside the stored compound text resolves under
        // the ExpandedOnce rules — the element words already went through
        // expand_words_no_vars during word expansion, and ASS_NOEXPAND (set
        // when array_expand_once) makes array_expand_index consume the
        // result verbatim; assoc keys take their expand_subscript_string
        // pass at arrayfunc.c:817 on the already-expanded word text.
        let value = if compound_assignment && value.starts_with('(') && value.ends_with(')') {
            match self.rewrite_compound_element_subscripts(
                base_name,
                &value,
                is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name),
                true,
            ) {
                Ok(rewritten) => rewritten,
                Err(partial) => {
                    // GNU assign_compound_array_list breaks on the failing
                    // element but keeps every element processed before it
                    // and materializes the array — store the partial list
                    // rather than abandoning the assignment. GNU
                    // arrayfunc.c assign_array_var_from_string converts the
                    // target to an array BEFORE the element loop runs, so a
                    // failed element on a new target still leaves it bound
                    // as `()` (array32.sub `b=( [$bad]=hi )`), while an
                    // existing or declared-but-unset target keeps its prior
                    // state.
                    if !self.shell_state.env_vars.contains_key(base_name)
                        && !is_marked_var(
                            &self.shell_state.env_vars,
                            DECLARED_UNSET_VARS,
                            base_name,
                        )
                    {
                        self.shell_state.env_vars.insert(
                            base_name.to_string(),
                            format_indexed_array_storage(BTreeMap::new()),
                        );
                        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, base_name);
                    }
                    let current = self
                        .shell_state
                        .env_vars
                        .get(base_name)
                        .cloned()
                        .unwrap_or_default();
                    let integer =
                        is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name);
                    let stored = if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
                    {
                        append_assoc_value(&current, &partial, integer, &self.shell_state.env_vars)
                    } else {
                        append_array_value(
                            &current,
                            &partial,
                            integer,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                            &self.shell_state.env_vars,
                        )
                        .unwrap_or(current)
                    };
                    self.shell_state
                        .env_vars
                        .insert(base_name.to_string(), stored);

                    self.exit_code = 1;
                    return false;
                }
            }
        } else {
            value
        };
        let value = if append {
            let current = self
                .shell_state
                .env_vars
                .get(base_name)
                .cloned()
                .unwrap_or_default();
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name) {
                if value.starts_with('(') && value.ends_with(')') {
                    append_assoc_value(
                        &current,
                        &value,
                        is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                        &self.shell_state.env_vars,
                    )
                } else {
                    append_assoc_scalar_value(&current, &value)
                }
            } else if is_array_storage(&current)
                || is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, base_name)
            {
                match append_array_value(
                    &current,
                    &value,
                    is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                    self.shell_state.env_vars.get("IFS").map(String::as_str),
                    &self.shell_state.env_vars,
                ) {
                    Ok(storage) => storage,
                    Err(pattern) => {
                        return self.fail_compound_array_assignment(base_name, &pattern);
                    }
                }
            } else if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name) {
                // GNU variables.c:2922-2935: `x+=v` on an integer variable
                // evaluates the current value first — its evalerror is just
                // as fatal as the rhs one, and neither bind happens.
                let Some(current) = self.eval_integer_assignment_checked(&current) else {
                    return false;
                };
                let Some(value) = self.eval_integer_assignment_checked(&value) else {
                    return false;
                };
                (current + value).to_string()
            } else {
                append_scalar_value(&current, &value)
            }
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
        {
            let bare_elements = assoc_bare_elements(&value);
            let empty_keys = assoc_empty_key_words(&value);
            // GNU assign_compound_array_list (arrayfunc.c:838-843): a bare
            // element in an assoc compound assignment reports an error and
            // breaks the loop, but elements already processed ARE stored.
            // Store the valid elements first, then report the error.
            let stored = append_assoc_value(
                "()",
                &value,
                is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                &self.shell_state.env_vars,
            );
            self.shell_state
                .env_vars
                .insert(base_name.to_string(), stored.clone());
            for bare in &bare_elements {
                let line = format!(
                    "{}{}: {}: must use subscript when assigning associative array
",
                    self.assignment_diagnostic_prefix(),
                    base_name,
                    bare
                );
                self.emit_assignment_diag(line);
            }
            // GNU assign_assoc_from_kvlist (arrayfunc.c:644-650): a kvpair
            // word whose expanded key is empty reports `<word>: bad array
            // subscript` but does NOT set any_failed — the pair is skipped
            // and the assignment still succeeds.
            for word in &empty_keys {
                let line = format!(
                    "{}{}: bad array subscript
",
                    self.assignment_diagnostic_prefix(),
                    word
                );
                self.emit_assignment_diag(line);
            }
            if !bare_elements.is_empty() {
                self.exit_code = 1;
            }
            stored
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
            && is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name)
            && integer_compound_assignment_is_scalar(&value)
        {
            // Bash keeps `typeset -i x; x=(1+2)` scalar.  A compound
            // assignment becomes an array only when it contains indexed o
            // multiple elements; the single arithmetic expression is still
            // assigned through the integer attribute.
            match self.eval_integer_assignment_checked(&value[1..value.len() - 1]) {
                Some(result) => result.to_string(),
                None => return false,
            }
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
        {
            // variables.c/arrayfunc.c: a compound `name=(...)` assignment
            // always makes an array, even when the variable previously had
            // the integer attribute (`typeset -i x; x=([0]=7+11)` becomes an
            // integer array with x[0]=18, not a scalar arithmetic result).
            match append_array_value(
                "()",
                &value,
                is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                self.shell_state.env_vars.get("IFS").map(String::as_str),
                &self.shell_state.env_vars,
            ) {
                Ok(storage) => storage,
                Err(pattern) => {
                    return self.fail_compound_array_assignment(base_name, &pattern);
                }
            }
        } else if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name) {
            // GNU variables.c:2937-2946 make_variable_value: evalexp failure
            // reports through evalerror and jump_to_top_level(DISCARD)s — the
            // variable keeps its previous value, it is not stored as empty.
            match self.eval_integer_assignment_checked(&value) {
                Some(result) => result.to_string(),
                None => return false,
            }
        } else {
            value
        };
        let value = self.apply_case_assignment_attributes(base_name, value);
        let protocol_scalar = self.pending_scalar_assignment;
        self.pending_scalar_assignment = false;
        if value.starts_with(STORAGE_WORD_PREFIX)
            && !protocol_scalar
            && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
        {
            mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, base_name);
        }
        unmark_env_name(
            &mut self.shell_state.env_vars,
            DECLARED_UNSET_VARS,
            base_name,
        );
        let is_array = compound_assignment
            || is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name);
        // GNU variables.c:3128-3142 bind_variable_internal: when the
        // variable is already an array, a scalar assignment sets array[0]
        // without clearing other elements (array.tests:171-174:
        // x[4]=bbb; x=abde keeps x[4]=bbb). Only compound `x=(...)` or
        // append `x+=...` should replace/extend the whole array.
        if !compound_assignment
            && !append
            && is_array
            && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
            && !value.starts_with(STORAGE_WORD_PREFIX)
        {
            let current = self
                .shell_state
                .env_vars
                .get(base_name)
                .cloned()
                .unwrap_or_default();
            let mut entries = indexed_array_entries(&current);
            entries.insert(0, value.clone());
            let storage = format_indexed_array_storage(entries);
            self.shell_state
                .env_vars
                .insert(base_name.to_string(), storage);
            mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, base_name);
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "allexport") {
                set_process_env(base_name, self.shell_state.env_vars[base_name].clone());
            }
            self.exit_code = 0;
            return true;
        }
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
        // GNU variables.c:3139-3140: for an assoc array, a scalar assignment
        // stores the value at key "0" via assign_func. Without this, a value
        // like `([a]=1)` is stored raw and later misinterpreted as assoc
        // storage format (assoc.tests:191 T='([a]=1)' -> ${T[@]} is `([a]=1)`,
        // not `1`).
        if !compound_assignment
            && !append
            && is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
            && !value.starts_with(STORAGE_WORD_PREFIX)
        {
            let storage = format!("([\"0\"]={})", quote_assoc_storage_value(&value));
            self.shell_state
                .env_vars
                .insert(base_name.to_string(), storage);
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "allexport") {
                self.mark_exported(base_name);
            }
            self.exit_code = 0;
            return true;
        }
        self.shell_state
            .env_vars
            .insert(base_name.to_string(), value.clone());
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "allexport") {
            self.mark_exported(base_name);
        }
        sync_shell_assignment_process_env(&self.shell_state.env_vars, base_name, value);
        true
    }
}

fn integer_compound_assignment_is_scalar(value: &str) -> bool {
    let Some(inner) = value.strip_prefix('(').and_then(|v| v.strip_suffix(')')) else {
        return false;
    };
    !inner.is_empty() && !inner.chars().any(|ch| ch.is_whitespace()) && !inner.contains(['[', ']'])
}
