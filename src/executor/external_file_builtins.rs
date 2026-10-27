use super::*;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::shell::var_table::VarTable;
use std::io::IsTerminal;

impl Executor {
    pub(in crate::executor) fn handle_external_file_builtins(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if !self.external_file_builtins_enabled {
            return Ok(false);
        }
        let name = cmd.words[0].as_str();
        // GNU findcmd.c search_for_command: with PATH unset no external
        // command resolves — and a set PATH holding no such command
        // fails the lookup identically (rubash#1545). The emulations
        // below stand in for PATH binaries, so a slash-free name must
        // fall through to the (failing) lookup instead of executing
        // (rubash#1126). Slash names go straight to execve in GNU and
        // keep working.
        if !name.contains('/') && !name.contains('\\') && self.emulated_external_stands_down(name) {
            return Ok(false);
        }
        let emulated = matches!(
            name,
            "/bin/pwd"
                | "/usr/bin/pwd"
                | "/bin/printf"
                | "/usr/bin/printf"
                | "mkdir"
                | "touch"
                | "chmod"
                | "cp"
                | "rm"
                | "rmdir"
                | "cat"
                | "/bin/cat"
                | "/usr/bin/cat"
                | "sed"
                | "mkfifo"
                | "tty"
                | "/bin/tty"
                | "/usr/bin/tty"
        );
        // GNU findcmd.c:365/416 (search_for_command): a plain name resolved
        // through PATH for execution enters the hash table with
        // times_found=1. The emulated commands below stand in for that PATH
        // binary, so record the same resolution — `hash -t`/`hash -l`/
        // `BASH_CMDS` must see it (builtins9.sub: a stale `hash -p` entry
        // forgotten under checkhash is re-recorded by the next run).
        if emulated
            && !name.contains('/')
            && !name.contains('\\')
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "hashall")
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_TEMP_PATH")
                .map(String::as_str)
                != Some("1")
        {
            if let Some(program) =
                crate::executor::path::find_user_command(name, &self.shell_state.env_vars)
            {
                let display = super::execution_misc::shell_display_path(
                    &program.to_string_lossy().replace('\\', "/"),
                );
                crate::builtins::hash::record_command_resolution(
                    &mut self.shell_state.env_vars,
                    name,
                    &display,
                );
            }
        }
        // GNU redir.c do_redirections opens every redirection in the
        // forked child before exec: file targets are created/truncated
        // and a failed open kills the command with status 1. The
        // emulations below re-open only their fd 1/2 mirrors when they
        // write, so without this walk an fd>=3 target was never opened
        // at all and a failing one was silently ignored (rubash#1593:
        // `cat 3>f` created nothing, `cat 3>sub/f` ran with rc 0).
        if self.apply_redirect_setup_side_effects(cmd)? {
            return Ok(true);
        }
        match cmd.words[0].as_str() {
            "/bin/pwd" | "/usr/bin/pwd" => {
                let mut pwd_cmd = cmd.clone();
                pwd_cmd.words[0] = "pwd".to_string();
                self.exit_code = self.execute_pwd(&pwd_cmd)?;
                Ok(true)
            }
            "/bin/printf" | "/usr/bin/printf" => {
                let mut printf_cmd = cmd.clone();
                printf_cmd.words[0] = "printf".to_string();
                self.exit_code = self.execute_printf(&printf_cmd)?;
                Ok(true)
            }
            "mkdir" => self.external_mkdir(cmd),
            "touch" => self.external_touch(cmd),
            "chmod" => self.external_chmod(cmd),
            "cp" => self.external_cp(cmd),
            "rm" => self.external_rm(cmd),
            "rmdir" => self.external_rmdir(cmd),
            "cat" | "/bin/cat" | "/usr/bin/cat" => self.external_cat(cmd),
            "sed" => self.external_sed(cmd),
            "mkfifo" => self.external_mkfifo(cmd),
            "tty" | "/bin/tty" | "/usr/bin/tty" => self.external_tty(cmd),
            // #800A: the tput arm is Windows-only (unix keeps the real
            // ncurses binary, which reads the authoritative system terminfo).
            // It ADMITS only (TERM, invocation) pairs verified byte-for-byte
            // against the GNU oracle and falls through to the real PATH
            // binary for everything else, so non-admitted behavior is
            // unchanged (src/executor/tput_emulation.rs header).
            #[cfg(windows)]
            "tput" | "/bin/tput" | "/usr/bin/tput" => self.external_tput(cmd),
            _ => Ok(false),
        }
    }

    /// #800A (eco-harvest SLOW list): the harvest runner paid ~300ms per
    /// `$(tput …)` spawn — kiddae/dotfiles hermine `.bashrc` sources in 9.7s
    /// across ~32 calls. Rubash's spawn floor is at parity (~0.5ms/spawn,
    /// benchmarks/29); the delta lives inside the tput child, so the arm
    /// answers admitted shapes in-process and falls through to the real
    /// binary otherwise. Admission = `resolve_admitted_tput`'s whitelist;
    /// the CHILD-VISIBLE TERM is what decides, i.e. a tempenv `TERM=x
    /// tput …` override (apply_external_environment order: last assignment
    /// wins) over the exported shell-state value — a TERM the spawned
    /// child could not see (unexported shell variable) must not be
    /// answered here either.
    #[cfg(windows)]
    fn external_tput(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let Some(outcome) = self.admitted_tput_outcome(cmd) else {
            return Ok(false);
        };
        self.record_tput_resolution(cmd);
        match outcome {
            crate::executor::tput_emulation::TputOutcome::Bytes(bytes) => {
                self.write_cat_output(cmd, bytes.as_bytes())?;
                self.exit_code = 0;
            }
            // ncurses: an absent capability prints nothing, exits 1
            // (verified `TERM=vt100 tput setaf 1`, `TERM=linux tput smcup`).
            crate::executor::tput_emulation::TputOutcome::Absent => {
                self.exit_code = 1;
            }
        }
        Ok(true)
    }

    /// The TERM value the spawned child would actually see: a tempenv
    /// `TERM=… tput …` override (expanded exactly like
    /// apply_external_environment expands it — last assignment wins,
    /// append-assignments skip like the child-env pass skips them) over
    /// the EXPORTED shell-state value. `None` = no child-visible TERM or
    /// an empty one: GNU errors (rc 2) and the real binary keeps that
    /// face, so the arm stands down.
    #[cfg(windows)]
    fn child_visible_term(&mut self, cmd: &CommandNode) -> Option<String> {
        let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
        let mut override_term: Option<String> = None;
        let mut saw_append = false;
        for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
            let (base_name, append) = assignment_name_and_append(var_name);
            if base_name != "TERM" {
                continue;
            }
            let raw = raws_aligned
                .then(|| cmd.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            let expanded_value = match expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
            {
                Some(rest) => rest.to_string(),
                None => expanded_value,
            };
            // A nameref redirecting TERM elsewhere binds the target, not
            // TERM (variables.c assign_in_env) — treat as no TERM visible.
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if env_name != "TERM" {
                continue;
            }
            if append {
                saw_append = true;
                continue;
            }
            if is_valid_process_env(&env_name, &expanded_value) {
                override_term = Some(expanded_value);
            }
        }
        if saw_append {
            // `TERM+=x` semantics differ from a plain spawn's inheritance;
            // leave the whole question to the real binary.
            return None;
        }
        if let Some(term) = override_term {
            return (!term.is_empty()).then_some(term);
        }
        // Exported shell state only: apply_child_environment crosses the
        // process boundary for EXPORTED_VARS (readonly_functions.rs), so
        // an unexported `TERM=xterm` never reaches a spawned tput.
        let term = self.shell_state.env_vars.get("TERM")?;
        crate::executor::parse_helpers::is_marked_var(
            &self.shell_state.env_vars,
            crate::executor::types::EXPORTED_VARS,
            "TERM",
        )
        .then(|| term.clone())
        .filter(|value| !value.is_empty())
    }

    /// Admission for one invocation shape, shared by the simple-command arm
    /// (`external_tput`) and the pipeline-stage inline arm: `Some(outcome)`
    /// answers in-process, `None` falls through to the real PATH binary.
    #[cfg(windows)]
    pub(in crate::executor) fn admitted_tput_outcome(
        &mut self,
        command: &CommandNode,
    ) -> Option<crate::executor::tput_emulation::TputOutcome> {
        let term = self.child_visible_term(command)?;
        crate::executor::tput_emulation::resolve_admitted_tput(&term, &command.words[1..])
    }

    /// The `&self` variant for the command-substitution external fast path
    /// (`run_external_command_substitution`): that route carries no tempenv
    /// assignments, so the child-visible TERM is the EXPORTED shell-state
    /// value alone — the same value `apply_child_environment` would hand
    /// the spawned child below.
    #[cfg(windows)]
    pub(in crate::executor) fn admitted_tput_outcome_for_words(
        &self,
        words: &[String],
    ) -> Option<crate::executor::tput_emulation::TputOutcome> {
        let term = self.shell_state.env_vars.get("TERM")?;
        let exported = crate::executor::parse_helpers::is_marked_var(
            &self.shell_state.env_vars,
            crate::executor::types::EXPORTED_VARS,
            "TERM",
        ) && !term.is_empty();
        if !exported {
            return None;
        }
        crate::executor::tput_emulation::resolve_admitted_tput(term, &words[1..])
    }

    /// Hash-table parity with the emulated family: an admitted invocation
    /// stands in for the PATH binary, so the same record_command_resolution
    /// the `emulated` list applies must run for it (builtins9.sub
    /// introspection via `hash -l`/BASH_CMDS). The fall-through records
    /// nothing — the real spawn path does it.
    #[cfg(windows)]
    pub(in crate::executor) fn record_tput_resolution(&mut self, command: &CommandNode) {
        if command.words[0].contains('/')
            || command.words[0].contains('\\')
            || !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "hashall")
            || self
                .shell_state
                .env_vars
                .get("__RUBASH_TEMP_PATH")
                .map(String::as_str)
                == Some("1")
        {
            return;
        }
        if let Some(program) =
            crate::executor::path::find_user_command("tput", &self.shell_state.env_vars)
        {
            let display = super::execution_misc::shell_display_path(
                &program.to_string_lossy().replace('\\', "/"),
            );
            crate::builtins::hash::record_command_resolution(
                &mut self.shell_state.env_vars,
                "tput",
                &display,
            );
        }
    }

    fn external_tty(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636 narrowing (2026-10-27 ruling): the emulation
        // evaluates exactly two shapes -- `-s` (either stdin state)
        // and a bare invocation whose stdin is not a terminal
        // (`not a tty`, status 1; the non-tty case is the
        // compatibility-critical path for bashdb command input).
        // Everything else declines to the real binary: an extra
        // operand is GNU's usage error (exit 2) and was silently
        // ignored here, other options (--help/--version included)
        // are unimplemented, and a bare invocation on a terminal
        // would have to print a device name the emulation cannot
        // know -- the `/dev/tty` it used to print is fabricated (no
        // such name on Windows, the wrong name everywhere else).
        let mut silent = false;
        for arg in &cmd.words[1..] {
            match arg.as_str() {
                "-s" | "--silent" | "--quiet" => silent = true,
                _ => return Ok(false),
            }
        }
        let on_terminal = std::io::stdin().is_terminal();
        if !silent && on_terminal {
            return Ok(false);
        }
        if !silent {
            self.write_cat_output(cmd, b"not a tty\n")?;
        }
        self.exit_code = if on_terminal { 0 } else { 1 };
        Ok(true)
    }

    fn external_mkdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636 narrowing (2026-10-27 ruling): the parse below
        // implements -p/--parents exactly; every other option shape
        // was accepted and dropped (-m's mode is consumed and never
        // applied, -v never prints, unknown options are silently
        // swallowed while the directory is still created). Those
        // shapes decline to the real binary (mkdir_declines).
        if mkdir_declines(&cmd.words[1..]) {
            return Ok(false);
        }
        // GNU mkdir (coreutils) parses options before operands: -p creates
        // parents (and only -p does — without it mkdir(2) is single-level
        // and a missing parent is ENOENT), -m MODE consumes a value,
        // -v is verbose; `--` ends option parsing so a following `-p` is
        // an operand. Without this the flag itself became a literal
        // directory entry (`mkdir -p d` created `./-p`).
        let mut mode_value_pending = false;
        let mut no_more_flags = false;
        let mut parents = false;
        let mut failed = false;
        // coreutils diagnostics are the emulated command's own stderr:
        // they go through the command's redirections like any external
        // command's output (rubash#1372: `mkdir ... >f 2>&1` must create
        // f holding the message), never straight to the terminal, and
        // they carry no shell diagnostic prefix.
        let mut stderr: Vec<u8> = Vec::new();
        for word in &cmd.words[1..] {
            // cmd.words are the already-expanded argv: the simple-command
            // executor ran each word through expand_command_words once
            // (execute_cmd.c execute_disk_command -> expand_words), which
            // performs quote removal — a single-quoted/escaped operand like
            // '$FIX/d' or \$FIX/d arrives with its quotes gone and a
            // LIVE-looking `$`. Re-running expand_word here is a second
            // expansion pass that re-scans that literal text and silently
            // creates a different path (issue #555, sibling of #511/#177).
            // Use the argv as-is; glob expansion already happened once at
            // dispatch. Same rule as external_sed (rubash#177) and
            // external_cat (rubash#511).
            let expanded = word.as_str();
            if !no_more_flags && !mode_value_pending && expanded == "--" {
                no_more_flags = true;
                continue;
            }
            if !no_more_flags && !mode_value_pending && expanded.starts_with('-') && expanded != "-"
            {
                if expanded == "-m" {
                    mode_value_pending = true;
                } else if expanded == "-p" || expanded == "--parents" {
                    parents = true;
                } else if !expanded.starts_with("--")
                    && expanded[1..].chars().all(|ch| "pvm".contains(ch))
                {
                    // Bundled short flags (-pv, -vp); a bundle carrying m
                    // (-pm) takes its mode from the next word.
                    if expanded.contains('p') {
                        parents = true;
                    }
                    if expanded.contains('m') {
                        mode_value_pending = true;
                    }
                }
                continue;
            }
            if mode_value_pending {
                mode_value_pending = false;
                continue;
            }
            // coreutils mkdir.c: a failed operand reports
            // `mkdir: cannot create directory 'NAME': <strerror>' and the
            // loop continues with the remaining operands, leaving status 1.
            // The wrapper is the cosmetic part rubash#280 fixes:
            // propagating the io::Error with `?` printed a bare
            // `<script>: line N: Invalid argument` with no mkdir context
            // (an NTFS-invalid wildcard name such as `x*s' triggers it on
            // Windows while ext4 accepts it). rubash#1372: the previous
            // code also called create_dir_all unconditionally, which
            // masked the single-level mkdir(2) semantics — a missing
            // parent must fail ENOENT without -p, not silently create
            // the whole chain (on the Android/Termux host it surfaced as
            // a bogus EROFS instead).
            let path =
                self.session_path(shell_path_to_windows(&expanded, &self.shell_state.env_vars));
            let result = if parents {
                fs::create_dir_all(&path)
            } else {
                fs::create_dir(&path)
            };
            if let Err(error) = result {
                writeln!(
                    stderr,
                    "mkdir: cannot create directory '{}': {}",
                    expanded,
                    crate::posix_errors::message(&error)
                )?;
                failed = true;
            }
        }
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        self.exit_code = i32::from(failed);
        Ok(true)
    }

    fn external_touch(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636 narrowing (2026-10-27 ruling): -h is parsed as
        // a no-op below, so the stamp lands on the symlink's TARGET
        // where GNU touches the link itself; and an invocation with
        // no operands exits 0 silently where GNU reports a missing
        // file operand. Both shapes decline (touch_declines); the
        // delegation further down already covers --help/--version
        // and out-of-grammar dates.
        if touch_declines(&cmd.words[1..]) {
            return Ok(false);
        }
        // GNU coreutils touch.c do_touch: for an EXISTING operand,
        // utimensat(AT_FDCWD, file, NULL, 0) stamps atime and mtime with
        // the current time — the file is opened O_WRONLY only when it has
        // to be created, and the content is never truncated or rewritten.
        // A missing operand is created empty (unless -c/--no-create) and
        // then stamped. Every operand is processed independently: a
        // failure prints `touch: cannot touch 'FILE': ...` and the loop
        // continues, leaving exit status 1 (touch.c do_touch loop).
        //
        // rubash#1069: the timestamp options carry their REAL time, not
        // "now": -r copies the reference file's atime/mtime, -t parses
        // [[CC]YY]MMDDhhmm[.ss] as local time, and -d accepts @epoch,
        // "now", and ISO-8601 local date-times. A -d string outside that
        // grammar (GNU's getdate accepts far more) and --version/--help
        // delegate to the real PATH binary when one resolves — the
        // emulation must not silently stamp the current time where GNU
        // would stamp the requested time or print its own version. With
        // no real binary to delegate to, an unparseable date is GNU's
        // `invalid date format` error and nothing is touched.
        let mut failed = false;
        let mut create_missing = true;
        let mut stamp_accessed = true;
        let mut stamp_modified = true;
        // (atime, mtime) requested by -t/-d/-r; None = current time.
        let mut stamp: Option<(SystemTime, SystemTime)> = None;
        // A -d/-t value this emulation cannot evaluate exactly.
        let mut unevaluated_date: Option<String> = None;
        let mut info_request = false;
        // The first -r failure's diagnostic text, held back instead of
        // printed on sight (rubash#1846): coreutils touch evaluates
        // the reference file before the date grammar, so when both
        // fail the reference line is the only one GNU prints, and a
        // delegated invocation lets the real binary report its own.
        let mut ref_error: Option<String> = None;
        let mut operands: Vec<String> = Vec::new();
        // coreutils diagnostics are the emulated command's own stderr
        // (rubash#1846): buffered and routed through the command's
        // redirections via write_buffered_builtin_output, never
        // straight to the terminal with eprintln!. A getopt failure
        // is terminal in GNU (message + Try line, nothing touched),
        // so the option-error arms flush and return at once.
        let mut stderr: Vec<u8> = Vec::new();
        let mut no_more_options = false;
        let mut index = 1;
        while let Some(word) = cmd.words.get(index) {
            index += 1;
            // argv as-is — no second expand_word pass (issue #555, see
            // external_mkdir): option/operand classification runs on the
            // same final text getopt would see in the real touch.exe.
            let expanded = word.as_str();
            if no_more_options || !expanded.starts_with('-') || expanded == "-" {
                operands.push(expanded.to_owned());
                continue;
            }
            if expanded == "--" {
                no_more_options = true;
                continue;
            }
            if let Some(long) = expanded.strip_prefix("--") {
                let (name, attached) = match long.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_string())),
                    None => (long, None),
                };
                match name {
                    "date" | "reference" => {
                        let value = attached.or_else(|| {
                            let value = cmd.words.get(index).cloned();
                            index += 1;
                            value
                        });
                        match value {
                            Some(value) => self.touch_apply_stamp_option(
                                if name == "date" { 'd' } else { 'r' },
                                &value,
                                &mut stamp,
                                &mut unevaluated_date,
                                &mut ref_error,
                            ),
                            None => failed = true,
                        }
                    }
                    "time" => {
                        let value = attached.or_else(|| {
                            let value = cmd.words.get(index).cloned();
                            index += 1;
                            value
                        });
                        match value.as_deref() {
                            Some("atime" | "access" | "use") => stamp_modified = false,
                            Some("mtime" | "modify") => stamp_accessed = false,
                            _ => failed = true,
                        }
                    }
                    "no-create" => create_missing = false,
                    "version" | "help" => info_request = true,
                    _ => {
                        // glibc getopt_long reports the whole word,
                        // an attached `=value` included.
                        writeln!(stderr, "touch: unrecognized option '--{}'", long)?;
                        writeln!(stderr, "Try 'touch --help' for more information.")?;
                        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
                continue;
            }
            let shorts = &expanded[1..];
            let mut position = 0;
            while position < shorts.len() {
                let flag = shorts[position..].chars().next().unwrap_or('\0');
                position += flag.len_utf8();
                match flag {
                    'c' => create_missing = false,
                    'a' => stamp_modified = false,
                    'm' => stamp_accessed = false,
                    'h' => {}
                    // -t/-d/-r take a value: the rest of this cluster when
                    // attached (-t202001010000), else the next word.
                    't' | 'd' | 'r' => {
                        let rest = &shorts[position..];
                        let value = if !rest.is_empty() {
                            Some(rest.to_string())
                        } else {
                            let value = cmd.words.get(index).cloned();
                            index += 1;
                            value
                        };
                        match value {
                            Some(value) => self.touch_apply_stamp_option(
                                flag,
                                &value,
                                &mut stamp,
                                &mut unevaluated_date,
                                &mut ref_error,
                            ),
                            None => failed = true,
                        }
                        break;
                    }
                    bad => {
                        writeln!(stderr, "touch: invalid option -- '{}'", bad)?;
                        writeln!(stderr, "Try 'touch --help' for more information.")?;
                        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
            }
        }
        // Delegation: --version/--help and dates outside the evaluated
        // grammar belong to the real binary whenever one resolves.
        let real_touch =
            crate::executor::path::find_user_command("touch", &self.shell_state.env_vars);
        if (info_request || unevaluated_date.is_some()) && !failed && real_touch.is_some() {
            return Ok(false);
        }
        // GNU evaluates the reference file before the date grammar:
        // a failed -r is the only diagnostic when both are wrong.
        if let Some(line) = &ref_error {
            writeln!(stderr, "{line}")?;
            failed = true;
        } else if let Some(value) = &unevaluated_date {
            writeln!(stderr, "touch: invalid date format '{}'", value)?;
            failed = true;
        }
        if failed {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }
        let (stamp_atime, stamp_mtime) = stamp.unwrap_or_else(|| {
            let now = SystemTime::now();
            (now, now)
        });
        for expanded in operands {
            let target =
                self.session_path(shell_path_to_windows(&expanded, &self.shell_state.env_vars));
            let make_times = || {
                let mut times = std::fs::FileTimes::new();
                if stamp_accessed {
                    times = times.set_accessed(stamp_atime);
                }
                if stamp_modified {
                    times = times.set_modified(stamp_mtime);
                }
                times
            };
            let stamped = File::options()
                .write(true)
                .open(&target)
                .and_then(|file| file.set_times(make_times()));
            let outcome = match stamped {
                Ok(()) => Ok(()),
                // -c on a missing operand: GNU touch skips it silently
                // (touch.c: no_create + ENOENT -> no error, no create).
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create_missing => {
                    Ok(())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    File::create(&target).and_then(|file| file.set_times(make_times()))
                }
                Err(error) => Err(error),
            };
            if let Err(error) = outcome {
                writeln!(
                    stderr,
                    "touch: cannot touch '{}': {}",
                    expanded,
                    crate::posix_errors::message(&error)
                )?;
                failed = true;
            }
        }
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        self.exit_code = if failed { 1 } else { 0 };
        Ok(true)
    }

    /// One -t/-d/-r option for the emulated touch (rubash#1069): resolve
    /// it to a concrete (atime, mtime) stamp, record an unevaluated date
    /// for the delegation/error decision, or flag a missing reference.
    fn touch_apply_stamp_option(
        &self,
        flag: char,
        value: &str,
        stamp: &mut Option<(SystemTime, SystemTime)>,
        unevaluated_date: &mut Option<String>,
        ref_error: &mut Option<String>,
    ) {
        match flag {
            'r' => {
                let target =
                    self.session_path(shell_path_to_windows(value, &self.shell_state.env_vars));
                match std::fs::metadata(&target) {
                    Ok(metadata) => {
                        let atime = metadata.accessed().unwrap_or(SystemTime::UNIX_EPOCH);
                        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        *stamp = Some((atime, mtime));
                    }
                    Err(error) => {
                        *ref_error = Some(format!(
                            "touch: failed to get attributes of '{}': {}",
                            value,
                            crate::posix_errors::message(&error)
                        ));
                    }
                }
            }
            't' => match parse_touch_stamp_time(value) {
                Some(time) => *stamp = Some((time, time)),
                None => *unevaluated_date = Some(value.to_string()),
            },
            'd' => match parse_touch_date_string(value) {
                Some(time) => *stamp = Some((time, time)),
                None => *unevaluated_date = Some(value.to_string()),
            },
            _ => {}
        }
    }
}

