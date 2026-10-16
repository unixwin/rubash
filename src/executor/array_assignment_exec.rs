use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    /// GNU never expands `name[subscript]=value` as one word:
    /// `general.c:480 assignment()` splits the SYNTACTIC word into name,
    /// subscript and value, then `subst.c expand_subscript_string` expands
    /// the subscript and the W_ASSIGNMENT pass expands the value. Splitting
    /// the EXPANDED word instead loses the real `=` whenever the subscript
    /// expands to a `]` or an `=` (`x='a=b'; A[$x]=1` became `A[a` +
    /// `b]=2`).
    ///
    /// Returns the word the array-element executor should see when the
    /// expanded key would be unparseable: the key is the expanded subscript
    /// (marker-encoded when it carries a delimiter) and the value the
    /// assignment-RHS expansion of the raw value. `None` keeps the ordinary
    /// whole-word expansion, whose split is reliable for every other key.
    pub(in crate::executor) fn prepare_array_element_assignment_word(
        &mut self,
        assignment: &crate::parser::ArrayElementAssignment,
    ) -> Option<String> {
        let name = assignment.name.as_str();
        let raw_subscript = assignment.subscript_metadata.raw.as_str();
        let associative = is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
            || self.is_assoc_parameter_array(name);
        if !associative {
            return None;
        }
        // Expand only for the associative branch: expand_subscript_string
        // runs command substitutions, and an indexed subscript must see its
        // single expansion inside eval_indexed_subscript below — expanding
        // here too would execute `a[$(echo INJ)]=v` twice.
        let key = self.expand_subscript_string(raw_subscript);
        // expand_subscript_string is the subscript's ONE expansion pass
        // (GNU subst.c:11063 — W_NOTILDE is deliberately absent, so a leading
        // unquoted `~` inside the subscript tilde-expands). When its result
        // differs from the raw text — expanded `~`, quotes, `$x` — the whole
        // word expansion will NOT reproduce it (mid-word `~` never tilde-
        // expands), so the resolved key must ride the synthetic word.
        if key == raw_subscript && !key.contains(['[', ']', '=']) {
            return None;
        }
        // Dynamic arrays are consumed by name before the associative branch,
        // so their subscript must stay readable text.
        if matches!(name, "BASH_ALIASES" | "BASH_CMDS" | "DIRSTACK") {
            return None;
        }
        // GNU assign_array_element (arrayfunc.c:815-865): the value part of
        // `name[sub]=word` is expanded once as an assignment string; the
        // parser's cooked `value` already carries the lexer's quote/data
        // marking, while `raw_value` (source text) would feed live quote
        // syntax into an expansion pass that treats `'` as compound-element
        // data — `a[k]='n1'` stored the quotes verbatim.
        let synthetic = format!("{name}={}", assignment.value);
        let expanded =
            self.expand_word_mut_with_context(&synthetic, SubstitutionQuoteContext::Unquoted);
        let prefix = format!("{name}=");
        let value = expanded.strip_prefix(prefix.as_str()).unwrap_or(&expanded);
        Some(format!(
            "{name}[{}]{}{value}",
            super::arithmetic::encode_arithmetic_assoc_key(&key),
            assignment.operator
        ))
    }

    pub(in crate::executor) fn execute_array_element_assignment(
        &mut self,
        cmd: &CommandNode,
    ) -> bool {
        // TODO(variables.c/array.c/assoc.c): Bash array element assignment
        // carries typed SHELL_VAR attributes. This stores the element count
        // shape needed by upstream builtins5.sub.
        if cmd.words.len() != 1 {
            if !(0..cmd.words.len())
                .all(|index| command_word_is_array_element_assignment(cmd, index))
            {
                return false;
            }
            for word in &cmd.words {
                let mut single = cmd.clone();
                single.words = vec![word.clone()];
                single.array_element_assignments = cmd
                    .array_element_assignments
                    .iter()
                    .filter(|assignment| {
                        assignment
                            .word_index
                            .and_then(|word_index| cmd.words.get(word_index))
                            .is_some_and(|assignment_word| assignment_word == word)
                    })
                    .cloned()
                    .map(|mut assignment| {
                        assignment.word_index = Some(0);
                        assignment
                    })
                    .collect();
                if !self.execute_array_element_assignment(&single) {
                    return false;
                }
                if self.exit_code != 0 {
                    return true;
                }
            }
            self.exit_code = 0;
            return true;
        }
        if !command_word_is_array_element_assignment(cmd, 0) {
            return false;
        }
        let Some((left, value)) = cmd.words[0].split_once('=') else {
            return false;
        };
        let (left, append) = if let Some(left) = left.strip_suffix('+') {
            (left, true)
        } else {
            (left, false)
        };
        let Some((name, index)) = left.split_once('[') else {
            return false;
        };
        if !index.ends_with(']') || !is_shell_name(name) {
            return false;
        }
        let element_assignment = cmd.array_element_assignments.iter().find(|assignment| {
            assignment
                .word_index
                .and_then(|word_index| cmd.words.get(word_index))
                .is_some_and(|word| word == &cmd.words[0])
                && assignment.name == name
        });
        let raw_subscript =
            element_assignment.map(|assignment| assignment.subscript_metadata.raw.as_str());
        // GNU reports the LHS the way it was WRITTEN -- `h[]`, `A[""]`,
        // `A[$EMPTY]` -- not the whole assignment word and not the expanded
        // index (arrays.c `err_badarraysub`).
        let lhs_as_written = format!("{name}[{}]", raw_subscript.unwrap_or(index));
        let value_is_syntactic_compound_list = element_assignment.is_some_and(|assignment| {
            let raw_value = assignment.value.trim();
            raw_value.starts_with('(')
                && raw_value.ends_with(')')
                // Only real quote delimiters disqualify the compound list;
                // a `\x` escape inside it is still data (GNU parses `(x\*)`
                // as a list whose element contains a literal *).
                && !assignment
                    .word_quotes
                    .iter()
                    .any(|quote| quote.kind != crate::parser::QuoteKind::Backslash)
        });
        let name = match self.nameref_resolution(name) {
            NamerefResolution::Target(target) => {
                // GNU assign_array_element -> find_variable_nameref_for_create
                // (variables.c:2199-2203) requires a bare identifier:
                // `ref[k]=v` where ref -> `XXX[0]` fails sh_invalidid, while
                // a scalar `ref=v` forwards to the element. The caller raises
                // evalerror on nonzero exit_code, discarding the list.
                if parse_array_subscript(&target).is_some() {
                    eprintln!(
                        "{}`{target}': not a valid identifier",
                        self.diagnostic_prefix()
                    );
                    self.exit_code = 1;
                    return true;
                }
                target
            }
            NamerefResolution::Circular => {
                eprintln!(
                    "{}warning: {}: circular name reference",
                    self.diagnostic_prefix(),
                    name
                );
                self.exit_code = 1;
                return true;
            }
            NamerefResolution::MaxDepth => {
                eprintln!(
                    "{}warning: {}: maximum nameref depth (8) exceeded",
                    self.diagnostic_prefix(),
                    name
                );
                self.exit_code = 1;
                return true;
            }
            // GNU arrayfunc.c:270-275 bind_array_variable ->
            // variables.c:2182 find_variable_nameref_for_create: an element
            // assignment whose operand base resolves to an uncreated nameref
            // target fails sh_invalidid on the last nameref's cell text and
            // leaves the nameref untouched (`typeset -n ref; ref[0]=foo`
            // reports `': not a valid identifier`, ref stays `declare -n ref`
            // rather than becoming an array).
            NamerefResolution::Unresolved => {
                let cell = self.last_nameref_cell(name).unwrap_or_default();
                eprintln!(
                    "{}`{cell}': not a valid identifier",
                    self.diagnostic_prefix()
                );
                self.exit_code = 1;
                return true;
            }
            NamerefResolution::NotNameref => name.to_string(),
        };
        let name = name.as_str();
        if name == "BASH_ALIASES" {
            // TODO(variables.c/alias.c): BASH_ALIASES is a dynamic
            // associative array backed by the alias table. Keep this narrow
            // bridge here so array assignment does not swallow alias.tests'
            // invalid-name diagnostic.
            let alias_name = index
                .trim_end_matches(']')
                .trim_matches('\'')
                .trim_matches('"');
            if !valid_alias_assignment_name(alias_name) {
                eprintln!(
                    "{}`{alias_name}': invalid alias name",
                    self.diagnostic_prefix()
                );
                self.exit_code = 1;
                return true;
            }
            self.shell_state
                .aliases
                .insert(alias_name.to_string(), Alias::new(value));
            self.sync_dynamic_assoc_vars();
            self.exit_code = 0;
            return true;
        }
        if name == "DIRSTACK" {
            // TODO(builtins/pushd.def/variables.c): Bash exposes the
            // directory stack as a dynamic array variable. Keep assignments
            // wired to the pushd module's stack storage until SHELL_VAR array
            // attributes are ported.
            let Some(index) = index.trim_end_matches(']').parse::<usize>().ok() else {
                self.exit_code = 1;
                return true;
            };
            crate::builtins::pushd::set_stack_value(
                &mut self.shell_state.env_vars,
                index,
                value.to_string(),
            );
            self.exit_code = 0;
            return true;
        }
        if name == "GROUPS" {
            self.exit_code = 0;
            return true;
        }
        if is_noassign_bash_array(name) {
            self.exit_code = 0;
            return true;
        }
        if name == "BASH_CMDS" {
            let command_name = index
                .trim_end_matches(']')
                .trim_matches('\'')
                .trim_matches('"');
            crate::builtins::hash::set_hashed_path(
                &mut self.shell_state.env_vars,
                command_name,
                value,
            );
            self.sync_dynamic_assoc_vars();
            self.exit_code = 0;
            return true;
        }

        // Only the LAST `]` is the delimiter — `m[]]=v` keys on `]`, and
        // `m[a]]=v` keys on `a]`: trimming every trailing bracket would
        // erase a key that is itself made of `]` characters
        // (GNU assign_array_element uses the subscript text verbatim).
        let index = index.strip_suffix(']').unwrap_or(index);
        if value_is_syntactic_compound_list {
            // GNU subst.c:3603 reports the LHS name[subscript] only, not the
            // whole assignment word.
            eprintln!(
                "{}{name}[{index}]: cannot assign list to array member",
                self.diagnostic_prefix()
            );
            self.exit_code = 1;
            return true;
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
            eprintln!("{}{}: readonly variable", self.diagnostic_prefix(), name);
            self.exit_code = 1;
            return true;
        }
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
            || self.is_assoc_parameter_array(name)
        {
            // GNU arrayfunc.c:392-408 assign_array_element_internal: the
            // subscript gets exactly one expand_subscript_string pass and
            // the result is the literal key — a `$(...)` produced by that
            // expansion is data, never re-scanned (audit C11). Rubash's one
            // pass already ran inside expand_command_words, so the expanded
            // subscript text in `index` is consumed verbatim; the synthetic
            // word from prepare_array_element_assignment_word carries the
            // resolved key hex-encoded behind ARITH_ASSOC_KEY_MARKER.
            let key = super::arithmetic::decode_arithmetic_assoc_key(index)
                .unwrap_or_else(|| self.resolve_array_subscript(SubscriptSource::Protected(index)));
            if key.is_empty() {
                // An associative key is data, so nothing can fill in an empty
                // one: GNU rejects `A[]`, `A[""]` and `A[$unset]` with the
                // subscript as written (assoc.c assign_array_element).
                eprintln!(
                    "{}{lhs_as_written}: bad array subscript",
                    self.diagnostic_prefix()
                );
                self.exit_code = 1;
                return true;
            }
            // rubash#618: a plain element assignment never needs the array's
            // current contents — GNU assoc_insert just overwrites the binding
            // (assoc.c assoc_insert) — so it can append one `[k]=v` segment in
            // O(1) instead of parsing the whole storage string and re-rendering
            // every key. `+=` and a non-`(...)` storage still take the canonical
            // path below, which reads the current binding or rebuilds the cell.
            if !append {
                let stored = if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name) {
                    self.eval_integer_assignment_value(value).to_string()
                } else {
                    value.to_string()
                };
                let mut fast = false;
                if let Some(budget) = self.shell_state.array_compact_budget.get_mut(name) {
                    if *budget > 0 {
                        *budget -= 1;
                        fast = true;
                    }
                }
                if fast {
                    let segment = super::assignment_helpers::assoc_element_segment(&key, &stored);
                    let appended = match self.shell_state.env_vars.get_mut(name) {
                        Some(storage) => {
                            super::assignment_helpers::assoc_storage_append(storage, &segment)
                        }
                        None => false,
                    };
                    if appended {
                        // Same bookkeeping as the canonical path: an element
                        // assignment binds the variable (rubash#371).
                        unmark_env_name(&mut self.shell_state.env_vars, DECLARED_UNSET_VARS, name);
                        self.exit_code = 0;
                        return true;
                    }
                    // Not an appendable assoc cell yet — drop the budget so the
                    // canonical path below rebuilds it and re-seeds.
                    self.shell_state.array_compact_budget.remove(name);
                }
            }
            let current = self
                .shell_state
                .env_vars
                .get(name)
                .cloned()
                .unwrap_or_default();
            let mut entries = assoc_entries(&current);
            let value = if append {
                let current = entries
                    .iter()
                    .rev()
                    .find_map(|(entry_key, entry_value)| {
                        (entry_key == &key).then_some(entry_value.as_str())
                    })
                    .unwrap_or_default();
                if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name) {
                    // GNU bind_array_variable att_integer append adds the two
                    // expressions arithmetically (wheat[foo bar]+=7 with
                    // wheat[foo bar]=9 stores 16, not the concat-eval 97).
                    (self.eval_integer_assignment_value(current)
                        + self.eval_integer_assignment_value(value))
                    .to_string()
                } else {
                    append_scalar_value(current, value)
                }
            } else {
                value.to_string()
            };
            // GNU arrayfunc.c bind_array_variable (att_integer branch): when the
            // array carries the integer attribute, the value is evaluated as an
            // arithmetic expression before being stored (assoc.tests:
            // declare -Ai chaff; chaff[one]=3+7 stores 10, not 3+7).
            let value = if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name) {
                self.eval_integer_assignment_value(&value).to_string()
            } else {
                value
            };
            if let Some((_, entry_value)) = entries
                .iter_mut()
                .rev()
                .find(|(entry_key, _)| entry_key == &key)
            {
                *entry_value = value;
            } else {
                entries.push((key, value));
            }
            // format_assoc_storage folds duplicated segments back to one binding
            // per key, which makes this render the compaction step for the
            // append-updated form, and feeds the content-addressed parse cache
            // with the folded entries so a run of element assignments over this
            // array re-parses nothing (perf: OMB spectrum-style fills,
            // rubash#618).
            let entries = super::assignment_helpers::dedupe_assoc_entries(entries);
            // Re-seed the budget from the canonical entry count: that buys the
            // next O(entries) appends for free and keeps the storage string
            // within twice its canonical length, so reads stay O(entries).
            let compact_budget = entries.len().max(1) as u32;
            let new_value = super::assignment_helpers::format_assoc_storage(entries);
            self.shell_state
                .env_vars
                .insert(name.to_string(), new_value);
            self.shell_state
                .array_compact_budget
                .insert(name.to_string(), compact_budget);
            // GNU assign_array_element (arrayfunc.c:815) ->
            // bind_assoc_variable gives the variable a real assoc cell, so
            // var_isset is true and array_var_assignment (subst.c:8690-8697)
            // renders the `=(...)` body for `${assoc[*]@A}`. The rubash
            // declared-never-assigned state is the DECLARED_UNSET_VARS
            // marker; an element assignment must clear it or the @A
            // transform keeps printing the bare name (rubash#371).
            unmark_env_name(&mut self.shell_state.env_vars, DECLARED_UNSET_VARS, name);
            self.exit_code = 0;
            return true;
        }

        // GNU evaluates the subscript arithmetically even when it is all
        // whitespace (probe vs GNU 5.3: `h[ ]=10` stores h[0]=10 with
        // status 0, array25.sub), so only a truly EMPTY subscript is a bad
        // array subscript (`h[]=10`).
        if index.trim().is_empty() && raw_subscript.is_none_or(str::is_empty) {
            self.report_bad_array_subscript(&lhs_as_written);
            self.exit_code = 1;
            return true;
        }
        // GNU arrayfunc.c:420-434 assign_array_element_internal ->
        // array_expand_index (arrayfunc.c:1353-1391): the subscript's single
        // expand_arith_string pass already ran inside expand_command_words
        // (the word expansion IS that pass — re-expanding the syntactic raw
        // here executed `a[$(echo INJ)]=v` twice), and the result feeds
        // evalexp verbatim — a `$(...)` produced by the expansion is not
        // valid arithmetic and fails "operand expected" instead of
        // executing (audit C11).
        // The cooked `index` is that pass's output — except it lost the
        // raw spelling's single quotes at parse time, and GNU's arith pass
        // keeps sq literal for evalexp (`a[' ']` errors, `a[" "]` -> 0).
        // When the raw spelling de-quotes to `index` (no substitution ran),
        // rebuild the arith-context text from raw so sq/dq/backslash get
        // their GNU treatment; when they differ `index` IS the expansion
        // product and feeds evalexp verbatim.
        let eval_text;
        let mut eval_input = match raw_subscript {
            // A single-quoted span survives GNU's arith-context expansion as
            // literal text but does NOT protect the `$x` inside it —
            // `a['$v']` expands to `'2'` and evalexp reports operand
            // expected. The cooked `index` dropped the sq wrapper at parse
            // time, so rebuild via the same expand_arithmetic_special_
            // parameters pass the `${a[...]}` read path uses (sq -> \x17
            // data, `$x`/`$((...))`/`$(...)` expand, dq removed).
            Some(raw) if raw.contains('\'') => {
                eval_text = self.expand_arithmetic_special_parameters(raw);
                eval_text.as_str()
            }
            // Quoting other than sq (dq, backslash) normalizes identically
            // in the cooked index only when no substitution ran; rebuild the
            // dq/escape-stripped form from raw so `a[" "]` resolves to 0.
            Some(raw) if crate::lexer::remove_shell_quotes(raw) == index => {
                eval_text = super::arithmetic::arith_subscript_text(raw);
                eval_text.as_str()
            }
            // GNU never word-expands the LHS subscript of `name[sub]=v`:
            // array_expand_index (arrayfunc.c:1356) receives the RAW text
            // and expand_arith_string is its ONE expansion pass — a `$(...)`
            // produced by that pass is data to evalexp, never re-scanned
            // (a[$key] with key=`x],b[$(echo uname >&2)` errors with the
            // literal token, verified GNU 5.3). Feeding the already-expanded
            // `index` here ran the produced `$(...)` a second time.
            Some(raw) => {
                eval_text = super::arithmetic::arith_subscript_text(raw);
                eval_text.as_str()
            }
            _ => index,
        };
        // GNU evaluates the subscript arithmetically even when it is all
        // whitespace — `h[ ]=10` stores h[0]=10 (array25.sub): whitespace
        // only coerces to 0 AFTER the sq check above kept `' '` literal.
        if eval_input.trim().is_empty() {
            eval_input = "0";
        }
        // GNU arrayfunc.c:355-360 assign_array_element: ALL_ELEMENT_SUB
        // rejects both `@` and `*` subscripts on an indexed element
        // assignment before the arith eval (err_badarraysub prints w —
        // the whole `name[sub]` word).
        if matches!(eval_input.trim(), "*" | "@") {
            self.report_bad_array_subscript(&lhs_as_written);
            self.exit_code = 1;
            return true;
        }
        let computed_index =
            match self.eval_indexed_subscript(SubscriptSource::Protected(eval_input)) {
                IndexedSubscript::Index(index) => index,
                IndexedSubscript::Empty => {
                    self.report_bad_array_subscript(&lhs_as_written);
                    self.exit_code = 1;
                    return true;
                }
                IndexedSubscript::Error => {
                    // GNU expr.c:1190-1216: a nounset trap in the subscript
                    // is FORCE_EOF, not a plain bad-subscript error
                    // (rubash#639) — abandon with the context-owned fatal
                    // status (127 at the `-c` top level, 1 in script mode),
                    // the same rails the read path's nounset handling uses.
                    // Any other evaluation failure keeps the existing
                    // operand-expected behavior.
                    if self.raise_nounset_subscript_abort() {
                        return true;
                    }
                    self.exit_code = 1;
                    return true;
                }
            };
        if computed_index < 0
            && resolve_indexed_array_subscript(
                &self
                    .shell_state
                    .env_vars
                    .get(name)
                    .cloned()
                    .unwrap_or_default(),
                computed_index,
            )
            .is_none()
        {
            eprintln!(
                "{}{}: bad array subscript",
                self.diagnostic_prefix(),
                lhs_as_written
            );
            self.exit_code = 1;
            return true;
        }

        let index = if computed_index < 0 {
            let current = self
                .shell_state
                .env_vars
                .get(name)
                .cloned()
                .unwrap_or_default();
            let Some(index) = resolve_indexed_array_subscript(&current, computed_index) else {
                eprintln!(
                    "{}{}: bad array subscript",
                    self.diagnostic_prefix(),
                    lhs_as_written
                );
                self.exit_code = 1;
                return true;
            };
            index
        } else {
            let Ok(index) = usize::try_from(computed_index) else {
                return false;
            };
            index
        };
        let typed_index = i64::try_from(index).unwrap_or(i64::MAX);
        // rubash#616 (plan Phase 1b): GNU assigns an array element on the
        // ARRAY object itself — arrayfunc.c:815 assign_array_element reaches
        // array.c:516 array_insert, which appends ONE node — and never
        // re-serialises the array. When the structured cell is a complete
        // `IndexedArray`, read the old element from that cell rather than
        // re-parsing the mirror render (O(n) per write, O(n^2) to build an
        // n-element array).
        let structured = self.shell_state.variables.indexed_array_shape(name);
        let current_element = if structured.is_some() {
            self.shell_state
                .variables
                .get(name)
                .and_then(|variable| match &variable.value {
                    crate::shell::ShellValue::IndexedArray(values) => {
                        values.get(&typed_index).cloned()
                    }
                    _ => None,
                })
                .unwrap_or_default()
        } else {
            let current = self
                .shell_state
                .env_vars
                .get(name)
                .cloned()
                .unwrap_or_default();
            indexed_array_entries(&current)
                .get(&index)
                .cloned()
                .unwrap_or_default()
        };
        let element = if append {
            if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name) {
                (eval_arith_value(&current_element) + eval_arith_value(value)).to_string()
            } else {
                append_scalar_value(&current_element, value)
            }
        } else {
            value.to_string()
        };
        let element = if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name) {
            eval_arith_value(&element).to_string()
        } else {
            element
        };
        // rubash#616 (plan Phase 1b): an append (index == max_index + 1) onto a
        // canonical mirror is applied in place — one structured insert plus the
        // exact segment the renderer would have appended — instead of a full
        // re-parse and re-render. Every other shape (overwrite, sparse insert,
        // readonly, no structured cell, non-canonical mirror) falls through to
        // the unchanged full path below, so this is a whitelist whose fallback
        // is the real path.
        let mut appended = match structured {
            Some((len, max))
                if self
                    .shell_state
                    .variables
                    .get(name)
                    .is_none_or(|variable| !variable.readonly)
                    && match max {
                        Some(max) => typed_index == max + 1,
                        None => typed_index == 0,
                    }
                    && self.shell_state.env_vars.get(name).is_some_and(|mirror| {
                        mirror.starts_with(STORAGE_WORD_PREFIX) && mirror.ends_with(')')
                    }) =>
            {
                // Mirror first, cell second: the caller's fallback re-parses
                // the (already extended) mirror and re-inserts the same
                // element, so a failed cell write still lands on the correct
                // value instead of a stale mirror.
                append_indexed_mirror_segments(
                    &mut self.shell_state.env_vars,
                    name,
                    std::iter::once((index, element.as_str())),
                ) && self.shell_state.variables.extend_indexed_element(
                    name,
                    typed_index,
                    &element,
                    len + 1,
                )
            }
            _ => false,
        };
        // rubash#616 (overwrite/sparse-jump arm): 1b above handles only
        // index == max_index + 1. Any other shaped write with a complete
        // cell and a canonical mirror appends one duplicate segment in
        // O(segment) instead of re-parsing and re-rendering every element:
        // GNU array.c:516 array_insert overwrites in place and never
        // re-serialises. Every indexed reader folds duplicates through its
        // BTreeMap (last wins for value, distinct for keys/count, sorted
        // for order — verified), and the budget-seeded canonical render
        // below compacts. Falls through to the full path on any shape
        // mismatch, so this is a whitelist with the real path as fallback.
        if !appended {
            if let Some((len, _)) = structured {
                let mut budgeted = false;
                if let Some(budget) = self.shell_state.array_compact_budget.get_mut(name) {
                    if *budget > 0 {
                        *budget -= 1;
                        budgeted = true;
                    }
                }
                if budgeted
                    && self
                        .shell_state
                        .variables
                        .get(name)
                        .is_none_or(|variable| !variable.readonly)
                    && self.shell_state.env_vars.get(name).is_some_and(|mirror| {
                        mirror.starts_with(STORAGE_WORD_PREFIX) && mirror.ends_with(')')
                    })
                {
                    // An overwrite keeps the length; a new index grows it by
                    // one. The cell answers which (O(log n)); a wrong answer
                    // only costs a fallback, because extend verifies the size.
                    let expected_len = if self
                        .shell_state
                        .variables
                        .indexed_element(name, typed_index)
                        .is_some()
                    {
                        len
                    } else {
                        len.saturating_add(1)
                    };
                    let mirror_ok = append_indexed_mirror_segments(
                        &mut self.shell_state.env_vars,
                        name,
                        std::iter::once((index, element.as_str())),
                    );
                    // Mirror first, cell second (same rationale as 1b): the
                    // canonical fallback re-parses the extended mirror and
                    // re-syncs the cell from it, so a failed cell write still
                    // lands on the correct value instead of a stale mirror.
                    if mirror_ok
                        && self.shell_state.variables.extend_indexed_element(
                            name,
                            typed_index,
                            &element,
                            expected_len,
                        )
                    {
                        appended = true;
                    } else {
                        // Not an appendable cell (or cell/mirror disagree) —
                        // drop the budget so the canonical path below rebuilds
                        // both and re-seeds.
                        self.shell_state.array_compact_budget.remove(name);
                    }
                }
            }
        }
        if !appended {
            let current = self
                .shell_state
                .env_vars
                .get(name)
                .cloned()
                .unwrap_or_default();
            let mut entries = indexed_array_entries(&current);
            entries.insert(index, element);
            // rubash#616 (plan Phase 1a): the structured cell follows the
            // mirror in lockstep so `${#a[@]}` reads the count in O(1) (GNU
            // array.h:121 num_elements) instead of reparsing the render. The
            // in-place extension keeps the element-write loop off the O(n)
            // map rebuild.
            sync_typed_indexed_element(
                &mut self.shell_state.variables,
                name,
                typed_index,
                entries.get(&index).map(String::as_str).unwrap_or_default(),
                &entries,
            );
            // Re-seed the budget from the distinct entry count (BTreeMap is
            // already folded): that buys the next O(entries) appends for free
            // and keeps the storage string within twice its canonical length,
            // so reads stay O(entries). rubash#616 overwrite arm.
            let compact_budget = entries.len().max(1) as u32;
            self.shell_state
                .env_vars
                .insert(name.to_string(), format_indexed_array_storage(entries));
            self.shell_state
                .array_compact_budget
                .insert(name.to_string(), compact_budget);
        }
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, name);
        // Same GNU owner as the assoc branch above: bind_array_variable
        // (arrayfunc.c:815 assign_array_element) allocates a real array
        // cell, so a declared-but-unset array becomes set the moment an
        // element is assigned. Clear the rubash declared-unset marker so
        // `${idx[*]@A}` renders the `=(...)` body (rubash#371).
        unmark_env_name(&mut self.shell_state.env_vars, DECLARED_UNSET_VARS, name);
        self.exit_code = 0;
        true
    }
}
