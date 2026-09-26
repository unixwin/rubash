use super::*;

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

        if !cmd.words.iter().any(|word| {
            matches!(word.as_str(), ";" | "<" | ">" | ">>" | "|" | "&")
                || word.contains('=')
                || word.contains("$(")
                || word.contains('`')
        }) {
            return Ok(false);
        }

        let source = cmd.words.join(" ");
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
