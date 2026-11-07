use super::*;

impl Executor {
    pub(in crate::executor) fn execute_unset(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU execute_cmd.c:4370 fix_arrayref_words: a word that is a valid
        // array reference at the SYNTACTIC level (flag-0
        // valid_array_reference on the unexpanded token — `name[` outside
        // quotes with a matched `]` at end) gets W_ARRAYREF. unset's
        // builtin_arrayref_flags (builtins/common.c:1040) then gives it
        // VA_ONEWORD|VA_NOEXPAND unconditionally, so the element subscript is
        // bound verbatim. A `[` inside quotes (`"dict[$k]"`) or single
        // quotes (`'dict[$k]'`) does not count — the expanded operand falls
        // back to the flag-0/VA_NOEXPAND validity scan.
        let arrayref_flags: Vec<bool> = (1..cmd.words.len())
            .map(|index| {
                cmd.word_metadata.get(index).is_some_and(|metadata| {
                    crate::executor::subscript_expansion::valid_array_reference_env(
                        &metadata.raw,
                        false,
                        false,
                        &self.shell_state.env_vars,
                    )
                })
            })
            .collect();

        // GNU execute_cmd.c:5606 (execute_builtin_or_function):
        // do_redirections binds the command's fds exactly ONCE, before
        // the builtin body runs, and builtin_error (builtins/common.c:98)
        // then writes to the bound fd 2 — an enclosing `exec 2>...`
        // binding, a dup (`2>&1`, the rubash#271 case), or a capture
        // contains it. rubash's single application is the ordered
        // buffered route below: the Diag speaks into a buffered stderr
        // and write_buffered_builtin_output resolves cmd's redirect
        // state once (dup/closed targets are fd-state resolutions,
        // never filename opens — a `&`-prefixed target must not become
        // a bogus file literally named `&1`, rubash#271). A target
        // that cannot be opened is settled before the body runs by
        // the dispatch pregate (probe_output_redirect), the same
        // suppression GNU's failed do_redirections produces.
        //
        // Regression shape this replaces: the pre-migration branches
        // opened a plain `2>file` target directly and returned; the
        // Phase 4 migration kept those branches but appended the
        // buffered route for the new stdout buffer (set.def
        // CASE_HELPOPT long help prints to stdout), so the route
        // re-applied the redirect — its second, truncating open
        // erased the diagnostic the Diag had just flushed into the
        // first handle, and `unset uv 2>f` lost it entirely (append
        // targets survived: no truncation). Every other migrated
        // family member (declare/readonly/export/local/alias) runs
        // this buffered shape; unset now does too (A3-R1, rubash#2707).
        let mut stdout_buf: Vec<u8> = Vec::new();
        let mut stderr = Vec::new();
        let status = self.execute_unset_with_stderr(
            &cmd.words[1..],
            &arrayref_flags,
            &mut stdout_buf,
            &mut stderr,
        )?;
        let _ = self.write_buffered_builtin_output(cmd, &stdout_buf, &stderr);
        Ok(status)
    }

    pub(in crate::executor) fn execute_unset_with_stderr<S>(
        &mut self,
        args: &[String],
        arrayref_flags: &[bool],
        stdout: &mut S,
        stderr: &mut dyn crate::executor::builtin_diag::DiagSink,
    ) -> Result<i32, ExecuteError>
    where
        S: Write,
    {
        // TODO(builtins/set.def/variables.c/execute_cmd.c): `unset` searches
        // variables and functions with nuanced attributes. Keep function table
        // and variable table behavior aligned for builtins6.sub.
        if unset_args_need_builtin_diagnostics(args) {
            let status = crate::builtins::set::unset_with_stderr(
                args.iter().map(String::as_str),
                &mut self.shell_state.env_vars,
                stdout,
                stderr,
            )?;
            // Convert error statuses > EX_SHERRBASE (256) and set
            // special_builtin_failed (GNU execute_cmd.c:4886-4888).
            return Ok(if status > 256 {
                self.special_builtin_failed.set(true);
                match status {
                    258..=259 => 2,
                    _ => 1,
                }
            } else {
                status
            });
        }

        // The Diag's sink is the stderr the dispatcher bound for this
        // command (its redirect file, the sink, or the buffered Vec),
        // so every `unset:` diagnostic lands in the command's own fd 2
        // (A3-R1, rubash#2707) — including the bad-subscript lines
        // `unset_array_element` used to write to the process stderr.
        // Built after the branch above: that path builds its own Diag
        // over the same sink inside the builtin.
        let row = crate::executor::builtin_table::builtin_table_row("unset").expect("unset row");
        let mut diag = crate::executor::builtin_diag::Diag::new(row, stderr);

        let function_only = args.iter().any(|arg| arg == "-f");
        let variable_only = args.iter().any(|arg| arg == "-v");
        // GNU builtins/set.def:866-867: `unset -f` cancels -n, and -n is
        // only meaningful for variables anyway.
        let nameref_only = args.iter().any(|arg| arg == "-n") && !function_only;
        // Operand words may carry the in-band W_ARRAYREF flag
        // (ARRAYREF_FLAG prefix — arrayref.rs); the syntactic flag was
        // already captured in `arrayref_flags` above, so strip the prefix
        // here before any name/subscript consumer sees it.
        // Borrowed names (no per-operand String alloc): the strip above
        // only trims the in-band ARRAYREF flag prefix, so the result
        // borrows `args` for the rest of this call.
        let names: Vec<(usize, &str)> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| !arg.starts_with('-'))
            .map(|(index, arg)| (index, crate::builtins::arrayref::take_arrayref_flag(arg).1))
            .collect();

