use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    pub(in crate::executor) fn update_underscore_parameter(&mut self, cmd: &CommandNode) {
        if let Some(value) = cmd.words.last() {
            self.shell_state
                .env_vars
                .insert("_".to_string(), value.clone());
        }
    }

    pub(in crate::executor) fn removes_unquoted_null_word(
        &self,
        cmd: &CommandNode,
        index: usize,
    ) -> bool {
        if cmd.words.first().is_some_and(|word| word == "[[") {
            return false;
        }

        // GNU subst.c:12026-12035 expand_word_internal: a null result is
        // discarded only when the word carried no quoting — a quoted
        // `${...}` that expands empty (`"${foo:-$@}"` with no positional
        // parameters) still yields one empty field. raw_word_is_quoted
        // skips ${...}/$() bodies, so only the word's own outer quoting
        // counts here.
        if cmd.word_metadata.get(index).is_some_and(|metadata| {
            crate::executor::command_prepare::raw_word_is_quoted(Some(&metadata.raw))
        }) {
            return false;
        }

        cmd.word_kinds
            .get(index)
            .is_some_and(|kind| *kind == TokenKind::Variable)
            || cmd
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str())
                .or_else(|| cmd.words.get(index).map(String::as_str))
                .is_some_and(word_has_unquoted_command_substitution)
    }

    pub(in crate::executor) fn splits_unquoted_expanded_word(
        &self,
        cmd: &CommandNode,
        index: usize,
        expanded: &str,
    ) -> bool {
        // GNU general.c:480 (parse.y) marks `name[subscript]=value` an
        // ASSIGNMENT word, and an assignment word is never field-split: the
        // parser expands the subscript with expand_subscript_string and the
        // value as an assignment RHS, neither of which splits. An unquoted
        // `$(...)` or `$var` inside the SUBSCRIPT must therefore not break
        // the word into `A[m` + `n]=v` -- the expanded word is consumed
        // verbatim by execute_array_element_assignment.
        if cmd
            .array_element_assignments
            .iter()
            .any(|assignment| assignment.word_index == Some(index))
        {
            return false;
        }
        // GNU parse.y:5366 read_token_word / general.c:480 assignment(): a
        // declaration-builtin operand whose RAW token is assignment-shaped
        // (`name[sub]=value`) carries W_ASSIGNMENT and is never field-split,
        // even when the subscript expands to text containing IFS
        // whitespace (`declare A[$k]=v` stays one word). The check must use
        // the raw token — the expanded text has already lost the quoting
        // that would prove it is not an assignment word.
        if matches!(
            cmd.words.first().map(String::as_str),
            Some("export" | "readonly" | "declare" | "typeset" | "local")
        ) && cmd
            .word_metadata
            .get(index)
            .is_some_and(|metadata| Self::raw_word_is_assignment(&metadata.raw))
        {
            return false;
        }
        // A word wrapped in quotes (e.g. `"$(cmd) extra"`) keeps its spaces
        // together: quote removal happens after field splitting in Bash, so
        // quoted words must not be split even when they expand to whitespace.
        // Quote state comes from the raw word only: word_quotes (the
        // lexer quote segments) also records quotes nested inside a braced
        // parameter expansion (e.g. ${1-"$@"}), but those are word syntax
        // of the expansion, not outer quoting, so they must not suppress
        // field splitting of the unquoted word's result. raw_word_is_quoted
        // skips ${...}, $(), backticks and $'...'/$"..." bodies.
        let word_is_quoted = cmd
            .word_metadata
            .get(index)
            .map(|metadata| {
                crate::executor::command_prepare::raw_word_is_quoted(Some(&metadata.raw))
            })
            .unwrap_or(false);
        let unquoted_variable = cmd
            .word_kinds
            .get(index)
            .is_some_and(|kind| *kind == TokenKind::Variable)
            && !word_is_quoted;
        let unquoted_dynamic_parameter = unquoted_variable
            && cmd
                .words
                .get(index)
                .and_then(|word| dynamic_scalar_parameter_name(word))
                .is_some_and(|name| self.dynamic_parameter_is_set(name));
        let unquoted_command_substitution = cmd
            .word_metadata
            .get(index)
            .map(|metadata| metadata.raw.as_str())
            .or_else(|| cmd.words.get(index).map(String::as_str))
            .is_some_and(word_has_unquoted_command_substitution);
        let unquoted_indirect_name_list = cmd
            .words
            .get(index)
            .is_some_and(|word| word_is_unquoted_indirect_name_list(word));
        let unquoted_embedded_parameter = !word_is_quoted
            && cmd
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str())
                .or_else(|| cmd.words.get(index).map(String::as_str))
                .is_some_and(raw_word_has_unquoted_parameter_expansion);

        let field_split_values = self.field_split_values(expanded);
        let field_split_would_change_word = field_split_values.len() != 1
            || field_split_values
                .first()
                .is_some_and(|field| field != expanded);

        (unquoted_variable && !unquoted_dynamic_parameter && field_split_would_change_word)
            || (unquoted_embedded_parameter && field_split_would_change_word)
            || (unquoted_command_substitution && field_split_would_change_word)
            || (unquoted_indirect_name_list && field_split_would_change_word)
    }

    /// `${arr[@]+"${arr[@]}"}` (and the `${arr+"${arr[@]}"}` spelling
    /// bash-completion's _comp_get_words uses) whole-word guard — GNU
    /// subst.c `+` operator: a set array's alternative expands as the
    /// quoted array expansion, one word per element. Only the exact
    /// self-referential idiom is claimed; everything else falls through to
    /// the general expander.
    pub(in crate::executor) fn guarded_quoted_array_guard_values(
        &self,
        raw: Option<&str>,
    ) -> Option<Vec<String>> {
        let raw = raw?;
        let inner = raw.strip_prefix("${")?.strip_suffix("}")?;
        let plus = inner.find('+')?;
        let (guard_name, alt) = inner.split_at(plus);
        let alt = &alt[1..];
        // The alternative must be exactly the quoted array expansion
        // `"${name[@]}"` (or `[*]`), and the guard's name must be the same
        // array — with or without the element subscript (GNU checks the
        // same variable; `${arr+...}` and `${arr[@]+...}` both test
        // whether the array is set).
        let array_name = alt
            .strip_prefix("\"${")
            .and_then(|rest| rest.strip_suffix("[@]}\""))
            .or_else(|| {
                alt.strip_prefix("\"${")
                    .and_then(|rest| rest.strip_suffix("[*]}\""))
            })?;
        if !is_shell_name(array_name) {
            return None;
        }
        let guard_base = guard_name
            .strip_suffix("[@]")
            .or_else(|| guard_name.strip_suffix("[*]"))
            .unwrap_or(guard_name);
        if guard_base != array_name {
            return None;
        }
        Some(self.array_at_word_values(alt).unwrap_or_default())
    }

    pub(in crate::executor) fn expand_for_word_values_result(
        &mut self,
        word: &str,
        raw: Option<&str>,
        metadata: Option<&WordMetadata>,
    ) -> Result<Vec<String>, String> {
        // A fully single-quoted for-list item carries literal data only
        // (the lexer already removed the outer quotes); any double quotes
        // left in the value are characters, not operators. Without this
        // fast path expand_word re-removed them, so
        // `for testcmd in 'set -- ${foo="$*"}'` iterated over
        // `set -- ${foo=$*}` (posixexp5 lost the testcmd echo).
        if super::command_prepare::raw_word_is_fully_single_quoted(raw) {
            // The token value carries the protected-dollar marker; the walker
            // normally restores it, so do that here too (posixexp5 echoed the
            // raw marker instead of the dollar).
            return Ok(vec![word.replace('\u{1f}', "$")]);
        }
        let suppress_glob = word.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
            || word.starts_with(STORAGE_WORD_PREFIX)
            || super::command_prepare::raw_word_suppresses_pathname_expansion(raw, metadata);
        if let Some(values) = self.quoted_positional_at_word_values_with_raw(word, raw, None) {
            return Ok(values);
        }
        // GNU subst.c parameter_brace_expand's `+` arm: the alternative word
        // expands with full quoting, so the self-referential guard idiom
        // `${arr[@]+"${arr[@]}"}` (bash-completion's
        // ${v+"${a[@]}"} pattern, selfref.rubash bucket) yields one quoted
        // word per element — never IFS-split, never one joined argument.
        // The single-string operator expander cannot carry that shape.
        if let Some(values) = self.guarded_quoted_array_guard_values(raw) {
            return Ok(values);
        }
        if let Some(values) = self.array_at_word_values(word) {
            if word_is_unquoted_array_list_expansion(word) {
                return Ok(field_split_array_values_with_ifs(
                    values,
                    self.shell_state.env_vars.get("IFS").map(String::as_str),
                ));
            }
            return Ok(values);
        }
        // GNU runs brace expansion before parameter expansion; the brace
        // scanner skips dollar-brace bodies, so a dollar-brace in the word
        // does not suppress the split (foo{bar,${var.} -> foobar foobaz.).
        if self.is_brace_expand_enabled() {
            let braced = super::command_prepare::expand_braces_with_optional_raw(word, raw);
            if braced.len() > 1 {
                let values = braced
                    .into_iter()
                    .map(|word| self.expand_for_brace_word_values(&word, raw, suppress_glob))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
                    .collect();
                return Ok(values);
            }
        }

        self.expand_for_brace_word_values(word, raw, suppress_glob)
    }

    fn expand_for_brace_word_values(
        &mut self,
        word: &str,
        raw: Option<&str>,
        suppress_glob: bool,
    ) -> Result<Vec<String>, String> {
        let mut expanded = self.expand_word(word);
        if expanded.contains("<(") || expanded.contains(">(") {
            expanded = self
                .materialize_assignment_process_substitutions(&expanded)
                .unwrap_or(expanded);
        }
        if for_word_has_unquoted_expansion(word, raw) {
            // Field splitting follows $IFS (builtins/eval.def word_list /
            // word_expand: split on IFS whitespace and delimiters), not on
            // generic Unicode whitespace. `IFS=$'\001' for x in $a` must
            // split on \001 exactly like an external command's word list.
            return Ok(self.field_split_values(&expanded));
        }
        if suppress_glob {
            return Ok(vec![expanded]);
        }
        // Apply glob expansion for for-loop words
        match glob::pathname_expand_word(&expanded, &self.shell_state.env_vars) {
            glob::PathnameExpansion::Matches(matches) => Ok(matches),
            glob::PathnameExpansion::NoMatch => Ok(vec![expanded]),
            glob::PathnameExpansion::Fail(pattern) => Err(pattern),
        }
    }

    pub(in crate::executor) fn field_split_values(&self, value: &str) -> Vec<String> {
        field_split_values_with_ifs(
            value,
            self.shell_state.env_vars.get("IFS").map(String::as_str),
        )
    }

    pub(in crate::executor) fn expand_escaped_indirect_parameter_literal(
        &self,
        value: &str,
    ) -> Option<String> {
        let marker = "\\${$";
        let start = value.find(marker)?;
        let mut output = String::new();
        output.push_str(&value[..start]);
        let mut index = start + marker.len();
        let rest = &value[index..];
        let mut name = String::new();
        for ch in rest.chars() {
            if !is_shell_name_char(ch) {
                break;
            }
            name.push(ch);
            index += ch.len_utf8();
        }
        if name.is_empty() {
            return None;
        }
        let tail = &value[index..];
        let end = tail.find('}')?;
        let resolved = self.expand_embedded_parameters(&format!("${name}"));
        output.push_str("${");
        output.push_str(&resolved);
        output.push_str(&tail[..end]);
        output.push('}');
        output.push_str(&tail[end + 1..]);
        Some(output)
    }
}