/// GNU touch -t [[CC]YY]MMDDhhmm[.ss], interpreted as LOCAL time
/// (coreutils touch.c posarg + posixtime). Years 69-99 are 19xx, 00-68
/// are 20xx; the 8-digit form uses the current year.
fn parse_touch_stamp_time(value: &str) -> Option<SystemTime> {
    let (main, second) = match value.split_once('.') {
        Some((main, frac)) => {
            if frac.len() != 2 || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (main, frac.parse::<u32>().ok()?)
        }
        None => (value, 0),
    };
    if !main.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (year, rest) = match main.len() {
        8 => (current_local_year(), main),
        10 => {
            let yy = main[..2].parse::<i64>().ok()?;
            (if yy >= 69 { 1900 + yy } else { 2000 + yy }, &main[2..])
        }
        12 => (main[..4].parse::<i64>().ok()?, &main[4..]),
        _ => return None,
    };
    let month = rest[0..2].parse::<u32>().ok()?;
    let day = rest[2..4].parse::<u32>().ok()?;
    let hour = rest[4..6].parse::<u32>().ok()?;
    let minute = rest[6..8].parse::<u32>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    local_civil_to_system_time(year, month, day, hour, minute, second)
}

/// The -d forms the emulation evaluates exactly (rubash#1069): `@epoch`,
/// `now`, and ISO-8601 local date-times `YYYY-MM-DD[ T]HH:MM[:SS]`
/// (a bare date means local midnight). Anything else delegates to the
/// real binary or errors, like GNU's getdate grammar would decide.
fn parse_touch_date_string(value: &str) -> Option<SystemTime> {
    let text = value.trim();
    if let Some(epoch) = text.strip_prefix('@') {
        let seconds: i64 = epoch.parse().ok()?;
        return Some(system_time_from_epoch_seconds(seconds));
    }
    if text.eq_ignore_ascii_case("now") {
        return Some(SystemTime::now());
    }
    let (date, time) = match text.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time.trim())),
        None => (text, None),
    };
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hour, minute, second) = match time {
        None => (0, 0, 0),
        Some(time) => {
            let mut parts = time.split(':');
            let hour: u32 = parts.next()?.parse().ok()?;
            let minute: u32 = parts.next()?.parse().ok()?;
            let second: u32 = match parts.next() {
                Some(part) => part.parse().ok()?,
                None => 0,
            };
            if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
                return None;
            }
            (hour, minute, second)
        }
    };
    local_civil_to_system_time(year, month, day, hour, minute, second)
}

/// Local civil time -> SystemTime via the C library's UTC offset
/// (two passes so a DST boundary picks the offset in force at the
/// target instant). Non-unix hosts have no localtime_r here and treat
/// the civil time as UTC — the pre-#1069 approximation narrowed to
/// this one conversion.
fn local_civil_to_system_time(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<SystemTime> {
    let days = days_from_civil(year, month, day);
    let naive = days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    let epoch = naive - local_utc_offset_seconds(naive);
    let epoch = naive - local_utc_offset_seconds(epoch);
    Some(system_time_from_epoch_seconds(epoch))
}

fn system_time_from_epoch_seconds(epoch: i64) -> SystemTime {
    if epoch >= 0 {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(epoch as u64)
    } else {
        SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(epoch.unsigned_abs())
    }
}

/// Howard Hinnant's days-from-civil: days since 1970-01-01 for a
/// proleptic Gregorian date (the inverse of identity.rs civil_from_days).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = if month > 2 {
        i64::from(month) - 3
    } else {
        i64::from(month) + 9
    };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn current_local_year() -> i64 {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let local = now + local_utc_offset_seconds(now);
    let days = local.div_euclid(86_400);
    // civil_from_days (Hinnant), year component only.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    if mp >= 10 {
        year + 1
    } else {
        year
    }
}

