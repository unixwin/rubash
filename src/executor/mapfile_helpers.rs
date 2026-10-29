use super::types::INHERIT_PROCESS_STDIN;
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
                    if !target.starts_with("<(") && !is_closed_redirect_target(&target) {
                        // niubash#177: same operand resolution as
                        // open_input_redirect — a virtual-root input
                        // operand may exist only under its `.exe` spelling.
                        let path =
                            self.session_path(crate::executor::path::windows_operand_file_path(
                                shell_path_to_windows(&target, &self.shell_state.env_vars),
                                &self.shell_state.env_vars,
                            ));
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

        if let Some(input) = self.mapfile_redirected_fd_input(cmd, fd) {
            return Some(input);
        }
        if let Some(input) = self.mapfile_virtual_fd_input(fd) {
            return Some(input);
        }
        self.mapfile_heredoc_fd_input(cmd, fd)
    }

    pub(in crate::executor) fn mapfile_fd_is_available(&self, cmd: &CommandNode, fd: u32) -> bool {
        if self.fd_table.is_open_for_read(fd) {
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
        let Some(mut source) = self.mapfile_record_source(cmd, read_fd, delimiter, trim_newline)
        else {
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
                source.sync_offset();
                self.execute_mapfile_callback(callback, target, &record, cmd)?;
            }
            let previous_max = entries.keys().next_back().copied();
            entries.insert(target, record.clone());
            self.bind_mapfile_element_live(name, &entries, target, &record, previous_max);
        }
        source.sync_offset();
        // Canonical final render: byte-identical to the shape the
        // slurp path stores for the same input.
        self.shell_state
            .env_vars
            .insert(name.to_string(), format_indexed_array_storage(entries));
        mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", name);
        let _ = self.write_buffered_builtin_output(cmd, &[], stderr);
        Ok(Some(0))
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

    /// One fd's record source: real handles pull incrementally;
    /// text / process-substitution endpoints are already materialized,
    /// so they are drained whole (as the slurp path drains them) and
    /// pre-split — the callbacks still fire between records.
    fn mapfile_endpoint_source(
        &mut self,
        fd: u32,
        delimiter: Option<u8>,
        trim_newline: bool,
    ) -> Option<MapfileRecordSource> {
        let endpoint = self.fd_table.read_endpoint(fd)?;
        if matches!(
            endpoint,
            FdReadEndpoint::Text(_) | FdReadEndpoint::ProcessSubstitution(_)
        ) {
            let bytes = self.fd_table.read_all_bytes(fd)?;
            let text = String::from_utf8(bytes)
                .unwrap_or_else(|error| String::from_utf8_lossy(&error.into_bytes()).into_owned());
            return Some(MapfileRecordSource::values(text, delimiter, trim_newline));
        }
        MapfileRecordSource::from_endpoint(&endpoint)
    }

    /// Resolve where the callback form reads its records from,
    /// mirroring mapfile_input_for_command's source order, but keeping
    /// real fds/files incremental (MapfileRecordPuller) instead of
    /// slurping them: only the already-materialized sources (command
    /// redirects staged for another fd, command string input, text /
    /// process-substitution endpoints) are drained and pre-split, and
    /// even those fire their callbacks between records.
    fn mapfile_record_source(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        delimiter: Option<u8>,
        trim_newline: bool,
    ) -> Option<MapfileRecordSource> {
        if let Some(fd) = read_fd.filter(|fd| *fd != 0) {
            let staged_redirect = cmd.redirects.iter().any(|redirect| redirect.fd == Some(fd));
            if !staged_redirect {
                if let Some(source) = self.mapfile_endpoint_source(fd, delimiter, trim_newline) {
                    return Some(source);
                }
            }
            return self
                .mapfile_input_for_command(cmd, read_fd)
                .map(|input| MapfileRecordSource::values(input, delimiter, trim_newline));
        }
        // fd 0 domain, in mapfile_input_for_command's order.
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_redirect_target(redirect);
                if !target.starts_with("<(") && !is_closed_redirect_target(&target) {
                    let path = self.session_path(crate::executor::path::windows_operand_file_path(
                        shell_path_to_windows(&target, &self.shell_state.env_vars),
                        &self.shell_state.env_vars,
                    ));
                    if let Ok(mut file) = std::fs::File::open(&path) {
                        let origin = std::io::Seek::stream_position(&mut file).unwrap_or(0);
                        return Some(MapfileRecordSource::Pull(MapfileRecordPuller {
                            source: MapfilePullSource::File(file),
                            buffer: Vec::new(),
                            eof: false,
                            taken: 0,
                            rewind: Some(origin),
                            next_fill_offset: 0,
                        }));
                    }
                    // Open failed: fall through — the string chain
                    // fails the same read and moves on, as the slurp
                    // path does.
                }
            }
        }
        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            return Some(MapfileRecordSource::values(input, delimiter, trim_newline));
        }
        if self.fd_table.is_open_for_read(0) {
            if let Some(source) = self.mapfile_endpoint_source(0, delimiter, trim_newline) {
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
enum MapfileRecordSource {
    /// Already-materialized input, pre-split in the shell-text domain.
    Values(std::vec::IntoIter<String>),
    /// Incremental pull in the raw-byte domain.
    Pull(MapfileRecordPuller),
}

impl MapfileRecordSource {
    fn values(input: String, delimiter: Option<u8>, trim_newline: bool) -> Self {
        MapfileRecordSource::Values(
            split_mapfile_input(&input, delimiter, trim_newline).into_iter(),
        )
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
                }))
            }
            // rubash#1937: a directory endpoint has no record
            // source; the read face itself reports EISDIR.
            FdReadEndpoint::Directory => None,
        }
    }

    fn next_record(&mut self, delimiter: u8, trim_newline: bool) -> Option<String> {
        match self {
            MapfileRecordSource::Values(values) => values.next(),
            MapfileRecordSource::Pull(puller) => puller.next_record(delimiter, trim_newline),
        }
    }

    fn sync_offset(&mut self) {
        if let MapfileRecordSource::Pull(puller) = self {
            puller.sync_offset();
        }
    }
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
    /// A plain file opened by path for the `< file` form.
    File(std::fs::File),
    /// The process's inherited stdin (byte-wise, like a pipe).
    InheritedStdin,
}

