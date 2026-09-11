use crate::lexer::{Token, TokenKind};

use super::*;

#[derive(Default)]
pub struct ParseLoopOptions {
    /// Treat a ")" or ";;" at command position as a syntax error that aborts
    /// the remaining input (GNU YYABORT). Enabled for eval reparse.
    pub stray_close_is_error: bool,
    /// The ORIGINAL text being parsed, when the caller has it (eval reparse).
    /// GNU echoes the offending input line verbatim; token reconstruction
    /// cannot recover the original spacing, so the guard slices this text.
    pub source_text: Option<String>,
    /// How many lines the caller shifted token positions by before parsing
    /// (eval reparse shifts by the caller's line counter minus one), so the
    /// guard can map a token position back to a 0-based source-text line.
    pub source_line_offset: usize,
}

pub(super) struct ParseState {
    pub(super) ast: Ast,
    pub(super) current_cmd: CommandNode,
    pub(super) in_subshell: bool,
}

/// Parse tokens into an AST
pub fn parse(tokens: &[Token]) -> Ast {
    parse_with_options(tokens, ParseLoopOptions::default())
}

/// Options for the parse loop. The stray-close guard is enabled only for
/// eval reparse: a top-level ")" can also be the legitimate closer of a
/// multi-line $(...) substitution whose folding the lexer does not yet
/// perform (comsub-posix.tests), so the top-level loop must keep dropping
/// it silently until the lexer folds multi-line substitutions.
pub fn parse_with_options(tokens: &[Token], options: ParseLoopOptions) -> Ast {
    let mut state = ParseState {
        ast: Ast {
            commands: Vec::new(),
        },
        current_cmd: CommandNode::new(),
        in_subshell: false,
    };

    let mut i = 0;
    while i < tokens.len() {
        // GNU parse.y: a ')' or a case clause terminator at command position
        // is a syntax error that aborts the remaining input ("case x in
        // esac)" -- the empty case list closes at esac and the ')' is
        // unexpected). The parser used to drop the token silently and run
        // the rest of the line as a simple command.
        if options.stray_close_is_error
            && command_is_empty(&state.current_cmd)
            && (tokens[i].value == ")" || matches!(tokens[i].raw.as_str(), ";;" | ";&" | ";;;&"))
        {
            state.current_cmd.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                format!("unexpected token `{}'", tokens[i].value),
            );
            // GNU echoes only the offending input line, from its first
            // token, not the rest of the file (parse.y y.error prints the
            // current input line). With the original text available (eval
            // reparse) echo that line verbatim; token reconstruction cannot
            // recover the original spacing.
            let verbatim = options.source_text.as_ref().and_then(|text| {
                let string_line = tokens[i]
                    .position
                    .checked_sub(options.source_line_offset)?
                    .checked_sub(1)?;
                text.split('\n').nth(string_line).map(str::to_string)
            });
            let source = verbatim.unwrap_or_else(|| {
                let line_number = tokens[i].position;
                let mut line_start = i;
                while line_start > 0 && tokens[line_start - 1].position == line_number {
                    line_start -= 1;
                }
                let mut joined = tokens[line_start].raw.clone();
                for token in tokens[line_start + 1..].iter() {
                    if token.position != line_number {
                        break;
                    }
                    joined.push(' ');
                    joined.push_str(&token.raw);
                }
                joined
            });
            state
                .current_cmd
                .insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
            state.ast.commands.push(state.current_cmd);
            state.current_cmd = CommandNode::new();
            break;
        }

        if let Some(next_i) = try_parse_compound_start(tokens, i, &mut state) {
            i = next_i;
            continue;
        }

        match handle_token(tokens, &mut i, &mut state) {
            TokenAction::Advance => i += 1,
            TokenAction::Continue => continue,
            TokenAction::Break => break,
        }
    }

    if !command_is_empty(&state.current_cmd) {
        state.ast.commands.push(state.current_cmd);
    }

    state.ast.commands = fold_pipeline_commands(state.ast.commands);
    state.ast.commands = fold_time_pipeline_commands(state.ast.commands);
    state.ast.commands = fold_time_simple_commands(state.ast.commands);
    state.ast.commands = fold_inverted_commands(state.ast.commands);
    state.ast.commands = fold_and_or_list_commands(state.ast.commands);
    state.ast.commands = fold_background_commands(state.ast.commands);
    mark_parse_time_extglob_errors(&mut state.ast, tokens);
    state.ast
}

