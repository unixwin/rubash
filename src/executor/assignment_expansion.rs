use super::*;
use crate::executor::embedded_mutations::collect_command_substitution_source;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};
use crate::lexer::dolbrace::{scan_braced_parameter_body, BraceContext, DolbraceState};

/// Hoisted data-quote sentinels: expand_assignment_value_inner lifts the
/// lexer's \x17/\x18 escaped-quote carriers out of the embedded-parameter
/// walker so they are not re-read as quote syntax (subst.c:4692
/// dequote_escapes keeps CTLESC-escaped quotes as data). The embedded
/// walker still tracks DATA_DOUBLE_QUOTE as a "..." region boundary for
/// its in_double state so `$'` inside it stays literal (issue #109).
pub(in crate::executor) const DATA_DOUBLE_QUOTE: &'static str =
    crate::executor::markers::ASSIGN_DATA_DQUOTE_STR;

#[derive(Debug, Eq, PartialEq)]
pub(in crate::executor) struct AssignmentExpansionResult {
    pub(in crate::executor) value: String,
    pub(in crate::executor) substitution_status: Option<i32>,
    pub(in crate::executor) arithmetic_error: bool,
    pub(in crate::executor) arithmetic_nonfatal_error: bool,
}

/// Hoist raw `"` quote DATA to `marker` before assignment expansion, but
/// leave the quotes inside a `${...}` body alone: those are quoting
/// operators owned by the parameter-expansion pipeline downstream (the
/// patsub replacement scanner mark_patsub_replacement_quotes consumes them,
/// exactly as it does on the echo path). Quote removal keeps `${...}`
/// bodies lexically intact (copy_braced_parameter_unquoted), and
/// single-quoted segment data can never contain a raw `${` (the lexer
/// protects those dollars as \x1f), so a plain `${` here is always a real
/// expansion body (array6.sub: a2=("${a[@]/#/"-iname '"}")).
/// Replace `"` with `marker` but skip a `"` preceded by a backslash (`\"`),
/// which is element-level data in a compound array assignment (unicode1.sub
/// `[0x0022]=\"` must stay `\"`, not become `\<marker>`).
fn replace_unescaped_double_quotes(value: &str, marker: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        if ch == '"' {
            out.push_str(marker);
        } else {
            out.push(ch);
        }
    }
    out
}

pub(in crate::executor) fn hoist_data_double_quotes(value: &str, marker: &str) -> String {
    // Simple fast path: no `${...}`, no `$'...'`, no `$(...)`, no backtick —
    // just replace all ". We must skip these constructs because a " inside
    // them is not outer-word data:
    //   * $'...' (ANSI-C quotes): a " inside is data, and hoisting it
    //     corrupts the ANSI-C decode (issue #109: x=($'a"b') stored "ab"
    //     instead of "a\"b").
    //   * $(...) and `...` (command substitution): a " inside is syntax for
    //     the nested parse (GNU parse.y:4451 parse_comsub re-parses the body
    //     with its own quote state). Hoisting it turns the inner quote into
    //     literal data (issue #119: `echo "x=[$(echo "y")]"` printed
    //     `x=["y"]` instead of `x=[y]`).
    // A backslash-escaped `\"` is element-level data, not a syntax quote:
    // hoisting it corrupts compound array elements like `[0x0022]=\"`
    // (unicode1.sub) by turning `\"` into `\<marker>`, which the storage
    // tokenizer mis-parses. Skip a `"` preceded by a backslash.
    if !value.contains("${")
        && !value.contains("$'")
        && !value.contains("$(")
        && !value.contains('`')
    {
        return replace_unescaped_double_quotes(value, marker);
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    loop {
        // Find the nearest of "${", "$'", "$(", or "`"
        let pos = ["${", "$'", "$(", "`"]
            .iter()
            .filter_map(|needle| rest.find(needle))
            .min();
        let Some(pos) = pos else {
            out.push_str(&replace_unescaped_double_quotes(rest, marker));
            return out;
        };
        // Replace " in the segment before the special construct
        out.push_str(&replace_unescaped_double_quotes(&rest[..pos], marker));
        if rest[pos..].starts_with("${") {
            let body_start = pos + 2;
            match matching_parameter_brace(&rest[body_start..]) {
                Some(close) => {
                    out.push_str(&rest[pos..body_start + close + 1]);
                    rest = &rest[body_start + close + 1..];
                }
                None => {
                    out.push_str(&rest[pos..]);
                    return out;
                }
            }
        } else if rest[pos..].starts_with("$(") {
            // Command substitution: the body's own quote state governs the
            // closing paren (GNU parse.y:4451 parse_comsub), so reuse the
            // storage-level collector — it already tracks quotes, comments,
            // heredocs and case-pattern parens. It consumes through the
            // closing `)`, so the remaining length locates the span; on
            // unterminated input the whole rest is consumed.
            let body = &rest[pos + 2..];
            let mut chars = body.chars().peekable();
            let _ =
                collect_command_substitution_source(&mut chars, &std::collections::HashMap::new());
            let remaining: usize = chars.map(|ch| ch.len_utf8()).sum();
            let consumed = body.len() - remaining;
            let end = pos + 2 + consumed;
            out.push_str(&rest[pos..end]);
            rest = &rest[end..];
        } else if rest[pos..].starts_with("$'") {
            // $'...' ANSI-C quote: skip everything until the closing '
            let body_start = pos + 2;
            match rest[body_start..].find('\'') {
                Some(close) => {
                    out.push_str(&rest[pos..body_start + close + 1]);
                    rest = &rest[body_start + close + 1..];
                }
                None => {
                    // Unterminated $'...: keep the rest verbatim
                    out.push_str(&rest[pos..]);
                    return out;
                }
            }
        } else {
            // Backtick command substitution: skip to the next unescaped `.
            let body_start = pos + 1;
            let mut escaped = false;
            let mut close = None;
            for (offset, ch) in rest[body_start..].char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                } else if ch == '`' {
                    close = Some(offset);
                    break;
                }
            }
            match close {
                Some(close) => {
                    out.push_str(&rest[pos..body_start + close + 1]);
                    rest = &rest[body_start + close + 1..];
                }
                None => {
                    out.push_str(&rest[pos..]);
                    return out;
                }
            }
        }
    }
}

/// Hoist raw `'` quote DATA to `marker` before compound-assignment expansion.
/// GNU arrayfunc.c:581 `parse_string_to_word_list` splits the compound
/// assignment into words with the shell parser, preserving the W_QUOTED flag
/// on each word. The expansion pass (`expand_words_no_vars`) then expands
/// each word individually. Rubash expands the entire compound body as one
/// string via `expand_embedded_parameters_mut`, which performs quote removal
/// — consuming `'` delimiters and outputting their content bare. That
/// destroys single-quote word grouping: `foo=('a b' 1 "$v1" 2)` loses the
/// `'a b'` boundary and `a b` splits into two words. Hoisting `'` to a
/// sentinel before expansion (and restoring after) preserves the boundary
/// exactly as `hoist_data_double_quotes` does for `"`.
///
/// `$'...'` ANSI-C quotes use `'` as delimiters; those must NOT be hoisted
/// or the construct breaks (assoc15.sub: `[$'\001']=$'\001\001\001\001'`).
/// `${...}` bodies are also preserved verbatim, matching
/// `hoist_data_double_quotes`.
/// Sentinels for expansion-trigger characters inside a hoisted `'...'`
/// span (GNU W_QUOTED - the span content never expands). Restored by the
/// same callers that restore the quote `marker`.
pub(in crate::executor) const SQ_DOLLAR_DATA: &'static str =
    crate::executor::markers::ASSIGN_SQ_DOLLAR_STR;
pub(in crate::executor) const SQ_BACKTICK_DATA: &'static str =
    crate::executor::markers::ASSIGN_SQ_BACKTICK_STR;
pub(in crate::executor) const SQ_BACKSLASH_DATA: &'static str =
    crate::executor::markers::ASSIGN_SQ_BACKSLASH_STR;

pub(in crate::executor) fn restore_sq_content_markers(value: String) -> String {
    value
        .replace(SQ_DOLLAR_DATA, "$")
        .replace(SQ_BACKTICK_DATA, "`")
        .replace(SQ_BACKSLASH_DATA, "\\")
}

