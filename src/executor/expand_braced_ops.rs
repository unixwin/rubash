use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_operator_or_array_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        // GNU subst.c:8927 parameter_brace_transform: for an unset variable
        // the `@xform` transform returns NULL before the transform's
        // validity is even checked, so `${unset@-d}` expands empty rather
        // than letting `-d` parse as a default-value operator on `x@`.
        // Set variables with an invalid xform were already reported as a
        // fatal bad substitution by the parameter-error pre-scan.
        if let Some(base) = invalid_at_transform_base(name) {
            let is_set = self.parameter_error_value(base).is_some()
                || self.shell_state.env_vars.contains_key(base);
            if !is_set {
                return Some(String::new());
            }
        }
        if let Some((var_name, word)) = split_once_outside_subscript_str(name, ":=") {
            if self
                .parameter_operator_value(var_name)
                .is_some_and(|value| !value.is_empty())
            {
                return Some(
                    self.parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default(),
                );
            }
            // GNU evaluates the `:=` operator once per `${}` occurrence
            // (subst.c:9777 parameter_brace_expand -> subst.c:10346
            // `case '='` with check_nullness -> parameter_brace_expand_rhs).
            // When the pre-scan already applied this fragment and the
            // alternate resolved to NULL (the set-check above fails),
            // reuse the memoized result instead of re-expanding the
            // alternate — a `$(...)` alternate would otherwise re-execute.
            if let Some(key) = crate::executor::expand_braced_indices::sub_site_key(name) {
                if let Some(resolved) =
                    crate::executor::expand_braced_indices::assign_applied_lookup(&key)
                {
                    return Some(resolved);
                }
            }
            // GNU parameter_brace_expand_word sets expand_no_split_dollar_star
            // for op == '=' (subst.c:4487), which includes `:=`. This makes
            // unquoted $* with null IFS join with IFS[0] inside the value.
            let old = super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
            let result = self.expand_parameter_word(word);
            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
            return Some(result);
        }
        if let Some((var_name, word)) = split_once_outside_subscript_str(name, ":-") {
            if self
                .parameter_operator_value(var_name)
                .is_some_and(|value| !value.is_empty())
            {
                return Some(
                    self.parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default(),
                );
            }
            return Some(self.expand_parameter_word(word));
        }
        if let Some((var_name, word)) = split_once_outside_subscript_str(name, ":+") {
            if self
                .parameter_operator_value(var_name)
                .is_some_and(|value| !value.is_empty())
            {
                return Some(self.expand_parameter_word(word));
            }
            return Some(String::new());
        }
        if let Some((var_name, word)) = split_once_outside_subscript_str(name, ":?") {
            if !var_name.is_empty() {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return Some(
                        self.parameter_operator_value(var_name)
                            .map(|value| shell_safe_value(&value))
                            .unwrap_or_default(),
                    );
                }
                return Some(self.expand_parameter_word(word));
            }
        }
        if let Some((var_name, word)) = split_once_outside_subscript(name, '?') {
            if !var_name.is_empty() {
                return Some(
                    self.parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_else(|| self.expand_parameter_word(word)),
                );
            }
        }
        if let Some((var_name, word)) = split_once_outside_subscript(name, '=') {
            // GNU parameter_brace_expand_word sets expand_no_split_dollar_star
            // for op == '=' (subst.c:4487). This makes unquoted $* with null
            // IFS join with IFS[0] inside the value (exp11.sub ${c=${*/}}).
            let old = super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
            let result = self
                .parameter_operator_value(var_name)
                .map(|value| shell_safe_value(&value))
                .unwrap_or_else(|| self.expand_parameter_word(word));
            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
            return Some(result);
        }
        if let Some((var_name, word)) = split_once_outside_subscript(name, '+') {
            if self.parameter_operator_value(var_name).is_some() {
                return Some(self.expand_parameter_word(word));
            }
            return Some(String::new());
        }
        if let Some((array_name, index)) = parse_array_integer_subscript(name) {
            if array_name == "GROUPS" {
                let Ok(index) = usize::try_from(index) else {
                    return Some(String::new());
                };
                return Some(self.group_value_at(index).unwrap_or_default());
            }
            // rubash#375: dynamic stack arrays read the element straight
            // from the live view (silent None posture, same as the storage
            // path this replaces).
            if let Some(values) = self.dynamic_stack_array_values(array_name) {
                return Some(dense_view_element(&values, index).unwrap_or_default());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .and_then(|value| {
                        resolve_indexed_array_subscript(&value, index)
                            .and_then(|index| array_value_at(&value, index))
                    })
                    .map(normalize_array_expanded_value)
                    .unwrap_or_default(),
            );
        }
        if let Some((var_name, word)) = split_once_outside_subscript(name, '-') {
            return Some(
                self.parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_parameter_word(word)),
            );
        }
        if let Some((array_name, default)) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .and_then(|array_name| {
                split_once_outside_subscript(array_name, '-').map(|_| (array_name, ""))
            })
        {
            // rubash#375: dynamic stack arrays join from the live view. The
            // empty-view test mirrors the storage path's `!value.is_empty()`
            // filter exactly (an empty view renders the empty storage
            // string; a non-empty view never does, even when every element
            // is empty).
            if let Some(values) = self.dynamic_stack_array_values(array_name) {
                if values.is_empty() {
                    return Some(default.to_string());
                }
                let joined = self.dynamic_array_joined(name).unwrap_or_default();
                return Some(joined);
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| self.join_array_parameter_values(&value, name))
                    .unwrap_or_else(|| default.to_string()),
            );
        }
        if let Some((array_expr, default)) = split_once_outside_subscript(name, '-') {
            if let Some(array_name) = array_expr
                .strip_suffix("[@]")
                .or_else(|| array_expr.strip_suffix("[*]"))
            {
                // rubash#375: dynamic stack arrays join from the live view
                // (same empty-view mirror as the plain `[@]-` arm above).
                if let Some(values) = self.dynamic_stack_array_values(array_name) {
                    if values.is_empty() {
                        return Some(default.to_string());
                    }
                    let joined = self.dynamic_array_joined(array_expr).unwrap_or_default();
                    return Some(joined);
                }
                return Some(
                    self.parameter_array_storage(array_name)
                        .filter(|value| !value.is_empty())
                        .map(|value| self.join_array_parameter_values(&value, array_expr))
                        .unwrap_or_else(|| default.to_string()),
                );
            }
            return Some(
                self.shell_variable_value(array_expr)
                    .filter(|value| !value.is_empty() && !is_array_storage(value))
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| default.to_string()),
            );
        }
        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
        {
            if array_name == "GROUPS" {
                return Some(self.groups_words().join(" "));
            }
            // rubash#375: dynamic stack arrays join from the live view.
            if let Some(joined) = self.dynamic_array_joined(name) {
                return Some(joined);
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| self.join_array_parameter_values(&value, name))
                    .unwrap_or_default(),
            );
        }
        if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
            if array_name == "GROUPS" {
                return Some(self.group_value_at(index).unwrap_or_default());
            }
            // rubash#375: dynamic stack arrays read the element straight
            // from the live view.
            if let Some(values) = self.dynamic_stack_array_values(array_name) {
                return Some(dense_view_element(&values, index as i128).unwrap_or_default());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .and_then(|value| array_value_at(&value, index))
                    .map(normalize_array_expanded_value)
                    .unwrap_or_default(),
            );
        }
        if let Some((array_name, key)) = parse_array_subscript(name) {
            if self.is_assoc_parameter_array(array_name) {
                let key = self.assoc_subscript_key(key);
                return Some(
                    self.parameter_array_storage(array_name)
                        .and_then(|value| assoc_value_at(&value, &key))
                        .unwrap_or_default(),
                );
            }
            if let Some(value) = self.array_element_parameter_value(name) {
                return Some(normalize_array_expanded_value(value));
            }
        }
        None
    }
}

