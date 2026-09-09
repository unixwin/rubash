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
        // GNU builtin_error_prolog prefixes readonly reassignment diagnostics
        // with `this_command_name`; while a function body runs, that name is
        // the enclosing function (execute_cmd.c run_builtin sets it on the
        // function call and the body's assignment-word expansion runs before
        // the builtin resets it).
        let context_name = self.function_name_stack.first().map(String::as_str);
        let status = crate::builtins::setattr::readonly_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &mut self.env_vars,
            &mut stdout,
            &mut stderr,
            context_name,
        )?;
        if status == 0 {
            self.sync_setattr_typed_assignments(cmd.words[1..].iter().map(String::as_str));
        }
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
            let mut names = marked_env_names(&self.env_vars, READONLY_FUNCTIONS);
            names.sort();
            for name in names {
                if let Some(body) = self.functions.get(&name) {
                    self.write_function_definition(&name, &body.commands, false, stdout)?;
                    writeln!(stdout, "declare -fr {name}")?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for name in &args[index..] {
            let Some(body) = self.functions.get(name) else {
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
            mark_env_name(&mut self.env_vars, READONLY_FUNCTIONS, name);
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
        if exported {
            writeln!(stdout, "declare -fx {name}")?;
        }
        // declare -f prints the stored command tree through the GNU
        // print_cmd.c port so it matches `type NAME` and upstream bash,
        // including the body kind and definition-level redirects.
        let info = self.function_def_infos.get(name);
        let text = crate::parser::ast_print::multiline_function_def_text_with(
            name,
            body,
            info.and_then(|info| info.body_kind),
            info.map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]),
        );
        writeln!(stdout, "{text}")
    }

    pub(in crate::executor) fn apply_exported_functions_to_child(&self, process: &mut Command) {
        for name in marked_env_names(&self.env_vars, EXPORTED_FUNCTIONS) {
            let Some(body) = self.functions.get(&name) else {
                continue;
            };
            let def_redirects = self
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
        for name in marked_env_names(&self.env_vars, EXPORTED_VARS) {
            if let Some(value) = self.env_vars.get(&name) {
                if is_valid_process_env(&name, value) {
                    process.env(&name, self.child_env_value(&name, value));
                }
            }
        }
        for (name, value) in local_export_env_values(&self.env_vars) {
            if is_valid_process_env(&name, &value) {
                process.env(&name, self.child_env_value(&name, &value));
            }
        }
        apply_required_windows_child_environment(process, &self.env_vars);
        self.apply_exported_functions_to_child(process);
    }

    pub(in crate::executor) fn child_env_value(&self, name: &str, value: &str) -> String {
        if cfg!(windows) && name.eq_ignore_ascii_case("PATH") {
            return shell_path_to_process(value, &self.env_vars);
        }
        if cfg!(windows) && name == "TMPDIR" {
            return shell_display_path(
                &shell_path_to_windows(value, &self.env_vars)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        value.to_string()
    }
}