/// Extglob is enabled while Bash parses a command unit, not after the unit
/// has already been read.  Therefore `shopt -s extglob; echo @(x)` is a
/// syntax error even though a later input line can use the pattern.
fn mark_parse_time_extglob_errors(ast: &mut Ast, tokens: &[Token]) {
    let mut saw_shopt = false;
    let mut saw_enable = false;
    let mut extglob_enabled_in_unit = false;

    for token in tokens {
        if token.kind == TokenKind::Semicolon && !token.line_break {
            if saw_shopt && saw_enable {
                extglob_enabled_in_unit = true;
            }
            saw_shopt = false;
            saw_enable = false;
            continue;
        }
        if token.value == "shopt" {
            saw_shopt = true;
            continue;
        }
        if saw_shopt && token.value == "-s" {
            saw_enable = true;
            continue;
        }
        if saw_enable && token.value == "extglob" {
            continue;
        }
        if extglob_enabled_in_unit && is_unquoted_extglob_word(token) {
            for command in &mut ast.commands {
                if command.words.iter().any(|word| word == &token.value) {
                    command.insert_assignment(
                        "__RUBASH_PARSE_ERROR__".to_string(),
                        "unexpected token `('".to_string(),
                    );
                    break;
                }
            }
        }
        if token.kind != TokenKind::Semicolon {
            saw_shopt = false;
            saw_enable = false;
        }
    }
}

fn is_unquoted_extglob_word(token: &Token) -> bool {
    token.raw == token.value
        && token
            .value
            .chars()
            .any(|operator| matches!(operator, '@' | '!' | '+' | '?' | '*'))
        && token.value.contains("(")
}

fn fold_inverted_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command.inverted {
                return command;
            }

            command.inverted = false;
            let line = command.line;
            let and_or = command.and_or.take();
            let background_flag = command.background;
            command.background = false;
            let mut inverted = CommandNode::new();
            inverted.line = line;
            inverted.and_or = and_or;
            inverted.background = background_flag;
            inverted.inverted_command = Some(InvertedCommand {
                operator: "!".to_string(),
                operator_metadata: operator_metadata("!"),
                command: Box::new(command),
            });
            inverted
        })
        .collect()
}

fn fold_background_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command.background {
                return command;
            }

            command.background = false;
            let line = command.line;
            let mut background = CommandNode::new();
            background.line = line;
            background.background_command = Some(BackgroundCommand {
                operator: "&".to_string(),
                operator_metadata: operator_metadata("&"),
                command: Box::new(command),
            });
            background
        })
        .collect()
}

fn operator_metadata(operator: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, operator, operator))
}

fn fold_and_or_list_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut folded = Vec::new();
    let mut index = 0;
    while index < commands.len() {
        let command = commands[index].clone();
        if command.and_or.is_none() {
            folded.push(command);
            index += 1;
            continue;
        }

        let mut list_commands = vec![command];
        let mut connectors = Vec::new();
        let mut operators = Vec::new();
        index += 1;
        while let Some(connector) = list_commands.last().and_then(|command| command.and_or) {
            connectors.push(connector);
            operators.push(if connector { "&&" } else { "||" }.to_string());
            while commands.get(index).is_some_and(command_is_empty) {
                index += 1;
            }
            let Some(next) = commands.get(index).cloned() else {
                break;
            };
            list_commands.push(next);
            index += 1;
        }

        if connectors.is_empty() || list_commands.len() != connectors.len() + 1 {
            if list_commands.len() == 1 && !connectors.is_empty() {
                let mut command = list_commands
                    .into_iter()
                    .next()
                    .expect("and-or list has a command");
                command.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    "unexpected end of file".to_string(),
                );
                folded.push(command);
                continue;
            }
            folded.extend(list_commands);
            continue;
        }

        let first = list_commands
            .first()
            .expect("and-or list has a first command");
        let last = list_commands
            .last()
            .expect("and-or list has a last command");
        let mut list = CommandNode::new();
        list.line = first.line;
        list.background = last.background;
        list.and_or_list = Some(AndOrListCommand {
            commands: list_commands,
            connectors,
            operator_metadata: operators_metadata(&operators),
            operators,
        });
        folded.push(list);
    }
    folded
}