#[cfg(unix)]
// `libc::time_t` is deprecated on musl/ohos targets and the niubash
// cross-target checks compile this crate with `-D warnings`; see the twin in
// printf/zone.rs. The value domain is an epoch second count that
// `localtime_r` consumes unchanged, so allow the alias locally.
#[allow(deprecated)]
fn local_utc_offset_seconds(epoch: i64) -> i64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time = epoch as libc::time_t;
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        0
    } else {
        i64::from(tm.tm_gmtoff)
    }
}

#[cfg(not(unix))]
fn local_utc_offset_seconds(_epoch: i64) -> i64 {
    0
}
impl Executor {
    fn external_cp(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636 narrowing (2026-10-27 ruling): option shapes
        // the parse below accepts but does not implement decline
        // before any work happens (cp_declines): link creation,
        // no-dereference policy, --parents, the backup machinery,
        // context, force, --update values other than the implemented
        // `older` semantics, and the --help/--version info requests.
        if cp_declines(&cmd.words[1..]) {
            return Ok(false);
        }
        // GNU cp (coreutils cp.c) parses long and short options before and
        // between operands, and `--` ends option parsing. -r/-R/-a recurse
        // into directories, -n/--no-clobber skips files that already exist
        // (and warns that the short form is non-portable), -i/--interactive
        // prompts before overwriting, -v/--verbose prints `'src' -> 'dst',
        // -u/--update copies only when the source is newer or the destination
        // is absent, --remove-destination unlinks the destination first, and
        // -t/--target-directory routes every operand into one directory.
        // -p/--preserve keeps the source's mode and timestamps on the
        // destination (niubash#533: the flags used to parse as no-ops, so
        // the destination always took the copy time); -a/--archive is
        // -dR --preserve=all, so it preserves too. Ownership from the
        // preserve list is parsed but not actionable (an unprivileged GNU
        // cp skips it silently as well).
        // Formerly "tolerated but unimplemented" shapes (-f, -l, -s,
        // -d/-P, --parents, --backup, -S/--suffix) are no longer
        // tolerated: cp_declines turns them away above, since the
        // emulation does not implement them; the no-op arms in the
        // parse below remain only as a defensive fallback.
        let mut recursive = false;
        let mut preserve_mode = false;
        let mut preserve_times = false;
        let mut no_clobber = false;
        let mut interactive = false;
        let mut verbose = false;
        let mut update_only = false;
        let mut remove_dest = false;
        let mut target_dir: Option<String> = None;
        let mut no_more_flags = false;
        let mut operands: Vec<String> = Vec::new();

        // coreutils diagnostics are the emulated command's own stderr:
        // bare `cp: ...` lines with no shell diagnostic prefix
        // (rubash#1596; same convention as mkdir, rubash#1372). The
        // sites below previously formatted through a `prefix` variable
        // holding self.diagnostic_prefix().
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut words = cmd.words[1..].iter();

        while let Some(word) = words.next() {
            // argv as-is — no second expand_word pass (issue #555, see
            // external_mkdir): option/operand classification runs on the
            // same final text getopt would see in the real cp.exe.
            let expanded = word.as_str();
            if no_more_flags {
                operands.push(expanded.to_owned());
                continue;
            }
            if expanded == "--" {
                no_more_flags = true;
                continue;
            }
            if expanded == "-" || !expanded.starts_with('-') {
                operands.push(expanded.to_owned());
                continue;
            }
            if let Some(rest) = expanded.strip_prefix("--") {
                let (name, value) = match rest.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_string())),
                    None => (rest, None),
                };
                match name {
                    "recursive" => recursive = true,
                    "archive" => {
                        recursive = true;
                        preserve_mode = true;
                        preserve_times = true;
                    }
                    "no-clobber" => no_clobber = true,
                    "interactive" => interactive = true,
                    "verbose" => verbose = true,
                    "preserve" => cp_apply_preserve_list(
                        value.as_deref(),
                        &mut preserve_mode,
                        &mut preserve_times,
                        true,
                    ),
                    "no-preserve" => cp_apply_preserve_list(
                        value.as_deref(),
                        &mut preserve_mode,
                        &mut preserve_times,
                        false,
                    ),
                    "update" => update_only = true,
                    "remove-destination" => remove_dest = true,
                    "force" | "link" | "symbolic-link" | "parents" | "backup"
                    | "no-dereference" | "suffix" | "context" => {}
                    "target-directory" | "target-dir" => {
                        // The -t/--target-directory value is argv too: take
                        // it as-is (issue #555).
                        target_dir = value.or_else(|| words.next().map(|next| next.clone()));
                    }
                    "help" => {
                        let _ = writeln!(stderr, "{}", cp_usage_text());
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                    other => {
                        let _ = writeln!(stderr, "cp: unknown option '--{}'", other);
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
                continue;
            }
            // Short option cluster such as -upv; -t and -S take the next
            // word as their value.
            let chars: Vec<char> = expanded[1..].chars().collect();
            let mut index = 0;
            while index < chars.len() {
                match chars[index] {
                    'r' | 'R' => recursive = true,
                    'a' => {
                        recursive = true;
                        preserve_mode = true;
                        preserve_times = true;
                    }
                    'n' => no_clobber = true,
                    'i' => interactive = true,
                    'v' => verbose = true,
                    'p' => {
                        preserve_mode = true;
                        preserve_times = true;
                    }
                    'u' => update_only = true,
                    'f' | 'l' | 's' | 'd' | 'P' | 'Z' => {}
                    't' => {
                        // The -t value is argv too: take it as-is (issue #555).
                        target_dir = words.next().map(|next| next.clone());
                        break;
                    }
                    'S' => {
                        words.next();
                        break;
                    }
                    other => {
                        let _ = writeln!(stderr, "cp: invalid option -- '{}'", other);
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
                index += 1;
            }
        }

        if no_clobber {
            let _ = writeln!(
                stderr,
                "cp: warning: behavior of -n is non-portable and may change in future; use --update=none instead",
            );
        }

        // With -t DIR every operand is a source copied into DIR, so the
        // directory becomes the trailing destination operand.
        let effective: Vec<String> = match target_dir {
            Some(dir) if !operands.is_empty() => {
                let mut all = operands.clone();
                all.push(dir);
                all
            }
            _ => operands,
        };

        // GNU copy.c rejects `cp /dev/null /dev/null` as a same-file copy
        // before any data would move.
        if effective.len() == 2
            && crate::executor::path::is_shell_null_device(&effective[0])
            && crate::executor::path::is_shell_null_device(&effective[1])
        {
            let _ = writeln!(
                stderr,
                "cp: '{}' and '{}' are the same file",
                effective[0], effective[1]
            );
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        if effective.len() < 2 {
            let _ = match effective.last() {
                Some(last) => writeln!(
                    stderr,
                    "cp: missing destination file operand after '{}'\nTry 'cp --help' for more information.",
                    last
                ),
                None => writeln!(
                    stderr,
                    "cp: missing file operand\nTry 'cp --help' for more information.",
                ),
            };
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let destination_word = &effective[effective.len() - 1];
        let destination = self.session_path(shell_path_to_windows(
            destination_word,
            &self.shell_state.env_vars,
        ));

        // GNU null-device semantics: coreutils copy.c opens the destination
        // and the write goes nowhere, so `cp FILE /dev/null` succeeds without
        // creating anything. Windows has no device CopyFileExW can stat, so
        // model both directions explicitly.
        if effective.len() == 2 && crate::executor::path::is_shell_null_device(destination_word) {
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 0;
            return Ok(true);
        }

        if effective.len() > 2 && !destination.exists() {
            let _ = writeln!(
                stderr,
                "cp: target '{}': No such file or directory",
                destination_word
            );
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let mut status = 0;
        for source_word in &effective[..effective.len() - 1] {
            // /dev/null as a source reads as an empty stream, so the copy
            // creates or truncates the target as an empty file (basename
            // "null" when the destination is a directory).
            if crate::executor::path::is_shell_null_device(source_word) {
                let name = source_word
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or("null")
                    .to_string();
                let target_path = if destination.is_dir() {
                    destination.join(name)
                } else {
                    destination.clone()
                };
                if let Err(error) = File::create(&target_path) {
                    status = 1;
                    let _ = writeln!(
                        stderr,
                        "cp: cannot create '{}': {}",
                        target_path.display(),
                        crate::posix_errors::message(&error)
                    );
                }
                continue;
            }
            let source = self.session_path(shell_path_to_windows(
                source_word,
                &self.shell_state.env_vars,
            ));

            // GNU coreutils cp.c copy_internal: an operand whose final
            // component is `.` (`src/.`, `./`, `.`) names the directory's
            // CONTENTS — dotfiles included — not the directory itself.
            // With an existing directory destination the children land
            // directly inside it instead of under a `dst/src` subdir.
            let source_names_contents = {
                let trimmed = source_word.replace('\\', "/");
                let trimmed = trimmed.trim_end_matches('/');
                trimmed == "." || trimmed.ends_with("/.")
            };
            let mut copy_units: Vec<(PathBuf, String)> = Vec::new();
            if source_names_contents {
                if let Err(error) = super::copy_source::require_directory_contents_source(&source) {
                    status = 1;
                    let _ = writeln!(
                        stderr,
                        "cp: cannot stat '{}': {}",
                        source_word,
                        crate::posix_errors::message(&error)
                    );
                    continue;
                }
                if !recursive {
                    status = 1;
                    let _ = writeln!(
                        stderr,
                        "cp: -r not specified; omitting directory '{}'",
                        source_word
                    );
                    continue;
                }
                if destination.exists() && !destination.is_dir() {
                    status = 1;
                    let _ = writeln!(
                        stderr,
                        "cp: cannot overwrite non-directory '{}' with directory '{}'",
                        destination_word, source_word
                    );
                    continue;
                }
                if destination.is_dir() {
                    let mut entries: Vec<_> = match fs::read_dir(&source) {
                        Ok(read_dir) => read_dir.filter_map(Result::ok).collect(),
                        Err(error) => {
                            let _ = writeln!(
                                stderr,
                                "cp: cannot access '{}': {}",
                                source_word,
                                crate::posix_errors::message(&error)
                            );
                            status = 1;
                            continue;
                        }
                    };
                    entries.sort_by_key(|entry| entry.file_name());
                    for entry in entries {
                        let name = entry.file_name().to_string_lossy().to_string();
                        copy_units.push((
                            entry.path(),
                            format!("{}/{}", source_word.trim_end_matches('/'), name),
                        ));
                    }
                } else {
                    copy_units.push((source.clone(), source_word.clone()));
                }
            } else {
                copy_units.push((source.clone(), source_word.clone()));
            }

            for (source, source_word) in copy_units {
                // `/dev/std*`, `/dev/fd/N` sources name a descriptor, not a
                // filesystem path: GNU copies the fd's content into a
                // regular file (cp.c copy_internal reads through the dup'd
                // descriptor), so serve the endpoint's remaining bytes.
                if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&source_word) {
                    match self.dev_fd_operand_bytes(fd) {
                        Some(bytes) => {
                            let (target, target_display) = if destination.is_dir() {
                                let leaf = source_word
                                    .replace('\\', "/")
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&source_word)
                                    .to_string();
                                (
                                    destination.join(&leaf),
                                    format!("{}/{}", destination_word.trim_end_matches('/'), leaf),
                                )
                            } else {
                                (destination.clone(), destination_word.clone())
                            };
                            match fs::write(&target, &bytes) {
                                Ok(()) => {
                                    if verbose {
                                        let _ = writeln!(
                                            stdout,
                                            "{} -> {}",
                                            cp_quoted_name(&source_word),
                                            cp_quoted_name(&target_display)
                                        );
                                    }
                                }
                                Err(error) => {
                                    let _ = writeln!(
                                        stderr,
                                        "cp: cannot create '{}': {}",
                                        target.display(),
                                        crate::posix_errors::message(&error)
                                    );
                                    status = 1;
                                }
                            }
                        }
                        None => {
                            let _ = writeln!(
                                stderr,
                                "cp: cannot stat '{}': No such file or directory",
                                source_word
                            );
                            status = 1;
                        }
                    }
                    continue;
                }
                let (target, target_display) = if destination.is_dir() {
                    let Some(name) = source.file_name() else {
                        let _ = writeln!(
                            stderr,
                            "cp: missing destination file operand after '{}'",
                            source_word
                        );
                        status = 1;
                        continue;
                    };
                    let leaf = name.to_string_lossy().to_string();
                    // `cp -rv src/. dst` names children 'dst/./name' — the
                    // `/.` join is part of GNU's verbose target spelling.
                    let separator = if source_names_contents { "/./" } else { "/" };
                    (
                        destination.join(name),
                        format!(
                            "{}{}{}",
                            destination_word.trim_end_matches('/'),
                            separator,
                            leaf
                        ),
                    )
                } else {
                    (destination.clone(), destination_word.clone())
                };

                if !source.exists() {
                    let _ = writeln!(
                        stderr,
                        "cp: cannot stat '{}': No such file or directory",
                        source_word
                    );
                    status = 1;
                    continue;
                }
                if source.is_dir() && !recursive {
                    let _ = writeln!(
                        stderr,
                        "cp: -r not specified; omitting directory '{}'",
                        source_word
                    );
                    status = 1;
                    continue;
                }
                if target.exists() {
                    if no_clobber {
                        continue;
                    }
                    if interactive {
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        stdout.clear();
                        stderr.clear();
                        if !cp_confirm_overwrite(&target_display) {
                            status = 1;
                            continue;
                        }
                    }
                }
                if remove_dest {
                    let _ = fs::remove_file(&target);
                }
                if update_only && target.exists() && cp_source_not_newer(&source, &target) {
                    continue;
                }

                let result = if source.is_dir() {
                    cp_copy_tree(&source, &target, 0, preserve_mode, preserve_times)
                } else {
                    fs::copy(&source, &target).map(|_| ())
                };
                match result {
                    Ok(_) => {
                        if preserve_mode || preserve_times {
                            // Tree copies apply preservation per node
                            // inside cp_copy_tree; a plain file copy is
                            // stamped here (niubash#533).
                            if !source.is_dir() {
                                cp_apply_preserved(&source, &target, preserve_mode, preserve_times);
                            }
                        } else if !source.is_dir() {
                            cp_set_modified_now(&target);
                        }
                        if verbose {
                            let _ = writeln!(
                                stdout,
                                "{} -> {}",
                                cp_quoted_name(&source_word),
                                cp_quoted_name(&target_display)
                            );
                        }
                    }
                    Err(error) => {
                        let _ = writeln!(
                            stderr,
                            "cp: cannot create '{}': {}",
                            target.display(),
                            crate::posix_errors::message(&error)
                        );
                        status = 1;
                    }
                }
            }
        }

        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        self.exit_code = status;
        Ok(true)
    }

    fn external_rm(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636 narrowing (2026-10-27 ruling): -i/-I/
        // --interactive and --one-file-system are accepted by the
        // parse below but never implemented — the files are deleted
        // without the prompt GNU gives (a data-safety silent subset)
        // or the filesystem boundary GNU honors. Those shapes
        // decline to the real binary (rm_declines); the emulation
        // keeps the forms it evaluates exactly.
        if rm_declines(&cmd.words[1..]) {
            return Ok(false);
        }
        // GNU rm (coreutils rm.c): -r/-R/--recursive removes directories,
        // -d/--dir removes empty ones, and a directory operand without
        // either is an error — never an implicit recursion. -f ignores
        // failures for NONEXISTENT operands only; every other failure keeps
        // its diagnostic and status 1. Options accept the `--` terminator
        // and may follow operands (argv permutation in getopt_long).
        let mut force = false;
        let mut recursive = false;
        let mut remove_empty_dir = false;
        let mut verbose = false;
        // coreutils rm.c: --preserve-root is the default; the
        // failsafe it arms is applied per operand below (rubash#1847).
        let mut preserve_root = true;
        let mut no_more_options = false;
        let mut operands: Vec<&str> = Vec::new();
        // coreutils diagnostics are the emulated command's own stderr
        // (rubash#1846): they go through the command's redirections
        // via write_buffered_builtin_output, never straight to the
        // terminal with eprintln!. GNU's getopt failure is terminal:
        // the first bad option prints its line plus the Try line and
        // the invocation ends before any operand is touched, so the
        // error arms below flush this buffer and return at once.
        let mut stderr: Vec<u8> = Vec::new();

        for word in cmd.words.iter().skip(1) {
            // argv as-is — no second expand_word pass (issue #555, see
            // external_mkdir): a quoted '$FIX/r' must fail on the literal
            // name, never remove the expanded neighbor.
            let expanded = word.as_str();
            if no_more_options || expanded == "-" || !expanded.starts_with('-') {
                operands.push(expanded);
                continue;
            }
            if expanded == "--" {
                no_more_options = true;
                continue;
            }
            if let Some(long) = expanded.strip_prefix("--") {
                let (name, value) = match long.split_once('=') {
                    Some((name, value)) => (name, Some(value)),
                    None => (long, None),
                };
                match name {
                    // The real binary owns these two texts.
                    "help" | "version" => return Ok(false),
                    "force" => force = true,
                    "recursive" => recursive = true,
                    "dir" => remove_empty_dir = true,
                    "verbose" => verbose = true,
                    "interactive" | "one-file-system" => {}
                    // getopt_long declares --preserve-root with an
                    // optional argument whose only accepted value is
                    // `all`; any other attached value is its own
                    // diagnostic with no Try line (coreutils 9.4).
                    "preserve-root" => match value {
                        None | Some("all") => preserve_root = true,
                        Some(other_value) => {
                            writeln!(
                                stderr,
                                "rm: unrecognized --preserve-root argument: '{other_value}'"
                            )?;
                            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                            self.exit_code = 1;
                            return Ok(true);
                        }
                    },
                    "no-preserve-root" => match value {
                        None => preserve_root = false,
                        Some(_) => {
                            writeln!(
                                stderr,
                                "rm: option '--no-preserve-root' doesn't allow an argument"
                            )?;
                            writeln!(stderr, "Try 'rm --help' for more information.")?;
                            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                            self.exit_code = 1;
                            return Ok(true);
                        }
                    },
                    _ => {
                        // glibc getopt_long reports the whole word,
                        // an attached `=value` included.
                        writeln!(stderr, "rm: unrecognized option '--{long}'")?;
                        writeln!(stderr, "Try 'rm --help' for more information.")?;
                        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
                continue;
            }
            for flag in expanded[1..].chars() {
                match flag {
                    'f' => force = true,
                    'r' | 'R' => recursive = true,
                    'd' => remove_empty_dir = true,
                    'v' => verbose = true,
                    // Prompting needs the interactive input path; the option
                    // is accepted so -i/-I never deletes a literal `-i`.
                    'i' | 'I' => {}
                    other => {
                        writeln!(stderr, "rm: invalid option -- '{other}'")?;
                        writeln!(stderr, "Try 'rm --help' for more information.")?;
                        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
            }
        }

        if operands.is_empty() {
            writeln!(stderr, "rm: missing operand")?;
            writeln!(stderr, "Try 'rm --help' for more information.")?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let mut status = 0;
        let mut stdout = Vec::new();
        // The root the preserve-root failsafe protects: the shell
        // namespace's own `/` mapping (the payload root on Windows,
        // the filesystem root elsewhere).
        let root_target = self.session_path(shell_path_to_windows("/", &self.shell_state.env_vars));
        for expanded in operands {
            let target =
                self.session_path(shell_path_to_windows(expanded, &self.shell_state.env_vars));
            // coreutils rm.c (niubash#534 wording residual): a recursive
            // removal whose operand's last component is `.` or `..` is
            // refused outright — `refusing to remove '.' or '..' directory:
            // skipping '<operand>'`, status 1 — instead of attempting the
            // removal and reporting the OS error (rubash answered
            // "No such file or directory"). The refusal fires only when the
            // operand resolves to an existing directory: a missing `foo/.`
            // keeps the plain existence error (or the -f silence) GNU
            // gives it, and without -r the operand still earns the plain
            // `Is a directory` refusal.
            if recursive && rm_operand_is_dot_or_dotdot(expanded) {
                if fs::metadata(&target).is_ok_and(|meta| meta.is_dir()) {
                    status = 1;
                    writeln!(
                        &mut stderr,
                        "rm: refusing to remove '.' or '..' directory: skipping '{expanded}'"
                    )?;
                    continue;
                }
            }
            // coreutils remove.c preserve-root failsafe (rubash#1847):
            // a recursive removal whose operand IS the root directory
            // is refused before anything is touched; the remaining
            // operands still run and the status is 1. The dot/dotdot
            // refusal above pre-empts the spellings whose last
            // component is `.`/`..` (GNU's order — `/.`, `/tmp/..`
            // get the refusing line, not this one), so the guard
            // sees the bare-root spellings and root symlink shapes.
            if recursive && preserve_root && rm_operand_resolves_to_root(&target, &root_target) {
                status = 1;
                let shown = rm_root_display_name(expanded);
                if shown == "/" {
                    writeln!(
                        &mut stderr,
                        "rm: it is dangerous to operate recursively on '/'"
                    )?;
                } else {
                    writeln!(
                        &mut stderr,
                        "rm: it is dangerous to operate recursively on '{shown}' (same as '/')"
                    )?;
                }
                writeln!(
                    &mut stderr,
                    "rm: use --no-preserve-root to override this failsafe"
                )?;
                continue;
            }
            let result = match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    // A symlink operand names the link itself; GNU
                    // unlinks it and never touches the target. Windows
                    // removes a DIRECTORY symlink with RemoveDirectory
                    // (DeleteFile refuses it), hence remove_dir there;
                    // on unix unlink serves every link — remove_dir
                    // on a link path is ENOTDIR, which -f then
                    // swallowed, leaving `rm -rf dirlink` a silent
                    // no-op (rubash#1847 boundary shape).
                    if cfg!(windows) && target.is_dir() {
                        fs::remove_dir(&target)
                    } else {
                        fs::remove_file(&target)
                    }
                }
                Ok(metadata) if metadata.is_dir() => {
                    if recursive {
                        fs::remove_dir_all(&target)
                    } else if remove_empty_dir {
                        // gnulib remove.c reports a -d failure by the
                        // directory's contents, not by rmdir's errno:
                        // a non-empty directory is always "Directory
                        // not empty", while a bare rmdir on the root
                        // directory itself reports EBUSY (rubash#1847).
                        fs::remove_dir(&target).map_err(|error| {
                            if fs::read_dir(&target)
                                .is_ok_and(|mut entries| entries.next().is_some())
                            {
                                io::Error::new(
                                    io::ErrorKind::DirectoryNotEmpty,
                                    "Directory not empty",
                                )
                            } else {
                                error
                            }
                        })
                    } else {
                        status = 1;
                        writeln!(
                            &mut stderr,
                            "rm: cannot remove '{expanded}': Is a directory"
                        )?;
                        continue;
                    }
                }
                Ok(_) => fs::remove_file(&target),
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => {
                    if verbose {
                        writeln!(&mut stdout, "removed '{expanded}'")?;
                    }
                }
                // GNU rm -f removes the "did not exist" diagnostic; it is
                // not a license to hide a failed removal. gnulib remove.c
                // counts ENOTDIR with ENOENT there (`rm -rf f/.` on a
                // regular file f is silent, status 0 — niubash#534).
                Err(error)
                    if force
                        && (error.kind() == io::ErrorKind::NotFound
                            || error.kind() == io::ErrorKind::NotADirectory
                            || (cfg!(windows)
                                && contains_windows_forbidden_posix_filename_char(&expanded))) => {}
                Err(error) => {
                    status = 1;
                    let message = if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                    ) || (cfg!(windows)
                        && contains_windows_forbidden_posix_filename_char(&expanded))
                    {
                        "No such file or directory".to_string()
                    } else if cfg!(windows) && error.raw_os_error() == Some(145) {
                        "Directory not empty".to_string()
                    } else {
                        crate::posix_errors::message(&error)
                    };
                    writeln!(&mut stderr, "rm: cannot remove '{expanded}': {message}")?;
                }
            }
        }
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        self.exit_code = status;
        Ok(true)
    }

    fn external_rmdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636 narrowing (2026-10-27 ruling): this emulation
        // is a stub, not a subset — it treats every option as an
        // operand, swallows every failure, and always exits 0, so
        // `rmdir missing && ...` takes the success branch GNU never
        // takes and `rmdir -p a/b/c` leaves the ancestors behind.
        // There is no shape it evaluates exactly, so every invocation
        // declines to the real binary (dispatch reaches this handler
        // only when the name resolved in PATH, rubash#1545). The
        // #555 invariant is preserved by construction: the operand
        // text reaches the real rmdir exactly as expanded once by
        // the simple-command executor.
        let _ = cmd;
        Ok(false)
    }

    fn external_cat(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#415: GNU cat parses its whole option surface (getopt_long
        // with argv permutation) before opening anything — a usage error
        // reports `cat: invalid option -- 'Z'' + the Try line and exits 1
        // with NO output, even when operands exist (`cat f -Z', WSL 9.4
        // probe perm.sh). --help/--version print the real binary's own
        // text, so they fall through to the PATH subprocess.
        let parsed = match parse_cat_argv(cmd) {
            CatParsed::HelpOrVersion => return Ok(false),
            CatParsed::Usage(message) => {
                let mut stderr = Vec::new();
                stderr.extend_from_slice(message.as_bytes());
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 1;
                return Ok(true);
            }
            CatParsed::Options { options, operands } => (options, operands),
        };
        let (options, operands) = parsed;
        let filter = |data: &[u8]| -> Vec<u8> { cat_format(data, &options) };
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_redirect_target(redirect);
                if let Some(fd) = redirect_target_fd(&target) {
                    if let Some(FdReadEndpoint::CoprocStdout { fd: pipe, .. }) =
                        self.fd_table.read_endpoint(fd)
                    {
                        // Drain the coproc's stdout pipe through the slot's
                        // real HANDLE (BROKEN_PIPE maps to EOF in read_some).
                        let mut input = Vec::new();
                        loop {
                            match crate::fd::read_some(pipe.handle, 8192) {
                                Ok(buf) if buf.is_empty() => break,
                                Ok(buf) => input.extend_from_slice(&buf),
                                Err(_) => break,
                            }
                        }
                        self.fd_table.close_input(fd);
                        self.write_cat_output(cmd, &filter(&input))?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                }
            }
        }

        if cmd.heredoc.is_some() {
            let input = self.stdin_string_for_command_mut(cmd).unwrap_or_default();
            let output =
                filter(&crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input));
            if let Some(redirect) = &cmd.append {
                let target = self.expand_redirect_target(redirect);
                // /dev/std*, /dev/fd/N aliases resolve as a dup of the
                // aliased fd (write_dev_stdio_redirect_output) — reopening
                // the alias as a literal path hit CONOUT$ and failed with
                // Permission denied under a redirected stderr (rubash#216).
                if let Some(result) = self.write_dev_stdio_redirect_output(&target, &output) {
                    result?;
                    self.exit_code = 0;
                    return Ok(true);
                }
                let mut file = open_redirect_append(
                    &self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars)),
                )?;
                file.write_all(&output)?;
                self.exit_code = 0;
                return Ok(true);
            }

            if let Some(redirect) = &cmd.redirect_out {
                // GNU redir.c: all `>` redirects open in order (each file
                // created/truncated); only the last receives output (rubash#1392).
                let mut seen_last = false;
                for r in cmd.redirects.iter().rev() {
                    if !matches!(
                        r.kind,
                        crate::parser::RedirectKind::Output
                            | crate::parser::RedirectKind::ClobberOutput
                    ) {
                        continue;
                    }
                    if r.fd.unwrap_or(1) != 1 {
                        continue;
                    }
                    if seen_last {
                        let t = self.expand_redirect_target(r);
                        if crate::executor::execution_misc::redirect_target_fd(&t).is_none()
                            && !t.starts_with("<(")
                            && !t.starts_with(">(")
                        {
                            let _ = self.create_redirect_output(&t, r.clobber);
                        }
                    } else {
                        seen_last = true;
                    }
                }
                let target = self.expand_redirect_target(redirect);
                if crate::executor::execution_misc::is_closed_redirect_target(&target) {
                    return Err(super::external_finish::cat_closed_stdout_error());
                }
                if let Some(result) = self.write_dev_stdio_redirect_output(&target, &output) {
                    result?;
                    self.exit_code = 0;
                    return Ok(true);
                }
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(&output)?;
                self.exit_code = 0;
                return Ok(true);
            }
        }

        // GNU cat: when file operands are present, read from them. Only fall
        // through to stdin when there are no file operands. This matters after
        // `exec 0</dev/null` which stores empty bytes in fd 0: without this
        // ordering, `cat file` would see the empty stdin and return early
        // instead of reading the file operand.
        if !operands.is_empty() {
            let mut output = Vec::new();
            // rubash#1371: operand diagnostics ACCUMULATE here and are
            // emitted together with the output at the end of the loop.
            // Flushing each diagnostic mid-loop through
            // write_buffered_builtin_output applied the command's
            // redirects once (creating `f` for `>f 2>&1` and landing the
            // text there), after which write_cat_output re-created /
            // truncated the same target for the (empty) stdout — the
            // diagnostic was silently lost. GNU opens the redirects
            // once, before cat runs; one combined emit reproduces that.
            let mut diagnostics: Vec<u8> = Vec::new();
            // GNU cat.c: operand failures (missing file, input==output)
            // print a diagnostic and set exit 1 but do NOT stop the loop —
            // later operands are still processed.
            let mut status = 0;
            // A `-' operand is stdin at that position; the first one
            // drains the source, later ones see EOF (GNU cat.c reads
            // stdin once — shared file offset).
            let mut stdin_remaining: Option<Vec<u8>> = None;
            for word in operands {
                // cmd.words are the already-expanded argv: the simple-command
                // executor ran each word through expand_command_words once
                // (execute_cmd.c execute_disk_command -> expand_words), which
                // performs quote removal — a single-quoted/escaped operand
                // like '$FIX/c' or \$FIX/c arrives with its quotes gone and a
                // LIVE-looking `$`. Re-running expand_word here is a second
                // expansion pass that re-scans that literal text and silently
                // opens a different file (issue #511). Use the argv as-is;
                // same rule as external_sed (rubash#177).
                let target = word.as_str();
                if target == "-" {
                    if stdin_remaining.is_none() {
                        if self.cat_stdin_closed(cmd) {
                            // rubash#1102: `cat -` on a closed fd 0 — the
                            // read fails EBADF (see fail text in
                            // report_cat_closed_stdin).
                            self.report_cat_closed_stdin(cmd)?;
                            status = 1;
                            stdin_remaining = Some(Vec::new());
                        } else if options.identity() && self.cat_stdin_is_real_process_stdin(cmd) {
                            // rubash#415 (CI coproc.tests hang): GNU cat streams
                            // stdin as chunks arrive (cat.c byte-copy loop); a
                            // buffered drain deadlocks interactive producers —
                            // coproc.tests runs `coproc { cat - ; }`, writes,
                            // reads the echo back, and closes the writer much
                            // later. With identity options and the REAL process
                            // stdin, flush what earlier operands collected and
                            // stream; in-memory sources and formatting keep the
                            // buffered drain (bounded, no cross-process wait).
                            if !output.is_empty() {
                                self.write_cat_output(cmd, &filter(&output))?;
                                output.clear();
                            }
                            self.stream_cat_stdin_operand(cmd)?;
                            stdin_remaining = Some(Vec::new());
                        } else {
                            stdin_remaining = Some(self.cat_stdin_operand_bytes(cmd));
                        }
                    }
                    if let Some(bytes) = stdin_remaining.take() {
                        output.extend(bytes);
                    }
                    continue;
                }
                // `/dev/std*`, `/dev/fd/N`, `/proc/self/fd/N`: GNU opens a
                // dup of the descriptor — the in-process equivalent reads
                // the fd endpoint's remaining bytes (subst.c shared
                // offset).
                if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&target) {
                    match self.dev_fd_operand_bytes_for_command(cmd, fd) {
                        Some(read) if read.stdout_file && !read.bytes.is_empty() => {
                            // GNU cat.c: input resolves to the file fd 1
                            // writes to — report and skip it. Bare, like
                            // every cat diagnostic (rubash#1596).
                            writeln!(
                                &mut diagnostics,
                                "cat: {}: input file is output file",
                                target
                            )?;
                            status = 1;
                            continue;
                        }
                        Some(read) => output.extend(read.bytes),
                        None => {
                            // GNU cat.c reports its own errors bare, with
                            // no shell diagnostic prefix (rubash#1309).
                            writeln!(
                                &mut diagnostics,
                                "cat: {}: No such file or directory",
                                target
                            )?;
                            status = 1;
                        }
                    }
                    continue;
                }
                // Q11 /proc P1 (docs/proc-vfs-plan.md hook B2): synthetic
                // files are served before the filesystem.
                if let Some(bytes) = crate::proc_vfs::proc_file_content(&target) {
                    output.extend(bytes);
                    continue;
                }
                // niubash#177: resolve through the operand form so a
                // virtual-root file operand that exists only under its
                // `.exe` spelling (`/usr/bin/seq`) opens here exactly as it
                // would in a spawned cat.exe (same exists-first rule as the
                // argv funnel). `<(cmd)` carrier paths exist, so the
                // resolver keeps them byte-identical for procsub_stream_take.
                // G-02: cat is emulated in-process here (not spawned),
                // so a relative operand must anchor at the session cwd
                // itself; procsub carrier paths are absolute and pass
                // through unchanged.
                let win = self.session_path(crate::executor::path::windows_operand_file_path(
                    shell_path_to_windows(&target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                ));
                // `<(cmd)` carrier path: the word names a draining stream —
                // serve the shared remainder (subst.c:7143).
                let read = match self.procsub_stream_take(&win) {
                    Some(bytes) => Ok(bytes),
                    None => fs::read(&win),
                };
                match read {
                    Ok(bytes) => output.extend(bytes),
                    Err(error) => {
                        // GNU cat.c: cat reports its own operand errors
                        // bare — `cat: <file>: <strerror>` with the REAL
                        // strerror and no shell diagnostic prefix. The
                        // discarded error flattened EACCES/EISDIR to a
                        // blanket ENOENT text behind the script prefix
                        // (rubash#1309).
                        writeln!(
                            &mut diagnostics,
                            "cat: {}: {}",
                            target,
                            crate::posix_errors::message(&error)
                        )?;
                        status = 1;
                    }
                }
            }
            let output = filter(&output);
            if diagnostics.is_empty() {
                self.write_cat_output(cmd, &output)?;
            } else {
                // rubash#1371: ONE emit for both streams so the ordered
                // redirect walk opens each target exactly once — with
                // `>f 2>&1` the diagnostic lands in f alongside the
                // output instead of being truncated away by a second
                // open (see the accumulator note above).
                self.write_buffered_builtin_output(cmd, &output, &diagnostics)?;
            }
            self.exit_code = status;
            return Ok(true);
        }

        // rubash#1102: no operands and fd 0 is closed — GNU cat's read
        // fails EBADF (the string funnel models closed as empty, which
        // would silently exit 0).
        if self.cat_stdin_closed(cmd) {
            self.report_cat_closed_stdin(cmd)?;
            self.exit_code = 1;
            return Ok(true);
        }

        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            self.write_cat_output(
                cmd,
                &filter(&crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input)),
            )?;
            self.exit_code = 0;
            return Ok(true);
        }

        // No operands: plain stdin shapes (the operand loop above already
        // returned). This block keeps the original cascade order.
        {
            if let Some(input) = self.read_function_stdin('\0', None, false, false, false) {
                self.write_cat_output(
                    cmd,
                    &filter(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
                    ),
                )?;
                self.exit_code = 0;
                return Ok(true);
            }
            if cmd.redirect_in.is_none() && cmd.heredoc.is_none() && cmd.here_string.is_none() {
                // fd 0 may carry a real/virtual endpoint from a compound
                // redirect (`{ cat; } <&3`) — GNU reads fd 0 directly.
                if !matches!(
                    self.fd_table.read_endpoint(0),
                    None | Some(FdReadEndpoint::InheritedProcessStdin)
                ) {
                    if let Some(bytes) = self.fd_table.read_all_bytes(0) {
                        self.write_cat_output(cmd, &filter(&bytes))?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                }
            }
            if cmd.redirect_in.is_none()
                && cmd.heredoc.is_none()
                && cmd.here_string.is_none()
                && self
                    .shell_state
                    .env_vars
                    .get(INHERIT_PROCESS_STDIN)
                    .map(String::as_str)
                    == Some("1")
            {
                return self.stream_inherited_cat(cmd, &options);
            }
            return Ok(false);
        }
    }

    /// True when cat's stdin source is a CLOSED fd 0 (rubash#1102): the
    /// command's own `<&-`/`0<&-`, or an `exec <&-` earlier in the stage
    /// left a closed entry in the fd table. Heredocs/here-strings are
    /// bodies, not descriptors, and never count as closed.
    fn cat_stdin_closed(&self, cmd: &CommandNode) -> bool {
        if cmd.heredoc.is_some() || cmd.here_string.is_some() {
            return false;
        }
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) != 0 {
                return false;
            }
            let target = self.expand_redirect_target(redirect);
            return is_closed_redirect_target(&target);
        }
        if matches!(
            super::shell_options::fd0_stdin_redirect_winner(cmd),
            Some(crate::parser::RedirectKind::CloseInput)
        ) {
            return true;
        }
        self.fd_table.is_closed(0)
    }

    /// GNU coreutils cat on a closed stdin (cat.c): the read fails and the
    /// close of standard input fails too — two diagnostics, and the caller
    /// sets exit status 1. The lines are cat's OWN stderr, written bare:
    /// an external command's diagnostics never carry the shell's
    /// `script: line N:` envelope (rubash#1596; the same convention
    /// rubash#1309 established for cat's operand errors).
    fn report_cat_closed_stdin(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(&mut stderr, "cat: -: Bad file descriptor")?;
        writeln!(
            &mut stderr,
            "cat: closing standard input: Bad file descriptor"
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(())
    }

    /// The `-' operand's bytes: the command's own stdin source (heredoc /
    /// captured stdin / fd 0 endpoint), drained once. Best-effort for the
    /// common shapes — a live inherited pipe is read to EOF.
    fn cat_stdin_operand_bytes(&mut self, cmd: &CommandNode) -> Vec<u8> {
        use std::io::Read;
        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            return crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input);
        }
        if !matches!(
            self.fd_table.read_endpoint(0),
            None | Some(FdReadEndpoint::InheritedProcessStdin)
        ) {
            if let Some(bytes) = self.fd_table.read_all_bytes(0) {
                return bytes;
            }
        }
        if cmd.redirect_in.is_none() && cmd.heredoc.is_none() && cmd.here_string.is_none() {
            let mut stdin = std::io::stdin().lock();
            let mut bytes = Vec::new();
            let _ = stdin.read_to_end(&mut bytes);
            return bytes;
        }
        Vec::new()
    }

    /// True when the command's stdin is the real process stdin — the only
    /// `cat -' source whose buffered drain can block on a live cross-process
    /// writer (coproc pipes). Mirrors the last-resort arm of
    /// cat_stdin_operand_bytes.
    fn cat_stdin_is_real_process_stdin(&self, cmd: &CommandNode) -> bool {
        if cmd.redirect_in.is_some() || cmd.heredoc.is_some() || cmd.here_string.is_some() {
            return false;
        }
        if self.stdin_string_for_command(cmd).is_some() {
            return false;
        }
        matches!(
            self.fd_table.read_endpoint(0),
            None | Some(FdReadEndpoint::InheritedProcessStdin)
        )
    }

    /// The `-' operand's streaming copy: read a chunk, emit it, repeat —
    /// GNU cat.c's byte-copy loop. Identity options only (formatting needs
    /// whole-stream state).
    fn stream_cat_stdin_operand(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        use std::io::{Read, Write};
        // The output target is opened ONCE for the whole copy: `write_cat_output`
        // recreates (truncates) it per call, so a per-8 KiB-chunk caller kept
        // only the last chunk (niubash#198).
        let mut sink = self.open_cat_output_sink(cmd)?;
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = stdin.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            match sink.as_mut() {
                Some(file) => file.write_all(&buffer[..count])?,
                None => self.write_cat_output(cmd, &buffer[..count])?,
            }
        }
        Ok(())
    }
    fn stream_inherited_cat(
        &mut self,
        cmd: &CommandNode,
        options: &CatOptions,
    ) -> Result<bool, ExecuteError> {
        use std::io::Read;

        if !options.identity() {
            // The line filters (numbering, squeeze, $-ends) carry state
            // across the whole stream, so a formatting invocation reads to
            // EOF first (GNU streams; the in-process emulation buffers —
            // bounded by the input, like every other emulated reader).
            let mut stdin = std::io::stdin().lock();
            let mut input = Vec::new();
            stdin.read_to_end(&mut input)?;
            self.write_cat_output(cmd, &cat_format(&input, options))?;
            self.exit_code = 0;
            return Ok(true);
        }
        let mut sink = self.open_cat_output_sink(cmd)?;
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = stdin.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            match sink.as_mut() {
                Some(file) => file.write_all(&buffer[..count])?,
                None => self.write_cat_output(cmd, &buffer[..count])?,
            }
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_sed(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // cmd.words are the already-expanded argv (the simple-command
        // executor expanded each word once, execute_cmd.c
        // execute_disk_command -> expand_words). Re-running expand_word here
        // would be a second expansion pass: a quoted argument whose literal
        // text contains ` or $ (e.g. sed 's/\`x\`/y/' — single-quoted data,
        // parse.y:5416 read_token_word shellquote branch) gets its decoded
        // backticks re-scanned as live command substitution and executed
        // (rubash#177). Use the argv as-is.
        let args = cmd.words[1..].to_vec();
        if apply_simple_sed_args("", &args).is_none() {
            return Ok(false);
        }
        let Some(input) = self
            .stdin_string_for_command_mut(cmd)
            .or_else(|| self.read_function_stdin('\0', None, false, false, false))
            .or_else(|| self.read_inherited_process_stdin_to_string())
        else {
            return Ok(false);
        };
        let Some(output) = apply_simple_sed_args(&input, &args) else {
            return Ok(false);
        };
        self.write_cat_output(cmd, output.as_bytes())?;
        self.exit_code = 0;
        Ok(true)
    }

    fn external_mkfifo(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1725 (2026-10-27 narrowing ruling): off Windows the
        // platform has a real fifo object, so the emulation must not
        // answer at all — decline and let the resolved PATH binary
        // create the fifo (dispatch reaches this handler only when the
        // name resolved, rubash#1545). The unconditional
        // `Operation not supported` refusal below is the Windows-only
        // answer: Windows has no POSIX fifo object, and creating a
        // regular file instead would silently change semantics (a
        // fifo's blocking open/read behavior would become an instant
        // regular-file EOF). GNU coreutils mkfifo reports per-operand
        // failures and exits 1 — match that honestly:
        // `mkfifo: cannot create fifo 'NAME': Operation not supported`.
        #[cfg(not(windows))]
        {
            let _ = cmd;
            return Ok(false);
        }
        #[cfg(windows)]
        {
            // -m MODE consumes a value; other options are accepted and
            // ignored. argv as-is — no second expand_word pass (issue
            // #555, see external_mkdir). Diagnostics are the emulated
            // command's own stderr and go through the command's
            // redirections (rubash#1372/#1725), never straight to the
            // terminal via the shell's default stderr.
            let mut mode_value_pending = false;
            let mut no_more_flags = false;
            let mut failed = false;
            let mut stderr: Vec<u8> = Vec::new();
            for word in &cmd.words[1..] {
                let expanded = word.as_str();
                if mode_value_pending {
                    mode_value_pending = false;
                    continue;
                }
                if !no_more_flags && expanded == "--" {
                    no_more_flags = true;
                    continue;
                }
                if !no_more_flags && expanded.starts_with('-') && expanded != "-" {
                    if expanded == "-m" {
                        mode_value_pending = true;
                    }
                    continue;
                }
                writeln!(
                    stderr,
                    "mkfifo: cannot create fifo '{expanded}': Operation not supported"
                )?;
                failed = true;
            }
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = if failed { 1 } else { 0 };
            Ok(true)
        }
    }
}

