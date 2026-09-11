use super::*;
use crate::executor::parameter_core::word_contains_current_shell_command_substitution;

// The quoted-null carrier (GNU subst.c CTLNUL): an empty quoted span
// ('', "", an unset "$e", a no-output "$( : )") in a ${var+word}
// alternate leaves a marker in the expansion so the field splitter can
// keep it as an empty argument (subst.c expand_word_internal adds CTLNUL
// to the word text at 11940-11944 / 11841-11847, and list_string:3181-3186
// keeps a QUOTED_NULL field as an empty argv entry). U+E000/U+E001 are
// taken by the raw-byte and assignment data-quote sentinels; U+E002 free.
pub(in crate::executor) const QUOTED_NULL_MARKER: char = '\u{E002}';

// Whitespace that an expansion produced inside a quoted region of an
// alternate word must survive field splitting (GNU carries CTLESC on
// quoted expansion results); the \x1c prefix marks it for the splitter.
fn mark_alternate_whitespace(value: &str) -> String {
    let mut marked = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, ' ' | '\t' | '\n') {
            marked.push('\x1c');
        }
        marked.push(ch);
    }
    marked
}

impl Executor {
    pub(in crate::executor) fn expand_embedded_parameters_mut(&mut self, word: &str) -> String {
        self.expand_embedded_parameters_mut_with_context(word, SubstitutionQuoteContext::Unquoted)
    }

    pub(in crate::executor) fn expand_embedded_parameters_mut_with_context(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        self.expand_embedded_parameters_mut_inner(word, context, false, false)
    }

    // Alternate-operator rhs (`${var-word}` word half) expansion: the same
    // walker with Unquoted quote removal plus two additions GNU applies to
    // an unquoted word's expansion (subst.c expand_string_for_rhs with
    // quoted == 0): whitespace that was quoted or escaped is marked with
    // the \x1c sentinel so the field splitter keeps it glued to its field
    // (posixexp2 37, more-exp ${B:-"$A"}), and backslash escapes resolve
    // per unquoted-word rules (\$name is a protected literal $ that must
    // NOT expand -- rhs-exp t33/t34 -- while \p drops the backslash,
    // t47). The quote structure stays in the word during the walk, so
    // expansions that happen inside a quoted region see in_double and
    // their own whitespace gets the sentinel too.
    pub(in crate::executor) fn expand_embedded_parameters_alternate_mut(
        &mut self,
        word: &str,
    ) -> String {
        self.expand_embedded_parameters_mut_inner(
            word,
            SubstitutionQuoteContext::Unquoted,
            false,
            true,
        )
    }

    // Here-string content has already had quote removal applied by the parser
    // (single quotes stripped). Bare quotes left behind are literal data, not
    // delimiters; only parameter/command/arithmetic substitution applies. This
    // mirrors GNU subst.c here-string handling, where the word is expanded
    // without re-parsing quotes.
    pub(in crate::executor) fn expand_here_string_mut(&mut self, word: &str) -> String {
        self.expand_embedded_parameters_mut_inner(
            word,
            SubstitutionQuoteContext::Unquoted,
            true,
            false,
        )
    }

    fn expand_embedded_parameters_mut_inner(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
        heredoc: bool,
        alternate: bool,
    ) -> String {
        self.apply_parameter_assignment_expansions_in_word(word);
        let saved_parameter_state = word_contains_current_shell_command_substitution(word)
            .then(|| (self.env_vars.clone(), self.pipestatus.clone()));
        let expanded = self.expand_embedded_parameters_ordered_mut(
            word,
            saved_parameter_state.as_ref(),
            context,
            heredoc,
            alternate,
        );
        let expanded = if word.contains("$(") || word.contains('`') {
            if matches!(context, SubstitutionQuoteContext::HereDocument) {
                expanded
            } else {
                unescape_remaining_shell_escapes(&expanded)
                    .replace("\\\\'", "'")
                    .replace("\\'", "'")
            }
        } else {
            expanded
        };
        let restored = restore_protected_replacement_quotes(&expanded)
            .replace('\x1f', "$")
            .replace('\x1a', "`")
            .replace('\x14', "\\")
            .replace(crate::lexer::PARAM_NAME_END_MARKER, "");
        // Quoted-null markers only matter to the alternate field splitter
        // (unquoted_outer_braced_alternate_values); every other consumer
        // (assignment rhs, quoted alternates) drops them like GNU's
        // dequote_list.
        if alternate {
            restored
        } else {
            restored.replace(QUOTED_NULL_MARKER, "")
        }
    }