fn fold_pipeline_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut folded = Vec::new();
    let mut index = 0;
    while index < commands.len() {
        let command = commands[index].clone();
        if command.pipe.is_none() {
            folded.push(command);
            index += 1;
            continue;
        }

        let mut stages = vec![command];
        let mut operators = Vec::new();
        index += 1;
        skip_empty_pipeline_separators(&commands, &mut index);
        while let Some(command) = commands.get(index) {
            if let Some(pipe) = stages.last().and_then(|stage| stage.pipe) {
                operators.push(if pipe == 2 { "|&" } else { "|" }.to_string());
            }
            stages.push(command.clone());
            index += 1;
            if command.pipe.is_none() {
                break;
            }
            skip_empty_pipeline_separators(&commands, &mut index);
        }

        if stages.len() == 1
            || stages.last().is_some_and(|command| command.pipe.is_some())
            || looks_like_case_pattern_alternate(&stages)
        {
            if stages.len() == 1 || stages.last().is_some_and(|command| command.pipe.is_some()) {
                let mut command = stages
                    .into_iter()
                    .next()
                    .expect("pipeline has a first stage");
                command.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    "unexpected token `|'".to_string(),
                );
                folded.push(command);
                continue;
            }
            folded.extend(stages);
            continue;
        }

        fold_time_pipeline_stage_commands(&mut stages);

        let first = stages.first().expect("pipeline has a first stage");
        let last = stages.last().expect("pipeline has a last stage");
        let mut pipeline = CommandNode::new();
        pipeline.line = first.line;
        pipeline.inverted = first.inverted;
        pipeline.background = last.background;
        pipeline.and_or = last.and_or;
        if let Some(first_stage) = stages.first_mut() {
            first_stage.inverted = false;
        }
        pipeline.pipeline_command = Some(PipelineCommand {
            stages,
            operator_metadata: operators_metadata(&operators),
            operators,
        });
        folded.push(pipeline);
    }
    folded
}

fn skip_empty_pipeline_separators(commands: &[CommandNode], index: &mut usize) {
    while commands.get(*index).is_some_and(command_is_empty) {
        *index += 1;
    }
}

fn operators_metadata(operators: &[String]) -> Vec<WordMetadata> {
    operators
        .iter()
        .enumerate()
        .map(|(index, operator)| build_word_metadata(index, operator, operator))
        .collect()
}

fn looks_like_case_pattern_alternate(stages: &[CommandNode]) -> bool {
    let Some(first) = stages.first() else {
        return false;
    };
    if first.words.get(2).map(String::as_str) != Some("in") {
        return false;
    }
    first.words.len() >= 4 && stages.len() >= 2
}

fn fold_time_pipeline_stage_commands(stages: &mut [CommandNode]) {
    for stage in stages.iter_mut().skip(1) {
        let folded = fold_time_pipeline_stage_command(std::mem::take(stage));
        *stage = folded;
    }
}

fn fold_time_pipeline_stage_command(mut command: CommandNode) -> CommandNode {
    if !command_is_time_simple_candidate(&command) {
        return command;
    }
    let Some(prefix) = time_prefix_from_command(&mut command) else {
        return command;
    };

    let mut timed = CommandNode::new();
    timed.line = command.line;
    timed.inverted = command.inverted;
    timed.pipe = command.pipe.take();
    timed.redirect_in = command.redirect_in.clone();
    timed.redirect_out = command.redirect_out.clone();
    timed.append = command.append.clone();
    timed.redirect_err = command.redirect_err.clone();
    timed.redirect_err_append = command.redirect_err_append.clone();
    command.inverted = false;
    timed.time_command = Some(TimeCommand {
        keyword: prefix.keyword,
        keyword_metadata: prefix.keyword_metadata,
        prefix_words: prefix.prefix_words,
        prefix_word_metadata: prefix.prefix_word_metadata,
        command: Box::new(command),
        posix_format: prefix.posix_format,
        inverted: prefix.inverted,
    });
    timed
}

