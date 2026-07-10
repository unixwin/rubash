use crate::executor::Executor;
use crate::parser::{Ast, CommandNode};

pub(super) fn control_words(
    executor: &Executor,
    command: &CommandNode,
    controls: &[&str],
) -> Option<Vec<String>> {
    let words = expanded_command_words(executor, &command.words);
    let first = words.first()?;
    controls.contains(&first.as_str()).then_some(words)
}

pub(super) fn find_word_command(
    executor: &Executor,
    ast: &Ast,
    start: usize,
    word: &str,
) -> Option<usize> {
    (start..ast.commands.len())
        .find(|index| command_first_word_is(executor, &ast.commands[*index], word))
}

pub(super) fn find_matching_fi(executor: &Executor, ast: &Ast, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..ast.commands.len() {
        if command_starts_if(executor, &ast.commands[index]) {
            depth += 1;
            continue;
        }
        if command_first_word_is(executor, &ast.commands[index], "fi") {
            if depth == 0 {
                return Some(index);
            }
            depth = depth.saturating_sub(1);
        }
    }
    None
}

pub(super) fn find_if_branch_command(
    executor: &Executor,
    ast: &Ast,
    start: usize,
    end: usize,
    word: &str,
) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..end {
        if command_starts_if(executor, &ast.commands[index]) {
            depth += 1;
            continue;
        }
        if command_first_word_is(executor, &ast.commands[index], "fi") {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && command_first_word_is(executor, &ast.commands[index], word) {
            return Some(index);
        }
    }
    None
}

pub(super) fn command_tail_starts_if(
    executor: &Executor,
    command: &CommandNode,
    start: usize,
) -> bool {
    command_tail_first_word_is(executor, command, start, "if")
}

fn command_starts_if(executor: &Executor, command: &CommandNode) -> bool {
    if command_first_word_is(executor, command, "if") {
        return true;
    }
    let Some(first) = command_control_word(executor, &command.words) else {
        return false;
    };
    matches!(first.as_str(), "then" | "else" | "do")
        && command_tail_first_word_is(executor, command, 1, "if")
}

fn command_first_word_is(executor: &Executor, command: &CommandNode, word: &str) -> bool {
    command_control_word(executor, &command.words).as_deref() == Some(word)
}

fn command_tail_first_word_is(
    executor: &Executor,
    command: &CommandNode,
    start: usize,
    word: &str,
) -> bool {
    command_control_word(executor, &command.words[start..]).as_deref() == Some(word)
}

fn command_control_word(executor: &Executor, words: &[String]) -> Option<String> {
    expanded_command_words(executor, words).into_iter().next()
}

fn expanded_command_words(executor: &Executor, words: &[String]) -> Vec<String> {
    if executor.alias_expansion_enabled() {
        executor.expand_aliases(words)
    } else {
        words.to_vec()
    }
}
