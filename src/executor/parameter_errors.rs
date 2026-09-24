use super::*;
use crate::executor::parameter_core::word_contains_current_shell_command_substitution;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

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
        let word = word
            .strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX)
            .or_else(|| word.strip_prefix(STORAGE_WORD_PREFIX))
            .unwrap_or(word);
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
            "@" | "*" => !self.shell_state.positional_params.is_empty(),
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
            let values = self
                .parameter_array_storage(array_name)
                .map(|storage| array_values(&storage))
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
            return match self.shell_state.positional_params.get(index.saturating_sub(1)) {
                Some(target_name) => Some(self.parameter_operator_value(target_name)),
                None => Some(None),
            };
        }
        let target_expr = self.shell_state.env_vars.get(indirect_name)?;
        if target_expr.ends_with("[@]") || target_expr.ends_with("[*]") {
            let values = self.indirect_target_values(target_expr);
            if values.is_empty() {
                return Some(None);
            }
            return Some(Some(self.join_expanded_array_values(values, target_expr)));
        }

        Some(self.indirect_target_values(target_expr).into_iter().next())
    }

    fn is_valid_length_parameter_name(name: &str) -> bool {
        if name.is_empty() {
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

    pub(in crate::executor) fn parameter_expansion_error(
        &self,
        cmd: &CommandNode,
    ) -> Option<(String, String, i32)> {
        for word in &cmd.words {
            if let Some(error) = self.parameter_expansion_error_in_word(word) {
                return Some(error);
            }
        }
        for value in cmd.assignment_values() {
            if let Some(error) = self.parameter_expansion_error_in_word(value) {
                return Some(error);
            }
        }

        None
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

    /// GNU subst.c:8221 parameter_brace_expand_error →
    /// set_exit_status(EXECUTION_FAILURE): a fatal `${var?msg}` / nounset
    /// expansion error reports status 1 in script mode. In `-c` mode
    /// shell.c:1471 run_one_command maps FORCE_EOF to 127.
    pub(in crate::executor) fn expansion_fatal_status(&self) -> i32 {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_IS_C")
        {
            127
        } else {
            1
        }
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
        let word = word
            .strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX)
            .or_else(|| word.strip_prefix(STORAGE_WORD_PREFIX))
            .unwrap_or(word);
        // Preserve Rubash's nested current-shell extension; its `${| ... }`
        // marker disambiguates the legacy enclosing form from this Bash error.
        if word.contains("${|") {
            return None;
        }
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            if let Some(name) = self.nounset_unbound_parameter(word) {
                // expr.c expr_streval raises FORCE_EOF for unbound vars
                // under set -u — fatal like `${x?}` (see command_execute).
                return Some((
                    name,
                    "unbound variable".to_string(),
                    Self::FATAL_PARAMETER_EXPANSION_STATUS,
                ));
            }
        }
        let mut rest = word;
        while let Some(start) = if quote_aware {
            next_quoted_parameter_expansion_start(rest)
        } else {
            rest.find("${")
        } {
            let after_start = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(after_start) else {
                return Some((
                    "${".to_string() + after_start + "}",
                    "unexpected EOF while looking for matching `}'".to_string(),
                    2,
                ));
            };
            let inner = &after_start[..end];
            // GNU rejects a parameter expansion nested inside another
            // parameter expansion inside an array subscript. Reject it
            // before the mutable whole-word expander can recurse indefinitely;
            // a single nested subscript such as `${A[${i}]}` remains valid.
            if inner.contains("[${${") {
                return Some((format!("${{{inner}}}"), "bad substitution".to_string(), 1));
            }
            // GNU subst.c:10053 valid_brace_expansion_word: after ${, the
            // parameter name must be a valid name/special-var/subscripted
            // form. `${$(...` parses as special var $ followed by a (`
            // token, which is not a valid operator tail — bad substitution
            // (new-exp.tests `${c//${$(($#-1))}/x/}`).
            if inner.starts_with("$(") || inner.contains("${$(") {
                return Some((format!("${{{inner}}}"), "bad substitution".to_string(), 1));
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
                        format!("${{{inner}}}"),
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
                    return Some((format!("${{{inner}}}"), "bad substitution".to_string(), 1));
                }
            }
            // Indirect expansions accept a simple parameter reference, an
            // array subscript form, or the prefix@ variable-name listing
            // form; anything else (e.g. a stray trailing '!' in ${!bad!})
            // is a bad substitution in GNU.
            if let Some(indirect) = self.indirect_parameter_body(inner) {
                if !Self::is_valid_indirect_expression(indirect) {
                    return Some((format!("${{{inner}}}"), "bad substitution".to_string(), 1));
                }
                // Value-dependent GNU diagnostics for the plain indirect
                // form ${!name} (subst.c parameter_brace_expand_indir): an
                // unset name reports "invalid indirect expansion" and aborts
                // the command; a set name whose value is not a valid
                // indirect target reports "invalid variable name". A set
                // name whose value is a valid identifier expands normally -
                // even when the target itself is unset (GNU probe:
                // x=validname with validname unset expands empty).
                if !indirect.is_empty() && is_shell_name(indirect) {
                    match self.shell_state.env_vars.get(indirect) {
                        None => {
                            return Some((
                                indirect.to_string(),
                                "invalid indirect expansion".to_string(),
                                1,
                            ));
                        }
                        Some(value) => {
                            if !Self::is_valid_indirect_target(value) {
                                return Some((
                                    value.clone(),
                                    "invalid variable name".to_string(),
                                    1,
                                ));
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
                    if head_end > 0
                        && is_shell_name(head)
                        && tail.starts_with('[')
                        && tail.ends_with(']')
                    {
                        let base_exists = match self.nameref_resolution(head) {
                            NamerefResolution::Target(target) => {
                                let base = target.split('[').next().unwrap_or(target.as_str());
                                self.shell_state.env_vars.contains_key(base)
                                    || self.shell_state.variables.get(base).is_some()
                            }
                            NamerefResolution::NotNameref => {
                                self.shell_state.env_vars.contains_key(head)
                                    || self.shell_state.variables.get(head).is_some()
                            }
                            // Unresolved (empty/unresolvable cell), Circular,
                            // and MaxDepth all mean find_variable returned
                            // NULL for the base.
                            _ => false,
                        };
                        if !base_exists {
                            return Some((
                                indirect.to_string(),
                                "invalid indirect expansion".to_string(),
                                1,
                            ));
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
                        if let Some(value) = self.shell_state.env_vars.get(base) {
                            if !Self::is_valid_indirect_target(value) {
                                // GNU reports the dollar_at join of the
                                // variable's values (array_value with
                                // AV_ALLOWALL, arrayfunc.c:1563), not the
                                // raw storage text.
                                let display =
                                    if let Some(storage) = self.parameter_array_storage(base) {
                                        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base) {
                                            assoc_hash_ordered_values(
                                                &storage,
                                                assoc_nbuckets(&self.shell_state.env_vars, base),
                                            )
                                            .join(" ")
                                        } else {
                                            array_values(&storage).join(" ")
                                        }
                                    } else if is_array_storage(value) {
                                        array_values(value).join(" ")
                                    } else {
                                        value.clone()
                                    };
                                return Some((display, "invalid variable name".to_string(), 1));
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
                return Some((format!("${{{inner}}}"), "bad substitution".to_string(), 1));
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
                    // GNU subst.c:8482: for VT_POSPARMS (@/*) and VT_ARRAYVAR
                    // (array[@]/array[*]), a negative length is unconditionally
                    // an error. For scalar variables, the length is adjusted
                    // by the string length and only errors if still negative.
                    let is_pospar_or_array = matches!(name, "@" | "*")
                        || name
                            .strip_suffix("[@]")
                            .or_else(|| name.strip_suffix("[*]"))
                            .is_some();
                    let is_invalid = is_pospar_or_array
                        || self.parameter_error_value(name).is_some_and(|value| {
                            parameter_substring_has_negative_result(
                                value.chars().count(),
                                offset,
                                length,
                            )
                        });
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

    pub(in crate::executor) fn nounset_unbound_parameter(&self, word: &str) -> Option<String> {
        let mut chars = word.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == DATA_DOLLAR {
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
                    if index > 0 && self.shell_state.positional_params.get(index - 1).is_none() {
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
                    // GNU subst.c: the last-background-pid parameter is unset
                    // until a background job runs and errors under nounset
                    // (posixexp1).
                    chars.next();
                    if self.shell_state.last_background_pid.is_none() {
                        return Some(String::from("!"));
                    }
                }
                Some('(') => {
                    chars.next();
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
    pub(in crate::executor) fn nounset_braced_parameter_is_unbound(
        &self,
        name: &str,
    ) -> Option<String> {
        if name == "!" {
            // GNU subst.c: the last-background-pid parameter is unset until a
            // background job runs; under nounset it reports an unbound
            // variable (posixexp1).
            return self
                .shell_state.last_background_pid
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
            let Some(target) = self.shell_state.env_vars.get(&resolved).cloned() else {
                return None;
            };
            if target.ends_with("[@]") || target.ends_with("[*]") {
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

        if name.is_empty()
            || matches!(name, "#" | "@" | "*" | "?" | "$" | "-" | "0")
            || name.starts_with('!')
            || parse_parameter_error_operator(name, self.posix_mode_enabled()).is_some()
            || name.contains(":-")
            || name.contains(":=")
            || name.contains(":+")
            || name.contains('-')
            || name.contains('=')
            || name.contains('+')
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
            return (!self.nounset_variable_bound(&resolved))
                .then(|| stripped.to_string());
        }

        if name.contains('@') {
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
            if let Ok(index) = base.parse::<usize>() {
                return (index > 0 && self.shell_state.positional_params.get(index - 1).is_none())
                    .then(|| name.to_string());
            }
            if is_shell_name(base) {
                // GNU subst.c: `${a[k]}` (and the `#a[k]` length form) is
                // unbound when the element itself does not exist, reporting
                // the full `a[k]` reference (nameref25.sub ok 1 reports
                // `a[k]: unbound variable` for an empty array).
                if let Some((abase, sub)) = parse_array_subscript(core) {
                    return self.nounset_array_element_unbound(abase, sub, name);
                }
                return (!self.dynamic_parameter_is_set(base)
                    && !self.shell_state.env_vars.contains_key(base)
                    && std::env::var(base).is_err())
                .then(|| name.to_string());
            }
            return None;
        }

        if let Ok(index) = name.parse::<usize>() {
            return (index > 0 && self.shell_state.positional_params.get(index - 1).is_none())
                .then(|| name.to_string());
        }

        if is_shell_name(name) {
            // GNU subst.c check_unbound_variable: under nounset a nameref
            // is tested through to its final target, so a reference to an
            // unset variable (or to an absent array element) is unbound
            // (nameref25.sub ok 2-4); @/* cells and invalid cells are not.
            if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
                let cell = self.shell_state.env_vars.get(name).cloned().unwrap_or_default();
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
        let has_array_marker = is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
            || is_marked_array_var(&self.shell_state.env_vars, name);
        match self.shell_state.env_vars.get(name) {
            Some(value) => {
                if has_array_marker || is_array_storage(value) {
                    let resolved =
                        self.resolved_variable_name(name).unwrap_or_else(|| name.to_string());
                    return self
                        .parameter_array_storage(&resolved)
                        .map(|storage| {
                            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) {
                                !assoc_hash_ordered_values(
                                    &storage,
                                    assoc_nbuckets(&self.shell_state.env_vars, &resolved),
                                )
                                .is_empty()
                            } else {
                                !indexed_array_entries(&storage).is_empty()
                            }
                        })
                        .unwrap_or(false);
                }
                true
            }
            None => !has_array_marker && std::env::var(name).is_ok(),
        }
    }

    /// GNU subst.c check_unbound_variable: whether an `a[sub]` element
    /// reference is unbound -- `[@]`/`[*]` are never unbound, an absent
    /// element reports the whole `a[k]` reference text.
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
            Some(storage) if storage.starts_with(STORAGE_WORD_PREFIX) || storage.starts_with('(') => self
                .nounset_indexed_element_absent(base, sub)
                .then(|| reported.to_string()),
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
        self.shell_state.env_vars
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
        match name {
            "#" => Some(self.shell_state.positional_params.len().to_string()),
            // GNU chk_atstar (subst.c): `$@`/`$*` are UNSET with zero
            // positional parameters, so `-` substitutes the default word
            // (more-exp.tests `${*-x}` with no args prints x) instead of
            // keeping a null value.
            "@" | "*" if self.shell_state.positional_params.is_empty() => None,
            // GNU string_list_dollar_star: `*` joins with IFS[0]; `@` space.
            "@" => Some(self.shell_state.positional_params.join(" ")),
            "*" => Some(self.positional_params_star_joined()),
            "?" => Some(self.exit_code.to_string()),
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
                    return self.shell_state.positional_params.get(index.saturating_sub(1)).cloned();
                }
                if let Some(value) = self.array_element_parameter_value(name) {
                    return Some(value);
                }
                self.shell_state.env_vars.get(name).cloned()
            }
        }
    }
}