pub(in crate::executor) fn hoist_data_single_quotes(value: &str, marker: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let chars_vec: Vec<(usize, char)> = value.char_indices().collect();
    let mut idx = 0;
    let mut in_span = false;
    while idx < chars_vec.len() {
        let (_, ch) = chars_vec[idx];
        // Skip $'...' ANSI-C quote bodies: the ' delimiters belong to the
        // construct, not to single-quote word grouping.
        if ch == '$' && idx + 1 < chars_vec.len() && chars_vec[idx + 1].1 == '\'' {
            out.push('$');
            out.push('\'');
            idx += 2;
            while idx < chars_vec.len() {
                let (_, c) = chars_vec[idx];
                if c == '\\' && idx + 1 < chars_vec.len() {
                    out.push(c);
                    out.push(chars_vec[idx + 1].1);
                    idx += 2;
                    continue;
                }
                out.push(c);
                if c == '\'' {
                    idx += 1;
                    break;
                }
                idx += 1;
            }
            continue;
        }
        // Skip ${...} bodies: quotes inside are parameter-expansion syntax.
        if ch == '$' && idx + 1 < chars_vec.len() && chars_vec[idx + 1].1 == '{' {
            let body_start = chars_vec[idx + 1].0 + 1;
            if let Some(close) = matching_parameter_brace(&value[body_start..]) {
                out.push_str(&value[chars_vec[idx].0..body_start + close + 1]);
                idx = chars_vec
                    .iter()
                    .position(|(offset, _)| *offset >= body_start + close + 1)
                    .unwrap_or(chars_vec.len());
                continue;
            }
        }
        if ch == '\'' {
            out.push_str(marker);
            in_span = !in_span;
        } else if in_span {
            // GNU parse_string_to_word_list (arrayfunc.c:581) marks the
            // whole '...' word W_QUOTED, so the expansion pass never sees
            // the span's `$`/backtick/`\` - hoisting only the
            // delimiters would still let '$xtra' expand (assoc12.sub).
            // Carry the inner expansion triggers to sentinels too; the
            // caller restores them alongside the quote marker.
            match ch {
                '$' => out.push_str(SQ_DOLLAR_DATA),
                '`' => out.push_str(SQ_BACKTICK_DATA),
                '\\' => out.push_str(SQ_BACKSLASH_DATA),
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
        idx += 1;
    }
    out
}

/// Hoist `\\` (escaped backslash) to `marker` before compound-assignment
/// expansion. GNU arrayfunc.c:581 `parse_string_to_word_list` splits the
/// compound body into words FIRST, then `expand_words_no_vars` expands each
/// word individually — so `\\` is a standalone word that produces `\`.
/// Rubash expands the whole body as one string via
/// `expand_embedded_parameters_mut`, which turns `\\` into `\`; the remaining
/// `\` then escapes the following space in `split_storage_words`, merging
/// `\\ 5` into ` 5` instead of two words `\` and `5` (assoc11.sub line 30).
/// Hoisting `\\` to a sentinel before expansion preserves it for
/// `split_storage_words` to handle correctly.
/// `${...}` and `$'...'` bodies are skipped (their backslashes are syntax).
pub(in crate::executor) fn hoist_data_backslashes(value: &str, marker: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let chars: Vec<(usize, char)> = value.char_indices().collect();
    let mut idx = 0;
    while idx < chars.len() {
        let (_, ch) = chars[idx];
        // Skip $'...' ANSI-C quote bodies
        if ch == '$' && idx + 1 < chars.len() && chars[idx + 1].1 == '\'' {
            out.push('$');
            out.push('\'');
            idx += 2;
            while idx < chars.len() {
                let (_, c) = chars[idx];
                if c == '\\' && idx + 1 < chars.len() {
                    out.push(c);
                    out.push(chars[idx + 1].1);
                    idx += 2;
                    continue;
                }
                out.push(c);
                if c == '\'' {
                    idx += 1;
                    break;
                }
                idx += 1;
            }
            continue;
        }
        // Skip ${...} bodies
        if ch == '$' && idx + 1 < chars.len() && chars[idx + 1].1 == '{' {
            let body_start = chars[idx + 1].0 + 1;
            if let Some(close) = matching_parameter_brace(&value[body_start..]) {
                out.push_str(&value[chars[idx].0..body_start + close + 1]);
                idx = chars
                    .iter()
                    .position(|(offset, _)| *offset >= body_start + close + 1)
                    .unwrap_or(chars.len());
                continue;
            }
        }
        // Hoist \\ (escaped backslash) to sentinel
        if ch == '\\' && idx + 1 < chars.len() && chars[idx + 1].1 == '\\' {
            out.push_str(marker);
            idx += 2;
            continue;
        }
        out.push(ch);
        idx += 1;
    }
    out
}

impl Executor {
    pub(in crate::executor) fn expand_assignment_value_result(
        &mut self,
        name: &str,
        value: &str,
    ) -> AssignmentExpansionResult {
        self.last_command_substitution_status.set(None);
        let expanded = self.expand_assignment_value(name, value);
        let substitution_status = self.last_command_substitution_status.get();
        self.last_command_substitution_status.set(None);
        let arithmetic_error = self.shell_state.arithmetic_expansion_error.replace(false);
        let arithmetic_nonfatal_error = self.shell_state.arithmetic_nonfatal_error.replace(false);
        AssignmentExpansionResult {
            value: expanded,
            substitution_status,
            arithmetic_error,
            arithmetic_nonfatal_error,
        }
    }

    /// Raw double quotes surviving in a token value are single-quote DATA at
    /// this point: remove_shell_quotes already consumed the active ones, so a
    /// remaining `"` can only come from a single-quoted segment (GNU keeps it
    /// literal in the assigned value). The downstream embedded-parameter
    /// re-scan would re-process it as syntax, so carry those quotes with the
    /// internal DATA_DOUBLE_QUOTE marker across expansion and restore them on
    /// the way out (assignment_expansion hoist/restore contract).
    pub(in crate::executor) fn expand_assignment_value(
        &mut self,
        name: &str,
        value: &str,
    ) -> String {
        // GNU subst.c param_expand carries PF_ASSIGNRHS through the whole
        // assignment value expansion (W_ASSIGNMENT words); key-list `@`
        // expansions read this flag to pick the dollar_at join. Command
        // substitution and subshells run on fresh Executor instances, so
        // the flag does not leak past a substitution boundary, matching
        // GNU dropping PF_ASSIGNRHS there.
        let saved_assignment_rhs = self.inside_assignment_rhs.replace(true);
        let expanded = self.expand_assignment_value_hoisting(name, value);
        self.inside_assignment_rhs.set(saved_assignment_rhs);
        expanded
    }

    fn expand_assignment_value_hoisting(&mut self, name: &str, value: &str) -> String {
        // Only hoist when no command-substitution payload is present: quotes
        // inside a $()/backtick body are syntax for the nested parse, not data.
        if (!value.contains('"') && !value.contains('\'') && !value.contains("\\\\"))
            || value.contains('`')
            || value.contains("$(")
            || contains_command_substitution_payload(value)
        {
            return self.expand_assignment_value_inner(name, value);
        }
        const DQ_DATA: &'static str = crate::executor::markers::ASSIGN_DATA_DQUOTE_STR;
        // NOTE: \u{E303}/\u{E304} are already taken below by DATA_BACKTICK /
        // DATA_ESCAPED_DQUOTE; these sentinels must use free codepoints.
        const SQ_DATA: &'static str = crate::executor::markers::ASSIGN_HOISTED_SQUOTE_STR;
        const BS_DATA: &'static str = crate::executor::markers::ASSIGN_HOISTED_BACKSLASH_STR;
        // GNU arrayfunc.c:581 parse_string_to_word_list preserves the
        // W_QUOTED flag on each compound-assignment word; the expansion pass
        // expands words individually. Rubash expands the whole body as one
        // string, whose quote removal consumes `'` and `"` delimiters and
        // destroys word grouping (`foo=('a b' 1 "$v1" 2)` would lose the
        // `'a b'` boundary). Hoist both quote families to sentinels before
        // expansion and restore after, exactly as DQ_DATA already did for `"`.
        // Similarly, `\\` (escaped backslash) must be hoisted or
        // expand_embedded_parameters_mut turns it into `\`, which then
        // escapes the following space in split_storage_words (assoc11.sub).
        let hoisted_dq = hoist_data_double_quotes(value, DQ_DATA);
        let hoisted_sq = hoist_data_single_quotes(&hoisted_dq, SQ_DATA);
        let hoisted_bs = hoist_data_backslashes(&hoisted_sq, BS_DATA);
        let expanded = self.expand_assignment_value_inner(name, &hoisted_bs);
        restore_sq_content_markers(
            expanded
                .replace(DQ_DATA, "\"")
                .replace(SQ_DATA, "'")
                .replace(BS_DATA, "\\\\"),
        )
    }

    /// GNU subst.c:4357 expand_string_assignment (reached with
    /// W_ASSIGNMENT from subst.c:11432): each unquoted element value of a
    /// compound assignment undergoes the assignment tilde pass (leading
    /// `~` and `~` after `:`). Quoted elements stay literal, and tilde
    /// text introduced by parameter expansion is never re-expanded because
    /// this pass sees the raw element text (array.tests: aa=([0]=~/a:~/b)
    /// stores the expanded paths while bb=([0]="~/a:~/b") stays literal).
    pub(in crate::executor) fn expand_tilde_in_compound_assignment(
        &self,
        name: &str,
        value: &str,
    ) -> String {
        let Some(inner) = value
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
        else {
            return value.to_string();
        };

        // GNU arrayfunc.c:630 assign_assoc_from_kvlist: an associative
        // kv-pair KEY expands through expand_subscript_string (leading `~`
        // only — `:`-tilde needs the W_ASSIGNMENT internal_tilde flag that
        // subst.c:11063 deliberately leaves unset), while the VALUE expands
        // through expand_assignment_string_to_string (`:`-tilde armed). An
        // indexed bare element goes through expand_words_no_vars
        // (arrayfunc.c:610) which has no `:`-tilde either. Only `[k]=v`
        // element values and odd-position assoc kvlist values take `:`-tilde.
        let base = name.strip_suffix('+').unwrap_or(name);
        let base = base.split('[').next().unwrap_or(base);
        let assoc = is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base);
        let tokens: Vec<String> = split_compound_element_words(inner);
        // kvpair_assignment_p (arrayfunc.c:665): a list is kv-pair form when
        // its first element does not open a `[subscript]=` element.
        let kvlist = tokens.first().is_some_and(|token| !token.starts_with('['));
        let mut elements: Vec<String> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let expand_after_colon = token.starts_with('[') || (assoc && kvlist && index % 2 == 1);
            elements.push(self.expand_compound_element_tilde(token, expand_after_colon));
        }
        format!("({})", elements.join(" "))
    }

    fn expand_compound_element_tilde(&self, token: &str, expand_after_colon: bool) -> String {
        const DQ_DATA: &'static str = crate::executor::markers::ASSIGN_DATA_DQUOTE_STR;
        let (prefix, element) = if token.starts_with('[') {
            match token.find("]=") {
                Some(offset) => {
                    // The `[k]` subscript takes the same expand_subscript_string
                    // pass as a direct element assignment (arrayfunc.c:815/865):
                    // a leading unquoted `~` in the key tilde-expands while
                    // `:`-tilde stays unarmed (assoc19.sub `[~/key]=v`).
                    let key = self.expand_compound_tilde_segment(&token[1..offset]);
                    (format!("[{key}]="), token[offset + 2..].to_string())
                }
                None => (String::new(), token.to_string()),
            }
        } else {
            (String::new(), token.to_string())
        };
        let element = element.as_str();
        // E307 is the hoisted single-quote sentinel (SQ_DATA) — a hoisted
        // quoted element is literal like a `'`/`"`-quoted one.
        if element.starts_with('\'')
            || element.starts_with('"')
            || element.starts_with(DQ_DATA)
            || element.starts_with('\u{E307}')
        {
            return token.to_string();
        }
        if !tilde_expand::assignment_value_needs_tilde_expansion(element, expand_after_colon) {
            return token.to_string();
        }
        // GNU subst.c:4357 expand_string_assignment: each `:`-separated
        // segment expands a leading `~`/`~word`; the `/rest` tail keeps
        // source status so `$x`/`$( )` there still expand later. The
        // expanded tilde text is DATA — GNU backslash-quotes expanded
        // subscript text for the same reason (sh_backslash_quote,
        // subst.c:11147): escape it so re-tokenization keeps `\`,
        // whitespace and metacharacters verbatim without suppressing the
        // tail's later expansions.
        let mut output = String::new();
        if expand_after_colon {
            let mut start = 0;
            for (index, ch) in element.char_indices() {
                if index == 0 || ch != ':' {
                    continue;
                }
                output.push_str(&self.expand_compound_tilde_segment(&element[start..index]));
                output.push(':');
                start = index + ch.len_utf8();
            }
            output.push_str(&self.expand_compound_tilde_segment(&element[start..]));
        } else {
            // expand_subscript_string / expand_words_no_vars semantics: only
            // a leading `~` expands; `:`-tilde is not armed.
            output.push_str(&self.expand_compound_tilde_segment(element));
        }
        format!("{prefix}{output}")
    }

    fn expand_compound_tilde_segment(&self, segment: &str) -> String {
        if !segment.starts_with('~') {
            return segment.to_string();
        }
        // GNU lib/tilde/tilde.c: the tilde word is `~`/`~word` up to `/`.
        let word_end = segment[1..]
            .find('/')
            .map(|i| i + 1)
            .unwrap_or(segment.len());
        let (tilde_word, rest) = segment.split_at(word_end);
        let expanded = tilde_expand::expand_tilde_segment(tilde_word, &self.shell_state.env_vars);
        let mut protected = String::with_capacity(expanded.len() * 2 + rest.len());
        for ch in expanded.chars() {
            if !(ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '/' | '.' | ':' | ',' | '%' | '@' | '+' | '-'))
            {
                protected.push('\\');
            }
            protected.push(ch);
        }
        protected.push_str(rest);
        protected
    }

    fn expand_assignment_value_inner(&mut self, name: &str, value: &str) -> String {
        // GNU expand_string_for_assignment (subst.c:4365) sets
        // expand_no_split_dollar_star=1 for the whole assignment-RHS
        // expansion, so `${*/a/x}` on the RHS joins with IFS[0]
        // (string_list_dollar_star) instead of the unquoted dollar_at space
        // join (array26.sub: `A=${*/a/x}` under IFS='' yields `xabb`).
        struct AssignmentRhsGuard(bool);
        impl Drop for AssignmentRhsGuard {
            fn drop(&mut self) {
                super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(self.0));
            }
        }
        let _assignment_rhs = AssignmentRhsGuard(
            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.replace(true)),
        );
        // One cross-pass subscript-eval memo scope per assignment value —
        // `${a[i++]:=x}` on an RHS is one GNU evaluation across the
        // pre-scan and the expansion below. Its own word context keeps
        // the RHS's fragment sites distinct from the command words'.
        let _xpass = crate::executor::expand_braced_indices::SubXpassFrame::new();
        let _wctx = crate::executor::expand_braced_indices::WordCtxGuard::new(
            crate::executor::expand_braced_indices::next_word_ctx(),
        );
        // The verbatim single-element fast path is only for storage-shaped
        // values without expansions: a compound value containing a
        // parameter expansion (e.g. (${!xx})) must reach the compound
        // expander below or the expansion text lands in the array as a
        // literal element (new-exp4.sub Case05).
        if !value.contains("$(") && !value.contains('`') && !value.contains('$') {
            if let Some(array_value) = normalize_single_element_array_assignment(value) {
                return array_value;
            }
        }

        let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
        let value = tilde_expand::strip_assignment_quote_marker(value);
        let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
        let value = value
            .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
            .unwrap_or(value);
        // A quoted value with no expansion syntax is already fully decoded
        // and must not go through the general expansion walker, which would
        // also run quote removal on quote syntax that the value legitimately
        // carries as DATA.
        //
        // The pending markers are NOT the same in both sub-cases, so the
        // restore below is not uniform:
        //   * A single-quoted word (`FOO='$$'`) reaches here with the
        //     walker's C0 carriers still in place -- the lexer stores `$`
        //     as U+001F, backtick as U+001A, backslash as U+0014. They are
        //     markers here, not data, and must be restored to the real
        //     characters. Skipping this is what leaked U+001F into storage
        //     and made `declare -x FOO` print the value as $'\037\037':
        //     declare/storage.rs::quote_declare_value emits ansic_quote for
        //     any value holding a control character, and a leaked carrier
        //     satisfied that test. GNU stores real `24 24` here, so the
        //     restore keeps rubash's storage byte-compatible with GNU's.
        //   * A $'...' word has already been ANSI-C decoded, so a byte in
        //     the C0 range is genuine DATA (`$'\037'` is a real U+001F) and
        //     restoring it would corrupt the value. Those words carry the
        //     PUA quote markers from escape_decoded_ansi_c_quotes instead.
        // The two families are disjoint, so restoring the C0 carriers
        // unconditionally is safe: a $'...' value never contains one,
        // because the decoder octal-escapes every control byte rather than
        // substituting a carrier for it.
        if quoted
            && !compound_assignment
            && !value.contains('$')
            && !value.contains('`')
            && !value.contains("$(")
        {
            // Order matters. The carriers are restored while the ANSI-C
            // data bytes are still tagged: a byte that survived decoding is
            // held as the U+E000 raw-byte marker pair, not as its literal
            // code point, so it cannot be mistaken for a carrier. Only
            // after the carriers are gone are the tags decoded into real
            // bytes (that final step is what makes $'\037' store a genuine
            // U+001F and render as $'\037', while '$$' stores 24 24 and
            // renders as "\$\$", both matching GNU).
            let restored = value
                .replace(DATA_DOLLAR, "$")
                .replace(crate::executor::markers::DATA_BACKTICK, "`")
                .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
                // `\"` and `'` inside double quotes travel as the walker's
                // data-quote markers (\x18 for \" and \x17 for ' inside "
                // quotes, quotes.rs skip_double_quoted / quoted=='\'' arm):
                // restoring them here is what keeps the stored value
                // byte-identical to GNU's `q"q` (niubash#103 regression —
                // commit 7ab91ffd introduced the \x18 marker but this fast
                // path never un-did it, so the value leaked U+0018 into
                // storage and into files written by printf). The PUA quote
                // markers below are the $'...' family and are disjoint.
                .replace(crate::executor::markers::DATA_DQUOTE, "\"")
                .replace(crate::executor::markers::DATA_SQUOTE, "'")
                .replace(crate::lexer::ANSI_C_QUOTE_MARKER_STR, "'")
                .replace(crate::lexer::ANSI_C_DQUOTE_MARKER_STR, "\"");
            // The lexer marks quoted glob metacharacters (*?[!@+) with a
            // leading CTLESC (\x11) so the glob engine treats them as data.
            // This fast path skips the general expander (which strips \x11),
            // so dequote here the same way glob.rs dequote_pathname does:
            // drop the \x11 sentinel and keep the following character.
            //
            // `restored` is already canonical storage form and must NOT go
            // through the bytes_to_shell_text(shell_text_to_raw_bytes(..))
            // round-trip it used before: that boundary treats every control
            // byte as data and re-encodes it as a U+E000 marker pair, which
            // also sweeps the lexer's marker-role C0 chars (0x17 hoisted
            // quote, 0x14 backslash, 0x1a backtick) into data pairs. The
            // downstream quote-restore passes then find no 0x17 to turn
            // back into a literal quote and the character is dropped
            // (x="a'b'c" stored aE000-pair bE000-pair c; echo "${x//\'/\'}"
            // printed abc instead of a'b'c, quote.tests 77-80/88).
            // Data bytes from $'...' stay as U+E000 pairs here -- exactly
            // how the ANSI-C decoder emits them -- and every consumer that
            // needs raw bytes decodes the pairs at its exact-once boundary
            // (shell_text_to_raw_bytes), so nothing is lost by keeping them.
            let restored = dequote_ctlesc(&restored);
            return restored;
        }
        // GNU parse.y FUNSUB_CHAR / subst.c param_expand: a `${ command; }` or
        // `${| command; }` span is a nofork substitution, not a parameter
        // form — and its body may itself hold `$(...)`, so it must reach the
        // embedded-parameter walker (which routes top-level funsub spans to
        // execution) before the `$(`/backtick fast paths below claim the
        // inner substitution and leave the `${` literal (comsub2.tests:
        // `x=${ echo ${ echo one;} $(echo two) }`).
        if !compound_assignment
            && crate::executor::parameter_core::word_contains_current_shell_command_substitution(
                value,
            )
            && crate::executor::parameter_core::funsub_span_is_top_level(value)
        {
            return self.expand_embedded_parameters_mut_with_context(
                value,
                if quoted {
                    SubstitutionQuoteContext::DoubleQuoted
                } else {
                    SubstitutionQuoteContext::Unquoted
                },
            );
        }
        // GNU subst.c:4357 expand_string_assignment (W_ASSIGNMENT,
        // subst.c:11432): unquoted element values of a compound assignment
        // undergo the assignment tilde pass on the RAW element text, before
        // parameter expansion, so tilde text produced by $params is never
        // re-expanded (array.tests: aa=([0]=~/a:~/b) expands both segments
        // while w=([0]=~/a [1]=$p) keeps $p's result literal). Quoted
        // elements stay literal; quoted whole-RHS values skip the pass.
        let tilde_value =
            if compound_assignment && !quoted && value.starts_with('(') && value.ends_with(')') {
                std::borrow::Cow::Owned(self.expand_tilde_in_compound_assignment(name, value))
            } else {
                std::borrow::Cow::Borrowed(value)
            };
        let value: &str = &tilde_value;
        // The previous early return for "\\$(" treated "\\$(" (literal backslash + comsub)
        // as a literal, breaking cases like c=\$\'\\$(printf ...)\' where the
        // "\\$(" is actually "\\" (escaped backslash) + "$(comsub)" that must expand.
        // Let the general expander handle escaping correctly (it distinguishes
        // "\$(" (escaped) from "\\$(" (backslash + comsub)).
        if quoted && value.contains(":$((") {
            return self.expand_quoted_prompt_arithmetic_assignment(value);
        }
        let value = if quoted && (value.contains("$(") || value.contains('`')) {
            strip_matching_quotes(value)
        } else {
            value
        };
        if quoted {
            if let Some(expanded) = self.expand_quoted_array_assignment_value(value) {
                return expanded;
            }
        }
        if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !value.contains('$')
            && !value.contains('`')
        {
            return format!("{COMPOUND_ASSIGNMENT_MARKER}{value}");
        }
        if !quoted && !compound_assignment {
            if let Some(expanded) = self.expand_fast_assignment_value(value) {
                return expanded;
            }
        }
        let apply_result = self.apply_parameter_assignment_expansions_in_word(value);
        if let Some(expanded) = self.expand_compound_positional_at_assignment(value, quoted) {
            if compound_assignment {
                return format!("{COMPOUND_ASSIGNMENT_MARKER}{expanded}");
            }
            return expanded;
        }
        if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(value) {
            if compound_assignment {
                return format!("{COMPOUND_ASSIGNMENT_MARKER}{expanded}");
            }
            return expanded;
        }

        if !compound_assignment && !value.starts_with("$((") && !value.starts_with("$[") {
            if let Some(source) = value
                .strip_prefix("$(")
                .and_then(|rest| rest.strip_suffix(')'))
                // GNU expands each $() span in the RHS separately ("$(a)$(b)"
                // concatenates two substitution outputs, subst.c string
                // extraction never spans across substitutions). Keep the
                // single-substitution fast path only for words that are
                // exactly one $() group; multi-span values fall through to
                // expand_mixed_command_substitution_assignment (issue
                // niubash#71).
                .filter(|_| command_substitution_spans_whole_word(value))
            {
                let result = self.expand_command_substitution_mut_typed_with_context(
                    source,
                    if quoted {
                        SubstitutionQuoteContext::DoubleQuoted
                    } else {
                        SubstitutionQuoteContext::Unquoted
                    },
                );
                return result.assignment_text();
            }
        }

        if !compound_assignment {
            if let Some(output) = self.expand_backtick_substitution_typed(value, quoted) {
                return output.assignment_text();
            }
            if let Some(separator) = value.find('=') {
                let (prefix, rhs) = value.split_at(separator);
                if is_shell_name(prefix) {
                    if let Some(output) = self.expand_backtick_substitution_typed(&rhs[1..], quoted)
                    {
                        return format!("{prefix}={}", output.assignment_text());
                    }
                }
            }
            if let Some(expanded) = self.expand_mixed_command_substitution_assignment(value) {
                return expanded;
            }
        }

        if let Some(expanded) = self.expand_backtick_substitution(value) {
            return expanded;
        }

        let expanded = if quoted {
            let expanded_value = self.expand_embedded_parameters_mut(value);
            // Prompt transforms consume Bash's `\!` and `\#` escapes after
            // parameter expansion. Keep those two quoted backslashes until
            // `${var@P}` reaches prompt_expansion; ordinary shell escapes
            // still undergo the normal assignment quote-removal pass.
            {
                let mut restored = preserve_prompt_escapes(&expanded_value)
                    .replace(crate::executor::markers::CTLESC, "");
                if value.contains([
                    crate::executor::markers::PROTECTED_ESCAPED_SQUOTE,
                    crate::executor::markers::DATA_SQUOTE,
                    crate::executor::markers::DATA_DQUOTE,
                ]) {
                    // No `\'`->`'` collapse here: a source `\'` already
                    // arrives as the bare \x17 carrier (lexer quotes.rs
                    // backslash arm), and a `\` sitting in front of the
                    // carrier is a LITERAL backslash that rode at the end
                    // of a single-quoted span — GNU parse.y parse_matched_pair
                    // keeps `\` as data inside '...' and dequote_string
                    // (subst.c:4807) removes only quote delimiters, so
                    // `'^([^\''`...` stores `^([^\'` with the backslash
                    // (rubash#144; collapsing it lost the backslash).
                    restored = restored
                        .replace(crate::executor::markers::PROTECTED_ESCAPED_SQUOTE, "'")
                        .replace(crate::executor::markers::DATA_SQUOTE, "'")
                        .replace(crate::executor::markers::DATA_DQUOTE, "\"");
                }
                restored
            }
        } else {
            // GNU strips quote syntax that parameter expansion introduced into
            // an unquoted assignment RHS (`v=${IFS+'}'z}` stores `}z`). Quotes
            // inside protected substitution payloads are data, so leave those
            // values alone. Escaped-quote markers (\x17 from \' and \x18 from
            // \" in the source word) are DATA quotes: parse.y records a
            // backslash-escaped quote as a quoted literal that survives quote
            // removal into the stored value (`x=a\'b` stores `a'b`). Hoist the
            // markers out of the quote-removal pass so the data quotes they
            // become are not re-stripped as syntax, then restore them.
            const DATA_SINGLE_QUOTE: &'static str =
                crate::executor::markers::ASSIGN_DATA_SQUOTE_STR;
            // GNU parse.y/arrayfunc.c: a compound array assignment preserves
            // the raw parenthesized text so split_storage_words sees the
            // original quoting. The embedded parameter walker treats a bare
            // backtick as command substitution and strips the backslash from
            // `\`` (and from `\"`), which corrupts compound elements like
            // `[0x0060]=\`` (unicode1.sub). Hoist escaped backticks (and the
            // backslash before `"`) out of the walker for compound
            // assignments so they survive as literal element text.
            let compound_paren_value = value.starts_with('(') && value.ends_with(')');
            const DATA_BACKTICK: &'static str = crate::executor::markers::ASSIGN_DATA_BACKTICK_STR;
            const DATA_ESCAPED_DQUOTE: &'static str =
                crate::executor::markers::ASSIGN_ESCAPED_DQUOTE_STR;
            const DATA_ESCAPED_SQUOTE: &'static str =
                crate::executor::markers::ASSIGN_ESCAPED_SQUOTE_STR;
            const DATA_ESCAPED_BACKSLASH: &'static str =
                crate::executor::markers::ASSIGN_ESCAPED_BACKSLASH_STR;
            // In preserve mode the walker emits escape pairs verbatim with
            // quote-context awareness, so hoisting `\X` here is both
            // redundant and wrong: a context-blind `.replace("\\'", ..)`
            // eats the backslash inside the single-quoted element `'\'`
            // and leaves an unclosed quote that swallows the rest of the
            // list (assoc11.sub). Only the \x17/\x18 quote sentinels still
            // need hoisting for this path.
            let hoisted_value = if compound_paren_value {
                value
                    .replace(crate::executor::markers::DATA_SQUOTE, DATA_SINGLE_QUOTE)
                    .replace(crate::executor::markers::DATA_DQUOTE, DATA_DOUBLE_QUOTE)
            } else {
                value
                    .replace(crate::executor::markers::DATA_SQUOTE, DATA_SINGLE_QUOTE)
                    .replace(crate::executor::markers::DATA_DQUOTE, DATA_DOUBLE_QUOTE)
            };
            // GNU arrayfunc.c:557 expand_compound_array_assignment tokenizes
            // the raw parenthesized text first; each element's own quote
            // syntax must survive the walker so split_storage_words sees the
            // same words GNU's tokenizer produced. The preserve variant
            // keeps '...'/"..." delimiters in the output instead of
            // dequoting them into bare quote data that the re-split would
            // read back as syntax (assoc11.sub: ('"' dquote "'" squote)).
            let expanded_value = if compound_paren_value {
                self.expand_compound_assignment_parameters_mut(&hoisted_value)
            } else {
                self.expand_embedded_parameters_mut(&hoisted_value)
            };
            // A compound assignment never takes a whole-value quote-removal
            // pass: element words carry their own quote structure through the
            // embedded walker, and quotes that patsub replacement produced as
            // DATA (array6.sub ${a[@]/#/-iname '"}) would be re-stripped as
            // syntax here.
            let compound_paren_value = value.starts_with('(') && value.ends_with(')');
            // GNU subst.c:4807 dequote_word only removes quote syntax from the
            // original word, not quotes introduced by parameter expansion
            // (those are CTLESC-protected). A bare `"` or `'` in the expanded
            // value that came from ${arr[0x0022]} (whose value is `"`) is DATA,
            // not syntax. Only strip when the original word carried quote
            // syntax (e.g. `v=${IFS+'}'z}` stores `}z`).
            let stripped = if !compound_paren_value
                && expanded_value.contains(['\'', '"'])
                && word_level_quote_syntax(&hoisted_value)
                && !contains_command_substitution_payload(&expanded_value)
            {
                crate::lexer::remove_shell_quotes(&expanded_value)
            } else {
                expanded_value.clone()
            };
            // GNU parse.y:5368-5397 read_token_word: a backslash outside any
            // quote removes itself and keeps the next char literal. In a
            // compound assignment, the backslash is part of the element
            // token structure and must survive for split_storage_words to
            // recognize it (unicode1.sub [0x0020]=\ stores a space, not an
            // empty element). unescape_remaining_shell_escapes would strip
            // the backslash from `\ `, turning it into a bare space that
            // the storage tokenizer treats as a field separator.
            let unescaped = if compound_paren_value {
                stripped
            } else {
                unescape_remaining_shell_escapes(&stripped)
            };
            unescaped
                .replace(DATA_SINGLE_QUOTE, "'")
                .replace(DATA_DOUBLE_QUOTE, "\"")
                .replace(DATA_BACKTICK, "\\`")
                .replace(DATA_ESCAPED_DQUOTE, "\\\"")
                .replace(DATA_ESCAPED_SQUOTE, "\\'")
                .replace(DATA_ESCAPED_BACKSLASH, "\\\\")
        };
        let mut expanded = decode_command_substitution_payload(&expanded);
        if expanded.contains("<(") || expanded.contains(">(") {
            if let Ok(materialized) = self.materialize_assignment_process_substitutions(&expanded) {
                expanded = materialized;
            }
        }
        if value.starts_with('(') && value.ends_with(')') {
            if compound_assignment {
                return format!("{COMPOUND_ASSIGNMENT_MARKER}{expanded}");
            }
            return expanded;
        }
        if value.contains('=') {
            return expanded;
        }

        if quoted {
            return expanded;
        }

        // GNU subst.c: expand_word_internal applies tilde expansion to the
        // RAW word before parameter expansion. A tilde that comes from
        // ${param} expansion is never re-expanded (unicode1.sub: EChar=${Array[0x7e]}
        // where the value is "~" must stay literal). GNU expands `~` at the
        // start of the RHS and after every `:` in an assignment value
        // (subst.c:11410-11460 internal_tilde + assignoff tracking).
        if !expanded.contains('=')
            && tilde_expand::assignment_value_needs_tilde_expansion(value, true)
        {
            self.expand_assignment_tilde(&expanded)
        } else {
            expanded
        }
    }

    fn expand_mixed_command_substitution_assignment(&mut self, value: &str) -> Option<String> {
        // GNU parse.y:4096-4125 rewrites $"..." into an ordinary
        // double-quoted string at parse time (locale_expand is the identity
        // without a translation catalog), so every later expansion pass sees
        // plain "..." (subst.c:1626-1649 re-decodes at quote removal). The
        // mixed-substitution splitter runs on raw word text where the $"
        // prefix would otherwise survive as a literal dollar plus a quoted
        // span (y=$"A$(echo B)C" stored `$"ABC` instead of `ABC`).
        let value = normalize_dollar_double_quotes(value);
        let all_spans = scan_substitution_spans(&value);
        if all_spans.is_empty() {
            return None;
        }
        // GNU subst.c:10345-10444 parameter_brace_expand: the operator RHS
        // of `${param:-word}` / `${param-word}` / `${param=word}` /
        // `${param?word}` is part of the `${...}` expansion unit. When the
        // parameter is set (non-null for the `:` forms) the RHS is freed
        // WITHOUT being expanded (subst.c:10385 FREE(value)) — nested
        // `$(...)`/backticks never execute; when it is used, the RHS is
        // expanded exactly once inside that unit. This splitter expands
        // pieces independently, so a span nested inside a top-level
        // `${...}` body must NOT be split out: running it here executed
        // skipped default values (side effects ran; output was appended
        // after the real value as `<value><output>}`, rubash#150) and ran
        // used `:=` words a second time. Leave the whole braced region to
        // the embedded-parameter walker, which consumes `${...}` as one
        // fragment with the operator's lazy semantics.
        let braced_regions = top_level_braced_parameter_regions(&value, &all_spans);
        let spans: Vec<_> = all_spans
            .into_iter()
            .filter(|span| {
                !braced_regions
                    .iter()
                    .any(|(start, end)| span.start >= *start && span.end <= *end)
            })
            .collect();
        if spans.is_empty() {
            return None;
        }
        let mut word = ExpandedWord::default();
        let mut cursor = 0usize;
        for span in spans {
            let raw = value.get(span.start..span.end)?;
            let prefix = self.expand_embedded_parameters_mut(value.get(cursor..span.start)?);
            word.append_literal(&prefix, true);
            let output = if let Some(source) = raw
                .strip_prefix("$(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                self.expand_command_substitution_mut_typed_with_context(source, span.context)
            } else if raw.starts_with('`') {
                self.expand_backtick_substitution_typed(
                    raw,
                    matches!(span.context, SubstitutionQuoteContext::DoubleQuoted),
                )?
            } else {
                return None;
            };
            word.append_substitution(output);
            cursor = span.end;
        }
        let suffix = self.expand_embedded_parameters_mut(value.get(cursor..)?);
        word.append_literal(&suffix, true);
        self.last_command_substitution_status.set(word.status);
        Some(word.materialize_lossy_at_boundary())
    }

    fn expand_fast_assignment_value(&mut self, value: &str) -> Option<String> {
        if let Some(expression) = value
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
            .filter(|expression| !expression.contains("${"))
        {
            let Some(value) = self.eval_arithmetic_command_value(expression) else {
                // GNU expr.c raises evalerror from the actual evaluation, so
                // the recorded real-environment category decides fatality.
                // A fresh-environment re-evaluation would lose state-dependent
                // errors like `x+=2` on a declared integer, and a `set -u`
                // unbound variable must stay fatal even though a fresh
                // environment would happily evaluate it as 0.
                let actual_fatal = self
                    .shell_state
                    .arithmetic_last_error_category
                    .take()
                    .is_some()
                    || self.shell_state.arithmetic_nounset_error.get();
                if !actual_fatal
                    && !crate::executor::arithmetic::arithmetic_expansion_is_fatal(expression)
                {
                    self.shell_state.arithmetic_nonfatal_error.set(true);
                }
                if self.shell_state.arithmetic_nounset_error.get() {
                    // `set -u` unbound is script-fatal (command_prepare turns
                    // the recorded flag into ExitCode). Returning an empty
                    // value here stops the slower assignment expanders from
                    // re-processing the `$(( ))` text as a command
                    // substitution, which produced a spurious
                    // `b: command not found` (issue #67).
                    return Some(String::new());
                }
                return None;
            };
            return Some(self.expand_assignment_tilde_if_needed(value.to_string()));
        }

        let parameter = value.strip_prefix('$')?;
        if parameter.len() != 1 {
            return None;
        }

        let expanded = match parameter.as_bytes()[0] {
            b'0' => self.script_name_value(),
            b'1'..=b'9' => {
                let index = usize::from(parameter.as_bytes()[0] - b'0' - 1);
                self.shell_state
                    .positional_params
                    .get(index)
                    .cloned()
                    .unwrap_or_default()
            }
            // Assignment RHS: $* joins with the first IFS character
            // (GNU subst.c:2930 string_list_dollar_star — IFS unset joins
            // with space, IFS empty joins with nothing), while $@ always
            // joins with a space (subst.c:3006 string_list_dollar_at —
            // PF_ASSIGNRHS || ifs == 0 || *ifs == 0 selects ' ').
            b'@' => self.shell_state.positional_params.join(" "),
            b'*' => self
                .shell_state
                .positional_params
                .join(&self.ifs_first_char_separator()),
            b'#' => self.shell_state.positional_params.len().to_string(),
            b'?' => self.exit_code.to_string(),
            b'$' => self.shell_pid_value().to_string(),
            b'!' => self.last_background_pid_value(),
            b'-' => self.shell_option_flags(),
            _ => return None,
        };
        Some(self.expand_assignment_tilde_if_needed(expanded))
    }

    fn expand_assignment_tilde_if_needed(&self, value: String) -> String {
        if value.contains('=')
            || !tilde_expand::assignment_value_needs_tilde_expansion(&value, true)
            || (self
                .shell_state
                .env_vars
                .get("__RUBASH_POSIX_MODE")
                .map(String::as_str)
                == Some("1")
                && !value.starts_with("~/"))
        {
            return value;
        }

        self.expand_assignment_tilde(&value)
    }

    fn expand_quoted_prompt_arithmetic_assignment(&mut self, value: &str) -> String {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum QuoteMode {
            None,
            Single,
            Double,
        }

        let mut output = String::with_capacity(value.len());
        let mut segment = String::new();
        let mut mode = QuoteMode::None;

        for ch in value.chars() {
            match (mode, ch) {
                (QuoteMode::None, '\'') => {
                    output.push_str(&self.expand_embedded_parameters_mut(&segment));
                    segment.clear();
                    mode = QuoteMode::Single;
                }
                (QuoteMode::None, '"') => {
                    output.push_str(&self.expand_embedded_parameters_mut(&segment));
                    segment.clear();
                    mode = QuoteMode::Double;
                }
                (QuoteMode::Single, '\'') => {
                    output.push_str(&segment);
                    segment.clear();
                    mode = QuoteMode::None;
                }
                (QuoteMode::Double, '"') => {
                    output.push_str(&self.expand_embedded_parameters_mut(&segment));
                    segment.clear();
                    mode = QuoteMode::None;
                }
                _ => segment.push(ch),
            }
        }

        if mode == QuoteMode::Single {
            output.push_str(&segment);
        } else {
            output.push_str(&self.expand_embedded_parameters_mut(&segment));
        }

        preserve_prompt_escapes(&output)
    }

    pub(in crate::executor) fn expand_compound_positional_at_assignment(
        &self,
        value: &str,
        quoted: bool,
    ) -> Option<String> {
        self.expand_compound_positional_at_assignment_impl(value, quoted, false)
    }

    /// GNU eval-argument flatten (xtrace evidence, gg7.sh E1): an
    /// assignment-shaped word after `eval` expands as a NORMAL word -- the
    /// per-element results join with a bare space and NO synthetic
    /// re-quoting (`eval b2=("${x[@]}")` with x=("a b" c) receives
    /// `b2=(a b c)` and eval's reparse splits it into three elements).
    /// Declaration builtins take the re-quoted assignment flatten instead.
    pub(in crate::executor) fn expand_compound_positional_at_assignment_bare(
        &self,
        value: &str,
    ) -> Option<String> {
        self.expand_compound_positional_at_assignment_impl(value, false, true)
    }

    fn expand_compound_positional_at_assignment_impl(
        &self,
        value: &str,
        quoted: bool,
        bare: bool,
    ) -> Option<String> {
        let inner = value.strip_prefix('(')?.strip_suffix(')')?;
        let mut changed = false;
        let mut values = Vec::new();
        // Bare (eval-argument) flatten stores expansion results verbatim:
        // quote_array_value's synthetic wrapping would change what eval's
        // re-parsed string looks like versus GNU (gg7.sh E1/E2 divergence).
        macro_rules! store {
            ($v:expr) => {
                if bare {
                    $v.to_string()
                } else {
                    quote_array_value($v)
                }
            };
            // Literal fallback element: bare mode keeps the raw storage
            // token (quotes intact) so eval's reparse sees the same
            // quoting GNU's verbatim flatten produces.
            ($v:expr, $raw:expr) => {
                if bare {
                    $raw.clone()
                } else {
                    quote_array_value($v)
                }
            };
        }
        // Bare (eval-argument) flatten keeps the RAW token text: literal
        // quoted elements like 'a b' must reach eval's reparse with their
        // quotes intact so the element stays one field.
        for token_raw in split_storage_words(inner) {
            let token = unquote_storage_value(&token_raw);
            // GNU subst.c string_list_dollar_at: "$@" expands to one word
            // per positional parameter. The compound-assignment hoist
            // delivers "$@" either as a bare `$@` (split-form path) or as
            // \u{E302}$@\u{E302} (atomic lexer path, DQ_DATA markers).
            let token_stripped = token.trim_matches('\u{E302}');
            if token_stripped == "$@" || token.strip_prefix(STORAGE_WORD_PREFIX) == Some("${@}") {
                changed = true;
                values.extend(
                    self.shell_state
                        .positional_params
                        .iter()
                        .map(|value| store!(value)),
                );
            } else if let Some(array_name) = token
                .strip_prefix(STORAGE_WORD_PREFIX)
                .and_then(whole_word_braced_parameter_body)
                .and_then(|body| body.strip_suffix("[@]"))
                .or_else(|| {
                    // The atomic lexer path (skip_word_at) preserves the
                    // element's wrapping quotes as raw text, so the hoist
                    // pass delivers `"${a[@]}"` as
                    // \u{E302}${a[@]}\u{E302} with no \x1d quoted-RHS
                    // marker; the [@] list must still fan out per element
                    // (array.tests: local v=("${foo[@]}") keeps 'b c' one
                    // element).
                    whole_word_braced_parameter_body(token.trim_matches('\u{E302}'))
                        .and_then(|body| body.strip_suffix("[@]"))
                })
            {
                if let Some(storage) = self.parameter_array_storage(array_name) {
                    changed = true;
                    values.extend(array_values(&storage).iter().map(|value| store!(value)));
                } else {
                    values.push(store!(""));
                }
            } else if let Some(mut expanded) = self.compound_guard_list_values(&token_raw, bare) {
                // GNU parameter_brace_expand (subst.c:10345-10444) +
                // parameter_brace_expand_rhs (subst.c:7966): the guard
                // `${var+"${arr[@]}"}` with var set expands the rhs, and a
                // quoted `[@]` rhs is a multi-word list — subst.c:8023-8027
                // sets *qdollaratp for it, so even inside the compound
                // element the list keeps one word per array member
                // (arrayfunc.c:557 expand_compound_array_assignment expands
                // each word; bash_completion's
                // `cfg=("${cfg[@]}" ${cfg[@]+"${cfg[@]}"})` idiom, rubash#147).
                // The String path joins the members into one element.
                changed = true;
                values.append(&mut expanded);
            } else if let Some(indirect_name) = token
                .strip_prefix(STORAGE_WORD_PREFIX)
                .and_then(whole_word_braced_parameter_body)
                .and_then(|name| name.strip_prefix('!'))
                .or_else(|| {
                    // The atomic lexer path (skip_word_at) preserves the
                    // element's wrapping quotes as raw text, so the hoist
                    // pass delivers `"${!ref}"` as
                    // \u{E302}${!ref}\u{E302} with no \x1d quoted-RHS
                    // marker; the indirect reference must still fan out
                    // per element (new-exp4.sub Case08 `"${!xx}"` with
                    // xx=arrayA[@]). Only match when \u{E302} wrapping is
                    // actually present so unquoted `${!ref}` falls through
                    // to the unquoted indirect branch below.
                    if token.starts_with('\u{E302}') && token.ends_with('\u{E302}') {
                        whole_word_braced_parameter_body(token.trim_matches('\u{E302}'))
                            .and_then(|name| name.strip_prefix('!'))
                    } else {
                        None
                    }
                })
            {
                // GNU compound assignment of quoted "${!ref}": a direct
                // array reference in the braced name is the KEYS expansion
                // (arrayfunc.c array_keys), anything else is indirection
                // through the target value (new-exp9.sub / new-exp4.sub
                // Case06-08).
                match self.indirect_compound_assignment_values(indirect_name, true) {
                    Some(mut expanded) => {
                        changed = true;
                        values.append(&mut expanded);
                    }
                    None => values.push(store!(&token, token_raw)),
                }
            } else if let Some(indirect_name) =
                whole_word_braced_parameter_body(&token).and_then(|name| name.strip_prefix('!'))
            {
                // The lexer strips the token's quotes and marks the whole
                // quoted-RHS value, so the value-level flag decides between
                // the quoted (joined) and unquoted (field split) semantics
                // (new-exp4.sub Case05-08).
                match self.indirect_compound_assignment_values(indirect_name, quoted) {
                    Some(mut expanded) => {
                        changed = true;
                        values.append(&mut expanded);
                    }
                    None => values.push(store!(&token, token_raw)),
                }
            } else if let Some((var_name, pattern, replacement, global)) = {
                // The hoist pass carries the element's wrapping quotes as
                // DQ_DATA markers; strip them before matching the patsub
                // shape (`\u{E302}${a[@]/#/"q"}\u{E302}`). Parse the RAW
                // token rather than `token`: unquote_storage_value already
                // decoded `\'`/`\"` escapes to bare quotes, which
                // mark_patsub_replacement_quotes then re-reads as quote
                // SYNTAX and eats (array6.sub: `${a[@]/#/-iname \'}` must
                // keep \' as escaped-quote data entering
                // expand_patsub_replacement_text, GNU subst.c
                // parameter_brace_patsub's own quote pass).
                let core = token_raw.trim_matches('\u{E302}');
                let core = core
                    .strip_prefix("\\\"")
                    .and_then(|inner| inner.strip_suffix("\\\""))
                    .or_else(|| {
                        core.strip_prefix('"')
                            .and_then(|inner| inner.strip_suffix('"'))
                    })
                    .unwrap_or(core);
                whole_word_braced_parameter_body(core)
                    .and_then(parse_parameter_replacement)
                    .filter(|(var_name, _, _, _)| {
                        var_name.ends_with("[@]")
                            || var_name.ends_with("[*]")
                            || *var_name == "@"
                            || *var_name == "*"
                    })
            } {
                // A quoted `${a[@]/pat/rep}` (or `${@/pat/rep}`) element
                // expands PER ELEMENT (GNU subst.c param_expand +
                // arrayfunc.c: the [@] word list stays a list inside the
                // compound assignment). The generic word expander would join
                // it into one string, collapsing the array to a single
                // element.
                let storage_name = var_name
                    .strip_suffix("[@]")
                    .or_else(|| var_name.strip_suffix("[*]"))
                    .unwrap_or(var_name);
                let storage_opt = self.parameter_array_storage(storage_name);
                let element_values: Vec<String> = if var_name == "@" || var_name == "*" {
                    self.shell_state.positional_params.clone()
                } else {
                    match storage_opt {
                        Some(storage) => array_values(&storage),
                        None => Vec::new(),
                    }
                };
                if element_values.is_empty() && var_name != "@" && var_name != "*" {
                    values.push(store!(&token, token_raw));
                } else {
                    let pattern = self.expand_parameter_pattern_word(
                        &pattern
                            .replace(r"\/", "/")
                            .replace(crate::executor::markers::DATA_BACKSLASH, "/")
                            .replace(crate::executor::markers::DATA_DQUOTE, "/"),
                    );
                    let replacement = self.expand_patsub_replacement_text(replacement);
                    changed = true;
                    // GNU expand_words_no_vars (arrayfunc.c:557): an UNQUOTED
                    // element word's expansion is field-split on IFS
                    // whitespace (a3=(${a[@]/#/-iname \'}) stores the four
                    // elements -iname 'abc -iname 'def, not two quoted
                    // pairs); a quoted element stays one word.
                    let element_quoted = {
                        let core = token_raw.trim_matches('\u{E302}');
                        core.starts_with("\\\"")
                            || core.starts_with('"')
                            || token_raw.starts_with('\u{E302}')
                    };
                    let split_fields = |text: String| -> Vec<String> {
                        if element_quoted {
                            vec![text]
                        } else {
                            let fields = field_split_values_with_ifs(
                                &text,
                                self.shell_state.env_vars.get("IFS").map(String::as_str),
                            );
                            if fields.is_empty() {
                                vec![text]
                            } else {
                                fields
                            }
                        }
                    };
                    if var_name.ends_with("[*]") || var_name == "*" {
                        // A quoted `[*]` form joins into ONE compound element
                        // (GNU join_array_values with the first IFS char).
                        let joined = element_values
                            .iter()
                            .map(|value| {
                                self.replace_patsub_pattern(value, &pattern, &replacement, global)
                            })
                            .collect::<Vec<_>>()
                            .join(&self.ifs_first_char_separator());
                        for field in split_fields(joined) {
                            values.push(store!(&field));
                        }
                    } else {
                        for value in &element_values {
                            let replaced =
                                self.replace_patsub_pattern(value, &pattern, &replacement, global);
                            for field in split_fields(replaced) {
                                values.push(store!(&field));
                            }
                        }
                    }
                }
            } else if let Some(name) = token
                .strip_prefix(STORAGE_WORD_PREFIX)
                .and_then(whole_word_braced_parameter_body)
                .or_else(|| {
                    // The atomic lexer path (skip_word_at) wraps the
                    // element's quotes in DQ_DATA markers instead of the
                    // \x1d quoted-RHS marker: `"${a[@]:2}"` arrives as
                    // \u{E302}${a[@]:2}\u{E302}. The [@]:off[:len] slice
                    // must still fan out per element (GNU arrayfunc.c:557
                    // expand_compound_array_assignment expands each word
                    // through the real expander — new-exp5.sub
                    // `b=("${a[@]:2}")` stores C and D as two elements).
                    if token_raw.starts_with('\u{E302}') && token_raw.ends_with('\u{E302}') {
                        whole_word_braced_parameter_body(token_raw.trim_matches('\u{E302}'))
                    } else if token_raw.starts_with('"') {
                        whole_word_braced_parameter_body(&token)
                    } else {
                        None
                    }
                })
            {
                if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
                    if var_name == "@" {
                        changed = true;
                        values.extend(
                            positional_parameter_substring_with_zero(
                                &self.shell_state.positional_params,
                                &self.script_name_value(),
                                offset,
                                length,
                            )
                            .iter()
                            .map(|value| store!(value)),
                        );
                        continue;
                    }
                    let starred = var_name.ends_with("[*]");
                    if let Some(array_name) = var_name
                        .strip_suffix("[@]")
                        .or_else(|| var_name.strip_suffix("[*]"))
                    {
                        if let Some(storage) = self.parameter_array_storage(array_name) {
                            changed = true;
                            let sliced = array_parameter_slice(
                                &storage,
                                offset,
                                length.and_then(|length| usize::try_from(length).ok()),
                            );
                            if starred {
                                // GNU array_subrange + string_list_pos_params:
                                // a quoted `arr[*]:off` joins the slice into
                                // ONE word with IFS[0] (new-exp5.sub nd=1).
                                values.push(store!(&sliced.join(&self.ifs_first_char_separator())));
                            } else {
                                values.extend(sliced.iter().map(|value| store!(value)));
                            }
                            continue;
                        }
                    }
                }
                values.push(self.compound_plain_element_value(&token_raw, bare, &mut changed));
            } else {
                // GNU expand_words_no_vars -> shell_expand_word_list expands
                // simple $0, $1, ... $N positional parameters in compound
                // assignment words (array.tests: ARGV=( [0]=$0 "$@" )).
                // single_unquoted_parameter_name rejects digit names, so
                // handle them here. The token may be `[N]=$0` (subscript
                // form) or bare `$0`.
                let core = token.trim_matches('\u{E302}');
                let (prefix, param) = match core.split_once('=') {
                    Some((p, v)) => (Some(p), v),
                    None => (None, core),
                };
                if let Some(name) = param.strip_prefix('$') {
                    // GNU parse.y: a backslash-escaped `\$` in a compound
                    // assignment word is a literal `$`, not a parameter
                    // expansion (array.tests:408 `declare -a x=(\$0)`
                    // stores `$0`, not the script path). token_raw keeps
                    // the backslash; check it before expanding.
                    let raw_param = token_raw
                        .trim_matches('\u{E302}')
                        .split_once('=')
                        .map(|(_, v)| v)
                        .unwrap_or(token_raw.trim_matches('\u{E302}'));
                    if raw_param.starts_with("\\$") {
                        values.push(token_raw.clone());
                        continue;
                    }
                    if name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() {
                        let expanded = if name == "0" {
                            Some(self.script_name_value())
                        } else if let Ok(index) = name.parse::<usize>() {
                            self.shell_state
                                .positional_params
                                .get(index.saturating_sub(1))
                                .cloned()
                        } else {
                            None
                        };
                        if let Some(value) = expanded {
                            changed = true;
                            // Preserve [N]= subscript form without
                            // quote_array_value wrapping so both indexed
                            // and assoc storage recognize [key]=value.
                            let stored = if let Some(p) = prefix {
                                format!("{p}={value}")
                            } else {
                                value
                            };
                            values.push(stored);
                            continue;
                        }
                    }
                    // GNU expand_words_no_vars also expands dynamic
                    // variables like $LINENO that shell_variable_value
                    // does not cover. Only expand names that
                    // dynamic_parameter_value knows about; ordinary
                    // variables are left for
                    // expand_unquoted_parameter_compound_assignment to
                    // avoid breaking $(...) command substitution.
                    if let Some(value) = self.dynamic_parameter_value(name) {
                        changed = true;
                        let stored = if let Some(p) = prefix {
                            format!("{p}={value}")
                        } else {
                            value
                        };
                        values.push(stored);
                        continue;
                    }
                }
                values.push(self.compound_plain_element_value(&token_raw, bare, &mut changed));
            }
        }
        changed.then(|| format!("({})", values.join(" ")))
    }

    /// A guard token `${var<op>LIST}` whose used side is a quoted whole-list
    /// `"${arr[@]}"` / `"$@"`, expanded to one storage value per list member
    /// (GNU parameter_brace_expand_rhs sets *qdollaratp at subst.c:8023-8027
    /// when the rhs expands to a list, so the members keep their word
    /// boundaries in the compound element). `None` leaves the token on the
    /// general element paths.
    fn compound_guard_list_values(&self, token_raw: &str, bare: bool) -> Option<Vec<String>> {
        if bare {
            // The eval-argument flatten keeps element text verbatim for its
            // reparse; only the storage-form path re-quotes members.
            return None;
        }
        let core = token_raw.trim_matches('\u{E302}');
        let body = core.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(core);
        if !body.starts_with("${") || !body.ends_with('}') {
            return None;
        }
        if !crate::executor::parameter_ops::braced_parameter_spans_whole_word(body) {
            return None;
        }
        let inner = &body[2..body.len() - 1];
        let (var_name, alternate, use_when_set, require_non_empty) =
            if let Some((var_name, alternate)) =
                super::expand_braced_ops::split_once_outside_subscript_str(inner, ":+")
            {
                (var_name, alternate, true, true)
            } else if let Some((var_name, alternate)) =
                super::expand_braced_ops::split_once_outside_subscript(inner, '+')
            {
                (var_name, alternate, true, false)
            } else if let Some((var_name, alternate)) =
                super::expand_braced_ops::split_once_outside_subscript_str(inner, ":-")
            {
                (var_name, alternate, false, true)
            } else if let Some((var_name, alternate)) =
                super::expand_braced_ops::split_once_outside_subscript(inner, '-')
            {
                (var_name, alternate, false, false)
            } else {
                return None;
            };
        // Only the quoted whole-list rhs carries the multi-word contract.
        let quoted = alternate.starts_with('"') && alternate.ends_with('"') && alternate.len() >= 2;
        let list_body = if quoted {
            &alternate[1..alternate.len() - 1]
        } else {
            alternate
        };
        let members: Vec<String> = if list_body == "$@" || list_body == "${@}" {
            self.shell_state.positional_params.clone()
        } else if let Some(array_name) = whole_word_braced_parameter_body(list_body)
            .and_then(|name| name.strip_suffix("[@]"))
            .filter(|name| is_shell_name(name))
        {
            self.parameter_array_storage(array_name)
                .map(|storage| array_values(&storage))
                .unwrap_or_default()
        } else {
            return None;
        };
        let value = self.parameter_operator_value(var_name);
        let word_used = if use_when_set {
            value.is_some() && (!require_non_empty || !value.unwrap_or_default().is_empty())
        } else {
            value.is_none() || (require_non_empty && value.unwrap_or_default().is_empty())
        };
        if word_used {
            return Some(members.into_iter().map(|m| quote_array_value(&m)).collect());
        }
        match (var_name, use_when_set) {
            // `+`/`:+` with the parameter unset or null: the guard expands
            // to nothing, so the element contributes zero members.
            (_, true) => Some(Vec::new()),
            // `-`/`:-` with the parameter set (non-null for `:-`): the used
            // side is the parameter itself. A list operand keeps per-member
            // boundaries (subst.c:9990-9993 list semantics); everything else
            // stays on the general element path.
            ("@", false) => Some(
                self.shell_state
                    .positional_params
                    .iter()
                    .map(|m| quote_array_value(m))
                    .collect(),
            ),
            (name, false)
                if name.ends_with("[@]")
                    && name
                        .strip_suffix("[@]")
                        .is_some_and(|base| is_shell_name(base)) =>
            {
                Some(
                    self.parameter_array_storage(name.strip_suffix("[@]").unwrap_or(name))
                        .map(|storage| {
                            array_values(&storage)
                                .into_iter()
                                .map(|m| quote_array_value(&m))
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            }
            _ => None,
        }
    }

    /// The catch-all element fallback: GNU arrayfunc.c:557
    /// expand_compound_array_assignment expands EVERY element word through
    /// the real expander — an element this pass does not specially recognize
    /// (a guard like `${arr[@]+"${arr[@]}"}` after a quoted element, a
    /// command substitution, an ordinary `${var:-word}`) must still expand,
    /// not freeze its raw text into the stored array (rubash#147: the guard
    /// literal leaked as an element whenever another element had already set
    /// `changed`). The compound walker's preserve-quotes contract keeps
    /// literal tokens byte-identical, so routing through it only differs
    /// where expansion actually applies.
    fn compound_plain_element_value(
        &self,
        token_raw: &str,
        bare: bool,
        changed: &mut bool,
    ) -> String {
        if bare {
            return token_raw.to_string();
        }
        let expanded = self.expand_embedded_parameters_compound(token_raw);
        if expanded != token_raw {
            *changed = true;
        }
        expanded
    }

    /// Compound-assignment element values for a `"${!ref}"` token (GNU
    /// parameter_brace_expand_indir plus the array-assignment element
    /// splitting): a direct array reference in the braced name expands as
    /// KEYS (arrayfunc.c array_keys), any other value is indirection
    /// through the target parameter -- `@`/`*` targets keep their
    /// $@/$* word semantics, `[@]`/`[*]` targets expand the array values,
    /// and scalars read their value cell (field split when the token is
    /// unquoted). Returns None for unrecognized names so the token stays
    /// literal.
    fn indirect_compound_assignment_values(
        &self,
        indirect_name: &str,
        quoted: bool,
    ) -> Option<Vec<String>> {
        if let Some(array_name) = indirect_name
            .strip_suffix("[@]")
            .or_else(|| indirect_name.strip_suffix("[*]"))
        {
            if is_shell_name(array_name) {
                let storage_name = self.resolved_variable_name(array_name)?;
                let storage = self.parameter_array_storage(array_name)?;
                let keys = if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &storage_name) {
                    assoc_keys(
                        &storage,
                        assoc_nbuckets(&self.shell_state.env_vars, &storage_name),
                    )
                } else {
                    array_indices(&storage)
                };
                if indirect_name.ends_with("[*]") {
                    return Some(vec![quote_array_value(
                        &keys.join(&self.ifs_first_char_separator()),
                    )]);
                }
                return Some(
                    keys.into_iter()
                        .map(|key| quote_array_value(&key))
                        .collect(),
                );
            }
        }

        // A nameref indirection yields the referenced NAME itself, not the
        // target value (GNU parameter_brace_expand_indir subst.c:7896).
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, indirect_name) {
            return None;
        }
        let target_expr = self.resolve_indirect_target_expr(indirect_name)?;
        match target_expr.as_str() {
            "@" => {
                return Some(
                    self.shell_state
                        .positional_params
                        .iter()
                        .map(|value| quote_array_value(value))
                        .collect(),
                )
            }
            "*" => {
                return Some(vec![quote_array_value(
                    &self
                        .shell_state
                        .positional_params
                        .join(&self.ifs_first_char_separator()),
                )])
            }
            _ => {}
        }
        let starred = target_expr.ends_with("[*]");
        if starred || target_expr.ends_with("[@]") {
            let values = self.indirect_target_values(&target_expr);
            if starred {
                if quoted {
                    return Some(vec![quote_array_value(
                        &values.join(&self.ifs_first_char_separator()),
                    )]);
                }
                return Some(
                    field_split_array_values_with_ifs(
                        values,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    )
                    .into_iter()
                    .map(|value| quote_array_value(&value))
                    .collect(),
                );
            }
            return Some(
                values
                    .into_iter()
                    .map(|value| quote_array_value(&value))
                    .collect(),
            );
        }
        // Same bare-array decoding as the word path: implicit
        // `name=(...)` storage may be unmarked, so prefer
        // indirect_target_values before the parameter resolution.
        let mut target_values = self.indirect_target_values(&target_expr);
        let scalar = if target_values.len() == 1 {
            target_values.remove(0)
        } else if target_values.len() > 1 {
            target_values.join(&self.ifs_first_char_separator())
        } else {
            self.parameter_pattern_scalar_value(&target_expr)
                .unwrap_or_default()
        };
        if quoted {
            return Some(vec![quote_array_value(&scalar)]);
        }
        Some(
            field_split_values_with_ifs(
                &scalar,
                self.shell_state.env_vars.get("IFS").map(String::as_str),
            )
            .into_iter()
            .map(|value| quote_array_value(&value))
            .collect(),
        )
    }

    pub(in crate::executor) fn expand_unquoted_parameter_compound_assignment(
        &self,
        value: &str,
    ) -> Option<String> {
        let inner = value.strip_prefix('(')?.strip_suffix(')')?.trim();
        let unquoted_inner = strip_matching_quotes(inner);
        let is_quoted = unquoted_inner != inner;
        let parameter = if is_quoted { &unquoted_inner } else { inner };
        let value = if let Some(name) = single_unquoted_parameter_name(parameter) {
            self.shell_variable_value(name).unwrap_or_default()
        } else if let Some(name) = whole_word_braced_parameter_body(parameter) {
            let name = name.replace("\\\"", "\"").replace("\\'", "'");
            self.array_element_parameter_value(&name)?
        } else {
            return None;
        };
        // GNU expand_compound_array_assignment: parse_string_to_word_list
        // sets W_QUOTED on a quoted word ("$value"), and shell_expand_word_list
        // does not field-split W_QUOTED words. A quoted parameter in a
        // compound assignment stays one element (array19.sub:
        // declare -a var=("$value") stores [0]="a b c", not 3 elements).
        // The word also lacks W_ASSIGNMENT (only the parser sets it), so
        // assign_compound_array_list (arrayfunc.c:753) does NOT check
        // [subscript]=value form. Tag with ARRAY_FIELD_SPLIT_MARKER so
        // append_array_value skips the subscript detection (otherwise
        // "[$(echo total 0)]=1 [2]=2]" from a variable value would be
        // misparsed as a subscript assignment and the $(...) re-executed).
        if is_quoted {
            return Some(format!(
                "({ARRAY_FIELD_SPLIT_MARKER}{})",
                quote_compound_field_value(&value)
            ));
        }
        let values = field_split_values_with_ifs(
            &value,
            self.shell_state.env_vars.get("IFS").map(String::as_str),
        )
        .into_iter()
        .map(|value| {
            format!(
                "{ARRAY_FIELD_SPLIT_MARKER}{}",
                quote_compound_field_value(&value)
            )
        })
        .collect::<Vec<_>>();
        Some(format!("({})", values.join(" ")))
    }

    pub(in crate::executor) fn expand_quoted_array_assignment_value(
        &self,
        value: &str,
    ) -> Option<String> {
        let value = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(value);
        let name = whole_word_braced_parameter_body(value)?;
        let array_name = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))?;
        self.parameter_array_storage(array_name)
            .map(|value| self.join_array_parameter_values(&value, name))
    }

    pub(in crate::executor) fn expand_assignment_value_with_status(
        &mut self,
        name: &str,
        value: &str,
    ) -> (String, Option<i32>) {
        let result = self.expand_assignment_value_result(name, value);
        (result.value, result.substitution_status)
    }
}

