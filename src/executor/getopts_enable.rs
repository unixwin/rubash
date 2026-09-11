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
        if cmd.words.len() < 3 {
            let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
            return 2;
        }

        // getopts.def getopts_builtin: no_options rejects builtin-level
        // options, then `list = loptend` skips over a leading `--`, so
        // `getopts -- optstring name ...` parses the next word as the
        // optstring instead of failing with a usage error.
        let mut word_index = 1usize;
        if cmd.words[1] == "--" {
            if cmd.words.len() < 4 {
                let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
                return 2;
            }
            word_index = 2;
        } else if cmd.words[1].starts_with('-') && cmd.words[1].len() > 1 {
            let option = cmd.words[1].chars().nth(1).unwrap_or('-');
            let _ = writeln!(
                stderr,
                "{}getopts: -{option}: invalid option",
                self.diagnostic_prefix()
            );
            let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
            return 2;
        }

        let optstring = cmd.words[word_index].clone();
        // getopts.def dogetopts: the option scan (sh_getopt) runs and binds
        // OPTIND before any variable binding, so OPTIND still advances when
        // the name fails to bind afterwards (getopts7.sub:
        // `getopts :ab: opt-var "$@"` consumes -a, reports the identifier
        // error, and the loop stops with the option consumed).
        let variable = cmd.words[word_index + 1].clone();
        let args: Vec<String> = if cmd.words.len() > word_index + 2 {
            cmd.words[word_index + 2..].to_vec()
        } else {
            self.positional_params.clone()
        };

        let silent = optstring.starts_with(':');
        let optspec: &str = if silent { &optstring[1..] } else { &optstring };
        let mut optind = self
            .env_vars
            .get("OPTIND")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1);
        let mut offset = self
            .env_vars
            .get("__RUBASH_GETOPTS_OFFSET")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1);

        let Some(current) = args.get(optind.saturating_sub(1)) else {
            self.getopts_finish_eof(variable.as_str(), optind, stderr);
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
            // getopts.def G_INVALID_OPT: bind name "?" first (invalid
            // identifiers report here and fail the call), then either bind
            // OPTARG = option character (silent) or unbind OPTARG and print
            // the diagnostic (unless OPTERR suppresses it).
            self.env_vars
                .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
            self.set_optind(optind);
            if silent {
                let status = self.getopts_bind_name(&variable, "?", stderr);
                self.getopts_bind_optarg_checked(&option.to_string(), stderr);
                return status;
            }
            self.getopts_unbind_optarg();
            if self.getopts_opterr_enabled() {
                let _ = writeln!(stderr, "{}", self.getopts_invalid_option_diagnostic(option));
            }
            return self.getopts_bind_name(&variable, "?", stderr);
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
                // getopts.def G_ARG_MISSING: silent binds name ":" and
                // OPTARG = option character; otherwise name "?" with OPTARG
                // unbound and the "option requires an argument" diagnostic
                // (suppressed when OPTERR is 0).
                self.env_vars
                    .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
                self.set_optind(optind);
                if silent {
                    let status = self.getopts_bind_name(&variable, ":", stderr);
                    self.getopts_bind_optarg_checked(&option.to_string(), stderr);
                    return status;
                }
                self.getopts_unbind_optarg();
                if self.getopts_opterr_enabled() {
                    let _ = writeln!(
                        stderr,
                        "{}: option requires an argument -- {option}",
                        self.script_name_value()
                    );
                }
                return self.getopts_bind_name(&variable, "?", stderr);
            };

            // Success: bind OPTARG, then the option character into name. The
            // return code is the name bind's result (getopts.def line 305).
            self.getopts_bind_optarg_checked(&argument, stderr);
            let status = self.getopts_bind_name(&variable, &option.to_string(), stderr);
            self.env_vars
                .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
            self.set_optind(optind);
            return status;
        }

        self.getopts_unbind_optarg();
        let status = self.getopts_bind_name(&variable, &option.to_string(), stderr);
        self.env_vars
            .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
        self.set_optind(optind);
        status
    }

    pub(in crate::executor) fn getopts_invalid_option_diagnostic(&self, option: char) -> String {
        if self.getopts_uses_ash_diagnostics() {
            format!("Illegal option -{option}")
        } else {
            format!("{}: illegal option -- {option}", self.script_name_value())
        }
    }

    fn getopts_uses_ash_diagnostics(&self) -> bool {
        self.env_vars
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
        if is_marked_var(&self.env_vars, READONLY_VARS, variable) {
            let _ = writeln!(
                stderr,
                "{}{}: readonly variable",
                self.diagnostic_prefix(),
                variable
            );
            return 2;
        }
        if self.apply_shell_assignment(variable, value.to_string()) {
            0
        } else {
            2
        }
    }

    /// bind_variable("OPTARG", ...): a readonly OPTARG reports the
    /// "readonly variable" error through the builtin's stderr at the position
    /// of the getopts call and stays unset (getopts10.sub line 16), without
    /// changing the getopts return status.
    fn getopts_bind_optarg_checked<W>(&mut self, value: &str, stderr: &mut W)
    where
        W: Write,
    {
        if is_marked_var(&self.env_vars, READONLY_VARS, "OPTARG") {
            let _ = writeln!(
                stderr,
                "{}OPTARG: readonly variable",
                self.diagnostic_prefix()
            );
            return;
        }
        self.apply_shell_assignment("OPTARG", value.to_string());
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
        unmark_env_name(&mut self.env_vars, READONLY_VARS, "OPTARG");
        unmark_env_name(&mut self.env_vars, NAMEREF_VARS, "OPTARG");
    }

    /// variables.c sv_opterr: sh_opterr = OPTERR set and non-empty ?
    /// atoi(OPTERR) : 1. A non-numeric value parses as 0 (suppressed).
    fn getopts_opterr_enabled(&self) -> bool {
        match self.env_vars.get("OPTERR") {
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
        self.getopts_unbind_optarg();
        let _ = self.getopts_bind_name(variable, "?", stderr);
        self.set_optind(optind);
        self.env_vars
            .insert("__RUBASH_GETOPTS_OFFSET".to_string(), "1".to_string());
    }

    pub(in crate::executor) fn set_optind(&mut self, optind: usize) {
        let value = optind.to_string();
        self.env_vars.insert("OPTIND".to_string(), value.clone());
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
            &mut self.env_vars,
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