/// rubash#1636: whether an rm invocation carries an option shape
/// the emulation accepts but does not implement — the interactive
/// forms (-i/-I/--interactive, any `--interactive=WHEN`) and
/// --one-file-system. The scan mirrors the handler's classification:
/// options may appear anywhere before the `--` terminator (getopt
/// permutation); words after it are operands, as is a bare `-`.
fn rm_declines(words: &[String]) -> bool {
    let mut no_more_options = false;
    for word in words {
        let arg = word.as_str();
        if no_more_options {
            continue;
        }
        if arg == "--" {
            no_more_options = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or(long);
            if matches!(name, "interactive" | "one-file-system") {
                return true;
            }
            continue;
        }
        if arg.len() > 1
            && arg.starts_with('-')
            && arg[1..].chars().any(|ch| ch == 'i' || ch == 'I')
        {
            return true;
        }
    }
    false
}

/// rubash#1636: whether a mkdir invocation uses any option shape
/// other than the two the emulation implements exactly (-p and
/// --parents). The handler's parse consumes every other `-`-word as
/// a flag and drops it, so the declined class is "any option word in
/// flag position" (before the `--` terminator): -m/--mode in every
/// form, -v/--verbose, -Z/--context, bundles carrying them, unknown
/// options, and the --help/--version info requests.
fn mkdir_declines(words: &[String]) -> bool {
    let mut no_more_flags = false;
    for word in words {
        let arg = word.as_str();
        if no_more_flags {
            continue;
        }
        if arg == "--" {
            no_more_flags = true;
            continue;
        }
        if arg.len() > 1 && arg.starts_with('-') && arg != "-p" && arg != "--parents" {
            return true;
        }
    }
    false
}

