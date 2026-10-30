use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

/// The RUBASH_TRACE_APPAE debug switch is fixed for the process lifetime,
/// so read it once (same contract as trace_stdio_write's cached switch): a
/// raw std::env::var_os per `${...}` walker tail took the std::env lock and
/// scanned the environment on every parameter expansion in every loop
/// (rubash#437 measured the same pattern at 122ns/call missing / 1.2us
/// present on Windows).
fn apfae_trace_enabled() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("RUBASH_TRACE_APPAE").is_some())
}

thread_local! {
    /// Depth pool of synthetic `${name}` words for
    /// [`Executor::expand_braced_parameter_fragment_in_word`]. One buffer is
    /// live per nesting level, so sibling fragments reuse the same allocation
    /// and a nested fragment only allocates while an outer buffer is checked
    /// out. See that function for the GNU anchor.
    static SYNTHETIC_FRAGMENT_WORD: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Check out a synthetic-`${...}` buffer from the depth pool (see above).
/// The caller fills it, expands through it, and returns it via
/// [`recycle_synthetic_fragment_word`]; buffers are returned to the pool on
/// every path so a nested fragment only allocates while an outer buffer is
/// checked out.
pub(in crate::executor) fn pop_synthetic_fragment_word() -> String {
    SYNTHETIC_FRAGMENT_WORD
        .with(|pool| pool.borrow_mut().pop())
        .unwrap_or_default()
}

/// Return a buffer checked out via [`pop_synthetic_fragment_word`].
pub(in crate::executor) fn recycle_synthetic_fragment_word(word: String) {
    SYNTHETIC_FRAGMENT_WORD.with(|pool| pool.borrow_mut().push(word));
}

/// The EAGER secondary texts of a `${...}` body (rubash#1588): the
/// spans GNU expands while computing the fragment's own value, before
/// and regardless of the operator outcome — the leading name's
/// subscript, a substring's offset and length texts, and the pattern
/// (and replacement) texts of the trim / patsub / case-modification
/// operators. An operator's WORD (`:-`, `=`, `:?`, ...) is NOT eager:
/// it expands only when the operator selects it (rubash#1564), so it
/// is deliberately absent here. Extraction mirrors the value side's
/// own parsers (split_top_level_colon, parse_parameter_replacement,
/// parse_parameter_case_mod) so the apply view cannot drift from the
/// value view.
fn secondary_assignment_texts(inner: &str) -> Vec<&str> {
    let mut texts = Vec::new();
    // The leading name ends at the earliest top-level operator: every
    // operator split is tried and the shortest left-hand side wins
    // (string_extract, subst.c:9805, stops at the first operator
    // character outside a subscript).
    fn shorter<'a>(current: &'a str, candidate: &'a str) -> &'a str {
        if candidate.len() < current.len() {
            candidate
        } else {
            current
        }
    }
    let mut name = inner;
    for op in [":-", ":+", ":=", ":?", "##", "%%"] {
        if let Some((lhs, _)) =
            super::expand_braced_ops::split_once_outside_subscript_str(inner, op)
        {
            name = shorter(name, lhs);
        }
    }
    for op in ['-', '+', '=', '?', '#', '%'] {
        if let Some((lhs, _)) = super::expand_braced_ops::split_once_outside_subscript(inner, op) {
            name = shorter(name, lhs);
        }
    }
    if let Some((lhs, _, _, _)) = super::parameter_ops::parse_parameter_replacement(inner) {
        name = shorter(name, lhs);
    }
    if let Some((lhs, _, _)) = super::parameter_case::parse_parameter_case_mod(inner) {
        name = shorter(name, lhs);
    }
    let (colon_name, colon_rest, has_colon) = super::parameter_core::split_top_level_colon(inner);
    if has_colon {
        name = shorter(name, colon_name.trim_end());
    }
    // Subscript of the leading name (a `#` length prefix or `!`
    // indirect prefix may precede the name proper).
    for candidate in [
        name,
        name.strip_prefix('#').unwrap_or(name),
        name.strip_prefix('!').unwrap_or(name),
    ] {
        if let Some((_, subscript)) = parse_array_subscript(candidate) {
            texts.push(subscript);
            break;
        }
    }
    let rest = &inner[name.len()..];
    if rest.starts_with('/') {
        // Patsub: pattern and replacement are both eager.
        if let Some((_, pattern, replacement, _)) =
            super::parameter_ops::parse_parameter_replacement(inner)
        {
            texts.push(pattern);
            texts.push(replacement);
        }
    } else if rest.starts_with(['^', ',', '~']) {
        if let Some((_, _, pattern)) = super::parameter_case::parse_parameter_case_mod(inner) {
            texts.push(pattern);
        }
    } else if rest.starts_with("##") || rest.starts_with("%%") {
        texts.push(&rest[2..]);
    } else if rest.starts_with(['#', '%']) {
        texts.push(&rest[1..]);
    } else if has_colon
        && !matches!(colon_rest.chars().next(), Some('=' | '+' | '?'))
        && !colon_rest.starts_with('-')
        && !colon_name.trim_end().is_empty()
        && !colon_name
            .trim_end()
            .chars()
            .any(|c| matches!(c, '/' | '%' | '^' | ',' | '~'))
        && crate::executor::parameter_ops::is_parameter_error_name(colon_name.trim_end())
    {
        // Substring: offset and length texts are arithmetic-expanded
        // eagerly (same admission as parse_parameter_substring).
        let (offset, length, has_length) = super::parameter_core::split_top_level_colon(colon_rest);
        texts.push(offset);
        if has_length {
            texts.push(length);
        }
    }
    texts
}

/// The assignment targets bound through a body's eager secondary
/// texts (rubash#1588) — the #1169 freeze must cover these too, since
/// the pre-scan now applies them before the word's real pass.
fn secondary_assignment_targets(inner: &str) -> Vec<String> {
    fn collect(text: &str, out: &mut Vec<String>) {
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            let after = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(after) else {
                break;
            };
            let nested = &after[..end];
            if let Some(target) = parameter_assignment_target(nested) {
                out.push(target);
            }
            for text in secondary_assignment_texts(nested) {
                collect(text, out);
            }
            rest = &after[end + 1..];
        }
    }
    let mut targets = Vec::new();
    for text in secondary_assignment_texts(inner) {
        collect(text, &mut targets);
    }
    targets
}

/// The variable a `${...}` body assigns through a top-level `:=` or `=`
/// operator (base name, subscript stripped), when it is a plain shell
/// name — the target whose later assignment can leak into earlier
/// fragments of the same word (rubash#1169).
fn parameter_assignment_target(inner: &str) -> Option<String> {
    // rubash#1960: the `:=` split's left-hand side is not always the
    // target — in `${v=x:=y}` the first operator is `=` and the `:=`
    // is word text, so a `:=` candidate whose base is not a shell name
    // falls through to the `=` split instead of reporting no target.
    for (name, _word) in [
        super::expand_braced_ops::split_once_outside_subscript_str(inner, ":="),
        super::expand_braced_ops::split_once_outside_subscript(inner, '='),
    ]
    .into_iter()
    .flatten()
    {
        let base = name.split('[').next().unwrap_or(name);
        if is_shell_name(base) {
            return Some(base.to_string());
        }
    }
    None
}

/// Whether `name` — the left-hand side of a top-level `=`/`:=` split of
/// a `${...}` body — is syntactically an assignment target: a plain shell
/// name, a subscripted name (`name[sub]`, the admission
/// apply_array_element_parameter_assignment itself uses), or either form
/// behind the `!` indirect prefix. GNU parameter_brace_expand
/// (subst.c:9777) extracts the parameter name first and dispatches on the
/// FIRST operator character (subst.c:9886-9917), so an `=`/`:=` that sits
/// inside another operator's word is word text, never an assignment
/// operator: in `${a:-x=...}` the body's operator is `:-` and the `x=`
/// segment is literal default-word text. The pre-scan's split finds that
/// `=` anyway; without this form check it expanded the split's "value" —
/// the rest of the default word, nested fragment included — as if
/// applying a bind, then discarded it when every apply arm rejected the
/// name. That phantom evaluation ran side effects GNU never runs (the
/// word expands only when its operator selects it, subst.c:7663
/// parameter_brace_expand_word) and recurred at every nesting level,
/// multiplying the inner expansion into the rubash#1953 exponential.
fn parameter_assignment_target_form(name: &str) -> bool {
    let bare = name.strip_prefix('!').unwrap_or(name);
    if is_shell_name(bare) {
        return true;
    }
    matches!(parse_array_subscript(bare), Some((base, _)) if is_shell_name(base))
}

/// Whether a `${...}` body is side-effect-free to evaluate during the
/// assignment pre-scan: no command substitution and no backticks, and
/// not a bad-substitution shape whose diagnostic must stay with the
/// real pass (rubash#1169 freeze admission). `$((...))` arithmetic is
/// admitted only when it plainly carries no side effect: no `++`/`--`
/// and no `=` anywhere (which also excludes assignment expressions;
/// comparison `==` is excluded too, conservatively).
fn fragment_is_pure(inner: &str) -> bool {
    if inner.contains('`') {
        return false;
    }
    if inner.contains("$(") {
        let comsubs = inner.matches("$(").count();
        let ariths = inner.matches("$((").count();
        if comsubs != ariths || inner.contains("++") || inner.contains("--") || inner.contains('=')
        {
            return false;
        }
        // An assignment operator in the body itself (`${v:=...}` as the
        // frozen candidate) is handled by the `:=` memo, not the freeze.
        if super::expand_braced_ops::split_once_outside_subscript_str(inner, ":=").is_some()
            || super::expand_braced_ops::split_once_outside_subscript(inner, '=').is_some()
        {
            return false;
        }
    }
    !super::expand_word::braced_name_is_bad_substitution(inner)
        && !super::expand_word::braced_name_ends_on_quote(inner)
}

