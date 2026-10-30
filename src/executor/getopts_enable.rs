use super::*;

impl Executor {
    pub(in crate::executor) fn execute_getopts_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = self.execute_getopts(cmd, &mut stderr);
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_getopts<W>(
        &mut self,
        cmd: &CommandNode,
        stderr: &mut W,
    ) -> i32
    where
        W: Write,
    {
        self.getopts_optind_arith_fatal = false;
        // getopts.def getopts_builtin: no_options rejects builtin-level
        // options BEFORE the operand count is examined — bare
        // `getopts --badflag` reports `--: invalid option` plus usage,
        // not usage alone (rubash#1259). sh_invalidopt names `-` plus
        // the first character after the dash, so `--badflag` gives `--`.
        if cmd.words.len() >= 2
            && cmd.words[1] != "--"
            && cmd.words[1].starts_with('-')
            && cmd.words[1].len() > 1
        {
            let option = cmd.words[1].chars().nth(1).unwrap_or('-');
            let _ = writeln!(
                stderr,
                "{}getopts: -{option}: invalid option",
                self.diagnostic_prefix()
            );
            let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
            return 2;
        }

        if cmd.words.len() < 3 {
            let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
            return 2;
        }

        // `list = loptend` skips over a leading `--`, so
        // `getopts -- optstring name ...` parses the next word as the
        // optstring instead of failing with a usage error.
        let mut word_index = 1usize;
        if cmd.words[1] == "--" {
            if cmd.words.len() < 4 {
                let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
                return 2;
            }
            word_index = 2;
        }

        let optstring = cmd.words[word_index].clone();
        // getopts.def dogetopts: the option scan (sh_getopt) runs and binds
        // OPTIND before any variable binding, so OPTIND still advances when
        // the name fails to bind afterwards (getopts7.sub:
        // `getopts :ab: opt-var "$@"` consumes -a, reports the identifier
        // error, and the loop stops with the option consumed).
        let variable = cmd.words[word_index + 1].clone();
        // Borrow the argument window instead of cloning it: every read
        // below completes before the binding calls (the current word is
        // consumed into `option_chars`, the separate-argument value is
        // cloned at its read point, and `args.len()` is a by-value
        // read), so no call observes a window the binds could move.
        // Cloning made every getopts call O($#) — a scan over thousands
        // of positionals paid a full Vec<String> clone per option.
        let args: &[String] = if cmd.words.len() > word_index + 2 {
            &cmd.words[word_index + 2..]
        } else {
            self.shell_state.positional_window()
        };

        let silent = optstring.starts_with(':');
        let optspec: &str = if silent { &optstring[1..] } else { &optstring };
        let mut optind = self.getopts_start_optind();
        let mut offset = self
            .shell_state
            .env_vars
            .get("__RUBASH_GETOPTS_OFFSET")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1);

        let Some(current) = args.get(optind.saturating_sub(1)) else {
            self.getopts_finish_eof(variable.as_str(), args.len() + 1, stderr);
            return 1;
        };
        if offset == 1 {
            if current == "--" {
                self.getopts_finish_eof(variable.as_str(), optind + 1, stderr);
                return 1;
            }
            if current == "-" || !current.starts_with('-') {
                self.getopts_finish_eof(variable.as_str(), optind, stderr);
                return 1;
            }
        }

        let option_chars: Vec<char> = current.chars().collect();
        let Some(option) = option_chars.get(offset).copied() else {
            self.getopts_finish_eof(variable.as_str(), optind + 1, stderr);
            return 1;
        };

        let consumed_arg = offset + 1 >= option_chars.len();
        if consumed_arg {
            optind += 1;
            offset = 1;
        } else {
            offset += 1;
        }