/// Split `name` on the first top-level occurrence of `op`, skipping `[...]`
/// array subscripts and `${...}` nested parameter expansions. GNU
/// `param_expand` (subst.c) only recognizes the `=`, `+`, `-`, `?` operators
/// at the top level of the braced parameter body; an `=` inside a subscript
/// like `${_ENV[(_=1)]}` is an arithmetic assignment, not a `${var=word}`
/// operator (new-exp.tests line 45).
pub(in crate::executor) fn split_once_outside_subscript<'a>(
    name: &'a str,
    op: char,
) -> Option<(&'a str, &'a str)> {
    let op_byte = op as u8;
    split_once_outside_subscript_impl(name, &[op_byte])
}

/// Split on a two-character operator (e.g. `:=`, `:-`, `:+`, `:?`) at the top
/// level, skipping `[...]` subscripts and `${...}` nested expansions.
pub(in crate::executor) fn split_once_outside_subscript_str<'a>(
    name: &'a str,
    op: &str,
) -> Option<(&'a str, &'a str)> {
    // Operators reaching here are 1-2 ASCII bytes (`:`-led pairs); a stack
    // buffer avoids the per-call Vec the hot parameter-operator chain paid
    // (8 splits per `${}` fragment). Longer ops (none today) fall back.
    let mut stack_buf = [0u8; 8];
    let op_bytes: &[u8] = if op.len() <= stack_buf.len() {
        stack_buf[..op.len()].copy_from_slice(op.as_bytes());
        &stack_buf[..op.len()]
    } else {
        return split_once_outside_subscript_alloc(name, op);
    };
    split_once_outside_subscript_impl(name, op_bytes)
}