    fn expand_embedded_parameters_ordered_mut(
        &mut self,
        word: &str,
        saved_parameter_state: Option<&(std::collections::HashMap<String, String>, Vec<i32>)>,
        context: SubstitutionQuoteContext,
        heredoc: bool,
        alternate: bool,
    ) -> String {
        let mut output = String::new();
        let mut chars = word.chars().peekable();
        let mut in_double = false;
        // Output length when the current double-quoted span opened, for the
        // quoted-null carrier below (alternate mode only).
        let mut dquote_open_len: Option<usize> = None;

        while let Some(ch) = chars.next() {
            // Alternate rhs: whitespace inside a quoted region is quote
            // data for the field splitter even after quote removal.
            if alternate && in_double && matches!(ch, ' ' | '\t' | '\n') {
                output.push('\x1c');
                output.push(ch);
                continue;
            }
            if ch == '\x1a' {
                output.push('`');
                continue;
            }

            if ch == crate::lexer::PARAM_NAME_END_MARKER {
                // Lexer quote removal marks where a quote boundary ends an
                // unbraced $name; the name already stopped (the marker is a
                // non-name character) and must not reach the output.
                continue;
            }

            if ch == '\x1f' {
                output.push('$');
                continue;
            }

            if ch == '\x17' {
                // Only command-substitution words cross a later quote-removal
                // pass; ordinary words retain their existing decoding path.
                if word.contains("$(") || word.contains('`') {
                    output.push('\x17');
                } else {
                    output.push('\'');
                }
                continue;
            }

            if ch == '\x18' {
                // Quoted double marks are lexer sentinels; preserve the
                // quote and its expansion context for nested single quotes.
                if matches!(context, SubstitutionQuoteContext::Unquoted) {
                    in_double = !in_double;
                }
                output.push('"');
                continue;
            }

            // Quotes that survive to expansion belong to parameter-expansion
            // bodies (the lexer keeps `${...}` verbatim). GNU removes them
            // only in unquoted expansions; a double-quoted expansion keeps
            // them in its result (`"${IFS+'}'z}"` -> `'}'z`). Heredoc text
            // treats quotes as data.
            if !heredoc && matches!(context, SubstitutionQuoteContext::Unquoted) && ch == '"' {
                let closing = in_double;
                in_double = !in_double;
                // A double-quoted span whose expansion produced nothing is a
                // quoted null: "" and $xxx"" become CTLNUL (subst.c
                // 11841-11847 "What we have is \"\""). Alternate mode only:
                // the marker is consumed by the alternate field splitter.
                if alternate {
                    if closing {
                        if dquote_open_len == Some(output.len()) {
                            output.push(QUOTED_NULL_MARKER);
                        }
                        dquote_open_len = None;
                    } else {
                        dquote_open_len = Some(output.len());
                    }
                }
                continue;
            }

            if !heredoc
                && matches!(context, SubstitutionQuoteContext::Unquoted)
                && ch == '\''
                && !in_double
            {
                let span_start = output.len();
                let mut closed = false;
                for quoted_ch in chars.by_ref() {
                    if quoted_ch == '\'' {
                        closed = true;
                        break;
                    }
                    if alternate && matches!(quoted_ch, ' ' | '\t' | '\n') {
                        output.push('\x1c');
                    }
                    output.push(quoted_ch);
                }
                // An empty single-quoted span is a quoted null (subst.c
                // 11940-11944: c = CTLNUL; goto add_character).
                if alternate && closed && output.len() == span_start {
                    output.push(QUOTED_NULL_MARKER);
                }
                continue;
            }

            if ch == '\\' && alternate {
                // Unquoted-word escape rules for the alternate rhs: $, `,
                // \\, " keep their protection; an escaped blank stays out
                // of field splitting; a backslash before any other
                // character is removed (GNU quote removal).
                match chars.peek().copied() {
                    Some('`') => {
                        chars.next();
                        output.push('\x1a');
                        continue;
                    }
                    Some('$') | Some('"') => {
                        let next = chars.next().unwrap();
                        output.push(next);
                        continue;
                    }
                    Some('\\') => {
                        chars.next();
                        output.push('\\');
                        continue;
                    }
                    Some('\'') => {
                        chars.next();
                        if in_double {
                            // GNU dquote rule: a backslash before an
                            // ordinary character is literal data.
                            output.push('\\');
                        }
                        output.push('\'');
                        continue;
                    }
                    Some('\n') | Some('\r') => {
                        chars.next();
                        continue;
                    }
                    Some(ws @ (' ' | '\t')) => {
                        chars.next();
                        output.push('\x1c');
                        output.push(ws);
                        continue;
                    }
                    Some(other) => {
                        chars.next();
                        if in_double {
                            // GNU dquote rule: a backslash before an
                            // ordinary character is literal data.
                            output.push('\\');
                        }
                        output.push(other);
                        continue;
                    }
                    None => {}
                }
            }

            if ch == '\\' {
                if let Some(&next) = chars.peek() {
                    match next {
                        '`' => {
                            chars.next();
                            output.push('\x1a');
                            continue;
                        }
                        // A backslash inside a parameter body protects the
                        // next special character (GNU subst.c: the backslash
                        // keeps its escaping meaning before $, `, ", \.
                        // Emit $ and " literally here: by the time the later
                        // unescape pass runs, an unprotected $ has already
                        // opened a parameter expansion (esc6/esc7 probes:
                        // "${v-\$x}" must yield a$x, not drop the $x).
                        '$' => {
                            chars.next();
                            output.push(next);
                            continue;
                        }
                        '"' if !matches!(context, SubstitutionQuoteContext::HereDocument) => {
                            chars.next();
                            output.push(next);
                            continue;
                        }
                        // Here-document bodies expand with Q_HERE_DOCUMENT,
                        // where the escape set is CBSHDOC and not CBSDQUOTE
                        // (subst.c:11628; syntax.h slashify_in_here_document
                        // = backslash, backtick, dollar). The double quote is
                        // not special inside an unquoted heredoc body, so a
                        // backslash before one is literal data (heredoc.tests
                        // "echo \""). Fall through to the literal copy.
                        _ => {}
                    }
                }
            }

            if ch == '`' {
                let mut source = String::new();
                let mut escaped = false;
                let mut closed = false;
                for source_ch in chars.by_ref() {
                    if escaped {
                        if !matches!(source_ch, '$' | '`' | '\\' | '\n' | '\r') {
                            source.push('\\');
                        }
                        source.push(source_ch);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if source_ch == '`' {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                if closed {
                    let expanded = self
                        .expand_command_substitution_mut_typed_with_context(&source, context)
                        .text_lossy();
                    let protected = protect_command_substitution_output(&expanded);
                    if alternate && in_double {
                        let value = mark_alternate_whitespace(&protected);
                        if matches!(context, SubstitutionQuoteContext::HereDocument) {
                            output.push_str(&value.replace('\x15', "\x14"));
                        } else {
                            output.push_str(&value);
                        }
                    } else if matches!(context, SubstitutionQuoteContext::HereDocument) {
                        output.push_str(&protected.replace('\x15', "\x14"));
                    } else {
                        output.push_str(&protected);
                    }
                } else {
                    output.push('`');
                    output.push_str(&source);
                }
                continue;
            }

            if ch != '$' {
                output.push(ch);
                continue;
            }

            match chars.peek().copied() {
                Some('?') => {
                    chars.next();
                    output.push_str(&self.exit_code.to_string());
                }
                Some('$') => {
                    chars.next();
                    output.push_str(&self.shell_pid_value().to_string());
                }
                Some('!') => {
                    chars.next();
                    output.push_str(&self.last_background_pid_value());
                }
                Some('@') => {
                    chars.next();
                    output.push_str(&self.positional_params.join(" "));
                }
                Some('*') => {
                    chars.next();
                    // Bash joins `$*` with the first IFS character (not a space).
                    output.push_str(&self.positional_params_star_joined());
                }
                Some('#') => {
                    chars.next();
                    output.push_str(&self.positional_params.len().to_string());
                }
                Some('-') => {
                    chars.next();
                    output.push_str(&self.shell_option_flags());
                }
                Some('{') => {
                    chars.next();
                    if let Some(value) = self.expand_current_shell_braced_substitution(&mut chars) {
                        output.push_str(&value);
                    } else if matches!(context, SubstitutionQuoteContext::DoubleQuoted)
                        && self.posix_mode_enabled()
                    {
                        let remainder: String = chars.clone().collect();
                        if let Some(close) =
                            matching_parameter_brace_in_context(&remainder, true, true)
                        {
                            let consumed = word.len() - remainder.len();
                            let name = remainder[..close].to_string();
                            chars = word[consumed + close + 1..].chars().peekable();
                            let value =
                                self.expand_with_parameter_env(saved_parameter_state, |executor| {
                                    executor.expand_word_mut_with_context(
                                        &format!("${{{name}}}"),
                                        context,
                                    )
                                });
                            if alternate && in_double {
                                output.push_str(&mark_alternate_whitespace(&value));
                            } else {
                                output.push_str(&value);
                            }
                        } else {
                            let name = collect_braced_parameter_name(&mut chars);
                            let value =
                                self.expand_with_parameter_env(saved_parameter_state, |executor| {
                                    executor.expand_word_mut_with_context(
                                        &format!("${{{name}}}"),
                                        context,
                                    )
                                });
                            if alternate && in_double {
                                output.push_str(&mark_alternate_whitespace(&value));
                            } else {
                                output.push_str(&value);
                            }
                        }
                    } else {
                        let name = collect_braced_parameter_name(&mut chars);
                        let value =
                            self.expand_with_parameter_env(saved_parameter_state, |executor| {
                                // Propagate the outer quote context so a
                                // double-quoted "${v:-~}" keeps its quoted
                                // default-word semantics (no tilde expansion).
                                executor
                                    .expand_word_mut_with_context(&format!("${{{name}}}"), context)
                            });
                        if alternate && in_double {
                            output.push_str(&mark_alternate_whitespace(&value));
                        } else {
                            output.push_str(&value);
                        }
                    }
                }
                Some('(') => {
                    chars.next();
                    if chars.peek().copied() == Some('(') {
                        chars.next();
                        let (expression, matched) =
                            collect_dollar_paren_arithmetic_expansion(&mut chars);
                        if matched {
                            if let Some(value) = self.eval_arithmetic_expansion_value(&expression) {
                                output.push_str(&value.to_string());
                            } else {
                                let actual_fatal =
                                    self.arithmetic_last_error_category.take().is_some();
                                if (actual_fatal
                                    || crate::executor::arithmetic::arithmetic_expansion_is_fatal(
                                        &expression,
                                    ))
                                    && !embedded_command_substitution_expression(&expression)
                                {
                                    self.arithmetic_fatal_error.set(true);
                                    if !self.arithmetic_expansion_error.replace(true) {
                                        if let Some(message) =
                                            crate::executor::arithmetic::arithmetic_error_message(
                                                &expression,
                                                true,
                                            )
                                        {
                                            eprintln!("{}{}", self.diagnostic_prefix(), message);
                                        }
                                    }
                                } else {
                                    let value = protect_command_substitution_output(
                                        &self.expand_command_substitution_mut_with_context(
                                            &expression,
                                            context,
                                        ),
                                    );
                                    if alternate && in_double {
                                        output.push_str(&mark_alternate_whitespace(&value));
                                    } else {
                                        output.push_str(&value);
                                    }
                                }
                            }
                        } else {
                            output.push_str("$((");
                            output.push_str(&expression);
                        }
                        continue;
                    }

                    let source = collect_command_substitution_source(&mut chars);
                    let value = protect_command_substitution_output(
                        &self.expand_command_substitution_mut_with_context(&source, context),
                    );
                    if alternate && in_double {
                        output.push_str(&mark_alternate_whitespace(&value));
                    } else {
                        output.push_str(&value);
                    }
                }
                Some('[') => {
                    chars.next();
                    let (expression, matched) =
                        collect_dollar_bracket_arithmetic_expansion(&mut chars);
                    if matched {
                        if let Some(value) = self.eval_arithmetic_expansion_value(&expression) {
                            output.push_str(&value.to_string());
                        }
                    } else {
                        output.push_str("$[");
                        output.push_str(&expression);
                    }
                }
                Some(first) if first.is_ascii_digit() => {
                    chars.next();
                    let index = first.to_digit(10).unwrap_or(0) as usize;
                    if index == 0 {
                        let value = self.script_name_value();
                        if alternate && in_double {
                            output.push_str(&mark_alternate_whitespace(&value));
                        } else {
                            output.push_str(&value);
                        }
                    } else {
                        output.push_str(
                            self.positional_params
                                .get(index - 1)
                                .map(String::as_str)
                                .unwrap_or(""),
                        );
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
                    if let Some(value) =
                        self.expand_with_parameter_env(saved_parameter_state, |executor| {
                            executor.dynamic_parameter_value(&name).or_else(|| {
                                executor
                                    .shell_variable_value(&name)
                                    .or_else(|| std::env::var(&name).ok())
                            })
                        })
                    {
                        let value = shell_safe_value(&value);
                        if alternate && in_double {
                            output.push_str(&mark_alternate_whitespace(&value));
                        } else {
                            output.push_str(&value);
                        }
                    }
                }
                // GNU subst.c: `$'...'` is a self-contained ANSI-C quoted
                // string. Consuming only the `$` and the opening quote would
                // leave the closing quote for the single-quoted span pass
                // below, which then swallows the rest of the word (unicode1.sub
                // array elements: `A=([k]=$'\001')` stored `$'\001`).
                // Decode the span here, the way the lexer does for whole words.
                Some('\'') => {
                    chars.next();
                    let mut quoted = String::new();
                    let mut escaped = false;
                    let mut closed = false;
                    for quoted_ch in chars.by_ref() {
                        if escaped {
                            quoted.push('\\');
                            quoted.push(quoted_ch);
                            escaped = false;
                            continue;
                        }
                        if quoted_ch == '\\' {
                            escaped = true;
                            continue;
                        }
                        if quoted_ch == '\'' {
                            closed = true;
                            break;
                        }
                        quoted.push(quoted_ch);
                    }
                    if escaped {
                        quoted.push('\\');
                    }
                    if closed {
                        let decoded = crate::lexer::decode_ansi_c_quoted(&quoted);
                        if alternate {
                            for ch in decoded.chars() {
                                if matches!(ch, ' ' | '\t' | '\n') {
                                    output.push('\x1c');
                                }
                                output.push(ch);
                            }
                        } else if decoded
                            .chars()
                            .any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r'))
                        {
                            output.push('"');
                            for ch in decoded.chars() {
                                match ch {
                                    '\\' => output.push_str("\\\\"),
                                    '"' => output.push_str("\\\""),
                                    '$' => output.push_str("\\$"),
                                    '`' => output.push_str("\\`"),
                                    _ => output.push(ch),
                                }
                            }
                            output.push('"');
                        } else {
                            output.push_str(&decoded);
                        }
                    } else {
                        output.push('$');
                        output.push('\'');
                        output.push_str(&quoted);
                    }
                }
                Some(other) => {
                    chars.next();
                    output.push('$');
                    output.push(other);
                }
                None => output.push('$'),
            }
        }

        output
    }

    fn expand_with_parameter_env<T>(
        &mut self,
        saved_parameter_state: Option<&(std::collections::HashMap<String, String>, Vec<i32>)>,
        expand: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let Some((saved_parameter_env, saved_parameter_pipestatus)) = saved_parameter_state else {
            return expand(self);
        };

        let current_env = std::mem::replace(&mut self.env_vars, saved_parameter_env.clone());
        let current_pipestatus =
            std::mem::replace(&mut self.pipestatus, saved_parameter_pipestatus.clone());
        let expanded = expand(self);
        self.env_vars = current_env;
        self.pipestatus = current_pipestatus;
        expanded
    }

    pub(in crate::executor) fn expand_current_shell_braced_substitution(
        &mut self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> Option<String> {
        let pipe_output = chars.peek().copied() == Some('|');
        if pipe_output {
            chars.next();
        } else if !chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            return None;
        }

        let mut depth = 1usize;
        let mut source = String::new();
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        let mut closed = false;
        for source_ch in chars.by_ref() {
            if escaped {
                source.push(source_ch);
                escaped = false;
                continue;
            }
            if source_ch == '\\' && !single {
                source.push(source_ch);
                escaped = true;
                continue;
            }
            match source_ch {
                '\'' if !double => {
                    single = !single;
                    source.push(source_ch);
                }
                '"' if !single => {
                    double = !double;
                    source.push(source_ch);
                }
                '{' if !single && !double => {
                    depth += 1;
                    source.push(source_ch);
                }
                '}' if !single && !double => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                _ => source.push(source_ch),
            }
        }

        if closed {
            Some(protect_command_substitution_output(
                &self.expand_current_shell_command_substitution(&source, pipe_output),
            ))
        } else {
            let mut literal = if pipe_output {
                "${|".to_string()
            } else {
                "${".to_string()
            };
            literal.push_str(&source);
            Some(literal)
        }
    }

    fn expand_current_shell_command_substitution(
        &mut self,
        source: &str,
        pipe_output: bool,
    ) -> String {
        // The body text was cut out of the current word, so its tokens would
        // restart at line 1. GNU parse.y keeps the in-place line counter for
        // command substitutions — diagnostics inside the body report the
        // original script line of the substitution (comsub2.tests: line 68).
        let body_start_line = self
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let source = &self.comsub_body_alias_splice(source);
        let tokens = crate::lexer::tokenize_comsub_body(
            source,
            self.posix_mode_enabled(),
            body_start_line,
            true,
        );
        let ast = crate::parser::parse(&tokens);

        // GNU subst.c nofork substitution: the body's stdout is captured by
        // the walker itself (a valsub `${| ...; }` discards it — it never
        // reaches the enclosing output — while a funsub `${ ...; }` returns
        // it as the expansion value).
        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        // GNU subst.c valsub: REPLY is the value channel and the body's own
        // REPLY state is scoped to the body — the caller's value comes back
        // afterwards (comsub26.sub: `inside1-inside2-outside`).
        let saved_reply = self.env_vars.get("REPLY").cloned();
        if pipe_output {
            // Seed every frame store so restore_function_locals puts the
            // caller's REPLY back in env_vars, the typed variable table, and
            // the attribute set (assignments inside the body touch all three).
            self.local_var_scopes.push(HashMap::new());
            self.local_attr_scopes.push(HashMap::new());
            self.local_typed_scopes.push(HashMap::new());
            if let Some(scope) = self.local_var_scopes.last_mut() {
                scope.insert("REPLY".to_string(), saved_reply.clone());
            }
            if let Some(typed) = self.local_typed_scopes.last_mut() {
                typed.insert(
                    "REPLY".to_string(),
                    self.shell_state.variables.get("REPLY").cloned(),
                );
            }
        }
        let result = if pipe_output {
            // The seeded frame is already on the stack; run the body without
            // pushing another one.
            self.execute_ast(&ast)
        } else {
            self.execute_current_shell_body(&ast)
        };
        let body_reply = self.env_vars.get("REPLY").cloned();
        match saved_reply {
            Some(value) => {
                self.env_vars.insert("REPLY".to_string(), value);
            }
            None => {
                self.env_vars.remove("REPLY");
            }
        }
        let captured = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;

        // `exit N` inside the body aborts the enclosing (sub)shell with N
        // (comsub26.sub line 32: the subshell never prints and $? = 42).
        if let Err(crate::executor::ExecuteError::ExitCode(code)) = &result {
            self.current_shell_substitution_exit.set(Some(*code));
        }
        let status = command_substitution_status(result, self.exit_code);

        // The body's own exit status is $? for expansions later on the same
        // command line (comsub26.sub line 35: `echo ${ ...; return 42; } $?`
        // prints `var=inside 42`); the finished command then overwrites $?
        // with its own status as usual.
        self.exit_code = status;
        self.last_command_substitution_status.set(Some(status));

        if pipe_output {
            // GNU subst.c valsub: the expansion value is the body-final
            // REPLY, while the caller's REPLY is restored afterwards
            // (comsub26.sub: `inside1-inside2-outside`).
            body_reply.unwrap_or_default()
        } else {
            bytes_to_shell_text(&captured)
                .trim_end_matches('\n')
                .to_string()
        }
    }

    /// Bash 5.3 nofork command substitution (subst.c): the `${ command; }` /
    /// `${| command; }` body runs in the current shell but with a
    /// function-like variable frame — `local` scopes to the body and `return`
    /// ends only the body, while plain assignments still mutate the current
    /// environment (comsub2.tests: `outside: 42` vs `outside:` empty).
    fn execute_current_shell_body(&mut self, ast: &crate::parser::Ast) -> Result<(), ExecuteError> {
        self.local_var_scopes.push(HashMap::new());
        self.local_attr_scopes.push(HashMap::new());
        self.local_typed_scopes.push(HashMap::new());
        self.function_depth += 1;
        let result = self.execute_ast(ast);
        self.function_depth -= 1;
        self.restore_function_locals();
        result
    }

    pub(in crate::executor) fn expand_command_substitution_mut_typed_with_context(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> SubstitutionOutput {
        let source = source.trim();
        let words = self.expand_aliases(&split_shell_words(source));
        if let Some(output) = self.command_substitution_heredoc_output_mut_typed(source, context) {
            return output;
        }
        let saved_positional_params = self.positional_params.clone();
        if let Some(output) = self.run_function_command_substitution(&words) {
            self.set_positional_params(saved_positional_params);
            let status = self.last_command_substitution_status.get().unwrap_or(0);
            return SubstitutionOutput::readback(output.into_bytes(), status, context);
        }
        self.set_positional_params(saved_positional_params);
        if command_substitution_words_contain_here_string(&words) {
            let alias_source = words.join(" ");
            if let Some(output) =
                self.run_ast_command_substitution_with_context(&alias_source, context)
            {
                return output;
            }
        }
        if command_substitution_uses_specialized_path(self, source, &words) {
            let output = self.expand_command_substitution_with_context(source, context);
            let status = self.last_command_substitution_status.get().unwrap_or(0);
            return SubstitutionOutput::readback(output.into_bytes(), status, context);
        }
        // A command list (`echo a; echo b`, `a && b`) must run as an AST, not
        // through the single-command specialized dispatch below: routing
        // `echo mn; echo op` to the echo shortcut treats `;` as an argument
        // and yields `mn; echo op` instead of `mn\nop` (comsub.tests
        // `ab$(echo mn; echo op)yz`). A quoted `a;b` argument is harmless to
        // route here too: the AST still prints it correctly. Newlines in the
        // raw source are command separators the same way (old-style
        // backticks span lines: `echo ab\ncd` runs two commands).
        if words
            .iter()
            .any(|word| word.contains(';') || matches!(word.as_str(), "&&" | "||"))
            || source.contains('\n')
        {
            if let Some(output) = self.run_ast_command_substitution_with_context(source, context) {
                return output;
            }
        }
        // Simple builtins that the non-mut special-case dispatch handles with
        // proper quote stripping (echo/printf/cat/...). Prefer that path so
        // nested `"$(...)"` arguments do not leak quote characters through
        // the full-AST execution path.
        if is_specialized_command_substitution_word(&words) {
            let output = self.expand_command_substitution_with_context(source, context);
            let status = self.last_command_substitution_status.get().unwrap_or(0);
            return SubstitutionOutput::readback(output.into_bytes(), status, context);
        }
        if let Some(output) = self.run_ast_command_substitution_with_context(source, context) {
            return output;
        }
        let output = self.expand_command_substitution_with_context(source, context);
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        SubstitutionOutput::readback(output.into_bytes(), status, context)
    }

    pub(in crate::executor) fn expand_command_substitution_mut_with_context(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        self.expand_command_substitution_mut_typed_with_context(source, context)
            .text_lossy()
    }

    pub(in crate::executor) fn run_ast_command_substitution_with_context(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<SubstitutionOutput> {
        if command_substitution_contains_heredoc(source) {
            return None;
        }

        // Keep GNU's in-place line counter: the body was extracted from the
        // current word, so body diagnostics must report the original script
        // line instead of restarting at 1.
        let body_start_line = self
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let source = &self.comsub_body_alias_splice(source);
        let tokens = crate::lexer::tokenize_comsub_body(
            source,
            self.posix_mode_enabled(),
            body_start_line,
            true,
        );
        let ast = crate::parser::parse(&tokens);
        if !command_substitution_needs_ast_execution(&ast) {
            return None;
        }

        let saved_env = self.env_vars.clone();
        let saved_pipestatus = self.pipestatus.clone();
        let saved_functions = self.functions.clone();
        let saved_function_redirects = self.function_definition_redirects.clone();
        let saved_function_def_infos = self.function_def_infos.clone();
        let saved_aliases = self.aliases.clone();
        let saved_exit_code = self.exit_code;
        let saved_positional_params = self.positional_params.clone();
        let saved_dir = env::current_dir().ok();
        let saved_depth = self.subshell_depth.get();
        self.subshell_depth.set(saved_depth + 1);

        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        // Bash runs command substitution in a subshell where errexit is
        // suppressed: `$(false; echo ok)` prints ok because the inner `false`
        // does not abort the substitution (set-e.tests "command subst should
        // not inherit -e"); only the substitution's final status (echo's 0)
        // propagates to the outer assignment, which then checks -e.
        // POSIX mode is the exception: `set -o posix; z=$(false;echo posix)`
        // exits (set-e1.sub), so keep errexit active there.
        let posix_mode = self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1");
        let inherit_errexit =
            crate::builtins::shopt::option_enabled(&self.env_vars, "inherit_errexit");
        let result = if posix_mode || inherit_errexit {
            self.execute_ast(&ast)
        } else {
            self.with_errexit_suppressed(|executor| executor.execute_ast(&ast))
        };
        let output = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;

        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => {
                status
            }
            Err(_) => 1,
        };

        self.restore_shell_env(saved_env);
        self.pipestatus = saved_pipestatus;
        self.functions = saved_functions;
        self.function_definition_redirects = saved_function_redirects;
        self.function_def_infos = saved_function_def_infos;
        self.aliases = saved_aliases;
        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        self.subshell_depth.set(saved_depth);
        self.set_positional_params(saved_positional_params);
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(SubstitutionOutput::readback(output, status, context))
    }

    pub(in crate::executor) fn run_function_command_substitution(
        &mut self,
        words: &[String],
    ) -> Option<String> {
        let name = words.first()?;
        if !self.functions.contains_key(name) {
            return None;
        }
        // A function call is only a shortcut when the substitution body is a
        // single simple command. GNU subst.c parses the body into a command
        // list first: `$(f a b | wc -l)` must pipe f's output through wc,
        // not run f with `| wc -l` in its positional params (issue #70).
        if command_substitution_words_have_operators(words) {
            return None;
        }

        let args = words[1..]
            .iter()
            .flat_map(|word| self.expand_command_substitution_arg_values(word))
            .collect::<Vec<_>>();
        let mut call = CommandNode::new();
        call.words = words.to_vec();

        let saved_env = self.env_vars.clone();
        let saved_pipestatus = self.pipestatus.clone();
        let saved_exit_code = self.exit_code;
        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        let result = self.execute_function(name, &args, &call);
        let output = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;
        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => {
                status
            }
            Err(_) => 1,
        };
        self.env_vars = saved_env;
        self.pipestatus = saved_pipestatus;
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(
            bytes_to_shell_text(&output)
                .trim_end_matches('\n')
                .to_string(),
        )
    }
}

fn command_substitution_needs_ast_execution(ast: &Ast) -> bool {
    ast.commands.iter().any(command_has_ast_substitution_shape)
        || ast
            .commands
            .iter()
            .any(command_contains_current_shell_substitution)
        || (ast.commands.len() > 1 && ast.commands.iter().all(command_is_ast_list_substitution))
}

// GNU subst.c treats a syntactically command-like $((...)) body as a
// command substitution fallback, even though arithmetic parsing rejects it.
// Keep ordinary arithmetic diagnostics (notably 1/0 and invalid octal 08).
fn embedded_command_substitution_expression(expression: &str) -> bool {
    expression.contains(';') && expression.contains('(')
}

fn collect_dollar_paren_arithmetic_expansion(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> (String, bool) {
    let mut expression = String::new();
    let mut paren_depth: usize = 0;

    while let Some(ch) = chars.next() {
        match ch {
            '(' => {
                paren_depth += 1;
                expression.push(ch);
            }
            // A command-like body may end with its own `)` immediately before
            // the arithmetic expansion's closing `)`: `(echo hi))`.
            ')' if paren_depth == 1 && chars.peek().copied() == Some(')') => {
                expression.push(ch);
                chars.next();
                return (expression, true);
            }
            ')' if paren_depth == 0 && chars.peek().copied() == Some(')') => {
                chars.next();
                return (expression, true);
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                expression.push(ch);
            }
            _ => expression.push(ch),
        }
    }

    (expression, false)
}

fn collect_dollar_bracket_arithmetic_expansion(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> (String, bool) {
    let mut expression = String::new();
    let mut bracket_depth: usize = 0;

    for ch in chars.by_ref() {
        match ch {
            '[' => {
                bracket_depth += 1;
                expression.push(ch);
            }
            ']' if bracket_depth == 0 => return (expression, true),
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                expression.push(ch);
            }
            _ => expression.push(ch),
        }
    }

    (expression, false)
}

fn collect_command_substitution_source(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> String {
    let mut depth = 1usize;
    let mut source = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;

    while let Some(source_ch) = chars.next() {
        if escaped {
            source.push(source_ch);
            escaped = false;
            continue;
        }
        if source_ch == '\\' && !single {
            source.push(source_ch);
            escaped = true;
            continue;
        }
        if source_ch == '#' && !single && !double && word_boundary {
            source.push(source_ch);
            while let Some(comment_ch) = chars.peek().copied() {
                if comment_ch == '\n' {
                    break;
                }
                source.push(comment_ch);
                chars.next();
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            continue;
        }
        if source_ch == '`' && !single {
            source.push(source_ch);
            let mut backtick_escaped = false;
            for backtick_ch in chars.by_ref() {
                source.push(backtick_ch);
                if backtick_escaped {
                    backtick_escaped = false;
                    continue;
                }
                if backtick_ch == '\\' {
                    backtick_escaped = true;
                    continue;
                }
                if backtick_ch == '`' {
                    break;
                }
            }
            continue;
        }
        if source_ch == '<' && !single && !double && chars.peek().copied() == Some('<') {
            let mut lookahead = chars.clone();
            lookahead.next();
            if lookahead.peek().copied() != Some('<') {
                source.push(source_ch);
                source.push(chars.next().expect("heredoc second less-than"));
                let mut header = String::new();
                while let Some(header_ch) = chars.next() {
                    source.push(header_ch);
                    if header_ch == '\n' {
                        break;
                    }
                    header.push(header_ch);
                }
                let raw_delimiter = header.trim_end().trim_start_matches('-').trim();
                let delimiter = raw_delimiter
                    .trim_matches('\'')
                    .trim_matches('\"')
                    .to_string();
                if !delimiter.is_empty() {
                    let mut body_line = String::new();
                    while let Some(body_ch) = chars.next() {
                        source.push(body_ch);
                        if body_ch == '\n' {
                            if body_line.trim_end() == delimiter {
                                break;
                            }
                            body_line.clear();
                        } else {
                            body_line.push(body_ch);
                        }
                    }
                }
                continue;
            }
        }

        let rest = chars.clone().collect::<String>();
        update_command_substitution_case_depth(
            source_ch,
            single,
            double,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            &rest,
        );
        match source_ch {
            '\'' if !double => {
                single = !single;
                source.push(source_ch);
            }
            '"' if !single => {
                double = !double;
                source.push(source_ch);
            }
            '(' if !single && !double && case_depth == 0 => {
                depth += 1;
                source.push(source_ch);
            }
            ')' if !single && !double && case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
                source.push(source_ch);
            }
            _ => source.push(source_ch),
        }
    }

    unescape_storage_command_substitution_source(&source)
}

fn command_substitution_status(result: Result<(), ExecuteError>, exit_code: i32) -> i32 {
    match result {
        Ok(()) => exit_code,
        Err(ExecuteError::Return(status)) => status,
        Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => status,
        Err(_) => 1,
    }
}

fn command_has_ast_substitution_shape(command: &CommandNode) -> bool {
    command.and_or_list.is_some()
        || command.inverted_command.is_some()
        || command.background_command.is_some()
        || command_has_here_string_substitution(command)
        || command_has_compound_substitution(command)
}

fn command_has_compound_substitution(command: &CommandNode) -> bool {
    command.pipeline_command.as_ref().is_some_and(|pipeline| {
        pipeline
            .stages
            .iter()
            .any(command_has_compound_substitution)
    }) || command
        .and_or_list
        .as_ref()
        .is_some_and(|list| list.commands.iter().any(command_has_compound_substitution))
        || command
            .inverted_command
            .as_ref()
            .is_some_and(|inverted| command_has_compound_substitution(&inverted.command))
        || command
            .time_command
            .as_ref()
            .is_some_and(|time| command_has_compound_substitution(&time.command))
        || command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.select_command.is_some()
        || command.case_command.is_some()
        || command.coproc_command.is_some()
        || command.subshell_command.is_some()
        || command.brace_group.is_some()
        || command.arithmetic_command.is_some()
        || command.conditional_command.is_some()
}

fn command_contains_current_shell_substitution(command: &CommandNode) -> bool {
    command
        .words
        .iter()
        .any(|word| word_contains_current_shell_command_substitution(word))
}

fn command_has_here_string_substitution(command: &CommandNode) -> bool {
    command.here_string.is_some()
        || command
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.here_string)
        || command.pipeline_command.as_ref().is_some_and(|pipeline| {
            pipeline
                .stages
                .iter()
                .any(command_has_here_string_substitution)
        })
        || command.and_or_list.as_ref().is_some_and(|list| {
            list.commands
                .iter()
                .any(command_has_here_string_substitution)
        })
        || command
            .inverted_command
            .as_ref()
            .is_some_and(|inverted| command_has_here_string_substitution(&inverted.command))
        || command
            .time_command
            .as_ref()
            .is_some_and(|time| command_has_here_string_substitution(&time.command))
}

fn command_is_ast_list_substitution(command: &CommandNode) -> bool {
    if !command_has_simple_substitution_shape(command) {
        return false;
    }
    if !command.assignments.is_empty() {
        return true;
    }
    matches!(
        command.words.first().map(String::as_str),
        Some("echo" | "printf" | "true" | "false" | ":" | "pwd")
    )
}

fn command_has_simple_substitution_shape(command: &CommandNode) -> bool {
    command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.inverted_command.is_none()
        && command.background_command.is_none()
        && command.time_command.is_none()
        && command.for_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.select_command.is_none()
        && command.case_command.is_none()
        && command.coproc_command.is_none()
        && command.subshell_command.is_none()
        && command.brace_group.is_none()
        && command.arithmetic_command.is_none()
        && command.conditional_command.is_none()
}

fn command_substitution_uses_specialized_path(
    executor: &Executor,
    source: &str,
    words: &[String],
) -> bool {
    command_substitution_contains_heredoc(source)
        || (words.iter().any(|word| word == "|")
            && !command_substitution_contains_here_string(source))
        || words.first().map(String::as_str) == Some("time")
        || executor
            .command_substitution_cd_pwd_output(source)
            .is_some()
}

fn command_substitution_words_contain_here_string(words: &[String]) -> bool {
    words
        .iter()
        .any(|word| word == "<<<" || word.ends_with("<<<"))
}

/// Builtin commands whose command-substitution output is produced by the
/// non-mut special-case dispatch in `expand_command_substitution` (echo,
/// printf, cat, basename, ...). Routing these through that path keeps nested
/// `"$(...)"` argument quote handling consistent with Bash.
fn is_specialized_command_substitution_word(words: &[String]) -> bool {
    matches!(
        words.first().map(String::as_str),
        Some(
            "echo"
                | "recho"
                | "zecho"
                | "printf"
                | "cat"
                | "basename"
                | "umask"
                | "ulimit"
                | "pwd"
                | "type"
                | "kill"
                | "trap"
                | "mktemp"
                | "set"
                | "export"
                | "true"
                | ":"
        )
    )
}

fn command_substitution_contains_here_string(source: &str) -> bool {
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '<' if !single && !double && chars.peek().copied() == Some('<') => {
                chars.next();
                if chars.peek().copied() == Some('<') {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn command_substitution_contains_heredoc(source: &str) -> bool {
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '<' if !single && !double && chars.peek().copied() == Some('<') => {
                chars.next();
                if chars.peek().copied() == Some('<') {
                    chars.next();
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }

    false
}

pub(in crate::executor) fn update_command_substitution_case_depth(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    rest: &str,
) {
    if single || double {
        word.clear();
        *word_boundary = false;
        return;
    }

    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary => {
            *case_depth += 1;
            false
        }
        "esac" if *current_word_boundary && !case_pattern_starts_with_esac_rest(ch, rest) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary =>
        {
            true
        }
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

fn case_pattern_starts_with_esac_rest(delimiter: char, rest: &str) -> bool {
    if !matches!(delimiter, ')' | '|') {
        return false;
    }

    let chars = std::iter::once(delimiter)
        .chain(rest.chars())
        .collect::<Vec<_>>();
    let mut close = 0usize;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return false,
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        return false;
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return true;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        if ch == ')' {
            return false;
        }
        if word.is_empty() {
            if command_substitution_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && command_substitution_reserved_word_allows_next(&word);
        word.clear();
        word_boundary =
            reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn command_substitution_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}
