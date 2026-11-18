use super::types::{FUNCTION_STDIN, FUNCTION_STDIN_OFFSET, INHERIT_PROCESS_STDIN};
use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    pub(in crate::executor) fn parse_mapfile_usize(
        &self,
        command_name: &str,
        value: &str,
        diagnostic: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<usize, i32> {
        value.parse::<usize>().map_err(|_| {
            let _ = writeln!(
                stderr,
                "{}{command_name}: {value}: {diagnostic}",
                self.diagnostic_prefix()
            );
            1
        })
    }

    pub(in crate::executor) fn parse_mapfile_callback_quantum(
        &self,
        command_name: &str,
        value: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<usize, i32> {
        let quantum =
            self.parse_mapfile_usize(command_name, value, "invalid callback quantum", stderr)?;
        if quantum == 0 {
            let _ = writeln!(
                stderr,
                "{}{command_name}: {value}: invalid callback quantum",
                self.diagnostic_prefix()
            );
            return Err(1);
        }
        Ok(quantum)
    }

    pub(in crate::executor) fn parse_mapfile_fd(
        &self,
        command_name: &str,
        value: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<u32, i32> {
        value
            .parse::<i32>()
            .ok()
            .and_then(|fd| u32::try_from(fd).ok())
            .ok_or_else(|| {
                let _ = writeln!(
                    stderr,
                    "{}{command_name}: {value}: invalid file descriptor specification",
                    self.diagnostic_prefix()
                );
                1
            })
    }

    pub(in crate::executor) fn mapfile_bad_file_descriptor(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        fd: u32,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: {fd}: invalid file descriptor: Bad file descriptor",
            self.diagnostic_prefix()
        );
        self.finish_mapfile_error(cmd, stderr, 1)
    }

    pub(in crate::executor) fn mapfile_invalid_identifier(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        name: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: `{name}': not a valid identifier",
            self.diagnostic_prefix()
        );
        self.finish_mapfile_error(cmd, stderr, 1)
    }

    /// GNU builtins/mapfile.def:330: `builtin_error (_("empty array variable
    /// name"))` for an empty array name argument.
    pub(in crate::executor) fn mapfile_empty_array_name(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: empty array variable name",
            self.diagnostic_prefix()
        );
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn mapfile_missing_option_argument(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        option: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: -{option}: option requires an argument",
            self.diagnostic_prefix()
        );
        self.print_mapfile_usage(command_name, stderr);
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn mapfile_invalid_option(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        option: char,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: -{option}: invalid option",
            self.diagnostic_prefix()
        );
        self.print_mapfile_usage(command_name, stderr);
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn print_mapfile_usage(
        &self,
        command_name: &str,
        stderr: &mut Vec<u8>,
    ) {
        let _ = writeln!(
            stderr,
            "{command_name}: usage: {command_name} [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]"
        );
    }

    pub(in crate::executor) fn finish_mapfile_error(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
        status: i32,
    ) -> i32 {
        if self
            .write_buffered_builtin_output(cmd, &[], stderr)
            .is_err()
        {
            return 1;
        }
        status
    }

    pub(in crate::executor) fn mapfile_input_for_command(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
    ) -> Option<String> {
        // GNU mapfile.def: `-u fd` selects the descriptor zgetline reads;
        // `-u 0` names fd 0 itself, so it is the ordinary fd-0 read below
        // — the FUNCTION_STDIN mirror a pipeline stage parks its input on
        // — not a numbered-fd serve (read.def's `-u` has the same rule;
        // rubash#1207: `mapfile -u 0` from a pipe returned an empty array).
        let Some(fd) = read_fd.filter(|fd| *fd != 0) else {
            // mapfile preserves carriage returns from CRLF input files. The generic
            // stdin helper is text-oriented, so use the raw-byte reader for a plain
            // fd-0 file redirect before falling back to heredoc/process-substitution
            // and inherited stdin handling.
            if let Some(redirect) = &cmd.redirect_in {
                if redirect.fd.unwrap_or(0) == 0 {
                    let target = self.expand_redirect_target(redirect);
                    // rubash#3660: an fd-alias spelling names a
                    // descriptor in the shell's fd table, never a
                    // filesystem path (see mapfile_record_source) —
                    // skip the raw file slurp so the string ladder
                    // below serves the alias from the table.
                    if super::execution_misc::dev_stdio_redirect_fd(&target).is_none()
                        && !target.starts_with("<(")
                        && !is_closed_redirect_target(&target)
                    {
                        // niubash#177: same operand resolution as
                        // open_input_redirect — a virtual-root input
                        // operand may exist only under its `.exe` spelling.
                        let path =
                            self.session_path(crate::executor::path::windows_operand_file_path(
                                shell_path_to_windows(&target, &self.shell_state.env_vars),
                                &self.shell_state.env_vars,
                            ));
                        // rubash#1903: mapfile's fd-0 redirect slurp
                        // takes a fifo marker's stream through the
                        // rendezvous, not the marker bytes (same family
                        // as read_redirected_fd).
                        #[cfg(windows)]
                        if let Some(Ok(bytes)) = crate::winuxcmd_fifo::slurp_marker_bytes(
                            &path,
                            &self.shell_state.env_vars,
                        ) {
                            return Some(
                                crate::executor::substitution_metadata::bytes_to_shell_text(&bytes),
                            );
                        }
                        if let Ok(input) =
                            crate::executor::substitution_metadata::read_shell_input_file(path)
                        {
                            return Some(input);
                        }
                    }
                }
            }
            if let Some(input) = self.stdin_string_for_command_mut(cmd) {
                return Some(input);
            }
            // A compound `< file` binds fd 0 to the real shared-offset
            // handle (with_command_input_redirects_inner, rubash#260) with
            // no FUNCTION_STDIN mirror — mapfile without -u reads that
            // descriptor to EOF, exactly like GNU mapfile.def on fd 0.
            if self.fd_table.is_open_for_read(0) {
                if let Some(input) = self.fd_table.read_all_text(0) {
                    return Some(input);
                }
            }
            // Fallback: read from inherited process stdin when INHERIT_PROCESS_STDIN is set
            // (e.g., printf '...' | rubash -c 'mapfile arr')
            if self
                .shell_state
                .env_vars
                .get(INHERIT_PROCESS_STDIN)
                .map(String::as_str)
                == Some("1")
            {
                return self.read_inherited_process_stdin_to_string();
            }
            return None;
        };

        // rubash#2305: same own-list discipline as the `read -u`
        // serve — the command's own list is authoritative for the
        // descriptor (redir.c:237 do_redirections), so a heredoc or
        // dup own binding serves BEFORE the ambient table entry;
        // the historic order (own file, ambient, own heredoc last)
        // let an ambient same-fd binding shadow the effective one.
        match self.own_input_binding_scan(cmd, fd, None) {
            Some((_position, super::read_redirected_fd::OwnInputBinding::Heredoc)) => {
                return self.mapfile_heredoc_fd_input(cmd, fd);
            }
            Some((dup_position, super::read_redirected_fd::OwnInputBinding::Duplicate(source)))
                if source != fd =>
            {
                return self.mapfile_dup_fd_input(cmd, source, dup_position);
            }
            Some((_position, super::read_redirected_fd::OwnInputBinding::Close)) => return None,
            _ => {}
        }
        if let Some(input) = self.mapfile_redirected_fd_input(cmd, fd) {
            return Some(input);
        }
        if let Some(input) = self.mapfile_virtual_fd_input(fd) {
            return Some(input);
        }
        self.mapfile_heredoc_fd_input(cmd, fd)
    }

    /// Slurp counterpart of read_dup_fd_input (rubash#2305): a
    /// Duplicate own binding copies the source's binding as of the
    /// dup's position — follow the source through the own list
    /// first, then to the ambient descriptor state. rubash#2314:
    /// positioned like the read chase — each step resolves against
    /// the entries before the current bound and a list-sourced
    /// winner is served entry-exactly (the `_at` serves).
    fn mapfile_dup_fd_input(
        &mut self,
        cmd: &CommandNode,
        source: u32,
        dup_position: Option<usize>,
    ) -> Option<String> {
        let mut current = source;
        let mut bound = dup_position;
        let mut seen = std::collections::HashSet::new();
        loop {
            if !seen.insert((current, bound)) {
                return None;
            }
            match self.own_input_binding_scan(cmd, current, bound) {
                Some((Some(index), super::read_redirected_fd::OwnInputBinding::File)) => {
                    return self.mapfile_redirected_fd_input_at(cmd, current, index);
                }
                Some((None, super::read_redirected_fd::OwnInputBinding::File)) => {
                    return self.mapfile_redirected_fd_input(cmd, current);
                }
                Some((Some(index), super::read_redirected_fd::OwnInputBinding::Heredoc)) => {
                    return self.mapfile_heredoc_fd_input_at(cmd, current, index);
                }
                Some((None, super::read_redirected_fd::OwnInputBinding::Heredoc)) => {
                    return self.mapfile_heredoc_fd_input(cmd, current);
                }
                Some((position, super::read_redirected_fd::OwnInputBinding::Duplicate(next))) => {
                    current = next;
                    bound = position;
                }
                Some((_position, super::read_redirected_fd::OwnInputBinding::Close)) => {
                    return None
                }
                None => break,
            }
        }
        self.mapfile_virtual_fd_input(current)
    }

    pub(in crate::executor) fn mapfile_fd_is_available(&self, cmd: &CommandNode, fd: u32) -> bool {
        // rubash#2305: same own-list walk as the `read` gate — an
        // own close makes the fd invalid even over an ambient open
        // entry (GNU mapfile.def validates the redirected
        // descriptor), an own dup defers to its source's state.
        if let Some(available) = self.own_input_binding_availability(cmd, fd) {
            return available;
        }
        if self.fd_table.is_open_for_read(fd) {
            return true;
        }
        // GNU builtins/mapfile.def:258 validates with sh_validfd: the
        // descriptor must be open in the shell. fd 0's open transports
        // are wider than the fd table — the FUNCTION_STDIN mirror a
        // pipeline stage parks its input on, and the inherited process
        // stdin — and mapfile_input_for_command's fd-0 serve consults
        // exactly those, so the gate must agree with the serve
        // (rubash#2534: `printf … | { mapfile -u 0 arr; }` reported
        // EBADF at the gate while the serve path held the bytes). The
        // mirror only counts while fd 0 is not closed, mirroring the
        // serve's own condition in stdin_string_for_command_mut.
        if fd == 0
            && ((!self.fd_table.is_closed(0) && self.function_stdin_remaining().is_some())
                || self
                    .shell_state
                    .env_vars
                    .get(INHERIT_PROCESS_STDIN)
                    .map(String::as_str)
                    == Some("1"))
        {
            return true;
        }
        if cmd
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.fd == Some(fd) && redirect.body.is_some())
        {
            return true;
        }
        cmd.redirects.iter().rev().any(|redirect| {
            redirect.fd == Some(fd)
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        })
    }

    fn mapfile_redirected_fd_input(&mut self, cmd: &CommandNode, fd: u32) -> Option<String> {
        // Redirections are applied in source order; the last matching input
        // redirect is the descriptor's effective source.
        let redirect = cmd
            .redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;

        self.mapfile_redirected_fd_input_entry(redirect)
    }

    /// Entry-exact counterpart (rubash#2314): serve from the input
    /// entry AT `index` in the command's own list — the entry a
    /// positioned binding scan resolved as the winner.
    fn mapfile_redirected_fd_input_at(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        index: usize,
    ) -> Option<String> {
        let redirect = cmd
            .redirects
            .iter()
            .take(index + 1)
            .rev()
            .find(|redirect| {
                redirect.fd == Some(fd)
                    && matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
                    )
            })?;
        self.mapfile_redirected_fd_input_entry(redirect)
    }

    fn mapfile_redirected_fd_input_entry(
        &mut self,
        redirect: &crate::parser::Redirect,
    ) -> Option<String> {
        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            return self.process_substitution_output(source);
        }

        let target = self.expand_redirect_target(redirect);
        if is_closed_redirect_target(&target) {
            return None;
        }
        // rubash#2235: an fd-alias target (/dev/fd/N, /proc/self/fd/N)
        // names the fd table's endpoint at N (the rubash#1963 alias
        // layer) — above all a list-level input procsub the
        // materializer rewrote to exactly this spelling. Drain the
        // endpoint through the alias service (mapfile reads to EOF
        // by design); reading the spelling as a filesystem path
        // would read the process's unrelated real fd N.
        if let Some(alias_fd) = dev_stdio_redirect_fd(&target) {
            if self.fd_table.read_endpoint(alias_fd).is_some() {
                let mut file = self.open_fd_read_endpoint(alias_fd, &target).ok()?;
                let mut input = String::new();
                use std::io::Read as _;
                file.read_to_string(&mut input).ok()?;
                return Some(input);
            }
        }
        // niubash#177: same operand resolution as open_input_redirect —
        // a virtual-root input operand may exist only under its `.exe`
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
        fs::read_to_string(path).ok()
    }

    fn mapfile_virtual_fd_input(&mut self, fd: u32) -> Option<String> {
        if self.fd_table.is_open_for_read(fd) {
            if let Some(input) = self.fd_table.read_all_text(fd) {
                if let Some(offset) = self.fd_table.input_offset(fd) {
                    self.shell_state
                        .env_vars
                        .insert(fd_stdin_offset_key(fd), offset.to_string());
                }
                return Some(input);
            }
        }
        if matches!(
            self.fd_table.read_endpoint(fd),
            Some(FdReadEndpoint::InheritedProcessStdin)
        ) {
            return self.read_inherited_process_stdin_to_string();
        }
        None
    }

    fn mapfile_heredoc_fd_input(&mut self, cmd: &CommandNode, fd: u32) -> Option<String> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;
        self.mapfile_heredoc_fd_input_entry(redirect)
    }

    /// Entry-exact counterpart (rubash#2314): serve the body of the
    /// heredoc entry AT `index` in the command's own list, via the
    /// same ordinal correlation as read_heredoc_fd_input_at.
    fn mapfile_heredoc_fd_input_at(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        index: usize,
    ) -> Option<String> {
        let winner = cmd.redirects.get(index)?;
        let want_string = matches!(winner.kind, crate::parser::RedirectKind::HereString);
        let ordinal = cmd.redirects[..index]
            .iter()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
                ) && matches!(entry.kind, crate::parser::RedirectKind::HereString) == want_string
                    && entry.fd == Some(fd)
            })
            .count();
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .filter(|entry| entry.fd == Some(fd) && entry.here_string == want_string)
            .nth(ordinal)?;
        self.mapfile_heredoc_fd_input_entry(redirect)
    }

    fn mapfile_heredoc_fd_input_entry(
        &mut self,
        redirect: &crate::parser::HereDocRedirect,
    ) -> Option<String> {
        if redirect.body_carrier.is_some() {
            let mut input = if redirect.here_string {
                self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
            } else {
                self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
            };
            if redirect.here_string {
                input.push('\n');
            }
            return Some(input);
        }
        let body = redirect.body.as_deref()?;
        if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            return Some(input);
        }
        Some(self.expand_heredoc_body_mut(body))
    }

    /// GNU builtins/mapfile.def:108-128 run_callback: the callback
    /// string is NOT word-split — bash builds
    /// `"<callback> <curindex> <sh_single_quote(curline)>"` and hands
    /// it to evalstring (parse_and_execute). Quotes, `;` lists and
    /// redirections inside the callback therefore work, and the line
    /// arrives as exactly one argument (single-quoted, so never
    /// glob-expanded). mapfile discards run_callback's exit status;
    /// only a hard executor error propagates here (rubash#1586).
    pub(in crate::executor) fn execute_mapfile_callback(
        &mut self,
        callback: &str,
        index: usize,
        value: &str,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let source = format!("{callback} {index} {}", mapfile_single_quote(value));
        let caller_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        self.execute_eval_source(&source, caller_line, cmd)
    }

    /// GNU builtins/mapfile.def:181-214 read_array: zgetline pulls one
    /// record at a time from the fd and the callback fires DURING the
    /// read — for each quantum-th kept record, BEFORE that record is
    /// bound (run_callback precedes bind_array_variable), so the
    /// callback observes the array holding only the earlier records,
    /// and a producer on the other end of the pipe can react to the
    /// callback before writing more. The slurp-then-fire shape this
    /// replaces bound everything first and deadlocked such producers
    /// (rubash#1586). This driver is used only when -C is present; the
    /// callback-free path keeps its whole-input fast path.
    ///
    /// Returns Ok(None) when no input source exists (the caller then
    /// takes its usual clear-and-report tail), Ok(Some(status)) when
    /// the read ran.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::executor) fn execute_mapfile_with_callback(
        &mut self,
        cmd: &CommandNode,
        name: &str,
        delimiter: Option<u8>,
        trim_newline: bool,
        skip: usize,
        count: Option<usize>,
        origin: Option<usize>,
        callback: &str,
        callback_quantum: usize,
        read_fd: Option<u32>,
        stderr: &[u8],
    ) -> Result<Option<i32>, ExecuteError> {
        let Some(mut source) = self.mapfile_record_source(cmd, read_fd, true) else {
            return Ok(None);
        };
        let start = origin.unwrap_or(0);
        let mut entries: std::collections::BTreeMap<usize, String> = if origin.is_some() {
            // -O keeps the existing elements; seed the live binding
            // bookkeeping from the current value.
            self.shell_state
                .env_vars
                .get(name)
                .map(|current| indexed_array_entries(current))
                .unwrap_or_default()
        } else {
            // A fresh read clears the array up front (mapfile.def
            // read_array with no -O starts from an empty array), so a
            // callback inspecting the array mid-read sees exactly the
            // records bound so far.
            sync_typed_indexed_array(
                &mut self.shell_state.variables,
                name,
                &std::collections::BTreeMap::new(),
            );
            self.shell_state.env_vars.insert(
                name.to_string(),
                format_indexed_array_storage(std::collections::BTreeMap::new()),
            );
            std::collections::BTreeMap::new()
        };
        mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", name);
        let limit = count.filter(|limit| *limit > 0);
        let mut kept = 0usize;
        let mut skipped = 0usize;
        loop {
            if limit.is_some_and(|limit| kept >= limit) {
                break;
            }
            let Some(record) = source.next_record(delimiter.unwrap_or(b'\n'), trim_newline) else {
                break;
            };
            if skipped < skip {
                skipped += 1;
                continue;
            }
            let target = start + kept;
            kept += 1;
            if kept % callback_quantum == 0 {
                // GNU syncs the fd offset to the consumed boundary
                // before run_callback when reads are buffered
                // (zsyncfd, mapfile.def:195) — a callback reading the
                // same fd must see exactly the unconsumed bytes.
                self.mapfile_source_sync(&mut source);
                // rubash#3660: the callback runs in-process on the
                // builtin's input descriptor (mapfile.def:108-131),
                // so bind the record source's own stream to that
                // descriptor for the invocation — a callback `read`
                // then consumes the shared stream, interleaved with
                // the driver's records, instead of the shell's
                // ambient stdin.
                let input_fd = read_fd.unwrap_or(0);
                let share = source.begin_callback_share(self, input_fd);
                let callback_result = self.execute_mapfile_callback(callback, target, &record, cmd);
                source.end_callback_share(self, input_fd, share);
                if let Err(error) = callback_result {
                    source.unstage_mirror(self);
                    return Err(error);
                }
                // A3-F4: GNU mapfile.def:142-218 re-looks-up the array
                // after run_callback ("Bad things can happen if the
                // callback modifies ENTRY... so we look it up again
                // every time"), so a callback that resets or unsets
                // the target array is observed, not clobbered by the
                // driver's local entry map at the final render.
                entries = self
                    .shell_state
                    .env_vars
                    .get(name)
                    .map(|current| indexed_array_entries(current))
                    .unwrap_or_default();
            }
            // rubash#3659: the callback above received the raw record
            // (GNU mapfile.def:195-203); the attribute conversion applies
            // at the bind, like bind_array_element.
            let record = self.convert_array_element_value(name, &record);
            let previous_max = entries.keys().next_back().copied();
            entries.insert(target, record.clone());
            self.bind_mapfile_element_live(name, &entries, target, &record, previous_max);
        }
        self.mapfile_source_sync(&mut source);
        source.unstage_mirror(self);
        // Canonical final render: byte-identical to the shape the
        // slurp path stores for the same input.
        self.shell_state
            .env_vars
            .insert(name.to_string(), format_indexed_array_storage(entries));
        mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", name);
        let _ = self.write_buffered_builtin_output(cmd, &[], stderr);
        Ok(Some(0))
    }

    /// `source.sync_offset()` plus the staged-text cursor publish:
    /// a pull over the FUNCTION_STDIN mirror reports its consumed
    /// byte boundary so the next reader of the same stage input
    /// resumes there (rubash#2779, mapfile.def:162-166 bounded
    /// pipe reads leaving the tail).
    pub(in crate::executor) fn mapfile_source_sync(&mut self, source: &mut MapfileRecordSource) {
        source.sync_offset();
        if let Some(target) = source.staged_target_offset() {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), target.to_string());
        }
    }

    /// Bind one record into the live array stores (typed cell + mirror)
    /// so a callback fired for a LATER record observes it. `entries` is
    /// the full post-insert entry set; `previous_max` is the largest
    /// index present before this insert — the mirror append fast path
    /// is only taken on a strict extension, exactly like the element
    /// assignment path (array_assignment_exec.rs).
    fn bind_mapfile_element_live(
        &mut self,
        name: &str,
        entries: &std::collections::BTreeMap<usize, String>,
        index: usize,
        value: &str,
        previous_max: Option<usize>,
    ) {
        sync_typed_indexed_element(
            &mut self.shell_state.variables,
            name,
            index as i64,
            value,
            entries,
        );
        let is_extension = previous_max.is_none_or(|max| index > max);
        if !is_extension
            || !append_indexed_mirror_segments(
                &mut self.shell_state.env_vars,
                name,
                [(index, value)],
            )
        {
            self.shell_state.env_vars.insert(
                name.to_string(),
                format_indexed_array_storage(entries.clone()),
            );
        }
    }

    /// Install `endpoint` as the read arm of `fd` for one mapfile
    /// callback invocation, returning what end_callback_share needs
    /// to restore (rubash#3660). The entry is written directly, not
    /// through open_input: open_input retires the outgoing endpoint,
    /// which would close a saved FileFd the restore must hand back
    /// live. The saved entry's write arm stays in place — only the
    /// read face is rebound to the mapfile stream.
    fn bind_mapfile_callback_stream(
        &mut self,
        fd: u32,
        endpoint: FdReadEndpoint,
    ) -> MapfileCallbackShare {
        let saved = self.fd_table.entries.get(&fd).cloned();
        let entry = crate::executor::fd_table::FdEntry {
            read: Some(endpoint),
            write: saved.as_ref().and_then(|entry| entry.write.clone()),
            closed: false,
            dynamic: saved.as_ref().is_some_and(|entry| entry.dynamic),
        };
        self.fd_table.entries.insert(fd, entry);
        MapfileCallbackShare::Bound { saved }
    }

    /// Restore the entry bind_mapfile_callback_stream saved (or the
    /// entry's absence) once a callback invocation ends.
    fn unbind_mapfile_callback_stream(
        &mut self,
        fd: u32,
        saved: Option<crate::executor::fd_table::FdEntry>,
    ) {
        match saved {
            Some(entry) => {
                self.fd_table.entries.insert(fd, entry);
            }
            None => {
                self.fd_table.entries.remove(&fd);
            }
        }
    }

    /// One fd's record source: real handles pull incrementally;
    /// text / process-substitution endpoints are already materialized.
    /// On a numbered descriptor the source SHARES the endpoint's own
    /// cell, so a callback reading that descriptor interleaves with
    /// the driver (rubash#3660); on fd 0 the endpoint is drained whole
    /// (as the slurp path drains it) into a record stream — mirrored
    /// for the callback driver — and the callbacks still fire between
    /// records.
    fn mapfile_endpoint_source(&mut self, fd: u32, mirror: bool) -> Option<MapfileRecordSource> {
        let endpoint = self.fd_table.read_endpoint(fd)?;
        match &endpoint {
            FdReadEndpoint::Text(stream) | FdReadEndpoint::ProcessSubstitution(stream)
                if fd != 0 =>
            {
                return Some(MapfileRecordSource::values_shared(stream.clone()));
            }
            FdReadEndpoint::Text(_) | FdReadEndpoint::ProcessSubstitution(_) => {
                let bytes = self.fd_table.read_all_bytes(fd)?;
                let text = String::from_utf8(bytes).unwrap_or_else(|error| {
                    String::from_utf8_lossy(&error.into_bytes()).into_owned()
                });
                return Some(if mirror {
                    MapfileRecordSource::values_mirrored(text, self)
                } else {
                    MapfileRecordSource::values(text)
                });
            }
            _ => {}
        }
        MapfileRecordSource::from_endpoint(&endpoint)
    }

    /// Resolve where the callback form reads its records from,
    /// mirroring mapfile_input_for_command's source order, but keeping
    /// real fds/files incremental (MapfileRecordPuller) instead of
    /// slurping them: only the already-materialized sources (command
    /// redirects staged for another fd, command string input, text /
    /// process-substitution endpoints) are drained into a record
    /// stream, and even those fire their callbacks between records.
    pub(in crate::executor) fn mapfile_record_source(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        for_callback: bool,
    ) -> Option<MapfileRecordSource> {
        if let Some(fd) = read_fd.filter(|fd| *fd != 0) {
            let staged_redirect = cmd.redirects.iter().any(|redirect| redirect.fd == Some(fd));
            if !staged_redirect {
                if let Some(source) = self.mapfile_endpoint_source(fd, false) {
                    return Some(source);
                }
            }
            return self
                .mapfile_input_for_command(cmd, read_fd)
                .map(MapfileRecordSource::values);
        }
        // fd 0 domain, in mapfile_input_for_command's order.
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_redirect_target(redirect);
                // rubash#3660: an fd-alias spelling (/dev/fd/N,
                // /dev/stdin, /proc/self/fd/N) names a descriptor in
                // the shell's own fd table — GNU's open of the magic
                // link serves THAT descriptor's stream (redir.c
                // do_redirections). Opening the spelling as a plain
                // filesystem path instead reads whatever real
                // descriptor the process happens to hold at that
                // number, a different stream whenever the table and
                // the OS disagree (an exec-bound procsub pipe left at
                // a high real fd, say). Resolve through the table
                // endpoint; with no endpoint there, fall through to
                // the string ladder, which owns the alias arms.
                if let Some(alias_fd) = super::execution_misc::dev_stdio_redirect_fd(&target) {
                    if let Some(source) = self.mapfile_endpoint_source(alias_fd, for_callback) {
                        return Some(source);
                    }
                } else if !target.starts_with("<(") && !is_closed_redirect_target(&target) {
                    let path = self.session_path(crate::executor::path::windows_operand_file_path(
                        shell_path_to_windows(&target, &self.shell_state.env_vars),
                        &self.shell_state.env_vars,
                    ));
                    if let Ok(mut file) = std::fs::File::open(&path) {
                        let origin = std::io::Seek::stream_position(&mut file).unwrap_or(0);
                        return Some(MapfileRecordSource::Pull(MapfileRecordPuller {
                            source: MapfilePullSource::File { file, path },
                            buffer: Vec::new(),
                            eof: false,
                            taken: 0,
                            rewind: Some(origin),
                            next_fill_offset: 0,
                            home_endpoint: None,
                        }));
                    }
                    // Open failed: fall through — the string chain
                    // fails the same read and moves on, as the slurp
                    // path does.
                }
            }
        }
        // A command with no stdin staging of its own reads the fd-0
        // endpoint the shell currently holds (an `exec 0<file`
        // binding, a pipeline stage's pipe): when that endpoint is a
        // real handle, pull from it incrementally instead of
        // materializing it whole through the stdin-string snapshot
        // below — the snapshot drains the shared stream, so even a
        // bounded reader starves the next command on the same fd.
        if cmd.redirect_in.is_none()
            && cmd.heredoc.is_none()
            && cmd.here_string.is_none()
            && cmd.heredoc_redirects.is_empty()
            && matches!(
                self.fd_table.read_endpoint(0),
                Some(FdReadEndpoint::File(_) | FdReadEndpoint::CoprocStdout { .. })
            )
        {
            if let Some(source) = self.mapfile_endpoint_source(0, for_callback) {
                return Some(source);
            }
        }
        // rubash#2779 — when the command's fd 0 resolves to the staged
        // FUNCTION_STDIN mirror (a pipeline stage's brace-group/subshell
        // input, not a real handle), the slurp arm below drains the
        // shared remainder wholesale: stdin_string_for_command_mut
        // parks FUNCTION_STDIN_OFFSET at end, so even a bounded read
        // starves the next reader of the same stage input. GNU's
        // bounded read never over-consumes — a pipe switches to
        // unbuffered zgetline record reads (builtins/mapfile.def:162-166,
        // lseek ESPIPE). Pull records incrementally over the staged
        // bytes; the consumer's mapfile_source_sync publishes only the
        // consumed cursor back to FUNCTION_STDIN_OFFSET, leaving the
        // tail for the next reader of the shared fd.
        if cmd.heredoc.is_none()
            && cmd.here_string.is_none()
            && cmd.heredoc_redirects.is_empty()
            && self.function_stdin_is_command_source(cmd)
        {
            if let Some(text) = self.function_stdin_remaining() {
                let origin = self
                    .shell_state
                    .env_vars
                    .get(FUNCTION_STDIN_OFFSET)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
                return Some(MapfileRecordSource::staged(text, origin));
            }
        }
        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            return Some(if for_callback {
                MapfileRecordSource::values_mirrored(input, self)
            } else {
                MapfileRecordSource::values(input)
            });
        }
        if self.fd_table.is_open_for_read(0) {
            if let Some(source) = self.mapfile_endpoint_source(0, for_callback) {
                return Some(source);
            }
        }
        if self
            .shell_state
            .env_vars
            .get(INHERIT_PROCESS_STDIN)
            .map(String::as_str)
            == Some("1")
        {
            return Some(MapfileRecordSource::Pull(MapfileRecordPuller {
                source: MapfilePullSource::InheritedStdin,
                buffer: Vec::new(),
                eof: false,
                taken: 0,
                rewind: None,
                next_fill_offset: 0,
                home_endpoint: None,
            }));
        }
        None
    }
}

