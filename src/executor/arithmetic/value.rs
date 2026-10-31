use super::{ArithEvalDiag, ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::{
    bash_arith, eval_mutable_arith_value_with_random, strip_arith_double_quotes,
};
use crate::executor::{
    array_value_at, assoc_entries, assoc_value_at, current_epoch_seconds,
    env_derived_dynamic_parameter_value, format_assoc_storage, format_indexed_array_storage,
    indexed_array_entries, is_array_storage, is_marked_var, is_noassign_bash_array, is_shell_name,
    is_shell_name_char, mark_env_name, next_random_from_state, next_srandom_from_state,
    parse_array_subscript, resolve_indexed_array_subscript, set_process_env,
    sync_shell_assignment_process_env, unmark_env_name, ARRAY_VARS, ASSOC_128_VARS, ASSOC_VARS,
    NAMEREF_VARS, READONLY_VARS, SECONDS_OFFSET, SHELL_START_EPOCH, UNSET_DYNAMIC_VARS,
};

impl ConditionalArithParser<'_> {
    /// Element text for an indexed arithmetic read. PIPESTATUS
    /// consults the live-cell snapshot first (rubash#1364 R3): the
    /// env mirror never carries pipeline results — the completion
    /// writer updates the cell state only — so the mirror leg would
    /// read a stale assignment-era value (or nothing). Every other
    /// array reads through the mirror storage as before.
    fn indexed_element_text(&self, name: &str, index: i128) -> Option<String> {
        if name == "PIPESTATUS" {
            if let Some(verdict) = self
                .dynamic_values
                .and_then(|context| context.pipestatus_element(index))
            {
                return verdict;
            }
        }
        // GNU get_array_value -> arrayfunc.c:1580-1586
        // array_value_internal: a negative subscript rebases ONLY
        // against a real array cell; a scalar (or unset) base keeps
        // the negative index, which is INDEX_ERROR and reads as
        // unset. The mirror resolve below is dense (it rebases
        // against the storage length), so guard the non-array base
        // here; the diagnostic itself is the caller's (see
        // arith_negative_subscript_is_bad, rubash#1555).
        if index < 0 && !self.arith_base_is_array(name) {
            return None;
        }
        // Typed-cell leg: when the Executor supplied the store and no
        // unsynced mirror write is pending, the complete cell (rubash#616
        // Phase 1a invariant) answers in O(log n) — the mirror leg below
        // re-hashes the whole serialized array per read (Θ(bytes)), which
        // is what made arithmetic reads scale linearly with array size.
        // PIPESTATUS is excluded: its reads are owned by the dynamic
        // context snapshot above (rubash#1364 R3). A cell miss falls
        // through to the mirror leg unchanged (holes in a sparse cell
        // and mirror-only arrays keep their existing answers).
        if name != "PIPESTATUS" && !self.env_vars.array_cells_dirty() {
            if let Some(cells) = self.cells {
                if index >= 0 {
                    if let Ok(typed_index) = i64::try_from(index) {
                        if let Some(text) = cells.indexed_element(name, typed_index) {
                            return Some(crate::executor::arrays::normalize_array_expanded_value(
                                text.to_string(),
                            ));
                        }
                    }
                } else if let Some((_, max_index)) = cells.indexed_array_shape(name) {
                    // Negative subscript: the cell shape rebases exactly
                    // like resolve_indexed_array_subscript does against
                    // the mirror's max index. A still-negative result is
                    // INDEX_ERROR (the caller diagnoses it); an absent
                    // resolved index is an unset element — both are the
                    // mirror leg's answers, reached without the parse.
                    let resolved = match max_index {
                        Some(max) => i128::from(max) + 1 + index,
                        None => -1,
                    };
                    if resolved < 0 {
                        return None;
                    }
                    if let Ok(typed_index) = i64::try_from(resolved) {
                        if let Some(text) = cells.indexed_element(name, typed_index) {
                            return Some(crate::executor::arrays::normalize_array_expanded_value(
                                text.to_string(),
                            ));
                        }
                        return None;
                    }
                }
            }
        }
        self.env_vars.get(name).and_then(|value| {
            resolve_indexed_array_subscript(value, index)
                .and_then(|resolved| array_value_at(value, resolved))
        })
    }

    /// Whether the arithmetic base cell for `name` is a real array
    /// (GNU array_p(var)): the ARRAY_VARS mark or array-shaped
    /// mirror storage. Assoc cells take the string-key paths and
    /// never reach the indexed reads.
    fn arith_base_is_array(&self, name: &str) -> bool {
        is_marked_var(self.env_vars, ARRAY_VARS, name)
            || self
                .env_vars
                .get(name)
                .is_some_and(|value| is_array_storage(value))
    }

    /// GNU array_value_internal's INDEX_ERROR verdict for a
    /// negative arithmetic subscript (rubash#1555): a non-array base
    /// is always bad; an array base is bad only when the max-index
    /// rebase still lands below zero (an empty array is bad for
    /// every negative subscript).
    fn arith_negative_subscript_is_bad(&self, name: &str, index: i128) -> bool {
        if name == "PIPESTATUS" {
            return self
                .dynamic_values
                .is_none_or(|context| context.pipestatus_negative_is_bad(index));
        }
        if !self.arith_base_is_array(name) {
            return true;
        }
        // The typed cell's shape carries the same max index the mirror
        // parse below would fold to, without building the entry map.
        if name != "PIPESTATUS" && !self.env_vars.array_cells_dirty() {
            if let Some(cells) = self.cells {
                if let Some((_, max_index)) = cells.indexed_array_shape(name) {
                    return match max_index {
                        Some(max_index) => i128::from(max_index) + 1 + index < 0,
                        None => true,
                    };
                }
            }
        }
        let Some(value) = self.env_vars.get(name) else {
            return true;
        };
        match indexed_array_entries(value).into_iter().last() {
            Some((max_index, _)) => max_index as i128 + 1 + index < 0,
            None => true,
        }
    }

    pub(super) fn lvalue_value(&mut self, lvalue: &ArithLValue) -> Option<i128> {
        // GNU expr.c:1164-1167 expr_streval: under noeval the whole read
        // (array lookup, subscript evaluation, dynamic special variables)
        // short-circuits to 0 before any side effect runs.
        if self.noeval > 0 {
            return Some(0);
        }
        match lvalue {
            ArithLValue::Scalar(name) => self.variable_value(name),
            ArithLValue::Indexed { name, index } => {
                let value = self.indexed_element_text(name, *index);
                // GNU get_array_value -> array_value_internal
                // INDEX_ERROR: an unresolvable negative subscript
                // reports `name: bad array subscript` and reads as 0
                // (rubash#1555; the dense mirror resolve used to hand
                // back the scalar value / stay silent).
                if *index < 0 && self.arith_negative_subscript_is_bad(name, *index) {
                    self.diags.push(ArithEvalDiag::BadSubscript(name.clone()));
                }
                let value = value.unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{index}]"), &value)
            }
            ArithLValue::IndexedRaw { name, subscript } => {
                // Read context: GNU expr_streval evaluates the subscript at
                // STR-token time (expr.c:1183 array_variable_part ->
                // array_expand_index), once — `a[i++]` reads a[0] and
                // increments i once (verified GNU 5.3).
                let index = self.eval_subscript_index(subscript)?;
                let value = self.indexed_element_text(name, index);
                if index < 0 && self.arith_negative_subscript_is_bad(name, index) {
                    self.diags.push(ArithEvalDiag::BadSubscript(name.clone()));
                }
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
            ArithLValue::BadSubscriptElement { display } => {
                // `a[@]`/`a[*]` read: GNU array_expand_index diagnoses
                // `a[@]: bad array subscript` ONCE and the element reads
                // as 0 (arrayfunc.c; verified GNU 5.3.20, rubash#1136).
                self.diags
                    .push(ArithEvalDiag::BadSubscript(display.clone()));
                Some(0)
            }
            ArithLValue::AssocEmptyKey { name, .. } => {
                // `C['']` read: the akey failed `*akey == 0` inside
                // array_value_internal — GNU INDEX_ERROR diagnoses the bare
                // NAME once (arrayfunc.c:1462-1470 err_badarraysub(var->name))
                // and the element reads as 0.
                self.diags.push(ArithEvalDiag::BadSubscript(name.clone()));
                Some(0)
            }
            ArithLValue::Assoc { name, key } => {
                // Typed-cell leg, same discipline as indexed_element_text:
                // a hit answers from the complete cell; a miss keeps the
                // mirror leg (which also owns mirror-only assoc state).
                if !self.env_vars.array_cells_dirty() {
                    if let Some(cells) = self.cells {
                        if let Some(text) = cells.associative_element(name, key) {
                            let value = text.to_string();
                            return self.evaluate_variable_text(&format!("{name}[{key}]"), &value);
                        }
                    }
                }
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
        // GNU expr.c:1164-1167: subscript evaluation lives inside
        // expr_streval (array_variable_part -> array_expand_index), which
        // returns 0 under noeval before touching the subscript — so
        // `0?a[n++]+=5:2` never bumps n.
        if self.noeval > 0 {
            return Some(0);
        }
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
        let (value, _cat) = eval_mutable_arith_value_with_random(
            &stripped,
            self.env_vars,
            self.random_state,
            self.dynamic_values,
            self.cells,
        );
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
                        self.dynamic_values,
                        self.cells,
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
        // GNU variables.c:3839 unbind_variable: `unset -v` unbinds a
        // dynamic variable permanently — the name then reads like an
        // ordinary unset variable through the env fallback below.
        if !is_marked_var(self.env_vars, UNSET_DYNAMIC_VARS, name) {
            if name == "RANDOM" {
                return self
                    .random_state
                    .map(|state| i128::from(next_random_from_state(state, self.env_vars)));
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
            // Executor-injected dynamic values (BASHPID, BASH_SUBSHELL,
            // ...) — GNU expr.c:1150 expr_streval reaches them through
            // find_variable; the evaluator has no Executor handle, so they
            // arrive as a lazily-resolved context (the INIT_DYNAMIC_VAR
            // getter port). String values (SHELLOPTS, BASH_COMMAND)
            // re-enter as nested expressions, matching expr_streval's
            // evalexp recursion.
            if let Some(context) = self.dynamic_values {
                if let Some(value) = context.resolve(self.env_vars, name) {
                    return self.evaluate_variable_text(name, &value);
                }
            }
        }

        // GNU expr.c treats a bare indexed-array operand as element zero.
        // The typed store serializes the whole array, which is not itself an
        // arithmetic expression, so resolve the scalar view before parsing.
        // An unset element zero (an empty array, or a sparse array with no
        // [0]) evaluates 0 — the same empty-expanded-text-is-0 contract as
        // the empty-subscript read (arrayfunc.c:1356 -> expr.c:417); it
        // must NOT fall through to the serialized storage text, which is
        // not an expression (niubash#508: `a=(); $((a))` reported
        // `error token is "()"`).
        if is_marked_var(self.env_vars, ARRAY_VARS, name) {
            return match self
                .env_vars
                .get(name)
                .and_then(|value| array_value_at(value, 0))
            {
                Some(value) => self.evaluate_variable_text(name, &value),
                None => Some(0),
            };
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
        // GNU expr.c:1557 strlong (contract at expr.c:1539: "0nnn ->
        // base 8"): a variable value with a leading zero and only octal
        // digits re-parses octal — `x=010; $((x))` is 8 (rubash#341).
        // `0x` hex and `base#n` forms keep flowing to the nested
        // expression parser below.
        if let Some(number) = strlong_leading_zero_octal(value) {
            return Some(bash_arith(number));
        }
        // rubash#1101: a leading-zero digit string containing 8/9 (`08`,
        // `-08`, `+09`) is NOT decimal — GNU strlong takes the octal arm
        // for every `0nnn` value and evalerrors "value too great for
        // base" (expr.c:1539-1557). The plain-decimal shortcut must not
        // swallow it; let it fall through to the nested expression parser,
        // whose literal reader raises exactly that error (the same path
        // that already handles `08x` / `0x1g` values correctly).
        if !leading_zero_digit_string(value) {
            if let Ok(number) = value.parse::<i128>() {
                return Some(bash_arith(number));
            }
        }

        let mut resolving = self.resolving.clone();
        resolving.push(resolving_name.to_string());
        let mut parser = ConditionalArithParser {
            input: value.as_bytes(),
            pos: 0,
            env_vars: &mut *self.env_vars,
            resolving,
            random_state: self.random_state,
            dynamic_values: self.dynamic_values,
            cells: self.cells,
            error_category: None,
            no_expand: false,
            error: None,
            diags: Vec::new(),
            last_tok_start: 0,
            last_tok_operand: false,
            noeval: 0,
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
        // GNU expr.c:1081-1105: under noeval the STR's value came from the
        // expr_streval short-circuit (0) and the ++/-- bind is skipped
        // entirely — `7||x++` leaves x untouched and evaluates to 0.
        if self.noeval > 0 {
            return Some(0);
        }
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
        self.assign_lvalue_precaptured(lvalue, op, rhs, None)
    }

    /// `precaptured` is the lvalue's value read BEFORE the RHS was parsed
    /// (GNU expassign's expcond read, expr.c:511-626); compound assignment
    /// must combine THAT value with the RHS, not a post-RHS re-read
    /// (rubash#1113). Callers that evaluate the RHS before ever seeing the
    /// lvalue pass None and keep the re-read.
    pub(super) fn assign_lvalue_precaptured(
        &mut self,
        lvalue: &ArithLValue,
        op: &str,
        rhs: i128,
        precaptured: Option<i128>,
    ) -> Option<i128> {
        // GNU expr.c:607-615 expassign: the bind is skipped under noeval —
        // no readonly check, no current-value read, no write. The computed
        // value is still returned to the caller.
        if self.noeval > 0 {
            return Some(rhs);
        }
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
        let current = match precaptured {
            Some(value) => value,
            None => self.lvalue_value(&lvalue)?,
        };
        let value = match op {
            "+=" => bash_arith(current + rhs),
            "-=" => bash_arith(current - rhs),
            "*=" => bash_arith(current * rhs),
            // GNU expr.c expshift C-shift semantics (rubash#1135): the
            // count wraps modulo the word size; it never evalerrors.
            "<<=" => bash_arith((current as i64).wrapping_shl(rhs as u32) as i128),
            ">>=" => bash_arith((current as i64).wrapping_shr(rhs as u32) as i128),
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
            // `a[@]` bind likewise fails as a bad subscript, never as a
            // readonly error (rubash#1136).
            ArithLValue::BadSubscriptElement { .. } => return true,
            // GNU rejects the empty akey at arrayfunc.c:402-407 BEFORE the
            // readonly check inside bind_assoc_variable, so the empty-key
            // bind is never a readonly failure.
            ArithLValue::AssocEmptyKey { .. } => return true,
        };
        if is_marked_var(self.env_vars, READONLY_VARS, name) {
            self.env_vars
                .insert("__RUBASH_ARITH_READONLY_ERROR".to_string(), name.clone());
            return false;
        }
        // GNU expr.c:350 (expr_bind_variable -> bind_variable): an
        // att_noassign lvalue fails the evaluation the same way
        // (sh_longjmp out of evalexp) but with NO diagnostic — the
        // drain in arithmetic/mod.rs latches the failure silently
        // (rubash#2369).
        if is_noassign_bash_array(name) {
            self.env_vars
                .insert("__RUBASH_ARITH_NOASSIGN_ERROR".to_string(), name.clone());
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
            ArithLValue::BadSubscriptElement { display } => {
                // `a[@] = v` bind: GNU diagnoses `a[@]: bad array
                // subscript` once and stores nothing (rubash#1136).
                self.diags
                    .push(ArithEvalDiag::BadSubscript(display.clone()));
            }
            ArithLValue::AssocEmptyKey { bind_display, .. } => {
                // `C['']` on the bind side: GNU assign_array_element_internal
                // fails the empty akey with err_badarraysub —
                // `C['']: bad array subscript` (arrayfunc.c:402-407) — and
                // binds nothing.
                self.diags
                    .push(ArithEvalDiag::BadSubscript(bind_display.clone()));
            }
            ArithLValue::Assoc { name, key } => self.set_assoc_element(name, key, value),
        }
    }

    pub(super) fn set_variable(&mut self, name: &str, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        // A scalar bind over an array-shaped binding replaces the
        // variable wholesale: the typed cell still holds the old array,
        // so — exactly like the element binds in set_array_element /
        // set_assoc_element — mark the cells dirty so element reads
        // later in this evaluation take the (updated) mirror instead
        // of trusting the cell. The check is one map probe plus a
        // storage-shape test on the outgoing value.
        if self
            .env_vars
            .get(name)
            .is_some_and(|old| is_array_storage(old))
            || is_marked_var(self.env_vars, ARRAY_VARS, name)
            || is_marked_var(self.env_vars, ASSOC_VARS, name)
        {
            self.env_vars.mark_array_cells_dirty();
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
                        sync_shell_assignment_process_env(self.env_vars, name, &value);
                        // rubash#1551 sv_path: initializing a nameref
                        // cell named PATH is still a PATH bind.
                        if name == "PATH" {
                            crate::builtins::hash::flush_command_hash_table(self.env_vars);
                        }
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
                        self.dynamic_values,
                        self.cells,
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
            // GNU expr.c:350: a nameref chain ending at an att_noassign
            // variable fails the bind silently (rubash#2369).
            if is_noassign_bash_array(base_target) {
                self.env_vars.insert(
                    "__RUBASH_ARITH_NOASSIGN_ERROR".to_string(),
                    base_target.to_string(),
                );
                return;
            }
            let numeric = value.parse::<i128>().unwrap_or(0);
            self.set_variable(&base_target.to_string(), numeric);
            return;
        }
        // GNU expr.c: an unsubscripted array name in arithmetic denotes
        // element 0 for writes as well as reads — the bind lands on that
        // element and the remaining elements survive; rebinding the name
        // to the scalar text collapses the whole array (rubash#1614). An
        // associative name takes key "0" the same way (verified GNU
        // 5.3.20: declare -A m=([x]=1 [y]=2); $((m=5)) leaves
        // m=([0]="5" [y]="2" [x]="1")).
        if is_marked_var(self.env_vars, ARRAY_VARS, name) {
            let numeric = value.parse::<i128>().unwrap_or(0);
            self.set_array_element(name, 0, numeric);
            return;
        }
        if is_marked_var(self.env_vars, ASSOC_VARS, name) {
            let numeric = value.parse::<i128>().unwrap_or(0);
            self.set_assoc_element(name, "0", numeric);
            return;
        }
        let old_value = self.env_vars.get(name).cloned();
        self.env_vars.insert(name.to_string(), value.clone());
        super::super::record_arith_write(name, old_value);
        // GNU expr.c's assignment lands on the shell variable; the process
        // env mirror only tracks exported (or host-special) names — same
        // rule as sync_shell_assignment_process_env. `i++` in a for-update
        // paid a Win32 SetEnvironmentVariable per iteration.
        sync_shell_assignment_process_env(self.env_vars, name, &value);
        // GNU variables.c:5819/5919 sv_path -> phash_flush (rubash#1551):
        // an arithmetic bind of PATH is a bind.
        if name == "PATH" {
            crate::builtins::hash::flush_command_hash_table(self.env_vars);
        }
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
        // No structured store here (parser context): mark typed cells
        // dirty so reads re-parse instead of trusting a stale cell.
        self.env_vars.mark_array_cells_dirty();
    }

    pub(super) fn set_assoc_element(&mut self, name: &str, key: &str, value: i128) {
        if key.is_empty() {
            // GNU arrayfunc.c:402-407 assign_array_element_internal: an empty
            // akey fails err_badarraysub and returns NULL before
            // bind_assoc_variable — nothing is stored. (The `(( C['']=1 ))`
            // family reaches its own diagnostic through ArithLValue::
            // AssocEmptyKey; this guard covers any other caller spelling.)
            return;
        }
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
        // No structured store here (parser context): mark typed cells
        // dirty so assoc reads re-parse instead of trusting a stale cell
        // (same discipline as the indexed element store above).
        self.env_vars.mark_array_cells_dirty();
    }
}

/// Whether a variable value is a pure digit string (optional sign) with
/// a leading zero and more than one digit (`08`, `010`, `-09`): GNU
/// strlong reads every such value in the octal arm, never as decimal.
fn leading_zero_digit_string(value: &str) -> bool {
    let digits = match value.as_bytes().first() {
        Some(b'-' | b'+') => &value[1..],
        _ => value,
    };
    digits.len() > 1 && digits.starts_with('0') && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// GNU expr.c:1539/1557 strlong's `0nnn -> base 8` arm for variable
/// VALUES: a leading zero followed only by octal digits (with an optional
/// sign) is octal. Returns None for `0`, `0x...`, non-octal tails like
/// `09`, and non-numeric text so those keep their existing paths.
fn strlong_leading_zero_octal(value: &str) -> Option<i128> {
    let (sign, digits) = match value.as_bytes().first() {
        Some(b'-') => (-1i128, &value[1..]),
        Some(b'+') => (1i128, &value[1..]),
        _ => (1i128, value),
    };
    let rest = digits.strip_prefix('0')?;
    if rest.is_empty() || !rest.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return None;
    }
    i128::from_str_radix(rest, 8)
        .ok()
        .map(|number| sign * number)
}