fn split_once_outside_subscript_alloc<'a>(name: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let op_bytes: Vec<u8> = op.bytes().collect();
    split_once_outside_subscript_impl(name, &op_bytes)
}

/// One-pass operator-boundary index over a braced-parameter body, answering
/// the same "first top-level occurrence" questions the
/// `split_once_outside_subscript*` family answers, without re-running the
/// quote/bracket state machine per operator.
///
/// GNU anchor: subst.c:9777 `parameter_brace_expand` extracts the parameter
/// name in ONE `string_extract(string, &t_index, "#%^,:-=?+/@}", SX_VARNAME)`
/// pass and dispatches on the single character that ended the name
/// (subst.c:9886-9917: `:` + VALID_PARAM_EXPAND_CHAR -> null-test operator,
/// bare `-=?+` -> operators, `/` -> patsub, `^`,`,~` -> casemod). The Rust
/// port instead runs a sequence of `split_once_outside_subscript` probes,
/// each a full re-scan with the same state machine; this index restores the
/// GNU shape — one scan, many O(1) queries — while keeping each probe's
/// answer byte-identical to the split function it replaces (the state
/// machine below is copied verbatim from
/// `split_once_outside_subscript_impl`, and an operator byte at a top-level
/// position never alters the machine's state, so the byte following a
/// top-level operator byte is itself top-level — exactly the condition the
/// pair match in the impl checks).
pub(in crate::executor) struct TopLevelOpIndex {
    /// First top-level byte offset of each ASCII byte, or u32::MAX.
    first: [u32; 128],
    /// First top-level offset of the `:`-led pairs `:-`, `:=`, `:+`, `:?` —
    /// indexed [first_byte_is_colon? no: by second byte] as offsets into
    /// `PAIR_SECONDS` (b'-', b'=', b'+', b'?').
    first_pairs: [u32; 4],
}

const PAIR_SECONDS: [u8; 4] = [b'-', b'=', b'+', b'?'];
const NO_POS: u32 = u32::MAX;

impl TopLevelOpIndex {
    /// Build the index with one pass (the `split_once_outside_subscript_impl`
    /// state machine, recording instead of returning).
    pub(in crate::executor) fn new(name: &str) -> Self {
        let mut index = TopLevelOpIndex {
            first: [NO_POS; 128],
            first_pairs: [NO_POS; 4],
        };
        let bytes = name.as_bytes();
        let mut bracket_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut escaped = false;
        let mut single = false;
        let mut double = false;
        let mut offset = 0usize;
        while offset < bytes.len() {
            if escaped {
                escaped = false;
                offset += 1;
                continue;
            }
            let ch = bytes[offset];
            if ch == b'\\' && !single {
                escaped = true;
                offset += 1;
                continue;
            }
            if ch == b'\'' && !double {
                single = !single;
                offset += 1;
                continue;
            }
            if ch == b'"' && !single {
                double = !double;
                offset += 1;
                continue;
            }
            if single || double {
                offset += 1;
                continue;
            }
            if ch == b'`' {
                offset = skip_backtick_span(bytes, offset + 1);
                continue;
            }
            if ch == b'$' && bytes.get(offset + 1) == Some(&b'{') {
                brace_depth += 1;
                offset += 2;
                continue;
            }
            if ch == b'$' && bytes.get(offset + 1) == Some(&b'(') {
                offset = skip_parenthesized_span(bytes, offset + 2);
                continue;
            }
            if ch == b'}' && brace_depth > 0 {
                brace_depth -= 1;
                offset += 1;
                continue;
            }
            if brace_depth == 0 && ch == b'[' {
                bracket_depth += 1;
                offset += 1;
                continue;
            }
            if brace_depth == 0 && ch == b']' && bracket_depth > 0 {
                bracket_depth -= 1;
                offset += 1;
                continue;
            }
            if bracket_depth == 0 && brace_depth == 0 && ch < 128 {
                if index.first[ch as usize] == NO_POS {
                    index.first[ch as usize] = offset as u32;
                }
                if ch == b':' {
                    if let Some(next) = bytes.get(offset + 1).copied() {
                        if let Some(pair) = PAIR_SECONDS.iter().position(|sec| *sec == next) {
                            if index.first_pairs[pair] == NO_POS {
                                index.first_pairs[pair] = offset as u32;
                            }
                        }
                    }
                }
            }
            offset += 1;
        }
        index
    }

