use super::*;

impl Executor {
    pub(in crate::executor) fn reparse_reserved_word_aliases(
        &self,
        source: &str,
    ) -> Option<String> {
        let mut tokens = crate::lexer::tokenize(source);
        let mut changed = false;
        for token in &mut tokens {
            if !matches!(
                token.kind,
                crate::lexer::TokenKind::Word | crate::lexer::TokenKind::Keyword
            ) {
                continue;
            }
            let Some(alias) = self.aliases.get(&token.value) else {
                continue;
            };
            let value = alias.value.trim();
            if !matches!(value, "if" | "then" | "elif" | "else" | "fi") {
                continue;
            }
            token.value = value.to_string();
            token.raw = value.to_string();
            changed = true;
        }
        if !changed {
            return None;
        }
        Some(
            tokens
                .iter()
                .filter(|token| token.kind != crate::lexer::TokenKind::Eof)
                .map(|token| token.raw.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    pub(in crate::executor) fn report_arithmetic_error_with_label(
        &self,
        label: &str,
        expression: &str,
        trailing_space: bool,
    ) {
        if let Some(token) = arithmetic_division_by_zero_token(expression) {
            eprintln!(
                "{}{}: {expression}: division by 0 (error token is \"{token}\")",
                self.diagnostic_prefix(),
                label
            );
        } else if let Some(message) =
            crate::executor::arithmetic::arithmetic_error_message(expression, trailing_space)
        {
            eprintln!("{}{}: {message}", self.diagnostic_prefix(), label);
        }
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    pub(in crate::executor) fn report_arithmetic_error(&self, expression: &str) {
        self.report_arithmetic_error_with_label("((", expression, true);
    }

    /// GNU expr.c::evalerror for the raw-captured `(( ))` expression: the
    /// echoed expression skips leading blanks only, and the division-by-0
    /// error token is the real lasttp remainder (trailing blank included),
    /// so no synthesized token space is added. All other diagnostics keep
    /// the established normalized-expression path byte for byte.
    ///
    /// Arithmetic-for loop sections report through the same raw-display
    /// contract: the parser records each section's whitespace-carrying text
    /// (`7++ ` keeps its trailing blank before `))`), and the error token is
    /// the raw suffix, so the caller must not synthesize an extra space.
    pub(in crate::executor) fn report_arithmetic_error_raw_display(&self, raw_display: &str) {
        let display = raw_display.trim_start_matches([' ', '\t']);
        if let Some(message) = crate::executor::arithmetic::arithmetic_error_message(display, false)
        {
            eprintln!("{}((: {message}", self.diagnostic_prefix());
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
    }

    pub(in crate::executor) fn report_arithmetic_division_by_zero_raw(
        &self,
        display: &str,
        token: &str,
    ) {
        eprintln!(
            "{}((: {display}: division by 0 (error token is \"{token}\")",
            self.diagnostic_prefix()
        );
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    pub(in crate::executor) fn report_let_arithmetic_error(&self, expression: &str) {
        self.report_arithmetic_error_with_label("let", expression, true);
    }

    pub(in crate::executor) fn report_conditional_arithmetic_error(
        &self,
        expression: &str,
    ) {
        // [[ ]] conditional context: GNU expr.c omits the trailing space
        // in the error token (e.g. "+" not "+ ").
        self.report_arithmetic_error_with_label("[[", expression, false);
    }

    pub(in crate::executor) fn execute_arithmetic_command(&mut self, cmd: &CommandNode) -> i32 {
        let raw_expression = cmd
            .arithmetic_command
            .as_ref()
            .and_then(|command| command.raw_expression.as_deref());
        let expression = cmd
            .arithmetic_command
            .as_ref()
            .map(|command| command.expression.as_str())
            .or_else(|| cmd.words.get(1).map(String::as_str))
            .unwrap_or_default();
        match self.eval_arithmetic_command_value(expression) {
            Some(0) => 1,
            Some(_) => 0,
            None => {
                // Raw-captured `(( ))` commands report division by 0 with
                // GNU's exact lasttp remainder; every other diagnostic
                // keeps the established normalized-expression path.
                let raw_division = raw_expression
                    .map(|raw| raw.trim_start_matches([' ', '\t']))
                    .and_then(|display| {
                        arithmetic_division_by_zero_token(display)
                            .map(|token| (display, token))
                    });
                match raw_division {
                    Some((display, token)) => {
                        self.report_arithmetic_division_by_zero_raw(&display, &token)
                    }
                    None => self.report_arithmetic_error(expression),
                }
                if self.arithmetic_nounset_error.get() {
                    127
                } else {
                    1
                }
            }
        }
    }

    pub(in crate::executor) fn execute_let(&mut self, expressions: &[String]) -> i32 {
        if expressions.is_empty() {
            eprintln!("{}let: expression expected", self.diagnostic_prefix());
            return 1;
        }

        let mut value = None;
        let mut index = 0;
        while index < expressions.len() {
            let mut expression = expressions[index].clone();
            if expression.contains(COMPOUND_ASSIGNMENT_MARKER)
                && expressions
                    .get(index + 1)
                    .is_some_and(|word| arithmetic_assignment_suffix(word))
            {
                expression.push_str(&expressions[index + 1]);
                index += 1;
            }
            let expression = arithmetic_expression_arg(&expression);
            value = self.eval_arithmetic_command_value(&expression);
            if value.is_none() {
                self.report_let_arithmetic_error(&expression);
                return 1;
            }
            index += 1;
        }
        match value {
            Some(0) | None => 1,
            Some(_) => 0,
        }
    }

    pub(crate) fn expand_aliases(&self, words: &[String]) -> Vec<String> {
        if !self.alias_expansion_enabled() {
            return words.to_vec();
        }

        let mut expanded = Vec::new();
        let mut expand_next = true;

        for word in words {
            if expand_next {
                let mut seen = Vec::new();
                let (mut alias_words, alias_expand_next) = self.expand_alias_word(word, &mut seen);
                if alias_words.is_empty() && !self.aliases.contains_key(word) {
                    expanded.push(word.clone());
                } else {
                    expanded.append(&mut alias_words);
                }
                expand_next = alias_expand_next;
            } else {
                expanded.push(word.clone());
                expand_next = false;
            }
        }

        expanded
    }

    /// Alias expansion that honours quote state: Bash never expands an alias
    /// whose word is quoted (`'hi'`, `"hi"` stay literal). `raws` carries the
    /// per-word raw text from word metadata so the caller can distinguish
    /// `hi` from `'hi'` after quote removal.
    pub(in crate::executor) fn expand_aliases_with_raw(
        &self,
        words: &[String],
        raws: &[Option<&str>],
    ) -> Vec<String> {
        if !self.alias_expansion_enabled() {
            return words.to_vec();
        }

        let mut expanded = Vec::new();
        let mut expand_next = true;

        for (index, word) in words.iter().enumerate() {
            let raw = raws.get(index).copied().flatten();
            if expand_next && !crate::executor::command_prepare::raw_word_is_quoted(raw) {
                let mut seen = Vec::new();
                let (mut alias_words, alias_expand_next) = self.expand_alias_word(word, &mut seen);
                if alias_words.is_empty() && !self.aliases.contains_key(word) {
                    expanded.push(word.clone());
                } else {
                    expanded.append(&mut alias_words);
                }
                expand_next = alias_expand_next;
            } else {
                expanded.push(word.clone());
                expand_next = false;
            }
        }

        expanded
    }

    pub(in crate::executor) fn expand_aliases_preserving_reserved(
        &self,
        words: &[String],
    ) -> Vec<String> {
        if !self.alias_expansion_enabled() {
            return words.to_vec();
        }

        // TODO(parse.y/alias.c): In POSIX mode Bash does not alias reserved
        // words. This keeps just enough parser-state awareness for alias7.sub.
        let mut expanded = Vec::new();
        let mut expand_next = true;

        for word in words {
            if expand_next && !is_reserved_word(word) {
                let mut seen = Vec::new();
                let (mut alias_words, alias_expand_next) = self.expand_alias_word(word, &mut seen);
                expanded.append(&mut alias_words);
                expand_next = alias_expand_next;
            } else {
                expanded.push(word.clone());
                expand_next = false;
            }
        }

        expanded
    }

    /// GNU parse.y:4529-4534 (parse_command_substitution): the substitution
    /// body is parsed with alias expansion active (posix mode forces it on;
    /// with `shopt expand_aliases` the net behavior still expands the body's
    /// command-word aliases). Bash implements this with alias_expand_token's
    /// push_string/RE_READ_TOKEN loop; splicing the alias value into the
    /// body text before the body parse gives the same token stream — the
    /// alias value becomes real parser words whose `$` expansions happen at
    /// execution (comsub21.sub: `echo ${ my_alias; }` prints $DATE's value,
    /// not the literal word).
    pub(in crate::executor) fn comsub_body_alias_splice(&self, source: &str) -> String {
        if !(self.alias_expansion_enabled() || self.posix_mode_enabled()) {
            return source.to_string();
        }
        let trimmed = source.trim_start_matches([' ', '\t', '\n']);
        let first_end = trimmed
            .find(|ch: char| ch.is_whitespace() || ch == ';' || ch == '\n')
            .unwrap_or(trimmed.len());
        if first_end == 0 {
            return source.to_string();
        }
        let first = &trimmed[..first_end];
        // Only a plain word can be an alias invocation; quoted or expanded
        // words never are (parse.y alias_expand_token).
        if first.contains('$')
            || first.contains('`')
            || first.contains('\\')
            || first.starts_with('\'')
            || first.starts_with('"')
        {
            return source.to_string();
        }
        let Some(alias) = self.aliases.get(first) else {
            return source.to_string();
        };
        // AL_BEINGEXPANDED: a body already expanding this alias does not
        // recurse (parse.y alias_expand_token cycle guard).
        if self.expanding_aliases.iter().any(|seen| seen == first) {
            return source.to_string();
        }
        let mut spliced = alias.value.replace('\x1f', "$");
        let rest = &trimmed[first_end..];
        if !rest.is_empty()
            && !spliced.ends_with(' ')
            && !spliced.ends_with('\t')
            && !spliced.ends_with('\n')
        {
            spliced.push(' ');
        }
        spliced.push_str(rest);
        spliced
    }

    pub(in crate::executor) fn execute_parser_level_alias(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if !self.alias_expansion_enabled() {
            return Ok(false);
        }

        // TODO(parse.y/alias.c): GNU Bash pushes alias text back into the
        // parser input stream (`alias_expand_token` + `push_string`). This
        // reparses complex alias values at command position so aliases that
        // introduce `;`, newlines, or redirections behave closer to Bash until
        // Rubash has a real parser input stack.
        let Some(word) = cmd.words.first() else {
            return Ok(false);
        };

        if self.expanding_aliases.iter().any(|alias| alias == word) {
            return Ok(false);
        }

        // Bash never expands a quoted word as an alias (`'hi'` / `"hi"`).
        let word_is_quoted = cmd
            .word_metadata
            .first()
            .map(|metadata| {
                crate::executor::command_prepare::raw_word_is_quoted(Some(&metadata.raw))
            })
            .unwrap_or(false);
        if word_is_quoted {
            return Ok(false);
        }

        let Some(alias) = self.aliases.get(word).cloned() else {
            return Ok(false);
        };

        if !needs_parser_level_alias_expansion(&alias.value) {
            return Ok(false);
        }

        let mut source = alias.value.replace('\x1f', "$");
        if !cmd.words[1..].is_empty()
            && (has_unclosed_quote(&alias.value)
                || (!source.ends_with(' ') && !source.ends_with('\t')))
        {
            source.push(' ');
        }
        source.push_str(&cmd.words[1..].join(" "));

        self.expanding_aliases.push(word.clone());
        let tokens = crate::lexer::tokenize_with_options(
            &source,
            crate::lexer::TokenizeOptions {
                input_origin: crate::lexer::InputOrigin::AliasReplacementDeferredHeredoc,
                ..Default::default()
            },
        );
        let ast = crate::parser::parse(&tokens);
        let result = self.execute_ast(&ast);
        self.expanding_aliases.pop();
        result.map(|_| true)
    }

    pub(in crate::executor) fn alias_parser_source(
        &self,
        word: &str,
        rest: &[String],
    ) -> Option<String> {
        let mut seen = Vec::new();
        let mut source = self.alias_parser_source_inner(word, rest, &mut seen)?;
        while let Some((first, remainder)) = split_first_shell_word(&source) {
            let remainder = remainder.to_string();
            if seen.iter().any(|seen_word| seen_word == &first) {
                break;
            }
            let Some(expanded) = self.alias_parser_source_inner(&first, &[], &mut seen) else {
                break;
            };
            source = expanded;
            if !remainder.is_empty() {
                if !source.ends_with(' ') && !source.ends_with('\t') && !source.ends_with('\n') {
                    source.push('\n');
                }
                source.push_str(&remainder);
            }
        }
        Some(source)
    }

    pub(in crate::executor) fn alias_parser_source_inner(
        &self,
        word: &str,
        rest: &[String],
        seen: &mut Vec<String>,
    ) -> Option<String> {
        if seen.iter().any(|seen_word| seen_word == word) {
            return None;
        }
        let alias = self.aliases.get(word)?;
        if !needs_parser_level_alias_expansion(&alias.value)
            && !matches!(alias.value.trim(), "if" | "then" | "elif" | "else" | "fi")
        {
            return None;
        }

        seen.push(word.to_string());
        let mut source = alias.value.replace('\x1f', "$");
        if !rest.is_empty()
            && (has_unclosed_quote(&alias.value)
                || (!source.ends_with(' ') && !source.ends_with('\t')))
        {
            source.push(' ');
        }
        source.push_str(&rest.join(" "));
        Some(source)
    }

    pub(in crate::executor) fn expand_alias_word(
        &self,
        word: &str,
        seen: &mut Vec<String>,
    ) -> (Vec<String>, bool) {
        // TODO(alias.c/alias.h/parse.y): Bash marks AL_BEINGEXPANDED in
        // parse.y::alias_expand_token and re-reads parser input. This executor-level
        // approximation preserves AL_EXPANDNEXT and recursion suppression, but it
        // cannot make redirections or compound commands introduced by aliases parse
        // exactly like GNU Bash yet.
        if seen.iter().any(|seen_word| seen_word == word) {
            return (vec![word.to_string()], false);
        }

        let Some(alias) = self.aliases.get(word) else {
            return (vec![word.to_string()], false);
        };

        // Bash expands aliases while reading a line. An alias defined earlier
        // on the same physical line is therefore not visible to later tokens
        // on that line; the executor otherwise sees the mutation immediately.
        if self.alias_defined_on_current_line(word)
            && !needs_parser_level_alias_expansion(&alias.value)
            && !alias_value_starts_reserved_word(&alias.value)
        {
            return (vec![word.to_string()], false);
        }

        if alias.value.is_empty() {
            return (Vec::new(), false);
        }

        seen.push(word.to_string());
        let mut parts: Vec<String> = alias.value.split_whitespace().map(str::to_string).collect();

        if let Some(first) = parts.first().cloned() {
            let (mut first_expanded, nested_expand_next) = self.expand_alias_word(&first, seen);
            parts.remove(0);
            first_expanded.extend(parts);
            // TODO(alias.c/parse.y): Bash preserves AL_EXPANDNEXT through
            // chained alias expansion. This approximates that propagation for
            // nested aliases like `a2=a1`, `a1='echo '`.
            (first_expanded, alias.expand_next || nested_expand_next)
        } else {
            (Vec::new(), alias.expand_next)
        }
    }

    fn alias_defined_on_current_line(&self, word: &str) -> bool {
        let Some(current_line) = self
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
        else {
            return false;
        };
        let key = format!("__RUBASH_ALIAS_LINE_{word}");
        self.env_vars
            .get(&key)
            .and_then(|line| line.parse::<usize>().ok())
            == Some(current_line)
    }
}

fn alias_value_starts_reserved_word(value: &str) -> bool {
    matches!(
        value.split_whitespace().next(),
        Some(
            "if" | "case"
                | "for"
                | "while"
                | "until"
                | "select"
                | "function"
                | "!"
                | "("
                | "{"
                | "then"
                | "elif"
                | "else"
                | "fi"
                | "do"
                | "done"
                | "in"
                | "esac"
                | "coproc",
        )
    )
}
