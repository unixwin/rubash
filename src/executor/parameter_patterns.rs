use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

/// subst.c::dequote_string (4807-4841): strip quote-protection marks and
/// return the plain value. The pattern-matching code cannot interpret
/// CTLESC, so `get_var_and_type` hands pat_subst a dequoted value
/// (subst.c:8598 `*valp = value ? dequote_string (value) : NULL`), and the
/// C source says so directly at parameter_brace_patsub:9468-9473:
/// "The pattern matching code doesn't understand CTLESC quoting CTLESC and
/// CTLNUL so we use the dequoted variable values passed in (VT_VARIABLE)".
///
/// Rubash carries the same information in the walker's C0 marks, so the
/// equivalent is restoring them to the characters they stand for. Without
/// this, `t="a'b"` keeps U+0017 in the value handed to the matcher, the
/// pattern `'` (a real U+0027) never matches, and `${t//"'"</* replacement */>}`
/// silently does nothing -- the quote1.sub failure where `'weferfds'\''dsfsdf'`
/// came out as `'weferfds'dsfsdf'`.
fn dequote_storage_marks(value: &str) -> String {
    value
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(crate::executor::markers::DATA_SQUOTE, "'")
        .replace(crate::executor::markers::DATA_DQUOTE, "\"")
        .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
        .replace(crate::lexer::ANSI_C_QUOTE_MARKER_STR, "'")
        .replace(crate::lexer::ANSI_C_DQUOTE_MARKER_STR, "\"")
}

impl Executor {
    pub(in crate::executor) fn indirect_parameter_transform(
        &self,
        name: &str,
        transform: ParameterTransform,
    ) -> Option<String> {
        let indirect_name = name.strip_prefix('!')?;
        let ref_name = indirect_name
            .strip_suffix("[@]")
            .or_else(|| indirect_name.strip_suffix("[*]"));
        if ref_name.is_none() {
            let target_name = self.shell_state.env_vars.get(indirect_name)?;
            if transform == ParameterTransform::Assignment {
                return Some(self.parameter_assignment_transform(target_name));
            }
            if transform == ParameterTransform::Attributes {
                return Some(self.parameter_attribute_transform(target_name));
            }
            if transform == ParameterTransform::KeyValueQuoted {
                return Some(self.parameter_key_value_transform(target_name, true));
            }
            if transform == ParameterTransform::KeyValueSplit {
                return Some(self.parameter_key_value_transform(target_name, false));
            }
            let value = self
                .array_element_parameter_value(target_name)
                .or_else(|| {
                    self.shell_state
                        .env_vars
                        .get(target_name)
                        .and_then(|value| {
                            if is_array_storage(value)
                                || is_marked_array_var(&self.shell_state.env_vars, target_name)
                            {
                                array_value_at(value, 0)
                            } else {
                                Some(value.clone())
                            }
                        })
                })
                .unwrap_or_default();
            return Some(self.apply_parameter_transform_value(&value, transform));
        }
        let ref_name = ref_name?;
        let target_name = self.shell_state.env_vars.get(ref_name)?;
        let value = if let Some(array_expr) = target_name
            .strip_suffix("[@]")
            .or_else(|| target_name.strip_suffix("[*]"))
        {
            self.shell_state
                .env_vars
                .get(array_expr)
                .and_then(|value| array_value_at(value, 0))
                .unwrap_or_default()
        } else {
            self.shell_state
                .env_vars
                .get(target_name)
                .and_then(|value| {
                    if is_array_storage(value)
                        || is_marked_array_var(&self.shell_state.env_vars, target_name)
                    {
                        array_value_at(value, 0)
                    } else {
                        Some(value.clone())
                    }
                })
                .unwrap_or_default()
        };
        Some(self.apply_parameter_transform_value(&value, transform))
    }

    pub(in crate::executor) fn expand_parameter_pattern_removal(
        &self,
        var_name: &str,
        pattern: &str,
        operation: PatternRemoval,
    ) -> Option<String> {
        let pattern = self.expand_parameter_pattern_word(pattern);
        if matches!(var_name, "@" | "*") {
            let result = self
                .shell_state
                .positional_params
                .iter()
                .map(|value| {
                    remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled())
                })
                .collect::<Vec<_>>()
                .join(" ");
            return Some(result);
        }

