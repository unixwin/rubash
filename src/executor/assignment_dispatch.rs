use super::*;

/// Quote one already-expanded word for execute_alias_expanded_syntax's
/// re-parse so its content is re-read as a single DATA word: single-quote
/// wrapping with the standard `'\''` escape keeps spaces, `=', operators
/// and substitution markers inert while tokenize re-splits the line.
fn quote_reparse_word(word: &str) -> String {
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('\'');
    for ch in word.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

impl Executor {
    pub(in crate::executor) fn execute_alias_expanded_syntax(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        // TODO(parse.y/alias.c/redir.c): Bash pushes alias replacement text
        // back into the parser, so `;`, redirections, and reserved words
        // introduced by chained aliases regain their syntactic meaning. This
        // reparses the already-expanded word list for the alias7.sub cases.
        const ALIAS_SYNTAX_REPARSE: &str = "__rubash_alias_syntax_reparse";
        if self
            .shell_state
            .expanding_aliases
            .iter()
            .any(|alias| alias == ALIAS_SYNTAX_REPARSE)
        {
            return Ok(false);
        }

        // parse.y push_string: only the words emitted by alias VALUES are
        // alias-introduced syntax. The words AFTER them are original input
        // whose post-expansion content is DATA — a plain argument that
        // happens to contain `=' (modernish tst/run.sh `let "opt_e = opt_q
        // = ..."' under `alias let='let --'') must not re-tokenize as an
        // assignment or split on its spaces. Scan only the introduced
        // prefix.
        let introduced = self.alias_introduced_words.get();
        let syntax_in_prefix = cmd.words.iter().take(introduced).any(|word| {
            matches!(word.as_str(), ";" | "<" | ">" | ">>" | "|" | "&")
                || word.contains('=')
                || word.contains("$(")
                || word.contains('`')
        });
        if !syntax_in_prefix {
            return Ok(false);
        }

        // Rebuild the source with the tail words shell-quoted so the
        // re-tokenizer keeps each one a single data word, then suppress
        // alias expansion for the re-parse (the alias values already
        // expanded; re-running the word expander fired them a second time:
        // `let -- ...' became `let -- -- ...').
        let mut source = String::new();
        for (index, word) in cmd.words.iter().enumerate() {
            if index > 0 {
                source.push(' ');
            }
            if index < introduced {
                source.push_str(word);
            } else {
                source.push_str(&quote_reparse_word(word));
            }
        }
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        self.shell_state
            .expanding_aliases
            .push(ALIAS_SYNTAX_REPARSE.to_string());
        let result = self.execute_ast(&ast);
        self.shell_state.expanding_aliases.pop();
        result?;
        Ok(true)
    }

    #[allow(unreachable_code)]
    pub(in crate::executor) fn execute_assignment_words(&mut self, cmd: &CommandNode) -> bool {
        // TODO(variables.c/arrayfunc.c/subst.c): Bash recognizes assignment
        // words after alias expansion and routes compound array assignments
        // through `assign_array_var_from_string`. This only handles commands
        // made entirely of `name=value` words.
        if cmd.words.is_empty() || !cmd.assignments.is_empty() {
            return false;
        }
        // GNU decides assignment words at parse time (parse.y read_token_word
        // flags W_ASSIGNMENT via general.c assignment()); execute_cmd.c never
        // re-derives them from word text. Every word reaching here was kept
        // a command word by the parser on purpose (a''=b runs the command
        // a=b: not found, rc 127), so re-deriving assignments from
        // de-quoted values would silently swallow them. The historical
        // promotion below is unreachable by design.
        return false;

        // GNU performs each assignment expansion and binding left-to-right,
        // so a later assignment sees the earlier one (var4.tests:
        // X=usbdev1.2 X=\${X#usbdev} B=\${X%%.*} D=\${X#*.} -> bus/usb/1/2).
        let mut command_substitution_status = None;
        let mut apply_failed = false;
        for word in &cmd.words {
            let Some((name, value)) = split_assignment_word(word) else {
                return false;
            };
            let (expanded_value, status) = self.expand_assignment_value_with_status(&name, value);
            if status.is_some() {
                command_substitution_status = status;
            }
            if !self.apply_shell_assignment(&name, expanded_value) {
                apply_failed = true;
            }
        }

        let mut status = command_substitution_status.unwrap_or(0);
        if apply_failed {
            status = 1;
        }
        self.exit_code = status;
        true
    }

    pub(in crate::executor) fn execute_integer_assignment_suffix(
        &mut self,
        cmd: &CommandNode,
    ) -> bool {
        if cmd.assignments.len() != 1 || cmd.words.len() != 1 {
            return false;
        }
        let Some(suffix) = cmd
            .words
            .first()
            .filter(|word| arithmetic_assignment_suffix(word))
        else {
            return false;
        };
        let Some((name, value)) = cmd.assignments.iter().next() else {
            return false;
        };
        let (base_name, _) = assignment_name_and_append(name);
        if !is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name)
            || !value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
        {
            return false;
        }

        let mut value = value.clone();
        value.push_str(suffix);
        let expanded_value = self.expand_assignment_value(name, &value);
        self.exit_code = if self.apply_shell_assignment(name, expanded_value) {
            0
        } else {
            1
        };
        true
    }
}
