use super::*;

impl Executor {
    pub(in crate::executor) fn execute_arithmetic_command(&mut self, cmd: &CommandNode) -> i32 {
        let expression = cmd.words.get(1).map(String::as_str).unwrap_or_default();
        match self.eval_arithmetic_command_value(expression) {
            Some(0) => 1,
            Some(_) => 0,
            None => {
                if let Some(token) = arithmetic_division_by_zero_token(expression) {
                    eprintln!(
                        "{}((: {expression} : division by 0 (error token is \"{token}\")",
                        self.diagnostic_prefix()
                    );
                }
                1
            }
        }
    }

    pub(in crate::executor) fn execute_let(&mut self, expressions: &[String]) -> i32 {
        if expressions.is_empty() {
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
                return 1;
            }
            index += 1;
        }
        match value {
            Some(0) | None => 1,
            Some(_) => 0,
        }
    }

    pub(in crate::executor) fn expand_aliases(&self, words: &[String]) -> Vec<String> {
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

    pub(in crate::executor) fn expand_aliases_preserving_reserved(
        &self,
        words: &[String],
    ) -> Vec<String> {
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

    pub(in crate::executor) fn execute_parser_level_alias(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
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
        let tokens = crate::lexer::tokenize(&source);
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
        if !needs_parser_level_alias_expansion(&alias.value) {
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
}