/// Whether a `${...}` body reads variable `var`: as its subject
/// (`${var}`, `${var:-...}`, `${var[...]}`, `${!var...}`, `${#var}`), or
/// as a nested `$var` / `${var...}` expansion in its word part
/// (rubash#1169 freeze admission).
fn fragment_references_var(inner: &str, var: &str) -> bool {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut subject = inner;
    if let Some(rest) = subject.strip_prefix('!') {
        subject = rest;
    } else if let Some(rest) = subject.strip_prefix('#') {
        // `${#var}` length form; `${#}`/`${#@}` are special parameters.
        if rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            subject = rest;
        }
    }
    if let Some(rest) = subject.strip_prefix(var) {
        if rest.is_empty() || !rest.chars().next().is_some_and(is_name_char) {
            return true;
        }
    }
    // Nested reads anywhere in the body (`${x:-$var}`, `${x:-${var}}`).
    let bytes = inner.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let mut j = i + 1;
            if bytes.get(j) == Some(&b'{') {
                j += 1;
            }
            if inner[j..].starts_with(var) {
                let after = inner[j + var.len()..].chars().next();
                if after.is_none() || !after.is_some_and(is_name_char) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

impl Executor {
    pub(in crate::executor) fn expand_parameter_word(&self, word: &str) -> String {
        // TODO(subst.c/parse.y): The `word` half of ${parameter:-word},
        // ${parameter:=word}, and ${parameter+word} has quote-aware expansion
        // flags. This covers tilde2.tests while the lexer still discards most
        // quote state.
        //
        // GNU subst.c:4462 expand_string_for_rhs (quoted == 0) +
        // posixexp2 37: whitespace that was quoted inside the rhs keeps its
        // quoted status through field splitting. The protect-ifs walk is the
        // designated engine (it marks quoted whitespace with the \x1c
        // sentinel while bare spaces stay splittable); the plain walk lost
        // that distinction and made `a=(${v:-"a b"})` two elements where GNU
        // stores ONE (rubash#212; arrayfunc.c:610 expand_words_no_vars
        // splits only unquoted expansion whitespace).
        let expanded = self.expand_embedded_parameters_protect_ifs(word);
        // GNU subst.c:4807 dequote_string: quote removal ran inside the
        // expansion pass, so `'`/`"` left in its output are
        // expansion-produced data, not syntax. Mark them \x17/\x18 before
        // decode_parameter_word_quotes, whose quote rules would otherwise
        // re-read a data `"` as an opener and drop it plus everything after
        // (array6.sub: `X${dbg-'x"'}Y` -> `Ax"Y`, not `AxY`).
        let expanded = expanded.replace('"', "\u{18}").replace('\'', "\u{17}");
        // GNU subst.c:4462 expand_string_for_rhs with quoted == 0 runs the
        // rhs through expand_word_internal, whose unquoted backslash branch
        // (subst.c:11671-11674) consumes the backslash before ANY character
        // — CTLESC + c for the glob-metacharacter class, the plain char
        // otherwise (`${x:+a\qb}` is `aqb`; rubash#209). The former
        // admission-list unescape kept `\@`/`\q` pairs outside the
        // whole-word argument position.
        let expanded = unescape_unquoted_rhs_escapes(&decode_parameter_word_quotes(&expanded));
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.shell_state.env_vars, false)
    }

    // Alternate word of the `-`/`+`/`:-`/`:+` operators. GNU expands the
    // rhs like an unquoted word (subst.c parameter_brace_expand_rhs with
    // quoted == 0) while keeping quoted/escaped whitespace out of field
    // splitting (posixexp2 37). The walk-time variant in
    // expand_embedded_parameters_alternate_mut applies Unquoted quote
    // removal, the whitespace sentinel, and unquoted backslash rules in
    // one pass, so expansions inside quoted regions see the quote state
    // and their own whitespace stays protected (more-exp ${B:-"$A"}).
    pub(in crate::executor) fn expand_alternate_parameter_word(&mut self, word: &str) -> String {
        let expanded = self.expand_embedded_parameters_alternate_mut(word);
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.shell_state.env_vars, false)
    }

    pub(in crate::executor) fn expand_parameter_word_mut(&mut self, word: &str) -> String {
        // GNU subst.c:4462 expand_string_for_rhs with quoted == 0 (the
        // `=`/`:=` rhs on an assignment RHS is the same unquoted-rhs
        // expansion): the alternate-rhs walker resolves backslashes for ANY
        // character in one pass, CTLESC-quotes the glob-metachar class, and
        // IFS-glues escaped whitespace — a final text unescape cannot
        // distinguish a source `\X` from a `\` the generic walker already
        // collapsed out of `\\` (rubash#209: `${z=a\\b}` stores `a\b`,
        // `${z=a\@b}` stores `a@b`).
        let expanded = self.expand_embedded_parameters_alternate_mut(word);
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.shell_state.env_vars, false)
    }

    /// GNU subst.c pos_params (3745) + string_list_pos_params (3030):
    /// `${*:offset:length}` selects a range of positional parameters and
    /// joins them with IFS[0] (string_list_dollar_star, subst.c:2902),
    /// while `${@:offset:length}` joins with a space (string_list). The
    /// offset==0 case prepends $0 (dollar_vars[0]) per pos_params:3759.
    /// expand_braced_substring_parameter always joins with a space, so
    /// intercept `*`/`@` here to apply the correct separator.
    fn expand_star_at_substring(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> String {
        let selected = positional_parameter_substring_with_zero(
            self.shell_state.positional_window(),
            &self.script_name_value(),
            offset,
            length,
        );
        if var_name == "*" {
            let ifs = self
                .shell_state
                .env_vars
                .get("IFS")
                .cloned()
                .unwrap_or_else(|| " \t\n".to_string());
            match ifs.chars().next() {
                Some(separator) => selected.join(&separator.to_string()),
                None => selected.concat(),
            }
        } else {
            selected.join(" ")
        }
    }

    /// GNU subst.c:9990-9993 + param_expand: for a list operand
    /// (`name[@]`, `name[*]`, `@`, `*`) the `-`/`:-`/`+`/`:+` set/null test
    /// applies to the operand's string form — string_list_dollar_at joins
    /// [@]/@ with " ", string_list_dollar_star joins [*]/* with IFS[0] — so
    /// `"<${A[*]:-X}>"` under IFS='' with A=('' '') joins to "" and yields
    /// `X`, while `"${A[@]:-Y}"` joins to " " and expands (array22.sub).
    /// Returns Some((joined_word, non_empty)) for list operands.
    fn list_operand_joined_word(&self, var_name: &str) -> Option<(String, bool)> {
        let (values, is_at) = self.braced_operator_list_values(var_name)?;
        let separator = if is_at {
            " ".to_string()
        } else {
            self.ifs_first_char_separator()
        };
        let non_empty = !values.is_empty();
        Some((values.join(&separator), non_empty))
    }

    pub(in crate::executor) fn expand_quoted_parameter_word(&self, word: &str) -> String {
        // TODO(subst.c/parse.y): Quoted parameter expansion should carry
        // CTLESC/CTLQUOTEMARK state from the parser. This preserves the
        // tilde2.tests distinction that quoted default/alternate words do not
        // perform tilde expansion.
        let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return self.expand_embedded_parameters(word);
        };
        if !braced_parameter_spans_whole_word(word) {
            return self.expand_embedded_parameters(word);
        }

        if let Some((var_name, default)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":-")
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        return joined;
                    }
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                default,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    default,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        )
                    });
            }
        }

        if let Some((var_name, alternate)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":+")
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        return unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    alternate,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        );
                    }
                    return String::new();
                }
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                alternate,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return String::new();
            }
        }

        if let Some((var_name, error_word)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":?")
        {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                return self.expand_embedded_parameters(error_word);
            }
        }

        if let Some((var_name, error_word)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '?')
        {
            // GNU subst.c parameter_brace_expand: `${#?}` is the length of
            // `$?`, not `$#` with the `?` error operator. Only the bare `#?`
            // form (no error word) is the length-of-special case; `${#?word}`
            // stays the `?` operator.
            if is_parameter_error_name(var_name) && !(var_name == "#" && error_word.is_empty()) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(error_word));
            }
        }

        if let Some((var_name, word)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":=")
        {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(word));
            }
        }

        if let Some((var_name, word)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '=')
        {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(word));
            }
        }

        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            // `${#:offset}` / `${#?:0}` slice the special parameter value,
            // not a length expansion. expand_braced_substring_parameter does
            // not resolve special-parameter names, so fetch the value here.
            if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
                let value = self.expand_parameter_named_value(var_name);
                return parameter_substring(&value, offset, length);
            }
            if matches!(var_name, "*" | "@") {
                return self.expand_star_at_substring(var_name, offset, length);
            }
            return self.expand_braced_substring_parameter(var_name, offset, length);
        }

        if name.starts_with('#') && !hash_is_special_param_with_operator(name) {
            if let Some(value) = self.expand_braced_indexed_parameter(name) {
                return value;
            }
        }

        if let Some((array_name, _)) = parse_array_subscript(name) {
            // GNU valid_array_reference only treats NAME[...] as a subscript
            // when NAME is a valid identifier; pattern words like `z//[^;]`
            // or `z/#[^;][^;]` must not produce a spurious bad-array-
            // subscript diagnostic here (new-exp8.sub).
            if is_shell_name(array_name) {
                if let Some(value) = self.array_element_parameter_value(name) {
                    return shell_safe_value(&value);
                }
            }
        }

        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))
        {
            // rubash#375: dynamic stack arrays join from the live view
            // (GNU: real ARRAY objects, variables.c INIT_DYNAMIC_VAR).
            if let Some(joined) = self.dynamic_array_joined(name) {
                return joined;
            }
            return self
                .parameter_array_storage(array_name)
                .map(|value| {
                    // Scalar base: one element, the whole string, joined
                    // with itself — never list storage re-parsed
                    // (rubash#755).
                    if self.parameter_is_array_variable(array_name) {
                        self.join_array_parameter_values(&value, name)
                    } else {
                        shell_safe_value(&value)
                    }
                })
                .unwrap_or_default();
        }

        if let Some((var_name, alternate)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '+')
        {
            if is_parameter_error_name(var_name) {
                if let Some((_, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        return unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    alternate,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        );
                    }
                    return String::new();
                }
                if self.parameter_operator_value(var_name).is_some() {
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                alternate,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return String::new();
            }
        }

        if let Some((var_name, default)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '-')
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        return joined;
                    }
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                default,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    default,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        )
                    });
            }
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name, false) {
            return value;
        }

        self.expand_word(word)
    }

    // GNU param_expand: the word of ${var-word}/${var:=word}/${var+word}/...
    // undergoes tilde expansion only in an unquoted expansion context and only
    // when the word itself starts bare (an explicitly quoted `~` stays
    // literal). Tilde applies at the word start, not after colons.
    fn tilde_expand_operator_word(&self, word: &str, context: SubstitutionQuoteContext) -> String {
        if !matches!(context, SubstitutionQuoteContext::Unquoted) {
            return word.to_string();
        }
        if word.starts_with('"') || word.starts_with('\'') || word.starts_with('\\') {
            return word.to_string();
        }
        tilde_expand::expand_assignment_tilde_value(word, &self.shell_state.env_vars, false)
    }

    /// GNU parameter_brace_expand_word (subst.c:7663): the `word` half of
    /// `${var op word}` is expanded under the word's own quote context. The
    /// double-quote sentence decode (slashify_in_quotes port) applies inside
    /// "${...}" and in here-document bodies (Q_HERE_DOCUMENT behaves like a
    /// dq context for ${} words); for an unquoted ${...} the rhs keeps its
    /// original
    /// quote syntax so the walker's quote removal strips it (`o=${x-' '}`
    /// stores a space; `${f-'$HOME'}` keeps `$HOME` unexpanded).
    fn decode_operator_word_for_context(
        &self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        if matches!(
            context,
            SubstitutionQuoteContext::DoubleQuoted | SubstitutionQuoteContext::HereDocument
        ) {
            decode_double_quotes_in_quoted_parameter_word(
                word,
                self.posix_mode_enabled(),
                matches!(context, SubstitutionQuoteContext::HereDocument),
            )
        } else {
            word.to_string()
        }
    }

    /// GNU subst.c:4462 expand_string_for_rhs / 7663
    /// parameter_brace_expand_word: the `word` half of `${var op word}`
    /// expands under the word's own quote context. The UNQUOTED case is the
    /// alternate-rhs walker — the expand_string_for_rhs quoted==0 port with
    /// unquoted backslash rules for ANY character (subst.c:11671-11674: the
    /// escaped char is added, CTLESC-quoted for the glob-metachar class),
    /// IFS_GLUE for quoted/escaped whitespace, and single-pass resolution so
    /// a source `\\` becomes one literal `\` exactly once. A final
    /// text-level unescape cannot reproduce that: the generic walker
    /// pre-collapses `\\` to a bare `\` that is then indistinguishable from
    /// a source escape (`${v-foo\\bar}` is `foo\bar` while `${v-foo\bar}`
    /// is `foobar`), which is what kept `\@`/`\q` alive in assignment and
    /// fragment positions (rubash#209).
    fn expand_operator_rhs_for_context(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        if !matches!(context, SubstitutionQuoteContext::Unquoted) {
            return unescape_parameter_operator_result(
                &self.expand_embedded_parameters_mut_with_context(
                    &self.decode_operator_word_for_context(word, context),
                    context,
                ),
                context,
                self.shell_state.env_vars.get("IFS").map(String::as_str),
            );
        }
        self.expand_embedded_parameters_alternate_mut(word)
    }

    /// Resolve one `${...}` fragment collected by the embedded walker
    /// WITHOUT re-entering the whole word-expansion pipeline (rubash#281).
    ///
    /// GNU anchor — subst.c:11229 `expand_word_internal()` is a single pass
    /// over the word: its `${` arm calls `parameter_brace_expand`
    /// (subst.c:9777) inline, and only the fragment's own machinery runs;
    /// GNU never re-enters a fresh expand_word_internal over a synthetic
    /// `${...}` word. The old rubash reconstruction
    /// (`expand_word_mut_with_context(&format!("${{{name}}}"))`) re-ran the
    /// pipeline preamble per fragment — a fresh SubXpassFrame scope, the
    /// `:=`/`=` assignment pre-scan, the funsub and whole-word routing
    /// scans — which was 2.5M allocations on expansion-heavy probes
    /// (perf2 profile: ~58 allocs per `${}` fragment).
    ///
    /// Equivalence admission (rubash#117 whitelist — each condition proves
    /// one preamble step of `expand_word_mut_with_context` is a no-op for
    /// the synthetic `${name}` word, so its routing lands on
    /// `expand_quoted_parameter_word_mut(synthetic, context)`, the exact
    /// call made here):
    ///
    /// - `name` contains no `=`: both splits of
    ///   `apply_parameter_assignment_expansions_in_word`
    ///   (`split_once_outside_subscript_str(inner, ":=")` and
    ///   `split_once_outside_subscript(inner, '=')`) fail, so the pre-scan
    ///   assigns nothing. (`split_assignment_word` also cannot match: the
    ///   word begins with `$`, not a legal variable starter.)
    /// - `name` does not start with a funsub introducer (whitespace/`|`):
    ///   the synthetic word's only top-level `${` is not a current-shell
    ///   command substitution, so the funsub routing branch is not taken; a
    ///   funsub NESTED in the name is not top-level (the outer `${` counts
    ///   as an open brace) and reaches the same braced expander either way.
    /// - `braced_parameter_spans_whole_word(synthetic)`: the re-entry's
    ///   whole-word `${...}` routing applies; without the span the old path
    ///   fell through to later arms, so those shapes keep the full
    ///   re-entry.
    ///
    /// SubXpassFrame memo semantics: the skipped frame is the point of the
    /// change. The nested fragment's cross-pass entries now land in the
    /// ENCLOSING word's frame instead of a private one dropped on return.
    /// Keys are (word-context id, fragment path, text) and the walker's
    /// site guard has already extended the path with this fragment's
    /// ordinal, so sibling fragments cannot false-hit; a fragment re-probed
    /// by a later pass over the same word now HITS the memo — the memo's
    /// documented purpose (one `${}` evaluation per GNU
    /// parameter_brace_expand, subscript side effects included). The
    /// WordCtxGuard is kept so a walker entered without an active word
    /// context still keys its fragments exactly as before.
    pub(in crate::executor) fn expand_braced_parameter_fragment_in_word(
        &mut self,
        name: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        // Fall-through shapes (a `=` assignment-operator candidate, or a
        // funsub introducer `|`/whitespace) need the full pipeline; they
        // keep the allocating form because they are the rare arm.
        if name.contains('=')
            || name
                .chars()
                .next()
                .is_some_and(|first| first == '|' || first.is_whitespace())
        {
            return self.expand_word_mut_with_context(&format!("${{{name}}}"), context);
        }
        // GNU subst.c:11229 expand_word_internal runs parameter_brace_expand
        // inline on the SAME word buffer; the fragment re-entry needs a
        // standalone `${...}` string, so reuse one from the depth pool
        // instead of allocating per fragment. The buffer is returned to the
        // pool after the expansion, so a nested fragment (which re-enters
        // this function) only allocates while an outer buffer is live.
        let mut synthetic = pop_synthetic_fragment_word();
        synthetic.clear();
        synthetic.push_str("${");
        synthetic.push_str(name);
        synthetic.push('}');
        let result = if braced_parameter_spans_whole_word(&synthetic) {
            let _wctx = crate::executor::expand_braced_indices::WordCtxGuard::new_if_absent();
            self.expand_quoted_parameter_word_mut_routed(&synthetic, context, true)
        } else {
            self.expand_word_mut_with_context(&synthetic, context)
        };
        recycle_synthetic_fragment_word(synthetic);
        result
    }

    pub(in crate::executor) fn expand_quoted_parameter_word_mut(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        // Non-POSIX whole-word span, computed once here and threaded into
        // the routed body so the per-`${}`-fragment re-entry below can skip
        // its own scan (same string, same value).
        let spans_nonposix = braced_parameter_spans_whole_word(word);
        // Bash 5.3 (parser.h FUNSUB_CHAR): a whitespace-led `${ command; }` /
        // `${|command;}` word is a nofork command substitution, not a
        // parameter form. The operator split_once parsing below would treat
        // the body's first `=` as ${name=word} assignment syntax and expand
        // the rest of the body as literal text (comsub22.sub: a multi-line
        // funsub in double quotes printed the body instead of executing it).
        // Route the whole word through the embedded walker, which collects
        // the funsub span and executes it in the current shell. Nested
        // funsubs inside an outer parameter form stay on the operator path.
        if crate::executor::parameter_core::word_contains_current_shell_command_substitution(word)
            && crate::executor::parameter_core::funsub_span_is_top_level(word)
        {
            // Frame placement mirrors the routed body below: the whole-word
            // gate value decides, so a funsub word keeps the exact memo
            // scope the pre-split code provided (push-if-empty shares nested
            // `${}`s with the outer scope either way).
            let _memo_frame =
                spans_nonposix.then(crate::executor::expand_braced_indices::AepvMemoFrame::new);
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }
        self.expand_quoted_parameter_word_mut_routed(word, context, spans_nonposix)
    }

    /// Routed body of [`Executor::expand_quoted_parameter_word_mut`] after
    /// the funsub scan. `spans_nonposix` MUST be
    /// `braced_parameter_spans_whole_word(word)` for this exact `word` (the
    /// outer entry just computed it; the fragment entry re-proved it on its
    /// identical synthetic word) — it feeds both the memo-frame gate and the
    /// non-POSIX span verdict below, so neither is re-scanned. Callers that
    /// reach here have proven the word carries no TOP-LEVEL funsub span:
    /// the outer entry ran the scan; the fragment entry's whitelist (see
    /// `expand_braced_parameter_fragment_in_word`) admits only a synthetic
    /// word whose sole top-level `${` is not a funsub introducer, and any
    /// `${...}` nested inside its name is enclosed by that outer `${`, so
    /// `funsub_span_is_top_level` is false either way and the routed arm
    /// runs. GNU subst.c:11229 runs the same inline parameter_brace_expand
    /// with no re-scan.
    pub(in crate::executor) fn expand_quoted_parameter_word_mut_routed(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
        spans_nonposix: bool,
    ) -> String {
        // GNU param_expand resolves one `${}` expansion once: memoize
        // array-element fetches for the duration of this word so repeated
        // helper-layer reads (operator set-checks, error probes) do not
        // re-run subscript side effects (AEPV_MEMO). Whole-word `\x1d`-marked
        // `${}` forms reach here without passing a walker `${` arm, so a
        // frame is needed at this entry — but only when the word is one
        // `${}` span; a multi-`${}` word must keep per-`${}` scoping, which
        // the walker's `${` arms provide (push-if-empty shares nested `${}`s
        // with the outer scope).
        let _memo_frame =
            spans_nonposix.then(crate::executor::expand_braced_indices::AepvMemoFrame::new);
        // In POSIX mode, a double-quoted `${...}` may close at a `}` inside
        // the apparent word when a single quote is literal (Interp 221).
        // Expand that braced head separately, then continue with the suffix.
        // rubash#1109 item 2: above compat 42 only — at <= 42 the quote
        // quotes again and the whole-word span below governs.
        if matches!(context, SubstitutionQuoteContext::DoubleQuoted)
            && self.posix_mode_enabled()
            && crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) > 42
        {
            if let Some(rest) = word.strip_prefix("${") {
                if let Some(close) = matching_parameter_brace_in_context(rest, true, true) {
                    if close + 1 < rest.len() {
                        let braced_end = 2 + close + 1;
                        let head =
                            self.expand_quoted_parameter_word_mut(&word[..braced_end], context);
                        let tail = self.expand_embedded_parameters_mut_with_context(
                            &word[braced_end..],
                            context,
                        );
                        return format!("{head}{tail}");
                    }
                }
            }
        }

        let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        };
        // rubash#1109: this extraction assumed the final `}` closes, but
        // under the subst.c:1954 span rules (no posix+compat>42 literal
        // gate) a body ending inside an unterminated single-quote span
        // has NO closing brace in GNU — the extraction fails there.
        if matches!(
            context,
            SubstitutionQuoteContext::DoubleQuoted | SubstitutionQuoteContext::HereDocument
        ) && self.no_closing_brace_verdict(name, false, true, &format!("\"{word}\""))
        {
            return String::new();
        }
        // The whole-word span check must use the same quote rules the lexer
        // used to build the word. In POSIX mode inside double quotes the
        // Interp 221 big hammer closes `${...}` at the first `}` (single
        // quotes are literal), so a body with an unbalanced `'` such as
        // `${IFS+"'"x ~ x'}` still closes at the final `}`; the non-POSIX
        // scan leaves that `}` quote-hidden and mis-routes the word to the
        // embedded walker (posixexp2 case 28).
        let spans_whole_word = if matches!(context, SubstitutionQuoteContext::DoubleQuoted)
            && self.posix_mode_enabled()
            && crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) > 42
        {
            braced_parameter_spans_whole_word_in_context(word, true, true)
        } else {
            spans_nonposix
        };
        if !spans_whole_word {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }

        // The extquote decode (expand_brace_dollar / extract_dollar_brace_
        // string's quoted state) runs BEFORE the bad-name gates — the same
        // order expand_braced_parameter_word uses — but only where the
        // fragment's effective quote state is QUOTED: a double-quoted word,
        // a here-document body, or a command-substitution body (bash
        // decodes `$'…'`-names there even unquoted, huck#8). A top-level
        // or offset-position unquoted `${$'…'}` keeps the bad-substitution
        // verdict (rubash#539).
        let extquote_decodes_here = !matches!(context, SubstitutionQuoteContext::Unquoted)
            || self
                .shell_state
                .env_vars
                .contains_key("__RUBASH_COMSUB_BODY");
        if extquote_decodes_here {
            if let Some(resolved) =
                crate::executor::expand_word::resolve_dollar_quoted_parameter_name(name)
            {
                let resolved_word = format!("${{{resolved}}}");
                return self.expand_quoted_parameter_word_mut(&resolved_word, context);
            }
        }
        // The bad-substitution diagnostic names the ENCLOSING word GNU was
        // expanding (subst.c:10278), recorded by the mutable walker; consume
        // the one-shot record so a nested fragment keeps its own word
        // (rubash#758).
        let diagnostic_word = crate::executor::expand_word::take_pending_diagnostic_word()
            .unwrap_or_else(|| word.to_string());
        // GNU subst.c:10272-10288 (parameter_brace_expand): a quote
        // terminating the parameter name matches no operator arm and lands
        // on the `bad substitution` default. Mirror of the same check in
        // expand_braced_parameter_word for the `\x1d`/whole-word entry.
        if crate::executor::expand_word::braced_name_ends_on_quote(name) {
            self.write_diagnostic_fd2(
                format!(
                    "{}{}: bad substitution\n",
                    self.diagnostic_prefix(),
                    crate::executor::expand_word::bad_substitution_display(&diagnostic_word)
                )
                .as_bytes(),
            );
            self.shell_state.parameter_bad_substitution.set(true);
            return String::new();
        }

        // GNU subst.c:10042-10046 `valid_brace_expansion_word` gate — the
        // mutable-path mirror of the check expand_braced_parameter_word
        // runs at its whole-word entry, for words that arrive here without
        // passing a walker `${` arm.
        if crate::executor::expand_word::braced_name_is_bad_substitution(name) {
            self.write_diagnostic_fd2(
                format!(
                    "{}{}: bad substitution\n",
                    self.diagnostic_prefix(),
                    crate::executor::expand_word::bad_substitution_display(&diagnostic_word)
                )
                .as_bytes(),
            );
            self.shell_state.parameter_bad_substitution.set(true);
            return String::new();
        }

        // This `\x1d`-quoted word IS one `${}` fragment: record site [0]
        // so the `:=`/`-=` operator set-checks dedup subscript side
        // effects against the pre-scan (SUB_RES_XPASS). An active site
        // means the enclosing fragment already named it — keep it.
        let _site_guard = (!crate::executor::expand_braced_indices::sub_site_active())
            .then(|| crate::executor::expand_braced_indices::SubSiteGuard::new(0));

        // rubash#1169: the assignment pre-scan may have frozen this
        // fragment at its pre-assignment value (it precedes a `:=`/`=`
        // fragment, in the same word, that assigns a variable it reads).
        // The live env already carries that assignment, so the dispatch
        // below must not re-derive the fragment from it.
        if let Some(key) = crate::executor::expand_braced_indices::sub_site_key(name) {
            if let Some(frozen) =
                crate::executor::expand_braced_indices::frozen_fragment_lookup(&key)
            {
                return frozen;
            }
        }

        // One-pass operator-boundary index for the dispatch chain below:
        // GNU parameter_brace_expand (subst.c:9777) extracts the name up to
        // the FIRST operator character in a single string_extract pass and
        // dispatches on that character; this index answers the same
        // first-top-level-occurrence questions for every operator probe at
        // once (byte-identical to the per-operator splits it replaces —
        // see TopLevelOpIndex).
        let ops = super::expand_braced_ops::TopLevelOpIndex::new(name);

        if let Some((var_name, default)) = ops.split_colon_pair(name, b'-') {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        return joined;
                    }
                    let default = self.tilde_expand_operator_word(default, context);
                    return self.expand_operator_rhs_for_context(&default, context);
                }
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let default = self.tilde_expand_operator_word(default, context);
                        self.expand_operator_rhs_for_context(&default, context)
                    });
            }
        }

        if let Some((var_name, alternate)) = ops.split_colon_pair(name, b'+') {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        let alternate = self.tilde_expand_operator_word(alternate, context);
                        return self.expand_operator_rhs_for_context(&alternate, context);
                    }
                    return String::new();
                }
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    let alternate = self.tilde_expand_operator_word(alternate, context);
                    return self.expand_operator_rhs_for_context(&alternate, context);
                }
                return String::new();
            }
        }

        if let Some((var_name, error_word)) = ops.split_colon_pair(name, b'?') {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                let error_word = self.tilde_expand_operator_word(error_word, context);
                return self.expand_embedded_parameters_mut_with_context(&error_word, context);
            }
        }

        if let Some((var_name, error_word)) = ops.split_byte(name, b'?') {
            // GNU subst.c parameter_brace_expand: `${#?}` is the length of
            // `$?`, not `$#` with the `?` error operator. Only the bare `#?`
            // form (no error word) is the length-of-special case; `${#?word}`
            // stays the `?` operator.
            if is_parameter_error_name(var_name) && !(var_name == "#" && error_word.is_empty()) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let error_word = self.tilde_expand_operator_word(error_word, context);
                        self.expand_embedded_parameters_mut_with_context(&error_word, context)
                    });
            }
        }

        if let Some((var_name, word)) = ops.split_colon_pair(name, b'=') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        // GNU evaluates the `:=` operator once per `{}`
                        // occurrence (subst.c:9777 parameter_brace_expand ->
                        // subst.c:10346 `case '='` with check_nullness ->
                        // parameter_brace_expand_rhs): the assigned value IS
                        // the expansion result. When a pre-scan pass already
                        // applied this fragment (site-keyed memo), reuse its
                        // resolved value instead of re-expanding the
                        // alternate — a `$(...)` alternate would otherwise
                        // re-execute (base: `v=${v:=$(echo HI >&2)}` ran the
                        // comsub 4x, GNU 1x).
                        if let Some(key) =
                            crate::executor::expand_braced_indices::sub_site_key(name)
                        {
                            if let Some(resolved) =
                                crate::executor::expand_braced_indices::assign_applied_lookup(&key)
                            {
                                return resolved;
                            }
                        }
                        let word = self.tilde_expand_operator_word(word, context);
                        // GNU parameter_brace_expand_word sets
                        // expand_no_split_dollar_star for op == '='
                        // (subst.c:4487), which includes `:=`. This makes
                        // unquoted $* with null IFS join with IFS[0] inside
                        // the value (exp11.sub ${c=${*/}}).
                        let old =
                            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
                        let result =
                            self.expand_embedded_parameters_mut_with_context(&word, context);
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
                        result
                    });
            }
        }

        if let Some((var_name, word)) = ops.split_byte(name, b'=') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let word = self.tilde_expand_operator_word(word, context);
                        // GNU parameter_brace_expand_word sets
                        // expand_no_split_dollar_star for op == '='
                        // (subst.c:4487). This makes unquoted $* with null
                        // IFS join with IFS[0] inside the value
                        // (exp11.sub ${c=${*/}}).
                        let old =
                            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
                        let result =
                            self.expand_embedded_parameters_mut_with_context(&word, context);
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
                        result
                    });
            }
        }

        if let Some((var_name, offset, length)) = self.parse_parameter_substring_mut(name) {
            // `${#:offset}` / `${#?:0}` slice the special parameter value,
            // not a length expansion. expand_braced_substring_parameter does
            // not resolve special-parameter names, so fetch the value here.
            if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
                let value = self.expand_parameter_named_value(var_name);
                return parameter_substring(&value, offset, length);
            }
            if matches!(var_name, "*" | "@") {
                return self.expand_star_at_substring(var_name, offset, length);
            }
            return self.expand_braced_substring_parameter(var_name, offset, length);
        }

        if name.starts_with('#') && !hash_is_special_param_with_operator(name) {
            if let Some(value) = self.expand_braced_indexed_parameter(name) {
                return value;
            }
        }

        if let Some((array_name, _)) = parse_array_subscript(name) {
            // GNU valid_array_reference only treats NAME[...] as a subscript
            // when NAME is a valid identifier; pattern words like `z//[^;]`
            // or `z/#[^;][^;]` must not produce a spurious bad-array-
            // subscript diagnostic here (new-exp8.sub).
            if is_shell_name(array_name) {
                if let Some(value) = self.array_element_parameter_value(name) {
                    return shell_safe_value(&value);
                }
            }
        }

        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))
        {
            // rubash#375: dynamic stack arrays join from the live view
            // (GNU: real ARRAY objects, variables.c INIT_DYNAMIC_VAR).
            if let Some(joined) = self.dynamic_array_joined(name) {
                return joined;
            }
            return self
                .parameter_array_storage(array_name)
                .map(|value| {
                    // Scalar base: one element, the whole string, joined
                    // with itself — never list storage re-parsed
                    // (rubash#755).
                    if self.parameter_is_array_variable(array_name) {
                        self.join_array_parameter_values(&value, name)
                    } else {
                        shell_safe_value(&value)
                    }
                })
                .unwrap_or_default();
        }

        if let Some((var_name, alternate)) = ops.split_byte(name, b'+') {
            if is_parameter_error_name(var_name) {
                if let Some((_, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        let alternate = self.tilde_expand_operator_word(alternate, context);
                        return self.expand_operator_rhs_for_context(&alternate, context);
                    }
                    return String::new();
                }
                if self.parameter_operator_value(var_name).is_some() {
                    let alternate = self.tilde_expand_operator_word(alternate, context);
                    return self.expand_operator_rhs_for_context(&alternate, context);
                }
                return String::new();
            }
        }

        if let Some((var_name, default)) = ops.split_byte(name, b'-') {
            if is_parameter_error_name(var_name) {
                if let Some((joined, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        return joined;
                    }
                    let default = self.tilde_expand_operator_word(default, context);
                    return self.expand_operator_rhs_for_context(&default, context);
                }
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let default = self.tilde_expand_operator_word(default, context);
                        self.expand_operator_rhs_for_context(&default, context)
                    });
            }
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            if apfae_trace_enabled() {
                eprintln!("[EQPWM-tail-pattern] name={name:?}");
            }
            return value;
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(
            name,
            matches!(context, SubstitutionQuoteContext::Unquoted),
        ) {
            if apfae_trace_enabled() {
                eprintln!("[EQPWM-tail-special] name={name:?}");
            }
            return value;
        }

        if apfae_trace_enabled() {
            eprintln!("[EQPWM-tail-word] name={name:?}");
        }
        self.expand_word(word)
    }

    /// Expand a `=`/`:=` alternate for assignment. Outside double quotes the
    /// alternate is fully quote-removed (`a\ b` -> `a b`, posixexp2 case 35).
    /// Inside double quotes `\` only escapes $, `, ", \, and newline; any
    /// other `\X` is literal data and survives into the assigned value
    /// (`"${v=a\ b}"` assigns `a\ b`, posixexp2 case 36). Inside double
    /// quotes single quotes are data, not delimiters, so `$var` inside
    /// `'...'` is expanded (`"${fox='$foo'}"` assigns `'bar'`,
    /// more-exp.tests:112). Double quotes in the alternate are removed
    /// by `decode_double_quotes_in_quoted_parameter_word` (matching the
    /// `+`/`-` operator path), so `"${und="foo"}"` assigns `foo`
    /// (new-exp.tests:33).
    fn expand_assignment_alternate_mut(&mut self, value: &str, double_quoted: bool) -> String {
        // GNU parameter_brace_expand_word sets expand_no_split_dollar_star
        // for op == '=' (subst.c:4487), which includes `:=`. This makes
        // unquoted $* with null IFS join with IFS[0] inside the value
        // (exp11.sub ${c=${*/}}).
        let old = super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
        let result = self.expand_assignment_alternate_mut_inner(value, double_quoted);
        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
        result
    }

    fn expand_assignment_alternate_mut_inner(
        &mut self,
        value: &str,
        double_quoted: bool,
    ) -> String {
        if !double_quoted {
            // GNU subst.c:4807 dequote_word on the stored value: the CTLESC
            // carriers expand_parameter_word_mut puts in front of escaped
            // glob metacharacters are dropped (an assignment never undergoes
            // pathname expansion), and the IFS_GLUE sentinel is data-less
            // once the value no longer field-splits.
            return self
                .expand_parameter_word_mut(value)
                .replace(crate::executor::markers::CTLESC, "")
                .replace(crate::executor::markers::IFS_GLUE, "");
        }
        // 0x0e is unused by every other sentinel layer (the lexer's
        // PARAM_NAME_END_MARKER is 0x13); protect/restore is local to this
        // function, so the two never interact.
        const PROTECTED_LITERAL_BACKSLASH: char =
            crate::executor::markers::PARAM_WORD_BACKSLASH_GUARD;
        let chars: Vec<char> = value.chars().collect();
        let mut protected = String::with_capacity(value.len());
        let mut index = 0usize;
        while index < chars.len() {
            if chars[index] == '\\' {
                if let Some(next) = chars.get(index + 1).copied() {
                    if !matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                        protected.push(PROTECTED_LITERAL_BACKSLASH);
                        protected.push(next);
                        index += 2;
                        continue;
                    }
                }
            }
            protected.push(chars[index]);
            index += 1;
        }
        // Remove double quotes from the alternate (matching the `+`/`-`
        // operator path which calls decode_double_quotes_in_quoted_parameter_word).
        let decoded = decode_double_quotes_in_quoted_parameter_word(
            &protected,
            self.posix_mode_enabled(),
            false,
        );
        // Use DoubleQuoted context so single quotes are treated as data
        // (not quote delimiters), matching GNU's expand_string_for_rhs
        // behavior inside double quotes. Use unescape_parameter_operator_result
        // (not decode_parameter_word_quotes) so single quotes survive as data.
        let expanded = self.expand_embedded_parameters_mut_with_context(
            &decoded,
            SubstitutionQuoteContext::DoubleQuoted,
        );
        let unescaped = unescape_parameter_operator_result(
            &expanded,
            SubstitutionQuoteContext::DoubleQuoted,
            self.shell_state.env_vars.get("IFS").map(String::as_str),
        );
        unescaped.replace(PROTECTED_LITERAL_BACKSLASH, "\\")
    }

    pub(in crate::executor) fn apply_parameter_assignment_expansions_in_word(
        &mut self,
        word: &str,
    ) {
        // Admission (rubash#117 whitelist discipline, same shape as the
        // parameter_errors perf6 gate): both applied arms below split their
        // `${...}` body on `=` / `:=`, so a word carrying no `=` anywhere
        // can never fire one — the early-out only skips provably-inert
        // work. Plain words (`payload line`, `-lt`, `2000`) and plain
        // parameter expansions (`${a#alpha}`, `${a%%:*}`) reach this scan
        // on EVERY command word / assignment RHS; one scan-and-quit beats
        // the per-fragment quote-state walk that finds nothing.
        if !word.contains('=') {
            return;
        }
        // Assignment alternates are quote-removed with the quote rules of
        // the region their `${...}` sits in: inside double quotes `\` only
        // escapes $, `, ", \, and newline, so `"${v=a\ b}"` assigns `a\ b`
        // (posixexp2 case 36), while an unquoted `${v=a\ b}` assigns `a b`
        // and then field-splits (case 35). Parameter bodies inside a
        // command substitution span do NOT inherit the surrounding word's
        // quote context — they are applied later, when the inner command's
        // own words expand (`"x $(printf '%s ' ${v=a\ b})"` assigns `a b`).
        let quoted_word = word.starts_with(STORAGE_WORD_PREFIX);
        // When this word IS the `${}` fragment currently being evaluated
        // (a `${name}` body re-entered through expand_word_mut), its single
        // `${` occurrence inherits the enclosing fragment's site rather
        // than re-keying on the synthetic string (SUB_RES_XPASS docs).
        let inherit_site = braced_parameter_spans_whole_word(word)
            && crate::executor::expand_braced_indices::sub_site_active();
        // The prefix scanned for quote/CS context always runs from the word
        // start, so state skipped-over bodies (e.g. inside a command
        // substitution) still counts toward the next body's context.
        let mut consumed = 0usize;
        // Top-level `${` ordinal — the same fragments in the same order as
        // the expansion walker's `${` arms see them (command-substitution
        // bodies do not reach either scan's arm).
        let mut frag_index = 0usize;
        // Earlier top-level fragments (body text, ordinal), for the
        // rubash#1169 freeze below.
        let mut earlier_fragments: Vec<(String, usize)> = Vec::new();
        while let Some(rel) = word[consumed..].find("${") {
            let start = consumed + rel;
            let (in_double, inside_cs) = scan_word_prefix_quote_state(&word[..start], quoted_word);
            let body_start = start + 2;
            let Some(end) = matching_parameter_brace(&word[body_start..]) else {
                break;
            };
            if inside_cs {
                // Skip the whole body; the inner expansion applies it.
                consumed = body_start + end + 1;
                continue;
            }
            let inner = &word[body_start..body_start + end];
            let this_frag = frag_index;
            if !inherit_site {
                // rubash#1169: GNU evaluates a word's fragments strictly
                // left-to-right, but this pre-scan applies a `:=`/`=`
                // fragment's assignment before the real pass evaluates
                // ANY fragment — so an earlier fragment reading the
                // assigned variable would see the new value. Freeze each
                // earlier pure fragment that reads the target at its
                // current (pre-assignment) value, keyed by its site; the
                // real pass consults the frozen map first.
                // rubash#1588: targets bound through the fragment's
                // eager secondary texts (subscript / substring / pattern
                // spans) are applied by this same pre-scan, so the
                // freeze covers them exactly like a top-level target.
                let freeze_targets: Vec<String> = parameter_assignment_target(inner)
                    .into_iter()
                    .chain(secondary_assignment_targets(inner))
                    .collect();
                for target in &freeze_targets {
                    for (prev_inner, prev_index) in &earlier_fragments {
                        if !fragment_is_pure(prev_inner)
                            || !fragment_references_var(prev_inner, target)
                        {
                            continue;
                        }
                        let _freeze_guard =
                            crate::executor::expand_braced_indices::SubSiteGuard::new(*prev_index);
                        if let Some(key) =
                            crate::executor::expand_braced_indices::sub_site_key(prev_inner)
                        {
                            if crate::executor::expand_braced_indices::frozen_fragment_lookup(&key)
                                .is_none()
                            {
                                let synthetic = format!("${{{prev_inner}}}");
                                let value =
                                    self.expand_braced_parameter_word(&synthetic, prev_inner);
                                crate::executor::expand_braced_indices::frozen_fragment_store(
                                    key, value,
                                );
                            }
                        }
                    }
                }
            }
            let _site_guard = (!inherit_site).then(|| {
                let guard = crate::executor::expand_braced_indices::SubSiteGuard::new(frag_index);
                frag_index += 1;
                guard
            });
            // rubash#1588: the fragment's eager secondary texts
            // (subscript, substring offset/length, pattern spans)
            // evaluate before its own operator — apply any
            // `${name:=word}` nested in them first.
            self.apply_secondary_assignment_expansions(inner, in_double);
            self.apply_parameter_assignment_expansion_with_context(inner, in_double);
            if !inherit_site {
                earlier_fragments.push((inner.to_string(), this_frag));
            }
            consumed = body_start + end + 1;
        }
    }

    /// Latch a failed `${var:=word}`/`${var=word}` bind (subst.c
    /// parameter_brace_expand_rhs: bind_variable fails, then EX_BADUSAGE
    /// is written and bash_variable_assignment_error jumps DISCARD).
    /// `prev_status` is `$?` as it stood before the bind attempts — GNU
    /// prints the diagnostic from inside bind_variable (err_readonly ->
    /// error.c report_error) BEFORE the EX_BADUSAGE write, and
    /// report_error's errexit branch exit_shells with that earlier value
    /// (rubash#1542), so the consumer needs it snapshotted here.
    fn latch_parameter_assignment_failure(&self, prev_status: i32) {
        if !self.parameter_assignment_failure.replace(true) {
            self.parameter_assignment_failure_prev_status
                .set(Some(prev_status));
        }
    }

    /// rubash#1597: a triggered `=`/`:=` bind through the all-elements
    /// spelling (`name[@]` / `name[*]`) on a non-associative base has
    /// no element to bind — GNU parameter_brace_expand's `case '='`
    /// (subst.c:10346) routes the still-subscripted name to
    /// parameter_brace_expand_rhs (subst.c:8096) and the bind fails
    /// with `name[@]: bad array subscript`, EX_BADUSAGE. An associative
    /// base is the one legal case (the literal "@" / "*" text is a
    /// key); apply_array_element_parameter_assignment has already
    /// claimed it by the time this runs. Contexts the textual pre-scan
    /// (parameter_assignment_error_in_word) covers never reach the
    /// apply path — the command is discarded first — so reporting here
    /// cannot double-print.
    fn reject_bad_array_subscript_assignment(&mut self, name: &str, prev_status: i32) -> bool {
        let Some(base) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
        else {
            return false;
        };
        let resolved_base = self
            .nameref_target_name(base)
            .unwrap_or_else(|| base.to_string());
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved_base) {
            return false;
        }
        eprintln!("{}{}: bad array subscript", self.error_prefix(), name);
        self.latch_parameter_assignment_failure(prev_status);
        true
    }

    /// rubash#1588: apply the `${name:=word}` / `${name=word}` binds
    /// nested in a fragment's EAGER secondary texts (see
    /// secondary_assignment_texts). GNU parameter_brace_expand
    /// (subst.c:9777) evaluates the operand — subscript, substring
    /// arithmetic, pattern texts — through the regular expansion
    /// machinery, so a nested assignment operator binds exactly as if
    /// it stood alone; rubash's value paths read those texts through
    /// &self expanders (expand_subscript_string,
    /// eval_parameter_substring_offset, the pattern expanders), which
    /// produced the right VALUE but silently dropped the bind. The
    /// pre-scan is the one &mut pass over the word, so it applies the
    /// nested binds here, in textual order, before the fragment's own
    /// operator. Operator WORDS are not descended into (rubash#1564).
    fn apply_secondary_assignment_expansions(&mut self, inner: &str, double_quoted: bool) {
        for text in secondary_assignment_texts(inner) {
            self.apply_assignment_expansions_in_text(text, double_quoted);
        }
    }

    /// Apply every top-level `${...}` fragment found in an eager
    /// secondary text, recursively (a fragment's own secondary texts
    /// before its own operator, matching evaluation order).
    fn apply_assignment_expansions_in_text(&mut self, text: &str, double_quoted: bool) {
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            let after = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(after) else {
                break;
            };
            let nested = &after[..end];
            self.apply_secondary_assignment_expansions(nested, double_quoted);
            self.apply_parameter_assignment_expansion_with_context(nested, double_quoted);
            rest = &after[end + 1..];
        }
    }

    fn apply_parameter_assignment_expansion_with_context(
        &mut self,
        inner: &str,
        double_quoted: bool,
    ) {
        // GNU subst.c: a failed := assignment is an expand_word_error that
        // aborts the whole word expansion (DISCARD at top level), so once a
        // failure is latched the remaining expansions are skipped — this also
        // prevents the duplicate diagnostic when the word is re-expanded.
        if self.parameter_assignment_failure.get() {
            return;
        }
        // GNU subst.c param_expand: `=`/`:=` are parameter operators only at
        // the top level of the `${...}` body; inside `[...]` they belong to
        // the array subscript's arithmetic (nameref10.sub:
        // `${x[i=0$(...)]}` expands element i=0 -- it does not assign
        // through `x[i` and does not expand the subscript text as the
        // operator word, which would run the embedded $(...) twice).
        // rubash#1953: the `:=` split names an assignment only when its
        // left-hand side is an assignment target; otherwise the `:=`
        // sits inside another operator's word and the split's "value"
        // is that word's text, which must not be expanded here (see
        // parameter_assignment_target_form). rubash#1960: a rejected
        // `:=` split must not swallow the fragment either — GNU
        // parameter_brace_expand (subst.c:9777) extracts the parameter
        // name first and dispatches on the FIRST operator character,
        // so in `${v=x:=y}` the operator is the earlier `=` and the
        // `:=` is word text. The filter leaves such a body unclaimed
        // here so the `=` arm below binds `v` to `x:=y`.
        if let Some((name, value)) =
            super::expand_braced_ops::split_once_outside_subscript_str(inner, ":=")
                .filter(|(name, _)| parameter_assignment_target_form(name))
        {
            if self
                .parameter_operator_value(name)
                .is_some_and(|value| !value.is_empty())
            {
                return;
            }
            // GNU evaluates the `:=` operator once per `${}` occurrence
            // (subst.c:9777 parameter_brace_expand -> subst.c:10346
            // `case '='` with check_nullness -> parameter_brace_expand_rhs).
            // A NULL-resolving alternate fails the non-empty set-check in
            // every layered pass, re-executing the alternate each time; the
            // applied fragment is memoized under the cross-pass site key so
            // later passes (walker pre-scan, real operator arm) reuse the
            // resolved value instead of re-expanding.
            let assign_site = crate::executor::expand_braced_indices::sub_site_key(inner);
            if let Some(key) = &assign_site {
                if crate::executor::expand_braced_indices::assign_applied_lookup(key).is_some() {
                    return;
                }
            }
            let value = self.expand_assignment_alternate_mut(value, double_quoted);
            if let Some(key) = assign_site {
                crate::executor::expand_braced_indices::assign_applied_store(key, value.clone());
            }
            // GNU subst.c:8096 parameter_brace_expand_rhs: for op '=' (and
            // ':=', which only adds check_nullness) the value BOUND to the
            // parameter is `t1 = dequote_string (temp)` (subst.c:8151
            // bind_variable, subst.c:8145 assign_array_element) — fully
            // dequoted. The CTLESC protection expand_string_for_rhs gave the
            // quoted whitespace (posixexp2 37) belongs to the CURRENT word's
            // field splitting only; the expansion result is re-derived from
            // the quote context (subst.c:8196 quote_string(t1)). The IFS_GLUE
            // pairs unescape_parameter_operator_result added for the
            // double-quoted context are that CTLESC port, so they must be
            // decoded out of the stored value — otherwise the variable's
            // whitespace stays un-splittable forever (rubash#428: after
            // `: "${W:=u s p}"`, `set -- $W` yielded one word where GNU
            // yields three).
            let bound =
                crate::executor::command_substitution_values::decode_protected_ifs_chars(&value);
            // GNU parameter_brace_assign resolves the subscript AGAIN for
            // the assignment target (array_expand_index on the
            // assign_array_element path), distinct from the set-test's
            // array_variable_part evaluation — so `${b[i++]:=z}` stores
            // at the SECOND index. The marker path keeps this evaluation
            // out of the set-test's cross-pass memo.
            let _assign_site =
                crate::executor::expand_braced_indices::SubSiteGuard::new(usize::MAX);
            // rubash#1542: `$?` before the bind attempts — the value
            // error.c report_error would exit_shell with under errexit
            // (the diagnostic prints inside the bind, before subst.c
            // writes EX_BADUSAGE over it).
            let prev_status = self.exit_code;
            if self.apply_array_element_parameter_assignment(name, bound.clone()) {
                return;
            }
            if self.reject_bad_array_subscript_assignment(name, prev_status) {
                return;
            }
            if self.apply_indirect_parameter_assignment(name, bound.clone()) {
                return;
            }
            if !is_shell_name(name) {
                return;
            }
            if !self.apply_shell_assignment(name, bound) {
                self.latch_parameter_assignment_failure(prev_status);
            }
            return;
        }

        if let Some((name, value)) =
            super::expand_braced_ops::split_once_outside_subscript(inner, '=')
        {
            // rubash#1953: same target-form gate as the `:=` arm above —
            // an `=` inside another operator's word (`${a:-x=...}`) is
            // word text, not this fragment's assignment operator.
            if !parameter_assignment_target_form(name) {
                return;
            }
            if self.parameter_operator_value(name).is_some() {
                return;
            }
            let value = self.expand_assignment_alternate_mut(value, double_quoted);
            // Same bind-side dequote as the `:=` arm above (GNU subst.c:8096
            // dequote_string -> subst.c:8151 bind_variable): the double-quoted
            // context's IFS_GLUE whitespace protection is word-local and must
            // not be baked into the stored value (rubash#428).
            let bound =
                crate::executor::command_substitution_values::decode_protected_ifs_chars(&value);
            // Same assign-target re-evaluation as `:=` above.
            let _assign_site =
                crate::executor::expand_braced_indices::SubSiteGuard::new(usize::MAX);
            // rubash#1542: same pre-bind `$?` snapshot as the `:=` arm.
            let prev_status = self.exit_code;
            if self.apply_array_element_parameter_assignment(name, bound.clone()) {
                return;
            }
            if self.reject_bad_array_subscript_assignment(name, prev_status) {
                return;
            }
            if self.apply_indirect_parameter_assignment(name, bound.clone()) {
                return;
            }
            if !is_shell_name(name) {
                return;
            }
            if !self.apply_shell_assignment(name, bound) {
                self.latch_parameter_assignment_failure(prev_status);
            }
        }
    }

    fn apply_indirect_parameter_assignment(&mut self, name: &str, value: String) -> bool {
        let Some(indirect_name) = name.strip_prefix('!') else {
            return false;
        };
        if self.nameref_target_name(indirect_name).is_some() {
            return false;
        }
        let Some(target_name) = self.shell_state.env_vars.get(indirect_name).cloned() else {
            return false;
        };
        if self.apply_array_element_parameter_assignment(&target_name, value.clone()) {
            return true;
        }
        if !is_shell_name(&target_name) {
            return false;
        }
        self.apply_shell_assignment(&target_name, value);
        true
    }
}