/// rubash#1636: whether a touch invocation declines — it carries
/// -h/--no-dereference (parsed as a no-op; the stamp would land on
/// the symlink target instead of the link), or it has no operands at
/// all (the emulation exited 0 silently; GNU reports a missing file
/// operand). The walk mirrors the handler's option grammar so value
/// words for -t/-d/-r/--date/--reference/--time are never mistaken
/// for operands or options.
fn touch_declines(words: &[String]) -> bool {
    let mut operands = 0usize;
    let mut no_more_options = false;
    let mut index = 0;
    while index < words.len() {
        let arg = words[index].as_str();
        index += 1;
        if no_more_options || !arg.starts_with('-') || arg == "-" {
            operands += 1;
            continue;
        }
        if arg == "--" {
            no_more_options = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            };
            match name {
                "no-dereference" => return true,
                "date" | "reference" | "time" => {
                    if attached.is_none() {
                        index += 1;
                    }
                }
                _ => {}
            }
            continue;
        }
        let shorts = &arg[1..];
        let mut position = 0;
        let mut cluster_done = false;
        while position < shorts.len() && !cluster_done {
            let flag = shorts[position..].chars().next().unwrap_or('\0');
            position += flag.len_utf8();
            match flag {
                'h' => return true,
                't' | 'd' | 'r' => {
                    if position >= shorts.len() {
                        index += 1;
                    }
                    cluster_done = true;
                }
                _ => {}
            }
        }
    }
    operands == 0
}

