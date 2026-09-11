use super::*;
use crate::executor::fd_table::FdEntry;

/// Saved descriptor state for a numbered fd opened by a compound command's
/// input redirection (see open_compound_numbered_input_redirects).
struct SavedNumberedFd {
    fd: u32,
    entry: Option<FdEntry>,
    fd_stdin: Option<String>,
    fd_stdin_offset: Option<String>,
    fd_dynamic: Option<String>,
    fd_closed: Option<String>,
}

impl Executor {
    pub(crate) fn with_command_input_redirects<T>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        // GNU redir.c do_redirection_internal (redir.c:767-955) applies a
        // compound command's redirections before the command runs, and
        // execute_cmd.c undoes them when the command finishes (RX_UNDOABLE /
        // redir.c:949-955). A numbered input redirection such as
        // `while read -ru3 x; do :; done 3< <(echo x)` (redir10.sub,
        // procsub.tests bug()) must therefore keep fd 3 open for the whole
        // compound command - condition and body alike - and restore the
        // previous descriptor state afterwards.
        let dynamic_names = cmd
            .redirects
            .iter()
            .filter_map(|redirect| redirect.fd_var.clone())
            .collect::<Vec<_>>();
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some() {
                let _ = self.execute_dynamic_fd_var_redirect(redirect, false)?;
            }
        }
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        let result = self.with_command_input_redirects_inner(cmd, execute);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        for name in dynamic_names {
            self.close_dynamic_fd(&name)?;
        }
        result
    }

    fn with_command_input_redirects_inner<T>(
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

    /// Opens the numbered input redirections of a compound command in the
    /// fd table for the command's whole duration. GNU applies compound
    /// redirections left to right before the command runs (redir.c
    /// do_redirection_internal:767); procsub targets feed
    /// FdReadEndpoint-backed text exactly like the persistent fd-var path
    /// (trap_exec.rs execute_dynamic_fd_var_redirect).
    fn open_compound_numbered_input_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Vec<SavedNumberedFd>, ExecuteError> {
        let mut saved: Vec<SavedNumberedFd> = Vec::new();
        for redirect in &cmd.redirects {
            let Some(fd) = redirect.fd else {
                continue;
            };
            if fd == 0 || redirect.fd_var.is_some() {
                continue;
            }
            if !matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
            ) {
                continue;
            }
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                continue;
            }
            let input = if is_null_device(&target) {
                Vec::new()
            } else if let Some(source) = target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                let Some(output) = self.process_substitution_output(source) else {
                    continue;
                };
                output.into_bytes()
            } else {
                let path = shell_path_to_windows(&target, &self.env_vars);
                if redirect.append {
                    // Mirrors the exec path (trap_exec.rs): [N]<> opens the
                    // file for reading and writing (redir.c r_input_output,
                    // O_RDWR).
                    let _ = OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(&path)
                        .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                }
                std::fs::read(&path)
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?
            };
            if !saved.iter().any(|saved| saved.fd == fd) {
                saved.push(SavedNumberedFd {
                    fd,
                    entry: self.fd_table.entries.get(&fd).cloned(),
                    fd_stdin: self.env_vars.get(&fd_stdin_key(fd)).cloned(),
                    fd_stdin_offset: self.env_vars.get(&fd_stdin_offset_key(fd)).cloned(),
                    fd_dynamic: self.env_vars.get(&fd_dynamic_input_key(fd)).cloned(),
                    fd_closed: self.env_vars.get(&fd_closed_key(fd)).cloned(),
                });
            }
            self.set_fd_input_bytes(fd, input, true);
            if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                self.set_fd_output_file(fd, target, true);
            }
        }
        Ok(saved)
    }

    fn restore_compound_numbered_input_redirects(&mut self, saved: Vec<SavedNumberedFd>) {
        for saved in saved {
            match saved.entry {
                Some(entry) => {
                    self.fd_table.entries.insert(saved.fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&saved.fd);
                }
            }
            restore_optional_env_var(
                &mut self.env_vars,
                &fd_stdin_key(saved.fd),
                saved.fd_stdin,
            );
            restore_optional_env_var(
                &mut self.env_vars,
                &fd_stdin_offset_key(saved.fd),
                saved.fd_stdin_offset,
            );
            restore_optional_env_var(
                &mut self.env_vars,
                &fd_dynamic_input_key(saved.fd),
                saved.fd_dynamic,
            );
            restore_optional_env_var(
                &mut self.env_vars,
                &fd_closed_key(saved.fd),
                saved.fd_closed,
            );
        }
    }

    pub(in crate::executor) fn command_input_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        if let Some(input) = self.loop_redirect_input(cmd) {
            return Some(input);
        }

        if let Some(here_string) = cmd.here_string.clone() {
            // Here-string content already had quote removal applied by the
            // parser; expand only substitutions with quotes-as-data semantics.
            return Some(self.expand_here_string_mut(&here_string));
        }

        if let Some(heredoc) = cmd.heredoc.clone() {
            return Some(self.expand_heredoc_body_mut(&heredoc));
        }

        cmd.heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_none())
            .and_then(|redirect| redirect.body.as_deref())
            .map(|body| self.expand_heredoc_body_mut(body))
    }

    /// Expands an unquoted heredoc body like Bash: parameter, command and
    /// arithmetic expansions run in the context of the receiving command.
    /// A dedicated quoted-heredoc marker keeps the body literal without sharing
    /// the compound-assignment transport protocol.
    pub(in crate::executor) fn expand_heredoc_body_mut(&mut self, body: &str) -> String {
        let quoted = body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER);
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        if quoted {
            return body.to_string();
        }
        // comsub-eof6: `read foo <<EOF` with body `$(seq 10` (missing `)`)
        // must not expand to `1`; GNU reports `command substitution:
        // unexpected EOF while looking for matching `)'` and leaves foo empty.
        // Detect unclosed `$(` in the raw body before expansion.
        if crate::lexer::has_unclosed_command_substitution(body) {
            eprintln!(
                "{}command substitution: line 1: unexpected EOF while looking for matching `)'",
                self.diagnostic_prefix()
            );
            return String::new();
        }
        let prepared = prepare_unquoted_heredoc_expansion(body);
        self.expand_embedded_parameters_mut_with_context(
            &prepared,
            SubstitutionQuoteContext::HereDocument,
        )
    }

    pub(in crate::executor) fn expand_heredoc_body_readback(
        &self,
        body: &str,
    ) -> SubstitutionOutput {
        let expanded = self.expand_heredoc_body(body);
        SubstitutionOutput::readback(
            expanded.into_bytes(),
            0,
            SubstitutionQuoteContext::HereDocument,
        )
    }

    pub(in crate::executor) fn expand_heredoc_body(&self, body: &str) -> String {
        let quoted = body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER);
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        if quoted {
            return body.to_string();
        }
        if crate::lexer::has_unclosed_command_substitution(body) {
            eprintln!(
                "{}command substitution: line 1: unexpected EOF while looking for matching `)'",
                self.diagnostic_prefix()
            );
            return String::new();
        }
        let expanded =
            self.expand_embedded_parameters_for_heredoc(&prepare_unquoted_heredoc_expansion(body));
        decode_command_substitution_payload(&restore_command_substitution_output(&expanded))
    }
}

