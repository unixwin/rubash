use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};
use crate::executor::parameter_core::word_contains_current_shell_command_substitution;

/// Recognize `=` / `:=` whose parameter name is a bare special parameter
/// (`!`, `@`, `*`). GNU subst.c parameter_brace_expand treats these like
/// positional parameters for the assignment operators (subst.c:10404 reports
/// "$%s: cannot assign in this way" for them); plain variable names and
/// numeric positionals are handled by parse_parameter_assignment_operator.
fn parse_special_assignment_operator(inner: &str) -> Option<(&str, bool)> {
    if let Some((name, _)) = inner.split_once(":=") {
        if matches!(name, "!" | "@" | "*") {
            return Some((name, true));
        }
    }
    if let Some((name, _)) = inner.split_once('=') {
        if matches!(name, "!" | "@" | "*") {
            return Some((name, false));
        }
    }
    None
}

/// Split a leading `name[subscript]` array-element reference out of a
/// `${...}` parameter body, returning `(base_name, subscript)`; text after
/// the closing `]` (an operator suffix) is ignored because GNU
/// parameter_brace_expand evaluates the subscript before the operator.
/// Nested subscripts are left to the real evaluator.
fn split_leading_array_ref(name: &str) -> Option<(&str, &str)> {
    let open = name.find('[')?;
    let base = &name[..open];
    if !is_shell_name(base) {
        return None;
    }
    let close = name[open..].find(']')? + open;
    let subscript = &name[open + 1..close];
    if subscript.contains('[') {
        return None;
    }
    Some((base, subscript))
}

/// Byte length of the parameter-reference portion of a `${...}` body,
/// mirroring GNU `subst.c:791 string_extract` with `SX_VARNAME` as called
/// from `parameter_brace_expand` (`subst.c:9807`, delimiter set
/// `#%^,:-=?+/@}`): the reference ends at the first unescaped operator
/// character, and a well-formed `[...]` subscript group is skipped as
/// part of the reference (`subst.c:812-818`, `skipsubscript` + the
/// `string[ni] == RBRACK` check). Brackets *after* the operator are
/// pattern/replacement text, never a subscript — `${line#[[:space:]]}`
/// references `line` (rubash#311), and `${a[5]#z}` references the
/// element `a[5]`, not an array named `a[5]#z`.
fn parameter_reference_len(body: &str) -> usize {
    const OPERATOR_SET: &[u8] = b"#%^,:-=?+/@}";
    let bytes = body.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch == b'\\' {
            if index + 1 < bytes.len() {
                index += 1;
            } else {
                break;
            }
        } else if ch == b'[' {
            // skipsubscript: nested brackets adjust depth, a backslash
            // protects the next character; an unterminated group leaves
            // the `[` as ordinary reference text.
            let mut depth = 1usize;
            let mut scan = index + 1;
            let mut close = None;
            while scan < bytes.len() {
                match bytes[scan] {
                    b'\\' => scan += 1,
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(scan);
                            break;
                        }
                    }
                    _ => {}
                }
                scan += 1;
            }
            if let Some(closed) = close {
                index = closed;
            }
        } else if OPERATOR_SET.contains(&ch) {
            break;
        }
        index += 1;
    }
    index
}

