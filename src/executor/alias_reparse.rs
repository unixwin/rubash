use super::alias_case::*;
use super::*;

impl Executor {
    pub(in crate::executor) fn execute_alias_introduced_time(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let words = self.expand_aliases(&command.words);
        if words.first().map(String::as_str) != Some("time") {
            return Ok(None);
        }

        let (source, next_index) = alias_time_source(ast, index, command, &words);
        if let Some(time_command) = alias_time_arithmetic_command(command, &words) {
            self.execute_time_ast_command(&time_command)?;
            return Ok(Some(index + 1));
        }
        if let Some((case_command, redirect_command, next_index)) =
            alias_time_case_command(ast, index, command, &words)
        {
            let time_command = alias_time_command_from_words(
                &words,
                alias_timed_case_command(case_command, redirect_command),
            );
            self.execute_time_ast_command(&time_command)?;
            return Ok(Some(next_index));
        }

        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if !reparsed
            .commands
            .first()
            .is_some_and(|command| command.time_command.is_some())
        {
            return Ok(None);
        }

        self.execute_ast(&reparsed)?;
        Ok(Some(next_index))
    }

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

    pub(in crate::executor) fn execute_alias_introduced_coproc(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let words = self.expand_aliases(&command.words);
        if words.first().map(String::as_str) != Some("coproc") {
            return Ok(None);
        }

        let mut source = alias_compound_source_words(&words);
        append_source_redirects(&mut source, command);
        let mut next_index = index + 1;
        if alias_coproc_needs_following_body(&words) {
            if let Some(next_command) = ast.commands.get(next_index) {
                if command_is_coproc_body_candidate(next_command) {
                    source.push(' ');
                    source.push_str(&bash_command_source_text(next_command));
                    next_index += 1;
                }
            }
        }

        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if !reparsed
            .commands
            .first()
            .is_some_and(|command| command.coproc_command.is_some())
        {
            return Ok(None);
        }

        self.execute_ast(&reparsed)?;
        Ok(Some(next_index))
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
                pattern_open_delimiter: None,
                pattern_open_delimiter_metadata: None,
                pattern_separators: vec![
                    "|".to_string();
                    patterns.patterns.len().saturating_sub(1)
                ],
                pattern_separator_metadata: (0..patterns.patterns.len().saturating_sub(1))
                    .map(|index| synthetic_word_metadata(index, "|"))
                    .collect(),
                pattern_close_delimiter: ")".to_string(),
                pattern_close_delimiter_metadata: synthetic_keyword_metadata(")"),
                patterns: patterns.patterns,
                pattern_nodes,
                body,
                terminator: boundary.terminator,
                terminator_metadata: boundary
                    .terminator_text
                    .as_deref()
                    .map(synthetic_keyword_metadata),
                terminator_text: boundary.terminator_text,
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

        Some((
            CaseCommand {
                keyword: "case".to_string(),
                keyword_metadata: synthetic_keyword_metadata("case"),
                word_metadata: crate::parser::WordMetadata::new(0, word.clone(), word.clone()),
                word,
                in_keyword: "in".to_string(),
                in_keyword_metadata: synthetic_keyword_metadata("in"),
                clauses,
                end_keyword: "esac".to_string(),
                end_keyword_metadata: synthetic_keyword_metadata("esac"),
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

fn synthetic_keyword_metadata(keyword: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(synthetic_word_metadata(0, keyword))
}

fn synthetic_word_metadata(word_index: usize, value: &str) -> crate::parser::WordMetadata {
    crate::parser::WordMetadata::new(word_index, value.to_string(), value.to_string())
}

fn alias_coproc_needs_following_body(words: &[String]) -> bool {
    matches!(words.len(), 1 | 2)
}

fn alias_time_source(
    ast: &Ast,
    index: usize,
    command: &CommandNode,
    words: &[String],
) -> (String, usize) {
    let mut source = alias_compound_source_words(words);
    append_source_redirects(&mut source, command);
    let mut next_index = index + 1;
    let Some(end_word) = alias_time_compound_end_word(words) else {
        return (source, next_index);
    };

    if words.iter().any(|word| word == end_word) {
        return (source, next_index);
    }

    for command_index in index + 1..ast.commands.len() {
        let Some(next_command) = ast.commands.get(command_index) else {
            break;
        };
        let command_source = bash_command_source_text(next_command);
        if !command_source.is_empty() {
            source.push_str("; ");
            source.push_str(&command_source);
        }
        next_index = command_index + 1;
        if command_contains_word(next_command, end_word) {
            break;
        }
    }

    (source, next_index)
}

fn alias_time_compound_end_word(words: &[String]) -> Option<&'static str> {
    let index = alias_time_compound_word_index(words)?;

    match words.get(index).map(String::as_str)? {
        "if" => Some("fi"),
        "case" => Some("esac"),
        "for" | "while" | "until" | "select" => Some("done"),
        _ => None,
    }
}

fn alias_time_compound_word_index(words: &[String]) -> Option<usize> {
    let mut index = 1;
    while matches!(
        words.get(index).map(String::as_str),
        Some("-p" | "--" | "!")
    ) {
        index += 1;
    }

    words.get(index)?;
    Some(index)
}

fn alias_time_case_command<'a>(
    ast: &'a Ast,
    index: usize,
    command: &'a CommandNode,
    words: &[String],
) -> Option<(CaseCommand, &'a CommandNode, usize)> {
    let case_word_index = alias_time_compound_word_index(words)?;
    if words.get(case_word_index).map(String::as_str) != Some("case") {
        return None;
    }

    let mut case_words = words[case_word_index..].to_vec();
    let mut redirect_command = command;
    let mut next_index = index + 1;
    if !case_words.iter().any(|word| word == "esac") {
        for command_index in index + 1..ast.commands.len() {
            let Some(next_command) = ast.commands.get(command_index) else {
                break;
            };
            case_words.extend(next_command.words.iter().cloned());
            redirect_command = next_command;
            next_index = command_index + 1;
            if command_contains_word(next_command, "esac") {
                break;
            }
        }
    }

    case_command_from_words(&case_words)
        .map(|case_command| (case_command, redirect_command, next_index))
}

fn alias_time_arithmetic_command(command: &CommandNode, words: &[String]) -> Option<TimeCommand> {
    let expression_index = alias_time_compound_word_index(words)?;
    if matches!(
        words.get(expression_index).map(String::as_str),
        Some(
            "if" | "case"
                | "for"
                | "while"
                | "until"
                | "select"
                | "function"
                | "coproc"
                | "[["
                | "{"
                | "("
        )
    ) {
        return None;
    }
    if words
        .get(expression_index)
        .is_some_and(|word| word.starts_with('{') || word.starts_with('('))
    {
        return None;
    }

    let expression = words[expression_index..].join(" ");
    if !alias_time_arithmetic_expression_likely(&expression) {
        return None;
    }

    let metadata = ArithmeticExpressionMetadata::new(expression.clone());
    let mut arithmetic = CommandNode::new();
    arithmetic.words = vec!["((".to_string(), expression.clone(), "))".to_string()];
    arithmetic.arithmetic_command = Some(ArithmeticCommand {
        open_delimiter: "((".to_string(),
        open_delimiter_metadata: synthetic_keyword_metadata("(("),
        expression,
        close_delimiter: "))".to_string(),
        close_delimiter_metadata: synthetic_keyword_metadata("))"),
        operators: metadata.operators,
        variables: metadata.variables,
        has_assignment: metadata.has_assignment,
        has_comparison: metadata.has_comparison,
        has_logical: metadata.has_logical,
        has_update: metadata.has_update,
    });
    copy_command_redirects(command, &mut arithmetic);
    Some(alias_time_command_from_words(words, arithmetic))
}

fn alias_time_arithmetic_expression_likely(expression: &str) -> bool {
    let trimmed = expression.trim();
    if trimmed.parse::<i128>().is_ok() {
        return true;
    }

    trimmed.chars().any(|ch| {
        matches!(
            ch,
            '+' | '-' | '*' | '/' | '%' | '<' | '>' | '=' | '!' | '&' | '|'
        )
    })
}

fn alias_timed_case_command(
    case_command: CaseCommand,
    redirect_command: &CommandNode,
) -> CommandNode {
    let mut command = CommandNode::new();
    command.line = redirect_command.line;
    command.case_command = Some(Box::new(case_command));
    copy_command_redirects(redirect_command, &mut command);
    command
}

fn copy_command_redirects(from: &CommandNode, to: &mut CommandNode) {
    to.redirects = from.redirects.clone();
    to.redirect_in = from.redirect_in.clone();
    to.redirect_out = from.redirect_out.clone();
    to.append = from.append.clone();
    to.redirect_err = from.redirect_err.clone();
    to.redirect_err_append = from.redirect_err_append.clone();
    to.heredoc = from.heredoc.clone();
    to.heredoc_delimiter = from.heredoc_delimiter.clone();
    to.heredoc_redirects = from.heredoc_redirects.clone();
    to.here_string = from.here_string.clone();
}

fn alias_time_command_from_words(words: &[String], command: CommandNode) -> TimeCommand {
    let prefix_words = words
        .iter()
        .skip(1)
        .take_while(|word| matches!(word.as_str(), "-p" | "--" | "!"))
        .cloned()
        .collect::<Vec<_>>();
    let prefix_word_metadata = prefix_words
        .iter()
        .enumerate()
        .map(|(index, word)| WordMetadata::new(index, word.clone(), word.clone()))
        .collect::<Vec<_>>();
    TimeCommand {
        keyword: "time".to_string(),
        keyword_metadata: synthetic_keyword_metadata("time"),
        posix_format: prefix_words.iter().any(|word| word == "-p"),
        inverted: prefix_words
            .iter()
            .filter(|word| word.as_str() == "!")
            .count()
            % 2
            == 1,
        prefix_words,
        prefix_word_metadata,
        command: Box::new(command),
    }
}

fn alias_compound_source_words(words: &[String]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if index == 0 || alias_compound_word_is_source_safe(word) {
                word.clone()
            } else {
                shell_single_quote_assignment_value(word)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn alias_compound_word_is_source_safe(word: &str) -> bool {
    if word.starts_with('{') && word.ends_with('}') {
        return true;
    }

    !word.is_empty() && !word.contains(char::is_whitespace) && !word.contains('\'')
}

fn command_is_coproc_body_candidate(command: &CommandNode) -> bool {
    command.brace_group.is_some()
        || command.subshell_command.is_some()
        || command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.case_command.is_some()
        || command.select_command.is_some()
        || command.coproc_command.is_some()
        || command.time_command.is_some()
        || command.function_command.is_some()
        || command.arithmetic_command.is_some()
        || command.conditional_command.is_some()
}
