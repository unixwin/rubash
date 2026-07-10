use super::*;

pub(super) fn command_contains_word(command: &CommandNode, word: &str) -> bool {
    command.words.iter().any(|candidate| candidate == word)
}

pub(super) struct AliasCaseBoundary<'a> {
    pub(super) command: &'a CommandNode,
    pub(super) command_index: usize,
    pub(super) next_word_index: usize,
    pub(super) terminator: CaseTerminator,
    pub(super) ended_case: bool,
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
