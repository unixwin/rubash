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
            .expanding_aliases
            .iter()
            .any(|alias| alias == ALIAS_SYNTAX_REPARSE)
        {
            return Ok(false);
        }

        if !cmd
            .words
            .iter()
            .any(|word| matches!(word.as_str(), ";" | "<" | ">" | ">>" | "|" | "&"))
        {
            return Ok(false);
        }

        let source = cmd.words.join(" ");
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        self.expanding_aliases
            .push(ALIAS_SYNTAX_REPARSE.to_string());
        let result = self.execute_ast(&ast);
        self.expanding_aliases.pop();
        result?;
        Ok(true)
    }

    pub(in crate::executor) fn execute_assignment_words(&mut self, cmd: &CommandNode) -> bool {
        // TODO(variables.c/arrayfunc.c/subst.c): Bash recognizes assignment
        // words after alias expansion and routes compound array assignments
        // through `assign_array_var_from_string`. This only handles commands
        // made entirely of `name=value` words.
        if cmd.words.is_empty() || !cmd.assignments.is_empty() {
            return false;
        }

        let mut assignments = Vec::new();
        let mut command_substitution_status = None;
        for word in &cmd.words {
            let Some((name, value)) = split_assignment_word(word) else {
                return false;
            };
            let (expanded_value, status) = self.expand_assignment_value_with_status(value);
            if status.is_some() {
                command_substitution_status = status;
            }
            assignments.push((name.to_string(), expanded_value));
        }

        let mut status = command_substitution_status.unwrap_or(0);
        for (name, value) in assignments {
            if !self.apply_shell_assignment(&name, value) {
                status = 1;
            }
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
        if !is_marked_var(&self.env_vars, INTEGER_VARS, base_name)
            || !value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
        {
            return false;
        }

        let mut value = value.clone();
        value.push_str(suffix);
        let expanded_value = self.expand_assignment_value(&value);
        self.exit_code = if self.apply_shell_assignment(name, expanded_value) {
            0
        } else {
            1
        };
        true
    }
}