/// lib/sh/shquote.c sh_single_quote: wrap in single quotes, rendering
/// each embedded quote as '\''. (A copy of the helper in
/// builtins/complete.rs — the same duplication pattern as shell_quote
/// in builtins/set.rs and builtins/trap.rs.)
fn mapfile_single_quote(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('\'');
    for c in value.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Record stream for the callback form of mapfile (rubash#1586).
pub(in crate::executor) enum MapfileRecordSource {
    /// Already-materialized input as a shared raw-byte stream: the
    /// driver extracts records from the same (data, offset) cell a
    /// callback's reads of the input descriptor serve, so the two
    /// consumers share ONE cursor exactly like GNU's single fd
    /// (builtins/mapfile.def:195-197 zsyncfd before run_callback;
    /// lib/sh/zread.c:294 — rubash#3660). Records are extracted in
    /// the raw-byte domain with the same scan split_mapfile_input's
    /// general path performs over the decoded bytes, so the values
    /// are identical to the pre-split form this replaces.
    Values {
        stream: Rc<RefCell<crate::executor::fd_table::TextInput>>,
        /// For an fd-0 text source consumed with callbacks: the
        /// stream is ALSO staged as the FUNCTION_STDIN mirror, the
        /// store every nested command's stdin is carved from (with
        /// the consumption folded back at function exit,
        /// function_calls.rs) — the driver extracts from the byte
        /// copy and the share protocol translates the cursor between
        /// the two domains at each callback boundary (rubash#3660).
        mirror: Option<MapfileMirrorSync>,
    },
    /// Incremental pull in the raw-byte domain.
    Pull(MapfileRecordPuller),
}

/// The staged-mirror half of an fd-0 Values source (rubash#3660):
/// the input text as staged in FUNCTION_STDIN, the byte→text offset
/// map between the driver's decoded-byte cursor and the mirror's
/// text cursor, and the mirror pair the staging displaced (restored
/// when the driver finishes).
pub(in crate::executor) struct MapfileMirrorSync {
    text: String,
    byte_to_text: Vec<usize>,
    saved_stdin: Option<String>,
    saved_offset: Option<String>,
    /// The fd-0 entry the staging displaced (restored at unstage):
    /// the staged mirror IS fd 0's binding for the driver's duration
    /// (the bind_command_fd0_input mirror branch's discipline,
    /// rubash#2170) — an ambient endpoint left in place would govern
    /// the callback's readers instead of the mapfile stream.
    saved_fd0_entry: Option<crate::executor::fd_table::FdEntry>,
}

impl MapfileRecordSource {
    fn values(input: String) -> Self {
        MapfileRecordSource::Values {
            stream: Rc::new(RefCell::new(
                crate::executor::fd_table::TextInput::from_data(
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
                ),
            )),
            mirror: None,
        }
    }

    /// A record stream over an already-shared text endpoint cell
    /// (a `-u` descriptor's materialized stream): extraction advances
    /// the endpoint's own cursor, so a callback reading that
    /// descriptor serves the same stream (rubash#3660).
    fn values_shared(stream: Rc<RefCell<crate::executor::fd_table::TextInput>>) -> Self {
        MapfileRecordSource::Values {
            stream,
            mirror: None,
        }
    }

    /// An fd-0 text source for the callback driver: stage the input
    /// as the FUNCTION_STDIN mirror (saving the displaced pair for
    /// unstage_mirror) so a callback's reads are carved from — and
    /// folded back into — the one cursor the driver itself consumes,
    /// the same discipline loop input follows (rubash#3660). When
    /// the carrier round-trip is not exact for this text, the offset
    /// map could lie, so fall back to the unstaged stream.
    fn values_mirrored(input: String, executor: &mut Executor) -> Self {
        let decoded = crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input);
        if crate::executor::substitution_metadata::bytes_to_shell_text(&decoded) != input {
            return Self::values(input);
        }
        // The encode is byte-local, so prefix lengths accumulate:
        // byte_to_text[b] is the staged-text length of the first b
        // decoded bytes.
        let mut byte_to_text = Vec::with_capacity(decoded.len() + 1);
        let mut text_len = 0usize;
        byte_to_text.push(0);
        for byte in &decoded {
            text_len += crate::executor::substitution_metadata::bytes_to_shell_text(
                std::slice::from_ref(byte),
            )
            .len();
            byte_to_text.push(text_len);
        }
        let saved_stdin = executor
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.clone());
        let saved_offset = executor
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        let saved_fd0_entry = executor.fd_table.entries.remove(&0);
        MapfileRecordSource::Values {
            stream: Rc::new(RefCell::new(
                crate::executor::fd_table::TextInput::from_data(decoded),
            )),
            mirror: Some(MapfileMirrorSync {
                text: input,
                byte_to_text,
                saved_stdin,
                saved_offset,
                saved_fd0_entry,
            }),
        }
    }

    /// Restore the FUNCTION_STDIN pair values_mirrored displaced.
    /// Called on every exit of the callback driver once a source
    /// exists; a no-op for unstaged sources.
    fn unstage_mirror(&mut self, executor: &mut Executor) {
        if let MapfileRecordSource::Values {
            mirror: Some(sync), ..
        } = self
        {
            match &sync.saved_stdin {
                Some(value) => {
                    executor
                        .shell_state
                        .env_vars
                        .insert(FUNCTION_STDIN.to_string(), value.clone());
                }
                None => {
                    executor.shell_state.env_vars.remove(FUNCTION_STDIN);
                }
            }
            match &sync.saved_offset {
                Some(value) => {
                    executor
                        .shell_state
                        .env_vars
                        .insert(FUNCTION_STDIN_OFFSET.to_string(), value.clone());
                }
                None => {
                    executor.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
                }
            }
            match &sync.saved_fd0_entry {
                Some(entry) => {
                    executor.fd_table.entries.insert(0, entry.clone());
                }
                None => {
                    executor.fd_table.entries.remove(&0);
                }
            }
        }
    }

    /// Incremental pull over the staged FUNCTION_STDIN mirror
    /// (rubash#2779). `origin` is the FUNCTION_STDIN_OFFSET the mirror
    /// was read at; `taken` counts consumed bytes, so the publish
    /// target is `origin + taken` — idempotent across repeated syncs.
    fn staged(text: String, origin: u64) -> Self {
        MapfileRecordSource::Pull(MapfileRecordPuller {
            source: MapfilePullSource::StagedText {
                bytes: text.into_bytes(),
                pos: 0,
                origin,
            },
            buffer: Vec::new(),
            eof: false,
            taken: 0,
            rewind: None,
            next_fill_offset: 0,
            home_endpoint: None,
        })
    }

    /// Absolute FUNCTION_STDIN_OFFSET a staged-text pull consumed to,
    /// or None for every other source kind.
    pub(in crate::executor) fn staged_target_offset(&self) -> Option<u64> {
        match self {
            MapfileRecordSource::Pull(puller) => match &puller.source {
                MapfilePullSource::StagedText { origin, .. } => Some(origin + puller.taken),
                _ => None,
            },
            _ => None,
        }
    }

    fn from_endpoint(endpoint: &FdReadEndpoint) -> Option<Self> {
        match endpoint {
            FdReadEndpoint::File(file) => {
                let handle = file.handle;
                let disk = crate::fd::is_disk_file(handle);
                let rewind = if disk {
                    crate::fd::file_position(handle).ok()
                } else {
                    None
                };
                Some(MapfileRecordSource::Pull(MapfileRecordPuller {
                    source: MapfilePullSource::Handle {
                        handle,
                        chunked: disk,
                    },
                    buffer: Vec::new(),
                    eof: false,
                    taken: 0,
                    rewind,
                    next_fill_offset: 0,
                    home_endpoint: Some(endpoint.clone()),
                }))
            }
            FdReadEndpoint::CoprocStdout { fd: file, .. } => {
                Some(MapfileRecordSource::Pull(MapfileRecordPuller {
                    source: MapfilePullSource::Handle {
                        handle: file.handle,
                        chunked: false,
                    },
                    buffer: Vec::new(),
                    eof: false,
                    taken: 0,
                    rewind: None,
                    next_fill_offset: 0,
                    home_endpoint: Some(endpoint.clone()),
                }))
            }
            FdReadEndpoint::Text(_) | FdReadEndpoint::ProcessSubstitution(_) => {
                // Fully materialized already; drain is handled by the
                // caller through FdTable::read_all_bytes (needs &mut).
                None
            }
            FdReadEndpoint::InheritedProcessStdin => {
                Some(MapfileRecordSource::Pull(MapfileRecordPuller {
                    source: MapfilePullSource::InheritedStdin,
                    buffer: Vec::new(),
                    eof: false,
                    taken: 0,
                    rewind: None,
                    next_fill_offset: 0,
                    home_endpoint: Some(endpoint.clone()),
                }))
            }
            // rubash#1937: a directory endpoint has no record
            // source; the read face itself reports EISDIR.
            FdReadEndpoint::Directory => None,
        }
    }

    pub(in crate::executor) fn next_record(
        &mut self,
        delimiter: u8,
        trim_newline: bool,
    ) -> Option<String> {
        match self {
            MapfileRecordSource::Values { stream, .. } => {
                // The puller's scan over the shared stream: one
                // record from the cursor, delimiter consumed, the
                // cursor left just past it — the same segmentation
                // split_mapfile_input performs (its trailing partial
                // segment is the cursor-to-EOF record here).
                let (record, next_offset) = {
                    let stream = stream.borrow();
                    let data = stream.data();
                    let start = stream.offset();
                    if start >= data.len() {
                        return None;
                    }
                    let rest = &data[start..];
                    match rest.iter().position(|byte| *byte == delimiter) {
                        Some(pos) => {
                            let mut raw = rest[..=pos].to_vec();
                            if trim_newline || delimiter == 0 {
                                raw.pop();
                            }
                            (finish_mapfile_record(raw, delimiter), start + pos + 1)
                        }
                        None => (finish_mapfile_record(rest.to_vec(), delimiter), data.len()),
                    }
                };
                stream.borrow_mut().set_offset(next_offset);
                Some(record)
            }
            MapfileRecordSource::Pull(puller) => puller.next_record(delimiter, trim_newline),
        }
    }

    pub(in crate::executor) fn sync_offset(&mut self) {
        if let MapfileRecordSource::Pull(puller) = self {
            puller.sync_offset();
        }
    }

    /// Bind this source's stream to the mapfile input descriptor for
    /// one callback invocation (rubash#3660): GNU runs the callback
    /// in-process with the builtin's own input fd (builtins/
    /// mapfile.def:108-131 run_callback → evalstring), so a callback
    /// `read` consumes the very stream mapfile is pulling records
    /// from, interleaved record by record. Materialized sources bind
    /// their shared (data, offset) cell as the descriptor's text
    /// endpoint; a path-opened file binds a dup of its handle (the
    /// dup shares the open file description, so both readers move
    /// one offset); handle and inherited-stdin sources already
    /// resolve through the descriptor's own endpoint and need no
    /// binding. The caller must pair every call with
    /// end_callback_share, on error paths too.
    fn begin_callback_share(
        &mut self,
        executor: &mut Executor,
        input_fd: u32,
    ) -> MapfileCallbackShare {
        match self {
            MapfileRecordSource::Values {
                stream,
                mirror: Some(sync),
            } => {
                // The mirror is the share: point its cursor at the
                // driver's consumed boundary, and the callback's
                // reads carve from there (function_call_stdin) with
                // the consumption folded back at function exit —
                // end_callback_share translates the folded cursor
                // back into the byte domain. No endpoint is bound:
                // an fd-0 text endpoint would make the carve gate
                // decline and snapshot instead (rubash#3660).
                let byte_cursor = stream.borrow().offset();
                let text_cursor = sync
                    .byte_to_text
                    .get(byte_cursor)
                    .copied()
                    .unwrap_or(sync.text.len());
                executor
                    .shell_state
                    .env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), text_cursor.to_string());
                MapfileCallbackShare::Unbound
            }
            MapfileRecordSource::Values {
                stream,
                mirror: None,
            } => executor
                .bind_mapfile_callback_stream(input_fd, FdReadEndpoint::Text(stream.clone())),
            MapfileRecordSource::Pull(puller) => puller.begin_callback_share(executor, input_fd),
        }
    }

    /// Undo begin_callback_share and fold any bytes the callback
    /// consumed through the shared descriptor back into the puller's
    /// bookkeeping, so the driver's next record continues exactly
    /// where the callback stopped (GNU: one fd, one offset).
    fn end_callback_share(
        &mut self,
        executor: &mut Executor,
        input_fd: u32,
        share: MapfileCallbackShare,
    ) {
        if let MapfileCallbackShare::Bound { saved } = share {
            executor.unbind_mapfile_callback_stream(input_fd, saved);
        }
        if let MapfileRecordSource::Values {
            stream,
            mirror: Some(sync),
        } = self
        {
            // Fold the mirror cursor the callback left behind back
            // into the driver's byte cursor (rubash#3660).
            let text_cursor = executor
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0)
                .min(sync.text.len());
            let mut end = text_cursor;
            while end > 0 && !sync.text.is_char_boundary(end) {
                end -= 1;
            }
            let byte_cursor =
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&sync.text[..end])
                    .len();
            stream.borrow_mut().set_offset(byte_cursor);
        }
        if let MapfileRecordSource::Pull(puller) = self {
            puller.resync_after_callback();
            puller.fold_staged_callback_consumption(executor);
        }
    }
}