/// Byte index of the next `${` in `text` that starts a parameter expansion
/// of the word itself, skipping spans whose contents belong to another
/// expansion layer (GNU subst.c: string_extract_double_quoted copies a
/// backtick body verbatim; extract_command_subst and
/// extract_dollar_brace_string own their spans):
///   * single-quoted spans and ANSI-C $'...' strings (data)
///   * backtick bodies and $(...) bodies (separate command sources)
///   * the lexer's legacy C0 data markers (0x11 glob, 0x14 backslash,
///     0x17 single quote, 0x18 double quote, 0x1a backtick, 0x1f dollar)
/// Inside double quotes `${` stays a real expansion start and a single
/// quote is literal data (parse.y skip_double_quoted), matching GNU
/// word expansion; heredoc bodies keep the raw scan instead.
fn next_quoted_parameter_expansion_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut in_double = false;
    while index < bytes.len() {
        let ch = bytes[index];
        match ch {
            b'\\' => {
                // Escaped pair: \$ / \` / \} never opens a span here.
                index += 2;
            }
            0x11 => {
                // Protected glob marker plus the guarded character.
                index += 2;
            }
            0x13 | 0x14 | 0x17 | 0x18 | 0x1a | 0x1f => {
                // Legacy data markers: literal data, never delimiters.
                index += 1;
            }
            b'\'' if !in_double => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'\'' {
                    index += 1;
                }
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'\'') && !in_double => {
                // ANSI-C string: backslash escapes carry through the span.
                index += 2;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index += 2;
                        continue;
                    }
                    if bytes[index] == b'\'' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => return Some(index),
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                // $(...) body: a separate command source with its own scan.
                index += 2;
                let mut depth = 1usize;
                let mut single = false;
                let mut double = false;
                while index < bytes.len() {
                    let current = bytes[index];
                    if current == b'\\' && !single {
                        index += 2;
                        continue;
                    }
                    match current {
                        b'\'' if !double => single = !single,
                        b'"' if !single => double = !double,
                        b'(' if !single && !double => depth += 1,
                        b')' if !single && !double => {
                            depth -= 1;
                            if depth == 0 {
                                index += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    index += 1;
                }
            }
            b'`' => {
                // Backtick body: the dequoter copies it verbatim, so its
                // `${` belongs to the inner command (quote.tests:117).
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index += 2;
                        continue;
                    }
                    if bytes[index] == b'`' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
            }
            b'"' => {
                in_double = !in_double;
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }
    None
}
/// Bodies of the `$((...))` arithmetic expansions in `word` that a
/// pure probe may evaluate: parens are balance-matched the way
/// parse.y:3877 parse_matched_pair matches an arithmetic span, and
/// bodies containing a command substitution are excluded (evaluating
/// them would run the substitution — a side effect a probe must not
/// have; subst.c:7143 command_substitute runs as the walk reaches it).
fn speculative_arithmetic_bodies(word: &str) -> Vec<String> {
    let bytes = word.as_bytes();
    let mut bodies = Vec::new();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'(' && bytes[i + 2] == b'(' {
            let mut depth = 0i32;
            let mut j = i + 1;
            let mut end = None;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let Some(end) = end else { break };
            if end >= i + 4 {
                let body = &word[i + 3..end - 1];
                if !body.contains("$(") && !body.contains('`') {
                    bodies.push(body.to_string());
                }
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    bodies
}

/// True when `word` (raw source text when available) carries a glob
/// metacharacter outside any quoting — the failglob risk shape. A
/// quoted metachar is a false hit, which only costs the concurrent
/// fast path its admission.
pub(in crate::executor) fn word_has_unquoted_glob_meta(word: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in word.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '*' | '?' | '[' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    false
}

/// Blank out (position-preserving) the spans of `word` whose `${...}`
/// fragments are NOT expansions of the current shell: `$( )` command
/// substitutions and backtick substitutions (their fragments belong to
/// the substitution child, subst.c:7143 command_substitute) and
/// single-quoted regions (no expansion at all). `$(( ))` arithmetic is
/// NOT masked — its fragments expand in the current shell. Used by the
/// parameter-assignment pre-scan (rubash#1542).
fn mask_nonlocal_expansion_spans(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let mut out: Vec<char> = chars.clone();
    let mut index = 0usize;
    let mut in_double = false;
    while index < chars.len() {
        match chars[index] {
            '\\' => {
                index += 2;
                continue;
            }
            '"' => {
                in_double = !in_double;
                index += 1;
            }
            '\'' if !in_double => {
                // Single-quoted region: blank through the closing quote.
                let start = index;
                index += 1;
                while index < chars.len() && chars[index] != '\'' {
                    index += 1;
                }
                let end = (index + 1).min(chars.len());
                for slot in &mut out[start..end] {
                    *slot = ' ';
                }
                index = end;
            }
            '`' => {
                let start = index;
                index += 1;
                while index < chars.len() && chars[index] != '`' {
                    if chars[index] == '\\' {
                        index += 1;
                    }
                    index += 1;
                }
                let end = (index + 1).min(chars.len());
                for slot in &mut out[start..end] {
                    *slot = ' ';
                }
                index = end;
            }
            '$' if chars.get(index + 1) == Some(&'(') && chars.get(index + 2) != Some(&'(') => {
                if let Some(end) =
                    crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                {
                    for slot in &mut out[index..end] {
                        *slot = ' ';
                    }
                    index = end;
                } else {
                    index += 1;
                }
            }
            _ => {
                index += 1;
            }
        }
    }
    out.into_iter().collect()
}

impl Executor {
    pub(in crate::executor) fn parameter_assignment_error(
        &self,
        cmd: &CommandNode,
    ) -> Option<(String, &'static str)> {
        for word in &cmd.words {
            if let Some(error) = self.parameter_assignment_error_in_word(word) {
                return Some(error);
            }
        }
        for value in cmd.assignment_values() {
            if let Some(error) = self.parameter_assignment_error_in_word(value) {
                return Some(error);
            }
        }
        None
    }

    pub(in crate::executor) fn parameter_assignment_error_in_word(
        &self,
        word: &str,
    ) -> Option<(String, &'static str)> {
        // perf6 admission: every error arm below requires an ASCII `$` in
        // the word — `${` spans (assignment/operator/transform/length/
        // indirect/substring arms) or a `$`-introduced parameter reference
        // (nounset arm; the DATA_DOLLAR literal-$ marker is skipped there
        // and never forms `${`). A `$`-free word therefore returns None;
        // one memchr beats the per-byte quote state machine. GNU has no
        // such pre-scan at all (subst.c expands once, reporting inline),
        // so this keeps the pre-scan off words that cannot use it without
        // touching any arm's semantics.
        if !word.as_bytes().contains(&b'$') {
            return None;
        }
        // rubash#1597: the bad-array-subscript class is adjudicated by
        // the shared helper below (also consulted by the word scan
        // behind for-list / case / [[ ]] positions); the
        // readonly/positional arms keep their own loop further down.
        if let Some(name) = self.bad_array_subscript_assignment_in_word(word) {
            return Some((name, "bad array subscript"));
        }
        let word = word
            .strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX)
            .or_else(|| word.strip_prefix(STORAGE_WORD_PREFIX))
            .unwrap_or(word);
        // rubash#1542: fragments inside a command-substitution span
        // belong to the substitution's child (subst.c:7143
        // command_substitute) — the child's own commands are scanned
        // when they run. This textual pre-scan of the ENCLOSING word
        // must not claim them: `x=$(echo ${v:=x})` reported the child's
        // failure against the parent command and discarded it at the
        // top level. Mask substitution/backtick/single-quoted spans
        // before scanning (positions preserved).
        let masked;
        let word = if word.contains("$(") || word.contains('`') || word.contains('\'') {
            masked = mask_nonlocal_expansion_spans(word);
            &masked
        } else {
            word
        };
        let mut rest = word;
        while let Some(start) = rest.find("${") {
            let after_start = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(after_start) else {
                return None;
            };
            let inner = &after_start[..end];
            if let Some((name, require_non_empty)) = parse_parameter_assignment_operator(inner) {
                // GNU only reports when the assignment is actually
                // attempted against an unassignable target — check the
                // (subscript-free) target first so `${a[$((i++))]:=x}` on
                // a normal array does not evaluate the subscript here at
                // all; param_expand's single array_expand_index does it
                // once during the real expansion.
                let is_positional = name.parse::<usize>().is_ok_and(|index| index > 0);
                let target = parse_array_subscript(name)
                    .map(|(array_name, _)| array_name.to_string())
                    .unwrap_or_else(|| {
                        self.nameref_target_name(name)
                            .unwrap_or_else(|| name.to_string())
                    });
                let readonly = is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &target);
                if (is_positional || readonly)
                    && self.parameter_assignment_required(name, require_non_empty)
                {
                    if is_positional {
                        return Some((format!("${name}"), "cannot assign in this way"));
                    }
                    return Some((target, "readonly variable"));
                }
            } else if let Some((name, require_non_empty)) = parse_special_assignment_operator(inner)
            {
                // GNU subst.c:10404-10410: `=` / `:=` on a special parameter
                // (`!`, `@`, `*`) with the parameter unset (or null for `:=`)
                // reports "$X: cannot assign in this way" and returns
                // &expand_wdesc_error (non-fatal DISCARD, subst.c:4296).
                // #, $, ?, -, 0 are always set, so only !/@/* reach the error
                // path. parse_parameter_assignment_operator rejects these
                // names, and without this branch `${!=x}` fell into the
                // indirect-expansion scanner and misreported "bad
                // substitution" while `${@=x}` expanded silently empty.
                if self.special_assignment_required(name, require_non_empty) {
                    return Some((format!("${name}"), "cannot assign in this way"));
                }
            }
            rest = &after_start[end + 1..];
        }
        None
    }

    /// rubash#1597: the name (as written) of a triggered `=` / `:=`
    /// whose left-hand side is the all-elements spelling (`name[@]` /
    /// `name[*]`) on a base that is NOT an associative array. GNU
    /// parameter_brace_expand's `case '='` (subst.c:10346) hands the
    /// RHS to parameter_brace_expand_rhs (subst.c:8096) with the
    /// subscript text still attached, and the bind fails with
    /// `name[@]: bad array subscript` (EX_BADUSAGE). The one legal
    /// base is an associative array, where the literal "@" / "*" text
    /// is just a key. The trigger test is the operator's own set/null
    /// test over the joined elements (parameter_assignment_required),
    /// so a set array whose elements are non-null never reaches the
    /// bind. Shared by the simple-command pre-scan and the word scan
    /// behind for-list / case / [[ ]] positions
    /// (parameter_expansion_error_in_word_context).
    pub(in crate::executor) fn bad_array_subscript_assignment_in_word(
        &self,
        word: &str,
    ) -> Option<String> {
        if !word.as_bytes().contains(&b'$') || !word.contains('=') {
            return None;
        }
        let word = word
            .strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX)
            .or_else(|| word.strip_prefix(STORAGE_WORD_PREFIX))
            .unwrap_or(word);
        // Same #1542 masking discipline as the sibling scan: fragments
        // inside a command-substitution span belong to the child.
        let masked;
        let word = if word.contains("$(") || word.contains('`') || word.contains('\'') {
            masked = mask_nonlocal_expansion_spans(word);
            &masked
        } else {
            word
        };
        let mut rest = word;
        while let Some(start) = rest.find("${") {
            let after_start = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(after_start) else {
                return None;
            };
            let inner = &after_start[..end];
            if let Some((name, require_non_empty)) = parse_parameter_assignment_operator(inner) {
                if let Some(base) = name
                    .strip_suffix("[@]")
                    .or_else(|| name.strip_suffix("[*]"))
                {
                    let resolved_base = self
                        .nameref_target_name(base)
                        .unwrap_or_else(|| base.to_string());
                    let base_is_assoc =
                        is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved_base);
                    if !base_is_assoc && self.parameter_assignment_required(name, require_non_empty)
                    {
                        return Some(name.to_string());
                    }
                }
            }
            rest = &after_start[end + 1..];
        }
        None
    }

    pub(in crate::executor) fn parameter_assignment_required(
        &self,
        name: &str,
        require_non_empty: bool,
    ) -> bool {
        match self.parameter_operator_value(name) {
            Some(value) => require_non_empty && value.is_empty(),
            None => true,
        }
    }

    /// GNU fatality condition for `=` / `:=` on a special parameter
    /// (subst.c:10404 path): the error fires only when the parameter is
    /// UNSET (`=`) or unset-or-null (`:=`). `!` is unset until the first
    /// background job; `@` / `*` are unset with zero positional parameters.
    fn special_assignment_required(&self, name: &str, require_non_empty: bool) -> bool {
        let is_set = match name {
            "!" => self.shell_state.last_background_pid.is_some(),
            "@" | "*" => !self.shell_state.positional_window().is_empty(),
            _ => true,
        };
        if require_non_empty {
            !is_set
                || self
                    .parameter_error_value(name)
                    .is_none_or(|value| value.is_empty())
        } else {
            !is_set
        }
    }

    pub(in crate::executor) fn parameter_operator_value(&self, name: &str) -> Option<String> {
        if let Some(value) = self.indirect_parameter_operator_value(name) {
            return value;
        }
        // `${arr[*]:-word}` / `${arr[@]:-word}`: operator tests and values
        // apply to the whole array joined with spaces (Bash semantics).
        if let Some(array_name) = name
            .strip_suffix("[*]")
            .or_else(|| name.strip_suffix("[@]"))
        {
            // rubash#375: the six dynamic stack arrays answer set-ness and
            // the joined value from the live view — the storage render
            // existed only to be parsed back for is_empty()/join (GNU
            // tests array elements on the maintained ARRAY object,
            // variables.c INIT_DYNAMIC_VAR). bats' per-firing
            // `${BASH_LINENO[@]+"${BASH_LINENO[@]}"}` paid the round trip
            // twice per DEBUG trap. Same contract as the storage path
            // below: space join for BOTH `[@]` and `[*]`, elements
            // normalized like array_values does.
            if let Some(values) = self.dynamic_stack_array_values(array_name) {
                let values = values
                    .into_iter()
                    .map(normalize_array_expanded_value)
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    return None;
                }
                return Some(values.join(" "));
            }
            let values = self
                .parameter_array_storage(array_name)
                .map(|storage| {
                    if self.parameter_is_array_variable(array_name) {
                        array_values(&storage)
                    } else {
                        // Scalar base (`x[@]`/`x[*]` on VT_VARIABLE): one
                        // element, the whole string — never re-parsed as
                        // list storage (rubash#755).
                        vec![shell_safe_value(&storage)]
                    }
                })
                .unwrap_or_default();
            if values.is_empty() {
                return None;
            }
            return Some(values.join(" "));
        }
        if is_shell_name(name) {
            return self
                .dynamic_parameter_value(name)
                .or_else(|| self.shell_variable_value(name));
        }
        if let Some(value) = self.array_element_parameter_value(name) {
            return Some(value);
        }
        self.parameter_error_value(&name)
    }

    /// GNU subst.c:9917 want_indir + subst.c:122 VALID_INDIR_PARAM: a `!`
    /// prefix introduces indirect expansion only when the next character is
    /// a valid indirect parameter; `#` and `?` qualify only outside posix
    /// mode, so `${!?}` under `sh` is the `!` parameter under the `?`
    /// operator (posixexp2.sub test 6), not indirection through `$?`.
    pub(in crate::executor) fn indirect_parameter_body<'a>(
        &self,
        name: &'a str,
    ) -> Option<&'a str> {
        let body = name.strip_prefix('!')?;
        if self.posix_mode_enabled() && matches!(body.chars().next(), Some('#' | '?')) {
            return None;
        }
        Some(body)
    }

    /// GNU subst.c:7839-7885 parameter_brace_find_indir resolves the
    /// indirect SOURCE in scalar form: t is the source variable's value
    /// with an array name yielding element 0 (assoc: key "0") and an
    /// array with no such element yielding NULL. Every validity and
    /// value decision for `${!...}` uses THIS text, never the raw
    /// storage encoding of the variable (whose element separators are
    /// in-band markers) — rubash#1147 validated/joined the storage text
    /// instead, inverting `${!assoc[@]:-d}` outcomes and leaking marker
    /// bytes into the `invalid variable name` diagnostic.
    pub(in crate::executor) fn indirect_source_scalar_value(&self, name: &str) -> Option<String> {
        if let Some(value) = self.shell_state.env_vars.get(name) {
            return self.scalar_parameter_value(name, value);
        }
        // The PIPESTATUS cell lives in shell_state in its array form —
        // env_vars carries it only for a user-assigned scalar shape —
        // so the scalar-form value (array element 0, what
        // parameter_brace_find_indir reads for an array source) comes
        // from the state cell (rubash#1566).
        if name == "PIPESTATUS" {
            return self.pipestatus_element(0);
        }
        None
    }

    /// Whether `name` names the materialized PIPESTATUS cell. GNU
    /// set_pipestatus_array (variables.c:6291) writes the PIPESTATUS
    /// ARRAY object directly, so find_variable finds the variable
    /// whenever the cell exists — every completed command materializes
    /// it — even though rubash keeps that form in shell_state rather
    /// than env_vars. An Absent shape (no command has completed yet,
    /// or an `unset` the next completion has not yet undone) is
    /// genuinely unset, matching GNU's lazy creation (rubash#1566).
    pub(in crate::executor) fn pipestatus_cell_exists(&self, name: &str) -> bool {
        name == "PIPESTATUS"
            && self.shell_state.pipestatus_shape != crate::shell::state::PipestatusShape::Absent
    }

    fn indirect_parameter_operator_value(&self, name: &str) -> Option<Option<String>> {
        let indirect_name = self.indirect_parameter_body(name)?;
        if let Some(target_name) = self.nameref_target_name(indirect_name) {
            return Some(Some(target_name));
        }
        // GNU param_expand (subst.c:10058-10068) resolves an indirect
        // POSITIONAL reference to the parameter NAME held by that positional,
        // then applies the operator to *that* parameter's set-state and
        // value. With $1=a and a="" (set but null), ${!1-$z} must yield ""
        // (`-` does not substitute for a set parameter) while ${!1:-$z}
        // substitutes $z; with $9 unset the operator sees an unset
        // parameter either way. The previous code fell through to
        // parameter_error_value, which returned $1's raw value ("a") as the
        // operator result.
        if let Ok(index) = indirect_name.parse::<usize>() {
            return match self
                .shell_state
                .positional_window()
                .get(index.saturating_sub(1))
            {
                Some(target_name) => Some(self.parameter_operator_value(target_name)),
                None => Some(None),
            };
        }
        // GNU subst.c:7924-7945 parameter_brace_expand_indir: a
        // subscripted source (`${!name[sub]<op>...}`) resolves t through
        // the subscripted reference itself — the element value, or the
        // all-elements value for `[@]`/`[*]` — then the operator applies
        // to the parameter t names (rubash#1147).
        if parse_array_subscript(indirect_name).is_some() {
            return match self.parameter_operator_value(indirect_name) {
                Some(t) => Some(self.indirect_target_values(&t).into_iter().next()),
                None => Some(None),
            };
        }
        let Some(raw_value) = self.shell_state.env_vars.get(indirect_name) else {
            // No value entry: a declared-unset source still yields an
            // (unset) t, so the operator adjudicates an unset parameter;
            // a wholly absent source is not this form.
            if is_marked_var(
                &self.shell_state.env_vars,
                DECLARED_UNSET_VARS,
                indirect_name,
            ) {
                return Some(None);
            }
            // The PIPESTATUS state cell is a materialized source too
            // (rubash#1566): adjudicate the operator through its
            // scalar-form value, exactly like the env-backed path
            // below the gate.
            if self.pipestatus_cell_exists(indirect_name) {
                return match self.indirect_source_scalar_value(indirect_name) {
                    Some(t) => Some(self.indirect_target_values(&t).into_iter().next()),
                    None => Some(None),
                };
            }
            return None;
        };
        if raw_value.ends_with("[@]") || raw_value.ends_with("[*]") {
            let values = self.indirect_target_values(raw_value);
            if values.is_empty() {
                return Some(None);
            }
            return Some(Some(self.join_expanded_array_values(values, raw_value)));
        }

        match self.indirect_source_scalar_value(indirect_name) {
            Some(t) => Some(self.indirect_target_values(&t).into_iter().next()),
            None => Some(None),
        }
    }

    fn is_valid_length_parameter_name(name: &str) -> bool {
        if name.is_empty() {
            return true;
        }
        // rubash#1287: `${#$'...'}` — a complete ANSI-C quoted literal is
        // a valid length operand (GNU subst.c parameter_brace_expand
        // accepts it and expands the length of the unset parameter, 0).
        // The k5 fix admitted the form in the expansion gate and the
        // length dispatcher but missed this pre-scan, so the shell still
        // rejected it end to end; admitted here at integration.
        let name_bytes = name.as_bytes();
        if crate::executor::expand_word::ansi_c_literal_end(name_bytes)
            == Some(name_bytes.len().saturating_sub(1))
            && name_bytes.len() >= 3
        {
            return true;
        }
        // `${##}` is the length of `$#` (exp.tests:376 -> 1 with $#=5) and
        // `${#!}` the length of `$!` (more-exp.tests -> 0 while unset), so
        // both special names are valid length parameters in GNU.
        if matches!(name, "@" | "*" | "?" | "$" | "-" | "0" | "#" | "!") {
            return true;
        }
        if name.parse::<usize>().is_ok() {
            return true;
        }
        if name.ends_with("[@]") || name.ends_with("[*]") {
            let base = &name[..name.len() - 3];
            return !base.is_empty() && is_shell_name(base);
        }
        // GNU subst.c valid_length_expression: `${#arr[index]}` is valid —
        // it returns the length of element `index` of array `arr`.  Both
        // integer-indexed (`${#a[5]}`) and associative (`${#aa[key]}`) forms
        // are accepted.  array.tests:89 `${#a[5]}` -> 11 ("hello world").
        if let Some((array_name, _key)) = parse_array_subscript(name) {
            if is_shell_name(array_name) {
                return true;
            }
        }
        // GNU subst.c valid_length_expression: after `#`, a leading name
        // character continues as a name; ANY other first character means the
        // `#` itself is the special parameter `$#` and the remainder must be
        // a parameter operator expression with a non-empty word. GNU probe
        // 2026-09-02 (WSL bash 5.2.21): `${#-posparams}` and `${#?:-xyz}`
        // are VALID (`0`), `${#:x}`/`${#:foo}` are valid, while the
        // empty-word operator forms `${#:}`, `${#/}`, `${#%}`, `${#=}`,
        // `${#+}` and the non-name suffixes `${#1xyz}`, `${#x@}`,
        // `${#x:y}` are bad substitution.
        is_shell_name(name) || Self::is_length_operator_expression(name)
    }

    /// Valid indirect target values: shell names, all-digit positionals,
    /// the special parameters, and valid array references. Anything else
    /// triggers GNU's "invalid variable name" when indirected through
    /// (probe: x=123bad).
    fn is_valid_indirect_target(value: &str) -> bool {
        if value.is_empty() {
            return false;
        }
        if is_shell_name(value) || value.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
        if matches!(value, "@" | "*" | "#" | "?" | "$" | "-" | "!") {
            return true;
        }
        // GNU valid_brace_expansion_word (subst.c:7590) accepts a value
        // that is a valid array reference: ${!ref} with ref="a[@]"
        // performs array indirection (new-exp9.sub) instead of reporting
        // "invalid variable name".
        Self::is_valid_indirect_array_reference(value)
    }

    /// GNU valid_array_reference (arrayfunc.c): a `{name[...]}` form
    /// whose base is a shell identifier and whose bracketed subscript is
    /// non-empty and simple (no nested brackets).
    pub(in crate::executor) fn is_valid_indirect_array_reference(value: &str) -> bool {
        let Some(open) = value.find('[') else {
            return false;
        };
        if !value.ends_with(']') {
            return false;
        }
        let base = &value[..open];
        let subscript = &value[open + 1..value.len() - 1];
        !subscript.is_empty()
            && !subscript.contains('[')
            && !subscript.contains(']')
            && is_shell_name(base)
    }

    /// Validates the body of a `${!...}` expansion: a simple parameter
    /// reference (name, numeric positional, or special), optionally with a
    /// trailing array subscript (`${!arr[@]}` keys form) or a `${!prefix@}`
    /// variable-name listing suffix. `${!}` itself is the $! parameter.
    fn is_valid_indirect_expression(expr: &str) -> bool {
        if expr.is_empty() {
            return true;
        }
        // GNU param_expand (subst.c): a `!` whose next character is an
        // operator (`-` `=` `+` `:`) is the `$!` parameter WITH that
        // operator (`${!-ok 27}` substitutes `ok 27` while `$!` is unset;
        // `${!:-posparams}`), not an indirect expansion through a variable
        // named by the body. The body is valid and the operator family
        // expands it; only `?` `$` `#` etc. stay indirect (special params).
        if matches!(expr.chars().next(), Some('-' | '=' | '+' | ':')) {
            return true;
        }
        let base = match expr.rfind('[') {
            Some(start) if expr.ends_with(']') => &expr[..start],
            _ => expr,
        };
        if base.is_empty() {
            return false;
        }
        if let Some(prefix) = base.strip_suffix(['@', '*']) {
            return !prefix.is_empty() && is_shell_name(prefix);
        }
        if matches!(base, "@" | "*" | "#" | "?" | "$" | "-" | "!" | "0")
            || base.parse::<usize>().is_ok()
            || is_shell_name(base)
        {
            return true;
        }
        // GNU also accepts a valid leading name followed by an operator
        // expression applied to the indirect value (${!x//c/x}, ${!x:-y},
        // ${!x#pat}, ...; subst.c param_expand). A name followed by a
        // non-operator character (${!bad!}) stays a bad substitution.
        // A nested `${...}` parameter inside the indirect name (eval
        // re-expansion: ${!${1}[@]}) expands first in GNU, so the tail
        // after the balanced group decides validity.
        if expr.starts_with("${") {
            if let Some(end) = matching_parameter_brace(&expr[2..]) {
                let tail = &expr[end + 3..];
                if tail.is_empty() {
                    return true;
                }
                if tail.starts_with('[') {
                    return match Self::skip_indirect_array_subscript(tail) {
                        Some(after) => tail[after..].is_empty(),
                        None => false,
                    };
                }
                return matches!(
                    tail.chars().next(),
                    Some(':' | '-' | '+' | '=' | '?' | '#' | '%' | '/' | '^' | ',' | '@')
                );
            }
            return false;
        }
        let name_end = base
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(base.len());
        if name_end > 0 {
            let head = &base[..name_end];
            // GNU param_expand also applies operators to an indirect
            // POSITIONAL reference: ${!9:-$z} / ${!1-$z} look up $9 / $1 and
            // then apply the default operator to the indirect result
            // (probe: ${!9:-$z} with $9 unset expands to $z's value). The
            // previous is_shell_name-only check rejected these as bad
            // substitution.
            if head.bytes().all(|b| b.is_ascii_digit()) {
                let rest = &base[name_end..];
                return matches!(
                    rest.chars().next(),
                    Some(':' | '-' | '+' | '=' | '?' | '#' | '%' | '/' | '^' | ',' | '@')
                );
            }
            if is_shell_name(head) {
                let mut rest = &base[name_end..];
                // GNU string_extract with SX_VARNAME (subst.c:812-819)
                // skips `[...]` subscripts while scanning the parameter
                // name, so the name ends at the first operator character
                // AFTER a balanced subscript: ${!varname[@]@Q} and
                // ${!varname[@]%b} name the array reference varname[@]
                // with a transform / pattern operator applied to the
                // indirect result. A bracket that never closes stays a
                // bad substitution.
                if rest.starts_with('[') {
                    match Self::skip_indirect_array_subscript(rest) {
                        Some(end) => rest = &rest[end..],
                        None => return false,
                    }
                }
                return matches!(
                    rest.chars().next(),
                    Some(':' | '-' | '+' | '=' | '?' | '#' | '%' | '/' | '^' | ',' | '@')
                );
            }
        }
        false
    }

    /// True for a `${!name[sub]<op>...}` body: a subscript followed by an
    /// operator tail means GNU re-expands the base variable's value as the
    /// indirect target (indirection), unlike the bare `${!name[sub]}` keys
    /// form.
    fn is_indirect_array_operator_expression(indirect: &str) -> bool {
        let Some(open) = indirect.find('[') else {
            return false;
        };
        if !is_shell_name(&indirect[..open]) {
            return false;
        }
        let rest = &indirect[open..];
        let Some(end) = Self::skip_indirect_array_subscript(rest) else {
            return false;
        };
        matches!(
            rest[end..].chars().next(),
            Some(':' | '-' | '+' | '=' | '?' | '#' | '%' | '/' | '^' | ',' | '~' | '@' | '*')
        )
    }

    /// Offsets just past the `]` closing the `[...]` subscript at the head
    /// of `expr`, mirroring GNU skipsubscript bracket nesting (subst.c).
    fn skip_indirect_array_subscript(expr: &str) -> Option<usize> {
        let mut depth = 0usize;
        for (index, ch) in expr.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }
    /// True when the `${...}` body is a malformed subscripted reference:
    /// an empty subscript (`x[]`) or a subscript immediately followed by
    /// another (`a[0][1]`). Only bodies whose pre-bracket head is a shell
    /// name (or the all-digit positional spelling, which GNU
    /// valid_array_reference rejects as a base) participate; other `[`
    /// occurrences belong to operator tails and patterns and keep their own
    /// rules. See the call site for the GNU anchor.
    fn malformed_braced_reference(inner: &str) -> bool {
        let body = inner.strip_prefix('!').unwrap_or(inner);
        let body = body.strip_prefix('#').unwrap_or(body);
        let Some(open) = body.find('[') else {
            return false;
        };
        let head = &body[..open];
        if head.is_empty() {
            return false;
        }
        let head_is_name = is_shell_name(head);
        let head_is_digits = head.bytes().all(|b| b.is_ascii_digit());
        if !head_is_name && !head_is_digits {
            return false;
        }
        // Find the bracket matching the opening one (subscripts nest via
        // `$((x[0]))`-style arithmetic), then inspect the shape.
        let bytes = body.as_bytes();
        let mut depth = 0usize;
        let mut close = None;
        for (index, &byte) in bytes.iter().enumerate().skip(open) {
            if byte == b'[' {
                depth += 1;
            } else if byte == b']' {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
        }
        let Some(close) = close else {
            // An UNCLOSED subscript bracket (`${x[}`): the body parses
            // (the `${`...`}` pair is what the word level matches,
            // parse.y:3877) and GNU defers the malformed reference to the
            // expansion — `${x[}: bad substitution`, status 1, the command
            // dies and the script continues (rubash#532; WSL 5.3.0 probe:
            // `echo BEFORE; echo ${x[}; echo AFTER` runs BEFORE only, rc 1).
            return true;
        };
        if body[open + 1..close].is_empty() {
            return true;
        }
        body[close + 1..].starts_with('[')
    }

    /// Validates the remainder of a `${#...}` expansion when it does not
    /// start with a shell name: `#` is then the `$#` parameter itself and
    /// the text must be an operator applied to it. Operators with an empty
    /// word (`${#+}`, `${#:}`, `${#/}`) are bad substitution in GNU.
    fn is_length_operator_expression(name: &str) -> bool {
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if first == ':' {
            // GNU 5.3 parameter_brace_expand: a `:`-led operator accepts an
            // empty word — `${#:-}`/`${#:+}`/`${#:=}`/`${#:?}` are `$#` under
            // the colon operator (more-exp `recho ${#:-}` -> `0`), unlike
            // the colon-less `${#=}`/`${#+}` which stay bad substitution.
            return !name[1..].is_empty();
        }
        matches!(
            first,
            '-' | '+' | '=' | '?' | '@' | '^' | ',' | '/' | '%' | '#'
        ) && name.len() > 1
    }

    /// GNU subst.c:8949 parameter_brace_transform returns
    /// &expand_param_fatal for an invalid `@xform` operator; subst.c:4296
    /// maps that to FORCE_EOF, so a noninteractive shell exits with status
    /// 1 instead of merely abandoning the command list like
    /// &expand_param_error (DISCARD) does. The scan reports this as a
    /// sentinel status that command_prepare translates to ExitCode(1).
    pub(in crate::executor) const FATAL_PARAMETER_EXPANSION_STATUS: i32 = i32::MIN;

    /// rubash#1915: mask a command's recorded process-substitution
    /// spans out of a text the parent-side expansion-error scans are
    /// about to judge. GNU never expands a procsub body in the parent:
    /// the word walk extracts it verbatim (subst.c:1311
    /// extract_process_subst) and process_substitute (subst.c:6362)
    /// hands it to the child shell, whose own parse owns every `$`
    /// inside; the parent only ever sees the resulting /dev/fd word
    /// (subst.c:11366-11385 expand_word_internal). The parser records
    /// each real span with its verbatim target text (on the command
    /// node, and on a redirect target's word metadata), so replacing
    /// exactly those recorded spans — in recorded
    /// order, one occurrence each — removes precisely the child text
    /// and nothing else: look-alike text that is NOT a recorded span
    /// (e.g. a `<(...)`-shaped string inside double quotes, where the
    /// `$` stays a live parent reference) remains fully scanned.
    pub(in crate::executor) fn mask_recorded_process_substitutions<'a>(
        text: &'a str,
        spans: &[crate::parser::ProcessSubstitution],
    ) -> std::borrow::Cow<'a, str> {
        Self::mask_recorded_span_targets(
            text,
            spans
                .iter()
                .map(|substitution| substitution.target.as_str()),
        )
    }

    /// The masking core behind `mask_recorded_process_substitutions`,
    /// over bare span target texts: replace each recorded span — in
    /// recorded order, one occurrence each — with a single space, so
    /// the text a scan judges no longer contains the child body. The
    /// `[[ ]]` nounset scans hold only the recorded targets (paired
    /// with their word in `Executor::conditional_procsub_span_targets`)
    /// and enter here directly.
    pub(in crate::executor) fn mask_recorded_span_targets<'a, 't>(
        text: &'a str,
        targets: impl Iterator<Item = &'t str>,
    ) -> std::borrow::Cow<'a, str> {
        let mut masked: Option<String> = None;
        for target in targets {
            if target.is_empty() {
                continue;
            }
            let current: &str = masked.as_deref().unwrap_or(text);
            if current.contains(target) {
                masked = Some(current.replacen(target, " ", 1));
            }
        }
        masked.map_or(std::borrow::Cow::Borrowed(text), std::borrow::Cow::Owned)
    }

    pub(in crate::executor) fn parameter_expansion_error(
        &self,
        cmd: &CommandNode,
    ) -> Option<(String, String, i32)> {
        // GNU execute_arith_command owns the whole expansion verdict for an
        // arithmetic command: expand_arith_string (param_expand) reports
        // `<body>: bad substitution` for an unterminated `${` in the body
        // (rubash#533) and expr.c raises nounset itself, so the generic
        // word pre-scan — which models read_token_word's `${` pairing —
        // must not re-judge arithmetic body text where a `$`-`{` is
        // literal (parse.y parse_matched_pair P_ARITH).
        if cmd.arithmetic_command.is_some() {
            return None;
        }
        // rubash#319: GNU expands a command's words left-to-right
        // (subst.c:11229 expand_word_internal drives each word through
        // expand_word_list_internal order), and inside a word every
        // command substitution is forked as the walk reaches it
        // (subst.c:7143 command_substitute). By the time a later unbound
        // parameter raises the nounset fatal, every comsub to its left —
        // across the whole word list — already produced its side effects
        // (child stderr, files, ...): `w=$(echo hi >&2)${nope}` under
        // set -u prints the child's `hi` BEFORE `nope: unbound variable`.
        // The pre-scan cannot interleave, but it honors the order: walk
        // words and assignments left-to-right collecting comsub bodies,
        // and when the unbound hit comes, execute the collected prefix
        // bodies before reporting the fatal.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            let mut prefix_comsubs: Vec<String> = Vec::new();
            let scan = |text: &str, prefix: &mut Vec<String>| {
                let (unbound, bodies) = self.nounset_unbound_parameter_with_comsubs(text);
                prefix.extend(bodies);
                unbound
            };
            for word in &cmd.words {
                let masked =
                    Self::mask_recorded_process_substitutions(word, &cmd.process_substitutions);
                if let Some(name) = scan(&masked, &mut prefix_comsubs) {
                    for body in &prefix_comsubs {
                        self.expand_command_substitution(body);
                    }
                    let (name, message) = self.nounset_unbound_diagnostic(name);
                    return Some((name, message, Self::FATAL_PARAMETER_EXPANSION_STATUS));
                }
            }
            for value in cmd.assignment_values() {
                let masked =
                    Self::mask_recorded_process_substitutions(value, &cmd.process_substitutions);
                if let Some(name) = scan(&masked, &mut prefix_comsubs) {
                    for body in &prefix_comsubs {
                        self.expand_command_substitution(body);
                    }
                    let (name, message) = self.nounset_unbound_diagnostic(name);
                    return Some((name, message, Self::FATAL_PARAMETER_EXPANSION_STATUS));
                }
            }
        }
        for word in &cmd.words {
            let masked =
                Self::mask_recorded_process_substitutions(word, &cmd.process_substitutions);
            if let Some(error) = self.parameter_expansion_error_in_word(&masked) {
                return Some(error);
            }
        }
        for value in cmd.assignment_values() {
            let masked =
                Self::mask_recorded_process_substitutions(value, &cmd.process_substitutions);
            if let Some(error) = self.parameter_expansion_error_in_word(&masked) {
                return Some(error);
            }
        }

        None
    }

    /// Side-effect-free detection twin of `parameter_expansion_error` plus
    /// the heredoc/here-string gate (`parameter_heredoc_expansion_error`):
    /// true when executing this command through the full simple-command
    /// path would report a word-expansion error. The reporting entry point
    /// cannot be probed speculatively — its nounset arm executes the
    /// collected comsub prefix bodies for their side effects — so the
    /// concurrent-pipeline admission (pipeline_exec.rs) asks this pure
    /// question instead and declines to the sequential stage executor,
    /// whose own gates then report the error exactly once (rubash#1130:
    /// a pipeline element's here-string skipped the nounset check on both
    /// the concurrent path and, for the heredoc family, the stage path).
    pub(in crate::executor) fn parameter_expansion_error_pending(&self, cmd: &CommandNode) -> bool {
        if cmd.arithmetic_command.is_some() {
            return false;
        }
        if self.parameter_heredoc_expansion_error(cmd).is_some() {
            return true;
        }
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            for text in cmd.words.iter().chain(cmd.assignment_values()) {
                let masked =
                    Self::mask_recorded_process_substitutions(text, &cmd.process_substitutions);
                let (unbound, _bodies) = self.nounset_unbound_parameter_with_comsubs(&masked);
                if unbound.is_some() {
                    return true;
                }
            }
        }
        cmd.words.iter().chain(cmd.assignment_values()).any(|word| {
            let masked =
                Self::mask_recorded_process_substitutions(word, &cmd.process_substitutions);
            self.parameter_expansion_error_in_word(&masked).is_some()
        })
    }

    /// rubash#1540: side-effect-free prediction of the two DISCARD
    /// classes the textual pre-scan above cannot see — an arithmetic
    /// expansion error (the scan never evaluates `$(( ))`) and a
    /// failglob failure (pathname expansion depends on the live
    /// directory). Used only as concurrent-pipeline admission, beside
    /// `parameter_expansion_error_pending`: GNU forks every pipeline
    /// element and expands its words exactly once inside the child
    /// (execute_cmd.c:2620 execute_pipeline -> execute_simple_command
    /// -> subst.c:11229 expand_word_internal), so a member predicted
    /// to fail must decline the fast path BEFORE any speculative
    /// expansion prints — the sequential stage executor then reports
    /// the error exactly once, with the element's own status. A false
    /// hit only costs the fast path; the sequential path is always
    /// semantically complete.
    pub(in crate::executor) fn pipeline_member_arith_failglob_risk(
        &self,
        cmd: &CommandNode,
    ) -> bool {
        if cmd.arithmetic_command.is_some() {
            return false;
        }
        // Evaluate each command-substitution-free `$((...))` body
        // against the current environment on the evaluator's scratch
        // clone (its writes are dropped): the same verdict the real
        // expansion would reach for state-independent bodies
        // (division by zero, bad literals, syntax errors). Bodies
        // containing `$(`/backticks are skipped — GNU runs the
        // substitution as the expansion walk reaches it (subst.c:7143
        // command_substitute), which a pure probe must not do; the
        // stage-spec latch net in pipeline_exec covers those instead.
        let mut texts: Vec<&str> = cmd.words.iter().map(String::as_str).collect();
        texts.extend(cmd.assignment_values().map(String::as_str));
        for text in texts {
            for body in speculative_arithmetic_bodies(text) {
                let (value, _writes, _category) =
                    crate::executor::arithmetic::eval_conditional_arith_value_categorized_with_writes(
                        &body,
                        &self.shell_state.env_vars,
                    );
                if value.is_none() {
                    return true;
                }
            }
        }
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "failglob") {
            for (index, word) in cmd.words.iter().enumerate() {
                let text = cmd
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str())
                    .unwrap_or(word.as_str());
                if word_has_unquoted_glob_meta(text) {
                    return true;
                }
            }
        }
        false
    }

    pub(in crate::executor) fn parameter_expansion_error_in_heredoc_body(
        &self,
        body: &str,
    ) -> Option<(String, String, i32)> {
        if body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER) {
            return None;
        }
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        // Heredoc bodies are raw text: quote characters are literal data
        // (redir.c heredoc expansion has no quote removal), so the scan must
        // keep treating every `${` as an expansion start.
        self.parameter_expansion_error_in_word_context(body, false)
    }

    pub(in crate::executor) fn parameter_expansion_error_in_word(
        &self,
        word: &str,
    ) -> Option<(String, String, i32)> {
        self.parameter_expansion_error_in_word_context(word, true)
    }

    /// Report a word-expansion error found outside the simple-command
    /// pre-scan (for-list words, the case subject, [[ ]] operands —
    /// rubash#1130) and raise its GNU consequence, mirroring the
    /// simple-command handling in command_prepare: the diagnostic goes
    /// to fd 2 as currently bound; the FATAL class (nounset unbound,
    /// `${x?}`, bad @xform) is FORCE_EOF — ExitCode(expansion_fatal_status)
    /// noninteractively, a status-1 DISCARD in an interactive reader;
    /// lesser classes abandon the current command list with their status.
    pub(in crate::executor) fn raise_word_expansion_error(
        &mut self,
        (name, message, status): (String, String, i32),
    ) -> Result<(), ExecuteError> {
        let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
        self.write_diagnostic_fd2(line.as_bytes());
        if status == Self::FATAL_PARAMETER_EXPANSION_STATUS {
            if self.expansion_error_is_interactive_discard() {
                self.exit_code = 1;
                // rubash#439: expansion error, not a failed command.
                self.command_expansion_failed.set(true);
                return Err(ExecuteError::ExpansionFailure(1));
            }
            let code = self.expansion_fatal_status();
            self.exit_code = code;
            return Err(ExecuteError::ExitCode(code));
        }
        self.exit_code = status;
        self.command_expansion_failed.set(true);
        // Every lesser class abandons the current command list with its
        // status (the ExpansionFailure vehicle) — status 1 (substring /
        // bad substitution) and status 2 (the rubash#1597 bad array
        // subscript, EX_BADUSAGE) alike. ExitCode is reserved for the
        // FATAL class above; routing a lesser class through it aborted
        // the whole script from compound positions (for / case).
        Err(ExecuteError::ExpansionFailure(status))
    }

    /// The single owner of the fatal-expansion exit status (FORCE_EOF
    /// class: `${var?msg}` / nounset word errors, nounset and posix-fatal
    /// arithmetic). GNU chain: subst.c:10168/11027 (nounset),
    /// subst.c:8158 (`:?`), expr.c:1190 (`(( ))` nounset), subst.c:4295-4296
    /// and subst.c:8221 all `set_exit_status(EXECUTION_FAILURE)` = 1 and
    /// `jump_to_top_level(FORCE_EOF)`. The 127 remap exists ONLY at the
    /// `-c` top-level catch, shell.c:1471 run_one_command
    /// (`case FORCE_EOF: return last_command_exit_value = 127`); in script
    /// mode eval.c:98-107 reader_loop ends input and exits with
    /// last_command_exit_value (1). With errexit on the fatal never reaches
    /// that catch: error.c:169-187 report_error's `exit_immediately_on_error`
    /// branch EXITPROG-jumps with 1 — see the arm below.
    ///
    /// Every forked child that re-arms `top_level` contains the jump and
    /// exits with last_command_exit_value — 1 — instead: the `( ... )`
    /// subshell child (execute_cmd.c:1811 execute_in_subshell: any jump
    /// → `return_code = last_command_exit_value ?: EXECUTION_FAILURE`,
    /// niubash#163: `bash -c '( set -u; echo $U )'` exits 1) and the
    /// `$( )`/backtick comsub child (subst.c:7393-7404, rubash#154:
    /// `bash -c 'v=$(echo ${x:?})'` exits 1). In rubash those boundaries
    /// are `subshell_depth > 0` (compound_exec.rs/ast_exec.rs/
    /// external_finish.rs/comsub fork copies) plus the
    /// `__RUBASH_COMSUB_BODY` fast-path comsub marker. The nofork funsub
    /// `${ ...; }` runs in the parent and keeps the 127 mapping (GNU
    /// subst.c:7057 exp_jump_to_top_level out of the in-parent body).
    pub(in crate::executor) fn expansion_fatal_status(&self) -> i32 {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_COMSUB_BODY")
            || self.shell_state.subshell_depth.get() > 0
        {
            1
        } else if self.shell_state.env_vars.contains_key("__RUBASH_IS_C") {
            // GNU error.c:169-187 report_error: when `set -e' is in effect
            // (exit_immediately_on_error / flags.c:263) the error report
            // itself calls exit_shell(last_command_exit_value), which
            // EXITPROG-jumps out with EXECUTION_FAILURE (1) and never
            // reaches shell.c:1471 run_one_command's FORCE_EOF remap to 127.
            // So `bash -eu -c 'echo $UNDEF'` exits 1 while `bash -u -c` (no
            // -e) still exits 127; the `-c` 127 mapping holds only with
            // errexit off. The option value at the error site is what GNU
            // reads — an `if`/`||` context does not clear it (empirical GNU
            // 5.3.0: `bash -eu -c 'false || echo $UNDEF'` exits 1).
            if self.errexit_enabled() {
                1
            } else {
                127
            }
        } else {
            1
        }
    }

    /// True when this executor's top frame is a command-substitution body
    /// reader — GNU runs the body through parse_and_execute inside the
    /// forked child (subst.c:7413 command_substitute), whose DISCARD catch
    /// (builtins/evalstring.c:423-426) forces
    /// `last_result = last_command_exit_value = EXECUTION_FAILURE` and, in
    /// a subshell environment, stops the body outright. A DISCARD-class
    /// abandonment raised anywhere in the child's clone tree (eval/source
    /// frames, function bodies, `( )` regions, pipeline elements) therefore
    /// surfaces as status 1 at the substitution boundary, never as the
    /// pre-set EX_BADUSAGE 2 a script-level reader_loop catch preserves
    /// (eval.c:111-118) — rubash#1541/#1542. The marker lives only in the
    /// comsub child's cloned env (command_substitution.rs inserts
    /// `__RUBASH_COMSUB_BODY`), so the main executor and `( )`-only
    /// subshells (execute_in_subshell keeps the pre-set status,
    /// execute_cmd.c:1821-1825) never fold.
    pub(in crate::executor) fn discard_fold_frame(&self) -> bool {
        let has = self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_COMSUB_BODY");
        has
    }

    /// rubash#1563: GNU's dynamic `exit_immediately_on_error`
    /// (execute_cmd.c:5011-5016 sets it 0 for a framed eval/source body,
    /// :5110 restores it at builtin completion) — the errexit option as
    /// gated by an active CMD_IGNORE_RETURN frame. Distinct from
    /// errexit_enabled(), the raw option: exempt positions do not clear
    /// the dynamic flag, but the eval/source frame does, and a `set -e`
    /// executed inside the frame updates the option without re-arming
    /// the print-time exit (flags.c:262 change_flag consults
    /// builtin_ignoring_errexit). Diagnostic print sites that GNU routes
    /// through error.c report_error's exit check must read this, not the
    /// option.
    pub(in crate::executor) fn exit_immediately_on_error(&self) -> bool {
        self.errexit_enabled() && self.errexit_ignore_frame == 0
    }

    /// rubash#1563: true when a DISCARD raised now would land on a
    /// parse_and_execute catch — an eval/source body whose frame was
    /// entered at the CURRENT subshell_depth. A `( )` region entered
    /// since the frame bumps subshell_depth and installs
    /// execute_in_subshell's preserving catch, so the depths differ and
    /// the fold must not fire (execute_cmd.c:1821-1825).
    pub(in crate::executor) fn in_parse_and_execute_frame(&self) -> bool {
        self.parse_and_execute_frames.last() == Some(&self.shell_state.subshell_depth.get())
    }

    /// rubash#1563: true when this executor is executing a subshell
    /// environment's body — a `( )` region or command-substitution body
    /// on this executor, or a fork copy (pipeline stage, process
    /// substitution, substitution child), whose constructor bumps
    /// subshell_depth. GNU's parse_and_execute DISCARD arm checks the
    /// same condition (`subshell_environment`, evalstring.c:427-450):
    /// there the folded DISCARD re-jumps to the enclosing catch instead
    /// of returning from parse_and_execute.
    pub(in crate::executor) fn in_subshell_environment(&self) -> bool {
        self.shell_state.subshell_depth.get() > 0
            || self.shell_state.in_command_substitution.get()
            || self
                .shell_state
                .env_vars
                .contains_key("__RUBASH_COMSUB_BODY")
    }

    /// rubash#1563: enter a parse_and_execute body (an eval string or a
    /// sourced file). Pushes the frame — GNU evalstring.c
    /// parse_and_execute, reached from eval_builtin and from
    /// evalfile.c source_file — and, when the invoking builtin was
    /// itself dispatched under CMD_IGNORE_RETURN (rubash's
    /// suppress_errexit raised at entry) outside a subshell environment,
    /// raises the errexit ignore frame for the body's duration
    /// (execute_cmd.c:5004-5016: the frame applies only when
    /// `subshell == 0` and the builtin is eval/source under
    /// CMD_IGNORE_RETURN / CMD_COMMAND_BUILTIN). Returns the saved
    /// counters for exit_parse_and_execute_frame; every exit path of the
    /// body must restore them.
    pub(crate) fn enter_parse_and_execute_frame(&mut self) -> (usize, usize) {
        let saved = (
            self.parse_and_execute_frames.len(),
            self.errexit_ignore_frame,
        );
        self.parse_and_execute_frames
            .push(self.shell_state.subshell_depth.get());
        if self.suppress_errexit > 0 && !self.in_subshell_environment() {
            self.errexit_ignore_frame += 1;
        }
        saved
    }

    pub(crate) fn exit_parse_and_execute_frame(&mut self, saved: (usize, usize)) {
        self.parse_and_execute_frames.truncate(saved.0);
        self.errexit_ignore_frame = saved.1;
    }

    /// rubash#1542: the FINAL status a failed `${var:=word}`/`${var=word}`
    /// assignment produces in the current frame, given the class's
    /// pre-set status (`class_status`: EX_BADUSAGE 2 for a readonly
    /// bind, subst.c parameter_brace_expand_rhs; EXECUTION_FAILURE 1
    /// for a special/positional target, subst.c:10402-10404) and `$?`
    /// from before the expansion (`prev_status`). GNU prints the
    /// diagnostic through error.c report_error — from inside
    /// bind_variable for the readonly class (variables.c err_readonly),
    /// immediately after set_exit_status(1) for the special class — and
    /// report_error exit_shells on the spot when
    /// exit_immediately_on_error is live, with the status current at
    /// print time: the pre-bind `$?` (0 remapped to EXECUTION_FAILURE)
    /// for readonly, 1 for special. The flag is the DYNAMIC
    /// exit_immediately_on_error: errexit-exempt positions do not clear
    /// it, so they die too, but the eval/source CMD_IGNORE_RETURN frame
    /// does (execute_cmd.c:5004-5016, rubash#1563). Without the
    /// print-time exit the DISCARD proceeds with the pre-set status,
    /// folded to EXECUTION_FAILURE at a parse_and_execute catch — a
    /// command-substitution body (discard_fold_frame) or an eval/source
    /// body (in_parse_and_execute_frame).
    pub(in crate::executor) fn param_assign_discard_status(
        &self,
        class_status: i32,
        prev_status: i32,
    ) -> i32 {
        if self.exit_immediately_on_error() {
            if class_status == 1 || prev_status == 0 {
                1
            } else {
                prev_status
            }
        } else if self.discard_fold_frame() || self.in_parse_and_execute_frame() {
            1
        } else {
            class_status
        }
    }

    /// GNU's `interactive_shell` at the word-expansion error sites — the
    /// branch that decides expand_*_error (DISCARD) vs expand_*_fatal
    /// (FORCE_EOF):
    /// - subst.c:10416-10418 parameter_brace_expand `?` arm:
    ///   `return (interactive_shell ? &expand_wdesc_error :
    ///   &expand_wdesc_fatal)`;
    /// - subst.c:11032-11034 unbound_variable label (set -u nounset):
    ///   `return ((unbound_vars_is_error && interactive_shell == 0) ?
    ///   &expand_wdesc_fatal : &expand_wdesc_error)`;
    /// - subst.c:8949 parameter_brace_transform (`@xform`):
    ///   `return (interactive_shell ? &expand_param_error :
    ///   &expand_param_fatal)`;
    /// - expr.c:1208-1216 expr_streval (arithmetic unbound under set -u):
    ///   `if (interactive_shell) { ...; jump_to_top_level (DISCARD); } else
    ///   jump_to_top_level (FORCE_EOF)`.
    ///
    /// DISCARD keeps the interactive session alive: the current command is
    /// abandoned with status 1 and reader_loop reads the next line
    /// (eval.c:111-128). Every non-reader context counts as
    /// non-interactive at these sites: `-c` runs with interactive_shell 0
    /// even under `-i` (shell.c init_noninteractive; WSL 5.3.0 probe:
    /// `bash -i -c 'echo ${X?}'` exits 127 without running the tail), and
    /// a `( )`/`$( )` child has interactive_shell 0, so the fatal stays
    /// contained (expansion_fatal_status). `bash -i script` is
    /// init_interactive_script (shell.c:1870, interactive_shell = 1), so
    /// the DISCARD applies there (WSL probe: `bash -i xq.sh` prints the
    /// error and still runs the next line).
    pub(in crate::executor) fn expansion_error_is_interactive_discard(&self) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_INTERACTIVE")
            .map(String::as_str)
            == Some("1")
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_IS_C")
                .map(String::as_str)
                != Some("1")
            && !self
                .shell_state
                .env_vars
                .contains_key("__RUBASH_COMSUB_BODY")
            && self.shell_state.subshell_depth.get() == 0
    }

    /// The session half of the interactive-discard rule, WITHOUT the
    /// frame conditions: this executor is serving a reader (an
    /// interactive shell or a `bash -i` script — a `-c` unit is not a
    /// reader session even under `-i`). Foreground-wait SIGINT
    /// protection and the rubash#1639 line discard key on this alone:
    /// GNU protects every wait_for of the session and throws from the
    /// wait that owned the process's foreground job, and in the flat
    /// model a subshell frame's wait IS that wait (the frame is the
    /// job), so no depth/comsub condition applies there the way it
    /// does to expansion-error fatality.
    pub(in crate::executor) fn interactive_reader_session(&self) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_INTERACTIVE")
            .map(String::as_str)
            == Some("1")
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_IS_C")
                .map(String::as_str)
                != Some("1")
    }

    /// `quote_aware` mirrors GNU word expansion (subst.c): quotes in a word
    /// delimit data, so `${` inside `'...'`, a $'...' string, a backtick
    /// body or a $(...) body is not an expansion start of THIS word
    /// (quote.tests:117 `echo \`echo '${'\```). Heredoc bodies keep the raw
    /// scan because quotes are literal there.
    fn parameter_expansion_error_in_word_context(
        &self,
        word: &str,
        quote_aware: bool,
    ) -> Option<(String, String, i32)> {
        // perf6 admission: same necessary condition as
        // parameter_assignment_error_in_word — every arm (nounset `$ref`,
        // `${` spans, `${|`) needs an ASCII `$`; a `$`-free word cannot
        // produce a diagnostic. Heredoc bodies route here too (quotes are
        // literal data there, but the `$` requirement is unchanged).
        if !word.as_bytes().contains(&b'$') {
            return None;
        }
        let word = word
            .strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX)
            .or_else(|| word.strip_prefix(STORAGE_WORD_PREFIX))
            .unwrap_or(word);
        // Preserve Rubash's nested current-shell extension; its `${| ... }`
        // marker disambiguates the legacy enclosing form from this Bash error.
        if word.contains("${|") {
            return None;
        }
        // rubash#1597: a triggered `=`/`:=` on the all-elements
        // spelling errors in EVERY word position GNU expands (for-list
        // words, the case subject, [[ ]] operands reach this scan, not
        // the simple-command pre-scan). EX_BADUSAGE, like the readonly
        // assignment class.
        if let Some(name) = self.bad_array_subscript_assignment_in_word(word) {
            return Some((name, "bad array subscript".to_string(), 2));
        }
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            if let Some(name) = self.nounset_unbound_parameter(word) {
                // expr.c expr_streval raises FORCE_EOF for unbound vars
                // under set -u — fatal like `${x?}` (see command_execute).
                let (name, message) = self.nounset_unbound_diagnostic(name);
                return Some((name, message, Self::FATAL_PARAMETER_EXPANSION_STATUS));
            }
        }
        let mut rest = word;
        loop {
            // P_ARITH units are opaque to this scan (rubash#533): GNU reads
            // a `$((...))`/`$[...]` body through parse_matched_pair (P_ARITH)
            // where a `$`-`{` is literal text, so a `${` inside belongs to
            // the arithmetic string, never to a parameter expansion of this
            // word. An unterminated unit consumes the rest of the word; the
            // arithmetic expansion family owns that diagnostic.
            if let Some(len) = Self::arithmetic_unit_prefix_span(rest) {
                rest = &rest[len..];
                continue;
            }
            let start = if quote_aware {
                next_quoted_parameter_expansion_start(rest)
            } else {
                rest.find("${")
            };
            let Some(start) = start else {
                break;
            };
            let after_start = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(after_start) else {
                return Some((
                    "${".to_string() + after_start + "}",
                    "unexpected EOF while looking for matching `}'".to_string(),
                    2,
                ));
            };
            let inner = &after_start[..end];
            // GNU subst.c:10053 parameter_brace_expand -> valid_brace_expansion_word
            // (subst.c:7583): the reference must be all digits, a special
            // parameter, a valid identifier, or ONE well-formed non-empty
            // array subscript (valid_array_reference, arrayfunc.c:885). An
            // empty subscript (`${x[]}`) or a second subscript
            // (`${a[0][1]}`) is a bad substitution reported with the whole
            // WORD being expanded (subst.c:10278 report_error names the
            // expand_word_internal string — `a2:[${arr[0][0]}]`, not the
            // `${...}` span; the two coincide only when the word IS the
            // fragment, rubash#1483) — the read path used to expand
            // both silently empty (rubash#536). The check runs first
            // because GNU validates the reference before the operator
            // dispatch, and it covers the indirect/length prefixes (`${!x[]}`,
            // `${#x[]}`) whose reference obeys the same rule.
            if Self::malformed_braced_reference(inner) {
                return Some((
                    crate::executor::expand_word::bad_substitution_display(word),
                    "bad substitution".to_string(),
                    1,
                ));
            }
            // GNU rejects a parameter expansion nested inside another
            // parameter expansion inside an array subscript. Reject it
            // before the mutable whole-word expander can recurse indefinitely;
            // a single nested subscript such as `${A[${i}]}` remains valid.
            if inner.contains("[${${") {
                return Some((
                    crate::executor::expand_word::bad_substitution_display(word),
                    "bad substitution".to_string(),
                    1,
                ));
            }
            // GNU subst.c:10053 valid_brace_expansion_word: after ${, the
            // parameter name must be a valid name/special-var/subscripted
            // form. `${$(...` parses as special var $ followed by a (`
            // token, which is not a valid operator tail — bad substitution
            // (new-exp.tests `${c//${$(($#-1))}/x/}`).
            if inner.starts_with("$(") || inner.contains("${$(") {
                return Some((
                    crate::executor::expand_word::bad_substitution_display(word),
                    "bad substitution".to_string(),
                    1,
                ));
            }
            // GNU subst.c:8944-8951 parameter_brace_transform +
            // valid_parameter_transform (subst.c:8898): a `@xform` suffix on
            // a parameter name must be exactly one valid transform
            // character; `${x@C}`/`${x@}` are fatal bad substitutions.
            if let Some(base) = invalid_at_transform_base(inner) {
                // parameter_brace_transform (subst.c:8927) returns NULL for
                // an unset variable BEFORE validating the transform
                // character, so `${unset@C}`/`${unset@}` expand empty
                // without error; only a set variable reaches the fatal
                // valid_parameter_transform check at subst.c:8944.
                let is_set = self.parameter_error_value(base).is_some()
                    || self.shell_state.env_vars.contains_key(base);
                if is_set {
                    return Some((
                        crate::executor::expand_word::bad_substitution_display(word),
                        "bad substitution".to_string(),
                        Self::FATAL_PARAMETER_EXPANSION_STATUS,
                    ));
                }
            }
            // `${#X}` is the length form. X must be a valid parameter name
            // (special, shell name, numeric positional, or `arr[@]` index form).
            // Other suffixes such as `${#:}`, `${#/}`, `${#1xyz}` are bad
            // substitution in GNU Bash.
            if let Some(length_name) = inner.strip_prefix('#') {
                if !Self::is_valid_length_parameter_name(length_name) {
                    return Some((
                        crate::executor::expand_word::bad_substitution_display(word),
                        "bad substitution".to_string(),
                        1,
                    ));
                }
            }
            // Indirect expansions accept a simple parameter reference, an
            // array subscript form, or the prefix@ variable-name listing
            // form; anything else (e.g. a stray trailing '!' in ${!bad!})
            // is a bad substitution in GNU.
            if let Some(indirect) = self.indirect_parameter_body(inner) {
                if !Self::is_valid_indirect_expression(indirect) {
                    return Some((
                        crate::executor::expand_word::bad_substitution_display(word),
                        "bad substitution".to_string(),
                        1,
                    ));
                }
                // Value-dependent GNU diagnostics for the plain indirect
                // form ${!name} (subst.c parameter_brace_expand_indir): an
                // unset name reports "invalid indirect expansion" and aborts
                // the command; a set name whose value is not a valid
                // indirect target reports "invalid variable name". A set
                // name whose value is a valid identifier expands normally -
                // even when the target itself is unset (GNU probe:
                // x=validname with validname unset expands empty).
                //
                // "Unset" here means NO variable ENTRY: the nameref walk
                // find_variable_last_nameref (variables.c:2051) reports no
                // entry for an absent name and for a nameref chain that
                // stops on an empty cell (or cycles / exceeds depth) — both
                // keep the fatal (subst.c:7913 fires it exactly when the
                // walk found nothing and the body is a plain identifier).
                // A declared-unset non-nameref entry (bare `declare x` /
                // `local x`, the att_invisible port) IS found, so
                // parameter_brace_find_indir yields an unset value and the
                // whole expansion is empty — WSL 5.3.0:
                // `declare x; echo "[${!x}]"` prints `[]` and the script
                // continues (rubash#529).
                if !indirect.is_empty() && is_shell_name(indirect) {
                    let source_found = match self.nameref_resolution(indirect) {
                        NamerefResolution::Target(_) => true,
                        NamerefResolution::NotNameref => {
                            self.shell_state.env_vars.contains_key(indirect)
                                || is_marked_var(
                                    &self.shell_state.env_vars,
                                    DECLARED_UNSET_VARS,
                                    indirect,
                                )
                                || self.pipestatus_cell_exists(indirect)
                        }
                        NamerefResolution::Unresolved
                        | NamerefResolution::Circular
                        | NamerefResolution::MaxDepth => false,
                    };
                    match self.shell_state.env_vars.get(indirect) {
                        None if !source_found => {
                            return Some((
                                indirect.to_string(),
                                "invalid indirect expansion".to_string(),
                                1,
                            ));
                        }
                        None => {
                            // Declared-unset entry: the expansion is empty,
                            // never a diagnostic (rubash#529).
                        }
                        Some(_) => {
                            // GNU validates t — the scalar-form value from
                            // parameter_brace_find_indir (array element 0
                            // / assoc key "0") — not the raw storage text
                            // (rubash#1147: `${!a}` with a=(p q) validated
                            // the marker-joined storage and leaked it).
                            if let Some(t) = self.indirect_source_scalar_value(indirect) {
                                if !Self::is_valid_indirect_target(&t) {
                                    return Some((t, "invalid variable name".to_string(), 1));
                                }
                            }
                        }
                    }
                }
                // GNU subst.c:9978-10007 parameter_brace_expand:
                // `${!NAME*}` / `${!NAME@}` with a valid-name prefix is the
                // variable-name-prefix list form — handled before the indir
                // validation, so a trailing `@` or `*` is the list marker,
                // not an operator tail.
                else if indirect
                    .strip_suffix(['@', '*'])
                    .is_none_or(|prefix| prefix.is_empty() || !is_shell_name(prefix))
                {
                    // `${!name-op...}` indirect-with-operator form: GNU
                    // parameter_brace_expand_indir (subst.c:7911-7918) checks
                    // the indirect NAME before the operator tail is applied —
                    // a valid identifier that does not name a variable is an
                    // "invalid indirect expansion" error, aborting the command
                    // without evaluating the operator's rhs (nameref3.sub:29
                    // `recho "${!foo-unset}"` prints nothing).
                    let head_end = indirect
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(indirect.len());
                    let (head, tail) = indirect.split_at(head_end);
                    if head_end > 0
                        && is_shell_name(head)
                        && !tail.starts_with('[')
                        && matches!(
                            tail.chars().next(),
                            Some(':' | '-' | '+' | '=' | '?' | '#' | '%' | '/' | '^' | ',' | '@')
                        )
                        && self.shell_state.env_vars.get(head).is_none()
                        && self.shell_state.variables.get(head).is_none()
                        && !self.pipestatus_cell_exists(head)
                    {
                        return Some((
                            head.to_string(),
                            "invalid indirect expansion".to_string(),
                            1,
                        ));
                    }
                    // GNU subst.c:7924-7934 parameter_brace_expand_indir:
                    // for `${!name[sub]}` an unresolvable BASE variable is
                    // an error — array_variable_part reduces to
                    // find_variable(base), which follows namerefs, so a
                    // nameref whose target is unset reports
                    // `name[sub]: invalid indirect expansion` while a set
                    // scalar/array base expands silently even when the
                    // element itself is unset.
                    if head_end > 0 && is_shell_name(head) && tail.starts_with('[') {
                        // The subscript group may close mid-tail when an
                        // operator follows (`${!m[@]:-x}`): GNU reports
                        // the reference through the closing `]` (subst.c
                        // parameter_brace_expand_indir), so the diagnostic
                        // names `head + tail[..close]`, not the whole body
                        // (rubash#1147: unset m with `[@]:-` was silent).
                        if let Some(close) = Self::skip_indirect_array_subscript(tail) {
                            // The PURE keys form — the `[@]`/`[*]` group
                            // closes the body — never reaches
                            // parameter_brace_expand_indir at all: GNU
                            // parameter_brace_expand (subst.c:10016)
                            // dispatches it to array_keys first, which
                            // reports an absent variable's keys as the
                            // empty list, so there is no base-existence
                            // error to mirror (rubash#1566). An operator
                            // tail after the group (`${!m[@]:-x}`) is
                            // NOT the pure form and keeps the error.
                            let pure_keys_form =
                                close == tail.len() && matches!(&tail[1..close - 1], "@" | "*");
                            let base_exists = match self.nameref_resolution(head) {
                                NamerefResolution::Target(target) => {
                                    let base = target.split('[').next().unwrap_or(target.as_str());
                                    self.shell_state.env_vars.contains_key(base)
                                        || self.shell_state.variables.get(base).is_some()
                                        || self.pipestatus_cell_exists(base)
                                }
                                NamerefResolution::NotNameref => {
                                    self.shell_state.env_vars.contains_key(head)
                                        || self.shell_state.variables.get(head).is_some()
                                        || self.pipestatus_cell_exists(head)
                                }
                                // Unresolved (empty/unresolvable cell), Circular,
                                // and MaxDepth all mean find_variable returned
                                // NULL for the base.
                                _ => false,
                            };
                            if !base_exists && !pure_keys_form {
                                return Some((
                                    format!("{}{}", head, &tail[..close]),
                                    "invalid indirect expansion".to_string(),
                                    1,
                                ));
                            }
                            // The PLAIN subscripted form (the group closes
                            // the body) validates t too — here t is the
                            // ELEMENT value (subst.c:7955-7961): `${!v[0]}`
                            // with v="1bad" errors, while an unset element
                            // yields no t and proceeds silently. The
                            // trailing [@]/[*] KEYS form never validates
                            // (subst.c:7887 keys shortcut) — rubash#1147.
                            if close == tail.len() {
                                let subscript = &tail[1..close - 1];
                                if subscript != "@" && subscript != "*" {
                                    let reference = format!("{head}{}", &tail[..close]);
                                    if let Some(element) =
                                        self.array_element_parameter_value(&reference)
                                    {
                                        if !Self::is_valid_indirect_target(&element) {
                                            return Some((
                                                element,
                                                "invalid variable name".to_string(),
                                                1,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // GNU subst.c:7955-7961 parameter_brace_expand_indir:
                    // with an operator tail the form is indirection
                    // through t, the head's scalar-form value, and an
                    // invalid t reports `<t>: invalid variable name`
                    // BEFORE the operator applies — for scalar sources
                    // too (`${!v:-d}` with v="" or v="1bad"), not only
                    // for the `[@]` form (rubash#1147). A source with no
                    // scalar-form value proceeds silently; a nameref head
                    // names its target's value, adjudicated by the
                    // nameref arms.
                    if head_end > 0
                        && is_shell_name(head)
                        && !tail.starts_with('[')
                        && matches!(
                            tail.chars().next(),
                            Some(':' | '-' | '+' | '=' | '?' | '#' | '%' | '/' | '^' | ',' | '@')
                        )
                        && self.nameref_target_name(head).is_none()
                    {
                        if let Some(t) = self.indirect_source_scalar_value(head) {
                            if !Self::is_valid_indirect_target(&t) {
                                return Some((t, "invalid variable name".to_string(), 1));
                            }
                        }
                    }
                }
                if Self::is_indirect_array_operator_expression(indirect) {
                    // GNU treats `${!name[@]<op>...}` as indirection through
                    // the VALUE of `name` re-expanded as a parameter
                    // (parameter_brace_expand_indir, subst.c:7941-7945), so a
                    // value that is not a valid indirect target reports
                    // "<value>: invalid variable name" (new-exp13.sub:56
                    // ${!VAR4[@]@Q}). The plain `${!name[@]}` form has no
                    // operator tail: it is the KEYS expansion and never
                    // validates the value here.
                    let open = indirect.find('[').unwrap_or(0);
                    let base = &indirect[..open];
                    if !base.is_empty() && is_shell_name(base) {
                        // The PIPESTATUS array object lives in
                        // shell_state, not env_vars (rubash#1566): its
                        // storage render comes from
                        // parameter_array_storage, so the t-join
                        // adjudication below sees the same element
                        // values GNU joins.
                        let pipestatus_storage;
                        let base_value = match self.shell_state.env_vars.get(base) {
                            Some(value) => Some(value),
                            None if self.pipestatus_cell_exists(base) => {
                                pipestatus_storage = self.parameter_array_storage(base);
                                pipestatus_storage.as_ref()
                            }
                            None => None,
                        };
                        if let Some(value) = base_value {
                            // GNU's t here is the dollar_at join of the
                            // variable's ELEMENT VALUES (array_value with
                            // AV_ALLOWALL, arrayfunc.c:1563), and validity
                            // is judged on that join — not on the raw
                            // storage text, whose in-band separators made
                            // every multi-element array "invalid" and
                            // every empty assoc "invalid" too (rubash#1147:
                            // `${!assoc[@]:-d}` errored on an empty assoc
                            // whose t is NULL and proceeds silently, and on
                            // a one-element assoc whose value IS valid).
                            // An array with no elements yields no t at all;
                            // a scalar base yields its value as the sole
                            // element.
                            let element_values =
                                if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base) {
                                    self.parameter_array_storage(base).map(|storage| {
                                        assoc_hash_ordered_values(
                                            &storage,
                                            assoc_nbuckets(&self.shell_state.env_vars, base),
                                        )
                                    })
                                } else if is_array_storage(value) {
                                    Some(array_values(value))
                                } else {
                                    None
                                };
                            let t = match element_values {
                                Some(values) if !values.is_empty() => Some(values.join(" ")),
                                Some(_) => None,
                                None => Some(value.clone()),
                            };
                            if let Some(t) = t {
                                if !Self::is_valid_indirect_target(&t) {
                                    return Some((t, "invalid variable name".to_string(), 1));
                                }
                            }
                        }
                    }
                }
            }
            // `${ command; }` is not Bash command substitution. Rubash also
            // supports the distinct `${| command; }` current-shell form, so
            // reject only the whitespace-led form as a bad substitution.
            if inner.chars().next().is_some_and(char::is_whitespace)
                && !word_contains_current_shell_command_substitution(word)
            {
                return Some((
                    crate::executor::expand_word::bad_substitution_display(word),
                    "bad substitution".to_string(),
                    1,
                ));
            }
            if let Some((name, message, require_non_empty)) =
                parse_parameter_error_operator(inner, self.posix_mode_enabled())
            {
                let value = self.parameter_error_value(name);
                let is_error = if require_non_empty {
                    value.as_deref().map(str::is_empty).unwrap_or(true)
                } else {
                    value.is_none()
                };
                if is_error {
                    let message = if message.is_empty() {
                        if require_non_empty {
                            "parameter null or not set".to_string()
                        } else {
                            "parameter not set".to_string()
                        }
                    } else {
                        self.expand_parameter_word(message)
                    };
                    // GNU subst.c:8221 parameter_brace_expand_error →
                    // expand_wdesc_fatal (FORCE_EOF): `${x?}`/`${x:?}` abort
                    // the whole noninteractive script; subshell, pipeline,
                    // and command-substitution boundaries contain it.
                    return Some((
                        name.to_string(),
                        message,
                        Self::FATAL_PARAMETER_EXPANSION_STATUS,
                    ));
                }
            }
            if let Some((name, offset, Some(length))) = self.parse_parameter_substring(inner) {
                if length < 0 {
                    // GNU subst.c:8395 verify_substring_values, reached from
                    // parameter_brace_substring (subst.c:9050) only when the
                    // parameter's value is non-NULL: a negative length is an
                    // unconditional error for VT_POSPARMS (@/*) and for
                    // VT_ARRAYVAR. The [@]/[*] spelling alone does NOT make
                    // the operand an array — get_var_and_type resolves a
                    // scalar base to VT_VARIABLE, so the length is adjusted
                    // by the string length and errors only when the adjusted
                    // end still precedes the start (rubash#1600). An unset
                    // base (either spelling) and an array with no elements
                    // take parameter_brace_substring's early NULL return and
                    // never error.
                    let array_form_base = name
                        .strip_suffix("[@]")
                        .or_else(|| name.strip_suffix("[*]"));
                    let is_invalid = if matches!(name, "@" | "*") {
                        true
                    } else if let Some(base) = array_form_base {
                        if self.parameter_is_array_variable(base) {
                            // VT_ARRAYVAR: error exactly when the array
                            // has elements (its operator value is Some).
                            self.parameter_operator_value(name).is_some()
                        } else {
                            // Scalar (or unset) base under [@]/[*]:
                            // VT_VARIABLE rules over the base's own value.
                            self.parameter_error_value(base).is_some_and(|value| {
                                parameter_substring_has_negative_result(
                                    value.chars().count(),
                                    offset,
                                    length,
                                )
                            })
                        }
                    } else {
                        self.parameter_error_value(name).is_some_and(|value| {
                            parameter_substring_has_negative_result(
                                crate::executor::expand_braced_indices::parameter_char_length(
                                    &value,
                                ),
                                offset,
                                length,
                            )
                        })
                    };
                    if is_invalid {
                        return Some((
                            length.to_string(),
                            "substring expression < 0".to_string(),
                            1,
                        ));
                    }
                }
            }
            rest = &after_start[end + 1..];
        }
        None
    }

    /// Byte length of the balanced `open`/`close` group whose opener sits at
    /// `text[0]` (quote-aware), opener excluded from the count; `None` when
    /// the group never closes.
    fn balanced_group_span(text: &str, open: char, close: char) -> Option<usize> {
        let mut depth = 1usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        for (offset, ch) in text.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if single {
                if ch == '\'' {
                    single = false;
                }
                continue;
            }
            if double {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    double = false;
                }
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '\'' => single = true,
                '"' => double = true,
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(offset + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Leading P_ARITH unit span (`$((...))` / `$[...]`), quote-aware with
    /// balanced delimiters. `None` when the word does not start with an
    /// arithmetic unit; `Some(len)` covers the whole unit, and an
    /// unterminated unit covers the rest of the word.
    fn arithmetic_unit_prefix_span(word: &str) -> Option<usize> {
        let (open, close, body) = if word.starts_with("$((") {
            ('(', ')', 3)
        } else if word.starts_with("$[") {
            ('[', ']', 2)
        } else if word.starts_with("((") {
            // A command-position `((` word is parse_dparen's arithmetic
            // command when the inner group closes directly before `)` (GNU
            // accepts the arithmetic reading only then) or when the group
            // never closes at all (the arith reading is still pending at
            // EOF); the nested-subshell reinterpretation keeps `${` live.
            if let Some(span) = Self::balanced_group_span(&word[2..], '(', ')') {
                if word[2 + span..].starts_with(')') {
                    return Some(2 + span + 1);
                }
                return None;
            }
            return Some(word.len());
        } else {
            return None;
        };
        let mut depth = 1usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        for (offset, ch) in word[body..].char_indices() {
            let index = body + offset;
            if escaped {
                escaped = false;
                continue;
            }
            if single {
                if ch == '\'' {
                    single = false;
                }
                continue;
            }
            if double {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    double = false;
                }
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '\'' => single = true,
                '"' => double = true,
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + ch.len_utf8());
                    }
                }
                _ => {}
            }
        }
        Some(word.len())
    }

    /// rubash#319 companion to `nounset_unbound_parameter`: the same
    /// left-to-right walk, but the command-substitution bodies passed on
    /// the way (and backtick bodies) are returned alongside the unbound
    /// hit, so the caller can execute them before reporting the fatal —
    /// GNU's expand_word_internal (subst.c:11229) forks each comsub as the
    /// walk reaches it, so a child's stderr output precedes a later
    /// `unbound variable` diagnostic.
    pub(in crate::executor) fn nounset_unbound_parameter_with_comsubs(
        &self,
        word: &str,
    ) -> (Option<String>, Vec<String>) {
        let chars: Vec<char> = word.chars().collect();
        let mut comsubs = Vec::new();
        let mut index = 0usize;
        while index < chars.len() {
            let ch = chars[index];
            index += 1;
            if ch == DATA_DOLLAR {
                continue;
            }
            if ch == '`' {
                // Backtick command substitution: same forked-child nounset
                // boundary as `$( ... )` (GNU subst.c:7143 handles both
                // spellings). Collect the body for ordered execution and
                // never look inside (rubash#307).
                let mut body = String::new();
                while index < chars.len() {
                    let body_ch = chars[index];
                    index += 1;
                    if body_ch == '\\' {
                        if index < chars.len() {
                            body.push(chars[index]);
                            index += 1;
                        }
                        continue;
                    }
                    if body_ch == '`' {
                        break;
                    }
                    body.push(body_ch);
                }
                comsubs.push(body);
                continue;
            }
            if ch != '$' {
                continue;
            }

            match chars.get(index).copied() {
                Some('{') => {
                    index += 1;
                    let mut name = String::new();
                    while index < chars.len() {
                        let name_ch = chars[index];
                        index += 1;
                        if name_ch == '}' {
                            break;
                        }
                        name.push(name_ch);
                    }
                    if let Some(reported) = self.nounset_braced_parameter_is_unbound(&name) {
                        return (Some(reported), comsubs);
                    }
                }
                Some(first) if first.is_ascii_digit() => {
                    index += 1;
                    let digit = first.to_digit(10).unwrap_or(0) as usize;
                    if digit > 0
                        && self
                            .shell_state
                            .positional_window()
                            .get(digit - 1)
                            .is_none()
                    {
                        return (Some(format!("${first}")), comsubs);
                    }
                }
                Some(first) if is_shell_name_start(first) => {
                    let mut name = String::new();
                    while index < chars.len() && is_shell_name_char(chars[index]) {
                        name.push(chars[index]);
                        index += 1;
                    }
                    if let Some(reported) = self.nounset_braced_parameter_is_unbound(&name) {
                        return (Some(reported), comsubs);
                    }
                }
                Some('?') | Some('$') | Some('@') | Some('*') | Some('#') | Some('-') => {
                    index += 1;
                }
                Some('!') => {
                    index += 1;
                    if self.shell_state.last_background_pid.is_none() {
                        return (Some(String::from("$!")), comsubs);
                    }
                }
                Some('(') => {
                    index += 1;
                    if chars.get(index).copied() == Some('(') {
                        // `$(( ... ))` arithmetic is evaluated in the PARENT
                        // (no fork): keep scanning the text for `$name'
                        // references (GNU expr.c expr_streval).
                        continue;
                    }
                    // `$( ... )` command substitution: collect the balanced
                    // body for ordered execution; the child's own nounset
                    // check never leaks to this word (rubash#307).
                    let mut depth = 1usize;
                    let mut single = false;
                    let mut double = false;
                    let mut body = String::new();
                    while index < chars.len() {
                        let body_ch = chars[index];
                        index += 1;
                        if !single && body_ch == '\\' {
                            if index < chars.len() {
                                body.push(chars[index]);
                                index += 1;
                            }
                            continue;
                        }
                        if !double && body_ch == '\'' {
                            single = !single;
                        } else if !single && body_ch == '"' {
                            double = !double;
                        } else if !single && !double && body_ch == '(' {
                            depth += 1;
                        } else if !single && !double && body_ch == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        body.push(body_ch);
                    }
                    comsubs.push(body);
                }
                _ => {}
            }
        }
        (None, comsubs)
    }

    pub(in crate::executor) fn nounset_unbound_parameter(&self, word: &str) -> Option<String> {
        let mut chars = word.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == DATA_DOLLAR {
                continue;
            }
            if ch == '`' {
                // Backtick command substitution: same forked-child nounset
                // boundary as `$( ... )` (GNU subst.c:7143 command_substitute
                // handles both spellings; probe p5: `echo `echo $U`` prints
                // an empty substitution and the script continues). Skip the
                // whole body — backslash-escaped backticks included — so the
                // parent scan never fires on the child's variables
                // (rubash#307).
                while let Some(body_ch) = chars.next() {
                    if body_ch == '\\' {
                        chars.next();
                        continue;
                    }
                    if body_ch == '`' {
                        break;
                    }
                }
                continue;
            }
            if ch != '$' {
                continue;
            }

            match chars.peek().copied() {
                Some('{') => {
                    chars.next();
                    let mut name = String::new();
                    for name_ch in chars.by_ref() {
                        if name_ch == '}' {
                            break;
                        }
                        name.push(name_ch);
                    }
                    if let Some(reported) = self.nounset_braced_parameter_is_unbound(&name) {
                        return Some(reported);
                    }
                }
                Some(first) if first.is_ascii_digit() => {
                    chars.next();
                    let index = first.to_digit(10).unwrap_or(0) as usize;
                    if index > 0
                        && self
                            .shell_state
                            .positional_window()
                            .get(index - 1)
                            .is_none()
                    {
                        return Some(format!("${first}"));
                    }
                }
                Some(first) if is_shell_name_start(first) => {
                    let mut name = String::new();
                    while let Some(name_ch) = chars.peek().copied() {
                        if !is_shell_name_char(name_ch) {
                            break;
                        }
                        chars.next();
                        name.push(name_ch);
                    }
                    // Route through the braced check so the nounset test
                    // follows nameref chains to their final target
                    // (nameref25.sub: $r0 with r0->b unset is unbound).
                    if let Some(reported) = self.nounset_braced_parameter_is_unbound(&name) {
                        return Some(reported);
                    }
                }
                Some('?') | Some('$') | Some('@') | Some('*') | Some('#') | Some('-') => {
                    chars.next();
                }
                Some('!') => {
                    // GNU subst.c param_expand case '!' (10533-10550): a
                    // BARE `$!` with no asynchronous pid reports uerror =
                    // "$" + '!' — the diagnostic name carries the sigil
                    // (`$!: unbound variable`, rubash#290), exactly like
                    // the bare `$1`..`$9` arm above (uerror at 10489).
                    // The BRACED `${!}` form is different: it goes through
                    // parameter_brace_expand (10163-10170), whose
                    // err_unboundvar(name) has no `$` — that path is
                    // nounset_braced_parameter_is_unbound and stays "!"
                    // (posixexp1).
                    chars.next();
                    if self.shell_state.last_background_pid.is_none() {
                        return Some(String::from("$!"));
                    }
                }
                Some('(') => {
                    chars.next();
                    if chars.peek().copied() == Some('(') {
                        // `$(( ... ))` is arithmetic expansion, evaluated in
                        // the PARENT (no fork): an unbound name inside is a
                        // parent-side fatal error (GNU expr.c expr_streval;
                        // probe p3: `x=$(($U))' under set -u exits the
                        // script with 1), so keep scanning the arithmetic
                        // text for `$name' references.
                        continue;
                    }
                    // `$( ... )` command substitution: GNU forks
                    // (subst.c:7143 command_substitute -> the child's
                    // parse_and_execute does the nounset check), so an
                    // unbound variable in the body ends only the forked
                    // substitution — the parent word keeps expanding
                    // (rubash#307: `x=$(echo $U)' continues, assignment
                    // takes the comsub status). The parent scan must treat
                    // the whole span as a unit and never look inside — skip
                    // it balanced and quote-aware, like the byte-index
                    // comsub skipper in next_quoted_parameter_expansion_start.
                    let mut depth = 1usize;
                    let mut single = false;
                    let mut double = false;
                    while let Some(body_ch) = chars.next() {
                        match body_ch {
                            '\\' if !single => {
                                chars.next();
                            }
                            '\'' if !double => single = !single,
                            '"' if !single => double = !double,
                            '(' if !single && !double => depth += 1,
                            ')' if !single && !double => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some(_) | None => {}
            }
        }
        None
    }

    /// GNU subst.c check_unbound_variable / parameter_brace_expand: under
    /// nounset, decide whether the parameter inside `${...}` (or a bare
    /// `$name`) is unbound and, if so, which name GNU reports in the
    /// `unbound variable` diagnostic. An array-element reference evaluates
    /// its subscript arithmetically before the element check, so an unset
    /// name inside the subscript reports that name even when the array
    /// itself is unset (nameref25.sub ok 4 reports `k`, not `r` or `a[k]`).
    /// rubash#1168: for a `${subject<op>word}` fragment whose operator is
    /// `-`/`=`/`+` (either `:`-prefixed or bare), decide whether GNU
    /// would expand `word` at all, and if so scan it for unbound
    /// references with the ordinary word scanner. Branch conditions follow
    /// subst.c parameter_brace_expand check_nullness: the `:` forms test
    /// unset-or-null, the bare forms test unset only; `-`/`=` expand the
    /// word when the test FAILS the subject, `+` when it passes.
    fn nounset_operator_word_unbound(
        &self,
        brace_core: &str,
        operator_tail: &str,
    ) -> Option<String> {
        let (colon, op, word) = if let Some(rest) = operator_tail.strip_prefix(':') {
            (true, rest.as_bytes().first().copied()?, &rest[1..])
        } else {
            (
                false,
                operator_tail.as_bytes().first().copied()?,
                &operator_tail[1..],
            )
        };
        if !matches!(op, b'-' | b'=' | b'+') || word.is_empty() {
            return None;
        }
        let subject = &brace_core[..parameter_reference_len(brace_core)];
        let value = self.parameter_operator_value(subject);
        let word_expanded = match op {
            b'-' | b'=' => {
                if colon {
                    value.as_deref().map_or(true, |v| v.is_empty())
                } else {
                    value.is_none()
                }
            }
            _ => {
                if colon {
                    value.as_deref().map_or(false, |v| !v.is_empty())
                } else {
                    value.is_some()
                }
            }
        };
        if !word_expanded {
            return None;
        }
        self.nounset_unbound_parameter(word)
    }

    pub(in crate::executor) fn nounset_braced_parameter_is_unbound(
        &self,
        name: &str,
    ) -> Option<String> {
        if let Some(reported) = self.nounset_braced_head_is_unbound(name) {
            return Some(reported);
        }
        self.nounset_braced_operator_word_unbound(name)
    }

    /// GNU subst.c parameter_brace_expand: once the head reference's own
    /// unbound check (subst.c:10170-10180) passes, the words the expansion
    /// ALWAYS evaluates are expanded under the same nounset rule — the
    /// substring offset/length (subst.c:8395 verify_substring_values via
    /// expand_arith_string + param_expand), the patsub pattern and
    /// replacement (subst.c:9387 parameter_brace_patsub), the
    /// prefix/suffix removal patterns, and the casemod pattern. An unset
    /// `$name` inside one reports `name: unbound variable` exactly like a
    /// top-level reference (rubash#1149). The lazy default / assign /
    /// alternate / error words are NOT scanned here: GNU expands them
    /// only when the operator fires on the head's state, which the head
    /// check already adjudicated.
    fn nounset_braced_operator_word_unbound(&self, name: &str) -> Option<String> {
        // The `#` length form owns its tail elsewhere (its remainders are
        // the `$#` parameter's own operators or bad substitutions), and
        // the `!` indirect form's tails are adjudicated by the indir arms
        // of parameter_expansion_error_in_word_context.
        if name.starts_with(['#', '!']) {
            return None;
        }
        if let Some((_, pattern, replacement, _)) = parse_parameter_replacement(name) {
            if let Some(unbound) = self.nounset_unbound_parameter(pattern) {
                return Some(unbound);
            }
            if let Some(unbound) = self.nounset_unbound_parameter(replacement) {
                return Some(unbound);
            }
            return None;
        }
        if let Some((_, _, pattern)) = parse_parameter_case_mod(name) {
            return self.nounset_unbound_parameter(pattern);
        }
        // Single-char special parameters (`@`, `*`) are in OPERATOR_SET,
        // so parameter_reference_len stops at 0 for them; they are their
        // own one-char reference.
        let ref_len = parameter_reference_len(name);
        let ref_len = if ref_len == 0 {
            name.chars().next().map_or(0, |c| c.len_utf8())
        } else {
            ref_len
        };
        let tail = &name[ref_len..];
        if let Some(rest) = tail.strip_prefix(':') {
            if matches!(rest.chars().next(), Some('-' | '=' | '+' | '?')) {
                return None;
            }
            let (offset, length, has_length) =
                crate::executor::parameter_core::split_top_level_colon(rest);
            if let Some(unbound) = self.nounset_unbound_parameter(offset) {
                return Some(unbound);
            }
            if has_length {
                if let Some(unbound) = self.nounset_unbound_parameter(length) {
                    return Some(unbound);
                }
            }
            return None;
        }
        if let Some(pattern) = tail
            .strip_prefix("##")
            .or_else(|| tail.strip_prefix('#'))
            .or_else(|| tail.strip_prefix("%%"))
            .or_else(|| tail.strip_prefix('%'))
        {
            return self.nounset_unbound_parameter(pattern);
        }
        None
    }

    fn nounset_braced_head_is_unbound(&self, name: &str) -> Option<String> {
        if name == "!" {
            // GNU subst.c: the last-background-pid parameter is unset until a
            // background job runs; under nounset it reports an unbound
            // variable (posixexp1).
            return self
                .shell_state
                .last_background_pid
                .is_none()
                .then(|| String::from("!"));
        }

        // GNU subst.c parameter_brace_expand: an indexed-array element
        // reference `a[sub]` evaluates `sub` arithmetically before any
        // operator or element test, so under nounset an unset name inside
        // the subscript reports that name (nameref25.sub: `${a[k]}` and
        // `r->a[k]` with k unset both report `k: unbound variable`, even
        // behind a `:-` operator). Associative subscripts are literal keys
        // and are not evaluated.
        if let Some((abase, sub)) = split_leading_array_ref(name.strip_prefix('#').unwrap_or(name))
        {
            if !self.is_assoc_parameter_array(abase) {
                if let Some(unset) = self.nounset_subscript_unset_name(sub) {
                    return Some(unset);
                }
            }
        }

        // GNU subst.c parameter_brace_expand_indir under nounset:
        // `${!name}` / `${!name@T}` report `!name` when the indirection
        // TARGET is unbound — including a target array whose cell has no
        // elements (check_unbound_variable treats array_cell(v)==0 as
        // unbound for scalar forms; new-exp15 `-uc` runs). An unset
        // indirection source is "invalid indirect expansion" elsewhere,
        // not an unbound diagnostic, and `[@]`/`[*]` list forms (either on
        // the source or produced by the target) are never unbound.
        if let Some(indirect) = name.strip_prefix('!') {
            let base = indirect
                .strip_suffix("@a")
                .or_else(|| indirect.strip_suffix("@A"))
                .unwrap_or(indirect);
            if base.ends_with("[@]") || base.ends_with("[*]") {
                return None;
            }
            let resolved = self
                .resolved_variable_name(base)
                .unwrap_or_else(|| base.to_string());
            // GNU subst.c:7883-7920 parameter_brace_expand_indir + the
            // caller's unbound check at subst.c:10164-10175 — three ordered
            // outcomes for the indirection SOURCE:
            // 1. entry absent (find_variable_last_nameref == 0):
            //    "invalid indirect expansion" fires BEFORE any nounset
            //    check — defer to that scan (return None here);
            // 2. entry exists but declared-unset (bare declare/local):
            //    no indir error, the NULL value leaves the whole expansion
            //    unset, and the caller's err_unboundvar(name) reports the
            //    full `!base` under nounset (WSL 5.3.0: `set -u; declare x;
            //    echo ${!x}` -> `!x: unbound variable`, rubash#529);
            // 3. entry has a value: an invalid target value raises
            //    "invalid variable name" INSIDE parameter_brace_expand_indir
            //    (subst.c:7955-7961) before the caller's unbound check —
            //    defer to that scan too (WSL 5.3.0: `set -u; x=; ${!x}` and
            //    `set -u; x=1bad; ${!x}` both report the VALUE, not `!x`).
            match self.shell_state.env_vars.get(&resolved).cloned() {
                None => {
                    // Outcome 2 only for a declared-unset NON-nameref entry:
                    // a never-set name or an empty-cell / circular nameref
                    // keeps outcome 1 ("invalid indirect expansion" before
                    // any nounset check — variables.c:2070 returns NULL for
                    // an empty-cell nameref even though the entry exists).
                    let plain_entry = matches!(
                        self.nameref_resolution(&resolved),
                        NamerefResolution::NotNameref
                    ) && is_marked_var(
                        &self.shell_state.env_vars,
                        DECLARED_UNSET_VARS,
                        &resolved,
                    );
                    return plain_entry.then(|| format!("!{base}"));
                }
                Some(target) => {
                    if target.ends_with("[@]") || target.ends_with("[*]") {
                        return None;
                    }
                    if !Self::is_valid_indirect_target(&target) {
                        return None;
                    }
                    let bound = if let Some((tbase, tsub)) = parse_array_subscript(&target) {
                        if tsub == "@" || tsub == "*" {
                            true
                        } else if self.is_assoc_parameter_array(&tbase) {
                            let key = self.assoc_subscript_key(&tsub);
                            self.parameter_array_storage(&tbase)
                                .and_then(|storage| assoc_value_at(&storage, &key))
                                .is_some()
                        } else {
                            !self.nounset_indexed_element_absent(&tbase, &tsub)
                        }
                    } else {
                        self.nounset_variable_bound(&target)
                    };
                    return (!bound).then(|| format!("!{base}"));
                }
            }
        }

        // GNU subst.c:9807 parameter_brace_expand: the parameter reference
        // ends at the first `#%^,:-=?+/@}` operator character (string_extract
        // with SX_VARNAME, subst.c:791, skipping well-formed `[...]`
        // subscripts at subst.c:812-818). The unbound check at
        // subst.c:10170-10180 applies only to the substring / patsub /
        // casemod / attribute / `#` / `%` / bare forms — the default,
        // assign and alternate operators (`-`, `=`, `+`, the `:`-prefixed
        // versions) never report unbound, so bail only when the OPERATOR is
        // one of those. A `-`/`=`/`+` inside a pattern after `#`/`%`/`/` is
        // pattern text, not an operator (`${x#[a-]}` still reports x,
        // rubash#311).
        let brace_core = name.strip_prefix('#').unwrap_or(name);
        let operator_tail = &brace_core[parameter_reference_len(brace_core)..];
        let operator_head = operator_tail.as_bytes().first().copied();
        let default_value_operator = matches!(operator_head, Some(b'-') | Some(b'=') | Some(b'+'))
            || (operator_head == Some(b':')
                && matches!(
                    operator_tail.as_bytes().get(1),
                    Some(b'-') | Some(b'=') | Some(b'+') | Some(b'?')
                ));

        // GNU subst.c parameter_brace_expand: the default/assign/
        // alternate operators shield the SUBJECT from the unbound check,
        // but the operator's WORD is a fresh expansion context: when the
        // branch that expands the word is taken, an unbound reference
        // inside it (a bare `$u2` in `${u1:-$u2}`) reports exactly like a
        // top-level one (rubash#1168). A nested `${u2:-deep}` stays
        // shielded by its own operator — the recursive scan applies the
        // same rule per fragment. `:?` is excluded: its word is only
        // expanded on the error path itself.
        if default_value_operator {
            return self.nounset_operator_word_unbound(brace_core, operator_tail);
        }

        if name.is_empty()
            || matches!(name, "#" | "@" | "*" | "?" | "$" | "-" | "0")
            || name.starts_with('!')
            || parse_parameter_error_operator(name, self.posix_mode_enabled()).is_some()
        {
            return None;
        }

        // GNU subst.c parameter_brace_expand: under nounset the attribute
        // transforms ${name@a}/${name@A} report the target as unbound when it
        // has no value, even though the transforms themselves would expand
        // (new-exp15 `-uc` cases; check_unbound_variable precedes
        // string_transform). Run this before the generic `@` bail-out.
        if let Some(stripped) = name.strip_suffix("@a").or_else(|| name.strip_suffix("@A")) {
            // `${arr[@]@a}`/`${arr[*]@a}` expand the element list — like
            // `arr[@]` itself they are never unbound, even when the array
            // has no elements (GNU subst.c:8856 array_transform handles the
            // empty cell without a value check).
            if stripped.ends_with("[@]") || stripped.ends_with("[*]") {
                return None;
            }
            let target = stripped.strip_prefix('!').unwrap_or(stripped);
            let resolved = self
                .resolved_variable_name(target)
                .unwrap_or_else(|| target.to_string());
            return (!self.nounset_variable_bound(&resolved)).then(|| stripped.to_string());
        }

        // GNU subst.c:10170-10180: `c == '@'` (attribute/transform
        // operator) IS in the unbound-check list, so `${UNSET@Q}` and
        // `${UNSET@[Q]}` report `UNSET`. Only `@` inside the parameter
        // REFERENCE — `[@]`/`[*]` subscripts or @-bearing subscript
        // text — keeps the never-unbound bail (all-element forms are
        // excluded by the `(name[0] == '@' || name[0] == '*') &&
        // name[1] == 0` / all_element_arrayref guard at subst.c:10177).
        let reference_portion = &brace_core[..brace_core.len() - operator_tail.len()];
        if reference_portion.contains('@') {
            return None;
        }

        // GNU subst.c parameter_brace_expand / parameter_brace_expand_length:
        // value-consuming operators (pattern removal, case modification,
        // substitution, and the substring form) require the base parameter's
        // value, so under nounset an unset base reports an unbound variable
        // (posixexp1: the length form printed 0 and the pattern-removal forms
        // printed empty). Default/assign/alternate operators stay excluded
        // above; arrays keep their own semantics.
        let core = name.strip_prefix('#').unwrap_or(name);
        let base_len = core
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(core.len());
        let base = &core[..base_len];
        if base.len() < core.len() || core.len() != name.len() {
            // GNU subst.c:9935-9951 parameter_brace_expand: every unbound
            // diagnostic for the `${#...}` length form reports `name+1` —
            // the direct route calls err_unboundvar (name+1) (subst.c:9948)
            // and the scalar route rewrites `#name` to `$name` before
            // expanding (subst.c:8353-8357, reported by expand_word_internal
            // subst.c:11023-11027) — so the `#` never appears in the
            // message (`#UNSET` reports `UNSET`, `#1` reports `1`,
            // `#a[0]` reports `a[0]`; issue #200). `core` is `name`
            // unchanged when there is no leading `#`.
            if let Ok(index) = base.parse::<usize>() {
                // GNU err_unboundvar(name) carries the bare reference
                // (`1`, not `1#pat`).
                let reference = &core[..parameter_reference_len(core)];
                return (index > 0
                    && self
                        .shell_state
                        .positional_window()
                        .get(index - 1)
                        .is_none())
                .then(|| reference.to_string());
            }
            if is_shell_name(base) {
                // GNU subst.c:9807: the parameter reference ends at the
                // first `#%^,:-=?+/@}` operator. With an operator tail,
                // the unbound check at subst.c:10170-10180 runs on the
                // PRE-OPERATOR reference only: `${a[5]#z}` checks element
                // 5 and reports `a[5]`, and err_unboundvar(name) reports
                // the bare reference — `${UNSET#pat}` reports `UNSET`,
                // never the pattern text (rubash#311). Without a tail
                // the value/length forms below keep their routes.
                let reference_end = parameter_reference_len(core);
                if reference_end < core.len() {
                    let reference = &core[..reference_end];
                    if let Some((abase, sub)) = parse_array_subscript(reference) {
                        return self.nounset_array_element_unbound(abase, sub, reference);
                    }
                    return (!self.dynamic_parameter_is_set(base)
                        && !self.shell_state.env_vars.contains_key(base)
                        && std::env::var(base).is_err())
                    .then(|| reference.to_string());
                }
                // GNU subst.c: the VALUE form `${a[k]}` is unbound when the
                // element itself does not exist, reporting the full `a[k]`
                // reference (nameref25.sub ok 1 reports `a[k]: unbound
                // variable` for an empty array). The LENGTH form `${#a[k]}`
                // is owned by array_length_reference (subst.c:7488-7519)
                // instead: a real array never reports unbound there — unset
                // elements have length 0 (subst.c:7538-7539 and 7571-7574;
                // `${#a[5]}` with a=(x) and `${#h[missing]}` with a set
                // assoc both print `0` under nounset, probes 2026-09-27) —
                // while an absent, declared-unset, or scalar base IS unbound
                // (subst.c:7502-7508) via the fatal INTMAX_MIN route
                // (subst.c:9945-9951, e.g. `b[0]: unbound variable` for a
                // set scalar under nounset).
                if let Some((abase, sub)) = parse_array_subscript(core) {
                    if core.len() == name.len() {
                        return self.nounset_array_element_unbound(abase, sub, core);
                    }
                    if sub == "@" || sub == "*" {
                        // The `@`/`*` subscript takes array_length_reference's
                        // non-fatal ksh93 branch (subst.c:7510-7516), which
                        // expand_braced_indexed_parameter reports with the
                        // expansion-error rails; never fatal from here.
                        return None;
                    }
                    return self.nounset_length_reference_unbound(abase, core);
                }
                return (!self.dynamic_parameter_is_set(base)
                    && !self.shell_state.env_vars.contains_key(base)
                    && std::env::var(base).is_err())
                .then(|| core.to_string());
            }
            return None;
        }

        if let Ok(index) = name.parse::<usize>() {
            return (index > 0
                && self
                    .shell_state
                    .positional_window()
                    .get(index - 1)
                    .is_none())
            .then(|| name.to_string());
        }

        if is_shell_name(name) {
            // GNU subst.c check_unbound_variable: under nounset a nameref
            // is tested through to its final target, so a reference to an
            // unset variable (or to an absent array element) is unbound
            // (nameref25.sub ok 2-4); @/* cells and invalid cells are not.
            if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
                let cell = self
                    .shell_state
                    .env_vars
                    .get(name)
                    .cloned()
                    .unwrap_or_default();
                if cell.ends_with("[@]") || cell.ends_with("[*]") {
                    return None;
                }
                if let Some((base, key)) = parse_array_subscript(&cell) {
                    if self.is_assoc_parameter_array(base) {
                        let assoc_key = self.assoc_subscript_key(key);
                        return self
                            .parameter_array_storage(base)
                            .and_then(|storage| assoc_value_at(&storage, &assoc_key))
                            .is_none()
                            .then(|| name.to_string());
                    }
                    // GNU evaluates the cell's subscript first; an unset
                    // subscript name reports itself rather than the nameref
                    // (nameref25.sub ok 4 reports `k: unbound variable`).
                    if let Some(unset) = self.nounset_subscript_unset_name(key) {
                        return Some(unset);
                    }
                    return self
                        .nounset_indexed_element_absent(base, key)
                        .then(|| name.to_string());
                }
                if is_shell_name(&cell) {
                    let target = self
                        .resolved_variable_name(&cell)
                        .unwrap_or_else(|| cell.clone());
                    return (!self.dynamic_parameter_is_set(&target)
                        && !self.shell_state.env_vars.contains_key(&target)
                        && std::env::var(&target).is_err())
                    .then(|| name.to_string());
                }
                return None;
            }
            return (!self.nounset_variable_bound(name)).then(|| name.to_string());
        }

        None
    }

    /// GNU subst.c check_unbound_variable: a variable is unbound under
    /// nounset when it has no value — and an array/assoc cell with zero
    /// elements counts as unset for scalar-form expansions (new-exp15
    /// `-uc`: `${foo}` / `${foo@a}` on `declare -a foo=()` report
    /// `foo: unbound variable`, while `${foo[@]}` stays bound).
    fn nounset_variable_bound(&self, name: &str) -> bool {
        if self.dynamic_parameter_is_set(name) {
            return true;
        }
        // GNU variables.c bind: att_invisible (rubash DECLARED_UNSET_VARS)
        // means the cell carries no value — the name is unbound even when a
        // stale typed-mirror cell or an inherited process-env residue
        // exists (the same invariant the scalar value read applies at
        // variable_state.rs — rubash#409; a `local NAME` declared-unset
        // must report `NAME: unbound variable`, rubash#970). The
        // contains_key conjunct keeps a leftover mark from masking a real
        // binding the way a re-assignment leaves it.
        if is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, name)
            && !self.shell_state.env_vars.contains_key(name)
        {
            return false;
        }
        // GNU variables.c find_variable returns the CELL, and the scalar
        // form reads element/key "0" of it:
        //   subst.c:7787-7790 parameter_brace_expand_word()
        //     assoc_p (var) -> assoc_reference (assoc_cell (var), "0")
        //     array_p (var) -> array_reference (array_cell (var), 0)
        //   array.c:657 array_reference() / assoc.c:120 assoc_reference()
        //     return NULL when that element is absent
        //   subst.c:10162-10169 parameter_brace_expand()
        //     var_is_set == 0 -> err_unboundvar (error.c:448)
        // So the scalar form is bound EXACTLY when element 0 exists: a
        // non-empty array holding no element 0 (`a=([3]=x)`, or `(x y)` after
        // `unset "a[0]"`) is unbound for `${a}`, while `${a[@]}` stays bound
        // because all_element_arrayref exempts the `@`/`*` forms from that
        // check. The previous rule ("the cell holds any element") was wrong
        // in both directions. A scalar whose text merely looks like an array
        // (x='()', rubash #311: liquidprompt tools/config-from-doc.sh) is
        // still bound — it is a scalar cell, not an array one.
        //
        // For arrays and assoc arrays the MIRROR decides, through the same
        // helpers the value read uses (variable_state.rs:462-470
        // scalar_parameter_value -> assoc_value_at(storage, "0") /
        // array_value_at(storage, 0)), so boundness and the value it guards
        // agree by construction. The typed cell is consulted for scalars and
        // for unmarked arrays only: the ASSOC cell has no writer-side sync
        // at all — only declare/printf create it, and every later mirror
        // write (`m[k]=v`, `unset "m[k]"`, `m=(...)`, `printf -v m[k]`, …)
        // leaves it stale (rubash#618) — so trusting it let a stale cell
        // print a value while calling the name bound, or abort a working
        // script as unbound. Both parses are content-addressed memos
        // (arrays/storage.rs:64, assignment_helpers.rs:491) and the value
        // read performs the same one, so this costs a cache hit.
        let assoc_marked = is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name);
        let array_marked = is_marked_array_var(&self.shell_state.env_vars, name);
        if assoc_marked || array_marked {
            return self.array_scalar_form_is_bound(name, assoc_marked);
        }
        if let Some(variable) = self.shell_state.variables.get(name) {
            return match &variable.value {
                crate::shell::ShellValue::Scalar(_) => true,
                crate::shell::ShellValue::IndexedArray(entries) => entries.contains_key(&0),
                crate::shell::ShellValue::AssociativeArray(entries) => entries.contains_key("0"),
            };
        }
        match self.shell_state.env_vars.get(name) {
            Some(value) => {
                if is_array_storage(value) {
                    let resolved = self
                        .resolved_variable_name(name)
                        .unwrap_or_else(|| name.to_string());
                    return self
                        .parameter_array_storage(&resolved)
                        .map(|storage| {
                            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) {
                                assoc_value_at(&storage, "0").is_some()
                            } else {
                                array_value_at(&storage, 0).is_some()
                            }
                        })
                        .unwrap_or(false);
                }
                true
            }
            None => {
                !array_marked && crate::executor::env_helpers::exact_case_env_var(name).is_some()
            }
        }
    }

    /// Boundness of the scalar form for an array/assoc-marked name: element
    /// 0 (array.c:657) or key "0" (assoc.c:120), read from the mirror — the
    /// same representation the value read consults, so the two cannot
    /// disagree (see nounset_variable_bound).
    fn array_scalar_form_is_bound(&self, name: &str, assoc_marked: bool) -> bool {
        let resolved = self
            .resolved_variable_name(name)
            .unwrap_or_else(|| name.to_string());
        let assoc =
            assoc_marked || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved);
        self.parameter_array_storage(&resolved)
            .map(|storage| {
                if assoc {
                    assoc_value_at(&storage, "0").is_some()
                } else {
                    array_value_at(&storage, 0).is_some()
                }
            })
            .unwrap_or(false)
    }

    /// GNU subst.c check_unbound_variable: whether an `a[sub]` element
    /// reference is unbound -- `[@]`/`[*]` are never unbound, an absent
    /// element reports the whole `a[k]` reference text.
    /// GNU subst.c:7502-7508 array_length_reference: in the `${#base[sub]}`
    /// length form, an absent (var == 0), declared-unset (att_invisible ->
    /// DECLARED_UNSET_VARS), or non-array (assoc_p == 0 && array_p == 0,
    /// i.e. a plain scalar) base under nounset returns INTMAX_MIN, which the
    /// caller (subst.c:9945-9951 parameter_brace_expand) turns into a fatal
    /// `base[sub]: unbound variable`. A real array/assoc base never reaches
    /// that branch: unset elements report length 0 (subst.c:7538-7539).
    fn nounset_length_reference_unbound(&self, base: &str, reported: &str) -> Option<String> {
        let storage = self.parameter_array_storage(base);
        let resolved = self
            .resolved_variable_name(base)
            .unwrap_or_else(|| base.to_string());
        let is_array = storage.as_deref().is_some_and(is_array_storage)
            || is_marked_array_var(&self.shell_state.env_vars, &resolved);
        // FUNCNAME is att_invisible outside functions (variables.c:1812),
        // so it is in the unbound class at function_depth == 0 here too.
        let unbound = storage.is_none()
            || (base == "FUNCNAME" && self.shell_state.function_depth == 0)
            || is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, &resolved)
            || !is_array;
        unbound.then(|| reported.to_string())
    }

    /// Build the printed (name, message) pair for a nounset fatal.
    /// GNU parameter_brace_expand evaluates the subscript before
    /// check_unbound_variable, so an indexed reference whose negative
    /// subscript lands before the array start first draws
    /// `base: bad array subscript` and only then the fatal
    /// `base[sub]: unbound variable` (rubash#1170). The nounset scan is
    /// side-effect-free (it can run speculatively), so the earlier line
    /// rides inside the message of the single diagnostic the caller
    /// prints: every print site formats `{prefix}{name}: {message}`,
    /// which reproduces the two GNU lines byte-for-byte. Positive-index
    /// and associative references are valid references and keep the
    /// plain single-line form.
    fn nounset_unbound_diagnostic(&self, reported: String) -> (String, String) {
        if let Some((base, sub)) = parse_array_subscript(&reported) {
            if !self.is_assoc_parameter_array(base) {
                let index = self.eval_integer_assignment_value(sub);
                // A scalar/unset base has only element 0, so every
                // negative subscript is an invalid reference; an array
                // base is invalid only when the element is absent (the
                // subscript lands before the array start).
                let invalid = index < 0
                    && (!self.parameter_is_array_variable(base)
                        || self.nounset_indexed_element_absent(base, sub));
                if invalid {
                    return (
                        base.to_string(),
                        format!(
                            "bad array subscript\n{}{reported}: unbound variable",
                            self.diagnostic_prefix()
                        ),
                    );
                }
            }
        }
        (reported, "unbound variable".to_string())
    }

    fn nounset_array_element_unbound(
        &self,
        base: &str,
        sub: &str,
        reported: &str,
    ) -> Option<String> {
        if sub == "@" || sub == "*" {
            return None;
        }
        if self.is_assoc_parameter_array(base) {
            let key = self.assoc_subscript_key(sub);
            return self
                .parameter_array_storage(base)
                .and_then(|storage| assoc_value_at(&storage, &key))
                .is_none()
                .then(|| reported.to_string());
        }
        // Scalar (or unset) base: only element 0 exists, and only when the
        // variable itself is set. Array storage consults the element table.
        match self.shell_state.env_vars.get(base) {
            Some(storage)
                if storage.starts_with(STORAGE_WORD_PREFIX) || storage.starts_with('(') =>
            {
                self.nounset_indexed_element_absent(base, sub)
                    .then(|| reported.to_string())
            }
            Some(_) => (self.eval_integer_assignment_value(sub) != 0).then(|| reported.to_string()),
            None => (!self.dynamic_parameter_is_set(base) && std::env::var(base).is_err()
                || self.eval_integer_assignment_value(sub) != 0)
                .then(|| reported.to_string()),
        }
    }

    /// GNU arrayfunc.c valid_array_reference + variables.c array_value:
    /// element existence for an indexed-array `a[sub]` reference. `sub` has
    /// already been screened for unset names by the caller.
    fn nounset_indexed_element_absent(&self, base: &str, sub: &str) -> bool {
        let index = self.eval_integer_assignment_value(sub);
        self.shell_state
            .env_vars
            .get(base)
            .and_then(|storage| {
                resolve_indexed_array_subscript(storage, index)
                    .and_then(|i| array_value_at(storage, i))
            })
            .is_none()
    }

    /// GNU arith.c: while evaluating an indexed-array subscript under
    /// nounset, the first unset variable name in the expression is the one
    /// reported as unbound (nameref25.sub ok 4).
    fn nounset_subscript_unset_name(&self, subscript: &str) -> Option<String> {
        if subscript.contains('"') || subscript.contains('\'') {
            return None;
        }
        let mut chars = subscript.chars().peekable();
        while let Some(ch) = chars.next() {
            if !is_shell_name_start(ch) {
                continue;
            }
            let mut token = String::from(ch);
            while let Some(&next) = chars.peek() {
                if !is_shell_name_char(next) {
                    break;
                }
                token.push(next);
                chars.next();
            }
            if !self.dynamic_parameter_is_set(&token)
                && !self.shell_state.env_vars.contains_key(&token)
                && std::env::var(&token).is_err()
            {
                return Some(token);
            }
        }
        None
    }

    pub(in crate::executor) fn parameter_error_value(&self, name: &str) -> Option<String> {
        // GNU subst.c:7786 (parameter_brace_expand_word, PF_ALLINDS) +
        // subst.c:10142 (var_is_set/var_is_null): for `${arr[@]}` /
        // `${arr[*]}` the value an operator tests is the whole-array join
        // — NULL only when the array has NO elements, never merely
        // because an all-element subscript is not an element reference.
        // parameter_operator_value implements exactly that rule for
        // `:-`/`:=`; route the error-operator scan through it too, or
        // `${a[@]:?}` on a non-empty array misreads as unset
        // (array_element_parameter_value returns None for `[@]`/`[*]`)
        // and aborts the script (rubash#1174).
        if name.ends_with("[@]") || name.ends_with("[*]") {
            return self.parameter_operator_value(name);
        }
        match name {
            "#" => Some(self.shell_state.positional_window().len().to_string()),
            // GNU chk_atstar (subst.c): `$@`/`$*` are UNSET with zero
            // positional parameters, so `-` substitutes the default word
            // (more-exp.tests `${*-x}` with no args prints x) instead of
            // keeping a null value.
            "@" | "*" if self.shell_state.positional_window().is_empty() => None,
            // GNU string_list_dollar_star: `*` joins with IFS[0]; `@` space.
            "@" => Some(self.shell_state.positional_window().join(" ")),
            "*" => Some(self.positional_params_star_joined()),
            "?" => Some(self.dollar_question_status().to_string()),
            "$" => Some(self.shell_pid_value().to_string()),
            // `$!` is unset until the first background job (variables.c), so
            // `${!-ok 27}` substitutes `ok 27` and `${@-x}`/`${*-x}` with no
            // positionals substitute their default word.
            "!" if self.shell_state.last_background_pid.is_none() => None,
            "!" => Some(self.last_background_pid_value()),
            "-" => Some(self.shell_option_flags()),
            "0" => Some(self.script_name_value()),
            _ => {
                if let Some(value) = self.dynamic_parameter_value(name) {
                    return Some(value);
                }
                if let Ok(index) = name.parse::<usize>() {
                    return self
                        .shell_state
                        .positional_window()
                        .get(index.saturating_sub(1))
                        .cloned();
                }
                if let Some(value) = self.array_element_parameter_value(name) {
                    return Some(value);
                }
                // GNU var_is_set/var_is_null on a BARE array name read
                // element 0 (variables.c get_string_value; assoc: key
                // "0"): an assoc without key "0", or a sparse indexed
                // array with no element 0, is NULL for the error
                // operators even though other elements exist. The raw
                // storage string is non-empty for any non-empty array,
                // so reading it directly made `${m:?}`/`${a:?}` on such
                // arrays silently pass (rubash#1597).
                match self.shell_state.env_vars.get(name) {
                    Some(value) if self.parameter_is_array_variable(name) => {
                        self.scalar_parameter_value(name, value)
                    }
                    value => value.cloned(),
                }
            }
        }
    }
}