/// GNU subst.c parameter_brace_expand: `${#` followed by one of the operator
/// characters `-`, `+`, `=`, `?` and a word makes `#` the special parameter
/// (positional parameter count) with that operator, not a length prefix.
/// `${#-}` / `${#?}` (operator char alone before `}`) remain length of the
/// special parameter. A `:`-led operator (`:-`, `:+`, `:=`, `:?`) makes `#`
/// the parameter even with an empty word — `${#:-}` is `$#` under `:-`
/// (more-exp `recho ${#:-}` -> `0`), while a bare `${#:}` is still a bad
/// substitution. Returns true when the `#` length-prefix route should be
/// skipped so the operator splits downstream handle the form.
fn hash_is_special_param_with_operator(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('#') else {
        return false;
    };
    if let Some(colon_op) = rest.strip_prefix(':') {
        return colon_op
            .chars()
            .next()
            .is_some_and(|op| matches!(op, '-' | '+' | '=' | '?'));
    }
    let Some(op) = rest.chars().next() else {
        return false;
    };
    matches!(op, '-' | '+' | '=' | '?') && rest.len() > op.len_utf8()
}

/// Walk the text before a `${...}` body and report (a) whether the body
/// sits inside double quotes and (b) whether it sits inside a command
/// substitution span. `quoted_word` marks words whose whole text is a
/// double-quoted region (the executor's `\x1d` marker). A `$(...)` span
/// starts a fresh quoting context: quoting inside it does not affect the
/// enclosing region, and it does not inherit the enclosing word's quotes
/// (`"x $(printf '%s ' ${v=a\ b})"` expands the body unquoted).
fn scan_word_prefix_quote_state(prefix: &str, quoted_word: bool) -> (bool, bool) {
    #[derive(Clone, Copy)]
    struct Frame {
        in_single: bool,
        in_double: bool,
    }
    let mut stack: Vec<Frame> = vec![Frame {
        in_single: false,
        in_double: quoted_word,
    }];
    let chars: Vec<char> = prefix.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let frame = *stack.last().expect("quote frame stack is never empty");
        if frame.in_single {
            if chars[index] == '\'' {
                stack.last_mut().unwrap().in_single = false;
            }
            index += 1;
            continue;
        }
        if frame.in_double {
            match chars[index] {
                '\\' => index += 2,
                '"' => stack.last_mut().unwrap().in_double = false,
                '$' if chars.get(index + 1) == Some(&'(') => {
                    stack.push(Frame {
                        in_single: false,
                        in_double: false,
                    });
                    index += 2;
                }
                _ => index += 1,
            }
            continue;
        }
        match chars[index] {
            '\'' => {
                stack.last_mut().unwrap().in_single = true;
                index += 1;
            }
            '"' => {
                stack.last_mut().unwrap().in_double = true;
                index += 1;
            }
            '`' => {
                // Old-style substitution: skip to the closing backtick.
                if let Some(close) = chars[index + 1..].iter().position(|c| *c == '`') {
                    index += close + 2;
                    continue;
                }
                break;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                stack.push(Frame {
                    in_single: false,
                    in_double: false,
                });
                index += 2;
                continue;
            }
            '\\' => {
                index += 2;
                continue;
            }
            ')' if stack.len() > 1 => {
                stack.pop();
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }
    let top = *stack.last().expect("quote frame stack is never empty");
    (top.in_double, stack.len() > 1)
}

/// End index (exclusive) of the substitution unit starting at
/// chars[start] == '$' with '(' or '{' at start + 1, or None when the
/// unit is unterminated — the caller then falls back to per-character
/// handling, preserving the historical behavior for broken input.
fn substitution_unit_end(chars: &[char], start: usize, posix: bool) -> Option<usize> {
    if chars.get(start + 1) == Some(&'(') {
        crate::lexer::skip_parenthesized_unit_corrected(chars, start + 1)
    } else {
        let context = crate::lexer::dolbrace::BraceContext {
            outer_double_quote: true,
            posix,
            replacement_context: false,
            initial_state: crate::lexer::dolbrace::DolbraceState::Param,
        };
        crate::lexer::dolbrace::scan_braced_parameter_body_chars(&chars[start..], context)
            .map(|scan| start + 2 + scan.end)
            .filter(|end| *end <= chars.len())
    }
}

/// End index (exclusive) of the backtick unit starting at chars[start]:
/// a backslash escapes the next character (including the closing
/// backtick); an unterminated unit runs to the end of the word.
fn backtick_unit_end(chars: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '`' => return index + 1,
            _ => index += 1,
        }
    }
    chars.len()
}