/// What begin_callback_share installed on the input descriptor, to
/// be restored by end_callback_share (rubash#3660).
pub(in crate::executor) enum MapfileCallbackShare {
    /// Nothing installed: the source already resolves through the
    /// descriptor's own endpoint (handle / inherited-stdin sources).
    Unbound,
    /// An endpoint was installed over the descriptor's saved entry.
    Bound {
        saved: Option<crate::executor::fd_table::FdEntry>,
    },
}

enum MapfilePullSource {
    /// fd-table handle: chunked reads for disk files, byte-wise for
    /// pipes (the offset is shared with later readers — byte-wise
    /// reads can never over-consume a pipe, matching zgetline's
    /// unbuffered mode for non-regular files, mapfile.def:160-170).
    Handle {
        handle: crate::fd::HANDLE,
        chunked: bool,
    },
    /// The staged FUNCTION_STDIN mirror: already materialized, so the
    /// fill delivers the unread tail in one chunk and `taken` tracks
    /// the byte boundary the caller folds back into
    /// FUNCTION_STDIN_OFFSET (rubash#2779).
    StagedText {
        bytes: Vec<u8>,
        pos: usize,
        origin: u64,
    },
    /// A plain file opened by path for the `< file` form. The path
    /// is kept so a callback share can bind a dup of the handle into
    /// the fd table (FileFd carries the path for its read face).
    File {
        file: std::fs::File,
        path: std::path::PathBuf,
    },
    /// The process's inherited stdin (byte-wise, like a pipe).
    InheritedStdin,
}

