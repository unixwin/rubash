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
            if command.words.first().map(String::as_str) == Some("done") {
                if nested_loop_depth == 0 {
                    return Some(index);
                }
                nested_loop_depth -= 1;
            }
        }
        None
    }

    pub(in crate::executor) fn embedded_do_loop_depth(&self, command: &CommandNode) -> usize {
        if command.words.first().map(String::as_str) == Some("do")
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