    /// Equivalent of `split_once_outside_subscript(name, op)` for a
    /// single-byte `op`.
    pub(in crate::executor) fn split_byte<'a>(
        &self,
        name: &'a str,
        op: u8,
    ) -> Option<(&'a str, &'a str)> {
        let first = self.first.get(op as usize).copied().unwrap_or(NO_POS);
        if first == NO_POS {
            return None;
        }
        let at = first as usize;
        Some((&name[..at], &name[at + 1..]))
    }

    /// Equivalent of `split_once_outside_subscript_str(name, ":<second>")`.
    pub(in crate::executor) fn split_colon_pair<'a>(
        &self,
        name: &'a str,
        second: u8,
    ) -> Option<(&'a str, &'a str)> {
        let pair = PAIR_SECONDS.iter().position(|sec| *sec == second)?;
        let first = self.first_pairs[pair];
        if first == NO_POS {
            return None;
        }
        let at = first as usize;
        Some((&name[..at], &name[at + 2..]))
    }
}

fn split_once_outside_subscript_impl<'a>(name: &'a str, op: &[u8]) -> Option<(&'a str, &'a str)> {
    let bytes = name.as_bytes();
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut escaped = false;
    // GNU skip_matched_pair (subst.c:2086): `'` and `"` spans are skipped as
    // units, so a `]` or `=` inside a quoted subscript (`a['x]=y']`) is data,
    // never a delimiter or an operator.
    let mut single = false;
    let mut double = false;
    let mut index = 0;
    while index < bytes.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        let ch = bytes[index];
        if ch == b'\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == b'\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == b'"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if single || double {
            index += 1;
            continue;
        }
        if ch == b'`' {
            index = skip_backtick_span(bytes, index + 1);
            continue;
        }
        if ch == b'$' && bytes.get(index + 1) == Some(&b'{') {
            brace_depth += 1;
            index += 2;
            continue;
        }
        // `$(...)` bodies are scanned as matched pairs by skip_matched_pair,
        // so a `]`, `=` or other operator character produced inside a command
        // substitution (`x[$(echo a]=b)]`) never ends the subscript or splits
        // the operator.
        if ch == b'$' && bytes.get(index + 1) == Some(&b'(') {
            index = skip_parenthesized_span(bytes, index + 2);
            continue;
        }
        if ch == b'}' && brace_depth > 0 {
            brace_depth -= 1;
            index += 1;
            continue;
        }
        if brace_depth == 0 && ch == b'[' {
            bracket_depth += 1;
            index += 1;
            continue;
        }
        if brace_depth == 0 && ch == b']' && bracket_depth > 0 {
            bracket_depth -= 1;
            index += 1;
            continue;
        }
        if bracket_depth == 0 && brace_depth == 0 && index + op.len() <= bytes.len() {
            if bytes[index..index + op.len()] == *op {
                return Some((&name[..index], &name[index + op.len()..]));
            }
        }
        index += 1;
    }
    None
}

/// Skip a backtick command substitution starting just after the opening
/// backquote (skip_matched_pair handling of `` `...` ``).
fn skip_backtick_span(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'`' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

/// Skip a `$(...)` body starting just after the `$(`, tracking nested parens
/// and quote spans (skip_matched_pair handling of `$(...)`).
fn skip_parenthesized_span(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1usize;
    let mut quote = 0u8;
    while index < bytes.len() {
        let ch = bytes[index];
        index += 1;
        if ch == b'\\' {
            index += 1;
            continue;
        }
        if quote != 0 {
            if ch == quote {
                quote = 0;
            }
            continue;
        }
        match ch {
            b'\'' | b'"' => quote = ch,
            b'`' => index = skip_backtick_span(bytes, index),
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}
