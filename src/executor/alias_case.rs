use super::*;

pub(super) fn command_contains_word(command: &CommandNode, word: &str) -> bool {
    command.words.iter().any(|candidate| candidate == word)
}

pub(super) struct AliasCaseBoundary<'a> {
    pub(super) command: &'a CommandNode,
    pub(super) command_index: usize,
    pub(super) next_word_index: usize,
    pub(super) terminator: CaseTerminator,
    pub(super) terminator_text: Option<String>,
    pub(super) ended_case: bool,
}

pub(super) struct AliasCasePatterns<'a> {
    pub(super) patterns: Vec<String>,
    pub(super) command: &'a CommandNode,
    pub(super) command_index: usize,
    pub(super) words: &'a [String],
    pub(super) body_start: usize,
}

pub(super) fn collect_alias_case_patterns<'a>(
    ast: &'a Ast,
    command: &'a CommandNode,
    command_index: usize,
    words: &'a [String],
    pattern_index: usize,
) -> Option<AliasCasePatterns<'a>> {
    if let Some(patterns) =
        collect_alias_case_extglob_patterns(ast, command, command_index, words, pattern_index)
    {
        return Some(patterns);
    }

    let mut patterns = vec![words.get(pattern_index)?.clone()];
    let mut current_command = command;
    let mut current_command_index = command_index;
    let mut current_words = words;
    let mut body_start = pattern_index + 1;

    while current_command.pipe.is_some() && body_start >= current_words.len() {
        let next_index = current_command_index + 1;
        let next_command = ast.commands.get(next_index)?;
        patterns.push(next_command.words.first()?.clone());
        current_command = next_command;
        current_command_index = next_index;
        current_words = &next_command.words;
        body_start = 1;
    }

    Some(AliasCasePatterns {
        patterns,
        command: current_command,
        command_index: current_command_index,
        words: current_words,
        body_start,
    })
}

fn collect_alias_case_extglob_patterns<'a>(
    ast: &'a Ast,
    command: &'a CommandNode,
    command_index: usize,
    words: &'a [String],
    pattern_index: usize,
) -> Option<AliasCasePatterns<'a>> {
    let prefix = words.get(pattern_index)?;
    if !alias_case_extglob_prefix(prefix)
        || command.pipe.is_none()
        || pattern_index + 1 >= words.len()
    {
        return None;
    }

    let mut pattern = format!("{prefix}(");
    pattern.push_str(&words[pattern_index + 1..].join(""));

    let mut current_command_index = command_index;
    loop {
        let next_index = current_command_index + 1;
        let next_command = ast.commands.get(next_index)?;
        pattern.push('|');

        if next_command.pipe.is_some() {
            pattern.push_str(&next_command.words.join(""));
            current_command_index = next_index;
            continue;
        }

        pattern.push_str(next_command.words.first()?);
        pattern.push(')');
        return Some(AliasCasePatterns {
            patterns: vec![pattern],
            command: next_command,
            command_index: next_index,
            words: &next_command.words,
            body_start: 1,
        });
    }
}

fn alias_case_extglob_prefix(word: &str) -> bool {
    word.chars()
        .last()
        .is_some_and(|ch| matches!(ch, '@' | '*' | '+' | '?' | '!'))
}

pub(super) fn collect_alias_case_body<'a>(
    ast: &'a Ast,
    command: &'a CommandNode,
    command_index: usize,
    words: &[String],
    start: usize,
    body: &mut Vec<CommandNode>,
) -> Option<AliasCaseBoundary<'a>> {
    if let Some(boundary) = case_boundary_index_in_words(&words[start..]) {
        let boundary = start + boundary;
        push_case_body_words(command, &words[start..boundary], body);
        return Some(alias_case_boundary(command, command_index, words, boundary));
    }
    push_case_body_words(command, &words[start..], body);

    for next_index in command_index + 1..ast.commands.len() {
        let next_command = ast.commands.get(next_index)?;
        if let Some(boundary) = case_boundary_word_index(next_command) {
            push_case_body_words(next_command, &next_command.words[..boundary], body);
            return Some(alias_case_boundary(
                next_command,
                next_index,
                &next_command.words,
                boundary,
            ));
        }
        body.push(next_command.clone());
    }

    None
}

fn alias_case_boundary<'a>(
    command: &'a CommandNode,
    command_index: usize,
    words: &[String],
    boundary: usize,
) -> AliasCaseBoundary<'a> {
    let terminator = case_terminator_from_word(words.get(boundary).map(String::as_str));
    let terminator_text = words
        .get(boundary)
        .filter(|word| matches!(word.as_str(), ";;" | ";&" | ";;&"))
        .cloned();
    let mut next_word_index = boundary + 1;
    let ended_case = words.get(boundary).is_some_and(|word| word == "esac")
        || words
            .get(next_word_index)
            .is_some_and(|word| word == "esac");
    if ended_case
        && words
            .get(next_word_index)
            .is_some_and(|word| word == "esac")
    {
        next_word_index += 1;
    }
    AliasCaseBoundary {
        command,
        command_index,
        next_word_index,
        terminator,
        terminator_text,
        ended_case,
    }
}

fn case_terminator_from_word(word: Option<&str>) -> CaseTerminator {
    match word {
        Some(";&") => CaseTerminator::FallThrough,
        Some(";;&") => CaseTerminator::TestNext,
        _ => CaseTerminator::Break,
    }
}

pub(super) fn command_words_text(command: &CommandNode) -> String {
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
