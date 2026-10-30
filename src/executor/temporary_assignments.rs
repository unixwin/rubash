use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    /// Apply assignments from a command containing no command word. GNU Bash
    /// keeps these assignments in the current shell scope; they are not the
    /// temporary environment used by `name=value command`. Returns false when
    /// any assignment failed (the loop stops at the first failure, GNU subst.c
    /// expand_word_error DISCARD).
    pub(in crate::executor) fn apply_permanent_assignments(
        &mut self,
        assignments: &[(String, String)],
        raw_rhs: Option<&[String]>,
    ) -> bool {
        let raws_aligned = raw_rhs.is_some_and(|raws| raws.len() == assignments.len());
        for (index, (name, value)) in assignments.iter().enumerate() {
            let raw = raws_aligned
                .then(|| raw_rhs.and_then(|raws| raws.get(index)))
                .flatten()
                .map(String::as_str);
            // Debug knob resolved once (RUBASH_EXEC_PROFILE's ensure_init
            // precedent; GNU has no runtime debug gate at all here — this
            // loop runs per assignment word, and a per-word environment
            // scan dominated the assignment cost on Windows).
            static DEBUG_ASSIGN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *DEBUG_ASSIGN.get_or_init(|| std::env::var("RUBASH_DEBUG_ASSIGN").is_ok()) {
                eprintln!("ASSIGN {name}={value:?}");
            }
            let expanded_value = self.expand_assignment_value_with_raw(name, value, raw);
            if self.xtrace_enabled() {
                self.assignment_expansion_memo
                    .borrow_mut()
                    .insert((name.clone(), value.clone()), expanded_value.clone());
            }
            // GNU subst.c:10404+ expand_word_error -> DISCARD: a failed
            // assignment word (failglob no-match, readonly violation, ...)
            // abandons the rest of this command's assignment list.
            if !self.apply_shell_assignment(name, expanded_value) {
                // GNU subst.c:10404+ expand_word_error -> DISCARD: the failed
                // assignment reports status 1 and the evalerror abort armed
                // inside the apply path bounds the discard to this command
                // list.
                self.exit_code = 1;
                return false;
            }
            // GNU variables.c make_variable_value: an integer-attribute
            // assignment that fails arithmetic evaluation (e.g. `i=0#4`
            // with `declare -i i`) reports evalerror and propagates exit
            // status 1. apply_shell_assignment resets exit_code to 0 on
            // success, so promote the arithmetic_expansion_error flag here.
            if self.shell_state.arithmetic_expansion_error.get() {
                self.shell_state.arithmetic_expansion_error.set(false);
                self.exit_code = 1;
                return false;
            }
        }
        true
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
        // rubash#753: GNU's failglob discard abandons the rest of the
        // failing command's LOGICAL line, not just this assignment —
        // `{ a=(/no/such/*.zzz); echo in; }; echo after` prints neither
        // `in` nor `after` (oracle probe). Returning false maps the
        // failure to ExpansionFailure(1) at every caller, but that Err is
        // swallowed by the compound that carries the assignment (brace
        // pipeline, function frame, if body, case body), so arm the
        // reader-level abandon alongside it — the eval/source boundaries
        // absorb the class flag when the assignment ran inside an eval'd
        // string or a sourced file.
        self.raise_evalerror_abort();
        self.evalerror_failglob_pending.set(true);
        if !self.shell_state.env_vars.contains_key(base_name)
            && !is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, base_name)
        {
            self.shell_state.env_vars.insert(
                base_name.to_string(),
                format_indexed_array_storage(BTreeMap::new()),
            );
            mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, base_name);
            // Keep the structured cell in lockstep (empty array), same as
            // command_prepare's fail-compound path.
            crate::executor::arrays::sync_typed_indexed_array(
                &mut self.shell_state.variables,
                base_name,
                &std::collections::BTreeMap::new(),
            );
            self.shell_state.env_vars.clear_array_cells_dirty();
        }
        false
    }

    #[allow(clippy::type_complexity)]
    pub(in crate::executor) fn apply_temporary_assignments(
        &mut self,
        assignments: &[(String, String)],
        raw_rhs: Option<&[String]>,
    ) -> (
        Vec<(
            String,
            Option<String>,
            Option<crate::shell::Variable>,
            Option<VarAttrs>,
        )>,
        bool,
    ) {
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
            // The EXPORTED/NAMEREF whole-list snapshots cost O(marked
            // names) to render and again to restore, per command. The
            // apply loop below can only change list membership in two
            // ways, both visible before the loop runs: mark_exported on
            // a target that has neither an env value nor a typed cell
            // (its per-name entry restores with value/typed both None,
            // which skips the per-name attribute restore, so only the
            // list replay clears the bit), and the nameref paths (the
            // unresolved-cell fallback unmarks NAMEREF; a resolved
            // target is a different name whose state the base check
            // cannot see). Every other target's membership bit is
            // covered by its per-name entry's saved VarAttrs, whose
            // restore rewrites the serialized lists from the flags.
            // When no assignment can change membership, skip the
            // snapshots entirely (rubash#1120: `IFS= read` in a loop
            // paid both renders and both restores per record).
            let lists_may_change = assignments.iter().any(|(name, _)| {
                let (base_name, _) = assignment_name_and_append(name);
                is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base_name)
                    || (self.shell_state.env_vars.get(base_name).is_none()
                        && self.shell_state.variables.get(base_name).is_none())
            });
            if lists_may_change {
                // Materialize the write-behind marker lists: restore
                // replays the string, so the snapshot must reflect
                // current flags, not a stale serialized copy.
                let exported = self.shell_state.env_vars.rendered_attr_list(EXPORTED_VARS);
                let nameref = self.shell_state.env_vars.rendered_attr_list(NAMEREF_VARS);
                previous.push((
                    EXPORTED_VARS.to_string(),
                    exported,
                    self.shell_state.variables.get(EXPORTED_VARS).cloned(),
                    None,
                ));
                previous.push((
                    NAMEREF_VARS.to_string(),
                    nameref,
                    self.shell_state.variables.get(NAMEREF_VARS).cloned(),
                    None,
                ));
            }
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
        // rubash#1525: a tempenv PIPESTATUS bind replaces the cell
        // (scalar or compound) and the shape note follows the bind,
        // but the restore below only reinstalls the saved cell — the
        // shape (and the status vector an absorbed write may have
        // rewritten) stayed whatever the tempenv bind left behind,
        // so a scalar tempenv froze every later write behind a ghost
        // cell. GNU keeps shape and value in the one SHELL_VAR the
        // tempenv push/pop reinstalls whole (variables.c
        // push_temp_var / pop_var_context), so snapshot the pair
        // into the previous-list alongside the cell and mirror it
        // for the declare promotion path.
        if assignments
            .iter()
            .any(|(name, _)| assignment_name_and_append(name).0 == "PIPESTATUS")
        {
            let encoded = crate::shell::state::encode_pipestatus_state(
                self.shell_state.pipestatus_shape,
                &self.shell_state.pipestatus,
            );
            self.tempenv_pipestatus_state = Some(encoded.clone());
            previous.push((
                crate::shell::state::PIPESTATUS_STATE_SNAPSHOT_KEY.to_string(),
                Some(encoded),
                None,
                None,
            ));
        }
        let raws_aligned = raw_rhs.is_some_and(|raws| raws.len() == assignments.len());
        let mut assignments_failed = false;
        for (index, (name, value)) in assignments.iter().enumerate() {
            let raw = raws_aligned
                .then(|| raw_rhs.and_then(|raws| raws.get(index)))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(name, value, raw);
            // GNU subst.c:13219 expand_word_list_internal ->
            // shell_expand_word_list (subst.c:13092-13106): a `$((...))`
            // evalerror inside the assignment prefix's own expansion jumps
            // DISCARD mid-expansion — the command never runs and the rest of
            // the command list is abandoned (rubash#1226: `x=$((1/0)) echo
            // hi` prints only the diagnostic). Consume the flag the failing
            // expansion left armed, mark the apply failed, and arm the
            // reader-level abort; the dispatch sites below translate
            // assignments_failed+pending into the skip-and-abandon Err.
            if self.shell_state.arithmetic_expansion_error.replace(false) {
                self.shell_state.arithmetic_fatal_error.set(false);
                self.shell_state.arithmetic_nonfatal_error.set(false);
                self.exit_code = 1;
                // rubash#439: expansion error, not a failed command — no ERR
                // trap may fire for a command that never ran.
                self.command_expansion_failed.set(true);
                // rubash#1518: prefix-assignment evalerror is the same
                // pure-DISCARD class — no errexit promotion.
                self.evalerror_arith_pending.set(true);
                self.raise_evalerror_abort();
                assignments_failed = true;
                break;
            }
            if self.xtrace_enabled() {
                self.assignment_expansion_memo
                    .borrow_mut()
                    .insert((name.clone(), value.clone()), expanded_value.clone());
            }
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
                        "{}{base_name}: readonly variable\n",
                        self.assignment_readonly_prefix()
                    );
                    self.emit_assignment_diag(line);
                    assignments_failed = true;
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
                        self.assignment_readonly_prefix()
                    );
                    self.emit_assignment_diag(line);
                    assignments_failed = true;
                    continue;
                }
                // rubash#1620: an APPEND compound word (`a+=(9) cmd`)
                // prepends the old cell's scalar view to the literal
                // list text (GNU variables.c assign_in_env via
                // make_variable_value with ASS_APPEND: element 0 for an
                // indexed array, empty for an assoc array, the value
                // itself for a scalar) — the same view the #1556 array
                // shadow branch below computes for plain appends. The
                // non-append form keeps binding the list text alone.
                let (_, append) = assignment_name_and_append(name);
                let window_value = if append {
                    let target = resolved_target.as_deref().unwrap_or(base_name).to_string();
                    format!("{}{}", self.tempenv_scalar_view(&target), compound)
                } else {
                    compound.to_string()
                };
                self.shell_state
                    .env_vars
                    .insert(base_name.to_string(), window_value.clone());
                let _ = self.shell_state.variables.set(
                    base_name.to_string(),
                    crate::shell::Variable::scalar(window_value),
                );
                // rubash#1525: the compound tempenv binds as a
                // literal scalar tempvar, PIPESTATUS included.
                if base_name == "PIPESTATUS" {
                    self.shell_state.pipestatus_shape =
                        crate::shell::state::PipestatusShape::Scalar;
                }
                self.tempenv_names.push(base_name.to_string());
                // rubash#1556: the tempvar is a FRESH variable in GNU
                // (variables.c assign_in_env -> make_new_variable
                // carries only att_exported|att_tempvar). The outer
                // cell's marks (array/assoc/integer/nameref) must not
                // leak into the window, or declare -p and the
                // subscript readers keep rendering the shadow as the
                // outer array. The VarAttrs saved in `previous`
                // restore them after the command.
                set_var_attrs(
                    &mut self.shell_state.env_vars,
                    base_name,
                    VarAttrs {
                        exported: true,
                        ..VarAttrs::default()
                    },
                );
                continue;
            }
            // rubash#1525: a tempenv PIPESTATUS bind is a fresh
            // scalar tempvar in GNU (variables.c assign_in_env
            // creates the binding in the temporary_env table,
            // shadowing any array cell whole), so bind the scalar
            // directly instead of routing through the permanent
            // bind's element-0-on-array rule. The previous entry
            // saved above reinstalls the shadowed cell at restore,
            // and the (shape, vector) snapshot restores the note.
            if base_name == "PIPESTATUS" && name == "PIPESTATUS" && resolved_target.is_none() {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, base_name) {
                    let line = format!(
                        "{}{base_name}: readonly variable\n",
                        self.assignment_readonly_prefix()
                    );
                    self.emit_assignment_diag(line);
                    assignments_failed = true;
                    continue;
                }
                self.shell_state
                    .env_vars
                    .insert(base_name.to_string(), expanded_value.clone());
                let _ = self.shell_state.variables.set(
                    base_name.to_string(),
                    crate::shell::Variable::scalar(expanded_value.clone()),
                );
                self.shell_state.pipestatus_shape = crate::shell::state::PipestatusShape::Scalar;
                self.tempenv_names.push(base_name.to_string());
                self.mark_exported(base_name);
                continue;
            }
            // rubash#1556: GNU variables.c:3536 assign_in_env creates
            // EVERY tempenv binding as a fresh scalar in the separate
            // temporary_env table (make_new_variable + var_setvalue,
            // att_exported|att_tempvar only) — the outer cell is never
            // written. The permanent bind below instead writes a plain
            // scalar value into element 0 of an array cell in place,
            // so window reads (declare -p, ${a[1]}, ${a[@]}, ${!a[@]})
            // see a mutated array instead of the shadow scalar. When
            // the effective target's cell is an array, swap in the
            // tempvar shadow directly: scalar typed cell + mirror
            // text + fresh exported-only attrs; the `previous`
            // snapshot reinstalls cell, mirror and attrs at restore.
            // Append computes against the old cell's scalar view
            // (make_variable_value with ASS_APPEND: element 0 for an
            // indexed array, empty for an assoc array). The dynamic
            // stack arrays keep the permanent-bind path: they have no
            // ordinary cell to shadow.
            {
                let (_, append) = assignment_name_and_append(name);
                let target = resolved_target
                    .clone()
                    .unwrap_or_else(|| base_name.to_string());
                if target != "PIPESTATUS"
                    && !matches!(
                        target.as_str(),
                        "FUNCNAME"
                            | "BASH_ARGV"
                            | "BASH_ARGC"
                            | "BASH_LINENO"
                            | "BASH_SOURCE"
                            | "GROUPS"
                            | "DIRSTACK"
                    )
                    && self.tempenv_target_is_array(&target)
                {
                    if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &target) {
                        // GNU err_readonly for a tempenv bind carries
                        // no command-name segment (the permanent-bind
                        // path's plain form), even when the command
                        // word is a declaration builtin.
                        let line = format!("{}{target}: readonly variable\n", self.error_prefix());
                        self.emit_assignment_diag(line);
                        assignments_failed = true;
                        continue;
                    }
                    let window_value = if append {
                        format!("{}{}", self.tempenv_scalar_view(&target), expanded_value)
                    } else {
                        expanded_value.clone()
                    };
                    self.shell_state
                        .env_vars
                        .insert(target.clone(), window_value.clone());
                    let _ = self
                        .shell_state
                        .variables
                        .set(target.clone(), crate::shell::Variable::scalar(window_value));
                    set_var_attrs(
                        &mut self.shell_state.env_vars,
                        &target,
                        VarAttrs {
                            exported: true,
                            ..VarAttrs::default()
                        },
                    );
                    self.tempenv_names.push(target);
                    continue;
                }
            }
            // rubash#1567: GNU variables.c:3536 assign_in_env creates
            // EVERY tempenv binding as a fresh scalar in the separate
            // temporary_env table (make_new_variable + var_setvalue,
            // attributes att_exported|att_tempvar only) — the found
            // variable's attribute semantics apply ONLY to an append's
            // value computation (make_variable_value with ASS_APPEND
            // against the found var). The fallthrough below instead
            // routes an attributed scalar target (declare -i/-l/-u)
            // through the permanent bind, which evaluates/converts the
            // window value in place and leaks the attribute marks into
            // window reads (`declare -i s=5; s=007 declare -p s` printed
            // `declare -ix s="7"`; GNU prints `declare -x s="007"`).
            // Shadow such targets exactly like the array branch above:
            // verbatim scalar + fresh exported-only attrs for a plain
            // word; for an append, run the permanent bind (its
            // attribute evaluation IS the GNU append computation, and
            // the name-keyed hooks — sv_locale mirrors, dynamic
            // seeding — stay on their funnel) and then reset the marks
            // to the tempvar shape. Name-keyed dynamic variables keep
            // the permanent funnel entirely: their hooks ARE the
            // assignment semantics (compare bind_marks_free_scalar's
            // exclusion list in apply_shell_assignment_inner).
            {
                let (_, append) = assignment_name_and_append(name);
                let target = resolved_target
                    .clone()
                    .unwrap_or_else(|| base_name.to_string());
                let target_attrs = capture_var_attrs(&self.shell_state.env_vars, &target);
                if !name.contains('[')
                    && !target.contains('[')
                    && !matches!(
                        target.as_str(),
                        "OPTIND"
                            | "IGNOREEOF"
                            | "ignoreeof"
                            | "SECONDS"
                            | "RANDOM"
                            | "SRANDOM"
                            | "BASHPID"
                            | "BASH_SUBSHELL"
                            | "FUNCNAME"
                            | "LINENO"
                            | "BASH_COMMAND"
                            | "BASH_ARGC"
                            | "BASH_ARGV"
                            | "BASH_LINENO"
                            | "BASH_SOURCE"
                            | "PIPESTATUS"
                    )
                    && !target_attrs.array
                    && !target_attrs.assoc
                    && (target_attrs.integer
                        || target_attrs.uppercase
                        || target_attrs.lowercase
                        || target_attrs.trace
                        || target_attrs.declared_unset)
                {
                    if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &target) {
                        // GNU err_readonly for a tempenv bind carries
                        // no command-name segment (the permanent-bind
                        // path's plain form), same ruling as the array
                        // branch above.
                        let line = format!("{}{target}: readonly variable\n", self.error_prefix());
                        self.emit_assignment_diag(line);
                        assignments_failed = true;
                        continue;
                    }
                    if append {
                        // The permanent bind computes the append
                        // against the found variable's attributes
                        // (make_variable_value ASS_APPEND) and fires
                        // every name-keyed hook; only the attribute
                        // marks it leaves on the cell are the leak, so
                        // reset them to the fresh tempvar shape.
                        if !self.apply_shell_assignment(name, expanded_value) {
                            assignments_failed = true;
                        } else {
                            set_var_attrs(
                                &mut self.shell_state.env_vars,
                                &target,
                                VarAttrs {
                                    exported: true,
                                    ..VarAttrs::default()
                                },
                            );
                        }
                        self.tempenv_names.push(target.clone());
                        self.mark_exported(&target);
                        continue;
                    }
                    self.shell_state
                        .env_vars
                        .insert(target.clone(), expanded_value.clone());
                    let _ = self.shell_state.variables.set(
                        target.clone(),
                        crate::shell::Variable::scalar(expanded_value.clone()),
                    );
                    set_var_attrs(
                        &mut self.shell_state.env_vars,
                        &target,
                        VarAttrs {
                            exported: true,
                            ..VarAttrs::default()
                        },
                    );
                    // The permanent funnel's tail mirrors gate names
                    // to the process environment (the #1531 sv_locale
                    // family reads its categories from there); keep
                    // the same mirror for the shadow so an attributed
                    // locale name still drives the window's locale.
                    // Restore re-syncs the previous process value.
                    sync_shell_assignment_process_env(
                        &self.shell_state.env_vars,
                        &target,
                        &expanded_value,
                    );
                    // rubash#1692: the shadow store is a bind too.
                    self.note_locale_assignment(&target);
                    self.tempenv_names.push(target);
                    continue;
                }
            }
            // GNU variables.c bind_variable (ASS_MKLOCAL tempenv path): a
            // failed bind (readonly violation via ASSIGN_DISALLOWED, expansion
            // error) reports and keeps going through the REST of the operand
            // list here; the caller decides posix fatality (execute_cmd.c
            // jump_to_top_level on bash_variable_assignment_error).
            if !self.apply_shell_assignment(name, expanded_value) {
                assignments_failed = true;
            }
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
        (previous, assignments_failed)
    }

    /// Whether the tempenv target's current cell is an array
    /// (rubash#1556): the VarTable attribute word or the typed
    /// cell's shape. Either witness suffices — a declared but
    /// valueless array may carry only the mark, and a materialized
    /// array may predate its mark sync.
    fn tempenv_target_is_array(&self, target: &str) -> bool {
        let attrs = self.shell_state.env_vars.attr_of(target);
        attrs.array
            || attrs.assoc
            || matches!(
                self.shell_state.variables.get(target).map(|v| &v.value),
                Some(crate::shell::ShellValue::IndexedArray(_))
                    | Some(crate::shell::ShellValue::AssociativeArray(_))
            )
    }

    /// The scalar view of a cell for a tempenv append (GNU
    /// variables.c make_variable_value with ASS_APPEND, rubash#1556):
    /// element 0 of an indexed array, empty for an assoc array,
    /// the value itself for a scalar.
    pub(in crate::executor) fn tempenv_scalar_view(&self, target: &str) -> String {
        match self.shell_state.variables.get(target).map(|v| &v.value) {
            Some(crate::shell::ShellValue::IndexedArray(entries)) => {
                entries.get(&0).cloned().unwrap_or_default()
            }
            Some(crate::shell::ShellValue::AssociativeArray(_)) => String::new(),
            Some(crate::shell::ShellValue::Scalar(value)) => value.clone(),
            None => self
                .shell_state
                .env_vars
                .get(target)
                .and_then(|value| {
                    resolve_indexed_array_subscript(value, 0)
                        .and_then(|resolved| array_value_at(value, resolved))
                })
                .unwrap_or_default(),
        }
    }

    /// GNU execute_cmd.c: assignment errors (readonly violation, expansion
    /// error) jump to top level only in a POSIX NON-interactive shell
    /// (bash_variable_assignment_error -> exit status 1). With no command
    /// word the exit is unconditional in GNU (both modes — the standalone
    /// DISCARD path already aborts the command); the POSIX gate covers the
    /// command-word forms, where GNU 5.3 exits for every command class, not
    /// just special builtins (`ro=2 ls`, `ro=2 :`, `ro=2 true` all exit —
    /// oracle probes B/B2/B3; the command itself never runs).
    pub(in crate::executor) fn posix_assignment_failure_is_fatal(&self) -> bool {
        self.posix_mode_enabled()
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_INTERACTIVE")
                .map(String::as_str)
                != Some("1")
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
                self.assignment_error_prefix()
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
            // Typed-cell sync (assoc fast path), symmetric with the indexed
            // leg below: `entries` is the post-write key list this render
            // folds, so the structured assoc cell is synced from the same
            // list. GNU keeps ONE assoc table — variables.c:3266-3267
            // bind_variable (nameref cell -> assign_array_element) ->
            // arrayfunc.c:262 bind_array_variable -> assoc.c:68 assoc_insert
            // overwrites the key in place — so the cell can never hold a
            // stale element. The complete-or-absent invariant is what lets
            // the typed-cell read fast path (executor/arrays/executor.rs:194,
            // GNU assoc.c:120 assoc_reference) trust the cell; without this
            // sync a stale typed cell shadows the mirror (rubash#961).
            crate::executor::arrays::sync_typed_assoc_array(
                &mut self.shell_state.variables,
                elem_base,
                &entries,
            );
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
            self.shell_state.env_vars.clear_array_cells_dirty();
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
                // A nounset trap already set the fatal status and raised the
                // abort — skip the plain exit-1 so it survives (rubash#639).
                if self.raise_nounset_subscript_abort() {
                    return false;
                }
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
        crate::executor::arrays::sync_typed_indexed_array(
            &mut self.shell_state.variables,
            elem_base,
            &entries,
        );
        self.shell_state
            .env_vars
            .insert(elem_base.to_string(), format_indexed_array_storage(entries));
        self.shell_state.env_vars.clear_array_cells_dirty();
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
    pub(in crate::executor) fn eval_integer_assignment_checked(
        &mut self,
        value: &str,
    ) -> Option<i128> {
        if let Some(result) = crate::executor::arithmetic::eval_conditional_arith_value(
            value,
            &self.shell_state.env_vars,
        ) {
            return Some(result);
        }
        let message =
            crate::executor::arithmetic::take_arith_eval_error().map(|record| record.render());
        let message = message.or_else(|| {
            crate::executor::arithmetic::arithmetic_error_message(
                value,
                false,
                &self.shell_state.env_vars,
            )
        });
        if let Some(message) = message {
            let line = format!("{}{}\n", self.assignment_error_prefix(), message);
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
    pub(in crate::executor) fn emit_assignment_diag(&mut self, line: String) {
        if self.buffer_assignment_diagnostics {
            self.pending_assignment_diagnostics
                .extend_from_slice(line.as_bytes());
        } else if !self.fd_table.is_closed(2) && self.fd_table.output_endpoint(2).is_none() {
            // No fd-2 binding in the table: the process stderr is the
            // destination, as with the raw write this replaces.
            let _ = std::io::stderr().write_all(line.as_bytes());
        } else {
            // GNU error.c report_error family: assignment diagnostics go
            // to fd 2 AS BOUND by enclosing redirections — `( UID=5 )
            // 2>err` must land in err and `{ UID=5; } 2>/dev/null` must
            // stay silent. The raw process-stderr write bypassed every
            // compound binding (rubash#1317; same class as rubash#167).
            self.write_diagnostic_fd2(line.as_bytes());
        }
    }

    pub(in crate::executor) fn assignment_diagnostic_prefix(&self) -> String {
        match &self.assignment_command_name {
            Some(command) => format!("{}{command}: ", self.diagnostic_prefix()),
            None => self.diagnostic_prefix(),
        }
    }

    /// GNU error.c:453 err_readonly (report_error family) twin of
    /// assignment_diagnostic_prefix: a bare bind-layer `name: readonly
    /// variable` honors gnu_errfmt (rubash#1038); a builtin-command
    /// context (`declare: name: ...`) is sh_readonly (builtin_error, no
    /// gate) and keeps the ungated shape.
    fn assignment_readonly_prefix(&self) -> String {
        match &self.assignment_command_name {
            Some(command) => format!("{}{command}: ", self.diagnostic_prefix()),
            None => self.error_prefix(),
        }
    }

    /// error_prolog-family twin of assignment_diagnostic_prefix (evalerror,
    /// err_badarraysub, internal_warning): the line segment honors
    /// gnu_errfmt in both arms — an evalerror under a builtin still gates
    /// (`declare: 1/0: ...`, rubash#1038).
    fn assignment_error_prefix(&self) -> String {
        match &self.assignment_command_name {
            Some(command) => format!("{}{command}: ", self.error_prefix()),
            None => self.error_prefix(),
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
        // GNU variables.c:1528-1545 assign_bash_argv0: assigning BASH_ARGV0
        // rebinds dollar_vars[0] itself, so `unset BASH_ARGV0` afterwards
        // leaves $0 at the last assigned value (the dynamic var dies, the
        // dollar_vars slot does not). Snapshot into a dedicated slot that
        // only the post-unset fallback reads — deliberately NOT
        // __RUBASH_TOP_LEVEL_NAME, which also gates the bashdb source-path
        // bridges in public_accessors.rs (bashdb-generated rebinds
        // BASH_ARGV0 to the debugged script, a legitimate assign).
        if result && base == "BASH_ARGV0" {
            if let Some(value) = self.shell_state.env_vars.get("BASH_ARGV0").cloned() {
                self.set_env("__RUBASH_ARGV0_AFTER_UNSET", &value);
            }
        }
        // GNU variables.c:5778 { "GLOBIGNORE", sv_globignore } ->
        // pathexp.c:507 setup_glob_ignore: assigning a NON-NULL GLOBIGNORE
        // turns dotfile matching ON and assigning NULL leaves it unchanged;
        // the effect shares the dotglob cell with the shopt, so the write
        // lands in the same option storage. Unsetting GLOBIGNORE turns the
        // cell OFF (handled in the unset builtin). A set-but-null value
        // must NOT enable dotglob by itself (rubash#313: bash-completion's
        // `local GLOBIGNORE=""` made `*` match `.gitignore`).
        if result && base == "GLOBIGNORE" {
            if self
                .shell_state
                .env_vars
                .get("GLOBIGNORE")
                .is_some_and(|value| !value.is_empty())
            {
                let _ = crate::builtins::shopt::set_option(
                    &mut self.shell_state.env_vars,
                    "dotglob",
                    true,
                );
            }
        }
        // GNU variables.c:5817 { "OPTIND", sv_optind }: a bind of
        // OPTIND resets the getopt scan state from the variable's
        // current value — POSIX's `OPTIND=1` restart, a reseek for
        // values > 1. The hook fires for ANY bind attempt, including
        // one refused as readonly (the value is unchanged, so the scan
        // restarts from the frozen value). Rubash carries the live scan
        // position in shadow keys while OPTIND is readonly
        // (rubash#1268); dropping them makes the next getopts call
        // re-derive its start from the variable itself, and dropping
        // the char offset mirrors getopts_reset clearing nextchar
        // (rubash#1270).
        if base == "OPTIND" {
            self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OPTIND");
            self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        }
        // GNU variables.c bind_variable / arrayfunc.c
        // assign_array_var_from_string: a successful PIPESTATUS bind
        // fixes the cell shape — compound makes a live array, scalar
        // makes (or keeps) the shape note_pipestatus_assignment derives
        // from the stored result (rubash#1363/#1364).
        if result && base == "PIPESTATUS" {
            self.note_pipestatus_assignment();
        }
        // GNU variables.c:5760 { "BASH_COMPAT", sv_shcompat }: assigning
        // BASH_COMPAT re-syncs the compatNN shopt flags (and resets the
        // level to default on an unparseable/out-of-range value, keeping
        // the raw string). The diagnostic goes through the assignment
        // prologue like any bind-hook error; the assignment itself still
        // succeeds (verified oracle: `BASH_COMPAT=99` is status 0).
        if result && base == "BASH_COMPAT" {
            if let Err(diagnostic) = crate::builtins::shopt::sync_compat_flags_from_bash_compat(
                &mut self.shell_state.env_vars,
            ) {
                // GNU sv_shcompat errors never carry a command segment
                // (internal_error, verified oracle: even `read BASH_COMPAT`
                // reports bare `BASH_COMPAT: ...`), so the plain
                // diagnostic_prefix is used, not the assignment one.
                let line = format!("{}{diagnostic}\n", self.diagnostic_prefix());
                self.emit_assignment_diag(line);
            }
        }
        // GNU variables.c:5819 { "PATH", sv_path } -> variables.c:5919
        // sv_path -> hashcmd.c:57 phash_flush: every successful PATH
        // bind empties the command hash table (rubash#1551). This
        // funnel carries plain and `+=` assignments, temp-prefix binds,
        // and the read/for/select/getopts binds that route through
        // apply_shell_assignment; the restore side of a temp bind is a
        // re-bind handled in the tempenv restore path.
        if result && base == "PATH" {
            crate::builtins::hash::flush_command_hash_table(&mut self.shell_state.env_vars);
        }
        result
    }

    fn apply_shell_assignment_inner(&mut self, name: &str, value: String) -> bool {
        // TODO(variables.c/arrayfunc.c): Bash stores append assignment state
        // separately on WORD_DESC/ASSIGNMENT_WORD. This narrow path handles
        // scalar `name+=value` until SHELL_VAR attributes and arrays own it.
        let (base_name, append) = assignment_name_and_append(name);
        if !append {
            if let Some(done) = self.bind_marks_free_scalar(base_name, &value) {
                return done;
            }
        }
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
                        self.assignment_error_prefix(),
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
                    self.assignment_error_prefix(),
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
                    self.assignment_error_prefix(),
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
                    // rubash#616 (1a): a scalar bind over a name that still
                    // carries a structured cell drops it — the mirror is
                    // authoritative for this shape.
                    sync_typed_indexed_storage(&mut self.shell_state.variables, base_name, &value);
                    if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name) {
                        sync_typed_assoc_storage(
                            &mut self.shell_state.variables,
                            base_name,
                            &value,
                        );
                    } else {
                        drop_typed_assoc_array(&mut self.shell_state.variables, base_name);
                    }
                    // Sync-or-drop just ran over the landed mirror value
                    // (rubash#1510).
                    self.note_typed_cell_synced(base_name);
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
                        let line =
                            format!("{}{}: readonly variable\n", self.error_prefix(), elem_base);
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
                    // GNU arrayfunc err_badarraysub (report_error family):
                    // honors gnu_errfmt (rubash#1038).
                    let line = format!(
                        "{}{}: bad array subscript
",
                        self.assignment_error_prefix(),
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
                        let line =
                            format!("{}{}: readonly variable\n", self.error_prefix(), elem_base);
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
            let line = format!("{}{}: readonly variable\n", self.error_prefix(), base_name);
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
        // GNU variables.c:3839 unbind_variable: `unset` removes a dynamic
        // variable's cell entirely, so a later assignment binds an
        // ORDINARY variable (static value, `declare -- SECONDS="7"`)
        // instead of re-arming the dynamic getter (rubash#1050). When
        // the name carries the unbound mark, every dynamic assignment
        // arm below must stand down and let the plain bind happen.
        let dynamic_unbound =
            is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, base_name);
        if base_name == "SECONDS" && !append && !dynamic_unbound {
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
            // GNU variables.c:1393-1424 assign_random -> get_random_number
            // seeding: after `unset RANDOM` the cell is unbound
            // (variables.c:3839 unbind_variable) and the name is a plain
            // variable — `unset RANDOM; RANDOM=42` stores 42 verbatim and
            // reads it back (rubash#1082).
            if is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, "RANDOM") {
                self.shell_state
                    .env_vars
                    .insert(base_name.to_string(), value.to_string());
                set_process_env(base_name, value);
                return true;
            }
            // GNU assign_random evaluates the value with evalexp
            // (variables.c assign_random -> legal_number/evalexp path):
            // a plain integer seeds directly; any other expression
            // (`abc` -> 0 via the unset variable, `010` -> octal 8,
            // `0x10` -> 16) seeds with its arithmetic value; an evalexp
            // failure (`42x`, `3.5`) prints the evalexp diagnostic and
            // leaves the generator unseeded (rubash#1082).
            if let Some(seed) = Self::canonical_decimal_seed(&value) {
                self.shell_state.random_state.rseed.set(seed as u32);
                self.shell_state.random_state.last_value.set(0);
            } else if let Some(seed) = crate::executor::arithmetic::eval_conditional_arith_value(
                &value,
                &self.shell_state.env_vars,
            ) {
                self.shell_state.random_state.rseed.set(seed as u32);
                self.shell_state.random_state.last_value.set(0);
            } else if let Some(record) = crate::executor::arithmetic::take_arith_eval_error() {
                let line = format!("{}{}\n", self.diagnostic_prefix(), record.render());
                self.emit_assignment_diag(line);
            } else {
                // No value and no error (empty string): seed 0.
                self.shell_state.random_state.rseed.set(0);
                self.shell_state.random_state.last_value.set(0);
            }
            set_process_env(base_name, value);
            return true;
        }
        if base_name == "SRANDOM" && !append && !dynamic_unbound {
            return true;
        }
        if base_name == "BASHPID" && !append && !dynamic_unbound {
            return true;
        }
        if base_name == "BASH_SUBSHELL" && !append && !dynamic_unbound {
            return true;
        }
        if base_name == "FUNCNAME" && !append && !dynamic_unbound {
            return true;
        }
        if base_name == "LINENO" && !append && !dynamic_unbound {
            return true;
        }
        if base_name == "BASH_COMMAND" && !append && !dynamic_unbound {
            return true;
        }
        // PIPESTATUS is the one noassign-list array whose cell is an
        // ordinary bindable variable in GNU (variables.c:6291 gates
        // only the status WRITER on the cell shape): a scalar or
        // compound bind creates a real cell the writer then respects,
        // so it takes the generic store below and the caller's
        // note_pipestatus_assignment derives the shape from the
        // stored form (rubash#1364). The stack arrays (BASH_ARGC &
        // friends) keep the swallow: their storage is the frame stack.
        if is_noassign_bash_array(base_name) && base_name != "PIPESTATUS" && !append {
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
                        // rubash#616 (1a): the partial failure still
                        // materializes an empty array — keep the structured
                        // cell in lockstep with the `()` render.
                        sync_typed_indexed_array(
                            &mut self.shell_state.variables,
                            base_name,
                            &BTreeMap::new(),
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
                        append_assoc_value(
                            &current,
                            &partial,
                            integer,
                            &self.shell_state.env_vars,
                            base_name,
                        )
                    } else {
                        match append_array_value(
                            &current,
                            None,
                            &partial,
                            base_name,
                            integer,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                            &self.shell_state.env_vars,
                            &self.shell_state.cwd,
                        ) {
                            Ok(outcome) => render_indexed_outcome(&current, outcome),
                            Err(_) => current,
                        }
                    };
                    self.shell_state
                        .env_vars
                        .insert(base_name.to_string(), stored.clone());
                    // rubash#616 (1a): mirror written — refresh/drop the
                    // structured cell (indexed render syncs, other shapes
                    // invalidate).
                    sync_typed_indexed_storage(&mut self.shell_state.variables, base_name, &stored);

                    self.exit_code = 1;
                    return false;
                }
            }
        } else {
            value
        };
        // rubash#616 (plan Phase 1c): set when the append branch applied the
        // compound write straight to the canonical mirror + structured cell,
        // so the shared tail below must not re-render and re-sync it.
        let mut mirror_written_in_place = false;
        // rubash#631: the first element header whose negative subscript
        // resolved before the array start (GNU arrayfunc.c:804-809). The
        // partial element writes still land (see the tail), then the
        // assignment fails like the plain `a[-2]=q` path does.
        let mut bad_subscript: Option<String> = None;
        // rubash#1618: set when the value chain produced an array storage
        // render whose element values were already case-converted by the
        // producing walker (append_array_value / append_assoc_value). The
        // shared case pass below converts a SCALAR value text; running it
        // over a storage render instead would case-fold assoc keys (the
        // appended value was then unreachable under its written key), so
        // those renders skip it.
        let mut element_values_converted = false;
        let value = if append {
            // Borrowed, not cloned: the old code cloned the whole mirror
            // here on EVERY append, but the clone is read only by the
            // render fallback below — the in-place append path (the
            // fill-loop shape, where the mirror grows to Θ(n) bytes)
            // never reads it, so the clone was a Θ(mirror) memcpy per
            // append for nothing. The arms below consume the borrow
            // through &str parameters only; the fallback re-fetches.
            let current = self
                .shell_state
                .env_vars
                .get(base_name)
                .map(String::as_str)
                .unwrap_or_default();
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name) {
                if value.starts_with('(') && value.ends_with(')') {
                    element_values_converted = true;
                    append_assoc_value(
                        current,
                        &value,
                        is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                        &self.shell_state.env_vars,
                        base_name,
                    )
                } else {
                    element_values_converted = true;
                    append_assoc_scalar_append_value(
                        current,
                        &value,
                        is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                        &self.shell_state.env_vars,
                        base_name,
                    )
                }
            } else if is_array_storage(current)
                || is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, base_name)
                || (value.starts_with('(') && value.ends_with(')'))
            {
                // GNU subst.c:3611-3620 do_assignment_internal: a compound
                // `name+=(list)` word has assign_list set, so it ALWAYS goes
                // through do_compound_assignment (subst.c:3441) — never the
                // scalar bind at the bottom. For a target that is not yet an
                // array, find_or_make_array_variable (arrayfunc.c:454) makes
                // one: unset binds a fresh array (make_new_array_variable,
                // arrayfunc.c:478), an existing scalar converts with its
                // value as element 0 (convert_var_to_array, arrayfunc.c:498
                // — `s=abc; s+=("x y")` yields ([0]="abc" [1]="x y")), and
                // an integer scalar converts too, keeping the attribute and
                // evaluating each element (`declare -i i; i+=(1+2)` yields
                // declare -ai i=([0]="3"); assign_compound_array_list starts
                // at last_ind = array_max_index+1 = 0 for the fresh array,
                // arrayfunc.c:743). array_values("") is empty, so an unset
                // current seeds no phantom element 0. The store tail below
                // marks ARRAY_VARS from the STORAGE_WORD_PREFIX render
                // append_array_value returns, exactly like the non-append
                // compound branch (rubash#214: v+=("word") on an unset
                // variable used to fall through to append_scalar_value and
                // store the serialized `("word")` literal as a scalar).
                match append_array_value(
                    current,
                    self.shell_state.variables.indexed_array_cell(base_name),
                    &value,
                    base_name,
                    is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                    self.shell_state.env_vars.get("IFS").map(String::as_str),
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                ) {
                    Ok(outcome) => {
                        element_values_converted = true;
                        // rubash#631: the scan stopped on an out-of-range
                        // negative subscript; the renders below still carry
                        // every element written before it.
                        bad_subscript = outcome.bad_subscript.clone();
                        // rubash#616 (plan Phase 1c): an append-only compound
                        // write onto a canonical mirror with a structured cell
                        // is applied in place — the decode is seeded from the
                        // cell, so no O(n) element map is built, no render is
                        // produced and the mirror is never re-parsed. Every
                        // other shape (a subscript that overwrites, an integer
                        // array, no cell, a non-canonical mirror) renders
                        // exactly as before.
                        if outcome.appended_only
                            && self.apply_compound_append_in_place(base_name, &outcome)
                        {
                            // No ARRAY_VARS mark here: the render path this
                            // replaces does not set one either, and array-ness
                            // is recognised by `is_array_storage` on the mirror
                            // (see the arm condition above). The in-place write
                            // must leave the same observable state as the
                            // render + tail it short-circuits.
                            mirror_written_in_place = true;
                            String::new()
                        } else {
                            // Render fallback: re-fetch the mirror text
                            // (the pre-append borrow ended before the
                            // in-place attempt). When the attempt was
                            // refused nothing was written, so this is the
                            // pre-append text exactly as before; when
                            // the mirror was extended but the cell
                            // refused, this is the extended text, whose
                            // parse folds to the same entries as the
                            // pre-append text plus the outcome (the
                            // segment writer appends exactly the
                            // outcome's segments), so the render — and
                            // the cell re-sync in the tail — lands on
                            // the same state either way.
                            let current = self
                                .shell_state
                                .env_vars
                                .get(base_name)
                                .cloned()
                                .unwrap_or_default();
                            render_indexed_outcome(&current, outcome)
                        }
                    }
                    Err(pattern) => {
                        return self.fail_compound_array_assignment(base_name, &pattern);
                    }
                }
            } else if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name) {
                // GNU variables.c:2922-2935: `x+=v` on an integer variable
                // evaluates the current value first — its evalerror is just
                // as fatal as the rhs one, and neither bind happens.
                // (Materialize the small scalar text: the checked eval
                // takes &mut self, so the borrowed mirror cannot be held
                // across it. This arm is scalar-only by construction.)
                let current = current.to_string();
                let Some(current) = self.eval_integer_assignment_checked(&current) else {
                    return false;
                };
                let Some(value) = self.eval_integer_assignment_checked(&value) else {
                    return false;
                };
                (current + value).to_string()
            } else {
                append_scalar_value(current, &value)
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
                base_name,
            );
            // Partial store before the error report — keep the structured
            // cell in lockstep from the same render.
            sync_typed_assoc_storage(&mut self.shell_state.variables, base_name, &stored);
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
                // GNU arrayfunc.c:644-650 err_badarraysub (report_error
                // family): honors gnu_errfmt (rubash#1038).
                let line = format!(
                    "{}{}: bad array subscript
",
                    self.assignment_error_prefix(),
                    word
                );
                self.emit_assignment_diag(line);
            }
            if !bare_elements.is_empty() {
                self.exit_code = 1;
            }
            element_values_converted = true;
            stored
        } else if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
        {
            // GNU arrayfunc.c:454-500 find_or_make_array_variable: a compound
            // `name=(...)` assignment on a non-array variable CONVERTS it to
            // an indexed array (convert_var_to_array, arrayfunc.c:498)
            // unconditionally — there is no "single arithmetic expression
            // stays scalar" carve-out in 5.2 or 5.3 (rubash#246: both
            // /usr/local/bin/bash 5.3.0 and /usr/bin/bash 5.2.21 print
            // `declare -ai t=([0]="6")` for `declare -i t; t=(2*3)`), and
            // the integer attribute still evaluates each element
            // (arrayfunc.c:526-552 assign_array_var_from_word_list ->
            // bind_array_var_internal -> make_variable_value, so
            // `x=([0]=7+11)` gives x[0]=18).
            match append_array_value(
                "()",
                None,
                &value,
                base_name,
                is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name),
                self.shell_state.env_vars.get("IFS").map(String::as_str),
                &self.shell_state.env_vars,
                &self.shell_state.cwd,
            ) {
                Ok(outcome) => {
                    element_values_converted = true;
                    bad_subscript = outcome.bad_subscript.clone();
                    render_indexed_outcome("()", outcome)
                }
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
        let value = if element_values_converted {
            value
        } else {
            self.apply_case_assignment_attributes(base_name, value)
        };
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
        // GNU variables.c:6291 set_pipestatus_array gates the status
        // writer on the CELL's array_p, and PIPESTATUS has no standing
        // cell: a scalar bind creates a scalar cell unless the current
        // cell already holds an array object (LiveArray) or was
        // declared one (`declare -a`, ValuelessArray — bind_variable
        // then writes element 0 and the writer takes the cell over).
        // The static ARRAY_VARS mark init sets names the nominal type
        // for the expansion views; on its own it must not turn
        // `PIPESTATUS=99` into an element-0 write (rubash#1364).
        let pipestatus_cell_is_array = base_name == "PIPESTATUS"
            && matches!(
                self.shell_state.pipestatus_shape,
                crate::shell::state::PipestatusShape::LiveArray
                    | crate::shell::state::PipestatusShape::ValuelessArray
            );
        let is_array = compound_assignment
            || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
            || (is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, base_name)
                && (base_name != "PIPESTATUS" || pipestatus_cell_is_array));
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
            // rubash#616 (1a): the scalar-over-array bind rewrites element 0
            // (GNU variables.c:3128-3142 bind_variable_internal keeps the
            // array) — refresh the structured cell from the same entries the
            // render uses.
            sync_typed_indexed_array(&mut self.shell_state.variables, base_name, &entries);
            let storage = format_indexed_array_storage(entries);
            self.shell_state
                .env_vars
                .insert(base_name.to_string(), storage);
            // Cell refreshed from the same entries the render used
            // (rubash#1510).
            self.note_typed_cell_synced(base_name);
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
                    current.clone_from(&value);
                }
            } else {
                let _ = self
                    .shell_state
                    .variables
                    .set_scalar(base_name, value.clone());
            }
            // rubash#616 (1a): a scalar bind is mirror-authoritative — a
            // lingering structured cell (marker/typed divergence) must not
            // shadow it.
            drop_typed_indexed_array(&mut self.shell_state.variables, base_name);
            drop_typed_assoc_array(&mut self.shell_state.variables, base_name);
        }
        // GNU variables.c:3139-3140: for an assoc array, a scalar assignment
        // stores the value at key "0" via assign_func. Without this, a value
        // like `([a]=1)` is stored raw and later misinterpreted as assoc
        // storage format (assoc.tests:191 T='([a]=1)' -> ${T[@]} is `([a]=1)`,
        // not `1`). The bind MERGES into the existing entries like the
        // indexed-array arm above (bind_variable_internal keeps the other
        // elements): building a fresh `(["0"]=v)` storage here silently
        // dropped every pre-existing key (rubash#1347).
        if !compound_assignment
            && !append
            && is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
            && !value.starts_with(STORAGE_WORD_PREFIX)
        {
            let current = self
                .shell_state
                .env_vars
                .get(base_name)
                .cloned()
                .unwrap_or_default();
            let storage = append_assoc_scalar_value(&current, &value);
            // Assoc scalar bind stores at key "0" — keep the structured cell
            // in lockstep from the same render.
            sync_typed_assoc_storage(&mut self.shell_state.variables, base_name, &storage);
            self.shell_state
                .env_vars
                .insert(base_name.to_string(), storage);
            // Cell synced from the same render (rubash#1510).
            self.note_typed_cell_synced(base_name);
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "allexport") {
                self.mark_exported(base_name);
            }
            self.exit_code = 0;
            return true;
        }
        // rubash#616 (1a): final mirror write — a compound `a=(...)` render
        // syncs the structured cell, any other shape invalidates it (the
        // !is_array scalar bind above already handled the plain case).
        // Phase 1c: the append branch may already have written the mirror and
        // cell in place, in which case that write is skipped and only the
        // process-env projection (exported / special names) still runs.
        if !mirror_written_in_place {
            sync_typed_indexed_storage(&mut self.shell_state.variables, base_name, &value);
            // Assoc leg of the same discipline: a compound `A=(...)` render
            // syncs the structured assoc cell, any other shape drops it so
            // `${A[k]}` falls back to the mirror projection.
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name) {
                sync_typed_assoc_storage(&mut self.shell_state.variables, base_name, &value);
            } else {
                drop_typed_assoc_array(&mut self.shell_state.variables, base_name);
            }
            if let Some(current) = self.shell_state.env_vars.get_mut(base_name) {
                current.clone_from(&value);
            } else {
                self.shell_state
                    .env_vars
                    .insert(base_name.to_string(), value.clone());
            }
        }
        // The block above (or the in-place append it defers to) synced
        // the structured cell from the same render it wrote to the
        // mirror — or dropped the cell, in which case this no-ops:
        // register the pair as synced for the boundary verifier
        // (rubash#1510).
        self.note_typed_cell_synced(base_name);
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "allexport") {
            self.mark_exported(base_name);
        }
        // The in-place append already wrote the mirror, so the process-env
        // projection reads it back; every other path projects the render it
        // just produced.
        let projected = if mirror_written_in_place {
            self.shell_state
                .env_vars
                .get(base_name)
                .map(String::as_str)
                .unwrap_or_default()
        } else {
            value.as_str()
        };
        sync_shell_assignment_process_env(&self.shell_state.env_vars, base_name, projected);
        // rubash#1692: the store above is a bind — GNU's sv_locale
        // watcher fires on it (variables.c:6272).
        self.note_locale_assignment(base_name);
        if let Some(word) = bad_subscript {
            // GNU assign_compound_array_list (arrayfunc.c:804-809): when a
            // negative subscript still lands before the array start,
            // err_badarraysub (error.c:440) names the whole element word
            // (arrayfunc.c:63 bash_badsub_errmsg). It goes through
            // report_error, so — unlike `declare: x: readonly variable` —
            // the message carries no `command:` segment, which is why this
            // uses diagnostic_prefix() and not assignment_diagnostic_prefix().
            // `any_failed` is counted and the list stops — the elements written before it stay stored (the
            // writes just above), exactly as arrayfunc.c:768-772 inserts as it
            // scans. assign_array_var_from_string (arrayfunc.c:913-932) then
            // returns NULL, so the assignment fails with status 1.
            // `raise_evalerror_abort` is the same DISCARD jump the plain
            // `a[-2]=q` path makes: function frames unwind, a subshell ends
            // just itself, and a top-level list ends the noninteractive run.
            // GNU arrayfunc err_badarraysub (report_error family): honors
            // gnu_errfmt (rubash#1038).
            let line = format!("{}{}: bad array subscript\n", self.error_prefix(), word);
            self.emit_assignment_diag(line);
            self.exit_code = 1;
            // Bad-subscript class for the eval boundary (rubash#637), armed
            // alongside the abort the top-level list needs.
            self.evalerror_subscript_pending.set(true);
            self.raise_evalerror_abort();
            return false;
        }
        true
    }

    /// #609 slice 2: marks-free plain scalar bind. GNU subst.c:3524
    /// do_assignment_internal -> variables.c:3225 bind_variable (->:3054)
    /// is one hash insert with the value moved once; rubash pays two
    /// stores plus ten attribute probes. One attr_of probe (variables.h
    /// att_* word) whitelists the shape; identical values skip both
    /// stores, otherwise both are written together so the #616/#618
    /// mirror-must-never-go-stale rule holds. None = fall through.
    ///
    /// F7 widening: admission is by attribute SHAPE plus a closed
    /// registry of names whose bind semantics live INSIDE
    /// apply_shell_assignment_inner itself.
    /// - Attributes: only att_exported may be set, plus ONE further
    ///   shape — att_integer alone (with or without att_exported) when
    ///   the RHS is a pure numeric literal, whose evalexp result the
    ///   lane computes in closed form (eval_integer_literal, after
    ///   expr.c strlong()); every non-literal integer RHS still falls
    ///   through to the real evalexp. Every other bit makes the
    ///   generic tail perform work this lane must not skip
    ///   (att_integer with an expression RHS -> variables.c:2937
    ///   make_variable_value evalexp; att_uppercase / att_lowercase /
    ///   capcase -> the case conversion in the same function;
    ///   att_nameref -> find_variable_nameref resolution; att_array /
    ///   att_assoc -> arrayfunc.c storage machinery; att_readonly ->
    ///   the readonly refusal; att_declared_unset -> the tail's
    ///   DECLARED_UNSET unmark). An exported-only variable's generic
    ///   tail is exactly this lane's dual store: children are served
    ///   from the spawn env built off the EXPORTED mark (#916), and
    ///   set_process_env is gated to the process-env name set, which
    ///   is excluded separately below.
    /// - The name registry below holds only names with an INNER bind
    ///   arm in apply_shell_assignment_inner: dynamic write-back
    ///   (SECONDS / RANDOM / SRANDOM), swallow dynamics and noassign
    ///   arrays (BASHPID, BASH_SUBSHELL, FUNCNAME, LINENO, BASH_COMMAND,
    ///   BASH_ARGC / BASH_ARGV, BASH_LINENO, BASH_SOURCE, PIPESTATUS),
    ///   and the sv_ignoreeof option sync (IGNOREEOF / ignoreeof,
    ///   variables.c:6205). OPTIND is deliberately NOT in it: GNU
    ///   creates OPTIND with att_integer (`declare -i OPTIND="1"`),
    ///   so it reaches this lane through the integer-literal shape
    ///   above, not as a plain name — its only inner act beyond the
    ///   integer evaluation is dropping the __RUBASH_GETOPTS_OFFSET
    ///   shadow, and the sv_optind hook (variables.c:5817) is carried
    ///   by apply_shell_assignment's funnel, which drops BOTH getopts
    ///   shadows after every bind attempt on every path — so the dual
    ///   store of the evaluated value here is the whole observable
    ///   effect of an OPTIND bind.
    ///   (The tempenv shadow gate keeps its own copy of the wider list
    ///   for a different decision — whether a tempvar may shadow at
    ///   all — and is not synced with this registry.)
    /// - allexport no longer excludes: the generic tail's only extra
    ///   act for this shape is applying att_exported after the store
    ///   (GNU bind_variable_internal does the same under
    ///   mark_modified_vars, variables.c:3170), which the lane performs
    ///   on its success exits below.
    fn bind_marks_free_scalar(&mut self, base: &str, value: &str) -> Option<bool> {
        // Attribute shape: no att_* bit other than att_exported — plus
        // one shape more: att_integer ALONE (with or without exported)
        // whose RHS is a pure numeric literal. A GNU integer bind runs
        // evalexp (variables.c:2937-2946); for a literal-only RHS that
        // evaluation is a closed-form digit accumulation, reproduced
        // exactly by eval_integer_literal below (checked i64: overflow
        // or any non-literal shape falls through to the generic evalexp
        // path, so expression shapes, name references and every
        // error/DISCARD shape keep the generic behaviour bit-for-bit).
        // This is also OPTIND's lane: GNU creates OPTIND with
        // att_integer (`declare -i OPTIND="1"`), so `OPTIND=1` is an
        // integer-literal bind, not a plain one.
        let mut non_export_attrs = self.shell_state.env_vars.attr_of(base);
        non_export_attrs.exported = false;
        let integer_literal = if non_export_attrs.integer {
            let mut only_integer = non_export_attrs;
            only_integer.integer = false;
            if only_integer == VarAttrs::default() {
                Self::eval_integer_literal(value)
            } else {
                None
            }
        } else {
            None
        };
        if base.contains('[')
            || value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
            || value.starts_with(STORAGE_WORD_PREFIX)
            || matches!(
                base,
                "IGNOREEOF"
                    | "ignoreeof"
                    | "SECONDS"
                    | "RANDOM"
                    | "SRANDOM"
                    | "BASHPID"
                    | "BASH_SUBSHELL"
                    | "FUNCNAME"
                    | "LINENO"
                    | "BASH_COMMAND"
                    | "BASH_ARGC"
                    | "BASH_ARGV"
                    | "BASH_LINENO"
                    | "BASH_SOURCE"
                    | "PIPESTATUS"
            )
            || shell_assignment_needs_process_env(base)
            || (non_export_attrs != VarAttrs::default() && integer_literal.is_none())
            || is_marked_var(&self.shell_state.env_vars, CAPCASE_VARS, base)
        {
            return None;
        }
        let allexport =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "allexport");
        self.pending_scalar_assignment = false;
        // For an integer-literal bind the stored form is the evaluated
        // canonical decimal, exactly what the generic tail stores; the
        // skip/store logic below then runs on that value unchanged.
        let canonical_value;
        let value = match integer_literal {
            Some(evaluated) => {
                canonical_value = evaluated.to_string();
                canonical_value.as_str()
            }
            None => value,
        };
        // Identical rebinds skip both stores. The cell flag rides along:
        // a readonly rebind still falls through to the generic bind's
        // diagnostic even when the value is unchanged (GNU
        // variables.c:3258 bind_variable refuses before comparing).
        if self
            .shell_state
            .env_vars
            .get(base)
            .is_some_and(|cur| cur == value)
            && matches!(self.shell_state.variables.get(base), Some(v) if !v.readonly && matches!(&v.value, crate::shell::ShellValue::Scalar(cur) if cur == value))
        {
            if allexport {
                self.mark_exported(base);
            }
            return Some(true);
        }
        match self.shell_state.variables.get_mut(base) {
            Some(v) => {
                // The in-place write below skips VariableStore::set, so
                // enforce its readonly refusal here (the word above should
                // already have excluded it).
                if v.readonly {
                    return None;
                }
                match &mut v.value {
                    crate::shell::ShellValue::Scalar(cur) => {
                        cur.clear();
                        cur.push_str(value);
                    }
                    _ => return None,
                }
            }
            None => {
                if self
                    .shell_state
                    .variables
                    .set(base.to_string(), crate::shell::Variable::scalar(value))
                    .is_err()
                {
                    return None;
                }
            }
        }
        if let Some(cur) = self.shell_state.env_vars.get_mut(base) {
            cur.clear();
            cur.push_str(value);
        } else {
            self.shell_state
                .env_vars
                .insert(base.to_string(), value.to_string());
        }
        if allexport {
            self.mark_exported(base);
        }
        Some(true)
    }

    /// Closed-form evalexp for a PURE numeric literal RHS — the only
    /// integer-bind shape the scalar lane evaluates itself. The grammar
    /// is GNU expr.c `strlong()`'s single-literal token: optional
    /// surrounding whitespace, one optional sign, then decimal /
    /// 0-octal / 0x-hex / base#digits with strlong's digit values
    /// (bases <= 36 read letters case-insensitively; above that
    /// a-z = 10..35, A-Z = 36..61, @ = 62, _ = 63) and its base range
    /// (2..=64, written in plain decimal). Accumulation is CHECKED
    /// i64: overflow, an invalid digit for the base, an out-of-range
    /// or leading-zero base, or ANY other shape (names, operators,
    /// internal whitespace, empty) returns None, and the generic
    /// evalexp path then reproduces the true result or error — so
    /// this can only ever skip work, never change an outcome.
    fn eval_integer_literal(value: &str) -> Option<i64> {
        fn digit_value(c: char, base: i64) -> Option<i64> {
            let v = match c {
                '0'..='9' => i64::from(c as u32 - '0' as u32),
                'a'..='z' => 10 + i64::from(c as u32 - 'a' as u32),
                'A'..='Z' => {
                    if base <= 36 {
                        10 + i64::from(c as u32 - 'A' as u32)
                    } else {
                        36 + i64::from(c as u32 - 'A' as u32)
                    }
                }
                '@' => 62,
                '_' => 63,
                _ => return None,
            };
            (v < base).then_some(v)
        }
        fn accumulate(digits: &str, base: i64) -> Option<i64> {
            let mut acc: i64 = 0;
            for c in digits.chars() {
                acc = acc.checked_mul(base)?.checked_add(digit_value(c, base)?)?;
            }
            Some(acc)
        }
        let text = value.trim_matches(|c: char| c.is_ascii_whitespace());
        let (negative, rest) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        if rest.is_empty() {
            return None;
        }
        let magnitude = if let Some((base_text, digits)) = rest.split_once('#') {
            if digits.is_empty()
                || base_text.is_empty()
                || !base_text.bytes().all(|b| b.is_ascii_digit())
                || (base_text.len() > 1 && base_text.starts_with('0'))
            {
                return None;
            }
            let base: i64 = base_text.parse().ok()?;
            if !(2..=64).contains(&base) {
                return None;
            }
            accumulate(digits, base)?
        } else if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            if hex.is_empty() {
                return None;
            }
            accumulate(hex, 16)?
        } else if rest.len() > 1 && rest.starts_with('0') {
            accumulate(rest, 8)?
        } else {
            accumulate(rest, 10)?
        };
        if negative {
            magnitude.checked_neg()
        } else {
            Some(magnitude)
        }
    }

    /// rubash#616 (plan Phase 1c): apply an append-only compound write
    /// (`a+=(...)`) straight to the canonical mirror and the structured cell.
    /// The decode was seeded from that same cell (`indexed_array_cell`), so
    /// `outcome.appended` is exactly the tail GNU's assign_compound_array_list
    /// (arrayfunc.c:700) inserted past `array_max_index` (arrayfunc.c:743);
    /// the mirror therefore grows by those segments alone.
    ///
    /// Returns `false` — leaving the mirror and the cell untouched — when the
    /// cell is absent or readonly, or when the mirror is not the canonical
    /// `STORAGE_WORD_PREFIX(...)` render the in-place segment patch requires,
    /// so every other shape falls back to `render_indexed_outcome`. The mirror
    /// is extended BEFORE the cell: if the cell write is then refused, the
    /// caller's fallback re-renders and re-syncs from the mirror, landing on
    /// the correct value rather than a stale one.
    fn apply_compound_append_in_place(
        &mut self,
        name: &str,
        outcome: &super::arrays::IndexedAppendOutcome,
    ) -> bool {
        if outcome.appended.is_empty() {
            return false;
        }
        let Some((len, _)) = self.shell_state.variables.indexed_array_shape(name) else {
            return false;
        };
        if self
            .shell_state
            .variables
            .get(name)
            .is_some_and(|variable| variable.readonly)
        {
            return false;
        }
        if !append_indexed_mirror_segments(
            &mut self.shell_state.env_vars,
            name,
            outcome
                .appended
                .iter()
                .map(|(index, value)| (*index, value.as_str())),
        ) {
            return false;
        }
        for (offset, (index, value)) in outcome.appended.iter().enumerate() {
            let typed_index = i64::try_from(*index).unwrap_or(i64::MAX);
            if !self.shell_state.variables.extend_indexed_element(
                name,
                typed_index,
                value,
                len + offset + 1,
            ) {
                return false;
            }
        }
        true
    }
}

/// rubash#616 (plan Phase 1c): the render for an `append_array_value` outcome
/// that was NOT applied in place. A pure append carries no full element map
/// (see `IndexedAppendOutcome::entries`), so the map is rebuilt from the
/// mirror `current` and the appended tail is folded back in — byte-identical
/// to what the scan would have rendered itself.
fn render_indexed_outcome(current: &str, outcome: super::arrays::IndexedAppendOutcome) -> String {
    match outcome.entries {
        Some(entries) => format_indexed_array_storage(entries),
        None => {
            let mut entries = indexed_array_entries(current);
            entries.extend(outcome.appended);
            format_indexed_array_storage(entries)
        }
    }
}
