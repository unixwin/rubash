use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

/// GNU bash accepts an ANSI-C quoted span as the parameter name (the extquote
/// feature: subst.c expand_brace_dollar decodes the name word and re-dispatches
/// the expansion). Only $'...' is accepted; single-quoted, double-quoted, and
/// nested-braced names are bad substitutions, so they are deliberately not tried
/// here. The span may be the whole name (${$'x1'}) or the leading part of an
/// operator form (${$'x1'%t}, ${$'x1'-fallback}). Returns the rewritten
/// body, or None when the name is not a decodable dollar-quoted span.
fn resolve_dollar_quoted_parameter_name(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() < 3 || chars[0] != '$' || chars[1] != '\'' {
        return None;
    }
    let mut body = String::new();
    let mut index = 2usize;
    while index < chars.len() {
        if chars[index] == '\'' {
            break;
        }
        if chars[index] == '\\' && index + 1 < chars.len() {
            body.push(chars[index]);
            body.push(chars[index + 1]);
            index += 2;
            continue;
        }
        body.push(chars[index]);
        index += 1;
    }
    if index >= chars.len() {
        return None;
    }
    let decoded = crate::lexer::ansi::decode_ansi_c_quoted(&body);
    if !is_shell_name(&decoded) {
        return None;
    }
    let rest: String = chars[index + 1..].iter().collect();
    let resolved = if rest.is_empty() {
        decoded
    } else {
        format!("{}{}", decoded, rest)
    };
    (resolved != name).then_some(resolved)
}

impl Executor {
    /// GNU redir.c:298 redirection_expand: a redirect word is expanded
    /// exactly once, inside do_redirections' left-to-right pass. Rubash
    /// resolves targets over several passes (ambiguity precheck, fd-scope
    /// binding, pipeline stream routing) and each pass may see a different
    /// Redirect object for the same source redirect (the ordered `redirects`
    /// list entry vs the legacy mirror fields vs materialized clones), so
    /// the memo keys on the redirect's semantic identity. Side effects
    /// (`$((n+=1))`, `$(cmd)`) in a target run once per command execution;
    /// the memo is cleared at each execute_command entry.
    pub(crate) fn expand_redirect_target(&self, redirect: &crate::parser::Redirect) -> String {
        let key = format!(
            "{:?}\x1f{}\x1f{:?}\x1f{}",
            redirect.kind, redirect.operator, redirect.fd, redirect.target
        );
        if let Some(hit) = self.redirect_target_memo.borrow().get(&key) {
            return hit.clone();
        }
        let expanded = self.expand_word(&redirect.target);
        self.redirect_target_memo
            .borrow_mut()
            .insert(key, expanded.clone());
        expanded
    }

    pub(crate) fn expand_word(&self, word: &str) -> String {
        let _xpass = crate::executor::expand_braced_indices::SubXpassFrame::new();
        let _wctx = crate::executor::expand_braced_indices::WordCtxGuard::new_if_absent();
        if let Some(value) = self.expand_marked_or_special_word(word) {
            return value;
        }

        if let Some(value) = self.expand_assignment_word(word) {
            return value;
        }

        if let Some(value) = self.expand_substitution_word(word) {
            return value;
        }

        if let Some(name) = word
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
        {
            // When this word IS the `${}` fragment being expanded (no
            // enclosing fragment site), record site (word, 0) so the
            // `:=`/`-=` layered re-checks dedup subscript side effects
            // (SUB_RES_XPASS). An active site means the `${` walker arm
            // already named this fragment — keep it.
            let _site_guard = (!crate::executor::expand_braced_indices::sub_site_active())
                .then(|| crate::executor::expand_braced_indices::SubSiteGuard::new(0));
            return self.expand_braced_parameter_word(word, name);
        }

        if let Some(name) = word.strip_prefix('$') {
            if is_shell_name(name) {
                return self
                    .dynamic_parameter_value(name)
                    .or_else(|| self.shell_variable_value(name))
                    .unwrap_or_default();
            }
        }

        let expanded = self.expand_embedded_parameters(word);
        if word.contains("$(") || word.contains('`') {
            restore_command_substitution_output(
                &restore_protected_replacement_quotes(&unescape_remaining_shell_escapes(&expanded))
                    .replace("\\\\'", "'")
                    .replace("\\'", "'"),
            )
        } else {
            restore_protected_replacement_quotes(&expanded)
        }
    }

