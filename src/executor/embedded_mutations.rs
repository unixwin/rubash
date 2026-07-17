use super::*;
use crate::executor::parameter_core::word_contains_current_shell_command_substitution;

impl Executor {
    pub(in crate::executor) fn expand_embedded_parameters_mut(&mut self, word: &str) -> String {
        self.apply_parameter_assignment_expansions_in_word(word);
        let saved_parameter_env =
            word_contains_current_shell_command_substitution(word).then(|| self.env_vars.clone());
        let word = self.expand_embedded_arithmetic_mut(word);
        let word = self.expand_embedded_command_substitutions_mut(&word);
        let expanded = if let Some(saved_parameter_env) = saved_parameter_env {
            let current_env = std::mem::replace(&mut self.env_vars, saved_parameter_env);
            let expanded = self.expand_embedded_parameters(&word);
            self.env_vars = current_env;
            expanded
        } else {
            self.expand_embedded_parameters(&word)
        };
        let expanded = if word.contains("$(") || word.contains('`') {
            unescape_remaining_shell_escapes(&expanded)
                .replace("\\\\'", "'")
                .replace("\\'", "'")
        } else {
            expanded
        };
        restore_protected_replacement_quotes(&expanded)
    }

    pub(in crate::executor) fn expand_embedded_command_substitutions_mut(
        &mut self,
        word: &str,
    ) -> String {
        let mut output = String::new();
        let mut chars = word.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek().copied() == Some('{') {
                chars.next();
                let pipe_output = chars.peek().copied() == Some('|');
                if pipe_output || chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    if pipe_output {
                        chars.next();
                    }
                    let mut depth = 1usize;
                    let mut source = String::new();
                    let mut single = false;
                    let mut double = false;
                    let mut escaped = false;
                    let mut closed = false;
                    for source_ch in chars.by_ref() {
                        if escaped {
                            source.push(source_ch);
                            escaped = false;
                            continue;
                        }
                        if source_ch == '\\' && !single {
                            source.push(source_ch);
                            escaped = true;
                            continue;
                        }
                        match source_ch {
                            '\'' if !double => {
                                single = !single;
                                source.push(source_ch);
                            }
                            '"' if !single => {
                                double = !double;
                                source.push(source_ch);
                            }
                            '{' if !single && !double => {
                                depth += 1;
                                source.push(source_ch);
                            }
                            '}' if !single && !double => {
                                depth = depth.saturating_sub(1);
                                if depth == 0 {
                                    closed = true;
                                    break;
                                }
                                source.push(source_ch);
                            }
                            _ => source.push(source_ch),
                        }
                    }
                    if closed {
                        output.push_str(&protect_command_substitution_output(
                            &self.expand_current_shell_command_substitution(&source, pipe_output),
                        ));
                    } else {
                        output.push_str(if pipe_output { "${|" } else { "${" });
                        output.push_str(&source);
                    }
                    continue;
                }

                output.push_str("${");
                continue;
            }