/// Pull-based record reader over raw bytes. `taken` counts the bytes
/// extracted into records so far; for seekable sources `rewind`
/// holds the origin position so sync_offset can move the shared
/// offset back to the consumed boundary — over-read bytes stay
/// valid in `buffer`, and `next_fill_offset` makes the next fill
/// skip the overlap it re-reads.
pub(in crate::executor) struct MapfileRecordPuller {
    source: MapfilePullSource,
    buffer: Vec<u8>,
    eof: bool,
    taken: u64,
    rewind: Option<u64>,
    next_fill_offset: u64,
    /// The fd-table endpoint this puller was built from, when it was
    /// built from one (rubash#3660): a callback share installs it at
    /// the mapfile input descriptor — which is not always the
    /// descriptor the endpoint lives on (an fd-alias redirect target
    /// resolves to the aliased descriptor's endpoint while the
    /// callback reads the builtin's own input fd) — so the callback
    /// and the driver serve the one underlying stream.
    home_endpoint: Option<FdReadEndpoint>,
}

impl MapfileRecordPuller {
    fn next_record(&mut self, delimiter: u8, trim_newline: bool) -> Option<String> {
        loop {
            if let Some(pos) = self.buffer.iter().position(|byte| *byte == delimiter) {
                let mut raw: Vec<u8> = self.buffer.drain(..=pos).collect();
                self.taken += raw.len() as u64;
                if trim_newline || delimiter == 0 {
                    raw.pop();
                }
                return Some(finish_mapfile_record(raw, delimiter));
            }
            if self.eof {
                if self.buffer.is_empty() {
                    return None;
                }
                let raw = std::mem::take(&mut self.buffer);
                self.taken += raw.len() as u64;
                return Some(finish_mapfile_record(raw, delimiter));
            }
            self.fill();
        }
    }