        if is_special_parameter_name(var_name) {
            return Some(remove_parameter_pattern(
                &self.expand_parameter_named_value(var_name),
                &pattern,
                operation,
                self.extglob_enabled(),
            ));
        }

        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.shell_state
                    .positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| {
                        remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled())
                    })
                    .unwrap_or_default(),
            );
        }

        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(remove_parameter_pattern(
                &value,
                &pattern,
                operation,
                self.extglob_enabled(),
            ));
        }

        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        let values = array_values(&value)
                            .into_iter()
                            .map(|value| {
                                remove_parameter_pattern(
                                    &value,
                                    &pattern,
                                    operation,
                                    self.extglob_enabled(),
                                )
                            })
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }

        if is_shell_name(var_name) {
            let value = self
                .parameter_pattern_scalar_value(var_name)
                .unwrap_or_default();
            return Some(remove_parameter_pattern(
                &value,
                &pattern,
                operation,
                self.extglob_enabled(),
            ));
        }

        None
    }

    /// nocasematch shopt state for pattern substitution (GNU subst.c applies
    /// FNMATCH_IGNCASE in match_upattern when nocasematch is set).
    pub(in crate::executor) fn nocasematch_enabled(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "nocasematch")
    }

    /// extglob shopt state for pattern removal (GNU subst.c match_upattern
    /// passes FNM_EXTMATCH when the extglob option is on).
    pub(in crate::executor) fn extglob_enabled(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extglob")
    }

    pub(in crate::executor) fn parameter_pattern_scalar_value(&self, name: &str) -> Option<String> {
        if is_special_parameter_name(name) {
            return Some(dequote_storage_marks(
                &self.expand_parameter_named_value(name),
            ));
        }

        if let Some(value) = self.dynamic_parameter_value(name) {
            return Some(dequote_storage_marks(&value));
        }

        let resolved = self.resolved_variable_name(name)?;
        // GNU subst.c: a nameref cell that is an array reference names the
        // element, not a variable literally called "arr[i]" — resolve it
        // through the array element path (nameref9.sub ${f/x/X} with
        // f -> arr[1]).
        if parse_array_subscript(&resolved).is_some() {
            return self
                .array_element_parameter_value(&resolved)
                .map(|value| dequote_storage_marks(&value));
        }
        let value = self.shell_state.env_vars.get(&resolved)?;
        // GNU subst.c resolves a bare array name to one element, not the whole
        // array (get_var_and_type -> VT_ARRAYVAR): associative arrays read key
        // "0" (assoc_cell), indexed arrays read element [0] (array_cell). Expanding
        // the raw storage marker here leaks ([FOO]=BAR) where GNU prints the
        // element value or empty when key "0" is absent.
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) {
            return Some(dequote_storage_marks(
                &assoc_value_at(value, "0").unwrap_or_default(),
            ));
        }

        if is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, &resolved) {
            return Some(dequote_storage_marks(
                &array_value_at(value, 0)
                    .or_else(|| assoc_value_at(value, "0"))
                    .unwrap_or_default(),
            ));
        }

        Some(dequote_storage_marks(value))
    }

    pub(in crate::executor) fn expand_parameter_pattern_word(&self, pattern: &str) -> String {
        // GNU parse.y parse_matched_pair: an unclosed quote inside the
        // ${...} word is an EOF syntax error, so expansion aborts the
        // command (`${c%' z'}` is fine; `${c%\' z'}` leaves an unclosed '
        // after the escaped quote and the whole command fails).
        if let Some(quote) = unclosed_pattern_quote(pattern) {
            eprintln!(
                "{}unexpected EOF while looking for matching `{}'",
                self.diagnostic_prefix(),
                quote
            );
            self.shell_state.arithmetic_fatal_error.set(true);
            self.shell_state.arithmetic_expansion_error.set(true);
            return String::new();
        }
        // Decode quotes before embedded expansion so quoted glob
        // metacharacters stay marked, but mask nested braced parameters
        // first so the decoder does not tag glob chars inside an inner
        // expansion (keeps ? a glob in the inner removal).
        let mut masked = String::with_capacity(pattern.len());
        let mut slots: Vec<String> = Vec::new();
        // GNU subst.c: a double-quoted span in a pattern word expands its
        // substitutions with quoting live, so the OUTPUT chars carry CTLESC
        // (`"${x//"$p"/!}"` with p='*' matches a literal `*`, and `p='\'`
        // matches a literal `\` — an unquoted `$p` instead feeds a live
        // escape to the matcher). The shared embedded expander strips the
        // marker context, so expand the span here and emit every output
        // char literal-marked through the slot mechanism below.
        let pre_masked = mask_quoted_pattern_spans(pattern, self, &mut slots);
        let mut rest: &str = &pre_masked;
        while let Some(pos) = rest.find("${") {
            masked.push_str(&rest[..pos]);
            let after = &rest[pos + 2..];
            match matching_parameter_brace(after) {
                Some(end) => {
                    slots.push(format!("${{{}}}", &after[..end]));
                    masked.push(crate::executor::markers::IFS_GLUE);
                    masked.push_str(&(slots.len() - 1).to_string());
                    rest = &after[end + 1..];
                }
                None => {
                    masked.push_str("${");
                    rest = after;
                }
            }
        }
        masked.push_str(rest);

        // GNU subst.c: an escaped anchor char `\%` or `\#` in a pattern
        // substitution pattern is a literal character, not a suffix/prefix
        // anchor. `decode_parameter_pattern_quotes` strips the backslash and
        // leaves a bare `%`/`#` which `replace_parameter_pattern` would
        // misinterpret as an anchor (subst.c parameter_brace_patsub:9451-
        // 9465). Mark the escaped anchor with \x11 so the backslash survives
        // as a glob escape (`\%`) after the final `.replace(crate::executor::markers::CTLESC, "\\")`,
        // routing through the glob matcher for a literal match instead of
        // the anchor fast path. Only mark backslashes that are not
        // themselves escaped (`\\%` keeps `\\` for the decoder).
        let masked = mark_escaped_pattern_anchors(&masked);

        let decoded = decode_parameter_pattern_quotes(&masked)
            .replace(crate::executor::markers::QUOTED_WORD_PREFIX, "");

        let mut restored = String::with_capacity(decoded.len());
        let mut rest = decoded.as_str();
        while let Some(pos) = rest.find(crate::executor::markers::IFS_GLUE) {
            restored.push_str(&rest[..pos]);
            let digits: String = rest[pos + 1..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                restored.push(crate::executor::markers::IFS_GLUE);
                rest = &rest[pos + 1..];
                continue;
            }
            let index: usize = digits.parse().unwrap_or(0);
            if let Some(slot) = slots.get(index) {
                restored.push_str(slot);
            }
            rest = &rest[pos + 1 + digits.len()..];
        }
        restored.push_str(rest);

        // \x18 is the literal-backslash marker from decode_parameter_pattern_quotes,
        // but expand_embedded_parameters_preserving_escaped_single_quotes treats \x18
        // as a double-quote marker and converts it to `"`.  Protect it by mapping
        // to \x14 (which the expander preserves as a literal backslash) and restore
        // after expansion so the pattern matcher sees the correct marker.
        let protected = restored.replace(
            crate::executor::markers::PATTERN_LITERAL_BACKSLASH,
            crate::executor::markers::DATA_BACKSLASH_STR,
        );
        let expanded = self.expand_embedded_parameters_preserving_escaped_single_quotes(&protected);
        // Quoted glob metacharacters remain pattern literals. Preserve the
        // escape for the parameter matcher instead of exposing a raw marker.
        expanded
            .replace(crate::executor::markers::CTLESC, "\\")
            .replace(
                crate::executor::markers::DATA_BACKSLASH,
                crate::executor::markers::PATTERN_LITERAL_BACKSLASH_STR,
            )
    }

    /// The key of an associative-array subscript, expanded through the one
    /// shared `expand_subscript_string` pass (`subscript_expansion.rs`) that
    /// the arithmetic path also uses, so `${A[key]=v}`, `A[key]=v` and
    /// `(( A[key] ))` always agree.
    pub(in crate::executor) fn assoc_subscript_key(&self, key: &str) -> String {
        // A `\x1d` marker is the lexer's wholly-double-quoted word bookkeeping
        // and never part of the key text.
        self.expand_subscript_string(key)
            .trim_matches(STORAGE_WORD_PREFIX)
            .to_string()
    }

    pub(in crate::executor) fn apply_array_element_parameter_assignment(
        &mut self,
        expression: &str,
        value: String,
    ) -> bool {
        let Some((array_name, key)) = parse_array_subscript(expression) else {
            return false;
        };
        let Some(array_name) = self.resolved_variable_name(array_name) else {
            return false;
        };
        let array_name = array_name.as_str();
        if !is_shell_name(array_name)
            || is_marked_var(&self.shell_state.env_vars, READONLY_VARS, array_name)
            || is_noassign_bash_array(array_name)
        {
            return false;
        }

        // Integer/uppercase/lowercase attributes transform the stored value
        // exactly like the declare assignment path does (arrayfunc.c). This
        // applies to both indexed and associative arrays.
        let value = if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, array_name) {
            match self.eval_arithmetic_expansion_value(&value) {
                Some(evaluated) => evaluated.to_string(),
                None => value,
            }
        } else {
            value
        };
        let value = if is_marked_var(&self.shell_state.env_vars, UPPERCASE_VARS, array_name) {
            value.to_uppercase()
        } else if is_marked_var(&self.shell_state.env_vars, LOWERCASE_VARS, array_name) {
            value.to_lowercase()
        } else {
            value
        };

        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
            let key = self.assoc_subscript_key(key);
            let current = self
                .shell_state
                .env_vars
                .get(array_name)
                .cloned()
                .unwrap_or_default();
            let mut entries = assoc_entries(&current);
            if let Some((_, entry_value)) = entries
                .iter_mut()
                .rev()
                .find(|(entry_key, _)| entry_key == &key)
            {
                *entry_value = value;
            } else {
                entries.push((key, value));
            }
            self.shell_state
                .env_vars
                .insert(array_name.to_string(), format_assoc_storage(entries));
            return true;
        }

        // GNU evaluates indexed-array subscripts arithmetically at assignment
        // time (subst.c/eval_arith_subscript): ${a[$(echo 42)]=x} lands at
        // index 42 instead of being dropped as an unparseable literal key.
        // The @ and * subscripts are expansion operators, not arithmetic
        // operands, and keep their existing handling. The subscript here is
        // the raw ${} inner text, so it receives its single
        // expand_subscript_string pass (Raw) and the product is evaluated
        // under no-expand rules — a surviving $name/$(...) is "operand
        // expected" (expr.c evalerror), not a silent drop.
        let resolved = self.resolve_array_subscript(SubscriptSource::Raw(&key));
        if matches!(resolved.as_str(), "@" | "*") || resolved.trim().is_empty() {
            return false;
        }
        let index = match self.eval_indexed_subscript_expression(&resolved) {
            Some(index) => index,
            None => {
                self.report_indexed_subscript_error(&resolved);
                // Runs during word expansion of a pending command: the
                // evalerror DISCARDs the command itself
                // (eval_indexed_subscript_deferred documents the model).
                self.shell_state.arithmetic_expansion_error.set(true);
                self.shell_state.arithmetic_fatal_error.set(true);
                return false;
            }
        };
        let Ok(index) = usize::try_from(index) else {
            return false;
        };

        let current = self
            .shell_state
            .env_vars
            .get(array_name)
            .cloned()
            .unwrap_or_default();
        let mut entries = indexed_array_entries(&current);
        entries.insert(index, value);
        self.shell_state.env_vars.insert(
            array_name.to_string(),
            format_indexed_array_storage(entries),
        );
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, array_name);
        true
    }
}