/// Byte ranges `[start, end)` of every top-level `${...}` braced-parameter
/// region in `value` (closing `}` inclusive), skipping single-quoted text and
/// whole command-substitution bodies — the same `${` + matching_parameter_brace
/// walk the `:=` pre-scan uses (apply_parameter_assignment_expansions_in_word).
/// A `${` without a matching `}` is left unrecorded (the walker reports it).
fn top_level_braced_parameter_regions(
    value: &str,
    spans: &[crate::executor::substitution_metadata::SubstitutionSpan],
) -> Vec<(usize, usize)> {
    let span_ends_by_start: std::collections::HashMap<usize, usize> =
        spans.iter().map(|span| (span.start, span.end)).collect();
    let bytes = value.as_bytes();
    let mut regions = Vec::new();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch == b'\\' && !single {
            index += 2;
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
        if single {
            index += 1;
            continue;
        }
        if let Some(&span_end) = span_ends_by_start.get(&index) {
            index = span_end;
            continue;
        }
        if ch == b'$' && bytes.get(index + 1) == Some(&b'{') {
            let body_start = index + 2;
            if let Some(end) = matching_parameter_brace(&value[body_start..]) {
                regions.push((index, body_start + end + 1));
                index = body_start + end + 1;
                continue;
            }
        }
        index += 1;
    }
    regions
}