pub(in crate::executor) fn decode_double_quotes_in_quoted_parameter_word(
    word: &str,
    posix: bool,
    heredoc: bool,
) -> String {
    let mut output = String::new();
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut in_sq = false;
    while index < chars.len() {
        // POSIX mode keeps ' as a real single-quote delimiter inside
        // "${v op w}" (parse.y:4036 Austin Group interp 221), so a `'`
        // there opens/closes a quoted span — a `$'` only starts ANSI-C
        // quoting when the ' is not a span closer (GNU's LEX_WASDOL check
        // in parse_matched_pair only sees delimiters).
        if in_sq {
            if chars[index] == '\'' {
                in_sq = false;
            }
            output.push(chars[index]);
            index += 1;
            continue;
        }
        // GNU parse.y:4451 parse_comsub / parse.y:3877 parse_matched_pair:
        // a `$(...)` or `` `...` `` unit inside the ${...} word is a
        // self-contained parse unit — its interior quoting belongs to the
        // substitution's own grammar and is re-read fresh when the body
        // executes (subst.c:7143 command_substitute -> parse_and_execute).
        // The flat quote marking below (a `'` becomes DATA_SQUOTE in
        // non-POSIX mode, a `$'...'` is ansiexpanded) must not cross the
        // unit boundary, or the body's own quote characters reach the
        // inner parse as literal data (`"${u:-$(printf 'x y')}"` expanded
        // to `'x`). Copy the whole unit verbatim (rubash#1455).
        if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
            let end = crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                .unwrap_or(chars.len());
            output.extend(&chars[index..end]);
            index = end;
            continue;
        }
        if chars[index] == '`' {
            let mut cursor = index + 1;
            output.push('`');
            while cursor < chars.len() {
                let ch = chars[cursor];
                output.push(ch);
                cursor += 1;
                if ch == '\\' && cursor < chars.len() {
                    output.push(chars[cursor]);
                    cursor += 1;
                } else if ch == '`' {
                    break;
                }
            }
            index = cursor;
            continue;
        }
        // GNU parse.y:4053-4070 (parse_matched_pair): inside a `${...}`
        // grouping construct a `$'...'` body is ansiexpanded at extraction
        // time and wrapped in sh_single_quote so the decoded bytes ride out
        // quote removal as data. Emit each decoded char in its data-carrier
        // form so the expansion walker never re-reads it as syntax; without
        // this the `'` is marked DATA_SQUOTE below and `$'\t'` inside
        // "${v-word}" leaks literal text (nquote.tests).
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') && !heredoc {
            let mut body = String::new();
            let mut cursor = index + 2;
            let mut closed = false;
            while cursor < chars.len() {
                let ch = chars[cursor];
                if ch == '\\' && cursor + 1 < chars.len() {
                    body.push(ch);
                    body.push(chars[cursor + 1]);
                    cursor += 2;
                    continue;
                }
                if ch == '\'' {
                    closed = true;
                    cursor += 1;
                    break;
                }
                body.push(ch);
                cursor += 1;
            }
            if !closed {
                output.push('$');
                index += 1;
                continue;
            }
            index = cursor;
            for ch in crate::lexer::decode_ansi_c_quoted(&body).chars() {
                match ch {
                    '\'' => output.push(crate::executor::markers::DATA_SQUOTE),
                    '"' => output.push(crate::executor::markers::DATA_DQUOTE),
                    '\\' => output.push(crate::executor::markers::DATA_BACKSLASH),
                    '$' => output.push(DATA_DOLLAR),
                    '`' => output.push(crate::executor::markers::DATA_BACKTICK),
                    ' ' | '\t' | '\n' => {
                        output.push(crate::executor::markers::IFS_GLUE);
                        output.push(ch);
                    }
                    _ => output.push(ch),
                }
            }
            continue;
        }
        // A backslash escape outside a double-quote span survives quote
        // removal intact: the expansion pass turns it into protected data
        // (`\"` yields a literal quote, posixexp2 case 8). Dropping the
        // escaped quote here made the expansion output an unmarked bare
        // quote that later stages swallowed.
        if chars[index] == '\\'
            && index + 1 < chars.len()
            && (matches!(chars[index + 1], '$' | '`' | '"' | '\\' | '}' | '\n')
                // GNU parse.y dolbrace is POSIX-only: outside POSIX mode a
                // `'` inside "${var op word}" is literal data, so \' is an
                // escape pair producing a literal quote.
                || (!posix && chars[index + 1] == '\''))
        {
            // `\\` becomes the escaped-backslash marker (\x14) so the
            // expansion walker treats it as data, not as an escape for
            // the following character (GNU slashify_in_quotes: `\\` → `\`
            // but does NOT escape `$`; rhs-exp: `\\$selvecs` → `\&m68kcoff_vec`).
            if chars[index + 1] == '\\' {
                output.push(crate::executor::markers::DATA_BACKSLASH);
            } else if chars[index + 1] == '\'' {
                // GNU retains the backslash: `\'` inside a double-quoted
                // "${var op word}" survives quote removal as literal
                // backslash+quote (rhs-exp.tests `\'$selvecs\'`).
                output.push(crate::executor::markers::DATA_BACKSLASH);
                output.push(crate::executor::markers::DATA_SQUOTE);
            } else {
                output.push(chars[index]);
                output.push(chars[index + 1]);
            }
            index += 2;
            continue;
        }
        if chars[index] == '\'' && !posix && !heredoc {
            // Non-POSIX "${var op word}": ' is literal text, never an sq
            // opener. Emit it escaped so the expansion pass yields a data
            // quote and any following $( still expands (braces.tests).
            output.push(crate::executor::markers::DATA_SQUOTE);
            index += 1;
            continue;
        }
        if chars[index] == '\'' && posix && !heredoc {
            in_sq = true;
            output.push(chars[index]);
            index += 1;
            continue;
        }
        // GNU parse.y:4451 parse_comsub / parse.y:3877
        // parse_matched_pair: a nested substitution inside the alternate
        // is a unit whose interior quotes belong to its own parse.
        // Consume $(...), ${...} and `...` verbatim — decoding an
        // interior `"` as an outer-context span corrupted the body
        // (`"${u:-$(printf "it's")}"` lost the quotes around `it's`,
        // and the surviving `'` then swallowed the substitution's
        // closer downstream). rubash#1468.
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('(') | Some('{')) {
            if let Some(end) = substitution_unit_end(&chars, index, posix) {
                output.extend(chars[index..end].iter());
                index = end;
                continue;
            }
        }
        if chars[index] == '`' {
            let end = backtick_unit_end(&chars, index);
            output.extend(chars[index..end].iter());
            index = end;
            continue;
        }
        if chars[index] != '"' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        index += 1;
        while index < chars.len() {
            // Nested substitution units stay verbatim inside a
            // double-quote span too (rubash#1468): their interior
            // quotes are their own, never span boundaries.
            if chars[index] == '$' && matches!(chars.get(index + 1), Some('(') | Some('{')) {
                if let Some(end) = substitution_unit_end(&chars, index, posix) {
                    output.extend(chars[index..end].iter());
                    index = end;
                    continue;
                }
            }
            if chars[index] == '`' {
                let end = backtick_unit_end(&chars, index);
                output.extend(chars[index..end].iter());
                index = end;
                continue;
            }
            match chars[index] {
                '"' => {
                    index += 1;
                    break;
                }
                '\\' if matches!(chars.get(index + 1), Some('\\' | '"' | '$' | '`' | '\n')) => {
                    let escaped = chars[index + 1];
                    index += 2;
                    match escaped {
                        '\n' => {}
                        // `\\` becomes the escaped-backslash marker (\x14)
                        // so the expansion walker treats it as data, not as
                        // an escape for the following character (GNU
                        // slashify_in_quotes: `\\` → `\` but does NOT escape
                        // `$`; rhs-exp: `"\\$selvecs"` → `\&m68kcoff_vec`).
                        '\\' => {
                            output.push(crate::executor::markers::DATA_BACKSLASH);
                        }
                        // Protect `$` and `` ` `` from re-expansion: inside
                        // double quotes `\$` and `\`` are literal data that
                        // must not trigger parameter/command substitution.
                        '$' => output.push(DATA_DOLLAR),
                        '`' => output.push(crate::executor::markers::DATA_BACKTICK),
                        _ => output.push(escaped),
                    }
                }
                // GNU expand_word_internal with Q_DOUBLE_QUOTES: backslash
                // before a char NOT in CBSDQUOTE ($ ` " \ newline) is removed
                // — only the char survives (rhs-exp.tests: `\p` → `p`,
                // `\'` → `'`).
                '\\' => {
                    if let Some(&next) = chars.get(index + 1) {
                        index += 2;
                        output.push(next);
                    } else {
                        output.push('\\');
                        index += 1;
                    }
                }
                ch => {
                    output.push(ch);
                    index += 1;
                }
            }
        }
    }
    output
}