    fn fill(&mut self) {
        let chunk = match &mut self.source {
            MapfilePullSource::Handle { handle, chunked } => {
                let size = if *chunked { 8192 } else { 1 };
                match crate::fd::read_some(*handle, size) {
                    Ok(bytes) if !bytes.is_empty() => bytes,
                    _ => {
                        self.eof = true;
                        return;
                    }
                }
            }
            MapfilePullSource::File { file, .. } => {
                let mut buf = [0u8; 8192];
                match std::io::Read::read(file, &mut buf) {
                    Ok(n) if n > 0 => buf[..n].to_vec(),
                    _ => {
                        self.eof = true;
                        return;
                    }
                }
            }
            MapfilePullSource::StagedText { bytes, pos, .. } => {
                if *pos >= bytes.len() {
                    self.eof = true;
                    return;
                }
                let chunk = bytes[*pos..].to_vec();
                *pos = bytes.len();
                chunk
            }
            MapfilePullSource::InheritedStdin => {
                match crate::fd::read_some(crate::fd::process_std_handle(0), 1) {
                    Ok(bytes) if !bytes.is_empty() => bytes,
                    _ => {
                        self.eof = true;
                        return;
                    }
                }
            }
        };
        // After a sync_offset rewind, the head of the next chunk
        // duplicates bytes still held in `buffer`; skip the overlap.
        let logical_end = self.taken + self.buffer.len() as u64;
        let overlap = logical_end.saturating_sub(self.next_fill_offset) as usize;
        self.next_fill_offset += chunk.len() as u64;
        let skip = overlap.min(chunk.len());
        self.buffer.extend_from_slice(&chunk[skip..]);
    }