/// Pull-based record reader over raw bytes. `taken` counts the bytes
/// extracted into records so far; for seekable sources `rewind`
/// holds the origin position so sync_offset can move the shared
/// offset back to the consumed boundary — over-read bytes stay
/// valid in `buffer`, and `next_fill_offset` makes the next fill
/// skip the overlap it re-reads.
struct MapfileRecordPuller {
    source: MapfilePullSource,
    buffer: Vec<u8>,
    eof: bool,
    taken: u64,
    rewind: Option<u64>,
    next_fill_offset: u64,
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
            MapfilePullSource::File(file) => {
                let mut buf = [0u8; 8192];
                match std::io::Read::read(file, &mut buf) {
                    Ok(n) if n > 0 => buf[..n].to_vec(),
                    _ => {
                        self.eof = true;
                        return;
                    }
                }
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
    /// never over-consume.
    fn sync_offset(&mut self) {
        let Some(origin) = self.rewind else {
            return;
        };
        if self.buffer.is_empty() {
            return;
        }
        let target = origin + self.taken;
        match &mut self.source {
            MapfilePullSource::Handle { handle, .. } => {
                let _ = crate::fd::seek_absolute(*handle, target);
            }
            MapfilePullSource::File(file) => {
                let _ = std::io::Seek::seek(file, std::io::SeekFrom::Start(target));
            }
            MapfilePullSource::InheritedStdin => {}
        }
        self.next_fill_offset = self.taken;
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