    pub(in crate::executor) fn expand_braced_parameter_word(
        &self,
        word: &str,
        name: &str,
    ) -> String {
        if !braced_parameter_spans_whole_word(word) {
            return self.expand_embedded_parameters(word);
        }
        if let Some(resolved) = resolve_dollar_quoted_parameter_name(name) {
            return self.expand_braced_parameter_word(word, &resolved);
        }

        // GNU subst.c:10272-10288 (parameter_brace_expand): the parameter
        // name is terminated by the first character that cannot be part of
        // it; when that character starts no operator the switch default
        // raises "bad substitution". A quote at name position
        // (`${'x1'%'t'}`, `${x'y'}`) hits it — quotes are only legal in the
        // word part, after a real operator. Detected here (not in the
        // command pre-scan) because a nested `${}` inside a pattern or
        // alternate word reaches expansion only when that word is actually
        // evaluated (`${x-${'x1'%'t'}}` with x set is silent in GNU).
        if braced_name_ends_on_quote(name) {
            eprintln!(
                "{}{}: bad substitution",
                self.diagnostic_prefix(),
                bad_substitution_display(word)
            );
            self.shell_state.parameter_bad_substitution.set(true);
            return String::new();
        }

        // GNU subst.c:10042-10046: `valid_brace_expansion_word` gates the
        // extracted parameter name BEFORE any operator arm; every failure is
        // the subst.c:10276-10288 `bad substitution` default. Detected here
        // (like the quote check above) rather than in a command pre-scan so
        // a bad name nested in an unevaluated word (`${x:-${(M)y}}` with x
        // set) stays silent, matching GNU's lazy expansion.
        if braced_name_is_bad_substitution(name) {
            eprintln!(
                "{}{}: bad substitution",
                self.diagnostic_prefix(),
                bad_substitution_display(word)
            );
            self.shell_state.parameter_bad_substitution.set(true);
            return String::new();
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name, true) {
            return value;
        }

        if let Some(value) = self.expand_braced_indexed_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_replacement_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_operator_or_array_parameter(name) {
            return value;
        }

        self.dynamic_parameter_value(name)
            .or_else(|| {
                self.shell_variable_value(name)
                    .map(|value| shell_safe_value(&value))
            })
            .unwrap_or_default()
    }

    fn expand_marked_or_special_word(&self, word: &str) -> Option<String> {
        if let Some(word) = word.strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX) {
            return Some(self.expand_embedded_parameters(word));
        }

        if let Some(word) = word.strip_prefix(STORAGE_WORD_PREFIX) {
            return Some(self.expand_quoted_parameter_word(word));
        }

