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

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if is_null_device(&target) {
                return self.execute_unset_with_stderr(
                    &cmd.words[1..],
                    &arrayref_flags,
                    &mut std::io::sink(),
                );
            }
            let mut file =
                File::create(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            return self.execute_unset_with_stderr(&cmd.words[1..], &arrayref_flags, &mut file);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_redirect_target(redirect);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            return self.execute_unset_with_stderr(&cmd.words[1..], &arrayref_flags, &mut file);
        }

        self.execute_unset_with_stderr(
            &cmd.words[1..],
            &arrayref_flags,
            &mut std::io::stderr().lock(),
        )
    }

    pub(in crate::executor) fn execute_unset_with_stderr<W>(
        &mut self,
        args: &[String],
        arrayref_flags: &[bool],
        stderr: &mut W,
    ) -> Result<i32, ExecuteError>
    where
        W: Write,
    {
        // TODO(builtins/set.def/variables.c/execute_cmd.c): `unset` searches
        // variables and functions with nuanced attributes. Keep function table
        // and variable table behavior aligned for builtins6.sub.
        if unset_args_need_builtin_diagnostics(args) {
            let status = crate::builtins::set::unset_with_stderr(
                args.iter().map(String::as_str),
                &mut self.shell_state.env_vars,
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

        let function_only = args.iter().any(|arg| arg == "-f");
        let variable_only = args.iter().any(|arg| arg == "-v");
        // GNU builtins/set.def:866-867: `unset -f` cancels -n, and -n is
        // only meaningful for variables anyway.
        let nameref_only = args.iter().any(|arg| arg == "-n") && !function_only;
        // Operand words may carry the in-band W_ARRAYREF flag
        // (ARRAYREF_FLAG prefix — arrayref.rs); the syntactic flag was
        // already captured in `arrayref_flags` above, so strip the prefix
        // here before any name/subscript consumer sees it.
        let names: Vec<(usize, String)> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| !arg.starts_with('-'))
            .map(|(index, arg)| {
                (
                    index,
                    crate::builtins::arrayref::take_arrayref_flag(arg)
                        .1
                        .to_string(),
                )
            })
            .collect();

        let mut function_status = 0;
        if !variable_only {
            for (_, name) in &names {
                if marked_env_names(&self.shell_state.env_vars, READONLY_FUNCTIONS)
                    .iter()
                    .any(|readonly| readonly == name)
                {
                    writeln!(
                        stderr,
                        "{}unset: {name}: cannot unset: readonly function",
                        self.diagnostic_prefix()
                    )?;
                    function_status = 1;
                    continue;
                }
                self.shell_state.functions.remove(name);
                self.shell_state.function_definition_redirects.remove(name);
                self.shell_state.function_def_infos.remove(name);
                unmark_env_name(&mut self.shell_state.env_vars, EXPORTED_FUNCTIONS, name);
            }
        }

        if function_only {
            return Ok(function_status);
        }

        let mut variable_args: Vec<String> = args
            .iter()
            .filter(|arg| arg.starts_with('-') && arg.as_str() != "-f")
            .cloned()
            .collect();
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
                        writeln!(
                            stderr,
                            "{}unset: {name}: cannot unset",
                            self.diagnostic_prefix()
                        )?;
                        nameref_status = 1;
                        continue;
                    }
                    if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &checked) {
                        writeln!(
                            stderr,
                            "{}unset: {checked}: cannot unset: readonly variable",
                            self.diagnostic_prefix()
                        )?;
                        nameref_status = 1;
                        continue;
                    }
                }
                if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, &name)
                    && !self.unset_outer_local_variable(&name)
                {
                    self.shell_state.env_vars.remove(&name);
                    std::env::remove_var(&name);
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
                    if let Some(status) = self.unset_through_nameref(&name, stderr) {
                        element_status = element_status.max(i32::from(status));
                        continue;
                    }
                    if let Some(status) = self.unset_array_element(&name, true) {
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
                        writeln!(
                            stderr,
                            "{}unset: `{name}': not a valid identifier",
                            self.diagnostic_prefix()
                        )?;
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
            if let Some(status) = self.unset_through_nameref(&name, stderr) {
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
            if let Some(status) = self.unset_array_element(&name, verbatim) {
                element_status = element_status.max(i32::from(status));
                continue;
            }
            if self.unset_outer_local_variable(&name) {
                continue;
            }
            variable_args.push(name);
        }

        let variable_status = crate::builtins::set::unset_with_stderr(
            variable_args.iter().map(String::as_str),
            &mut self.shell_state.env_vars,
            stderr,
        )
        .map_err(ExecuteError::from)?;
        // GNU builtins/set.def:995-1000: unsetting through a nameref removes
        // the referenced variable and keeps the nameref; drop the referenced
        // variable from the typed owner too so parameter expansion does not
        // see a stale value (unset foo with foo->bar must clear bar).
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
                if let Some(cell) = self
                    .shell_state
                    .env_vars
                    .get(name)
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
        // GNU variables.c:6205-6217 sv_ignoreeof: unbinding the IGNOREEOF
        // variable turns the ignoreeof option off (the option tracks
        // "the variable is set").
        for name in variable_args.iter().filter(|a| !a.starts_with('-')) {
            if matches!(name.as_str(), "IGNOREEOF" | "ignoreeof") {
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
    pub(in crate::executor) fn unset_through_nameref<W: Write>(
        &mut self,
        name: &str,
        stderr: &mut W,
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
                return self.unset_array_element(&cell, false).or(Some(0));
            }
            // GNU set.def:936-937 + 962-966: the resolved referent is
            // unbound by name; a readonly referent is an error on the
            // referent's name, and a missing referent is a silent no-op.
            if !cell.is_empty() && is_shell_name(&cell) {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &cell) {
                    let _ = writeln!(
                        stderr,
                        "{}unset: {cell}: cannot unset: readonly variable",
                        self.diagnostic_prefix()
                    );
                    return Some(1);
                }
                self.shell_state.env_vars.remove(&cell);
                std::env::remove_var(&cell);
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
        self.unset_array_element(&format!("{cell}[{subscript}]"), false)
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
            env::remove_var(name);
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
    /// GNU variables.c shell_compatibility_level — see
    /// shell_compatibility_level_value for the BASH_COMPAT parse.
    fn shell_compatibility_level(&self) -> u32 {
        shell_compatibility_level_value(&self.shell_state.env_vars)
    }

    pub(in crate::executor) fn unset_array_element(
        &mut self,
        name: &str,
        arrayref: bool,
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
            std::env::remove_var(array_name);
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
            let source = if arrayref {
                // VA_NOEXPAND: akey = sub verbatim (arrayfunc.c:1166) — the
                // `'`/`"` inside `dict["$k"]` already dequoted to data.
                SubscriptSource::Protected(&subscript)
            } else {
                SubscriptSource::ExpandedOnce(&subscript)
            };
            let key = self.resolve_array_subscript(source);
            if !arrayref && key.is_empty() && !subscript.is_empty() {
                // GNU arrayfunc.c:1160-1178 unbind_array_element: a
                // non-verbatim assoc subscript whose deferred expansion
                // produces an empty key fails array_expand_index, which
                // reports "[%s]: bad array subscript" via builtin_error
                // (e.g. `unset 'dict[$b]'` with b unset → `[$b]: bad
                // array subscript`) and returns -1 → status 1.
                let mut buf = Vec::new();
                let _ = writeln!(
                    &mut buf,
                    "{}unset: [{subscript}]: bad array subscript",
                    self.diagnostic_prefix()
                );
                let _ = std::io::Write::write_all(&mut std::io::stderr().lock(), &buf);
                return Some(1);
            }
            let mut entries = assoc_entries(&current);
            entries.retain(|(entry_key, _)| *entry_key != key);
            self.shell_state
                .env_vars
                .insert(array_name.to_string(), format_assoc_storage(entries));
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
                self.shell_state.env_vars.insert(
                    array_name.to_string(),
                    format_indexed_array_storage(Default::default()),
                );
                return Some(0);
            }
            let index = match self.eval_indexed_subscript(if arrayref {
                SubscriptSource::Protected(&subscript)
            } else {
                SubscriptSource::ExpandedOnce(&subscript)
            }) {
                IndexedSubscript::Index(index) => index,
                // GNU: `unset 'a[]'` is a silent no-op.
                IndexedSubscript::Empty => return Some(0),
                IndexedSubscript::Error => return Some(1),
            };
            // GNU arrayfunc.c:1207-1211: negative subscripts to indexed arrays
            // count back from end; if still negative, report "bad array
            // subscript" via builtin_error ("[%s]: %s", sub, ...).
            let Some(resolved) = resolve_indexed_array_subscript(&current, index) else {
                // Report the bad subscript error. The caller (execute_unset)
                // passes stderr via the executor, so emit directly.
                let mut stderr = Vec::new();
                let _ = writeln!(
                    &mut stderr,
                    "{}unset: [{subscript}]: bad array subscript",
                    self.diagnostic_prefix()
                );
                let _ = std::io::Write::write_all(&mut std::io::stderr().lock(), &stderr);
                return Some(1);
            };
            let mut entries = indexed_array_entries(&current);
            entries.remove(&resolved);
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
            IndexedSubscript::Empty => return Some(0),
            IndexedSubscript::Error => return Some(1),
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, array_name) {
            return None;
        }
        self.shell_state.env_vars.remove(array_name);
        std::env::remove_var(array_name);
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
