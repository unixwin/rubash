use super::alias_case::*;
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

        if let Some((case_command, redirect_command, next_index)) =
            self.alias_case_command_from_ast(ast, index, command, &words)
        {
            self.execute_case_command_with_redirects(redirect_command, &case_command)?;
            return Ok(Some(next_index));
        }

        let (source, redirect_command, next_index) =
            self.alias_case_source(ast, index, command, &words);
        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if let Some(case_command) = reparsed
            .commands
            .first()
            .and_then(|command| command.case_command.as_ref())
        {
            self.execute_case_command_with_redirects(redirect_command, case_command)?;
            return Ok(Some(next_index));
        }

        if let Some(case_command) = case_command_from_words(&words) {
            self.execute_case_command_with_redirects(redirect_command, &case_command)?;
            return Ok(Some(next_index));
        }

        Ok(None)
    }

    fn alias_case_command_from_ast<'a>(
        &self,
        ast: &'a Ast,
        index: usize,
        command: &'a CommandNode,
        words: &'a [String],
    ) -> Option<(CaseCommand, &'a CommandNode, usize)> {
        let word = words.get(1)?.clone();
        let mut header_index = 2;
        while header_index < words.len() && words[header_index] != "in" {
            header_index += 1;
        }
        if header_index >= words.len() {
            return None;
        }
        let mut clauses = Vec::new();

        let mut pattern_index = header_index + 1;
        let mut current_command = command;
        let mut current_words = words;
        let mut current_command_index = index;
        let (redirect_command, next_index) = loop {
            let patterns = collect_alias_case_patterns(
                ast,
                current_command,
                current_command_index,
                current_words,
                pattern_index,
            )?;
            let mut body = Vec::new();
            let boundary = collect_alias_case_body(
                ast,
                patterns.command,
                patterns.command_index,
                patterns.words,
                patterns.body_start,
                &mut body,
            )?;
            let clause_index = clauses.len();
            let pattern_nodes = patterns
                .patterns
                .iter()
                .enumerate()
                .map(|(pattern_index, pattern)| {
                    crate::parser::CasePattern::new(pattern.clone(), clause_index, pattern_index)
                })
                .collect();
            clauses.push(CaseClause {
                patterns: patterns.patterns,
                pattern_nodes,
                body,
                terminator: boundary.terminator,
            });
            if boundary.ended_case {
                break (boundary.command, boundary.command_index + 1);
            }
            let same_command = boundary.command_index == current_command_index;
            current_command = boundary.command;
            if !same_command {
                current_words = &boundary.command.words;
            }
            current_command_index = boundary.command_index;
            pattern_index = boundary.next_word_index;
        };

        Some((CaseCommand { word, clauses }, redirect_command, next_index))
    }

    fn alias_case_source<'a>(
        &self,
        ast: &'a Ast,
        index: usize,
        command: &'a CommandNode,
        words: &[String],
    ) -> (String, &'a CommandNode, usize) {
        let mut source = words.join(" ");
        let mut redirect_command = command;
        let mut next_index = index + 1;

        for command_index in index + 1..ast.commands.len() {
            let Some(next_command) = ast.commands.get(command_index) else {
                break;
            };
            let mut command_source = bash_command_text(next_command);
            if command_contains_word(next_command, "esac") {
                command_source = command_words_text(next_command);
                redirect_command = next_command;
                next_index = command_index + 1;
                if !command_source.is_empty() {
                    source.push_str("; ");
                    source.push_str(&command_source);
                }
                break;
            }

            if !command_source.is_empty() {
                source.push_str("; ");
                source.push_str(&command_source);
            }
            next_index = command_index + 1;
        }

        (source, redirect_command, next_index)
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
