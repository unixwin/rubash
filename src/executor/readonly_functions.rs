use super::*;

impl Executor {
    pub(in crate::executor) fn execute_readonly(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if readonly_args_request_functions(&cmd.words[1..]) {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status =
                self.execute_readonly_functions(&cmd.words[1..], &mut stdout, &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // GNU readonly.def shares declare.def's operand handling: a
        // `readonly name[sub]=value` subscript resolves under the same
        // ExpandedOnce (W_ASSIGNMENT -> ASS_NOEXPAND) rules.
        let args = match self.rewrite_declare_operand_subscripts(
            &cmd.words[1..],
            &cmd.word_metadata,
            "readonly",
        ) {
            Ok(args) => args,
            Err(()) => return Ok(1),
        };
        // GNU variables.c:3320 bind_variable_value → 2920-2937
        // make_variable_value: a `name=value` operand whose target carries
        // the integer attribute evaluates the RHS with evalexp (setattr.def
        // shares declare.def's operand handling) — `readonly int=100+42`
        // on an int local binds 142 (varenv25.sub). An evalexp failure
        // follows variables.c:2938-2944: print `readonly: <evalerror>`,
        // drop the failing operand and its successors, arm the DISCARD
        // abort, and fail the command.
        let (args, arith_eval_failure) = self.evaluate_integer_attribute_assignment_args(&args);
        if let Some(message) = &arith_eval_failure {
            if !message.is_empty() {
                writeln!(stderr, "{}readonly: {message}", self.error_prefix())?;
            }
            self.raise_evalerror_abort();
        }
        // GNU builtin_error_prolog prefixes readonly reassignment diagnostics
        // with `this_command_name`; while a function body runs, that name is
        // the enclosing function (execute_cmd.c run_builtin sets it on the
        // function call and the body's assignment-word expansion runs before
        // the builtin resets it).
        let context_name = self
            .shell_state
            .function_name_stack
            .first()
            .map(String::as_str);
        let mut oob_word = None;
        let mut traced_assignments = Vec::new();
        // rubash#1569: the -p listing renders the bindings underneath
        // this command's own tempenv frame, never the tempvars.
        let underlay = self.tempenv_listing_underlay();
        let status = crate::builtins::setattr::readonly_with_io(
            args.iter().map(String::as_str),
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
            context_name,
            &mut oob_word,
            Some(&mut traced_assignments),
            &underlay,
        )?;
        // GNU subst.c:3580: per-assignment xtrace lines, as in
        // execute_export (rubash#1340).
        self.xtrace_setattr_assignments(&traced_assignments);
        // rubash#616 (1a): `readonly -a name=(...)` rewrites the array mirror
        // inside the pure builtin — refresh the structured cell for
        // assignment operands (same mark-aware rule as export above, so the
        // assoc render of `readonly -A A=(...)` also re-syncs its cell).
        for arg in &args {
            if let Some((raw, _)) = arg.split_once('=') {
                let base = raw.trim_end_matches('+');
                let base = base.split('[').next().unwrap_or(base);
                if base.is_empty() {
                    continue;
                }
                if let Some(mirror) = self.shell_state.env_vars.get(base).cloned() {
                    if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base) {
                        sync_typed_assoc_storage(&mut self.shell_state.variables, base, &mirror);
                    } else {
                        sync_typed_indexed_storage(&mut self.shell_state.variables, base, &mirror);
                    }
                }
            }
        }
        // GNU err_badarraysub + any_failed (arrayfunc.c:804-809,
        // rubash#635): same nested-declare DISCARD as the export path
        // (subst.c:11169 bash_variable_assignment_error): bare
        // `<word>: bad array subscript`, no `readonly:` segment, list
        // aborted. Partial store already installed by the assign path.
        if let Some(word) = oob_word {
            writeln!(
                stderr,
                "{}{}: bad array subscript",
                self.diagnostic_prefix(),
                word
            )?;
            self.exit_code = 1;
            // Same eval-boundary class as execute_declare (rubash#637).
            self.evalerror_subscript_pending.set(true);
            self.raise_evalerror_abort();
            // Flush here: this early return skips the normal
            // buffered-output flush below (rubash#635).
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(1);
        }
        let status = if arith_eval_failure.is_some() {
            status.max(1)
        } else {
            status
        };
        // GNU execute_cmd.c:1004-1017 with the sh_invalidid/err_readonly
        // assignment class: in POSIX mode a non-interactive shell exits when
        // the special builtin readonly fails on an assignment (`readonly
        // 9bad=1`, `readonly ro3=2` on an existing readonly — oracle probes
        // both exit rc=1). Same dispatch-gate flag the export wrapper arms.
        if status != 0 && self.special_assignment_operand_failure(&args) {
            self.special_builtin_failed.set(true);
        }
        if status == 0 {
            self.sync_setattr_typed_assignments(cmd.words[1..].iter().map(String::as_str));
        }
        // PIPESTATUS cell-shape transitions for readonly's operands
        // (rubash#1363/#1364): a bare `readonly PIPESTATUS` before the
        // array materializes leaves a valueless cell the writer can
        // never fill; on a live array it changes nothing (the writer
        // ignores att_readonly). -p prints only.
        let print_requested = args.iter().any(|arg| {
            arg.len() > 1
                && arg.starts_with('-')
                && arg[1..].chars().all(|c| c.is_ascii_alphabetic())
                && arg[1..].contains('p')
        });
        if status == 0 {
            self.note_pipestatus_attribute_operands(&args, print_requested);
        }
        // A live readonly PIPESTATUS array prints in array form under
        // `readonly -p`, not the table's valueless line (rubash#1363).
        self.substitute_pipestatus_print_line(&mut stdout);
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_readonly_functions<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> io::Result<i32>
    where
        W: Write,
        E: Write,
    {
        let mut print = false;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                break;
            }
            for option in arg[1..].chars() {
                match option {
                    'f' => {}
                    'p' => print = true,
                    'a' | 'A' => {}
                    // GNU setattr.def ATTROPTS includes `n`; for readonly it's
                    // a silently-disallowed undo (no-op here too).
                    'n' => {}
                    other => {
                        writeln!(
                            stderr,
                            "{}readonly: -{other}: invalid option",
                            self.diagnostic_prefix()
                        )?;
                        writeln!(
                            stderr,
                            "readonly: usage: readonly [-aAf] [name[=value] ...] or readonly -p"
                        )?;
                        return Ok(2);
                    }
                }
            }
            index += 1;
        }

        if index >= args.len() {
            // declare.def set_or_show_attributes(NULL, f|r): `readonly -f`
            // and `readonly -fp` both print each readonly function's full
            // definition followed by the `declare -fr NAME` attribute line.
            let mut names = marked_env_names(&self.shell_state.env_vars, READONLY_FUNCTIONS);
            names.sort();
            for name in names {
                if let Some(body) = self.shell_state.functions.get(&name) {
                    self.write_function_definition(&name, &body.commands, false, stdout)?;
                    writeln!(stdout, "declare -fr {name}")?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for name in &args[index..] {
            let Some(body) = self.shell_state.functions.get(name) else {
                writeln!(
                    stderr,
                    "{}readonly: {name}: not a function",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            };
            if print {
                self.write_function_definition(name, &body.commands, false, stdout)?;
                writeln!(stdout, "declare -fr {name}")?;
            }
            mark_env_name(&mut self.shell_state.env_vars, READONLY_FUNCTIONS, name);
        }

        Ok(status)
    }

    pub(in crate::executor) fn write_function_definition<W>(
        &self,
        name: &str,
        body: &[CommandNode],
        exported: bool,
        stdout: &mut W,
    ) -> io::Result<()>
    where
        W: Write,
    {
        // declare -f prints the stored command tree through the GNU
        // print_cmd.c port so it matches `type NAME` and upstream bash,
        // including the body kind and definition-level redirects.
        let info = self.shell_state.function_def_infos.get(name);
        let text = crate::parser::ast_print::multiline_function_def_text_with(
            name,
            body,
            info.and_then(|info| info.body_kind),
            info.map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]),
        );
        writeln!(stdout, "{text}")?;
        // GNU setattr.def show_var_attributes: the definition prints
        // first, the `declare -fx NAME' attribute line follows it
        // (`export -pf`, `declare -pfx NAME`; rubash#1083).
        if exported {
            writeln!(stdout, "declare -fx {name}")?;
        }
        Ok(())
    }

    pub(in crate::executor) fn apply_exported_functions_to_child(&self, process: &mut Command) {
        for name in marked_env_names(&self.shell_state.env_vars, EXPORTED_FUNCTIONS) {
            let Some(body) = self.shell_state.functions.get(&name) else {
                continue;
            };
            let def_redirects = self
                .shell_state
                .function_def_infos
                .get(&name)
                .map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]);
            process.env(
                exported_function_env_name(&name),
                exported_function_env_value(&body.commands, def_redirects),
            );
        }
    }

