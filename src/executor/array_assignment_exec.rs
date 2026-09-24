use super::*;

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
        let associative =
            is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) || self.is_assoc_parameter_array(name);
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
            self.shell_state.aliases
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
            crate::builtins::pushd::set_stack_value(&mut self.shell_state.env_vars, index, value.to_string());
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
            crate::builtins::hash::set_hashed_path(&mut self.shell_state.env_vars, command_name, value);
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
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) || self.is_assoc_parameter_array(name) {
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
            let current = self.shell_state.env_vars.get(name).cloned().unwrap_or_default();
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
            self.shell_state.env_vars.insert(name.to_string(), new_value);
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
        let computed_index = match self.eval_indexed_subscript(SubscriptSource::Protected(eval_input)) {
            IndexedSubscript::Index(index) => index,
            IndexedSubscript::Empty => {
                self.report_bad_array_subscript(&lhs_as_written);
                self.exit_code = 1;
                return true;
            }
            IndexedSubscript::Error => {
                self.exit_code = 1;
                return true;
            }
        };
        if computed_index < 0
            && resolve_indexed_array_subscript(
                &self.shell_state.env_vars.get(name).cloned().unwrap_or_default(),
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

        let current = self.shell_state.env_vars.get(name).cloned().unwrap_or_default();
        let index = if computed_index < 0 {
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
        let mut entries = indexed_array_entries(&current);
        let current_element = entries.get(&index).cloned().unwrap_or_default();
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
        entries.insert(index, element);
        self.shell_state.env_vars
            .insert(name.to_string(), format_indexed_array_storage(entries));
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, name);
        self.exit_code = 0;
        true
    }
}