/// Split a compound assignment body into element tokens, treating single
/// quotes, double quotes and the hoisted DQ_DATA marker as quoting, so a
/// quoted space (`("a b"` hoisted to `(\u{E302}a b\u{E302}`) stays inside its
/// token. Tokens keep every character verbatim; only unquoted whitespace
/// separates elements.
pub(in crate::executor) fn split_compound_element_words(value: &str) -> Vec<String> {
    const DQ_DATA: char = crate::executor::markers::ASSIGN_DATA_DQUOTE;
    // The hoisted single-quote sentinel (expand_assignment_value_hoisting):
    // `'q k'` arrives as E307 q k E307 and must still count as ONE element
    // or the kv-pair key/value parity in the assoc tilde pass shifts.
    const SQ_DATA: char = crate::executor::markers::ASSIGN_HOISTED_SQUOTE;
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut chars = value.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }
        // GNU parse.y:3629-3642 read_token: a word-start `#' comments out
        // the rest of the line; parse.y:7131-7135 admits newlines inside a
        // compound assignment, so comment lines yield no elements here
        // either. `token` empty = word start (mid-word `a#b` is literal).
        if ch == '#' && !single && !double && token.is_empty() {
            for (_, comment_ch) in chars.by_ref() {
                if comment_ch == '\n' {
                    break;
                }
            }
            continue;
        }
        if ch == '$' && !single && matches!(chars.peek(), Some((_, '{'))) {
            // A `${...}` body is scanned by GNU parse_matched_pair with its
            // own nested-pair quote state: body quotes neither split the
            // element here nor leak into the element-level quote state
            // (array6.sub: ("${a[@]/#/"-iname '"}")).
            token.push(ch);
            token.push('{');
            chars.next();
            let rest = &value[offset + 2..];
            if let Some(scan) = scan_braced_parameter_body(
                rest,
                BraceContext {
                    outer_double_quote: double,
                    posix: false,
                    replacement_context: false,
                    initial_state: DolbraceState::Param,
                },
            ) {
                token.push_str(&rest[..scan.end]);
                for _ in 0..rest[..scan.end].chars().count() {
                    chars.next();
                }
            }
            continue;
        }
        if ch == '$' && !single && matches!(chars.peek(), Some((_, '('))) {
            // GNU parse.y:4473 parse_comsub: the $(...) body is scanned by
            // shell_getc under its own quoting state, so quotes inside the
            // command substitution neither split the element nor leak into
            // the element-level quote state (assoc11.sub: the ' inside
            // $(echo 'foo[bar') must not swallow the following words).
            token.push(ch);
            let rest = &value[offset + 1..];
            let rest_chars: Vec<char> = rest.chars().collect();
            if let Some(end) = crate::lexer::skip_parenthesized_unit_corrected(&rest_chars, 0) {
                let unit: String = rest_chars[..end].iter().collect();
                token.push_str(&unit);
                for _ in 0..end {
                    chars.next();
                }
            } else {
                token.push('(');
                chars.next();
            }
            continue;
        }
        match ch {
            '\\' if !single => {
                token.push(ch);
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                token.push(ch);
            }
            '"' if !single => {
                double = !double;
                token.push(ch);
            }
            SQ_DATA if !double => {
                single = !single;
                token.push(ch);
            }
            DQ_DATA if !single => {
                double = !double;
                token.push(ch);
            }
            ch if ch.is_whitespace() && !single && !double => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            ch => token.push(ch),
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn preserve_prompt_escapes(value: &str) -> String {
    const PROTECTED_PROMPT_ESCAPE: char = crate::executor::markers::PROMPT_ESCAPE_GUARD;
    let mut preserved = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && matches!(chars.peek(), Some('!' | '#')) {
            preserved.push(PROTECTED_PROMPT_ESCAPE);
            preserved.push(chars.next().expect("peeked prompt escape"));
        } else {
            preserved.push(ch);
        }
    }
    preserved.replace(PROTECTED_PROMPT_ESCAPE, "\\")
}