        match word {
            "$?" => Some(self.exit_code.to_string()),
            "$$" => Some(self.shell_pid_value().to_string()),
            "$!" => Some(self.last_background_pid_value()),
            "$@" => Some(self.shell_state.positional_params.join(" ")),
            // Bash joins `$*` with the first character of IFS, not a space.
            "$*" => Some(
                self.shell_state
                    .positional_params
                    .join(&self.ifs_first_char_separator()),
            ),
            "$#" => Some(self.shell_state.positional_params.len().to_string()),
            "$-" => Some(self.shell_option_flags()),
            _ => tilde_expand::expand_word_prefix(word, &self.shell_state.env_vars),
        }
    }

    fn expand_assignment_word(&self, word: &str) -> Option<String> {
        // GNU general.c:480 assignment(): assignment-ness is decided on the
        // raw token before expansion (see the matching removal in
        // expand_word_mut_with_context); expanding the name portion here
        // applied side effects for words that are not assignments at all.
        let (name, value) = split_assignment_word(word)?;
        Some(self.expand_plain_assignment_word(name, value))
    }

    fn expand_plain_assignment_word(&self, name: &str, value: &str) -> String {
        let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
        let value = tilde_expand::strip_assignment_quote_marker(value);
        if quoted {
            if let Some(expanded) = self.expand_quoted_array_assignment_value(value) {
                return format!("{name}={expanded}");
            }
        }
        let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
        let raw_value = value
            .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
            .unwrap_or(value);
        // GNU subst.c:4357 expand_string_assignment (W_ASSIGNMENT,
        // subst.c:11432): unquoted element values of a compound assignment
        // undergo the assignment tilde pass on the RAW word, before
        // parameter expansion, so tilde text produced by $params is never
        // re-expanded (array.tests: aa=([0]=~/a:~/b) expands both segments
        // while w=([0]=~/a [1]=$p) keeps $p's result literal). Quoted
        // elements stay literal; quoted whole-RHS values skip the pass.
        let tilde_raw_owned;
        let raw_value = if !quoted && raw_value.starts_with('(') && raw_value.ends_with(')') {
            tilde_raw_owned = self.expand_tilde_in_compound_assignment(name, raw_value);
            &tilde_raw_owned
        } else {
            raw_value
        };
        if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(raw_value) {
            let marker = if compound_assignment {
                COMPOUND_ASSIGNMENT_MARKER.to_string()
            } else {
                String::new()
            };
            return format!("{name}={marker}{expanded}");
        }
        if let Some(expanded) = self.expand_compound_positional_at_assignment(raw_value, quoted) {
            let marker = if compound_assignment {
                COMPOUND_ASSIGNMENT_MARKER.to_string()
            } else {
                String::new()
            };
            return format!("{name}={marker}{expanded}");
        }
        // GNU arrayfunc.c:557 expand_compound_array_assignment tokenizes the
        // raw parenthesized text first; the preserve variant keeps element
        // quote syntax so the storage tokenizer sees GNU's raw words
        // (assoc11.sub quote elements, d=(x $(echo 'y z') w) assoc glue).
        let expanded = if compound_assignment {
            self.expand_embedded_parameters_compound(value)
        } else {
            self.expand_embedded_parameters(value)
        };
        let expanded = if quoted {
            expanded.replace(crate::executor::markers::CTLESC, "")
        } else {
            expanded
        };
        // GNU subst.c: expand_word_internal applies tilde expansion to the
        // RAW word before parameter expansion. A tilde that comes from
        // ${param} expansion is never re-expanded (unicode1.sub: EChar=${Array[0x7e]}
        // where the value is "~" must stay literal). GNU expands `~` at the
        // start of the RHS and after every `:` in an assignment value
        // (subst.c:11410-11460 internal_tilde + assignoff tracking).
        // A compound `( ... )` RHS already took its per-element tilde pass
        // (assign_assoc_from_kvlist key/value split, arrayfunc.c:630) — a
        // whole-text `:`-tilde would wrongly expand `~` inside key-position
        // elements like `p:~/r` that GNU leaves literal.
        if !quoted
            && !compound_assignment
            && !expanded.contains('=')
            && tilde_expand::assignment_value_needs_tilde_expansion(value, true)
            && (self
                .shell_state
                .env_vars
                .get("__RUBASH_POSIX_MODE")
                .map(String::as_str)
                != Some("1")
                || expanded.starts_with("~/"))
        {
            return format!("{name}={}", self.expand_assignment_tilde(&expanded));
        }

        format!("{name}={expanded}")
    }

    fn expand_substitution_word(&self, word: &str) -> Option<String> {
        if let Some(expanded) = self.expand_backtick_substitution(word) {
            return Some(restore_command_substitution_output(
                &command_substitution_word_split(&expanded),
            ));
        }

        if let Some(value) = self.expand_dirstack_tilde(word) {
            return Some(value);
        }

        if let Some(expression) = word
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
        {
            let expression = self.expand_arithmetic_special_parameters(expression);
            // Pending deferred arithmetic writes are part of the effective
            // environment for this expansion (GNU applies redirect-side-effect
            // assignments immediately, so a later `$((` in the same command
            // sees them).
            let overlaid =
                crate::executor::expand_braced_indices::env_vars_with_pending_subscript_writes(
                    &self.shell_state.env_vars,
                );
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
                if let Some(name) = arithmetic_unbound_variable(&expression, &overlaid) {
                    if !self.shell_state.arithmetic_expansion_error.replace(true) {
                        eprintln!("{}{}: unbound variable", self.diagnostic_prefix(), name);
                    }
                    // GNU expr.c expr_streval: an unbound variable under `set
                    // -u` raises FORCE_EOF and exits the shell (127 in -c
                    // mode), regardless of the other words in the command.
                    self.shell_state.arithmetic_nounset_error.set(true);
                    return Some(String::new());
                }
            }
            // GNU redir.c:298 redirection_expand / subst.c word expansion run
            // in the live shell context, so `$((n+=1))` side effects commit.
            // This `&self` path cannot write env_vars; capture the eval deltas
            // into the deferred-write queue (same channel as the embedded
            // parameter walker's arithmetic arm) for the mutable caller to
            // apply.
            let (value, writes, actual_category) =
                eval_conditional_arith_value_categorized_with_writes(&expression, &overlaid);
            if !writes.is_empty() {
                crate::executor::expand_braced_indices::PENDING_SUBSCRIPT_WRITES
                    .with(|pending| pending.borrow_mut().extend(writes));
            }
            if let Some(value) = value {
                return Some(value.to_string());
            }
            self.shell_state
                .arithmetic_last_error_category
                .set(actual_category);
            // Bash reports arithmetic expansion errors (floating point,
            // negative exponent, division by zero, ...) on stderr instead of
            // silently producing nothing, and abandons the enclosing command
            // list (status 1; GNU probe d2: `echo $((1/0)); echo after` never
            // prints "after").
            let message = crate::executor::arithmetic::arithmetic_error_message(
                &expression,
                true,
                &self.shell_state.env_vars,
            )
            .unwrap_or_else(|| {
                format!(
                    "{expression}: syntax error in expression (error token is \"{expression}\")"
                )
            });
            let actual_fatal = self
                .shell_state
                .arithmetic_last_error_category
                .take()
                .is_some();
            if !actual_fatal
                && !crate::executor::arithmetic::arithmetic_expansion_is_fatal(&expression)
            {
                self.shell_state.arithmetic_nonfatal_error.set(true);
            } else {
                self.shell_state.arithmetic_fatal_error.set(true);
            }
            if !self.shell_state.arithmetic_expansion_error.replace(true) {
                eprintln!("{}{}", self.diagnostic_prefix(), message);
            }
        }

        if let Some(source) = word
            .strip_prefix("$(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            if command_substitution_spans_whole_word(word) {
                return Some(self.expand_command_substitution(source));
            }
        }

        None
    }
}