    /// Move a seekable source's shared offset back to the consumed
    /// boundary (origin + taken). No-op for pipes/stdin, whose reads
    /// never over-consume. The seek is unconditional (zsyncfd,
    /// mapfile.def:195): once a callback may have read through a dup
    /// of the handle, an empty buffer no longer proves the shared
    /// position already sits at the boundary — the last sync may
    /// predate records the buffer alone supplied (rubash#3660).
    fn sync_offset(&mut self) {
        let Some(origin) = self.rewind else {
            return;
        };
        self.seek_source(origin + self.taken);
        self.next_fill_offset = self.taken;
    }

    /// The source's current OS-level position, when it has one.
    fn source_position(&mut self) -> Option<u64> {
        match &mut self.source {
            MapfilePullSource::Handle { handle, .. } => crate::fd::file_position(*handle).ok(),
            MapfilePullSource::File { file, .. } => std::io::Seek::stream_position(file).ok(),
            MapfilePullSource::StagedText { .. } | MapfilePullSource::InheritedStdin => None,
        }
    }

    fn seek_source(&mut self, position: u64) {
        match &mut self.source {
            MapfilePullSource::Handle { handle, .. } => {
                let _ = crate::fd::seek_absolute(*handle, position);
            }
            MapfilePullSource::File { file, .. } => {
                let _ = std::io::Seek::seek(file, std::io::SeekFrom::Start(position));
            }
            MapfilePullSource::InheritedStdin => {}
            MapfilePullSource::StagedText { .. } => {}
        }
    }