/// rubash#1636: whether a cp invocation uses an option shape the
/// emulation accepts but does not implement — link creation
/// (-l/--link, -s/--symbolic-link: both produced a plain copy), the
/// no-dereference policy (-d/-P/--no-dereference: the copy always
/// follows symlinks), --parents, the backup machinery (--backup,
/// -S/--suffix), -Z/--context, -f/--force (the unlink-and-retry
/// semantics are not modeled), the --help/--version info requests
/// (--help printed the emulation's own short text to stderr), and
/// --update values other than the implemented `older` semantics
/// (any value, `all` included, was read as `older`; a bare --update
/// is `older` on the coreutils oracle and stays). The walk mirrors
/// the handler's grammar (notably -t consuming the next word).
fn cp_declines(words: &[String]) -> bool {
    let mut no_more_flags = false;
    let mut index = 0;
    while index < words.len() {
        let arg = words[index].as_str();
        index += 1;
        if no_more_flags || arg == "-" || !arg.starts_with('-') {
            continue;
        }
        if arg == "--" {
            no_more_flags = true;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--") {
            let (name, value) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (rest, None),
            };
            match name {
                "link" | "symbolic-link" | "parents" | "backup" | "context" | "force" | "help"
                | "version" | "no-dereference" | "suffix" => return true,
                "update" => {
                    if !matches!(value, None | Some("older")) {
                        return true;
                    }
                }
                "target-directory" | "target-dir" => {
                    if value.is_none() {
                        index += 1;
                    }
                }
                _ => {}
            }
            continue;
        }
        for ch in arg[1..].chars() {
            match ch {
                'l' | 's' | 'd' | 'P' | 'Z' | 'f' | 'S' => return true,
                't' => {
                    // The handler takes the next word as -t's value.
                    index += 1;
                    break;
                }
                _ => {}
            }
        }
    }
    false
}

/// cp's help text, scoped to the forms this implementation honours.
fn cp_usage_text() -> &'static str {
    "Usage: cp [OPTION]... SOURCE DEST\n\n\
     Copy SOURCE to DEST, or multiple SOURCES to DIRECTORY.\n\n\
     Supported: -a -d -f -i -l -n -P -p -R -r -s -u -v --archive --backup\
       --force --help --interactive --link --no-clobber --no-dereference\
       --parents --preserve[=ATTR_LIST] --remove-destination --recursive\
       --suffix=SUFFIX --symbolic-link --target-directory=DIR\
       --target-dir=DIR --update\n"
}

/// Whether an rm operand's target IS the root directory, for the
/// preserve-root failsafe (rubash#1847). A bare symlink operand
/// names the link itself — GNU unlinks it and never follows it to
/// the target — so symlink metadata excludes it; a trailing slash
/// makes symlink_metadata resolve through the link, so a slashed
/// link to the root still counts. Identity is canonical-path
/// equality with the root's own mapping, which covers the `//`
/// spelling and symlink chains without any name matching.
fn rm_operand_resolves_to_root(target: &std::path::Path, root: &std::path::Path) -> bool {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => return false,
        Ok(_) => {}
        Err(_) => return false,
    }
    match (fs::canonicalize(target), fs::canonicalize(root)) {
        (Ok(target), Ok(root)) => target == root,
        _ => false,
    }
}

/// The name the preserve-root diagnostic prints for an operand: the
/// walked path (fts) collapses a leading run of three or more
/// slashes to one, while a leading pair is preserved as its own
/// spelling — which is why `//` carries the ` (same as '/')` suffix
/// and `///` is reported as plain `/` (coreutils 9.4 probes).
fn rm_root_display_name(operand: &str) -> String {
    let leading = operand.bytes().take_while(|byte| *byte == b'/').count();
    if leading >= 3 {
        format!("/{}", &operand[leading..])
    } else {
        operand.to_string()
    }
}

/// cp quotes names with quotearg_colon: single quotes around the name, and
/// an embedded single quote becomes '\'' .
/// coreutils' last-component test for the rm dot refusal: trailing
/// slashes do not hide the component (`existing/../` and `./` still name
/// `..`/`.`); a bare root names the root, not a dot component.
fn rm_operand_is_dot_or_dotdot(operand: &str) -> bool {
    let normalized = operand.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    last == "." || last == ".."
}

fn cp_quoted_name(name: &str) -> String {
    let mut out = String::new();
    out.push('\u{27}');
    for ch in name.chars() {
        if ch == '\u{27}' {
            out.push('\u{27}');
            out.push('\u{5c}');
            out.push('\u{27}');
            out.push('\u{27}');
        } else {
            out.push(ch);
        }
    }
    out.push('\u{27}');
    out
}

/// cp's overwrite prompt. EOF or anything but a leading y/Y is a refusal.
fn cp_confirm_overwrite(target_display: &str) -> bool {
    use std::io::{BufRead, Write};
    let mut prompt = String::from("cp: overwrite ");
    prompt.push_str(&cp_quoted_name(target_display));
    prompt.push_str("? ");
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(prompt.as_bytes());
    let _ = stderr.flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(0) => false,
        Ok(_) => answer
            .trim_start()
            .starts_with(|c: char| c == 'y' || c == 'Y'),
        Err(_) => false,
    }
}

/// cp without -p sets the destination's modification time to now. Rust's
/// fs::copy preserves the source's timestamps (it uses CopyFileW on
/// Windows), so the value must be reset explicitly; otherwise -u compares
/// against the stale source time instead of the copy time.
#[cfg(windows)]
fn cp_set_modified_now(target: &std::path::Path) {
    use std::ffi::c_void;

    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const INVALID_HANDLE_VALUE: usize = usize::MAX;

    extern "system" {
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *mut c_void,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: usize,
        ) -> usize;
        fn SetFileTime(
            hFile: usize,
            lpCreationTime: *const c_void,
            lpLastAccessTime: *const c_void,
            lpLastWriteTime: *const c_void,
        ) -> i32;
        fn CloseHandle(hObject: usize) -> i32;
    }

    // FILETIME counts 100-nanosecond intervals since 1601-01-01 in UTC,
    // while SystemTime is anchored at 1970-01-01. The 11644473600-second
    // offset converts between the two.
    const FILETIME_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = (duration.as_secs() + FILETIME_EPOCH_OFFSET_SECS) * 10_000_000
        + (duration.subsec_nanos() as u64) / 100;
    let mut wide: Vec<u16> = target.to_string_lossy().encode_utf16().collect();
    wide.push(0);

    let last_write: i64 = ticks as i64;
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        if handle != INVALID_HANDLE_VALUE {
            SetFileTime(
                handle,
                std::ptr::null(),
                std::ptr::null(),
                &last_write as *const i64 as *const c_void,
            );
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
fn cp_set_modified_now(_target: &std::path::Path) {}

/// -u/--update: skip the copy when the source is not strictly newer than
/// the destination. Missing metadata is not grounds for skipping.
fn cp_source_not_newer(source: &std::path::Path, target: &std::path::Path) -> bool {
    match (
        fs::metadata(source).and_then(|meta| meta.modified()),
        fs::metadata(target).and_then(|meta| meta.modified()),
    ) {
        (Ok(source_time), Ok(target_time)) => source_time <= target_time,
        _ => false,
    }
}

/// cp -r: directories are created and walked in sorted entry order so the
/// result is deterministic. Symlinks are followed, which is cp's default
/// (without -d); the depth cap turns a link cycle into an error instead of
/// a stack overflow.
fn cp_copy_tree(
    source: &std::path::Path,
    target: &std::path::Path,
    depth: usize,
    preserve_mode: bool,
    preserve_times: bool,
) -> io::Result<()> {
    if depth > 40 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "too many levels of symbolic links",
        ));
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return cp_copy_tree(
            &fs::canonicalize(source)?,
            target,
            depth + 1,
            preserve_mode,
            preserve_times,
        );
    }
    if metadata.is_dir() {
        fs::create_dir_all(target)?;
        let mut entries: Vec<_> = fs::read_dir(source)?.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            cp_copy_tree(
                entry.path().as_path(),
                &target.join(entry.file_name()),
                depth + 1,
                preserve_mode,
                preserve_times,
            )?;
        }
        // GNU sets a copied directory's preserved attributes only after
        // its contents land, so child writes cannot clobber them.
        if preserve_mode || preserve_times {
            cp_apply_preserved(source, target, preserve_mode, preserve_times);
        }
        return Ok(());
    }
    fs::copy(source, target)?;
    if preserve_mode || preserve_times {
        cp_apply_preserved(source, target, preserve_mode, preserve_times);
    }
    Ok(())
}

