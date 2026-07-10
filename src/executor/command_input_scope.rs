use super::*;

impl Executor {
    pub(crate) fn with_command_input_redirects<T>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let Some(input) = self.command_input_redirect(cmd) else {
            return execute(self);
        };

        let old_function_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_function_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        let result = execute(self);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_function_stdin);
        restore_optional_env_var(
            &mut self.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_function_stdin_offset,
        );
        result
    }

    fn command_input_redirect(&mut self, cmd: &CommandNode) -> Option<String> {
        if let Some(input) = self.loop_redirect_input(cmd) {
            return Some(input);
        }

        if let Some(here_string) = &cmd.here_string {
            return Some(self.expand_word(here_string));
        }

        cmd.heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_none())
            .and_then(|redirect| redirect.body.as_deref())
            .map(strip_heredoc_body)
    }
}

fn strip_heredoc_body(body: &str) -> String {
    strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
}
