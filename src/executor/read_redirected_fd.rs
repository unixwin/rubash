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
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        // GNU redir.c applies the command's own redirections before the
        // builtin runs, so `read -u 3 x 3< file` reads the file. rubash
        // parks a numbered input redirect in cmd.redirects (only the
        // fd-0 one lands in redirect_in); mapfile already consults both
        // stores (mapfile_fd_is_available), read consulted only
        // redirect_in and reported the fd invalid (rubash#1296). The
        // last matching redirect in source order is the effective one.
        let redirect = cmd
            .redirect_in
            .as_deref()
            .filter(|redirect| redirect.fd == Some(fd))
            .or_else(|| {
                cmd.redirects.iter().rev().find(|redirect| {
                    redirect.fd == Some(fd)
                        && matches!(
                            redirect.kind,
                            crate::parser::RedirectKind::Input
                                | crate::parser::RedirectKind::ReadWrite
                        )
                })
            })?;

        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            // rubash#1952: stream first — serve the record from the
            // spawned body's live pipe through a scratch fd (GNU
            // subst.c:6362 process_substitute forks the body; the
            // record reader observes real arrival). Collection below
            // is the spawn-failure fallback.
            if let Some(scratch) = self.allocate_dynamic_fd() {
                if let Some(file_fd) = self.spawn_input_process_substitution_child(source, cmd.line)
                {
                    self.fd_table
                        .open_input(scratch, FdReadEndpoint::File(file_fd), true);
                    let line = self.fd_table.read_text(
                        scratch,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                    self.fd_table.entries.remove(&scratch);
                    return Some(match line {
                        Some(line) => trim_read_input(
                            line,
                            delimiter,
                            char_limit,
                            exact_char_limit,
                            escape_aware,
                            raw,
                        ),
                        None => {
                            if !self.fd_table.read_timed_out {
                                self.read_eof_no_delimiter = true;
                            }
                            String::new()
                        }
                    });
                }
            }
            let output = self.process_substitution_output(source)?;
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &output,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                output,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }

        let target = self.expand_redirect_target(redirect);
        if is_closed_redirect_target(&target) {
            return None;
        }
        // niubash#177: same operand resolution as open_input_redirect — a
        // virtual-root input operand may exist only under its `.exe`
        // spelling on disk.
        let path = self.session_path(crate::executor::path::windows_operand_file_path(
            shell_path_to_windows(&target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        if redirect.append {
            let _ = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path);
        }
        // rubash#1903: the read builtin's own slurp of its redirect
        // target must go through the rendezvous for a WinuxCmd fifo
        // marker — plain-reading the target hands `read` the 7 marker
        // bytes and strands the writer (the #2014 append pin's
        // `read x < g` face).
        #[cfg(windows)]
        if let Some(result) =
            crate::winuxcmd_fifo::slurp_marker_bytes(&path, &self.shell_state.env_vars)
        {
            let input = match result {
                Ok(bytes) => crate::executor::substitution_metadata::bytes_to_shell_text(&bytes),
                Err(_) => return None,
            };
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }
        // Same byte-preserving contract as read_io.rs for the shell-owned
        // descriptor path (GNU redir.c feeds raw bytes to the read owner).
        let input = match crate::executor::substitution_metadata::read_shell_input_file(&path) {
            Ok(text) => text,
            Err(_) => return None,
        };
        self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
            &input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        );
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ))
    }

    pub(in crate::executor) fn read_heredoc_fd_input(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
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
                self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                    &input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
        }
        if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }
        let input = if let Some(pre) = preexpanded_stdin_body(body) {
            pre.to_string()
        } else {
            strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
        };
        self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
            &input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        );
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ))
    }
}
