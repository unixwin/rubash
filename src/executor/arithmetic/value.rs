use super::{ArithEvalDiag, ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::{
    bash_arith, checked_arithmetic_pow, eval_mutable_arith_value_with_random,
    strip_arith_double_quotes,
};
use crate::executor::{
    array_value_at, assoc_entries, assoc_value_at, current_epoch_seconds,
    env_derived_dynamic_parameter_value, format_assoc_storage, format_indexed_array_storage,
    indexed_array_entries, is_marked_var, is_noassign_bash_array, is_shell_name,
    is_shell_name_char, mark_env_name, next_random_from_state, next_srandom_from_state,
    parse_array_subscript, resolve_indexed_array_subscript, set_process_env, unmark_env_name,
    ARRAY_VARS, ASSOC_128_VARS, ASSOC_VARS, NAMEREF_VARS, READONLY_VARS, SECONDS_OFFSET,
    SHELL_START_EPOCH,
};

impl ConditionalArithParser<'_> {
    pub(super) fn lvalue_value(&mut self, lvalue: &ArithLValue) -> Option<i128> {
        match lvalue {
            ArithLValue::Scalar(name) => self.variable_value(name),
            ArithLValue::Indexed { name, index } => {
                let value = self.env_vars.get(name).and_then(|value| {
                    resolve_indexed_array_subscript(value, *index)
                        .and_then(|index| array_value_at(value, index))
                });
                let value = value.unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{index}]"), &value)
            }
            ArithLValue::IndexedRaw { name, subscript } => {
                // Read context: GNU expr_streval evaluates the subscript at
                // STR-token time (expr.c:1183 array_variable_part ->
                // array_expand_index), once — `a[i++]` reads a[0] and
                // increments i once (verified GNU 5.3).
                let index = self.eval_subscript_index(subscript)?;
                let value = self.env_vars.get(name).and_then(|value| {
                    resolve_indexed_array_subscript(value, index)
                        .and_then(|index| array_value_at(value, index))
                });
                let value = value.unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{index}]"), &value)
            }
            ArithLValue::InvalidElement { display } => {
                // `a[]` read: GNU reports `a[]: bad array subscript` twice —
                // array_variable_part and get_array_value each diagnose the
                // empty subscript (expr.c:1183/1224) — then yields 0.
                self.diags
                    .push(ArithEvalDiag::BadSubscript(display.clone()));
                self.diags
                    .push(ArithEvalDiag::BadSubscript(display.clone()));
                Some(0)
            }
            ArithLValue::Assoc { name, key } => {
                let value = self
                    .env_vars
                    .get(name)
                    .and_then(|value| assoc_value_at(value, key))
                    .unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{key}]"), &value)
            }
        }
    }

    /// Evaluate a raw array-subscript expression, adopting the nested
    /// evaluation's evalerror record (GNU array_expand_index runs the
    /// subscript through its own evalexp frame, so the diagnostic names
    /// the subscript text — arrayfunc.c:1356-1391 — and a failure jumps
    /// DISCARD, which the `__RUBASH_ARITH_SUBSCRIPT_EXPR` marker lets the
    /// caller reproduce).
    pub(super) fn eval_subscript_index(&mut self, subscript: &str) -> Option<i128> {
        // \x1e-marker subscripts were already expanded by the caller's
        // array_expand_index-equivalent pass (mod.rs
        // expand_arith_indexed_subscripts); an empty expansion evaluates
        // to 0, not to a bad subscript.
        let stripped = match super::super::decode_arithmetic_assoc_key(subscript) {
            Some(decoded) => decoded,
            None => {
                // GNU expr.c:1171: when `array_expand_once` is set and the
                // operand was already expanded (EXP_EXPANDED — `let`'s
                // operand; the `__RUBASH_ARITH_EXP_EXPANDED` marker is set
                // by eval_arithmetic_command_value_with_flags), the
                // subscript reaches evalexp verbatim (AV_NOEXPAND) — a
                // `""`/`" "` subscript is junk, not a quote-removed empty
                // (`let 'a[""]=26'` -> `""`: operand expected, verified GNU
                // 5.3). Otherwise the subscript's expand_arith_string pass
                // removes the double quotes here.
                if self
                    .env_vars
                    .get("__RUBASH_ARITH_EXP_EXPANDED")
                    .is_some_and(|v| v == "1")
                {
                    subscript.to_string()
                } else {
                    let resolved = strip_arith_double_quotes(subscript);
                    // GNU array_expand_index (arrayfunc.c:1356-1391) runs the
                    // subscript text through expand_arith_string before
                    // evalexp, so `$name`/`${name}`/`$((...))` inside a
                    // variable value's subscript (`x='b[$d]'; $((x))`)
                    // expands once here (array17.sub). The \x1e-marked path
                    // above already received this pass at the top-level
                    // expression; nested frames reach raw text only. Command
                    // substitution cannot run without an Executor context,
                    // so `$(...)`/backquotes are left for evalexp to reject
                    // (the same failure shape as before).
                    self.expand_subscript_dollar_text(&resolved)
                }
            }
        };
        if stripped.trim().is_empty() {
            return Some(0);
        }
        let (value, _cat) =
            eval_mutable_arith_value_with_random(&stripped, self.env_vars, self.random_state);
        self.adopt_error(super::super::take_arith_eval_error());
        self.adopt_diags(super::super::take_arith_eval_diags());
        if value.is_none() {
            self.env_vars.insert(
                "__RUBASH_ARITH_SUBSCRIPT_EXPR".to_string(),
                stripped.clone(),
            );
        }
        value
    }

    /// expand_arith_string's parameter/arithmetic expansion over subscript
    /// text that reached evalexp unexpanded (variable-value recursion):
    /// `$name`, `${name}` and `$((...))` resolve against env_vars; anything
    /// else (`$(...)`, backquotes, `$@`, escapes) is left verbatim.
    fn expand_subscript_dollar_text(&mut self, text: &str) -> String {
        if !text.contains('$') {
            return text.to_string();
        }
        let bytes = text.as_bytes();
        let mut output = String::with_capacity(text.len());
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] != b'$' {
                let ch = text[index..].chars().next().unwrap_or_default();
                output.push(ch);
                index += ch.len_utf8();
                continue;
            }
            match bytes.get(index + 1) {
                // $((...)) evaluates in place.
                Some(b'(') if bytes.get(index + 2) == Some(&b'(') => {
                    let mut depth = 0usize;
                    let mut end = index + 1;
                    while end < bytes.len() {
                        if bytes[end] == b'(' {
                            depth += 1;
                        } else if bytes[end] == b')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        end += 1;
                    }
                    let inner = text[index + 3..end.saturating_sub(1)].to_string();
                    let (value, _cat) = eval_mutable_arith_value_with_random(
                        &inner,
                        self.env_vars,
                        self.random_state,
                    );
                    match value {
                        Some(value) => {
                            output.push_str(&value.to_string());
                            index = end + 1;
                        }
                        None => {
                            output.push('$');
                            index += 1;
                        }
                    }
                }
                Some(b'{') => {
                    let rest = &text[index + 2..];
                    let close = rest.find('}');
                    match close {
                        Some(close) if rest[..close].chars().all(|ch| is_shell_name_char(ch)) => {
                            let value = self
                                .env_vars
                                .get(&rest[..close])
                                .cloned()
                                .unwrap_or_default();
                            output.push_str(&value);
                            index += 2 + close + 1;
                        }
                        _ => {
                            output.push('$');
                            index += 1;
                        }
                    }
                }
                Some(&next) if next.is_ascii_alphabetic() || next == b'_' => {
                    let mut end = index + 1;
                    while end < bytes.len() && is_shell_name_char(bytes[end] as char) {
                        end += 1;
                    }
                    let value = self
                        .env_vars
                        .get(&text[index + 1..end])
                        .cloned()
                        .unwrap_or_default();
                    output.push_str(&value);
                    index = end;
                }
                _ => {
                    output.push('$');
                    index += 1;
                }
            }
        }
        output
    }

    /// GNU expr.c:269-272 pushexp: evaluation depth reaching
    /// MAX_EXPR_RECURSION_LEVEL (1024) evalerrors "expression recursion
    /// level exceeded" — and since pushexp runs before subexpr installs the
    /// new frame's globals, the diagnostic still describes frame 1023: its
    /// expression text and lasttp.
    ///
    /// Rubash detects the same failure either on a variable-value cycle
    /// (the C equivalent of GNU's longjmp arriving when the depth limit is
    /// reached inside the cycle) or on sheer depth. For a cycle
    /// `C = resolving[j..k]` of length L detected at depth k, frame 1023
    /// evaluates the value of `C[(1022 - k) mod L]` and its error token is
    /// the STR read there, `C[(1023 - k) mod L]` — for `a=b; b=a` GNU
    /// prints `b: expression recursion level exceeded (error token is
    /// "b")`.
    fn record_recursion_error(&mut self, name: &str) {
        if self.error.is_some() {
            return;
        }
        let (expr, tok_name) = match self.resolving.iter().position(|r| r == name) {
            Some(j) => {
                let l = self.resolving.len() - j;
                // The cycle resolving[j..] repeats: name at depth d is
                // resolving[j + (d - j) % l]. GNU fails in pushexp before
                // frame 1024 installs, so the diagnostic describes frame
                // 1023: the expression is the value of the name resolved at
                // depth 1022, the error token the STR read at depth 1023.
                let expr_name = self.resolving[j + (1022usize - j) % l].clone();
                let tok_name = self.resolving[j + (1023usize - j) % l].clone();
                (
                    self.env_vars.get(&expr_name).cloned().unwrap_or(expr_name),
                    tok_name,
                )
            }
            // Depth hit without a detected cycle: this frame is 1023, so
            // its own input is the displayed expression and the STR being
            // resolved is the error token.
            None => (
                String::from_utf8_lossy(self.input).into_owned(),
                name.to_string(),
            ),
        };
        let tok_start = expr.find(tok_name.as_str()).unwrap_or(0);
        let display_end = expr.len();
        self.error = Some(super::ArithEvalError {
            expr,
            msg: "expression recursion level exceeded".to_string(),
            tok_start,
            display_end,
        });
    }

    pub(super) fn variable_value(&mut self, name: &str) -> Option<i128> {
        // GNU expr.c:271 pushexp: only DEPTH is bounded — a name resolving
        // to itself is legal while the recursion converges
        // (arith6.sub: `a[0]` holding `(a[n]=++n)<7&&a[0]` recurses until
        // n reaches 7; `a=a` errors only at depth 1024).
        if self.resolving.len() >= 1023 {
            self.record_recursion_error(name);
            return None;
        }
        if name == "RANDOM" {
            return self
                .random_state
                .map(|state| i128::from(next_random_from_state(state)));
        }
        if name == "SRANDOM" {
            return self
                .random_state
                .map(|state| i128::from(next_srandom_from_state(state)));
        }
        if name == "LINENO" {
            return self
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<i128>().ok())
                .or(Some(1));
        }
        // Dynamic parameters ($SECONDS, $EPOCHSECONDS, ...) never have a
        // stored env_vars entry, so the fallback below would read them as 0.
        // Resolve them through the same path parameter expansion uses.
        if let Some(value) = env_derived_dynamic_parameter_value(self.env_vars, name) {
            if let Ok(number) = value.parse::<i128>() {
                return Some(bash_arith(number));
            }
        }

        // GNU expr.c treats a bare indexed-array operand as element zero.
        // The typed store serializes the whole array, which is not itself an
        // arithmetic expression, so resolve the scalar view before parsing.
        if is_marked_var(self.env_vars, ARRAY_VARS, name) {
            if let Some(value) = self
                .env_vars
                .get(name)
                .and_then(|value| array_value_at(value, 0))
            {
                return self.evaluate_variable_text(name, &value);
            }
        }

        let value = self
            .env_vars
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .unwrap_or_default();
        self.evaluate_variable_text(name, &value)
    }

    pub(super) fn evaluate_variable_text(
        &mut self,
        resolving_name: &str,
        value: &str,
    ) -> Option<i128> {
        // GNU expr.c:271: same depth-only bound as variable_value —
        // re-entering a name is how convergent self-recursion works.
        if self.resolving.len() >= 1023 {
            self.record_recursion_error(resolving_name);
            return None;
        }

        let value = value.trim();
        if value.is_empty() {
            return Some(0);
        }
        if let Ok(number) = value.parse::<i128>() {
            return Some(bash_arith(number));
        }

        let mut resolving = self.resolving.clone();
        resolving.push(resolving_name.to_string());
        let mut parser = ConditionalArithParser {
            input: value.as_bytes(),
            pos: 0,
            env_vars: &mut *self.env_vars,
            resolving,
            random_state: self.random_state,
            error_category: None,
            no_expand: false,
            error: None,
            diags: Vec::new(),
            last_tok_start: 0,
            last_tok_operand: false,
        };
        let value = parser.parse_comma();
        parser.skip_ws();
        let complete = parser.pos == parser.input.len();
        if !complete {
            // GNU expr.c:484-485: the nested frame's subexpr evalerrors on
            // its own trailing input ("in expression") before unwinding.
            parser.record_trailing();
        }
        if let Some(category) = parser.error_category {
            self.error_category = Some(category);
        }
        // GNU expr.c: the nested subexpr frame's evalerror ran through the
        // shared globals — its expression text and lasttp are what the
        // diagnostic prints (expr.c:1241 expr_streval -> subexpr).
        let inner_error = parser.error.take();
        let inner_diags = std::mem::take(&mut parser.diags);
        self.adopt_error(inner_error);
        self.adopt_diags(inner_diags);
        if !complete {
            return None;
        }
        value
    }

    pub(super) fn update_lvalue(
        &mut self,
        lvalue: &ArithLValue,
        delta: i128,
        prefix: bool,
    ) -> Option<i128> {
        // GNU expr.c:1081-1105: post-inc/dec reads the STR's value via
        // expr_streval first (an IndexedRaw subscript evaluates here, once),
        // then binds the saved lvalue.
        let lvalue = self.resolve_raw_subscript(lvalue)?;
        if !self.lvalue_is_writable(&lvalue) {
            return None;
        }
        let current = self.lvalue_value(&lvalue)?;
        let updated = bash_arith(current + delta);
        self.set_lvalue(&lvalue, updated);
        Some(if prefix { updated } else { current })
    }

    pub(super) fn assign_lvalue(
        &mut self,
        lvalue: &ArithLValue,
        op: &str,
        rhs: i128,
    ) -> Option<i128> {
        // Resolve a deferred (raw) subscript now — after the RHS has been
        // evaluated, so side effects in the RHS are visible to the subscript
        // (GNU expr.c:1395-1401 + expr_bind_variable re-evaluation).
        let lvalue = self.resolve_raw_subscript(lvalue)?;
        if !self.lvalue_is_writable(&lvalue) {
            return None;
        }
        if op == "=" {
            self.set_lvalue(&lvalue, rhs);
            return Some(rhs);
        }
        let current = self.lvalue_value(&lvalue)?;
        let value = match op {
            "+=" => bash_arith(current + rhs),
            "-=" => bash_arith(current - rhs),
            "*=" => bash_arith(current * rhs),
            "**=" => checked_arithmetic_pow(current, rhs)?,
            "<<=" => bash_arith((current as i64).wrapping_shl(u32::try_from(rhs).ok()?) as i128),
            ">>=" => bash_arith((current as i64).wrapping_shr(u32::try_from(rhs).ok()?) as i128),
            "&=" => bash_arith(current & rhs),
            "^=" => bash_arith(current ^ rhs),
            "|=" => bash_arith(current | rhs),
            "/=" if rhs != 0 => bash_arith((current as i64).wrapping_div(rhs as i64) as i128),
            "%=" if rhs != 0 => {
                // GNU expr.c:923-926: INTMAX_MIN % -1 is 0.
                if current == i128::from(i64::MIN) && rhs == -1 {
                    0
                } else {
                    current % rhs
                }
            }
            "/=" | "%=" => return None,
            _ => return None,
        };
        self.set_lvalue(&lvalue, value);
        Some(value)
    }

    /// Evaluate a deferred raw subscript into a concrete `Indexed` lvalue.
    /// Non-raw lvalues pass through unchanged.
    fn resolve_raw_subscript(&mut self, lvalue: &ArithLValue) -> Option<ArithLValue> {
        match lvalue {
            ArithLValue::IndexedRaw { name, subscript } => {
                let index = self.eval_subscript_index(subscript)?;
                Some(ArithLValue::Indexed {
                    name: name.clone(),
                    index,
                })
            }
            other => Some(other.clone()),
        }
    }

    fn lvalue_is_writable(&mut self, lvalue: &ArithLValue) -> bool {
        let name = match lvalue {
            ArithLValue::Scalar(name)
            | ArithLValue::Indexed { name, .. }
            | ArithLValue::IndexedRaw { name, .. }
            | ArithLValue::Assoc { name, .. } => name,
            // `a[]` has no writable name — GNU's bind fails later with
            // `not a valid identifier`; there is no readonly check.
            ArithLValue::InvalidElement { .. } => return true,
        };
        if is_marked_var(self.env_vars, READONLY_VARS, name) {
            self.env_vars
                .insert("__RUBASH_ARITH_READONLY_ERROR".to_string(), name.clone());
            return false;
        }
        true
    }

    pub(super) fn set_lvalue(&mut self, lvalue: &ArithLValue, value: i128) {
        match lvalue {
            ArithLValue::Scalar(name) => self.set_variable(name, value),
            ArithLValue::Indexed { name, index } => self.set_array_element(name, *index, value),
            ArithLValue::IndexedRaw { .. } => {
                // Should have been resolved by resolve_raw_subscript; no-op.
            }
            ArithLValue::InvalidElement { display } => {
                // `a[]` on the bind side: GNU expr_bind_variable ->
                // bind_variable fails the `a[]` name via sh_invalidid —
                // `` `a[]': not a valid identifier `` (non-fatal; the
                // expression value is unaffected).
                self.diags
                    .push(ArithEvalDiag::InvalidIdentifier(display.clone()));
            }
            ArithLValue::Assoc { name, key } => self.set_assoc_element(name, key, value),
        }
    }

    pub(super) fn set_variable(&mut self, name: &str, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        let value = bash_arith(value).to_string();
        if name == "SECONDS" {
            // Assignment resets the reference point so the dynamic value
            // becomes the assigned number and grows from there, matching
            // the parameter-assignment path in temporary_assignments.rs.
            let assigned = value.parse::<i64>().unwrap_or(0);
            let start = self
                .env_vars
                .get(SHELL_START_EPOCH)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_else(current_epoch_seconds);
            let elapsed = current_epoch_seconds() - start;
            self.env_vars
                .insert(SECONDS_OFFSET.to_string(), (assigned - elapsed).to_string());
            set_process_env(name, value);
            return;
        }
        if name == "RANDOM" {
            if let Some(state) = self.random_state {
                // GNU variables.c:1393-1408 assign_random: a non-numeric
                // value fails valid_number and returns without reseeding;
                // a numeric seed runs sbrand — rseed = seed,
                // last_random_value = 0.
                if let Ok(seed) = value.trim().parse::<i64>() {
                    state.rseed.set(seed as u32);
                    state.last_value.set(0);
                }
            }
        }
        if name == "SRANDOM" {
            return;
        }
        // GNU expr.c assigns through bind_variable: a nameref lvalue resolves
        // to its cell — an empty cell adopts the assigned text after
        // valid_nameref_value (invalid -> sh_invalidid via the marker below),
        // an already-invalid cell stays unchanged, and a valid cell forwards
        // the write to the referenced variable or element.
        if is_marked_var(self.env_vars, NAMEREF_VARS, name) {
            let cell = self.env_vars.get(name).cloned().unwrap_or_default();
            let cell_valid = is_shell_name(&cell) || parse_array_subscript(&cell).is_some();
            if !cell_valid {
                if cell.is_empty() {
                    if is_shell_name(&value) || parse_array_subscript(&value).is_some() {
                        let old_value = self.env_vars.get(name).cloned();
                        self.env_vars.insert(name.to_string(), value.clone());
                        super::super::record_arith_write(name, old_value);
                        set_process_env(name, value);
                    } else {
                        self.env_vars
                            .insert("__RUBASH_ARITH_NAMEREF_ERROR".to_string(), value);
                    }
                }
                return;
            }
            // Follow the chain to the last resolvable cell (NAMEREF_MAX=8).
            let mut target = cell;
            for _ in 0..8 {
                let base = target.split('[').next().unwrap_or(target.as_str());
                if !is_marked_var(self.env_vars, NAMEREF_VARS, base) {
                    break;
                }
                let next = self.env_vars.get(base).cloned().unwrap_or_default();
                if next.is_empty() || next == target {
                    break;
                }
                target = next;
            }
            if let Some((elem_base, subscript)) = target.split_once('[') {
                if let Some(subscript) = subscript.strip_suffix(']') {
                    let stripped = strip_arith_double_quotes(subscript);
                    let (index, _cat) = eval_mutable_arith_value_with_random(
                        &stripped,
                        self.env_vars,
                        self.random_state,
                    );
                    if let Some(index) = index {
                        self.set_array_element(
                            elem_base,
                            index,
                            value.parse::<i128>().unwrap_or(0),
                        );
                        return;
                    }
                }
            }
            let base_target = target.split('[').next().unwrap_or(target.as_str());
            if is_marked_var(self.env_vars, READONLY_VARS, base_target) {
                self.env_vars.insert(
                    "__RUBASH_ARITH_READONLY_ERROR".to_string(),
                    base_target.to_string(),
                );
                return;
            }
            let numeric = value.parse::<i128>().unwrap_or(0);
            self.set_variable(&base_target.to_string(), numeric);
            return;
        }
        let old_value = self.env_vars.get(name).cloned();
        self.env_vars.insert(name.to_string(), value.clone());
        super::super::record_arith_write(name, old_value);
        set_process_env(name, value);
    }

    pub(super) fn set_array_element(&mut self, name: &str, index: i128, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        let mut entries = self
            .env_vars
            .get(name)
            .map(|value| indexed_array_entries(value))
            .unwrap_or_default();
        let index = if index < 0 {
            let storage = format_indexed_array_storage(entries.clone());
            let Some(index) = resolve_indexed_array_subscript(&storage, index) else {
                return;
            };
            index
        } else {
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            index
        };
        entries.insert(index, value.to_string());
        let value = format_indexed_array_storage(entries);
        let old_value = self.env_vars.get(name).cloned();
        self.env_vars.insert(name.to_string(), value);
        super::super::record_arith_write(name, old_value);
        mark_env_name(self.env_vars, ARRAY_VARS, name);
    }

    pub(super) fn set_assoc_element(&mut self, name: &str, key: &str, value: i128) {
        let mut entries = self
            .env_vars
            .get(name)
            .map(|value| assoc_entries(value))
            .unwrap_or_default();
        let value = value.to_string();
        if let Some((_, existing)) = entries.iter_mut().find(|(entry_key, _)| entry_key == key) {
            *existing = value;
        } else {
            entries.push((key.to_string(), value));
        }
        let old_value = self.env_vars.get(name).cloned();
        let had_value = old_value.is_some();
        self.env_vars
            .insert(name.to_string(), format_assoc_storage(entries));
        super::super::record_arith_write(name, old_value);
        if !is_marked_var(self.env_vars, ASSOC_VARS, name) {
            // GNU arrayfunc.c:481/497 find_or_make_array_variable: an
            // existing scalar converts through convert_var_to_assoc (a
            // 128-bucket table); an unbound name takes a fresh
            // ASSOC_HASH_BUCKETS table.
            if had_value {
                mark_env_name(self.env_vars, ASSOC_128_VARS, name);
            } else {
                unmark_env_name(self.env_vars, ASSOC_128_VARS, name);
            }
        }
        mark_env_name(self.env_vars, ASSOC_VARS, name);
    }
}
