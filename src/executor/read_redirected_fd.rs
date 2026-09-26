use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    pub(in crate::executor) fn read_redirected_fd(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        let redirect = cmd.redirect_in.as_ref()?;
        if redirect.fd != Some(fd) {
            return None;
        }

        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            let output = self.process_substitution_output(source)?;
            return Some(trim_read_input(
                output,
                delimiter,
                char_limit,
                exact_char_limit,
            ));
        }

        let target = self.expand_redirect_target(redirect);
        if is_closed_redirect_target(&target) {
            return None;
        }
        let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
        if redirect.append {
            let _ = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path);
        }
        // Same byte-preserving contract as read_io.rs for the shell-owned
        // descriptor path (GNU redir.c feeds raw bytes to the read owner).
        let input = match crate::executor::substitution_metadata::read_shell_input_file(&path) {
            Ok(text) => text,
            Err(_) => return None,
        };
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(in crate::executor) fn read_heredoc_fd_input(
        &self,
        cmd: &CommandNode,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;
        let body = redirect.body.as_deref()?;
        if let Some(carrier) = &redirect.body_carrier {
            if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                let mut input = text.clone();
                input.push('\n');
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                ));
            }
        }
        if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
            ));
        }
        let input = if let Some(pre) = preexpanded_stdin_body(body) {
            pre.to_string()
        } else {
            strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
        };
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }
}