/// One `--preserve`/`--no-preserve` attribute list (coreutils cp.c:
/// the bare option means the default list mode,ownership,timestamps;
/// `all` names every attribute). Only mode and timestamps are actionable
/// in this emulation; ownership/links/context/xattr parse and are
/// skipped, matching what an unprivileged GNU cp can actually honor.
fn cp_apply_preserve_list(list: Option<&str>, mode: &mut bool, times: &mut bool, value: bool) {
    let Some(list) = list else {
        *mode = value;
        *times = value;
        return;
    };
    for attr in list.split(',') {
        match attr.trim() {
            "mode" => *mode = value,
            "timestamps" => *times = value,
            "all" => {
                *mode = value;
                *times = value;
            }
            _ => {}
        }
    }
}

/// niubash#533: stamp a copy destination with the source's attributes
/// for `cp -p`/`-a`/`--preserve`. Times go on before the mode so a
/// restrictive preserved mode cannot block the timestamp write on
/// handle-based platforms; failures are ignored — preservation is
/// best-effort exactly where GNU's is (unprivileged ownership).
fn cp_apply_preserved(source: &std::path::Path, target: &std::path::Path, mode: bool, times: bool) {
    let Ok(meta) = fs::metadata(source) else {
        return;
    };
    if times {
        let file_times = std::fs::FileTimes::new()
            .set_accessed(meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH))
            .set_modified(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        // SetFileTime needs a write-attributes handle on Windows while
        // futimens accepts a read-only one on Unix; try both shapes.
        let stamped = File::options()
            .write(true)
            .open(target)
            .and_then(|file| file.set_times(file_times))
            .is_ok()
            || File::open(target)
                .and_then(|file| file.set_times(file_times))
                .is_ok();
        let _ = stamped;
    }
    if mode {
        let _ = fs::set_permissions(target, meta.permissions());
    }
}

/// GNU cat -v: display control characters using `^` notation and high-bit
/// bytes with `M-` prefix (coreutils cat.c cat_main + simple_cat). TAB and
/// LF pass through unchanged unless -T/-E are also given.
pub(in crate::executor) fn cat_v_filter(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for &byte in input {
        match byte {
            0x09 | 0x0a => output.push(byte),
            0x00..=0x08 | 0x0b..=0x1f => {
                output.push(b'^');
                output.push(byte + 0x40);
            }
            0x7f => {
                output.push(b'^');
                output.push(b'?');
            }
            0x80..=0xff => {
                output.extend_from_slice(b"M-");
                let low = byte & 0x7f;
                match low {
                    0x09 | 0x0a => output.push(low),
                    0x00..=0x08 | 0x0b..=0x1f => {
                        output.push(b'^');
                        output.push(low + 0x40);
                    }
                    0x7f => {
                        output.push(b'^');
                        output.push(b'?');
                    }
                    _ => output.push(low),
                }
            }
            _ => output.push(byte),
        }
    }
    output
}

/// GNU coreutils cat option model (rubash#415). Byte-for-byte behavioral
/// reference: WSL GNU coreutils 9.4 `/usr/bin/cat` probes
/// (target/i415/gnu-matrix.out, corners/corners2/perm/order2 probes,
/// 2026-10-02), which carry the observable cat.c contract:
/// * `-b' beats `-n' in EVERY argv order (`-nb', `-bn', `-n -b', `-b -n'
///   all number nonblank lines only — WSL probes, order2.sh).
/// * numbering is `%6d\t' right-aligned, grows past six digits, and does
///   NOT terminate a final line the input left unterminated.
/// * `-E'/`-A'/`-e' print `$' only where the input HAS a newline; an
///   unterminated final line gets neither `$' nor a newline.
/// * `-s' collapses runs of adjacent EMPTY lines to one (across operand
///   boundaries — the concatenated stream is one filter input).
/// * `-u' is accepted and ignored.
/// * GNU getopt permutes argv: `cat f -n' still numbers, `cat f -Z' still
///   reports the invalid option; `--' ends option parsing; a bare `-'
///   operand is stdin at that position.
#[derive(Default)]
pub(in crate::executor) struct CatOptions {
    /// -b/--number-nonblank: number non-empty lines only (wins over -n).
    number_nonblank: bool,
    /// -n/--number: number all lines.
    number_all: bool,
    /// -s/--squeeze-blank.
    squeeze_blank: bool,
    /// -v/--show-nonprinting (implied by -e/-t/-A).
    show_nonprinting: bool,
    /// -E/--show-ends (implied by -e/-A).
    show_ends: bool,
    /// -T/--show-tabs (implied by -t/-A).
    show_tabs: bool,
}

impl CatOptions {
    /// No transformation at all: the plain-concatenation fast path keeps
    /// its byte-identical passthrough (and the streaming stdin read).
    pub(in crate::executor) fn identity(&self) -> bool {
        !(self.number_nonblank
            || self.number_all
            || self.squeeze_blank
            || self.show_nonprinting
            || self.show_ends
            || self.show_tabs)
    }

    fn number_mode(&self) -> Option<CatNumbering> {
        if self.number_nonblank {
            Some(CatNumbering::Nonblank)
        } else if self.number_all {
            Some(CatNumbering::All)
        } else {
            None
        }
    }
}

enum CatNumbering {
    All,
    Nonblank,
}

/// GNU cat long-option table. Order is behavior-carrying: an ambiguous
/// abbreviation lists the possibilities in table order (`--numb' ->
/// "possibilities: '--number-nonblank' '--number'", `--show-' -> show-
/// nonprinting, show-ends, show-tabs, show-all — WSL 9.4 probes).
const CAT_LONG_OPTIONS: [(&str, &str); 9] = [
    ("number-nonblank", "b"),
    ("number", "n"),
    ("squeeze-blank", "s"),
    ("show-nonprinting", "v"),
    ("show-ends", "E"),
    ("show-tabs", "T"),
    ("show-all", "A"),
    ("help", ""),
    ("version", ""),
];

pub(in crate::executor) enum CatParsed<'a> {
    Options {
        options: CatOptions,
        /// Operand words in argv order (`-' operands included verbatim).
        operands: Vec<&'a String>,
    },
    /// `--help' / `--version': print the real binary's own text — not
    /// emulated, the caller falls through to the PATH subprocess.
    HelpOrVersion,
    /// Usage error: the two GNU diagnostic lines, exit status 1.
    Usage(String),
}

/// GNU getopt_long over cat's argv: short clusters (-benstuvAET), long
/// options with unambiguous-prefix matching, `--' terminator, `-'
/// operand, and argv permutation (options after operands still parse).
/// Words glued to redirection operators and redirect-target words stay
/// out of the operand list, like the previous classifier kept them.
/// Shared owner: both the simple-command path (external_cat) and the
/// inline pipeline stage arm parse through this one function
/// (niubash#165 — a second, option-dropping parser there lost -n/-s/-E
/// on pipe input).
pub(in crate::executor) fn parse_cat_argv(cmd: &CommandNode) -> CatParsed<'_> {
    let program = cmd.words.first().map(String::as_str).unwrap_or("cat");
    let mut options = CatOptions::default();
    let mut operands: Vec<&String> = Vec::new();
    let mut options_done = false;
    let redirect_targets = cat_redirect_targets(cmd);
    let mut skip_next = false;
    for word in cmd.words.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if is_cat_redirect_operator_word(word) {
            skip_next = true;
            continue;
        }
        if redirect_targets.iter().any(|target| *target == word) {
            continue;
        }
        if !options_done && word.starts_with("--") && word.len() > 2 {
            let name = &word[2..];
            // Exact match wins even when the name is also a prefix of a
            // longer option (GNU getopt_long).
            let mut matches: Vec<&str> = CAT_LONG_OPTIONS
                .iter()
                .filter(|(long, _)| *long == name)
                .map(|(long, _)| *long)
                .collect();
            if matches.is_empty() {
                matches = CAT_LONG_OPTIONS
                    .iter()
                    .filter(|(long, _)| long.starts_with(name))
                    .map(|(long, _)| *long)
                    .collect();
                if matches.len() > 1 {
                    let possibilities = matches
                        .iter()
                        .map(|long| format!("'--{long}'"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    return CatParsed::Usage(format!(
                        "{program}: option '--{name}' is ambiguous; possibilities: {possibilities}\nTry '{program} --help' for more information.\n"
                    ));
                }
            }
            let Some(long) = matches.first() else {
                return CatParsed::Usage(format!(
                    "{program}: unrecognized option '--{name}'\nTry '{program} --help' for more information.\n"
                ));
            };
            match *long {
                "number-nonblank" => options.number_nonblank = true,
                "number" => options.number_all = true,
                "squeeze-blank" => options.squeeze_blank = true,
                "show-nonprinting" => options.show_nonprinting = true,
                "show-ends" => options.show_ends = true,
                "show-tabs" => options.show_tabs = true,
                "show-all" => {
                    options.show_nonprinting = true;
                    options.show_ends = true;
                    options.show_tabs = true;
                }
                "help" | "version" => return CatParsed::HelpOrVersion,
                _ => unreachable!("table is exhaustive"),
            }
            continue;
        }
        if !options_done && word == "--" {
            options_done = true;
            continue;
        }
        if !options_done && word.starts_with('-') && word.len() > 1 {
            for ch in word[1..].chars() {
                match ch {
                    'b' => options.number_nonblank = true,
                    'n' => options.number_all = true,
                    's' => options.squeeze_blank = true,
                    'v' => options.show_nonprinting = true,
                    'E' => options.show_ends = true,
                    'T' => options.show_tabs = true,
                    'e' => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                    }
                    't' => {
                        options.show_nonprinting = true;
                        options.show_tabs = true;
                    }
                    'A' => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                        options.show_tabs = true;
                    }
                    'u' => {}
                    other => {
                        return CatParsed::Usage(format!(
                            "{program}: invalid option -- '{other}'\nTry '{program} --help' for more information.\n"
                        ));
                    }
                }
            }
            continue;
        }
        operands.push(word);
    }
    CatParsed::Options { options, operands }
}

/// Apply the parsed cat formatting options to one complete byte stream
/// (the concatenation of all operands / stdin — GNU's line filters see
/// one stream, so a squeeze or numbering counter carries across operand
/// boundaries). Byte semantics fixed by the WSL 9.4 probes above.
pub(in crate::executor) fn cat_format(input: &[u8], options: &CatOptions) -> Vec<u8> {
    if options.identity() {
        return input.to_vec();
    }
    let numbering = options.number_mode();
    let mut output = Vec::with_capacity(input.len() + input.len() / 8);
    let mut number = 0u64;
    let mut previous_blank = false;
    let mut rest = input;
    while !rest.is_empty() {
        let (line, terminated, remainder) = match rest.iter().position(|&b| b == b'\n') {
            Some(index) => (&rest[..index], true, &rest[index + 1..]),
            None => (rest, false, &[][..]),
        };
        rest = remainder;
        let blank = terminated && line.is_empty();
        if options.squeeze_blank && blank && previous_blank {
            continue;
        }
        previous_blank = blank;
        match numbering {
            Some(CatNumbering::All) => {
                number += 1;
                let _ = write!(output, "{number:>6}\t");
            }
            Some(CatNumbering::Nonblank) if !blank => {
                number += 1;
                let _ = write!(output, "{number:>6}\t");
            }
            _ => {}
        }
        if options.show_tabs {
            let mut escaped = Vec::with_capacity(line.len());
            for &byte in line {
                if byte == b'\t' {
                    escaped.extend_from_slice(b"^I");
                } else {
                    escaped.push(byte);
                }
            }
            output.extend_from_slice(&cat_v_content(&escaped, options));
        } else {
            output.extend_from_slice(&cat_v_content(line, options));
        }
        if terminated {
            if options.show_ends {
                output.push(b'$');
            }
            output.push(b'\n');
        }
        // An unterminated final line gets neither `$' nor a newline
        // (WSL 9.4 probes: `cat -E f' / `cat -n f' with no trailing LF).
    }
    output
}

/// The show-nonprinting escape over one line's content bytes (LF cannot
/// occur; a TAB passes through literally — only -T/-A convert it, and
/// that replacement already happened before this runs).
fn cat_v_content(line: &[u8], options: &CatOptions) -> Vec<u8> {
    if options.show_nonprinting {
        cat_v_filter(line)
    } else {
        line.to_vec()
    }
}

