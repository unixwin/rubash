use super::*;

impl Executor {
    pub(in crate::executor) fn execute_alias_introduced_case(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // TODO(parse.y/alias.c/execute_cmd.c): Same parser-stream issue as the
        // alias-introduced `for` path, narrowed to single-line `case` forms in
        // alias7.sub.
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let words = self.expand_aliases(&command.words);
        if words.first().map(String::as_str) != Some("case") {
            return Ok(None);
        }

        let source = words.join(" ");
        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if let Some(case_command) = reparsed
            .commands
            .first()
            .and_then(|command| command.case_command.as_ref())
        {
            self.execute_case_command_with_redirects(command, case_command)?;
            return Ok(Some(index + 1));
        }

        if let Some(case_command) = case_command_from_words(&words) {
            self.execute_case_command_with_redirects(command, &case_command)?;
            return Ok(Some(index + 1));
        }

        Ok(None)
    }

    pub(in crate::executor) fn execute_alias_heredoc(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        if !self.alias_expansion_enabled() {
            return Ok(None);
        }

        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let Some(first_word) = command.words.first() else {
            return Ok(None);
        };
        if !self.aliases.contains_key(first_word) {
            return Ok(None);
        }

        let Some(mut source) = self.alias_parser_source(first_word, &command.words[1..]) else {
            return Ok(None);
        };
        if !source.contains("<<") {
            return Ok(None);
        }

        let mut next_index = index + 1;
        while let Some(delimiter) = pending_heredoc_delimiter(&source) {
            let Some(next_command) = ast.commands.get(next_index) else {
                break;
            };
            let line = command_node_source_line(next_command);
            source.push('\n');
            source.push_str(&line);
            next_index += 1;
            if heredoc_delimiter_line_matches(&line, &delimiter, false) {
                break;
            }
        }

        self.expanding_aliases.push(first_word.clone());
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        let result = self.execute_ast(&ast);
        self.expanding_aliases.pop();
        result?;
        Ok(Some(next_index))
    }
}