/// Quote-aware rewrite of $"..." into "..." for assignment expansion
/// (GNU parse.y:4096). Only a `$` immediately followed by an opening double
/// quote is rewritten: the scan tracks single- and double-quoted spans so a
/// `$` inside them ("a$"x", '...$"...') keeps its literal meaning, and
/// backslash escapes are passed through untouched.
fn normalize_dollar_double_quotes(value: &str) -> std::borrow::Cow<'_, str> {
    let needs_rewrite = value
        .chars()
        .zip(value.chars().skip(1))
        .any(|(ch, next)| ch == '$' && next == '"');
    if !needs_rewrite {
        return std::borrow::Cow::Borrowed(value);
    }
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            output.push('\\');
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' if !in_double => {
                in_single = !in_single;
                output.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                output.push(ch);
            }
            '$' if !in_single && !in_double && chars.peek() == Some(&'"') => {
                // Drop the locale-quote dollar; the quote stays.
            }
            _ => output.push(ch),
        }
    }
    std::borrow::Cow::Owned(output)
}

/// Remove CTLESC (\x11) sentinels the lexer inserts before quoted glob
/// metacharacters (*?[!@+). Mirrors glob.rs `dequote_pathname`: the \x11
/// is a marker, the following character is the data it protects.
fn dequote_ctlesc(value: &str) -> String {
    if !value.contains(crate::executor::markers::CTLESC) {
        return value.to_string();
    }
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == crate::executor::markers::CTLESC {
            if let Some(next) = chars.next() {
                output.push(next);
            }
        } else {
            output.push(ch);
        }
    }
    output
}

/// True when `word` still carries quote syntax at the WORD level. Raw
/// `'`/`"` characters inside `${...}`, `$(...)`, `$'...'`, or backtick
/// bodies belong to that expansion (the lexer keeps those bodies verbatim)
/// and do not count - GNU dequote_word only strips quote syntax the parser
/// placed on the original word.
fn word_level_quote_syntax(word: &str) -> bool {
    let bytes = word.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 1,
            b'\'' | b'"' => return true,
            b'`' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'`' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
            }
            b'$' => match bytes.get(index + 1) {
                Some(b'{') => match matching_parameter_brace(&word[index + 2..]) {
                    Some(close) => index += 2 + close,
                    None => index += 1,
                },
                Some(b'(') => {
                    let mut depth = 1usize;
                    index += 2;
                    while index < bytes.len() && depth > 0 {
                        match bytes[index] {
                            b'\\' => index += 1,
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            _ => {}
                        }
                        index += 1;
                    }
                    continue;
                }
                Some(b'\'') => {
                    index += 2;
                    while index < bytes.len() && bytes[index] != b'\'' {
                        index += if bytes[index] == b'\\' { 2 } else { 1 };
                    }
                }
                _ => {}
            },
            _ => {}
        }
        index += 1;
    }
    false
}
