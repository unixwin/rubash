use super::*;

enum SelectBodyFlow {
    ContinueLoop,
    BreakLoop,
}

impl Executor {
    pub(in crate::executor) fn execute_select_command(
        &mut self,
        cmd: &CommandNode,
        select_command: &SelectCommand,
    ) -> Result<(), ExecuteError> {
        // RESTRICTED_SHELL (#804 N1): the select loop's redirects apply
        // before the menu runs (do_redirections precedes the body, like
        // every compound), so the refusal lands before
        // with_command_input_redirects opens or binds anything — GNU
        // prints no menu and leaves no file (`select x in a; do :; done >
        // f` refuses `f`, rc 1).
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        if !is_shell_name(&select_command.variable) {
            // GNU execute_select_command (execute_cmd.c:3001-3010) runs
            // check_identifier BEFORE `line_number = select_command->line`
            // (3013) — the reverse of execute_for_command (2996-2999),
            // which stamps first. The failure therefore reports the
            // AMBIENT line: the function body-open line inside a function
            // (execute_cmd.c:5351 tc->line), or the reader's last line at
            // top level — never the `select` keyword's own line
            // (rubash#201 repro B: `{` on line 2, select on line 3, GNU
            // reports 2). The report has no command segment:
            // "./errors.tests: line 50: `1': not a valid identifier".
            let line = self.ambient_line.get().or(cmd.line).unwrap_or(1);
            eprintln!(
                "{}`{}': not a valid identifier",
                self.diagnostic_prefix_for_line(line),
                select_command.variable
            );
            self.exit_code = if self.posix_mode_enabled() { 2 } else { 1 };
            if self.posix_mode_enabled() {
                return Err(ExecuteError::ExitCode(2));
            }
            return Ok(());
        }

        // GNU execute_cmd.c:3525-3528: the select head is printed
        // (print_select_command_head, print_cmd.c:656 -> `select NAME in
        // WORDS`) and run_debug_trap fires once before the word list is
        // expanded; the implicit `select x; do` form prints the default
        // `"$@"` list, the same as the `for` head.
        let words_text = if select_command.default_positional {
            "\"$@\"".to_string()
        } else {
            crate::executor::command_text::command_words_source_text(
                &select_command.words,
                &select_command.word_metadata,
            )
        };
        // GNU execute_cmd.c:3513 `line_number = select_command->line` runs
        // before both the debug fire and the xtrace head print (3536
        // xtrace_print_select_command_head, print_cmd.c:655
        // `select %s in ` + words), so PS4's $LINENO renders the select
        // keyword's line (rubash#275).
        if let Some(line) = cmd.line {
            self.set_current_line_value(line);
        }
        if self.debug_trap_in_scope() {
            // execute_cmd.c:3528-3536: a non-zero trap status under
            // debugging_mode returns EXECUTION_SUCCESS — the select command
            // never expands its word list nor runs.
            if self
                .run_debug_trap(
                    crate::debugger::SeamKind::SelectHead,
                    &format!("select {} in {}", select_command.variable, words_text),
                )?
                .is_some()
            {
                self.exit_code = 0;
                return Ok(());
            }
        }
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            self.xtrace_write(
                format!(
                    "{prefix}select {} in {}\n",
                    select_command.variable, words_text
                )
                .as_bytes(),
            );
        }

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut select_command = select_command.clone();
        let mut body = Ast {
            commands: select_command.body,
        };
        let result = self.apply_command_output_redirects(&redirect_cmd, &mut body);
        let status = self.exit_code;
        if let Err(error) = result {
            // rubash#846 (GNU execute_cmd.c:854-883): the select's OWN
            // redirect failure — mark it for the `!` flip sites.
            if matches!(error, ExecuteError::IoError(_)) {
                self.note_compound_redirect_failure();
            }
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        select_command.body = body.commands;

        let values: Vec<String> = if select_command.default_positional {
            self.shell_state.positional_window_cloned()
        } else {
            let mut values = Vec::new();
            for (index, word) in select_command.words.iter().enumerate() {
                let raw = select_command
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                let metadata = select_command.word_metadata.get(index);
                match self.expand_for_word_values_result(word, raw, metadata) {
                    Ok(expanded) => values.extend(expanded),
                    Err(pattern) => {
                        self.report_failglob(&pattern);
                        self.finish_compound_output_process_substitutions(group_outputs)?;
                        // failglob is a fatal word-expansion error (GNU):
                        // the select command fails with status 1 and the
                        // next line runs.
                        // rubash#753: arm the reader-level abandon alongside
                        // the Err (same contract as the for head) so a
                        // compound carrying the select cannot resume the
                        // failing line.
                        self.raise_evalerror_abort();
                        self.evalerror_failglob_pending.set(true);
                        return Err(ExecuteError::ExpansionFailure(1));
                    }
                }
            }
            values
        };

        if values.is_empty() {
            self.exit_code = 0;
            self.finish_compound_output_process_substitutions(group_outputs)?;
            return Ok(());
        }

        // GNU execute_cmd.c:3513 `line_number = select_command->line`: the
        // `select` keyword's line is the ambient while the body runs.
        let select_line = cmd.line;
        let result = self.with_loop_fd_heredocs(cmd, |executor| {
            executor.with_command_input_redirects(cmd, |executor| {
                executor.with_ambient_line(select_line, |executor| {
                    executor.execute_select_loop(&select_command, &values)
                })
            })
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_select_loop(
        &mut self,
        select_command: &SelectCommand,
        values: &[String],
    ) -> Result<(), ExecuteError> {
        let has_stdin = self.shell_state.env_vars.contains_key(FUNCTION_STDIN);
        let mut stdin_offset = self.select_stdin_offset(has_stdin);

        // GNU execute_cmd.c:3495-3631 execute_select_command with
        // config-top.h:89 KSH_COMPATIBLE_SELECT: the menu prints once
        // (show_menu starts 1); after each body run only the prompt
        // prints, unless the body emptied REPLY.
        let mut show_menu = true;
        loop {
            // GNU execute_cmd.c:3566-3570: PS3 is re-read every iteration,
            // so a body assignment retargets the next prompt.
            let ps3 = self
                .shell_state
                .env_vars
                .get("PS3")
                .cloned()
                .unwrap_or_else(|| "#? ".to_string());
            let mut prompt = Vec::new();
            if show_menu {
                // GNU execute_cmd.c:3375-3418 print_select_list: the menu
                // wraps into columns against COLS (general.c:1461
                // default_columns), printed column-major with tab-aligned
                // column offsets — not one item per line.
                self.write_select_menu(&mut prompt, values)?;
            }
            write!(&mut prompt, "{ps3}")?;
            self.write_default_stderr(&prompt)?;

            let (input, at_eof) = self.read_select_line(has_stdin, &mut stdin_offset);
            // GNU builtins/read.def:987-1011 (no NAMEs, non-raw call from
            // select_query): every read attempt lands in REPLY verbatim —
            // no IFS strip — with backslash unescaping, including the
            // empty string on clean EOF and the partial line on an
            // unterminated tail.
            let reply = unescape_read_backslashes(&input);
            if !self.apply_shell_assignment("REPLY", reply.clone()) {
                // GNU: the read_builtin bind failure fails the query —
                // stdout newline, no body, status failure (rubash#1018
                // probe: `readonly REPLY` prints `REPLY: readonly
                // variable`, skips the body, exits 1).
                let _ = self.write_default_stdout(b"\n");
                self.exit_code = 1;
                if self.posix_assignment_failure_is_fatal() {
                    let code = self.expansion_fatal_status();
                    self.exit_code = code;
                    return Err(ExecuteError::ExitCode(code));
                }
                return Ok(());
            }
            set_process_env("REPLY", &reply);
            if at_eof {
                // GNU zread/read.def: an unterminated tail still binds
                // REPLY but fails the read, so the menu is done — stdout
                // newline, status 1, no body run.
                let _ = self.write_default_stdout(b"\n");
                self.exit_code = 1;
                return Ok(());
            }
            if reply.is_empty() {
                // GNU execute_cmd.c:3473-3477 select_query: an empty REPLY
                // re-arms the menu and reprompts without running the body.
                // The check is on the bound REPLY (post-unescape), so a
                // whitespace-only line is NOT empty — it runs the body
                // with an empty selection (valid_number fails on it).
                show_menu = true;
                continue;
            }

            let selected = select_reply_selection(&reply, values).unwrap_or_default();
            // GNU execute_cmd.c execute_select_command binds the reply
            // through bind_variable: nameref select variables follow full
            // assignment semantics (empty-cell nameref stores the cell,
            // invalid values report `` `x': not a valid identifier `` —
            // nameref11.sub `select r in /` on `declare -n r`).
            if !self.apply_shell_assignment(&select_command.variable, selected.clone()) {
                self.exit_code = 1;
                // POSIX-mode notes item 37 covers select alongside for: a
                // readonly select variable exits the non-interactive POSIX
                // shell. GNU execute_cmd.c:3587-3592
                // (execute_select_command) sets last_command_exit_value =
                // EXECUTION_FAILURE and jumps FORCE_EOF; the status is
                // context-owned — shell.c:1471-1472 run_one_command remaps
                // FORCE_EOF to 127 under -c, eval.c:104-109 keeps 1 for
                // scripts, execute_cmd.c:1811 contains it at 1 inside
                // subshell children.
                if self.posix_assignment_failure_is_fatal() {
                    let code = self.expansion_fatal_status();
                    self.exit_code = code;
                    return Err(ExecuteError::ExitCode(code));
                }
                return Ok(());
            }
            set_process_env(&select_command.variable, selected);

            let body_flow = self.execute_select_body(&select_command.body)?;
            // GNU execute_cmd.c:3606 REAP() (execute_select_command).
            self.reap_dead_jobs_after_loop_body();
            match body_flow {
                SelectBodyFlow::ContinueLoop => {
                    // GNU config-top.h:89 KSH_COMPATIBLE_SELECT tail: the
                    // menu stays hidden unless the body emptied REPLY (an
                    // unset REPLY is NULL, not empty — the menu stays
                    // hidden).
                    show_menu = self
                        .shell_state
                        .env_vars
                        .get("REPLY")
                        .is_some_and(|current| current.is_empty());
                    continue;
                }
                SelectBodyFlow::BreakLoop => break,
            }
        }

        Ok(())
    }

    fn select_stdin_offset(&self, has_stdin: bool) -> usize {
        if has_stdin {
            self.shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
        } else {
            0
        }
    }

    fn read_select_line(&mut self, has_stdin: bool, stdin_offset: &mut usize) -> (String, bool) {
        // Logical line for select_query's read_builtin(NULL) call: GNU
        // builtins/read.def:802-830 reads char-by-char (non-raw), so a
        // `\<newline>` pair vanishes and reading continues with the next
        // physical line; a trailing-backslash run of even length is an
        // escaped backslash (no continuation — `a\\` binds `a\`). EOF,
        // clean or as an unterminated tail, fails the read in every case.
        // No trimming: select_query parses and emptiness-checks the bound
        // REPLY value, and the no-NAMEs path leaves surrounding
        // whitespace intact.
        let mut logical = String::new();
        loop {
            let (physical, at_eof) = match self.read_select_physical_line(has_stdin, stdin_offset) {
                None => return (logical, true),
                Some(pair) => pair,
            };
            logical.push_str(&physical);
            if at_eof {
                return (logical, true);
            }
            if trailing_backslash_run(&logical) % 2 == 1 {
                // GNU backs up over the CTLESC: drop the continuating
                // backslash (one ASCII byte) and read on.
                logical.pop();
                continue;
            }
            return (logical, false);
        }
    }

    fn read_select_physical_line(
        &mut self,
        has_stdin: bool,
        stdin_offset: &mut usize,
    ) -> Option<(String, bool)> {
        // GNU select_query's read_builtin zreads fd 0 exactly as bound
        // (read.def:294): a CLOSED fd 0 fails with EBADF and read.def
        // reports `0: read error: Bad file descriptor`, after which the
        // read fails as EOF (REPLY empty, stdout newline, status 1).
        // Rubash fell through to the raw process stdin — blocking on a
        // live pipe, or a silent EOF where the descriptor model lost
        // the closure (rubash#1299).
        if self.fd_table.is_closed(0) {
            let message = format!(
                "{}0: read error: Bad file descriptor\n",
                self.diagnostic_prefix()
            );
            let _ = self.write_default_stderr(message.as_bytes());
            return None;
        }
        // Returns the raw physical line WITHOUT the trailing newline plus
        // whether it is an unterminated EOF tail; None is clean EOF with
        // no bytes.
        //
        // GNU do_redirections binds the compound's fd 0 before the select
        // loop runs and select_query's read_builtin consumes THAT
        // descriptor (read.def:294): `done < file` binds the FileFd from
        // bind_command_fd0_input, `done <&N` binds a dup of fd N through
        // open_compound_numbered_input_redirects. Both must be served
        // record-wise off the fd table — reading the raw process stdin
        // instead reported an instant EOF (rubash#1204). A buffered
        // record still reports EOF-no-delimiter like GNU's unterminated
        // tail.
        // rubash#1141: select's query is a read_builtin call
        // (execute_cmd.c:3426 select_query over read.def), so TMOUT
        // bounds it exactly like a read: an expired query returns NULL,
        // which the loop treats as EOF.
        let tmout_deadline = |executor: &Self| {
            executor
                .shell_state
                .env_vars
                .get("TMOUT")
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|t| t.is_finite() && *t > 0.0)
                .map(|s| std::time::Instant::now() + std::time::Duration::from_secs_f64(s))
        };
        if self.fd_table.fd0_bound_away_from_process_stdin() {
            self.fd_table.read_eof_no_delimiter = false;
            // Arm the fd table's read deadline so a materialized
            // endpoint's production schedule can judge the record
            // (fd_table::read_bytes). A regular file disables the
            // timeout (read.def:499-505 S_ISREG rule), matching
            // arm_read_deadline.
            let saved_deadline = self.fd_table.read_deadline;
            if saved_deadline.is_none()
                && !matches!(
                    self.fd_table.read_endpoint(0),
                    Some(FdReadEndpoint::File(file))
                        if crate::fd::is_disk_file(file.handle)
                )
            {
                self.fd_table.read_deadline = tmout_deadline(self);
            }
            let line = self.fd_table.read_text(0, '\n', None, false, false, false);
            self.fd_table.read_deadline = saved_deadline;
            if self.fd_table.read_timed_out {
                self.fd_table.read_timed_out = false;
                return None;
            }
            return match line {
                Some(line) => Some((line, self.fd_table.read_eof_no_delimiter)),
                None => None,
            };
        }
        if !has_stdin {
            // Raw handle read like GNU zread: io::stdin() owns a process-wide
            // BufReader that would prefetch the whole stream and break the
            // shared kernel offset for sibling reads.
            let stdin_handle = crate::fd::process_std_handle(0);
            // GNU execute_cmd.c select_query reads through the same
            // timeout machinery as `read`: TMOUT bounds the wait
            // (bash's -E manual: select uses TMOUT as its timeout), and
            // an expired query returns NULL exactly like EOF — the loop
            // above reports EXECUTION_FAILURE and prints the newline
            // (rubash#1141). A regular-file stdin disables the timeout
            // (read.def:499-505 S_ISREG rule, via arm_read_deadline's
            // parsing).
            let deadline = self
                .shell_state
                .env_vars
                .get("TMOUT")
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|t| t.is_finite() && *t > 0.0)
                .filter(|_| !crate::fd::is_disk_file(stdin_handle))
                .map(|s| std::time::Instant::now() + std::time::Duration::from_secs_f64(s));
            let mut line: Vec<u8> = Vec::new();
            let at_eof = loop {
                if let Some(deadline) = deadline {
                    let now = std::time::Instant::now();
                    if now >= deadline
                        || crate::fd::wait_readable(stdin_handle, deadline - now)
                            == crate::fd::ReadWait::Timeout
                    {
                        break true;
                    }
                }
                match crate::fd::read_some(stdin_handle, 1) {
                    Ok(buf) if buf.is_empty() => break true,
                    Ok(buf) => {
                        line.push(buf[0]);
                        if buf[0] == b'\n' {
                            break false;
                        }
                    }
                    Err(_) => {
                        self.exit_code = 1;
                        return Some((String::new(), false));
                    }
                }
            };
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            return Some((crate::executor::bytes_to_shell_text(&line), at_eof));
        }

        let stdin_content = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN)
            .cloned()
            .unwrap_or_default();
        // rubash#1141: judge the query against the stream's production
        // schedule (materialized pipeline input): a line — or the EOF —
        // that only arrives after TMOUT makes the query return NULL,
        // exactly like the fd-0 paths above. A partial line cut by the
        // deadline is discarded, as GNU's read timeout discards it
        // before select_query ever sees a value.
        if *stdin_offset >= stdin_content.len() {
            if let Some(timing) = self.timed_stdin.clone() {
                let mut timing = timing.borrow_mut();
                if timing.matches_len(stdin_content.len()) {
                    let _ = timing.judge(None, *stdin_offset, tmout_deadline(self));
                }
            }
            return None;
        }
        let remaining = &stdin_content[*stdin_offset..];
        let (input, at_eof, new_offset) = if let Some(newline_pos) = remaining.find('\n') {
            (
                remaining[..newline_pos].to_string(),
                false,
                *stdin_offset + newline_pos + 1,
            )
        } else {
            (remaining.to_string(), true, stdin_content.len())
        };
        if let Some(timing) = self.timed_stdin.clone() {
            let mut timing = timing.borrow_mut();
            if timing.matches_len(stdin_content.len()) {
                let record_end = if at_eof { None } else { Some(new_offset) };
                if matches!(
                    timing.judge(record_end, *stdin_offset, tmout_deadline(self)),
                    super::timed_input::TimedVerdict::Timeout { .. }
                ) {
                    return None;
                }
            }
        }
        *stdin_offset = new_offset;
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), stdin_offset.to_string());
        Some((input, at_eof))
    }

    /// GNU general.c:1461 default_columns: COLUMNS when it parses to a
    /// positive int, else the process console width (winsize.c
    /// get_new_window_size), else 80.
    fn select_default_columns(&self) -> usize {
        if let Some(cols) = self
            .shell_state
            .env_vars
            .get("COLUMNS")
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|cols| *cols > 0)
        {
            return cols as usize;
        }
        if let Some((_, cols)) = crate::script_driver::process_console_size() {
            if cols > 0 {
                return cols as usize;
            }
        }
        80
    }

    /// GNU execute_cmd.c:3375-3418 print_select_list — the column-major
    /// menu layout `select_query` prints before PS3. Column count comes
    /// from COLS / max_elem_len; separators are produced by `indent`
    /// (execute_cmd.c:3356), which advances to the next tab stop when one
    /// is crossed and pads with spaces otherwise (tabsize = 8).
    fn write_select_menu(&self, prompt: &mut Vec<u8>, values: &[String]) -> std::io::Result<()> {
        const TABSIZE: usize = 8;
        const RP_SPACE_LEN: usize = 2;
        let number_len = |value: usize| -> usize {
            match value {
                0..=9 => 1,
                10..=99 => 2,
                100..=999 => 3,
                1000..=9999 => 4,
                10000..=99999 => 5,
                _ => 6,
            }
        };
        let displen = |value: &str| value.chars().count();

        let cols_width = self.select_default_columns();
        let list_len = values.len();
        let mut max_elem_len = values.iter().map(|value| displen(value)).max().unwrap_or(0);
        let indices_len = number_len(list_len);
        max_elem_len += indices_len + RP_SPACE_LEN + 2;

        let mut cols = if max_elem_len > 0 {
            cols_width / max_elem_len
        } else {
            1
        };
        if cols == 0 {
            cols = 1;
        }
        let mut rows = if list_len > 0 {
            list_len / cols + usize::from(list_len % cols != 0)
        } else {
            1
        };
        cols = if list_len > 0 {
            list_len / rows + usize::from(list_len % rows != 0)
        } else {
            1
        };
        if rows == 1 {
            rows = cols;
            cols = 1;
        }
        let _ = cols;
        let first_column_indices_len = number_len(rows);

        for row in 0..rows {
            let mut ind = row;
            let mut pos = 0usize;
            loop {
                let this_indices_len = if pos == 0 {
                    first_column_indices_len
                } else {
                    indices_len
                };
                write!(
                    prompt,
                    "{:>width$}) {}",
                    ind + 1,
                    values[ind],
                    width = this_indices_len
                )?;
                let elem_len = displen(&values[ind]) + this_indices_len + RP_SPACE_LEN;
                ind += rows;
                if ind >= list_len {
                    break;
                }
                // indent(pos + elem_len, pos + max_elem_len): tabs to the
                // next stop while one is crossed, spaces otherwise.
                let mut from = pos + elem_len;
                let to = pos + max_elem_len;
                while from < to {
                    if to / TABSIZE > from / TABSIZE {
                        prompt.push(b'\t');
                        from += TABSIZE - from % TABSIZE;
                    } else {
                        prompt.push(b' ');
                        from += 1;
                    }
                }
                pos += max_elem_len;
            }
            prompt.push(b'\n');
        }
        Ok(())
    }

    fn execute_select_body(
        &mut self,
        body: &[CommandNode],
    ) -> Result<SelectBodyFlow, ExecuteError> {
        let body = Ast {
            commands: body.to_vec(),
        };
        self.shell_state.loop_depth += 1;
        let result = self.execute_ast(&body);
        self.shell_state.loop_depth -= 1;
        match result {
            Ok(()) => Ok(SelectBodyFlow::ContinueLoop),
            Err(ExecuteError::Break(level)) if level <= 1 => {
                self.exit_code = self.take_break_failure_status().unwrap_or(0);
                Ok(SelectBodyFlow::BreakLoop)
            }
            Err(ExecuteError::Break(level)) => Err(ExecuteError::Break(level - 1)),
            Err(ExecuteError::Continue(level)) if level <= 1 => {
                self.exit_code = 0;
                Ok(SelectBodyFlow::ContinueLoop)
            }
            Err(ExecuteError::Continue(level)) => Err(ExecuteError::Continue(level - 1)),
            Err(error) => Err(error),
        }
    }
}