        let Some(spec_index) = optspec.find(option) else {
            // getopts.def G_INVALID_OPT: the OPTIND bind (line 243)
            // precedes every outcome branch; bind name "?" next (invalid
            // identifiers report here and fail the call), then either
            // bind OPTARG = option character (silent) or unbind OPTARG.
            // A name bind that died in the OPTIND arithmetic evaluation
            // (name == "OPTIND", value "?") abandons the call before the
            // OPTARG step runs (rubash#1270).
            self.shell_state
                .env_vars
                .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
            // The scan-time diagnostic comes out of sh_getopt itself,
            // BEFORE dogetopts binds OPTIND (line 243) — with a readonly
            // OPTIND the `illegal option` line precedes the readonly
            // report (rubash#1268/#1270).
            if !silent && self.getopts_opterr_enabled() {
                let _ = writeln!(stderr, "{}", self.getopts_invalid_option_diagnostic(option));
            }
            self.set_optind(optind, stderr);
            if silent {
                let status = self.getopts_bind_name(&variable, "?", stderr);
                if !self.take_getopts_optind_arith_fatal() {
                    self.getopts_bind_optarg_checked(&option.to_string(), stderr);
                }
                return status;
            }
            let status = self.getopts_bind_name(&variable, "?", stderr);
            if !self.take_getopts_optind_arith_fatal() {
                self.getopts_unbind_optarg();
            }
            return status;
        };

        let requires_arg = optspec[spec_index + option.len_utf8()..].starts_with(':');
        if requires_arg {
            let argument = if !consumed_arg {
                // Inline argument: offset already points past the option character,
                // so option_chars[offset..] contains the inline argument value.
                // For -w2 with offset=2 after consuming 'w', this collects "2".
                let value = option_chars[offset..].iter().collect::<String>();
                optind += 1;
                offset = 1;
                Some(value)
            } else {
                let value = args.get(optind.saturating_sub(1)).cloned();
                if value.is_some() {
                    optind += 1;
                }
                value
            };

            let Some(argument) = argument else {
                // getopts.def G_ARG_MISSING: the scan-time diagnostic
                // comes first; silent then binds name ":" and OPTARG =
                // option character, otherwise name "?" with OPTARG
                // unbound. The OPTIND bind (line 243) precedes all of it,
                // and an OPTIND arithmetic death at the name bind skips
                // the OPTARG step (rubash#1270).
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
                if !silent && self.getopts_opterr_enabled() {
                    let _ = writeln!(
                        stderr,
                        "{}: option requires an argument -- {option}",
                        self.script_name_value()
                    );
                }
                self.set_optind(optind, stderr);
                if silent {
                    let status = self.getopts_bind_name(&variable, ":", stderr);
                    if !self.take_getopts_optind_arith_fatal() {
                        self.getopts_bind_optarg_checked(&option.to_string(), stderr);
                    }
                    return status;
                }
                let status = self.getopts_bind_name(&variable, "?", stderr);
                if !self.take_getopts_optind_arith_fatal() {
                    self.getopts_unbind_optarg();
                }
                return status;
            };

            // Success (getopts.def): the OPTIND bind (line 243) lands
            // first, then OPTARG (line 301), then the option character
            // into name (line 305); the return code is the name bind's.
            // Binding the counter first is what lets a name of OPTIND be
            // overwritten by the arith-evaluated option character
            // (rubash#1270).
            self.set_optind(optind, stderr);
            self.shell_state
                .env_vars
                .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
            self.getopts_bind_optarg_checked(&argument, stderr);
            return self.getopts_bind_name(&variable, &option.to_string(), stderr);
        }