fn fold_time_pipeline_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            let Some(pipeline) = command.pipeline_command.as_mut() else {
                return command;
            };
            let Some(prefix) = time_prefix_from_pipeline(pipeline) else {
                return command;
            };

            let line = command.line;
            let inverted = command.inverted;
            let and_or = command.and_or.take();
            let background = command.background;
            command.inverted = false;
            command.background = false;
            command.and_or = None;
            let mut timed = CommandNode::new();
            timed.line = line;
            timed.inverted = inverted;
            timed.and_or = and_or;
            timed.background = background;
            timed.time_command = Some(TimeCommand {
                keyword: prefix.keyword,
                keyword_metadata: prefix.keyword_metadata,
                prefix_words: prefix.prefix_words,
                prefix_word_metadata: prefix.prefix_word_metadata,
                command: Box::new(command),
                posix_format: prefix.posix_format,
                inverted: prefix.inverted,
            });
            timed
        })
        .collect()
}

struct TimePipelinePrefix {
    keyword: String,
    keyword_metadata: Box<WordMetadata>,
    prefix_words: Vec<String>,
    prefix_word_metadata: Vec<WordMetadata>,
    posix_format: bool,
    inverted: bool,
}

fn time_prefix_from_pipeline(pipeline: &mut PipelineCommand) -> Option<TimePipelinePrefix> {
    let first = pipeline.stages.first_mut()?;
    time_prefix_from_command(first)
}

fn fold_time_simple_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command_is_time_simple_candidate(&command) {
                return command;
            }
            let Some(prefix) = time_prefix_from_command(&mut command) else {
                return command;
            };

            let line = command.line;
            let inverted = command.inverted;
            let and_or = command.and_or.take();
            let background = command.background;
            command.inverted = false;
            command.background = false;
            let mut timed = CommandNode::new();
            timed.line = line;
            timed.inverted = inverted;
            timed.and_or = and_or;
            timed.background = background;
            timed.time_command = Some(TimeCommand {
                keyword: prefix.keyword,
                keyword_metadata: prefix.keyword_metadata,
                prefix_words: prefix.prefix_words,
                prefix_word_metadata: prefix.prefix_word_metadata,
                command: Box::new(command),
                posix_format: prefix.posix_format,
                inverted: prefix.inverted,
            });
            timed
        })
        .collect()
}

fn command_is_time_simple_candidate(command: &CommandNode) -> bool {
    command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.time_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.for_command.is_none()
        && command.arithmetic_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.conditional_command.is_none()
        && command.subshell_command.is_none()
        && command.case_command.is_none()
        && command.select_command.is_none()
        && command.function_command.is_none()
        && command.brace_group.is_none()
        && command.coproc_command.is_none()
}

fn time_prefix_from_command(command: &mut CommandNode) -> Option<TimePipelinePrefix> {
    if command.words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut posix_format = false;
    let mut inverted = false;
    let mut prefix_words = Vec::new();
    let mut prefix_word_metadata = Vec::new();
    while let Some(word) = command.words.get(index).map(String::as_str) {
        match word {
            "-p" | "--" | "!" => {
                prefix_word_metadata.push(build_word_metadata(prefix_words.len(), word, word));
                prefix_words.push(word.to_string());
                if word == "-p" {
                    posix_format = true;
                } else if word == "!" {
                    inverted = !inverted;
                }
                index += 1;
            }
            _ => break,
        }
    }
    let keyword = command.words[0].clone();
    let keyword_metadata = Box::new(build_word_metadata(0, &keyword, &keyword));
    let old_word_len = command.words.len();
    command.words = command.words[index..].to_vec();
    if command.word_kinds.len() == old_word_len {
        command.word_kinds = command.word_kinds[index..].to_vec();
    }
    if command.word_metadata.len() == old_word_len {
        command.word_metadata = command.word_metadata[index..].to_vec();
    }
    Some(TimePipelinePrefix {
        keyword,
        keyword_metadata,
        prefix_words,
        prefix_word_metadata,
        posix_format,
        inverted,
    })
}

