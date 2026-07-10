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
        words: &[String],
    ) -> Option<(CaseCommand, &'a CommandNode, usize)> {
        let word = words.get(1)?.clone();
        let mut header_index = 2;
        while header_index < words.len() && words[header_index] != "in" {
            header_index += 1;
        }
        if header_index >= words.len() {
            return None;
        }
        let pattern = words.get(header_index + 1)?.clone();

        let mut body = Vec::new();
        let first_body_words = words.get(header_index + 2..).unwrap_or_default();
        if let Some(boundary) = case_boundary_index_in_words(first_body_words) {
            push_case_body_words(command, &first_body_words[..boundary], &mut body);
            return Some((
                CaseCommand {
                    word,
                    clauses: vec![CaseClause {
                        patterns: vec![pattern],
                        body,
                        terminator: CaseTerminator::Break,
                    }],
                },
                command,
                index + 1,
            ));
        }
        push_case_body_words(command, first_body_words, &mut body);

        let mut redirect_command = command;
        let mut next_index = index + 1;
        for command_index in index + 1..ast.commands.len() {
            let next_command = ast.commands.get(command_index)?;
            if let Some(boundary) = case_boundary_word_index(next_command) {
                push_case_body_words(next_command, &next_command.words[..boundary], &mut body);
                redirect_command = next_command;
                next_index = command_index + 1;
                break;
            }
            body.push(next_command.clone());
            next_index = command_index + 1;
        }

        Some((
            CaseCommand {
                word,
                clauses: vec![CaseClause {
                    patterns: vec![pattern],
                    body,
                    terminator: CaseTerminator::Break,
                }],
            },
            redirect_command,
            next_index,
        ))
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

fn command_contains_word(command: &CommandNode, word: &str) -> bool {
    command.words.iter().any(|candidate| candidate == word)
}

fn command_words_text(command: &CommandNode) -> String {
    command.words.join(" ")
}

fn case_boundary_word_index(command: &CommandNode) -> Option<usize> {
    case_boundary_index_in_words(&command.words)
}

fn case_boundary_index_in_words(words: &[String]) -> Option<usize> {
    words
        .iter()
        .position(|word| matches!(word.as_str(), ";;" | ";&" | ";;&" | "esac"))
}

fn push_case_body_words(command: &CommandNode, words: &[String], body: &mut Vec<CommandNode>) {
    if words.is_empty() {
        return;
    }
    let mut body_command = command.clone();
    body_command.words = words.to_vec();
    body_command.word_kinds = Vec::new();
    clear_command_redirects(&mut body_command);
    body.push(body_command);
}

fn clear_command_redirects(command: &mut CommandNode) {
    command.redirect_in = None;
    command.redirect_out = None;
    command.append = None;
    command.redirect_err = None;
    command.redirect_err_append = None;
    command.heredoc = None;
    command.heredoc_delimiter = None;
    command.heredoc_redirects.clear();
    command.here_string = None;
}