            if ch == '$' && chars.peek().copied() == Some('(') {
                chars.next();
                if chars.peek().copied() == Some('(') {
                    output.push_str("$((");
                    chars.next();
                    continue;
                }

                let mut depth = 1usize;
                let mut source = String::new();
                let mut single = false;
                let mut double = false;
                let mut escaped = false;
                let mut case_depth = 0usize;
                let mut word = String::new();
                let mut word_boundary = true;
                let mut current_word_boundary = true;
                while let Some(source_ch) = chars.next() {
                    if escaped {
                        source.push(source_ch);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' && !single {
                        source.push(source_ch);
                        escaped = true;
                        continue;
                    }
                    if source_ch == '#' && !single && !double && word_boundary {
                        source.push(source_ch);
                        while let Some(comment_ch) = chars.peek().copied() {
                            if comment_ch == '\n' {
                                break;
                            }
                            source.push(comment_ch);
                            chars.next();
                        }
                        word.clear();
                        word_boundary = true;
                        current_word_boundary = true;
                        continue;
                    }
                    let rest = chars.clone().collect::<String>();
                    update_command_substitution_case_depth(
                        source_ch,
                        single,
                        double,
                        &mut word,
                        &mut case_depth,
                        &mut word_boundary,
                        &mut current_word_boundary,
                        &rest,
                    );
                    match source_ch {
                        '\'' if !double => {
                            single = !single;
                            source.push(source_ch);
                        }
                        '"' if !single => {
                            double = !double;
                            source.push(source_ch);
                        }
                        '(' if !single && !double && case_depth == 0 => {
                            depth += 1;
                            source.push(source_ch);
                        }
                        ')' if !single && !double && case_depth == 0 => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                            source.push(source_ch);
                        }
                        _ => source.push(source_ch),
                    }
                }
                let source = unescape_storage_command_substitution_source(&source);
                output.push_str(&protect_command_substitution_output(
                    &self.expand_command_substitution_mut(&source),
                ));
                continue;
            }

            if ch == '`' {
                let mut source = String::new();
                let mut escaped = false;
                let mut closed = false;
                for source_ch in chars.by_ref() {
                    if escaped {
                        source.push(source_ch);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if source_ch == '`' {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                if closed {
                    output.push_str(&protect_command_substitution_output(
                        &self.expand_command_substitution_mut(&source),
                    ));
                } else {
                    output.push('`');
                    output.push_str(&source);
                }
                continue;
            }

            output.push(ch);
        }

        output
    }

    fn expand_current_shell_command_substitution(
        &mut self,
        source: &str,
        pipe_output: bool,
    ) -> String {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        let saved_exit_code = self.exit_code;

        let (status, output) = if pipe_output {
            let result = self.execute_ast(&ast);
            let status = command_substitution_status(result, self.exit_code);
            (status, String::new())
        } else {
            let saved_capture = self.stdout_capture.take();
            self.stdout_capture = Some(Vec::new());
            let result = self.execute_ast(&ast);
            let status = command_substitution_status(result, self.exit_code);
            let output = String::from_utf8_lossy(&self.stdout_capture.take().unwrap_or_default())
                .trim_end_matches('\n')
                .to_string();
            self.stdout_capture = saved_capture;
            (status, output)
        };

        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        if pipe_output {
            self.env_vars.get("REPLY").cloned().unwrap_or_default()
        } else {
            output
        }
    }

    pub(in crate::executor) fn expand_command_substitution_mut(&mut self, source: &str) -> String {
        let source = source.trim();
        let words = self.expand_aliases(&split_shell_words(source));
        if let Some(output) = self.run_function_command_substitution(&words) {
            return output;
        }
        if command_substitution_uses_specialized_path(self, source, &words) {
            return self.expand_command_substitution(source);
        }
        if let Some(output) = self.run_ast_command_substitution(source) {
            return output;
        }
        self.expand_command_substitution(source)
    }

    pub(in crate::executor) fn run_ast_command_substitution(
        &mut self,
        source: &str,
    ) -> Option<String> {
        if source.contains("<<") {
            return None;
        }

        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        if !command_substitution_needs_ast_execution(&ast) {
            return None;
        }

        let saved_env = self.env_vars.clone();
        let saved_functions = self.functions.clone();
        let saved_function_redirects = self.function_definition_redirects.clone();
        let saved_aliases = self.aliases.clone();
        let saved_exit_code = self.exit_code;
        let saved_dir = env::current_dir().ok();
        let saved_depth = self.subshell_depth.get();
        self.subshell_depth.set(saved_depth + 1);

        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        let result = self.execute_ast(&ast);
        let output = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;

        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) => status,
            Err(_) => 1,
        };

        self.restore_shell_env(saved_env);
        self.functions = saved_functions;
        self.function_definition_redirects = saved_function_redirects;
        self.aliases = saved_aliases;
        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        self.subshell_depth.set(saved_depth);
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(
            String::from_utf8_lossy(&output)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    pub(in crate::executor) fn run_function_command_substitution(
        &mut self,
        words: &[String],
    ) -> Option<String> {
        let name = words.first()?;
        if !self.functions.contains_key(name) {
            return None;
        }

        let args = words[1..]
            .iter()
            .flat_map(|word| self.expand_command_substitution_arg_values(word))
            .collect::<Vec<_>>();
        let mut call = CommandNode::new();
        call.words = words.to_vec();

        let saved_env = self.env_vars.clone();
        let saved_exit_code = self.exit_code;
        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        let result = self.execute_function(name, &args, &call);
        let output = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;
        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) => status,
            Err(_) => 1,
        };
        self.env_vars = saved_env;
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(
            String::from_utf8_lossy(&output)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    pub(in crate::executor) fn expand_embedded_arithmetic_mut(&mut self, word: &str) -> String {
        let chars: Vec<char> = word.chars().collect();
        let mut output = String::new();
        let mut index = 0;

        while index < chars.len() {
            if chars[index] == '$'
                && chars.get(index + 1) == Some(&'(')
                && chars.get(index + 2) == Some(&'(')
            {
                index += 3;
                let mut expression = String::new();
                let mut paren_depth: usize = 0;
                let mut matched = false;

                while index < chars.len() {
                    match chars[index] {
                        '(' => {
                            paren_depth += 1;
                            expression.push(chars[index]);
                            index += 1;
                        }
                        ')' if paren_depth == 0 && chars.get(index + 1) == Some(&')') => {
                            index += 2;
                            matched = true;
                            break;
                        }
                        ')' => {
                            paren_depth = paren_depth.saturating_sub(1);
                            expression.push(chars[index]);
                            index += 1;
                        }
                        ch => {
                            expression.push(ch);
                            index += 1;
                        }
                    }
                }

                if matched {
                    if let Some(value) = self.eval_arithmetic_command_value(&expression) {
                        output.push_str(&value.to_string());
                    }
                } else {
                    output.push_str("$((");
                    output.push_str(&expression);
                }
                continue;
            }

            if chars[index] == '$' && chars.get(index + 1) == Some(&'[') {
                index += 2;
                let mut expression = String::new();
                let mut bracket_depth: usize = 0;
                let mut matched = false;

                while index < chars.len() {
                    match chars[index] {
                        '[' => {
                            bracket_depth += 1;
                            expression.push(chars[index]);
                            index += 1;
                        }
                        ']' if bracket_depth == 0 => {
                            index += 1;
                            matched = true;
                            break;
                        }
                        ']' => {
                            bracket_depth = bracket_depth.saturating_sub(1);
                            expression.push(chars[index]);
                            index += 1;
                        }
                        ch => {
                            expression.push(ch);
                            index += 1;
                        }
                    }
                }

                if matched {
                    if let Some(value) = self.eval_arithmetic_command_value(&expression) {
                        output.push_str(&value.to_string());
                    }
                } else {
                    output.push_str("$[");
                    output.push_str(&expression);
                }
                continue;
            }

            output.push(chars[index]);
            index += 1;
        }

        output
    }
}