fn try_parse_compound_start(tokens: &[Token], i: usize, state: &mut ParseState) -> Option<usize> {
    let token = &tokens[i];

    if token.kind == TokenKind::Keyword
        && token.value == "time"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((time_cmd, next_i)) = parse_time_prefixed_compound_command(tokens, i) {
            push_compound_command(state, time_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "if"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((if_cmd, next_i)) = parse_if_command(tokens, i) {
            push_compound_command(state, if_cmd);
            return Some(next_i);
        }

        // Alias expansion can introduce `then`/`elif`/`else`/`fi` after the
        // first parse attempt. Leave those non-empty conditions available to
        // the existing alias reparse path; only reject an actually empty
        // condition here.
        let condition_is_empty = tokens
            .get(i + 1)
            .map(|next| next.value == "then" || next.kind == TokenKind::Semicolon)
            .unwrap_or(true);
        let has_then_without_fi = tokens[i + 1..]
            .iter()
            .any(|candidate| candidate.value == "then")
            && !tokens[i + 1..]
                .iter()
                .any(|candidate| candidate.value == "fi");
        if !condition_is_empty && !has_then_without_fi {
            return None;
        }

        // A malformed `if` must remain a syntax error instead of falling
        // through to the simple-command parser (`if then; fi` used to be
        // accepted and silently ran the following commands).
        state.current_cmd.insert_assignment(
            "__RUBASH_PARSE_ERROR__".to_string(),
            if has_then_without_fi {
                "unexpected end of file while looking for `fi'".to_string()
            } else {
                "unexpected token `then'".to_string()
            },
        );
        // Keep the original token stream available to the executor.  Bash
        // expands aliases while parsing, so an alias such as `f=fi` can close
        // this compound command even though the first parse did not see `fi`.
        // The parse-error marker still makes genuinely malformed input fail.
        state.current_cmd.insert_assignment(
            "__RUBASH_PARSE_SOURCE__".to_string(),
            tokens[i..]
                .iter()
                .map(|token| token.raw.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let mut next_i = i + 1;
        while tokens.get(next_i).is_some() {
            next_i += 1;
            if is_keyword(tokens, next_i - 1, "fi") {
                break;
            }
        }
        state
            .ast
            .commands
            .push(std::mem::take(&mut state.current_cmd));
        return Some(next_i);
    }

    if token.kind == TokenKind::Keyword
        && matches!(token.value.as_str(), "while" | "until")
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((loop_cmd, next_i)) = parse_loop_command(tokens, i) {
            push_compound_command(state, loop_cmd);
            return Some(next_i);
        }

        state.current_cmd.insert_assignment(
            "__RUBASH_PARSE_ERROR__".to_string(),
            "unexpected token `do'".to_string(),
        );
        let mut next_i = i + 1;
        while tokens.get(next_i).is_some() {
            let is_done = is_keyword(tokens, next_i, "done");
            next_i += 1;
            if is_done {
                break;
            }
        }
        state
            .ast
            .commands
            .push(std::mem::take(&mut state.current_cmd));
        return Some(next_i);
    }

    if token.kind == TokenKind::Keyword
        && token.value == "for"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((for_cmd, next_i)) = parse_for_command(tokens, i) {
            push_compound_command(state, for_cmd);
            return Some(next_i);
        }
        let arithmetic_for_marker = tokens.get(i + 1).is_some_and(|next| next.value == "((")
            || (tokens.get(i + 1).is_some_and(|next| next.value == "(")
                && tokens.get(i + 2).is_some_and(|next| next.value == "("));
        if arithmetic_for_marker {
            let open = i + if tokens.get(i + 1).is_some_and(|t| t.value == "((") {
                2
            } else {
                3
            };
            let mut semicolons = 0u32;
            let mut depth = 0i32;
            let mut close = open;
            for j in open..tokens.len() {
                let t = &tokens[j];
                if t.value == "(("
                    || (t.value == "(" && tokens.get(j + 1).is_some_and(|n| n.value == "("))
                {
                    depth += 1;
                } else if t.value == "))"
                    || (t.value == ")" && tokens.get(j + 1).is_some_and(|n| n.value == ")"))
                {
                    if depth == 0 {
                        close = j;
                        break;
                    }
                    depth -= 1;
                } else if depth == 0 && t.kind == TokenKind::Semicolon {
                    semicolons += 1;
                }
            }
            let expr_raw: String = {
                let slice = &tokens[open..close];
                if slice.is_empty() {
                    String::new()
                } else {
                    let mut parts = Vec::with_capacity(slice.len());
                    for (idx, t) in slice.iter().enumerate() {
                        if idx == 0 {
                            parts.push(t.raw.clone());
                        } else {
                            let prev = &slice[idx - 1];
                            let prev_end = prev.column + prev.raw.len();
                            if t.column > prev_end {
                                parts.push(" ".to_string());
                            }
                            parts.push(t.raw.clone());
                        }
                    }
                    parts.concat()
                }
            };
            let error_msg = if semicolons < 2 {
                "syntax error: arithmetic expression required".to_string()
            } else {
                "syntax error: `;' unexpected".to_string()
            };
            state
                .current_cmd
                .insert_assignment("__RUBASH_PARSE_ERROR__".to_string(), error_msg);
            state.current_cmd.insert_assignment(
                "__RUBASH_PARSE_SOURCE__".to_string(),
                format!("(( {} ))", expr_raw.trim()),
            );
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(i + 1);
        }
    }

    if ((token.kind == TokenKind::Word)
        || (token.kind == TokenKind::Keyword && token.value == "function")
        || (token.kind == TokenKind::RedirectIn && matches!(token.value.as_str(), "<" | ">"))
        || (token.kind == TokenKind::Keyword && token.value == "!"))
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((function_cmd, next_i)) = parse_function_command(tokens, i) {
            push_compound_command(state, function_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "case"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((case_cmd, next_i)) = parse_case_command(tokens, i) {
            push_compound_command(state, case_cmd);
            return Some(next_i);
        }
        return Some(push_parse_error_until(
            state,
            tokens,
            i,
            "esac",
            case_parse_error_message(tokens, i),
        ));
    }

    if token.kind == TokenKind::Keyword
        && token.value == "select"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((select_cmd, next_i)) = parse_select_command(tokens, i) {
            push_compound_command(state, select_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "coproc"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((coproc_cmd, next_i)) = parse_coproc_command(tokens, i) {
            push_compound_command(state, coproc_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            push_compound_command(state, arith_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd) && token.value == "[[" {
        if let Some((conditional_cmd, next_i)) = parse_conditional_command(tokens, i) {
            push_compound_command(state, conditional_cmd);
            return Some(next_i);
        }
        return Some(push_parse_error_until(
            state,
            tokens,
            i,
            "]]",
            "unexpected EOF while looking for `]]'",
        ));
    }

    if command_allows_compound_start(&state.current_cmd)
        && token.kind == TokenKind::Keyword
        && token.value == "("
    {
        if let Some((subshell_cmd, next_i)) = parse_subshell_command(tokens, i) {
            push_compound_command(state, subshell_cmd);
            return Some(next_i);
        }
        return Some(push_parse_error_until(
            state,
            tokens,
            i,
            ")",
            "unexpected end of file",
        ));
    }

    if command_accepts_embedded_arithmetic_command(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            note_command_line(&mut state.current_cmd, token);
            state.current_cmd.words.extend(arith_cmd.words);
            state.current_cmd.and_or = arith_cmd.and_or;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd) {
        if let Some((brace_cmd, next_i)) = parse_brace_group_command(tokens, i) {
            push_compound_command(state, brace_cmd);
            return Some(next_i);
        }
        if token.value == "{"
            || (token.value.starts_with('{')
                && token.value.contains(';')
                && !token.value.contains('}'))
        {
            return Some(push_parse_error_until(
                state,
                tokens,
                i,
                "}",
                "unexpected end of file",
            ));
        }
    }

    None
}

fn command_allows_compound_start(command: &CommandNode) -> bool {
    command_is_empty(command) || command_is_pending_inversion(command)
}

fn push_parse_error_until(
    state: &mut ParseState,
    tokens: &[Token],
    start: usize,
    terminator: &str,
    message: &str,
) -> usize {
    state
        .current_cmd
        .insert_assignment("__RUBASH_PARSE_ERROR__".to_string(), message.to_string());
    let mut next_i = start + 1;
    while tokens.get(next_i).is_some() {
        let is_terminator = is_keyword(tokens, next_i, terminator);
        next_i += 1;
        if is_terminator {
            break;
        }
    }
    state
        .ast
        .commands
        .push(std::mem::take(&mut state.current_cmd));
    next_i
}

fn command_is_pending_inversion(command: &CommandNode) -> bool {
    if !command.inverted {
        return false;
    }
    let mut without_inversion = command.clone();
    without_inversion.inverted = false;
    command_is_empty(&without_inversion)
}

fn push_compound_command(state: &mut ParseState, mut command: CommandNode) {
    if command_is_pending_inversion(&state.current_cmd) {
        command.inverted = !command.inverted;
        command.line = command.line.or(state.current_cmd.line);
    }
    state.ast.commands.push(command);
    state.current_cmd = CommandNode::new();
}

pub(super) fn parse_time_prefixed_compound_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    tokens.get(start)?;
    let mut posix_format = false;
    let mut inverted = false;
    let mut prefix_words = Vec::new();
    let mut prefix_word_metadata = Vec::new();
    let mut i = start + 1;
    while tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "-p" | "--" | "!"))
    {
        prefix_word_metadata.push(build_word_metadata(
            prefix_words.len(),
            &tokens[i].value,
            &tokens[i].raw,
        ));
        prefix_words.push(tokens[i].value.clone());
        match tokens[i].value.as_str() {
            "-p" => posix_format = true,
            "!" => inverted = !inverted,
            _ => {}
        }
        i += 1;
    }

    let (mut command, next_i) = if is_keyword(tokens, i, "for") {
        parse_for_command(tokens, i)?
    } else if is_keyword(tokens, i, "if") {
        parse_if_command(tokens, i)?
    } else if tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "while" | "until"))
    {
        parse_loop_command(tokens, i)?
    } else if is_keyword(tokens, i, "case") {
        parse_case_command(tokens, i)?
    } else if is_keyword(tokens, i, "select") {
        parse_select_command(tokens, i)?
    } else if is_keyword(tokens, i, "coproc") {
        parse_coproc_command(tokens, i)?
    } else if let Some(parsed) = parse_function_command(tokens, i) {
        parsed
    } else if tokens.get(i).is_some_and(|token| token.value == "[[") {
        parse_conditional_command(tokens, i)?
    } else if is_keyword(tokens, i, "{")
        || tokens.get(i).is_some_and(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
    {
        parse_brace_group_command(tokens, i)?
    } else if let Some(parsed) = parse_arithmetic_command(tokens, i) {
        parsed
    } else if is_keyword(tokens, i, "(") {
        parse_subshell_command(tokens, i)?
    } else {
        return None;
    };

    let pipe = command.pipe.take();
    let and_or = command.and_or.take();
    let background = command.background;
    command.background = false;
    let mut timed = CommandNode::new();
    timed.line = tokens.get(start).map(|token| token.position);
    timed.pipe = pipe;
    timed.and_or = and_or;
    timed.background = background;
    timed.time_command = Some(TimeCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata: Box::new(build_word_metadata(
            0,
            &tokens[start].value,
            &tokens[start].raw,
        )),
        prefix_words,
        prefix_word_metadata,
        command: Box::new(command),
        posix_format,
        inverted,
    });
    Some((timed, next_i))
}

pub(super) fn parse_time_prefixed_shell_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    if !time_prefixed_shell_command_allows_simple_pipeline(tokens, start) {
        return None;
    }

    let end = time_prefixed_shell_command_end(tokens, start + 1);

    let mut commands = parse(&tokens[start..end]).commands;
    if commands.len() != 1 {
        return None;
    }

    Some((commands.remove(0), end))
}