        let mut function_status = 0;
        if !variable_only {
            for (_, name) in &names {
                let variable_exists = self.shell_state.env_vars.contains_key(*name)
                    || self.shell_state.variables.get(*name).is_some();
                if !function_only && variable_exists {
                    continue;
                }
                if marked_env_names(&self.shell_state.env_vars, READONLY_FUNCTIONS)
                    .iter()
                    .any(|readonly| readonly.as_str() == *name)
                {
                    diag.free(format_args!("{name}: cannot unset: readonly function"));
                    function_status = 1;
                    continue;
                }
                self.shell_state.functions.remove(*name);
                self.shell_state.function_definition_redirects.remove(*name);
                self.shell_state.function_def_infos.remove(*name);
                unmark_env_name(&mut self.shell_state.env_vars, EXPORTED_FUNCTIONS, name);
                unmark_env_name(
                    &mut self.shell_state.env_vars,
                    UNSTREAMED_FUNCTION_BODIES,
                    name,
                );
            }
        }

        if function_only {
            diag.flush_with(|| self.diagnostic_prolog());
            return Ok(function_status);
        }

        // Fall-through operand names, keyed by their position in `args`:
        // the lib re-scan below must see the arguments in the WRITTEN
        // order (assembled after this loop), not with every `-` word
        // hoisted ahead of the operands.
        let mut pushed: Vec<Option<&str>> = vec![None; args.len()];
        // GNU set.def:930-931/965-966: each failed name increments
        // posix_utility_error, so the first failure is reported while the
        // remaining names are still processed.
        let mut nameref_status = 0;
        let mut element_status = 0;
        for (arg_index, name) in names {
            // GNU builtins/set.def:925-968 + 1024 with nameref=1: the
            // non-unsettable and readonly checks run against
            // find_variable_last_nameref (the chain's last nameref, or the
            // variable itself), while the unbind is unbind_nameref
            // (variables.c:3807-3815) — it removes NAME only when NAME is
            // itself a nameref, so scalars and `a[sub]` names are a silent
            // no-op. Element/nameref-target unbinding must not run here.
            if nameref_only {
                if let Some(checked) = self.last_nameref_for_unset(&name) {
                    if matches!(checked.as_str(), "BASH_LINENO" | "BASH_SOURCE") {
                        diag.free(format_args!("{name}: cannot unset"));
                        nameref_status = 1;
                        continue;
                    }
                    if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &checked) {
                        diag.free(format_args!("{checked}: cannot unset: readonly variable"));
                        nameref_status = 1;
                        continue;
                    }
                }
                if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, &name)
                    && !self.unset_outer_local_variable(&name)
                {
                    self.shell_state.env_vars.remove(&name);
                    crate::executor::local_helpers::mirror_env_remove(&name);
                    self.shell_state.variables.remove(&name);
                    for key in [
                        EXPORTED_VARS,
                        READONLY_VARS,
                        ARRAY_VARS,
                        ASSOC_VARS,
                        ASSOC_128_VARS,
                        INTEGER_VARS,
                        UPPERCASE_VARS,
                        LOWERCASE_VARS,
                        NAMEREF_VARS,
                        DECLARED_UNSET_VARS,
                    ] {
                        unmark_env_name(&mut self.shell_state.env_vars, key, &name);
                    }
                }
                continue;
            }
            // GNU builtins/set.def:887-917: tokenize_array_reference(name,
            // vflags) runs before the identifier check. A `[`-shaped operand
            // that fails it is not an array reference at all — under
            // `unset -v` it reports `not a valid identifier` (sh_invalidid +
            // posix_utility_error), while a plain `unset` silently treats it
            // as a potential function name. A W_ARRAYREF word (syntactic
            // `name[` on the raw token) gets VA_ONEWORD|VA_NOEXPAND via
            // builtin_arrayref_flags (builtins/common.c:1040) and binds the
            // subscript verbatim.
            if name.contains('[') {
                let arrayref = arrayref_flags.get(arg_index).copied().unwrap_or(false);
                if arrayref {
                    // GNU builtins/set.def:924 find_variable follows the
                    // nameref chain, so `unset n[0]' with n -> v resolves the
                    // element against the REFERENCED array (nameref15.sub:
                    // v[0] is unbound and n survives) — resolve before the
                    // element path consumes the operand.
                    if let Some(status) = self.unset_through_nameref(&name, &mut diag) {
                        element_status = element_status.max(i32::from(status));
                        continue;
                    }
                    if let Some(status) = self.unset_array_element(&name, true, &mut diag) {
                        element_status = element_status.max(i32::from(status));
                        continue;
                    }
                }
                let noexpand = crate::builtins::shopt::option_enabled(
                    &self.shell_state.env_vars,
                    "array_expand_once",
                );
                if !crate::executor::subscript_expansion::valid_array_reference_env(
                    &name,
                    noexpand,
                    false,
                    &self.shell_state.env_vars,
                ) {
                    if variable_only {
                        diag.error(crate::executor::builtin_diag::DiagKind::InvalidId(
                            name.to_string(),
                        ));
                        element_status = element_status.max(1);
                    }
                    continue;
                }
            }
            // GNU builtins/set.def:927-935 + 990-1010 (unset_builtin): a
            // subscripted name whose base is a nameref unbinds the referenced
            // array's element (unset n[0] with n->v removes v[0]); a plain
            // nameref whose cell is an array reference unbinds that element
            // while keeping the nameref itself.
            if let Some(status) = self.unset_through_nameref(&name, &mut diag) {
                element_status = element_status.max(i32::from(status));
                continue;
            }
            // GNU set.def:887 + unbind_array_element: a non-W_ARRAYREF word
            // still gets base_vflags = VA_NOEXPAND under array_expand_once,
            // so its subscript unbinds verbatim; only VA_ONEWORD (the last
            // `]` tokenize) is exclusive to W_ARRAYREF.
            let verbatim = crate::builtins::shopt::option_enabled(
                &self.shell_state.env_vars,
                "array_expand_once",
            );
            if let Some(status) = self.unset_array_element(&name, verbatim, &mut diag) {
                element_status = element_status.max(i32::from(status));
                continue;
            }
            if self.unset_outer_local_variable(&name) {
                continue;
            }
            // GNU builtins/set.def:928 + variables.h:161 non_unsettable_p:
            // att_nounset dynamic variables refuse unbinding outright
            // (BASH_SOURCE/BASH_LINENO/BASH_ARGC/BASH_ARGV report
            // "cannot unset"). All other dynamic variables unbind like
            // ordinary ones (variables.c:3839 unbind_variable) — they do
            // not re-materialize, and a later assignment binds a plain
            // variable. Rubash materializes dynamic values on demand, so
            // record the unbind in UNSET_DYNAMIC_VARS; readonly names
            // (SHELLOPTS/BASHOPTS) keep their attribute and fall through
            // to the readonly diagnostic below.
            if matches!(
                name,
                "BASH_SOURCE" | "BASH_LINENO" | "BASH_ARGC" | "BASH_ARGV"
            ) {
                diag.free(format_args!("{name}: cannot unset"));
                element_status = element_status.max(1);
                continue;
            }
            if !is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &name)
                && (self.dynamic_parameter_is_set(&name)
                    // DIRSTACK's dynamic value is not served by
                    // dynamic_parameter_is_set (its faces are the
                    // carrier-derived array views), but its unbind is
                    // the same variables.c:3839 act and must be recorded
                    // the same way, or the braced faces keep reading the
                    // carrier after the unset (rubash#2434). A live
                    // local/tempenv shadow owns the name: unsetting it
                    // removes the shadow, not the global dynamic var.
                    || (name == "DIRSTACK" && !self.dirstack_shadowed()))
            {
                mark_env_name(&mut self.shell_state.env_vars, UNSET_DYNAMIC_VARS, &name);
            }
            pushed[arg_index] = Some(name);
        }

        // GNU set.def internal_getopt stops at the first operand: a `-`
        // word after an operand is a NAME, not an option (`unset uv
        // --help` unsets uv silently; `unset -v uv --help` reports
        // `` `--help': not a valid identifier ``). The old code hoisted
        // every `-` word ahead of the operands, so `--help`/`-x` after
        // an operand were misread as options in the lib re-scan below.
        // Rebuild the lib's argument list in the WRITTEN order: the
        // option-region words (minus `-f`, whose functions the loop
        // above already handled), the fall-through operand names at
        // their own positions, and post-operand `-` words verbatim as
        // names. Operands the loop handled upstream (nameref/array
        // element) leave an empty slot — the lib must not re-unset
        // the whole variable. Borrowed names only; every later
        // consumer only reads them.
        let mut variable_args: Vec<&str> = Vec::new();
        let mut seen_operand = false;
        for (index, arg) in args.iter().enumerate() {
            if !seen_operand && arg.starts_with('-') {
                if arg.as_str() != "-f" {
                    variable_args.push(arg.as_str());
                }
                if arg.as_str() == "--" {
                    seen_operand = true;
                }
                continue;
            }
            seen_operand = true;
            if let Some(name) = pushed[index] {
                variable_args.push(name);
            } else if arg.starts_with('-') {
                variable_args.push(arg.as_str());
            }
        }

        // Flush first: the builtin builds its own Diag over the
        // same sink, and its lines must follow the ones recorded
        // above in the byte stream.
        diag.flush_with(|| self.diagnostic_prolog());
        let variable_status = {
            crate::builtins::set::unset_with_stderr(
                variable_args.iter().copied(),
                &mut self.shell_state.env_vars,
                stdout,
                diag.sink(),
            )
            .map_err(ExecuteError::from)?
        };
        // GNU builtins/set.def:995-1000: unsetting through a nameref removes
        // the referenced variable and keeps the nameref; drop the referenced
        // variable from the typed owner too so parameter expansion does not
        // see a stale value (unset foo with foo->bar must clear bar).
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
                if let Some(cell) = self
                    .shell_state
                    .env_vars
                    .get(*name)
                    .filter(|cell| is_shell_name(cell))
                    .cloned()
                {
                    self.shell_state.variables.remove(cell.as_str());
                }
            }
        }
        // Also remove from shell_state.variables so that shell_variable_value
        // does not return a stale value for an unset variable (the builtin
        // only cleans env_vars).  Without this, ${var-word} after
        // `var=""; unset var` still sees the old empty value.
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            self.shell_state.variables.remove(name);
        }
        // GNU variables.c:3839 unbind_variable removes the PIPESTATUS
        // cell outright (any shape — live array, scalar, valueless), so
        // the next command completion's set_pipestatus_array recreates a
        // live array from absent (rubash#1363). A readonly PIPESTATUS
        // keeps its cell: unset_with_stderr refused the unbind above and
        // the READONLY mark survives, which is the guard here.
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            if *name == "PIPESTATUS"
                && !is_marked_var(&self.shell_state.env_vars, READONLY_VARS, "PIPESTATUS")
            {
                self.shell_state.pipestatus_shape = crate::shell::state::PipestatusShape::Absent;
            }
        }
        // GNU variables.c:6205-6217 sv_ignoreeof: unbinding the IGNOREEOF
        // variable turns the ignoreeof option off (the option tracks
        // "the variable is set").
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            if matches!(*name, "IGNOREEOF" | "ignoreeof") {
                crate::builtins::set::sync_shell_option_flag(
                    &mut self.shell_state.env_vars,
                    "ignoreeof",
                    false,
                );
            }
        }
        let raw_status = if nameref_status != 0 {
            nameref_status
        } else if function_status != 0 {
            function_status
        } else {
            variable_status
        }
        .max(element_status);
        // GNU execute_cmd.c:4886-4888: builtin_status converts error statuses
        // (> EX_SHERRBASE = 256) to the final exit code, and sets
        // special_builtin_failed for special builtins. EX_USAGE (258) → 2,
        // EX_UTILERROR (263) → 1 (EXECUTION_FAILURE).
        diag.flush_with(|| self.diagnostic_prolog());
        if raw_status > 256 {
            self.special_builtin_failed.set(true);
            let converted = match raw_status {
                258..=259 => 2, // EX_USAGE, EX_BADSYNTAX → EX_BADUSAGE
                _ => 1,         // EX_UTILERROR, etc. → EXECUTION_FAILURE
            };
            Ok(converted)
        } else {
            Ok(raw_status)
        }
    }

    /// GNU variables.c:2051-2075 find_variable_last_nameref (as used by
    /// `unset -n`, builtins/set.def:925): the variable the nounset/readonly
    /// checks run against — the last nameref in the chain, or NAME itself
    /// when it is not a nameref. Returns None when NAME does not exist or
    /// the chain hits an empty nameref cell (variables.c:2065-2066 returns
    /// NULL with vflags=0), which is why GNU silently unbinds a readonly
    /// nameref whose cell is empty.
    fn last_nameref_for_unset(&self, name: &str) -> Option<String> {
        if !self.shell_state.env_vars.contains_key(name)
            && self.shell_state.variables.get(name).is_none()
        {
            return None;
        }
        if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
            return Some(name.to_string());
        }
        let mut last = name.to_string();
        let mut seen = HashSet::from([last.clone()]);
        // GNU variables.h:181 NAMEREF_MAX.
        for _ in 0..8 {
            // GNU variables.c:2064-2066: a missing or empty nameref cell
            // ends the search with NULL (vflags=0), so the caller skips the
            // nounset/readonly checks yet still unbinds NAME itself.
            let Some(cell) = self.shell_state.env_vars.get(&last) else {
                return None;
            };
            if cell.is_empty() {
                return None;
            }
            if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, cell) {
                break;
            }
            if !seen.insert(cell.clone()) {
                return None;
            }
            last = cell.clone();
        }
        Some(last)
    }

    /// GNU builtins/set.def:990-1010 (unset_builtin): `unset -v` of a
    /// nameref whose cell is an array reference unbinds the referenced
    /// element and keeps the nameref; a subscripted name whose base is a
    /// nameref resolves the subscript against the referenced array
    /// (nameref3/nameref15.sub). Returns Some(status) when this call handled
    /// the operand and the ordinary variable path must be skipped.
    pub(in crate::executor) fn unset_through_nameref(
        &mut self,
        name: &str,
        diag: &mut crate::executor::builtin_diag::Diag,
    ) -> Option<u8> {
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
            // GNU builtins/set.def:925/990-1014: `unset` of a nameref walks to
            // the LAST nameref in the chain (find_variable_last_nameref) and
            // unbinds its cell -- a plain-name cell unbinds that variable, an
            // `x[i]` cell unbinds the element. Intermediate namerefs are
            // kept (nameref15.sub: `unset a` on a->b->a[1] keeps both).
            let mut last = name.to_string();
            let mut seen = HashSet::from([name.to_string()]);
            for _ in 0..8 {
                let Some(cell) = self.shell_state.env_vars.get(&last).cloned() else {
                    break;
                };
                if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, &cell)
                    || !seen.insert(cell.clone())
                {
                    break;
                }
                last = cell;
            }
            let cell = self
                .shell_state
                .env_vars
                .get(&last)
                .cloned()
                .unwrap_or_default();
            if parse_array_subscript(&cell).is_some() {
                return self.unset_array_element(&cell, false, diag).or(Some(0));
            }
            // GNU set.def:936-937 + 962-966: the resolved referent is
            // unbound by name; a readonly referent is an error on the
            // referent's name, and a missing referent is a silent no-op.
            if !cell.is_empty() && is_shell_name(&cell) {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &cell) {
                    diag.free(format_args!("{cell}: cannot unset: readonly variable"));
                    return Some(1);
                }
                self.shell_state.env_vars.remove(&cell);
                crate::executor::local_helpers::mirror_env_remove(&cell);
                self.shell_state.variables.remove(&cell);
                for key in [
                    EXPORTED_VARS,
                    READONLY_VARS,
                    ARRAY_VARS,
                    ASSOC_VARS,
                    ASSOC_128_VARS,
                    INTEGER_VARS,
                    UPPERCASE_VARS,
                    LOWERCASE_VARS,
                    NAMEREF_VARS,
                    DECLARED_UNSET_VARS,
                ] {
                    unmark_env_name(&mut self.shell_state.env_vars, key, &cell);
                }
                return Some(0);
            }
            return None;
        }
        let Some((base, subscript)) = parse_array_subscript(name) else {
            return None;
        };
        if !is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base) {
            return None;
        }
        let Some(cell) = self
            .shell_state
            .env_vars
            .get(base)
            .filter(|cell| is_shell_name(cell))
        else {
            return None;
        };
        let cell = cell.clone();
        self.unset_array_element(&format!("{cell}[{subscript}]"), false, diag)
            .or(Some(0))
    }

    pub(in crate::executor) fn unset_outer_local_variable(&mut self, name: &str) -> bool {
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
            return false;
        }
        let Some(current_scope_index) = self.shell_state.local_var_scopes.len().checked_sub(1)
        else {
            return false;
        };
        let Some(scope_index) = self.visible_local_scope_index(name) else {
            return false;
        };
        // GNU variables.c:3984-4001 makunbound: unsetting a local in its own
        // context (context == variable_context) — or, with localvar_unset,
        // an outer-context local — disposes the value and reinserts the
        // binding invisible in its owning context. The live binding
        // disappears (lookups report unset instead of falling through to the
        // outer binding), `declare -p` still reports the name's attributes,
        // the local attribute survives a later reassignment, and the frame's
        // saved snapshot still restores the pre-local value at function
        // end. The frame snapshots stay untouched.
        if scope_index == current_scope_index
            || crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "localvar_unset")
        {
            // GNU resets the attributes wholesale — att_local +
            // att_invisible — preserving att_exported only when the local
            // came from a temporary environment (exported_p && tempvar_p).
            // A live tempenv binding of the same name is the provenance
            // marker (`v=t f` where f runs `local v=x` keeps -x; an exported
            // outer variable's local copy does not).
            let keep_exported = is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name)
                && self.tempenv_names.iter().any(|tempenv| tempenv == name);
            self.shell_state.env_vars.remove(name);
            crate::executor::local_helpers::mirror_env_remove(name);
            self.shell_state.variables.remove(name);
            let mut attrs = VarAttrs {
                declared_unset: true,
                ..VarAttrs::default()
            };
            attrs.exported = keep_exported;
            set_var_attrs(&mut self.shell_state.env_vars, name, attrs);
            // GNU variables.c:6205-6217 sv_ignoreeof: unbinding the variable
            // drives the ignoreeof option off.
            if matches!(name, "IGNOREEOF" | "ignoreeof") {
                crate::builtins::set::sync_shell_option_flag(
                    &mut self.shell_state.env_vars,
                    "ignoreeof",
                    false,
                );
            }
            return true;
        }
        let previous = self.shell_state.local_var_scopes[scope_index].remove(name);
        let attrs = self.shell_state.local_attr_scopes[scope_index]
            .remove(name)
            .unwrap_or_default();
        // GNU variables.c:3959-3980 makunbound: the binding is removed at
        // the context that holds it, so the pre-local binding becomes
        // visible again. The typed owner needs the same restore — leaving
        // the local's stale cell makes `${res-word}` report the variable as
        // still set (varenv10.sub inner/outer).
        let typed_previous = self
            .shell_state
            .local_typed_scopes
            .get_mut(scope_index)
            .and_then(|scope| scope.remove(name));
        restore_optional_shell_var(&mut self.shell_state.env_vars, name, previous.flatten());
        self.shell_state.variables.remove(name);
        if let Some(variable) = typed_previous.flatten() {
            let _ = self.shell_state.variables.set(name.to_string(), variable);
        }
        set_var_attrs(&mut self.shell_state.env_vars, name, attrs);
        // Same sv_ignoreeof hook as the invisible-local branch above.
        if matches!(name, "IGNOREEOF" | "ignoreeof") {
            crate::builtins::set::sync_shell_option_flag(
                &mut self.shell_state.env_vars,
                "ignoreeof",
                false,
            );
        }
        true
    }

    /// `unset name[sub]` for a bracketed operand. Returns Some(status) when
    /// the operand is a bracketed lvalue so it doesn't fall through to
    /// scalar unbinding; the status propagates subscript-expansion errors
    /// (GNU arrayfunc.c:1290-1317 unbind_array_element -> arrayfunc.c:420
    /// expand_array_subscript sets return_code=1 on eval failure).
    /// `arrayref` models `VA_NOEXPAND` from `builtin_arrayref_flags`
    /// (builtins/common.c:1040): the operand word carried W_ARRAYREF, so
    /// `unbind_array_element` (arrayfunc.c:1165) binds the assoc subscript
    /// verbatim — `akey = sub` — instead of running the deferred
    /// `expand_subscript_string` pass.
    /// GNU `variables.c` `shell_compatibility_level` — the single derived
    /// level from `crate::builtins::shopt` (compatNN shopt flags win, else
    /// the strict `BASH_COMPAT` parse, else the 5.3 default). See
    /// `src/builtins/shopt/compat.rs`.
    fn shell_compatibility_level(&self) -> u32 {
        crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars)
    }

    pub(in crate::executor) fn unset_array_element(
        &mut self,
        name: &str,
        arrayref: bool,
        diag: &mut crate::executor::builtin_diag::Diag,
    ) -> Option<u8> {
        let Some((array_name, subscript)) = parse_array_subscript(name) else {
            return None;
        };
        if array_name == "BASH_ALIASES" {
            let key = subscript.trim_matches('\'').trim_matches('"');
            self.shell_state.aliases.remove(key);
            self.sync_dynamic_assoc_vars();
            return Some(0);
        }
        if array_name == "BASH_CMDS" {
            let key = subscript.trim_matches('\'').trim_matches('"');
            crate::builtins::hash::remove_hashed_path(&mut self.shell_state.env_vars, key);
            self.sync_dynamic_assoc_vars();
            return Some(0);
        }
        let Some(current) = self.shell_state.env_vars.get(array_name).cloned() else {
            return None;
        };

        // GNU unset.def:975-977: with shell_compatibility_level <= 51
        // (BASH_COMPAT=51 or lower) unset passes VA_ALLOWALL, and
        // unbind_array_element (arrayfunc.c:1153-1162) then unbinds the
        // WHOLE variable for `arr[@]'/`arr[*]' (behavior 1) instead of
        // flushing elements or treating @ as a literal assoc key.
        if (subscript == "@" || subscript == "*")
            && self.shell_compatibility_level() <= 51
            && (is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name)
                || is_marked_array_var(&self.shell_state.env_vars, array_name)
                || is_array_storage(&current))
        {
            self.shell_state.env_vars.remove(array_name);
            crate::executor::local_helpers::mirror_env_remove(array_name);
            self.shell_state.variables.remove(array_name);
            for key in [
                EXPORTED_VARS,
                READONLY_VARS,
                ARRAY_VARS,
                ASSOC_VARS,
                ASSOC_128_VARS,
                INTEGER_VARS,
                UPPERCASE_VARS,
                LOWERCASE_VARS,
                NAMEREF_VARS,
                DECLARED_UNSET_VARS,
            ] {
                unmark_env_name(&mut self.shell_state.env_vars, key, array_name);
            }
            return Some(0);
        }

        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
            // GNU arrayfunc.c:1241-1251 unbind_array_element assoc branch:
            // the operand's subscript already went through word expansion
            // once; with array_expand_once (ASS_NOEXPAND) that text is the
            // literal key, otherwise unbind performs the deferred
            // expand_subscript_string pass here.
            // GNU's deferred expand_subscript_string pass does NOT
            // re-recognize a quoted-source `$'...'` spelling as ANSI-C —
            // it quote-removes it (quotes drop, dollar stays literal,
            // content verbatim), so `unset 'A[$'"'"'x\ty'"'"']'` resolves
            // the key `$x\ty` and the `$'...'`-written cell SURVIVES
            // (rubash#847; the divergence #839 pinned as CR Nit 2).
            let deferred =
                crate::executor::subscript_expansion::quote_remove_ansi_c_spans(&subscript);
            let source = if arrayref {
                // VA_NOEXPAND: akey = sub verbatim (arrayfunc.c:1166) — the
                // `'`/`"` inside `dict["$k"]` already dequoted to data.
                SubscriptSource::Protected(&subscript)
            } else {
                SubscriptSource::ExpandedOnce(&deferred)
            };
            let key = self.resolve_array_subscript(source);
            if !arrayref && key.is_empty() && !subscript.is_empty() {
                // GNU arrayfunc.c:1160-1178 unbind_array_element: a
                // non-verbatim assoc subscript whose deferred expansion
                // produces an empty key fails array_expand_index, which
                // reports "[%s]: bad array subscript" via builtin_error
                // (e.g. `unset 'dict[$b]'` with b unset → `[$b]: bad
                // array subscript`) and returns -1 → status 1.
                // Was written straight to the process stderr: the
                // diagnostic bypassed the command's own `2>f` redirect
                // entirely (A3-R1, rubash#2707). It now travels through
                // the Diag, whose sink is the command's bound fd 2.
                diag.free(format_args!("[{subscript}]: bad array subscript"));
                diag.flush_with(|| self.diagnostic_prolog());
                return Some(1);
            }
            let mut entries = assoc_entries(&current);
            let before = entries.len();
            entries.retain(|(entry_key, _)| *entry_key != key);
            // Typed-cell sync (assoc read fast path): the retained entries
            // are the post-unset unique-key list, so sync the structured
            // cell from the same list instead of leaving it shadowing the
            // removed binding.
            sync_typed_assoc_array(&mut self.shell_state.variables, array_name, &entries);
            if entries.len() != before
                || !is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, array_name)
            {
                self.shell_state
                    .env_vars
                    .insert(array_name.to_string(), format_assoc_storage(entries));
            }
            return Some(0);
        }

        if is_marked_array_var(&self.shell_state.env_vars, array_name) || is_array_storage(&current)
        {
            // GNU unbind_array_element (arrayfunc.c:1180-1200): with the
            // default compat level (> 51), `unset arr[*]` / `unset arr[@]`
            // FLUSHES every element (behavior 2) instead of unsetting the
            // variable or treating * as an index; the variable itself stays
            // declared as an empty array (array.tests: `unset e[*]` then
            // `declare -a e=()`).
            if subscript == "*" || subscript == "@" {
                if array_name == "PIPESTATUS" {
                    // rubash#1364 R3: the @/* flush empties the sparse
                    // state list together with the mirror (see the
                    // element-unset sync below).
                    self.shell_state.pipestatus.clear();
                }
                self.shell_state.env_vars.insert(
                    array_name.to_string(),
                    format_indexed_array_storage(Default::default()),
                );
                // The mirror is flushed but the structured cell still holds
                // the old count — `${#a[@]}` reads the cell (rubash#651,
                // same dual-store class as #638). Sync it empty like the
                // element-unset path below does.
                sync_typed_indexed_array(
                    &mut self.shell_state.variables,
                    array_name,
                    &Default::default(),
                );
                return Some(0);
            }
            let index = match self.eval_indexed_subscript(if arrayref {
                SubscriptSource::Protected(&subscript)
            } else {
                SubscriptSource::ExpandedOnce(&subscript)
            }) {
                IndexedSubscript::Index(index) => index,
                // GNU: a literally empty `unset 'a[]'` is a silent
                // no-op. But an expanded-to-empty subscript evaluates
                // to 0 (arrayfunc.c:1141 unbind_array_element ->
                // array_expand_index -> expr.c:417 evalexp -> subexpr
                // returns 0 on empty text), so `unset 'a[$j]'` with an
                // empty $j unsets element 0. (#1009)
                IndexedSubscript::Empty if subscript.is_empty() => return Some(0),
                IndexedSubscript::Empty => 0,
                IndexedSubscript::Error => {
                    // A nounset trap aborts with the fatal status; other
                    // failures keep status 1 (rubash#639).
                    self.raise_nounset_subscript_abort();
                    return Some(1);
                }
            };
            // GNU arrayfunc.c:1207-1211: negative subscripts to indexed arrays
            // count back from end; if still negative, report "bad array
            // subscript" via builtin_error ("[%s]: %s", sub, ...).
            let Some(resolved) = resolve_indexed_array_subscript(&current, index) else {
                // Same A3-R1 fix as the assoc arm above: through
                // the Diag, never the process stderr.
                diag.free(format_args!("[{subscript}]: bad array subscript"));
                diag.flush_with(|| self.diagnostic_prolog());
                return Some(1);
            };
            let mut entries = indexed_array_entries(&current);
            entries.remove(&resolved);
            if array_name == "PIPESTATUS" {
                // rubash#1364 R3: the PIPESTATUS cell's authoritative
                // store is the sparse state list, not the mirror — an
                // element unset must remove the entry there too, or
                // the unset command's own completion (fast case:
                // count == nproc == 1) would rewrite the status into
                // the removed index instead of rebuilding densely.
                self.shell_state.pipestatus = entries
                    .iter()
                    .map(|(index, element)| {
                        (*index as i64, element.trim().parse::<i32>().unwrap_or(0))
                    })
                    .collect();
            }
            // rubash#616 (1a): element unset rewrites the mirror — the
            // structured cell must follow (count drops by one) or the
            // `${#a[@]}` fast path would shadow the removal.
            sync_typed_indexed_array(&mut self.shell_state.variables, array_name, &entries);
            self.shell_state.env_vars.insert(
                array_name.to_string(),
                format_indexed_array_storage(entries),
            );
            return Some(0);
        }

        // GNU arrayfunc.c:1218-1231 unbind_array_element scalar branch: for
        // a non-array variable the subscript is evaluated arithmetically and
        // subscript 0 IS the variable itself, so the whole variable is
        // unbound (array.tests: unset 'v[0]' on a scalar removes v). Any
        // other subscript returns -2, which unset.def reports as "not an
        // array variable" -- reached by returning None here, as does an
        // @/* subscript (arrayfunc.c:1163-1164).
        if subscript == "*" || subscript == "@" {
            return None;
        }
        match self.eval_indexed_subscript(if arrayref {
            SubscriptSource::Protected(&subscript)
        } else {
            SubscriptSource::ExpandedOnce(&subscript)
        }) {
            IndexedSubscript::Index(0) => {}
            IndexedSubscript::Index(_) => return None,
            // Same expanded-to-empty rule as the element arm above:
            // `unset 'v[$j]'` with an empty $j unsets the whole
            // variable (subscript 0 IS the variable,
            // arrayfunc.c:1218-1231); a literal `unset 'v[]'` is a
            // silent no-op. (#1009)
            IndexedSubscript::Empty if subscript.is_empty() => return Some(0),
            IndexedSubscript::Empty => {}
            IndexedSubscript::Error => {
                // Same nounset-abort routing (rubash#639).
                self.raise_nounset_subscript_abort();
                return Some(1);
            }
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, array_name) {
            return None;
        }
        self.shell_state.env_vars.remove(array_name);
        crate::executor::local_helpers::mirror_env_remove(array_name);
        self.shell_state.variables.remove(array_name);
        for key in [
            EXPORTED_VARS,
            READONLY_VARS,
            ARRAY_VARS,
            ASSOC_VARS,
            ASSOC_128_VARS,
            INTEGER_VARS,
            UPPERCASE_VARS,
            LOWERCASE_VARS,
            NAMEREF_VARS,
            DECLARED_UNSET_VARS,
        ] {
            unmark_env_name(&mut self.shell_state.env_vars, key, array_name);
        }
        Some(0)
    }
}