        self.set_optind(optind, stderr);
        self.shell_state
            .env_vars
            .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
        self.getopts_unbind_optarg();
        self.getopts_bind_name(&variable, &option.to_string(), stderr)
    }

    pub(in crate::executor) fn getopts_invalid_option_diagnostic(&self, option: char) -> String {
        if self.getopts_uses_ash_diagnostics() {
            format!("Illegal option -{option}")
        } else {
            format!("{}: illegal option -- {option}", self.script_name_value())
        }
    }

    fn getopts_uses_ash_diagnostics(&self) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_SHELL_NAME")
            .is_some_and(|name| is_ash_shell_name(name))
    }

    /// getopts.def getopts_bind_variable: bind through the assignment path;
    /// invalid identifiers report sh_invalidid and fail the call, while a
    /// disallowed (readonly) assignment returns EX_MISCERROR. The readonly
    /// diagnostic is reported through the builtin's error stream at the point
    /// of the call (getopts10.sub line 33), not through the deferred
    /// assignment-error path, to keep it ordered with the caller's output.
    fn getopts_bind_name<W>(&mut self, variable: &str, value: &str, stderr: &mut W) -> i32
    where
        W: Write,
    {
        if !is_shell_name(variable) {
            let _ = writeln!(
                stderr,
                "{}getopts: `{variable}': not a valid identifier",
                self.diagnostic_prefix()
            );
            return 1;
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, variable) {
            let _ = writeln!(
                stderr,
                "{}{}: readonly variable",
                self.error_prefix(),
                variable
            );
            return 2;
        }
        if variable == "OPTIND" {
            return self.getopts_bind_optind_name(value, stderr);
        }
        if self.getopts_apply_bind("getopts", variable, value, stderr) {
            0
        } else {
            2
        }
    }

    /// The name bind for name == "OPTIND" (rubash#1270): GNU creates
    /// OPTIND with att_integer, so bind_variable arith-evaluates the
    /// option character on its way in. A letter evaluates as an
    /// identifier (an unset name is 0; a set one's integer value is
    /// used) and overwrites the counter bound moments earlier, while
    /// `?`/`:` are evalexp syntax errors — the diagnostic prints, the
    /// call fails with status 1, OPTIND keeps the counter value, and
    /// the caller skips the pending OPTARG step.
    fn getopts_bind_optind_name<W>(&mut self, value: &str, stderr: &mut W) -> i32
    where
        W: Write,
    {
        let evaluated: i64 = match value {
            "?" | ":" => {
                let _ = writeln!(
                    stderr,
                    "{}getopts: {value}: arithmetic syntax error: operand expected (error token is \"{value}\")",
                    self.diagnostic_prefix()
                );
                self.getopts_optind_arith_fatal = true;
                return 1;
            }
            _ if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
                value.parse().unwrap_or(0)
            }
            _ if value.len() == 1
                && (value.as_bytes()[0].is_ascii_alphabetic() || value.as_bytes()[0] == b'_') =>
            {
                self.shell_state
                    .env_vars
                    .get(value)
                    .and_then(|current| current.trim().parse::<i64>().ok())
                    .unwrap_or(0)
            }
            _ => 0,
        };
        self.write_optind_value(evaluated.to_string());
        0
    }

    /// GNU getopts binds through bind_variable whose diagnostics go to the
    /// builtin's stderr — collect them into the buffered stream so they keep
    /// their position relative to diagnostics the caller already buffered
    /// (e.g. `illegal option -- h` precedes `getopts: `?': not a valid
    /// identifier`, nameref11.sub:31).
    fn getopts_apply_bind<W>(
        &mut self,
        command: &str,
        variable: &str,
        value: &str,
        stderr: &mut W,
    ) -> bool
    where
        W: Write,
    {
        self.buffer_assignment_diagnostics = true;
        let ok = self.apply_shell_assignment_command(command, variable, value.to_string());
        self.buffer_assignment_diagnostics = false;
        let pending = std::mem::take(&mut self.pending_assignment_diagnostics);
        let _ = stderr.write_all(&pending);
        ok
    }

    /// bind_variable("OPTARG", ...): a readonly OPTARG reports the
    /// "readonly variable" error through the builtin's stderr at the position
    /// of the getopts call and stays unset (getopts10.sub line 16), without
    /// changing the getopts return status.
    fn getopts_bind_optarg_checked<W>(&mut self, value: &str, stderr: &mut W)
    where
        W: Write,
    {
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, "OPTARG") {
            let _ = writeln!(stderr, "{}OPTARG: readonly variable", self.error_prefix());
            return;
        }
        self.getopts_apply_bind("getopts", "OPTARG", value, stderr);
    }

    /// getopts.def getopts_unbind_variable -> unbind_variable_noref: the
    /// variable is removed entirely, even with the readonly attribute (the
    /// unset builtin, not the variable layer, enforces readonly). getopts10.sub
    /// relies on this: the EOF unbind clears a readonly OPTARG so a later
    /// `typeset -n OPTARG=...` succeeds, and a nameref OPTARG is unset
    /// itself instead of following to its target.
    fn getopts_unbind_optarg(&mut self) {
        self.remove_env("OPTARG");
        self.shell_state.variables.remove("OPTARG");
        unmark_env_name(&mut self.shell_state.env_vars, READONLY_VARS, "OPTARG");
        unmark_env_name(&mut self.shell_state.env_vars, NAMEREF_VARS, "OPTARG");
    }

    /// variables.c sv_opterr: sh_opterr = OPTERR set and non-empty ?
    /// atoi(OPTERR) : 1. A non-numeric value parses as 0 (suppressed).
    fn getopts_opterr_enabled(&self) -> bool {
        match self.shell_state.env_vars.get("OPTERR") {
            Some(value) if !value.is_empty() => value
                .trim()
                .parse::<i64>()
                .map(|parsed| parsed != 0)
                .unwrap_or(false),
            _ => true,
        }
    }

    /// getopts.def G_EOF: OPTARG is unbound, name is bound to "?" (invalid
    /// identifiers report here too), and the return code is
    /// EXECUTION_FAILURE regardless of the bind result.
    fn getopts_finish_eof<W>(&mut self, variable: &str, optind: usize, stderr: &mut W)
    where
        W: Write,
    {
        // getopts.def: the OPTIND bind (line 243) precedes the G_EOF
        // branch too — a readonly OPTIND reports on the EOF call as well
        // (rubash#1268).
        self.set_optind(optind, stderr);
        self.getopts_unbind_optarg();
        let _ = self.getopts_bind_name(variable, "?", stderr);
        let _ = self.take_getopts_optind_arith_fatal();
        self.shell_state
            .env_vars
            .insert("__RUBASH_GETOPTS_OFFSET".to_string(), "1".to_string());
    }

    fn take_getopts_optind_arith_fatal(&mut self) -> bool {
        std::mem::take(&mut self.getopts_optind_arith_fatal)
    }

    /// The counter the next getopts call resumes from. GNU keeps it in
    /// the internal sh_optind (getopts.def dogetopts reads the OPTIND
    /// variable only through getopts_reset when the user assigns it);
    /// when OPTIND is readonly the variable can never carry the counter,
    /// so the advancing value rides in a shadow key instead
    /// (rubash#1268).
    fn getopts_start_optind(&self) -> usize {
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, "OPTIND") {
            if let Some(shadow) = self
                .shell_state
                .env_vars
                .get("__RUBASH_GETOPTS_OPTIND")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
            {
                return shadow;
            }
        }
        self.shell_state
            .env_vars
            .get("OPTIND")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1)
    }

    pub(in crate::executor) fn set_optind<W>(&mut self, optind: usize, stderr: &mut W)
    where
        W: Write,
    {
        // GNU getopts.def:243: dogetopts binds OPTIND through
        // bind_variable on every call. A readonly OPTIND reports
        // `OPTIND: readonly variable`, keeps its value, and the internal
        // counter still advances (rubash#1268) — the shadow key carries
        // it to the next call (see getopts_start_optind).
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, "OPTIND") {
            let _ = writeln!(stderr, "{}OPTIND: readonly variable", self.error_prefix());
            self.shell_state
                .env_vars
                .insert("__RUBASH_GETOPTS_OPTIND".to_string(), optind.to_string());
            return;
        }
        self.shell_state.env_vars.remove("__RUBASH_GETOPTS_OPTIND");
        self.write_optind_value(optind.to_string());
    }

    fn write_optind_value(&mut self, value: String) {
        self.shell_state
            .env_vars
            .insert("OPTIND".to_string(), value.clone());
        set_process_env("OPTIND", &value);

        // Also sync to shell_state.variables so parameter expansion sees the update
        if let Some(variable) = self.shell_state.variables.get_mut("OPTIND") {
            variable.value = crate::shell::ShellValue::Scalar(value.clone());
        } else {
            let _ = self.shell_state.variables.set_scalar("OPTIND", value);
        }
    }

    pub(in crate::executor) fn execute_enable(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::enable::execute_with_io(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }
}

fn is_ash_shell_name(name: &str) -> bool {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    matches!(leaf.to_ascii_lowercase().as_str(), "ash" | "ash.exe")
}