fn time_prefixed_shell_command_end(tokens: &[Token], mut index: usize) -> usize {
    let mut stack = Vec::new();
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;

    while let Some(token) = tokens.get(index) {
        if stack.is_empty()
            && paren_depth == 0
            && brace_depth == 0
            && token.kind == TokenKind::Semicolon
            && previous_significant_token_is_pipe(tokens, index)
        {
            index += 1;
            continue;
        }

        if stack.is_empty()
            && paren_depth == 0
            && brace_depth == 0
            && matches!(
                token.kind,
                TokenKind::Semicolon | TokenKind::And | TokenKind::Or | TokenKind::Background
            )
        {
            break;
        }

        update_compound_boundary_stack(tokens, index, &mut stack);
        if stack.last().copied() != Some("esac") && command_boundary_keyword_allowed(tokens, index)
        {
            if is_keyword(tokens, index, "(") {
                paren_depth += 1;
            } else if is_keyword(tokens, index, ")") {
                paren_depth = paren_depth.saturating_sub(1);
            } else if is_keyword(tokens, index, "{") {
                brace_depth += 1;
            } else if is_keyword(tokens, index, "}") {
                brace_depth = brace_depth.saturating_sub(1);
            }
        }

        index += 1;
    }

    index
}

fn previous_significant_token_is_pipe(tokens: &[Token], index: usize) -> bool {
    let mut previous = index;
    while let Some(next) = previous.checked_sub(1) {
        previous = next;
        let token = &tokens[previous];
        if token.kind == TokenKind::Semicolon {
            continue;
        }
        return matches!(token.kind, TokenKind::Pipe | TokenKind::PipeErr);
    }
    false
}

