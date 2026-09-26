use super::*;
use crate::executor::markers::DATA_DOLLAR;

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
            let Some(alias) = self.shell_state.aliases.get(&token.value) else {
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
        // GNU expr.c evalerror longjmps only to the innermost evalexp
        // (expr.c:429-445). For `((`/`let`/`[[` that frame belongs to the
        // command itself — expok==0 makes it a status-1 continuation
        // (execute_cmd.c:3940+, let.def, test.c arithcomp), not a command
        // list abort; DISCARD is chosen only by array_expand_index /
        // make_variable_value / word-expansion callers.
        if let Some(record) = crate::executor::arithmetic::take_arith_eval_error() {
            eprintln!(
                "{}{}: {}",
                self.diagnostic_prefix(),
                label,
                record.render(true)
            );
        } else if let Some(token) = arithmetic_division_by_zero_token(expression) {
            eprintln!(
                "{}{}: {expression}: division by 0 (error token is \"{token}\")",
                self.diagnostic_prefix(),
                label
            );
        } else if let Some(token) = self.undefined_var_operand_token(expression) {
            // GNU expr.c: when $var expands to empty (undefined variable),
            // the arithmetic evaluator reports "operand expected" with the
            // original $var token (e.g. `jv += $iv` -> token "$iv").
            eprintln!(
                "{}{}: {expression}: arithmetic syntax error: operand expected (error token is \"{token}\")",
                self.diagnostic_prefix(),
                label
            );
        } else if let Some(message) = crate::executor::arithmetic::arithmetic_command_error_message(
            expression,
            trailing_space,
        ) {
            eprintln!("{}{}: {message}", self.diagnostic_prefix(), label);
        }
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    /// Detect a $var reference in the expression that would expand to empty
    /// because the variable is undefined (not in env_vars or shell variables).
    /// Returns the $var token as it appears in the expression.
    fn undefined_var_operand_token(&self, expression: &str) -> Option<String> {
        let bytes = expression.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0x24 && index + 1 < bytes.len() {
                let start = index;
                index += 1;
                let braced = bytes[index] == 0x7b;
                if braced {
                    index += 1;
                }
                let name_start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == 0x5f)
                {
                    index += 1;
                }
                if index > name_start {
                    let name = &expression[name_start..index];
                    if braced && index < bytes.len() && bytes[index] == 0x7d {
                        index += 1;
                    }
                    let end = index;
                    let defined = self.dynamic_parameter_value(name).is_some()
                        || self.shell_variable_value(name).is_some()
                        || std::env::var(name).is_ok();
                    if !defined {
                        return Some(expression[start..end].to_string());
                    }
                }
            } else {
                index += 1;
            }
        }
        None
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
    /// GNU arrayfunc.c:1353-1391 array_expand_index runs the subscript
    /// through a nested `evalexp`, so when the subscript itself is the
    /// failure, evalerror names the SUBSCRIPT text — no `((`/`let`/`[[`
    /// label prefix (`matrix: a[$x]` prints `$(...): arithmetic syntax
    /// error`, not `((: a[$(...)]`). The evaluator records that text in
    /// __RUBASH_ARITH_SUBSCRIPT_EXPR (lvalue.rs / value.rs).
    fn report_subscript_eval_failure(&self) -> bool {
        let Some(subscript) = self
            .shell_state
            .env_vars
            .get("__RUBASH_ARITH_SUBSCRIPT_EXPR")
        else {
            return false;
        };
        let subscript = subscript.clone();
        self.report_indexed_subscript_error(&subscript);
        true
    }

    pub(in crate::executor) fn report_arithmetic_error_raw_display(&self, raw_display: &str) {
        if self.report_subscript_eval_failure() {
            return;
        }
        // Nonfatal: `((` and arithmetic-for sections return status 1 and
        // continue (the evalerror longjmp only reaches the command's own
        // evalexp frame).
        let record_opt = crate::executor::arithmetic::take_arith_eval_error();
        if let Some(record) = record_opt {
            let rendered = record.render(true);
            eprintln!("{}((: {}", self.diagnostic_prefix(), rendered);
            use std::io::Write;
            let _ = std::io::stderr().flush();
            return;
        }
        let display = raw_display.trim_start_matches([' ', '\t']);
        if let Some(message) =
            crate::executor::arithmetic::arithmetic_command_error_message(display, false)
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
        self.raise_evalerror_abort();
        eprintln!(
            "{}((: {display}: division by 0 (error token is \"{token}\")",
            self.diagnostic_prefix()
        );
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    pub(in crate::executor) fn report_let_arithmetic_error(&self, expression: &str) {
        // GNU expr.c: a failed array SUBSCRIPT eval (nested evalexp inside
        // array_expand_index) names the subscript, not the let operand.
        if self.report_subscript_eval_failure() {
            return;
        }
        // GNU let.def: let_builtin passes EXP_EXPANDED to evalexp, so the
        // arithmetic evaluator does NOT expand $var. In GNU expr.c,
        // legal_variable_starter(c) is ISALPHA(c) || (c == '_'), so '$' is
        // not a valid operand character. When the evaluator sees '$var'
        // after an operator, it reports "operand expected" with the '$var'
        // token. The recorded evalerror carries that token already; the
        // text scan is the fallback for paths that never reached the parser.
        if crate::executor::arithmetic::peek_arith_eval_error().is_none() {
            if let Some(token) = dollar_var_operand_token(expression) {
                eprintln!(
                    "{}let: {expression}: arithmetic syntax error: operand expected (error token is \"{token}\")",
                    self.diagnostic_prefix()
                );
                use std::io::Write;
                let _ = std::io::stderr().flush();
                return;
            }
        }
        self.report_arithmetic_error_with_label("let", expression, true);
    }

    // GNU expr.c assignments route through bind_variable; an invalid
    // nameref-cell value fails via sh_invalidid with this_command_name as
    // the label (`((`, `let`), or no label inside $(( )) expansion.
    pub(in crate::executor) fn report_arithmetic_nameref_error(
        &mut self,
        label: Option<&str>,
    ) -> bool {
        let Some(value) = self
            .shell_state
            .env_vars
            .remove("__RUBASH_ARITH_NAMEREF_ERROR")
        else {
            return false;
        };
        match label {
            Some(label) => eprintln!(
                "{}{label}: `{value}': not a valid identifier",
                self.diagnostic_prefix()
            ),
            None => eprintln!(
                "{}`{value}': not a valid identifier",
                self.diagnostic_prefix()
            ),
        }
        use std::io::Write;
        let _ = std::io::stderr().flush();
        true
    }

    pub(in crate::executor) fn report_conditional_arithmetic_error(&self, expression: &str) {
        if self.report_subscript_eval_failure() {
            return;
        }
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
        // GNU execute_cmd.c:3940-3945: if `set -x` is on, print `(( expr ))`
        // before evaluating the arithmetic command.  Use the raw expression to
        // preserve original whitespace.
        let xtrace_expr = raw_expression.unwrap_or(expression);
        self.xtrace_print_arith_cmd(xtrace_expr);
        // GNU parse.y parse_arith_cmd captures the text between `((` and `))`
        // verbatim (whitespace included) and execute_arith_command hands it to
        // evalexp after expand_arith_string. The parser's joined `expression`
        // loses that whitespace (`x ++ = 7 ` keeps the blank before `))` in
        // every GNU diagnostic), so the raw slice is the faithful eval input.
        let eval_result = self.eval_arithmetic_command_value(raw_expression.unwrap_or(expression));
        // An invalid nameref-cell assignment inside `((` fails the command
        // with status 1 even though the expression itself evaluated.
        if self.report_arithmetic_nameref_error(Some("((")) {
            return 1;
        }
        match eval_result {
            Some(0) => 1,
            Some(_) => 0,
            None => {
                // GNU expr.c:1528 evalerror echoes the expression with only
                // leading whitespace skipped, and every error token is the
                // raw lasttp remainder to end-of-input (trailing blanks
                // included). execute_arith_command (execute_cmd.c:3937)
                // expands the string before evalexp, so the diagnostic text
                // is the post-expansion expression — `(( 4 ? : $A ))` echoes
                // `4 ? : 7 `. arith_display_expand applies the parameter part
                // of expand_arith_string to the raw capture; the parser's
                // joined `expression` field (which loses token adjacency like
                // `<=`) and literal `$var` text are both wrong here. The
                // expanded evaluator input is the fallback when no raw
                // capture exists, normalized words the last resort.
                if let Some(raw) = raw_expression {
                    let display = self.arith_display_expand(raw);
                    self.report_arithmetic_error_raw_display(&display);
                } else {
                    let eval_input = self.arithmetic_last_eval_input.borrow().clone();
                    if !eval_input.is_empty() {
                        self.report_arithmetic_error_raw_display(&eval_input);
                    } else {
                        self.report_arithmetic_error(expression);
                    }
                }
                if self.shell_state.arithmetic_nounset_error.get() {
                    127
                } else {
                    1
                }
            }
        }
    }

    /// Parameter-level expansion of an arithmetic expression for error display.
    /// GNU's execute_arith_command (execute_cmd.c:3937) runs the raw expression
    /// through expand_arith_string(Q_DOUBLE_QUOTES|Q_ARITH) before evalexp, so a
    /// diagnostic like `(( 4 ? : $A ))` echoes the expanded text (`4 ? : 7 `).
    /// Only `$name`, `${name}`, positional, and special parameters are expanded
    /// here — `$(...)` command substitution and `${name:-...}` operators are left
    /// literal so nothing executes twice (the evaluation already expanded them).
    fn arith_display_expand(&self, expression: &str) -> String {
        if !expression.contains('$') {
            return expression.to_string();
        }
        let mut output = String::with_capacity(expression.len());
        let mut chars = expression.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '[' {
                // GNU subst.c:11107 expand_array_subscript (reached only
                // under Q_ARITH): the subscript is expanded once and every
                // expansion product byte that could restart an expansion or
                // delimit a subscript is backslash-quoted (abstab:
                // `[` `]` `$` `` ` `` `~` `\` `'` `"`). evalexp's error
                // token then echoes the escaped text — `assoc[x\],b\[
                // \$(...)]++`, not the bare product.
                let mut inner = String::new();
                let mut closed = false;
                for inner_ch in chars.by_ref() {
                    if inner_ch == ']' {
                        closed = true;
                        break;
                    }
                    inner.push(inner_ch);
                }
                if !closed {
                    // No terminator: skipsubscript fails and the `[` stays
                    // plain text in evalexp's input (subst.c:11133-11141).
                    output.push('[');
                    output.push_str(&inner);
                    continue;
                }
                let expanded_inner = self.arith_display_expand(&inner);
                output.push('[');
                for inner_ch in expanded_inner.chars() {
                    if matches!(inner_ch, '[' | ']' | '$' | '`' | '~' | '\\' | '\'' | '"') {
                        output.push('\\');
                    }
                    output.push(inner_ch);
                }
                output.push(']');
                continue;
            }
            if ch != '$' {
                output.push(ch);
                continue;
            }
            let lookup = |name: &str| -> Option<String> {
                self.dynamic_parameter_value(name)
                    .or_else(|| self.shell_variable_value(name))
                    .or_else(|| std::env::var(name).ok())
            };
            match chars.peek().copied() {
                Some('{') => {
                    chars.next();
                    let name = collect_braced_parameter_name(&mut chars);
                    // Only a plain `${name}` is expanded for display; operators
                    // like `${x:-word}` stay literal (rare inside failing arith).
                    if is_shell_name(&name) {
                        if let Some(value) = lookup(&name) {
                            output.push_str(&value);
                        }
                    } else {
                        output.push_str("${");
                        output.push_str(&name);
                        output.push('}');
                    }
                }
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
                Some('@') | Some('*') => {
                    chars.next();
                    output.push_str(&self.shell_state.positional_params.join(" "));
                }
                Some('#') => {
                    chars.next();
                    output.push_str(&self.shell_state.positional_params.len().to_string());
                }
                Some('-') => {
                    chars.next();
                    output.push_str(&self.shell_option_flags());
                }
                Some(first) if first.is_ascii_digit() => {
                    chars.next();
                    let index = first.to_digit(10).unwrap_or(0) as usize;
                    if index == 0 {
                        output.push_str(&self.script_name_value());
                    } else {
                        output.push_str(
                            self.shell_state
                                .positional_params
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
                    if let Some(value) = lookup(&name) {
                        output.push_str(&value);
                    }
                }
                Some(_) => output.push('$'),
                None => output.push('$'),
            }
        }
        output
    }

    pub(in crate::executor) fn execute_let(&mut self, expressions: &[String]) -> i32 {
        // GNU let.def:85: a leading `--` argument is skipped (ISOPTION),
        // so `let -- 'expr'` evaluates expr, not `--` as an operand.
        let expressions = if expressions.first().map(String::as_str) == Some("--") {
            &expressions[1..]
        } else {
            expressions
        };
        if expressions.is_empty() {
            eprintln!("{}let: expression expected", self.diagnostic_prefix());
            return 1;
        }

        let mut value = None;
        let mut index = 0;
        while index < expressions.len() {
            // W_ARRAYREF (in-band ARRAYREF_FLAG) is a no-op for let — GNU
            // marks `let a[k]` operands but let.def never consults it.
            let mut expression = crate::builtins::arrayref::take_arrayref_flag(&expressions[index])
                .1
                .to_string();
            if expression.contains(COMPOUND_ASSIGNMENT_MARKER)
                && expressions
                    .get(index + 1)
                    .is_some_and(|word| arithmetic_assignment_suffix(word))
            {
                expression.push_str(&expressions[index + 1]);
                index += 1;
            }
            let expression = arithmetic_expression_arg(&expression);
            // GNU let.def:102 evalexp(arg, EXP_EXPANDED): the operand was
            // word-expanded once already — top-level `$name`/`$(...)` is
            // readtok junk ("operand expected"), never re-expanded. Indexed
            // subscripts still expand inside array_expand_index unless
            // `shopt -s array_expand_once` (expr.c:1171 AV_NOEXPAND);
            // eval handles that internally.
            value = self.eval_arithmetic_command_value_no_expand(&expression);
            if value.is_none() {
                self.report_let_arithmetic_error(&expression);
                return 1;
            }
            if self.report_arithmetic_nameref_error(Some("let")) {
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
        if !self.alias_expansion_enabled() || self.alias_streamed() {
            return words.to_vec();
        }

        let mut expanded = Vec::new();
        let mut expand_next = true;

        for word in words {
            if expand_next {
                let mut seen = Vec::new();
                let (mut alias_words, alias_expand_next) = self.expand_alias_word(word, &mut seen);
                if alias_words.is_empty() && !self.shell_state.aliases.contains_key(word) {
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
        if !self.alias_expansion_enabled() || self.alias_streamed() {
            return words.to_vec();
        }

        let mut expanded = Vec::new();
        let mut expand_next = true;

        for (index, word) in words.iter().enumerate() {
            let raw = raws.get(index).copied().flatten();
            if expand_next && !crate::executor::command_prepare::raw_word_is_quoted(raw) {
                let mut seen = Vec::new();
                let (mut alias_words, alias_expand_next) = self.expand_alias_word(word, &mut seen);
                if alias_words.is_empty() && !self.shell_state.aliases.contains_key(word) {
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
        if !self.alias_expansion_enabled() || self.alias_streamed() {
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

    /// GNU parse.y alias_expand_token + push_string over a fresh parser
    /// input (command-substitution body, eval string, trap action): the
    /// whole input is re-read as one token stream, so command-position
    /// words expand against the alias table live NOW — an alias defined
    /// inside the input is not yet visible to it (GNU parses the whole
    /// string before executing: `eval 'alias echo="echo a"; echo b'`
    /// prints `b`, `$(alias echo="echo a"; echo b)` captures `b`).
    ///
    /// This replaces the earlier first-word-only splice: a body like
    /// `x | y` expands both command positions, a trailing-blank alias
    /// chains through AL_EXPANDNEXT, and a self-referencing alias does
    /// not re-expand inside its own pushed text (AL_BEINGEXPANDED), all
    /// handled by lexer::expand_aliases_in_source. The executor-level
    /// word expanders are suppressed for the parsed result via
    /// __RUBASH_ALIAS_STREAMED so nothing expands a second time.
    pub(in crate::executor) fn comsub_body_alias_splice(&self, source: &str) -> String {
        if !(self.alias_expansion_enabled() || self.posix_mode_enabled())
            || self.shell_state.aliases.is_empty()
        {
            return source.to_string();
        }
        let lookup = |word: &str| {
            self.shell_state
                .aliases
                .get(word)
                .map(|alias| (alias.value.replace(DATA_DOLLAR, "$"), alias.expand_next))
        };
        crate::lexer::expand_aliases_in_source(source, &lookup as &crate::lexer::AliasLookup<'_>)
    }

    /// `comsub_body_alias_splice` for a body extracted from input the
    /// grouped driver already ran through expand_aliases_in_source
    /// (__RUBASH_ALIAS_STREAMED): its `$(` bodies were spliced there, so a
    /// second pass would fire self-referential aliases again (`let` →
    /// `let --` → `let -- --`; AL_BEINGEXPANDED, parse.y:3259). Eval
    /// strings and trap actions are fresh input streams — their callers
    /// use `comsub_body_alias_splice` directly.
    pub(in crate::executor) fn comsub_body_alias_splice_extracted(&self, source: &str) -> String {
        if self.alias_streamed() {
            source.to_string()
        } else {
            self.comsub_body_alias_splice(source)
        }
    }

    pub(in crate::executor) fn execute_parser_level_alias(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if !self.alias_expansion_enabled() || self.alias_streamed() {
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

        if self
            .shell_state
            .expanding_aliases
            .iter()
            .any(|alias| alias == word)
        {
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

        let Some(alias) = self.shell_state.aliases.get(word).cloned() else {
            return Ok(false);
        };

        if !needs_parser_level_alias_expansion(&alias.value) {
            return Ok(false);
        }

        let mut source = alias.value.replace(DATA_DOLLAR, "$");
        if !cmd.words[1..].is_empty()
            && (has_unclosed_quote(&alias.value)
                || (!source.ends_with(' ') && !source.ends_with('\t')))
        {
            source.push(' ');
        }
        source.push_str(&cmd.words[1..].join(" "));

        self.shell_state.expanding_aliases.push(word.clone());
        let tokens = crate::lexer::tokenize_with_options(
            &source,
            crate::lexer::TokenizeOptions {
                input_origin: crate::lexer::InputOrigin::AliasReplacementDeferredHeredoc,
                ..Default::default()
            },
        );
        let ast = crate::parser::parse(&tokens);
        let result = self.execute_ast(&ast);
        self.shell_state.expanding_aliases.pop();
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
        let alias = self.shell_state.aliases.get(word)?;
        if !needs_parser_level_alias_expansion(&alias.value)
            && !matches!(alias.value.trim(), "if" | "then" | "elif" | "else" | "fi")
        {
            return None;
        }

        seen.push(word.to_string());
        let mut source = alias.value.replace(DATA_DOLLAR, "$");
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

        let Some(alias) = self.shell_state.aliases.get(word) else {
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
        // GNU parse.y: alias value is pushed back into the parser input and
        // re-tokenized. split_whitespace() would break command substitutions
        // (`echo $(echo $DATE)` → `["echo", "$(echo", "$DATE)"]`); use the
        // shell word splitter that respects `$(...)`, backticks, and quotes
        // (comsub6.sub: `alias foo='echo $(echo $DATE)'` → `foo` prints the
        // date, not `$(echo $DATE)`). Alias values store `$` as \x1f, so
        // restore it before splitting.
        let alias_value = alias.value.replace(DATA_DOLLAR, "$");
        let mut parts = super::alias_helpers::split_shell_words(&alias_value);

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
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
        else {
            return false;
        };
        let key = format!("__RUBASH_ALIAS_LINE_{word}");
        self.shell_state
            .env_vars
            .get(&key)
            .and_then(|line| line.parse::<usize>().ok())
            == Some(current_line)
    }
}

/// Detect ANY $var reference in the expression (defined or undefined).
/// Used for let context where GNU expr.c does not expand $var and treats
/// '$' as an invalid operand character, reporting "operand expected".
fn dollar_var_operand_token(expression: &str) -> Option<String> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x24 && index + 1 < bytes.len() {
            let start = index;
            index += 1;
            let braced = bytes[index] == 0x7b;
            if braced {
                index += 1;
            }
            let name_start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == 0x5f)
            {
                index += 1;
            }
            if index > name_start {
                if braced && index < bytes.len() && bytes[index] == 0x7d {
                    index += 1;
                }
                // GNU expr.c::readtok skips trailing whitespace after the
                // token, and lasttp points at the start of the token. The
                // error token includes trailing whitespace (e.g. "$iv " for
                // `jv += $iv `). Include trailing spaces/tabs.
                while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t') {
                    index += 1;
                }
                let end = index;
                return Some(expression[start..end].to_string());
            }
        } else {
            index += 1;
        }
    }
    None
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