fn cat_redirect_targets(cmd: &CommandNode) -> Vec<&String> {
    [
        cmd.redirect_in.as_ref(),
        cmd.redirect_out.as_ref(),
        cmd.append.as_ref(),
        cmd.redirect_err.as_ref(),
        cmd.redirect_err_append.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(|redirect| &redirect.target)
    .collect()
}

fn is_cat_redirect_operator_word(word: &str) -> bool {
    matches!(
        word,
        "<" | ">" | ">|" | ">>" | "2>" | "2>|" | "2>>" | "&>" | "&>>"
    ) || word.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && matches!(
            word.chars()
                .skip_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .as_str(),
            "<" | ">" | ">|" | ">>"
        )
}

impl Executor {
    /// GNU chmod: [options] mode file... The emulated mode bits live in
    /// __RUBASH_FILE_MODES so test -r/-w/-x observe them (Windows has no
    /// POSIX mode bits). Symbolic clauses [ugoa]*[+-=][rwxX]+ (comma
    /// separated) and octal modes are honored; unknown option-looking words
    /// that precede the mode are ignored the way coreutils skips -f/-R/-v.
    fn external_chmod(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#1636/#1726 narrowing (2026-10-27 ruling): the parse
        // below evaluates exactly one form — a bare MODE plus file
        // operands, with a mode made of plain rwx bits. Every option
        // is accepted but never applied (-R does not recurse, -c/-v
        // never print, --reference is swallowed and the following
        // word is misparsed as the mode), and whole mode classes are
        // silently mishandled (special bits masked off by `& 0o777`,
        // the `s`/`t` letters and copy classes rejected). Those
        // shapes decline to the real binary (chmod_declines) — and so
        // do invocations with no mode or no file operand, which the
        // emulation answered silently where GNU reports a missing
        // operand.
        if chmod_declines(&cmd.words[1..]) {
            return Ok(false);
        }
        let mut mode: Option<&str> = None;
        let mut files: Vec<String> = Vec::new();
        for arg in &cmd.words[1..] {
            if mode.is_none() {
                if matches!(arg.as_str(), "-f" | "-R" | "-v" | "-c" | "--") || arg.starts_with("--")
                {
                    continue;
                }
                if arg.starts_with('-')
                    && arg.len() > 1
                    && arg[1..].chars().all(|ch| "fRvc".contains(ch))
                {
                    continue;
                }
                mode = Some(arg);
                continue;
            }
            files.push(arg.clone());
        }
        let Some(mode) = mode else {
            self.exit_code = 0;
            return Ok(true);
        };
        let mut failures = 0usize;
        // coreutils diagnostics are the emulated command's own stderr and
        // must honor the command's redirections (rubash#1372:
        // `chmod ... >f 2>&1` creates f holding the message); buffer them
        // and flush through the builtin output router at the end.
        let mut stderr: Vec<u8> = Vec::new();
        for file in &files {
            let windows =
                crate::executor::path::shell_path_to_windows(file, &self.shell_state.env_vars)
                    .to_string_lossy()
                    .to_string();
            // Resolve the host path once: unix applies the mode to the real
            // inode below, windows syncs the readonly attribute.
            let host = if std::path::Path::new(&windows).is_absolute() {
                std::path::PathBuf::from(&windows)
            } else {
                // G-02: relative operands anchor at the session cwd.
                self.shell_state.cwd.join(&windows)
            };
            // Base mode for symbolic clauses: unix reads the file's real
            // inode mode (coreutils chmod(1) applies `+x`/`-w` clauses to
            // the file's current mode via chmod(2)); windows keeps the
            // emulated store, which is the only mode source there.
            #[cfg(unix)]
            let base = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&host)
                    .map(|metadata| metadata.permissions().mode())
                    .unwrap_or(0o644)
            };
            #[cfg(not(unix))]
            let base = crate::builtins::test::emulated_file_mode(file, &self.shell_state.env_vars)
                .unwrap_or_else(|| self.default_emulated_mode(&windows));
            match apply_chmod_mode(base, mode, current_umask(&self.shell_state.env_vars)) {
                Some(new_mode) => {
                    #[cfg(unix)]
                    {
                        // coreutils chmod(1) -> chmod(2): the real inode mode
                        // is the single source of truth on unix; the readonly
                        // attribute + emulated store below is the Windows
                        // port (no POSIX mode bits there).
                        match std::fs::metadata(&host) {
                            Ok(metadata) => {
                                use std::os::unix::fs::PermissionsExt;
                                let mut permissions = metadata.permissions();
                                permissions.set_mode(new_mode);
                                if std::fs::set_permissions(&host, permissions).is_err() {
                                    failures += 1;
                                }
                            }
                            Err(_) => {
                                failures += 1;
                                writeln!(
                                    stderr,
                                    "chmod: cannot access '{}': No such file or directory",
                                    file
                                )?;
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        // DrvFs/WSL materializes exactly one POSIX permission
                        // bit: stripping all write bits sets the Windows readonly
                        // attribute, so `>file`/`>>file` fail EACCES like under
                        // GNU-on-WSL (redir12.sub). Sync it for every real path —
                        // relative names resolve against the drive-relative cwd.
                        match std::fs::metadata(&host) {
                            Ok(metadata) => {
                                let readonly = new_mode & 0o222 == 0;
                                let mut permissions = metadata.permissions();
                                if permissions.readonly() != readonly {
                                    permissions.set_readonly(readonly);
                                    if std::fs::set_permissions(&host, permissions).is_err() {
                                        failures += 1;
                                    }
                                }
                            }
                            Err(_) => {
                                failures += 1;
                                writeln!(
                                    stderr,
                                    "chmod: cannot access '{}': No such file or directory",
                                    file
                                )?;
                            }
                        }
                        // DrvFs does not support POSIX mode bits (GNU on WSL
                        // leaves files 0777 after `chmod -x`), so only non-drive
                        // paths get the emulated mode store; drive paths keep the
                        // `test -x` existence fallback (posix2 negative -x).
                        let is_drive = windows.len() >= 2
                            && windows.as_bytes()[1] == b':'
                            && windows.as_bytes()[0].is_ascii_alphabetic();
                        if !is_drive
                            && !windows.starts_with("\\\\wsl$")
                            && !windows.starts_with("//wsl$")
                        {
                            store_emulated_file_mode(
                                &mut self.shell_state.env_vars,
                                &windows,
                                new_mode,
                            );
                        }
                    }
                }
                None => {
                    failures += 1;
                    writeln!(stderr, "chmod: invalid mode: '{}'", mode)?;
                }
            }
        }
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        self.exit_code = i32::from(failures > 0 || files.is_empty());
        Ok(true)
    }

    /// Default rwx bits for a file never chmod'd: readable and writable like
    /// a fresh Windows file; executable only for extension-based executables
    /// (GNU-on-Linux would say not executable for a fresh text file).
    /// Windows-port-only: unix bases symbolic clauses on the real inode mode.
    #[cfg_attr(unix, allow(dead_code))]
    fn default_emulated_mode(&self, windows: &str) -> u32 {
        let executable = std::path::Path::new(windows)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "exe" | "com" | "bat" | "cmd"
                )
            })
            .unwrap_or(false);
        let mut mode = 0o600u32;
        if executable {
            mode |= 0o111;
        }
        mode
    }
}

/// The shell's current umask for applet mode computations: the tracked
/// `__RUBASH_UMASK` when the builtin set one, else the live process mask
/// (the builtin keeps the two in sync on unix).
fn current_umask(env_vars: &HashMap<String, String>) -> u32 {
    if let Some(mask) = env_vars
        .get("__RUBASH_UMASK")
        .and_then(|value| u32::from_str_radix(value, 8).ok())
    {
        return mask & 0o777;
    }
    #[cfg(unix)]
    {
        let mask = unsafe { libc::umask(0) };
        unsafe { libc::umask(mask) };
        (mask as u32) & 0o777
    }
    #[cfg(not(unix))]
    {
        0o022
    }
}

/// rubash#1636/#1726: whether a chmod invocation uses a shape the
/// emulation does not implement — see external_chmod's header. The
/// scan mirrors the handler's own classification (options and the
/// mode come before file operands; `--` makes the next word the mode
/// whatever it looks like) and judges only shape classes, never
/// individual cases.
fn chmod_declines(words: &[String]) -> bool {
    let mut mode: Option<&str> = None;
    let mut files = 0usize;
    let mut after_terminator = false;
    for word in words {
        let arg = word.as_str();
        if mode.is_some() {
            files += 1;
            continue;
        }
        if after_terminator {
            mode = Some(arg);
            if chmod_mode_declines(arg) {
                return true;
            }
            continue;
        }
        if arg == "--" {
            after_terminator = true;
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            // Every option declines: the short letters the handler
            // parses (-f/-R/-v/-c) are accepted but never applied,
            // and every long option is swallowed the same way.
            return true;
        }
        mode = Some(arg);
        if chmod_mode_declines(arg) {
            return true;
        }
    }
    mode.is_none() || files == 0
}

/// rubash#1726: whether one chmod MODE operand needs semantics the
/// emulation's evaluator does not have — an octal value carrying
/// special bits (setuid/setgid/sticky) the emulation silently masks
/// off with `& 0o777`, or a symbolic clause using the `s`/`t`
/// permission letters, a copy class (`u`/`g`/`o` in the permission
/// slot), or more than one operator. GNU coreutils chmod evaluates
/// all of these (modechange.c), so the invocation declines instead
/// of answering wrongly or failing a legal mode.
fn chmod_mode_declines(mode: &str) -> bool {
    if !mode.is_empty() && mode.chars().all(|ch| ch.is_ascii_digit()) {
        // Octal form (a digit string with 8/9 is invalid in both
        // implementations and stays on the emulation's error path).
        return u32::from_str_radix(mode, 8)
            .map(|value| value > 0o777)
            .unwrap_or(false);
    }
    for clause in mode.split(',') {
        let chars: Vec<char> = clause.chars().collect();
        let mut index = 0;
        while index < chars.len() && matches!(chars[index], 'u' | 'g' | 'o' | 'a') {
            index += 1;
        }
        let mut operators = 0;
        while index < chars.len() {
            if !matches!(chars[index], '+' | '-' | '=') {
                break;
            }
            operators += 1;
            if operators > 1 {
                return true;
            }
            index += 1;
            // A copy class is a lone u/g/o in the permission slot.
            if index < chars.len()
                && matches!(chars[index], 'u' | 'g' | 'o')
                && (index + 1 == chars.len() || matches!(chars[index + 1], '+' | '-' | '='))
            {
                return true;
            }
            while index < chars.len() && matches!(chars[index], 'r' | 'w' | 'x' | 'X') {
                index += 1;
            }
            if index < chars.len() && matches!(chars[index], 's' | 't') {
                return true;
            }
            if index < chars.len() && !matches!(chars[index], '+' | '-' | '=') {
                break;
            }
        }
    }
    false
}

/// Apply one chmod MODE operand (octal or symbolic clauses) to BASE.
fn apply_chmod_mode(base: u32, mode: &str, umask: u32) -> Option<u32> {
    let trimmed = mode.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed.len() <= 4 {
        return u32::from_str_radix(trimmed, 8)
            .ok()
            .map(|value| value & 0o777);
    }
    let mut current = base;
    for clause in trimmed.split(',') {
        let mut chars = clause.chars().peekable();
        let mut who = 0u32;
        let mut who_seen = false;
        while let Some(&ch) = chars.peek() {
            let bit = match ch {
                'u' => 0o700,
                'g' => 0o070,
                'o' => 0o007,
                'a' => 0o777,
                _ => break,
            };
            who |= bit;
            who_seen = true;
            chars.next();
        }
        if !who_seen {
            who = 0o777;
        }
        let op = chars.next()?;
        if !matches!(op, '+' | '-' | '=') {
            return None;
        }
        let mut perms = 0u32;
        while let Some(&ch) = chars.peek() {
            match ch {
                'r' => perms |= 0o444,
                'w' => perms |= 0o222,
                'x' => perms |= 0o111,
                'X' => {
                    // Directory, or some execute bit already set.
                    if (current & 0o111) != 0 {
                        perms |= 0o111;
                    }
                }
                _ => return None,
            }
            chars.next();
        }
        if chars.next().is_some() {
            return None;
        }
        // coreutils modechange.c (the omitted-who rule, POSIX chmod):
        // a clause that names no who is masked by the umask — its
        // permission bits are ANDed with ~umask before the operator
        // applies, for '+', '-' and '=' alike (rubash#1478: umask 007
        // `chmod +x` on 0660 must yield 0770, not 0771; measured
        // coreutils: `-w` on 0777 yields 0557, `=r` yields 0440).
        let effective = if who_seen { perms } else { perms & !umask };
        match op {
            '+' => {
                current |= who & effective;
            }
            '-' => {
                current &= !(who & effective);
            }
            '=' => {
                current = (current & !who) | (who & effective);
            }
            _ => return None,
        }
    }
    Some(current & 0o777)
}

/// Windows-port-only emulated mode store: unix chmod writes the real inode
/// mode instead (PermissionsExt::set_mode above).
#[cfg_attr(unix, allow(dead_code))]
fn store_emulated_file_mode(env_vars: &mut VarTable, windows: &str, mode: u32) {
    let key = crate::builtins::test::EMULATED_FILE_MODES;
    let entries = env_vars.get(key).cloned().unwrap_or_default();
    let mut kept: Vec<String> = entries
        .split(DATA_DOLLAR)
        .filter(|entry| {
            !entry.is_empty()
                && entry
                    .rsplit_once('=')
                    .map(|(path, _)| path != windows)
                    .unwrap_or(true)
        })
        .map(str::to_string)
        .collect();
    kept.push(format!("{}={:o}", windows, mode));
    env_vars.insert(key.to_string(), kept.join(DATA_DOLLAR_STR));
}