/// Replace `\%` with `\x11%` and `\#` with `\x11#` for backslashes that are
/// not themselves escaped, so the escape survives `decode_parameter_pattern_quotes`
/// and the final `\x11 → \\` restoration produces `\%`/`\#` (a glob literal)
/// instead of a bare `%`/`#` (which `replace_parameter_pattern` would treat
/// as a suffix/prefix anchor).
fn mark_escaped_pattern_anchors(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut output = String::with_capacity(pattern.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '\\' => {
                    output.push('\\');
                    output.push('\\');
                    i += 2;
                }
                '%' | '#' => {
                    output.push(crate::executor::markers::CTLESC);
                    output.push(chars[i + 1]);
                    i += 2;
                }
                _ => {
                    output.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            output.push(chars[i]);
            i += 1;
        }
    }
    output
}

/// Scan a pattern word for a single/double quote that never closes
/// (backslash escapes its successor). Mirrors the quote tracking GNU's
/// parse_matched_pair performs while finding the `}` of `${...}`.
fn unclosed_pattern_quote(pattern: &str) -> Option<char> {
    let mut quote = None;
    let mut escaped = false;
    for ch in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' if quote.is_none() => quote = Some('\''),
            '"' if quote.is_none() => quote = Some('"'),
            c if Some(c) == quote => quote = None,
            _ => {}
        }
    }
    quote
}