    /// Bind this pull source to the input descriptor for one
    /// callback invocation (rubash#3660). A puller built from an
    /// fd-table endpoint rebinds that endpoint (see home_endpoint).
    /// A path-opened file is private to the puller, so bind a dup of
    /// its handle: the dup shares the open file description (one
    /// offset for both readers), and dropping the endpoint at unbind
    /// closes only the dup. Inherited stdin is the process stream —
    /// shared by construction.
    fn begin_callback_share(
        &mut self,
        executor: &mut Executor,
        input_fd: u32,
    ) -> MapfileCallbackShare {
        // A puller built from an fd-table endpoint serves the stream
        // that endpoint names; bind that endpoint at the input
        // descriptor for the callback — the same stream, reached
        // through the descriptor the callback reads (rubash#3660).
        if let Some(endpoint) = &self.home_endpoint {
            return executor.bind_mapfile_callback_stream(input_fd, endpoint.clone());
        }
        match &self.source {
            MapfilePullSource::File { file, path } => match file.try_clone() {
                Ok(dup) => executor.bind_mapfile_callback_stream(
                    input_fd,
                    FdReadEndpoint::File(crate::executor::fd_table::FileFd::from_file(
                        dup,
                        path.clone(),
                    )),
                ),
                Err(_) => MapfileCallbackShare::Unbound,
            },
            // A staged-text puller needs no endpoint bind: the share
            // IS the FUNCTION_STDIN mirror mapfile_source_sync just
            // published `origin + taken` into — the callback's fd-0
            // reads carve that mirror through function_call_stdin
            // (function_calls.rs), and end_callback_share folds the
            // advanced offset back into `taken` (rubash#3660 ×
            // rubash#2779).
            MapfilePullSource::Handle { .. }
            | MapfilePullSource::StagedText { .. }
            | MapfilePullSource::InheritedStdin => MapfileCallbackShare::Unbound,
        }
    }