    pub(in crate::executor) fn apply_child_environment(&self, process: &mut Command) {
        process.env_clear();
        // G-03: project the process-env mirror into the child env FIRST,
        // so the session overlays below override it name-by-name. The
        // mirror set (local_helpers::shell_assignment_needs_process_env:
        // the PATH/HOME family, the locale family, and every __RUBASH_*
        // protocol name) is written through to the process env on
        // assignment precisely so child processes can read it back —
        // every other spawn surface honors that transport (the
        // respawn/comsub env_map bases start from the ingested process
        // environment — env_helpers::process_env_pairs).
        // This builder env_clear()s, so without the projection the
        // mirror was a write-only sink for plain external commands: a
        // session's own mirrored writes never reached its children
        // (the multi-session harness caught a mirrored __RUBASH_* probe
        // invisible to the assigning session's /usr/bin/env). TMPDIR
        // stays out (set_process_env never mirrors it); __RUBASH_TRAP*
        // keys stay out except the merged ignore set, which the
        // transport_inherited_ignores arm below re-adds in child form —
        // the same strip the respawn env_map applies.
        for (name, value) in crate::executor::env_helpers::process_env_pairs() {
            if name == "TMPDIR"
                || !crate::executor::local_helpers::shell_assignment_needs_process_env(&name)
                || (name.starts_with("__RUBASH_TRAP")
                    && name != crate::builtins::trap::TRAP_ORIG_IGNORES)
                || !is_valid_process_env(&name, &value)
            {
                continue;
            }
            process.env(&name, self.child_env_value(&name, &value));
        }
        for name in marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS) {
            // GNU variables.c make_env_array_from_var_list: array and
            // associative variables never cross the child env boundary
            // (ARRAY_EXPORT off), exported attribute or not (rubash#534).
            if !is_exportable_env_name(&self.shell_state.env_vars, &name) {
                continue;
            }
            if let Some(value) = self.shell_state.env_vars.get(&name) {
                if is_valid_process_env(&name, value) {
                    process.env(&name, self.child_env_value(&name, value));
                }
            }
        }
        for (name, value) in local_export_env_values(&self.shell_state.env_vars) {
            // The stash yields to a bound exported local — the env-vars
            // loop above already wrote the local's own value
            // (local_export_env_entry_applies, rubash#970).
            if !local_export_env_entry_applies(&self.shell_state.env_vars, &name) {
                continue;
            }
            if is_valid_process_env(&name, &value) {
                process.env(&name, self.child_env_value(&name, &value));
            }
        }
        // SIG_IGN dispositions cross the process boundary in GNU (fork);
        // the per-signal "" trap keys cannot survive a Windows environment
        // block, so the merged ignore set rides in the ORIG_IGN variable
        // (trap.c original_signals -> SIG_HARD_IGNORE in the child).
        let inherited_ignores =
            crate::builtins::trap::transport_inherited_ignores(&self.shell_state.env_vars);
        if !inherited_ignores.is_empty() {
            process.env(crate::builtins::trap::TRAP_ORIG_IGNORES, inherited_ignores);
        }
        apply_required_windows_child_environment(process, &self.shell_state.env_vars);
        self.apply_exported_functions_to_child(process);
    }

    pub(in crate::executor) fn child_env_value(&self, name: &str, value: &str) -> String {
        if cfg!(windows) && name.eq_ignore_ascii_case("PATH") {
            return shell_path_to_process(value, &self.shell_state.env_vars);
        }
        if cfg!(windows) && name == "TMPDIR" {
            // unixwin/niubash#162: an exported TMPDIR must cross the child
            // boundary in Windows-native form (drive letter, forward slashes
            // so script children can re-parse it without backslash escapes —
            // docs/issue329-path-form-policy-analysis.md FFmpeg corollary).
            // The previous slash-drive `/c/...` form is not a Windows path:
            // foreign children that prefer TMPDIR over TEMP/TMP (Bun-compiled
            // executables, e.g. the opencode TUI) resolve it drive-relative,
            // the bunfs `B:` extraction then fails, and dlopen falls back to
            // the virtual `B:/~BUN/root/...` path — ERROR_MOD_NOT_FOUND 126.
            // Same boundary contract as the PATH branch above
            // (shell_path_to_process): shell display form inside, process
            // form outside. `/tmp` maps to the real temp backing dir via
            // shell_path_to_windows, so an exported POSIX TMPDIR stays
            // usable too.
            return shell_path_to_windows(value, &self.shell_state.env_vars)
                .to_string_lossy()
                .replace('\\', "/");
        }
        // GNU export puts the variable's raw bytes in the child's
        // environment (variables.c:4850 make_env_array_from_var_list:
        // line 4891 `value = value_cell (var)` then mk_env_string —
        // the value cell bytes are copied verbatim, no re-encoding). A
        // stored value carrying a raw-byte marker pair
        // must not leak the pair itself into the child's env block — the
        // child-argv boundary already maps each pair to its byte char
        // (rubash#141, path.rs decode_raw_byte_markers_to_byte_chars);
        // apply the same contract here so `x=$(printf '\377'); export x;
        // env` prints the byte, not U+E000 U+E100.
        crate::executor::substitution_metadata::decode_raw_byte_markers_to_byte_chars(value)
    }
}