fn unescape_double_quoted_backslashes(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        // The escaped-backslash marker (\x14) from decode_double_quotes...
        // resolves to a literal backslash here. The mut path already did
        // this via restore_protected_replacement_quotes; the non-mut path
        // reaches this function with \x14 still intact.
        if ch == crate::executor::markers::DATA_BACKSLASH {
            output.push('\\');
            continue;
        }
        if ch == '\\' {
            if let Some(next) = chars.peek().copied() {
                // `}` joins the escapable set because a double-quoted
                // ${...} alternate already lost one escaping level while
                // the body was extracted (subst.c): GNU gives `}z` for
                // `"${IFS+\}z}"` (posixexp2 cases 9/14/15).
                // `\\` is no longer in the set: decode_double_quotes...
                // already converted `\\` to \x14, so a bare `\\` reaching
                // here is two literal backslashes (from \x\x or expansion
                // data) and must NOT be collapsed to `\`.
                if matches!(next, '$' | '`' | '"' | '}' | '\n') {
                    chars.next();
                    if next != '\n' {
                        output.push(next);
                    }
                    continue;
                }
            }
        }
        output.push(ch);
    }
    output
}

// Post-expansion backslash handling for a ${op word} alternate. Quote
// removal already ran on the raw alternate (decode_double_quotes...); the
// expansion result itself is data and only needs its remaining escapes
// resolved (GNU subst.c never quote-removes expansion results).
pub(in crate::executor) fn unescape_parameter_operator_result(
    word: &str,
    context: SubstitutionQuoteContext,
    ifs: Option<&str>,
) -> String {
    let unescaped = if matches!(context, SubstitutionQuoteContext::DoubleQuoted) {
        unescape_double_quoted_backslashes(word)
    } else {
        unescape_remaining_shell_escapes(word)
    };
    // GNU parameter_brace_expand carries `quoted` into the operator word's
    // expansion (subst.c): inside "${name op word}" the whole result is
    // quote-protected, so its IFS characters are data for field splitting --
    // `"${x:-$(echo "foo bar")}"` stays one word (exp.tests:222).
    if matches!(context, SubstitutionQuoteContext::DoubleQuoted) {
        crate::executor::command_substitution_values::protect_ifs_field_chars(&unescaped, ifs)
    } else {
        unescaped
    }
}

#[cfg(test)]
mod scanner_tests {
    use super::scan_word_prefix_quote_state;

    #[test]
    fn cs_span_in_plain_word() {
        let word = "A: $(printf '<%s> ' ${w=a\\ b}) | x";
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], false);
        assert!(cs, "plain-word CS body must be inside_cs, dq={dq}");
        assert!(!dq);
    }

    #[test]
    fn cs_span_in_quoted_word() {
        let word = format!(
            "{}{}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR,
            "A: $(printf '<%s> ' ${w=a\\ b}) | x"
        );
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], true);
        assert!(cs);
        assert!(!dq, "CS-local quoting, not the outer dquote");
    }

    #[test]
    fn direct_dquote_body() {
        let word = format!(
            "{}{}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR,
            "${v=a\\ b}"
        );
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], true);
        assert!(dq);
        assert!(!cs);
    }

    #[test]
    fn cs_closes_and_next_body_is_outer() {
        let word = format!(
            "{}{}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR,
            "A: $(f) ${v=a\\ b}"
        );
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], true);
        assert!(!cs, "body after the CS span is outer");
        assert!(dq);
    }
}