fn command_substitution_needs_ast_execution(ast: &Ast) -> bool {
    ast.commands.iter().any(command_has_ast_substitution_shape)
        || ast
            .commands
            .iter()
            .any(command_contains_current_shell_substitution)
        || (ast.commands.len() > 1 && ast.commands.iter().all(command_is_ast_list_substitution))
}

fn command_substitution_status(result: Result<(), ExecuteError>, exit_code: i32) -> i32 {
    match result {
        Ok(()) => exit_code,
        Err(ExecuteError::Return(status)) => status,
        Err(ExecuteError::ExitCode(status)) => status,
        Err(_) => 1,
    }
}

fn command_has_ast_substitution_shape(command: &CommandNode) -> bool {
    command.and_or_list.is_some()
        || command.inverted_command.is_some()
        || command.background_command.is_some()
        || command_has_compound_substitution(command)
}

fn command_has_compound_substitution(command: &CommandNode) -> bool {
    command.pipeline_command.as_ref().is_some_and(|pipeline| {
        pipeline
            .stages
            .iter()
            .any(command_has_compound_substitution)
    }) || command
        .and_or_list
        .as_ref()
        .is_some_and(|list| list.commands.iter().any(command_has_compound_substitution))
        || command
            .inverted_command
            .as_ref()
            .is_some_and(|inverted| command_has_compound_substitution(&inverted.command))
        || command
            .time_command
            .as_ref()
            .is_some_and(|time| command_has_compound_substitution(&time.command))
        || command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.select_command.is_some()
        || command.case_command.is_some()
        || command.coproc_command.is_some()
        || command.subshell_command.is_some()
        || command.brace_group.is_some()
        || command.arithmetic_command.is_some()
        || command.conditional_command.is_some()
}

fn command_contains_current_shell_substitution(command: &CommandNode) -> bool {
    command
        .words
        .iter()
        .any(|word| word_contains_current_shell_command_substitution(word))
}

fn command_is_ast_list_substitution(command: &CommandNode) -> bool {
    if !command_has_simple_substitution_shape(command) {
        return false;
    }
    if !command.assignments.is_empty() {
        return true;
    }
    matches!(
        command.words.first().map(String::as_str),
        Some("echo" | "printf" | "true" | "false" | ":" | "pwd")
    )
}

fn command_has_simple_substitution_shape(command: &CommandNode) -> bool {
    command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.inverted_command.is_none()
        && command.background_command.is_none()
        && command.time_command.is_none()
        && command.for_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.select_command.is_none()
        && command.case_command.is_none()
        && command.coproc_command.is_none()
        && command.subshell_command.is_none()
        && command.brace_group.is_none()
        && command.arithmetic_command.is_none()
        && command.conditional_command.is_none()
}

fn command_substitution_uses_specialized_path(
    executor: &Executor,
    source: &str,
    words: &[String],
) -> bool {
    source.contains("<<")
        || words.iter().any(|word| word == "|")
        || words.first().map(String::as_str) == Some("time")
        || executor
            .command_substitution_cd_pwd_output(source)
            .is_some()
}

fn update_command_substitution_case_depth(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    rest: &str,
) {
    if single || double {
        word.clear();
        *word_boundary = false;
        return;
    }

    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary => {
            *case_depth += 1;
            false
        }
        "esac" if *current_word_boundary && !case_pattern_starts_with_esac_rest(ch, rest) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary =>
        {
            true
        }
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

fn case_pattern_starts_with_esac_rest(delimiter: char, rest: &str) -> bool {
    if !matches!(delimiter, ')' | '|') {
        return false;
    }

    let chars = std::iter::once(delimiter)
        .chain(rest.chars())
        .collect::<Vec<_>>();
    let mut close = 0usize;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return false,
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        return false;
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return true;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        if ch == ')' {
            return false;
        }
        if word.is_empty() {
            if command_substitution_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && command_substitution_reserved_word_allows_next(&word);
        word.clear();
        word_boundary =
            reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn command_substitution_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}