/// Length of the trailing `\` byte run. GNU builtins/read.def:802-830
/// swallows a `\<newline>` pair mid-stream, so continuation fires exactly
/// for an odd run (the last backslash is unescaped); an even run is
/// escaped backslashes with no continuation.
fn trailing_backslash_run(s: &str) -> usize {
    s.as_bytes()
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
}

/// GNU general.c:248 valid_number (strtoimax, base 10) as used by
/// execute_cmd.c:3479 select_query: leading C-locale whitespace is
/// skipped, an optional sign and ASCII digits are consumed, trailing
/// whitespace is skipped, and the whole string must convert with no
/// overflow. Returns the selected value for 1..=len, None otherwise
/// (select_query returns "" for both invalid and out-of-range input —
/// the body still runs with an empty selection).
fn select_reply_selection(reply: &str, values: &[String]) -> Option<String> {
    fn is_c_space(byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
    }
    let bytes = reply.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_c_space(bytes[i]) {
        i += 1;
    }
    let mut j = i;
    if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
        j += 1;
    }
    let digits_start = j;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    if j == digits_start {
        return None;
    }
    let mut k = j;
    while k < bytes.len() && is_c_space(bytes[k]) {
        k += 1;
    }
    if k != bytes.len() {
        return None;
    }
    // Slice boundaries sit on ASCII whitespace/sign/digits, so this
    // cannot split a UTF-8 sequence. i64 overflow (strtoimax errno)
    // fails the conversion, matching valid_number.
    let n: i64 = reply[i..j].parse().ok()?;
    if n < 1 || n as u64 > values.len() as u64 {
        return None;
    }
    Some(values[n as usize - 1].clone())
}