pub(super) fn time_prefixed_shell_command_allows_simple_pipeline(
    tokens: &[Token],
    start: usize,
) -> bool {
    if !is_keyword(tokens, start, "time") {
        return false;
    }

    let mut index = start + 1;
    while tokens
        .get(index)
        .is_some_and(|token| matches!(token.value.as_str(), "-p" | "--" | "!"))
    {
        index += 1;
    }

    let Some(token) = tokens.get(index) else {
        return true;
    };
    if matches!(
        token.kind,
        TokenKind::Semicolon | TokenKind::And | TokenKind::Or | TokenKind::Background
    ) {
        return true;
    }

    !time_prefixed_shell_command_starts_with_compound(tokens, index)
}

fn time_prefixed_shell_command_starts_with_compound(tokens: &[Token], index: usize) -> bool {
    if parse_function_command(tokens, index).is_some() {
        return true;
    }

    if tokens.get(index).is_some_and(|token| {
        matches!(
            token.value.as_str(),
            "for" | "case" | "select" | "coproc" | "if" | "while" | "until" | "[[" | "function"
        ) || token.value.starts_with("((")
    }) {
        return true;
    }

    if is_keyword(tokens, index, "{") || is_keyword(tokens, index, "(") {
        return true;
    }

    if tokens.get(index).is_some_and(|token| {
        token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
    }) {
        return true;
    }

    tokens.get(index).is_some_and(|token| {
        token.kind == TokenKind::Word
            && tokens.get(index + 1).is_some_and(|next| next.value == "(")
            && tokens.get(index + 2).is_some_and(|next| next.value == ")")
    })
}