fn dynamic_scalar_parameter_name(word: &str) -> Option<&str> {
    let name = word
        .strip_prefix("${")
        .and_then(|word| word.strip_suffix('}'))
        .or_else(|| word.strip_prefix('$'))?;
    is_shell_name(name).then_some(name)
}

fn word_is_unquoted_indirect_name_list(word: &str) -> bool {
    let Some(inner) = crate::executor::parameter_ops::whole_word_braced_parameter_body(word)
        .and_then(|body| body.strip_prefix('!'))
    else {
        return false;
    };

    inner
        .strip_suffix("[@]")
        .or_else(|| inner.strip_suffix("[*]"))
        .is_some_and(|name| !name.is_empty())
        || inner
            .strip_suffix('*')
            .or_else(|| inner.strip_suffix('@'))
            .is_some_and(|prefix| !prefix.is_empty())
}

pub(in crate::executor) fn raw_word_has_unquoted_parameter_expansion(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '$' {
            match chars.get(index + 1).copied() {
                Some('{') => {
                    if chars.get(index + 2).is_some_and(|ch| {
                        is_shell_name_start(*ch)
                            || matches!(*ch, '@' | '*' | '#' | '?' | '$' | '!' | '-' | '0')
                    }) {
                        return true;
                    }
                }
                Some(ch)
                    if is_shell_name_start(ch)
                        || matches!(ch, '@' | '*' | '#' | '?' | '$' | '!' | '-' | '0') =>
                {
                    return true;
                }
                _ => {}
            }
        }
        index += 1;
    }
    false
}
