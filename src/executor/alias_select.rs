use super::*;

impl Executor {
    pub(in crate::executor) fn execute_alias_introduced_select(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let mut command_index = index;
        while ast
            .commands
            .get(command_index)
            .is_some_and(|command| command.words.is_empty())
        {
            command_index += 1;
        }
        let Some(command) = ast.commands.get(command_index) else {
            return Ok(None);
        };

        let words = if self.posix_mode_enabled() {
            self.expand_aliases_preserving_reserved(&command.words)
        } else {
            self.expand_aliases(&command.words)
        };
        if words.first().map(String::as_str) != Some("select") {
            return Ok(None);
        }
        if words.len() < 2 {
            return Ok(None);
        }

        let mut do_index = command_index + 1;
        while ast
            .commands
            .get(do_index)
            .is_some_and(|command| command.words.is_empty() && command.brace_group.is_none())
        {
            do_index += 1;
        }

        let variable = words[1].clone();
        let (select_words, default_positional) = if words.get(2).map(String::as_str) == Some("in") {
            (words[3..].to_vec(), false)
        } else if words.len() == 2 {
            (Vec::new(), true)
        } else {
            return Ok(None);
        };
        let Some(do_command) = ast.commands.get(do_index) else {
            return Ok(None);
        };
        let in_keyword = (!default_positional).then(|| "in".to_string());
        if let Some(brace_group) = do_command.brace_group.clone() {
            let select_command = SelectCommand {
                keyword: "select".to_string(),
                keyword_metadata: synthetic_keyword_metadata("select"),
                variable: variable.clone(),
                variable_metadata: Box::new(WordMetadata::new(
                    0,
                    variable.clone(),
                    variable.clone(),
                )),
                in_keyword: in_keyword.clone(),
                in_keyword_metadata: in_keyword
                    .as_ref()
                    .map(|keyword| synthetic_keyword_metadata(keyword)),
                words: select_words,
                word_metadata: Vec::new(),
                default_positional,
                list_terminator: None,
                list_terminator_metadata: None,
                body_kind: CommandBodyKind::BraceGroup,
                body_open_delimiter: Some("{".to_string()),
                body_open_delimiter_metadata: Some(synthetic_keyword_metadata("{")),
                body_close_delimiter: Some("}".to_string()),
                body_close_delimiter_metadata: Some(synthetic_keyword_metadata("}")),
                do_keyword: None,
                do_keyword_metadata: None,
                end_keyword: None,
                end_keyword_metadata: None,
                body: brace_group.body,
            };
            self.execute_select_command(do_command, &select_command)?;
            return Ok(Some(do_index + 1));
        }
        if do_command.words.first().map(String::as_str) != Some("do") {
            return Ok(None);
        }

        let initial_depth = self.embedded_do_loop_depth(do_command);
        let Some(done_index) = self.find_matching_done_command(ast, do_index + 1, initial_depth)
        else {
            return Ok(None);
        };
        let done_command = ast.commands.get(done_index).expect("done index is valid");

        let mut body = Vec::new();
        if do_command.words.len() > 1 {
            let mut body_command = do_command.clone();
            body_command.words = body_command.words[1..].to_vec();
            body.push(body_command);
        }
        body.extend(ast.commands[do_index + 1..done_index].iter().cloned());

        let select_command = SelectCommand {
            keyword: "select".to_string(),
            keyword_metadata: synthetic_keyword_metadata("select"),
            variable: variable.clone(),
            variable_metadata: Box::new(WordMetadata::new(0, variable.clone(), variable)),
            in_keyword,
            in_keyword_metadata: (!default_positional).then(|| synthetic_keyword_metadata("in")),
            words: select_words,
            word_metadata: Vec::new(),
            default_positional,
            list_terminator: None,
            list_terminator_metadata: None,
            body_kind: CommandBodyKind::DoDone,
            body_open_delimiter: Some("do".to_string()),
            body_open_delimiter_metadata: Some(synthetic_keyword_metadata("do")),
            body_close_delimiter: Some("done".to_string()),
            body_close_delimiter_metadata: Some(synthetic_keyword_metadata("done")),
            do_keyword: Some("do".to_string()),
            do_keyword_metadata: Some(synthetic_keyword_metadata("do")),
            end_keyword: Some("done".to_string()),
            end_keyword_metadata: Some(synthetic_keyword_metadata("done")),
            body,
        };
        self.execute_select_command(done_command, &select_command)?;
        Ok(Some(done_index + 1))
    }
}

fn synthetic_keyword_metadata(keyword: &str) -> Box<WordMetadata> {
    Box::new(WordMetadata::new(
        0,
        keyword.to_string(),
        keyword.to_string(),
    ))
}
