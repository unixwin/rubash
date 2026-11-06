use super::*;

impl Executor {
    // rubash#289d: the raw-stdout `execute_command_describe` (raw println!
    // describe, no buffering) was removed — every `command -v/-V`
    // invocation must go through `execute_command_describe_redirected`,
    // whose buffered output reaches the comsub capture via
    // write_buffered_builtin_output and honors redirections elsewhere
    // (mirrors the niubash #108 `type` precedent on this file).

    pub(in crate::executor) fn execute_command_describe_redirected(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if self.execute_command_describe_with_io(&cmd.words[1..], &mut stdout, &mut stderr)? {
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(true);
        }

        let action = {
            let row = super::builtin_table::builtin_table_row("command")
                .expect("command row in BUILTIN_TABLE");
            let mut diag =
                super::builtin_diag::Diag::new(self.diagnostic_prolog(), row, &mut stderr);
            crate::builtins::command::execute_with_io(row, &cmd.words[1..], &mut diag, &mut stdout)?
        };
        match action {
            crate::builtins::command::CommandAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                self.exit_code = status;
                Ok(true)
            }
            crate::builtins::command::CommandAction::Execute { .. } => Ok(false),
        }
    }

    pub(in crate::executor) fn execute_command_describe_with_io<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
        E: Write,
    {
        // The parse is only a predicate here: its Diag is a discard
        // sink over a throwaway buffer — a missing mode, an invalid
        // option, or `--help` all return None and the caller falls
        // through to the builtin's own scan, which reports them. The
        // sink MUST be throwaway: the predicate's BuiltinOpt reports
        // an invalid option through its Diag, and writing that into
        // the real stderr would print the diagnostic TWICE (once here,
        // once in the builtin's own scan — caught by the E1 after-run,
        // 2026-11-07: `command -z` printed the error twice).
        let mut discard: Vec<u8> = Vec::new();
        let Some((mode, use_standard_path, first_name)) = ({
            let row = super::builtin_table::builtin_table_row("command")
                .expect("command row in BUILTIN_TABLE");
            let mut diag =
                super::builtin_diag::Diag::new(self.diagnostic_prolog(), row, &mut discard);
            parse_command_describe_args(args, &mut diag)
        }) else {
            return Ok(false);
        };
        let saved_path = self.use_standard_path_for_lookup(use_standard_path);
        // builtins/command.def:96-97 command_builtin(): `if (list == 0)
        // return (EXECUTION_SUCCESS);` -- the no-operand early return comes
        // BEFORE the describe loop, so `command -v`/`-V` with no names
        // succeeds with no output (GNU probe rc=0) even though the
        // any_found accumulator below would otherwise report failure.
        let names = &args[first_name..];
        if names.is_empty() {
            self.restore_lookup_path(saved_path);
            self.exit_code = 0;
            return Ok(true);
        }
        // rubash#704 R6.2: `command -v`/`-V` use GNU's ANY-found rule
        // (builtins/command.def: `if (describe_command (...) == 0)
        // any_found++` -> EXECUTION_SUCCESS when one operand resolved).
        // `type` is the all-found builtin and keeps its own accumulator
        // below/elsewhere, so this loop must not double as it: the old
        // shared "any miss sets status 1" made `command -v echo nosuch`
        // return 1 while GNU returns 0.
        let mut any_found = false;
        for name in names {
            if !use_standard_path && !name.contains('/') && !name.contains('\\') {
                crate::builtins::hash::bump_hashed_path_hit(&mut self.shell_state.env_vars, name);
            }
            if self.describe_name_with_io(name, mode, false, false, stdout)? {
                any_found = true;
            } else if mode == TypeDescribeMode::Verbose {
                writeln!(
                    stderr,
                    "{}command: {name}: not found",
                    self.diagnostic_prefix()
                )?;
            }
        }
        self.restore_lookup_path(saved_path);
        self.exit_code = if any_found { 0 } else { 1 };
        Ok(true)
    }

    pub(in crate::executor) fn use_standard_path_for_lookup(
        &mut self,
        enabled: bool,
    ) -> Option<Option<String>> {
        if !enabled {
            return None;
        }

        let saved_path = self.shell_state.env_vars.get("PATH").cloned();
        let standard = standard_path(&self.shell_state.env_vars);
        self.shell_state
            .env_vars
            .insert("PATH".to_string(), standard);
        Some(saved_path)
    }

    pub(in crate::executor) fn restore_lookup_path(&mut self, saved_path: Option<Option<String>>) {
        let Some(saved_path) = saved_path else {
            return;
        };

        match saved_path {
            Some(path) => {
                self.shell_state.env_vars.insert("PATH".to_string(), path);
            }
            None => {
                self.shell_state.env_vars.remove("PATH");
            }
        }
    }

    // niubash #108: the raw-stdout `execute_type` was removed — every `type`
    // invocation must go through `execute_type_redirected`, whose buffered
    // output reaches `self.stdout_capture` under command substitution
    // (`$(type -t ls)`) and honors explicit redirections elsewhere.

    pub(in crate::executor) fn execute_type_redirected(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = self.execute_type_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_type_with_io<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> Result<i32, ExecuteError>
    where
        W: Write,
        E: Write,
    {
        if let Some(status) = self.execute_type_with_disabled_builtin_state_with_io(args, stdout)? {
            return Ok(status);
        }

        // Stage-2 ④ Phase 2: the option scan is the (table row +
        // `BuiltinOpt` + `Diag`) triple (`internal_getopt (list,
        // "afptP")`, builtins/type.def:145-146). GNU runs a PRESCAN
        // first (type.def:122-141): the loop walks the LEADING dash
        // words and rewrites `-type`/`--type` to `-t`, `-path`/`--path`
        // to `-p`, `-all`/`--all` to `-a` IN PLACE, continuing past a
        // bare `-` and stopping at the first non-dash word — so
        // `type - -type echo` and `type -- -type echo` describe `-t`
        // as a NAME (rubash#2940). Only then does getopt scan the
        // rewritten list.
        let row =
            super::builtin_table::builtin_table_row("type").expect("type row in BUILTIN_TABLE");
        let rewritten: Vec<String> = {
            let mut words: Vec<String> = args.to_vec();
            for word in words.iter_mut() {
                if !word.starts_with('-') {
                    break;
                }
                let rest = &word[1..];
                if rest == "type" || rest == "-type" {
                    *word = "-t".to_string();
                } else if rest == "path" || rest == "-path" {
                    *word = "-p".to_string();
                } else if rest == "all" || rest == "-all" {
                    *word = "-a".to_string();
                }
            }
            words
        };
        let mut diag = super::builtin_diag::Diag::new(self.diagnostic_prolog(), row, stderr);
        let (all, force_path, mode, skip_functions, help, names) = {
            let mut opt = super::builtin_opt::BuiltinOpt::new(row, &rewritten, &mut diag, false);
            let mut all = false;
            let mut force_path = false;
            // GNU builtins/type.def:153-154 — `-f` sets CDESC_NOFUNCS,
            // which suppresses ONLY the function lookup (type.def:281);
            // aliases, keywords, builtins and files are still reported.
            let mut skip_functions = false;
            let mut mode = TypeDescribeMode::Verbose;
            let mut help = false;
            loop {
                match opt.next() {
                    super::builtin_opt::OptEvent::Opt(b'a') => all = true,
                    super::builtin_opt::OptEvent::Opt(b'f') => skip_functions = true,
                    super::builtin_opt::OptEvent::Opt(b'p') => {
                        mode = TypeDescribeMode::PathOnly;
                        force_path = false;
                    }
                    super::builtin_opt::OptEvent::Opt(b'P') => {
                        mode = TypeDescribeMode::PathOnly;
                        force_path = true;
                    }
                    super::builtin_opt::OptEvent::Opt(b't') => mode = TypeDescribeMode::TypeOnly,
                    super::builtin_opt::OptEvent::Help => {
                        help = true;
                        break;
                    }
                    super::builtin_opt::OptEvent::End => break,
                    // The row's spec is `"afptP"`: no other option
                    // character (and no argument-taking option) can
                    // ever arrive.
                    super::builtin_opt::OptEvent::Opt(_)
                    | super::builtin_opt::OptEvent::OptArg(_, _) => {
                        unreachable!("type spec is \"afptP\"")
                    }
                }
            }
            let names: Vec<String> = opt.operands().iter().map(|s| s.to_string()).collect();
            (all, force_path, mode, skip_functions, help, names)
        };
        if help {
            crate::builtins::help::print_builtin_help(row.name, stdout)?;
            return Ok(2);
        }
        if diag.status() != 0 {
            diag.usage();
            return Ok(diag.status());
        }

        let mut status = 0;
        for name in &names {
            // GNU type.def -> describe_command -> find_user_command: each
            // non-absolute name lookup runs phash_search, whose hit bumps
            // times_found (hashlib.c:254). `-P` (force_path) skips the table.
            if !force_path && !name.contains('/') && !name.contains('\\') {
                crate::builtins::hash::bump_hashed_path_hit(&mut self.shell_state.env_vars, name);
            }
            let found = if all {
                self.describe_name_all_with_io(name, mode, force_path, skip_functions, stdout)?
            } else {
                self.describe_name_with_io(name, mode, force_path, skip_functions, stdout)?
            };
            if !found {
                status = 1;
                if mode == TypeDescribeMode::Verbose {
                    diag.free(format_args!("{name}: not found"));
                }
            }
        }
        Ok(status)
    }
}