/// GNU subst.c:10272-10288 — parameter_brace_expand extracts the parameter
/// name and switches on the character that terminated it; a `'`/`"` there
/// matches no operator arm and lands on the `bad substitution` default.
/// The name head is an identifier run or a leading special-parameter char,
/// followed by an optional `[...]` subscript (quotes inside a subscript are
/// legal — `a[' ']` is an associative-style key, not a name terminator).
pub(in crate::executor) fn braced_name_ends_on_quote(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i == 0 {
        if matches!(
            bytes.first(),
            Some(b'!' | b'@' | b'*' | b'#' | b'?' | b'$' | b'-')
        ) {
            i = 1;
        }
    }
    if i < bytes.len() && bytes[i] == b'[' {
        // GNU skipsubscript (subst.c): a `]` quoted by single quotes, double
        // quotes or a backslash does not terminate the subscript.
        let mut depth = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                b'\'' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'\'' {
                        i += 1;
                    }
                }
                b'"' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        if bytes[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'\\' => i += 1,
                _ => {}
            }
            i += 1;
        }
    }
    // \x17/\x18 are the SQ/DQ data sentinels: an operator word quote-
    // decoded before re-expansion (decode_double_quotes_in_quoted_
    // parameter_word, "${x-${'u'%'v'}}") reaches here with quote evidence
    // erased; a sentinel at name position can only come from a source
    // quote, which lands on the same bad-substitution default.
    i < bytes.len() && matches!(bytes[i], b'\'' | b'"' | 0x17 | 0x18)
}

/// Renders a `${...}` word for the bad-substitution diagnostic: operator
/// words quote-decoded before re-expansion carry data sentinels that
/// must read back as the source characters like GNU's diagnostic.
pub(in crate::executor) fn bad_substitution_display(word: &str) -> String {
    crate::locale::decode_to_visible_text(word)
}

/// The parameter-name terminator set GNU scans for in
/// `parameter_brace_expand` (subst.c:9808, the CASEMOD_TOGGLECASE spelling
/// `#%^,~:-=?+/@}` — config-top.h:113 enables the `~` arm in this build).
/// `}` ends the scan the same way in GNU's charlist.
const BRACE_NAME_TERMINATORS: &[u8] = b"#%^,~:-=?+/@}";

fn brace_name_terminator(byte: u8) -> bool {
    BRACE_NAME_TERMINATORS.contains(&byte)
}