fn prepare_unquoted_heredoc_expansion(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let mut command_depth = 0usize;
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        if ch == '$' && !single && chars.peek() == Some(&'(') {
            chars.next();
            output.push('$');
            output.push('(');
            command_depth += 1;
            continue;
        }

        if command_depth > 0 {
            match ch {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '(' if !single && !double => command_depth += 1,
                ')' if !single && !double => command_depth = command_depth.saturating_sub(1),
                _ => {}
            }
        }

        if ch != '\\' {
            output.push(ch);
            continue;
        }

        if command_depth > 0 && single {
            output.push('\\');
            continue;
        }

        let mut slash_count = 1usize;
        while chars.peek() == Some(&'\\') {
            chars.next();
            slash_count += 1;
        }

        if matches!(chars.peek(), Some('\n') | Some('\r')) {
            for _ in 0..(slash_count / 2) {
                output.push('\\');
            }
            if slash_count % 2 == 1 {
                if chars.peek() == Some(&'\r') {
                    chars.next();
                }
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            continue;
        }

        for _ in 0..(slash_count / 2) {
            output.push('\x14');
        }
        if slash_count % 2 == 0 {
            continue;
        }

        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                output.push('\x1f');
            }
            Some('`') => {
                chars.next();
                output.push('\x1a');
            }
            Some('\\') => unreachable!("backslash runs are consumed above"),
            _ => output.push('\\'),
        }
    }
    output
}

#[allow(dead_code)]
fn strip_heredoc_body(body: &str) -> String {
    strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
}
