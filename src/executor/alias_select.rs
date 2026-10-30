use super::alias_loop_match::control_word;
use super::*;

impl Executor {
    /// rubash#2062: first index at or after `index` whose command
    /// carries words (or `commands.len()` when none does). The select
    /// probe needs only that command, but it runs at every index of
    /// the enclosing list walk, so the unmemoized linear scan made a
    /// list of N wordless commands (plain assignments) cost Θ(N²).
    /// The scan is a pure function of the list, so its result is
    /// memoized on the executor (see alias_select_scan_memo); a later
    /// probe whose start lies between the memoized scan start and its
    /// result reuses the result verbatim.
    fn alias_select_first_word_index(&self, commands: &[CommandNode], index: usize) -> usize {
        if let Some((ptr, len, from, found)) = *self.alias_select_scan_memo.borrow() {
            if ptr == commands.as_ptr() as usize
                && len == commands.len()
                && from <= index
                && index <= found
            {
                return found;
            }
        }
        let mut found = index;
        while commands
            .get(found)
            .is_some_and(|command| command.words.is_empty())
        {
            found += 1;
        }
        *self.alias_select_scan_memo.borrow_mut() =
            Some((commands.as_ptr() as usize, commands.len(), index, found));
        found
    }

    pub(in crate::executor) fn execute_alias_introduced_select(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let command_index = self.alias_select_first_word_index(commands, index);
        let Some(command) = commands.get(command_index) else {
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
        while commands.get(do_index).is_some_and(|command| {
            command.words.is_empty()
                && command.brace_group.is_none()
                && command.get_assignment("__RUBASH_PARSE_ERROR__").is_none()
        }) {
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
        let Some(do_command) = commands.get(do_index) else {
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
        if control_word(do_command) != Some("do") {
            return Ok(None);
        }

        let initial_depth = self.embedded_do_loop_depth(do_command);
        let Some(done_index) =
            self.find_matching_done_command(commands, do_index + 1, initial_depth)
        else {
            return Ok(None);
        };
        let done_command = commands.get(done_index).expect("done index is valid");

        let mut body = Vec::new();
        if do_command.words.len() > 1 {
            let mut body_command = do_command.clone();
            body_command.words = body_command.words[1..].to_vec();
            body.push(body_command);
        }
        body.extend(commands[do_index + 1..done_index].iter().cloned());

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
        let redirect_command = commands.get(done_index + 1).unwrap_or(done_command);
        self.execute_select_command(redirect_command, &select_command)?;
        Ok(Some((done_index + 2).min(commands.len())))
    }
}

fn synthetic_keyword_metadata(keyword: &str) -> Box<WordMetadata> {
    Box::new(WordMetadata::new(
        0,
        keyword.to_string(),
        keyword.to_string(),
    ))
}