fn is_variable_starter(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// general.c:288 `valid_identifier`: `legal_variable_starter` then
/// `legal_variable_name` (ASCII letters, digits, `_`).
fn valid_brace_identifier(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && is_variable_starter(bytes[0])
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

/// CSPECVAR — the single-character special parameters (mksyntax.c:232,
/// `@*#?-$!`; digits are covered by the all-digits arm).
fn brace_special_single(byte: u8) -> bool {
    matches!(byte, b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!')
}

/// VALID_INDIR_PARAM (subst.c:122): `@`/`*` always; `#`/`?` only outside
/// POSIX mode (rubash matches GNU's default build here).
fn brace_valid_indir_param(byte: u8) -> bool {
    matches!(byte, b'@' | b'*' | b'#' | b'?')
}

/// subst.c:791 `string_extract` with SX_VARNAME: `\X` pairs stay in the
/// name, a `[...]` subscript with a matching `]` is skipped whole, and the
/// scan stops at the first terminator (an unmatched `[` is an ordinary
/// character). Returns the extracted head slice.
fn brace_name_head(bytes: &[u8]) -> &[u8] {
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => match brace_subscript_end(bytes, index) {
                Some(end) => index = end + 1,
                None => index += 1,
            },
            b if brace_name_terminator(b) => return &bytes[..index],
            _ => index += 1,
        }
    }
    &bytes[..index.min(bytes.len())]
}

/// GNU skipsubscript: scan to the matching `]`, nesting `[`/`]` and taking
/// `\X` pairs as units; quotes do not open nesting here.
fn brace_subscript_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => {
                depth += 1;
                index += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// subst.c:7583 `valid_brace_expansion_word` minus its array arm: subscript
/// content errors keep their current reporting in the indexed-parameter
/// arms, so a head carrying a `[` is deferred there (both `x[0]` and the
/// malformed `x[a` class).
fn valid_brace_gate_word(head: &[u8]) -> bool {
    if !head.is_empty() && head.iter().all(|b| b.is_ascii_digit()) {
        return true;
    }
    if head.len() == 1 && brace_special_single(head[0]) {
        return true;
    }
    if head.contains(&b'[') {
        return true;
    }
    valid_brace_identifier(head)
}

/// subst.c:8244 `valid_length_expression` on the operand after `#`: empty
/// (`${#}`), a single special parameter (`${#!}`), all digits (`${#10}`),
/// an array reference (`${#a[7]}`), or an identifier (`${#PS1}`).
fn valid_length_operand(operand: &[u8]) -> bool {
    if operand.is_empty() {
        return true;
    }
    if operand.len() == 1 && brace_special_single(operand[0]) {
        return true;
    }
    if operand.iter().all(|b| b.is_ascii_digit()) {
        return true;
    }
    if operand.contains(&b'[') {
        return true;
    }
    valid_brace_identifier(operand)
}

/// GNU subst.c:9777 `parameter_brace_expand` name gate, mapped 1:1:
///
/// - subst.c:9800-9806: a `#` followed by an identifier starter extracts to
///   the closing `}` (terminators do not stop it), and subst.c:9929-9950
///   routes it to the length arm only when
///   `valid_length_expression` (subst.c:8244) accepts the operand —
///   `${#x:-y}` keeps its bad substitution.
/// - subst.c:9843-9853: when the scan stops at the first body character,
///   only the VALID_SPECIAL_LENGTH_PARAM leads (`-?#@`) are rebuilt from
///   that character plus the terminator-bounded remainder; any other lead
///   (`${}`, `${:-x}`, `${%x}`) keeps the empty name and fails the gate.
///   A rebuilt single character is the special parameter itself
///   (`${-}`, `${-?x}`); growth past it (`${-x}`, `${@x}`) is invalid.
/// - subst.c:9914: `!`-led names take the indirect path only when the next
///   byte is an identifier starter, a digit, or VALID_INDIR_PARAM; the
///   subst.c:9960-10021 early returns (`${!P*}` ending `*`/`@`, `${!A[@]}`
///   ending `]`) stay with their own arms, and everything else is gated on
///   the name after `!`.
/// - subst.c:10042-10046: the gate itself is `valid_brace_expansion_word`
///   (subst.c:7583) — all digits, a single special parameter, an array
///   reference, or an identifier. Everything else lands on the
///   subst.c:10276-10288 `bad substitution` default: `${(M)x}` (the zsh
///   form git-completion.bash guards behind `[[ -n $ZSH_VERSION ]]`),
///   `${x(M)}`, `${1a}`, `${x!}`, `${a\b}`, `${${x}}`, `${日本}`.
pub(in crate::executor) fn braced_name_is_bad_substitution(name: &str) -> bool {
    let bytes = name.as_bytes();

    // `${#name}` with an identifier starter: whole-body length form.
    if bytes.first() == Some(&b'#') && bytes.get(1).is_some_and(|b| is_variable_starter(*b)) {
        return !valid_length_operand(&bytes[1..]);
    }

    let head = brace_name_head(bytes);
    if head.is_empty() {
        let Some(&lead) = bytes.first() else {
            return true; // `${}`
        };
        if !matches!(lead, b'-' | b'?' | b'#' | b'@') {
            return true; // `${%x}`, `${:=x}`: no fixup rebuilds these
        }
        let rest = brace_name_head(&bytes[1..]);
        let mut rebuilt = Vec::with_capacity(1 + rest.len());
        rebuilt.push(lead);
        rebuilt.extend_from_slice(rest);
        if rebuilt.len() == 1 {
            return false; // `${-}`, `${?}`, `${#}`, `${@}`
        }
        if rebuilt[0] == b'#' {
            return !valid_length_operand(&rebuilt[1..]); // `${#-}`, `${#10}`
        }
        return true; // `${-x}`, `${@x}`: invalid grown name
    }

    if head[0] == b'!' && head.len() >= 2 {
        let want_indir = is_variable_starter(head[1])
            || head[1].is_ascii_digit()
            || brace_valid_indir_param(head[1]);
        if !want_indir {
            return true; // `${!(M)x}`: gate sees the whole name
        }
        // `${!P*}` / `${!A[@]}` early returns: the terminator-bounded head
        // must run to the end of the body for the list/key forms.
        if head.len() == bytes.len() {
            let last = head[head.len() - 1];
            if matches!(last, b'*' | b'@') && is_variable_starter(head[1]) {
                return false;
            }
            if last == b']' {
                return false;
            }
        }
        return !valid_brace_gate_word(&head[1..]);
    }

    !valid_brace_gate_word(head)
}

#[cfg(test)]
mod braced_name_gate_tests {
    use super::braced_name_is_bad_substitution;

    // Every row verified against WSL GNU Bash 5.3.0 (probe scripts under
    // target/issue-suites/results/eco-param-paren/): `bad substitution`
    // reports the word and abandons the command with status 1.
    #[test]
    fn invalid_names_are_gated() {
        for name in [
            "(M)x",      // zsh-ism (git-completion.bash:403)
            "(M)x:-def", // ...with an operator tail
            "x(M)",
            "x (y)", // a blank is not a terminator
            "",      // ${}
            ":-x",   // empty name before the operator
            "%x",    // no VALID_SPECIAL_LENGTH_PARAM lead
            ":=x",
            "1a", // not all digits, not an identifier
            "x!",
            r"a\b", // the escape pair stays in the name
            "${x}", // nested ${ is name text
            "-x",   // -x: rebuilt from `-`, grows invalid
            "@x",
            "!(M)x", // `!` without an indirect lead
            "#(M)x", // length of an invalid operand
            "#x:-y", // length scan runs to `}` (subst.c:9800-9806)
            "!P*-x", // `${!P*}` early return needs the `*` last
            "日本",  // names are ASCII
        ] {
            assert!(
                braced_name_is_bad_substitution(name),
                "expected bad substitution: {name:?}"
            );
        }
    }

    #[test]
    fn valid_names_pass_the_gate() {
        for name in [
            "x",
            "PATH",
            "_v1",
            "10",
            "1",
            "", // placeholder; "" is invalid (tested above)
            "-",
            "?",
            "#",
            "@",
            "*",
            "$",
            "!",   // single specials
            "-?x", // `-` then operator
            "#x",
            "#10",
            "#-",
            "#!",
            "#a[7]",
            "#BASH_REMATCH",
            "!ref",
            "!1",
            "!@",
            "!P*",
            "!P@",
            "!arr[@]",
            "!x[a]",
            "x:-def",
            "x:-",
            "x:=$(cmd)",
            "x:- (y)",
            "x-y",
            "x+word",
            "x[0]",
            "x[$((1+1))]",
            "x[(1)]",
            "arr[@]",
            "arr[*]",
            "x:1:2",
            "x//a/b",
            "x^^",
            "x~U",
            "x@Q",
            "x#a",
            "x%pat",
            "x:-$(echo sub)",
        ] {
            if name.is_empty() {
                continue;
            }
            assert!(
                !braced_name_is_bad_substitution(name),
                "unexpected bad substitution: {name:?}"
            );
        }
    }
}
