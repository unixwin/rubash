use super::*;

impl Executor {
    pub(in crate::executor) fn execute_alias_introduced_arithmetic_for(
        &mut self,
        ast: &Ast,
        command_index: usize,
        words: &[String],
    ) -> Result<Option<usize>, ExecuteError> {
        let Some((arithmetic, body_index)) =
            self.alias_arithmetic_for_header(ast, command_index, words)
        else {
            return Ok(None);
        };
        let Some(body_command) = ast.commands.get(body_index) else {
            return Ok(None);
        };

        if let Some(brace_group) = body_command.brace_group.clone() {
            let for_command = alias_arithmetic_for_command(arithmetic, brace_group.body);
            self.execute_for_command_with_redirects(&for_command, body_command)?;
            return Ok(Some(body_index + 1));
        }

        if body_command.words.first().map(String::as_str) != Some("do") {
            return Ok(None);
        }

        let initial_depth = self.embedded_do_loop_depth(body_command);
        let Some(done_index) = self.find_matching_done_command(ast, body_index + 1, initial_depth)
        else {
            return Ok(None);
        };

        let mut body = Vec::new();
        if body_command.words.len() > 1 {
            let mut first_body_command = body_command.clone();
            first_body_command.words = first_body_command.words[1..].to_vec();
            body.push(first_body_command);
        }
        body.extend(ast.commands[body_index + 1..done_index].iter().cloned());

        let for_command = alias_arithmetic_for_command(arithmetic, body);
        let Some(done_command) = ast.commands.get(done_index) else {
            return Ok(None);
        };
        self.execute_for_command_with_redirects(&for_command, done_command)?;
        Ok(Some(done_index + 1))
    }

    fn alias_arithmetic_for_header(
        &self,
        ast: &Ast,
        command_index: usize,
        words: &[String],
    ) -> Option<(ArithmeticForCommand, usize)> {
        if words.first().map(String::as_str) != Some("for") {
            return None;
        }

        let mut parts = split_compact_alias_arithmetic_empty_test(alias_arithmetic_part(
            ast.commands.get(command_index)?,
            &words[1..],
        ));
        let mut index = command_index + 1;
        while parts.len() < 3 {
            let command = ast.commands.get(index)?;
            if alias_arithmetic_empty_command(command) {
                parts.push(String::new());
                index += 1;
                continue;
            }
            if command.brace_group.is_some()
                || command.words.first().map(String::as_str) == Some("do")
            {
                break;
            }
            parts.push(alias_arithmetic_part(command, &command.words));
            index += 1;
        }

        if parts.len() == 2
            && ast.commands.get(index).is_some_and(|command| {
                command.brace_group.is_some()
                    || command.words.first().map(String::as_str) == Some("do")
            })
        {
            parts.push(String::new());
        }

        if parts.len() != 3 {
            return None;
        }

        while ast
            .commands
            .get(index)
            .is_some_and(alias_arithmetic_empty_command)
        {
            index += 1;
        }

        let init = parts[0].clone();
        let test = parts[1].clone();
        let update = parts[2].clone();

        Some((
            ArithmeticForCommand {
                open_delimiter: "((".to_string(),
                open_delimiter_metadata: synthetic_keyword_metadata("(("),
                init: init.clone(),
                init_metadata: ArithmeticExpressionMetadata::new(init),
                separators: vec![";".to_string(), ";".to_string()],
                separator_metadata: vec![
                    synthetic_word_metadata(0, ";"),
                    synthetic_word_metadata(1, ";"),
                ],
                test: test.clone(),
                test_metadata: ArithmeticExpressionMetadata::new(test),
                update: update.clone(),
                update_metadata: ArithmeticExpressionMetadata::new(update),
                close_delimiter: "))".to_string(),
                close_delimiter_metadata: synthetic_keyword_metadata("))"),
            },
            index,
        ))
    }
}

fn alias_arithmetic_for_command(
    arithmetic: ArithmeticForCommand,
    body: Vec<CommandNode>,
) -> ForCommand {
    ForCommand {
        keyword: "for".to_string(),
        keyword_metadata: synthetic_keyword_metadata("for"),
        variable: String::new(),
        variable_metadata: Box::new(WordMetadata::new(0, String::new(), String::new())),
        in_keyword: None,
        in_keyword_metadata: None,
        words: Vec::new(),
        word_metadata: Vec::new(),
        default_positional: false,
        list_terminator: None,
        list_terminator_metadata: None,
        arithmetic: Some(arithmetic),
        body_kind: CommandBodyKind::DoDone,
        body_open_delimiter: Some("do".to_string()),
        body_close_delimiter: Some("done".to_string()),
        do_keyword: Some("do".to_string()),
        do_keyword_metadata: Some(synthetic_keyword_metadata("do")),
        end_keyword: Some("done".to_string()),
        end_keyword_metadata: Some(synthetic_keyword_metadata("done")),
        body,
    }
}

fn synthetic_keyword_metadata(keyword: &str) -> Box<WordMetadata> {
    Box::new(synthetic_word_metadata(0, keyword))
}

fn synthetic_word_metadata(word_index: usize, value: &str) -> WordMetadata {
    WordMetadata::new(word_index, value.to_string(), value.to_string())
}

fn alias_arithmetic_part(command: &CommandNode, words: &[String]) -> String {
    let mut text = words.join(" ");
    append_alias_arithmetic_redirect(&mut text, command.redirect_in.as_ref(), "<");
    append_alias_arithmetic_redirect(&mut text, command.redirect_out.as_ref(), ">");
    append_alias_arithmetic_redirect(&mut text, command.append.as_ref(), ">>");
    text
}

fn split_compact_alias_arithmetic_empty_test(text: String) -> Vec<String> {
    let Some((init, update)) = text.split_once(";;") else {
        return vec![text];
    };
    vec![
        trim_alias_arithmetic_delimiters(init),
        String::new(),
        trim_alias_arithmetic_delimiters(update),
    ]
}

fn trim_alias_arithmetic_delimiters(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix("((").unwrap_or(text).trim_start();
    text.strip_suffix("))")
        .unwrap_or(text)
        .trim_end()
        .to_string()
}

fn append_alias_arithmetic_redirect(text: &mut String, redirect: Option<&Redirect>, op: &str) {
    let Some(redirect) = redirect else {
        return;
    };
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(op);
    text.push(' ');
    text.push_str(&redirect.target);
}

fn alias_arithmetic_empty_command(command: &CommandNode) -> bool {
    command.words.is_empty() && command.brace_group.is_none() && command_has_no_effect(command)
}