    /// Fold bytes the callback consumed through the shared
    /// descriptor back into the puller's bookkeeping (rubash#3660).
    /// The descriptor's position beyond the driver's logical cursor
    /// (origin + taken) is what the callback read: those bytes leave
    /// the buffered prefix first, then count as taken, and the
    /// source is re-aligned so the next fill continues exactly
    /// behind them. Unseekable sources never moved a shared offset,
    /// so there is nothing to fold.
    fn resync_after_callback(&mut self) {
        let Some(origin) = self.rewind else {
            return;
        };
        let Some(position) = self.source_position() else {
            return;
        };
        let logical = origin + self.taken;
        if position <= logical {
            return;
        }
        let consumed = position - logical;
        let from_buffer = (consumed as usize).min(self.buffer.len());
        self.buffer.drain(..from_buffer);
        self.taken += consumed;
        self.seek_source(origin + self.taken);
        self.next_fill_offset = self.taken;
    }

    /// Fold the FUNCTION_STDIN_OFFSET advancement a callback left
    /// behind back into a staged-text puller's bookkeeping
    /// (rubash#3660 × rubash#2779). For a StagedText source the share
    /// is the FUNCTION_STDIN mirror itself: mapfile_source_sync
    /// published `origin + taken` before the callback ran, the call's
    /// function_call_stdin carve advanced the offset by however much
    /// the callback read, and those bytes leave the buffered prefix
    /// exactly like a handle's shared-offset advance — one fd, one
    /// cursor (builtins/mapfile.def:195-197 zsyncfd). No-op for every
    /// other source kind.
    fn fold_staged_callback_consumption(&mut self, executor: &Executor) {
        let MapfilePullSource::StagedText { origin, .. } = &self.source else {
            return;
        };
        let published = *origin + self.taken;
        let current = executor
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(published);
        if current <= published {
            return;
        }
        // The staged fill materialized the whole unread tail, so the
        // consumed prefix always sits at the buffer's head; the min
        // only guards a cursor that somehow ran past it.
        let consumed = (current - published) as usize;
        let drained = consumed.min(self.buffer.len());
        self.buffer.drain(..drained);
        self.taken += drained as u64;
    }
}

/// Raw-byte record to shell text, in the slurp path's domain
/// (split_mapfile_input): NUL bytes truncate the segment unless NUL
/// is itself the delimiter, then bytes convert via the same locale
/// boundary the whole-input path uses.
fn finish_mapfile_record(mut raw: Vec<u8>, delimiter: u8) -> String {
    if delimiter != 0 {
        if let Some(pos) = raw.iter().position(|byte| *byte == 0) {
            raw.truncate(pos);
        }
    }
    bytes_to_shell_text(&raw)
}