#[cfg(test)]
mod stray_close_tests {
    use super::*;

    fn marker_source(ast: &Ast) -> String {
        ast.commands
            .iter()
            .find_map(|command| {
                command
                    .get_assignment("__RUBASH_PARSE_SOURCE__")
                    .map(|value| value.clone())
            })
            .unwrap_or_default()
    }

    #[test]
    fn eval_style_verbatim_line_echo_preserves_original_spacing() {
        // The eval string: one line, caller at script line 199 -> offset 198.
        let source_text = String::from("case esac in esac) ;; *) echo \"x\";; esac");
        let tokens = crate::lexer::tokenize(&source_text);
        let mut tokens = tokens;
        for token in tokens.iter_mut() {
            token.position += 198;
        }
        let ast = parse_with_options(
            &tokens,
            ParseLoopOptions {
                stray_close_is_error: true,
                source_text: Some(source_text.clone()),
                source_line_offset: 198,
            },
        );
        assert_eq!(marker_source(&ast), source_text);
    }

    #[test]
    fn without_source_text_falls_back_to_token_join() {
        let source_text = ") echo never ;; esac";
        let tokens = crate::lexer::tokenize(source_text);
        let ast = parse_with_options(
            &tokens,
            ParseLoopOptions {
                stray_close_is_error: true,
                ..ParseLoopOptions::default()
            },
        );
        assert_eq!(marker_source(&ast), ") echo never ;; esac");
    }
}
