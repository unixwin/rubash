use super::*;

impl Executor {
    pub(in crate::executor) fn find_matching_done_command(
        &self,
        ast: &Ast,
        start: usize,
        initial_depth: usize,
    ) -> Option<usize> {
        let mut nested_loop_depth = initial_depth;
        for index in start..ast.commands.len() {
            let command = &ast.commands[index];
            if self.command_starts_alias_loop(command) {
                nested_loop_depth += 1;
                continue;
            }
            if control_word(command) == Some("done") {
                if nested_loop_depth == 0 {
                    return Some(index);
                }
                nested_loop_depth -= 1;
            }
        }
        None
    }

    pub(in crate::executor) fn embedded_do_loop_depth(&self, command: &CommandNode) -> usize {
        if control_word(command) == Some("do")
            && command.words.len() > 1
            && self.words_start_alias_loop(&command.words[1..])
        {
            1
        } else {
            0
        }
    }

    fn command_starts_alias_loop(&self, command: &CommandNode) -> bool {
        self.words_start_alias_loop(&command.words)
    }

    fn words_start_alias_loop(&self, words: &[String]) -> bool {
        let words = if self.alias_expansion_enabled() {
            self.expand_aliases(words)
        } else {
            words.to_vec()
        };
        matches!(
            words.first().map(String::as_str),
            Some("for" | "while" | "until" | "select")
        )
    }
}

pub(in crate::executor) fn control_word(command: &CommandNode) -> Option<&str> {
    if let Some(word) = command.words.first() {
        return Some(word.as_str());
    }
    command
        .get_assignment("__RUBASH_PARSE_ERROR__")
        .and_then(|message| message.split_once("unexpected token `"))
        .map(|(_, token)| token.trim_end_matches(['`', '\'']))
}
