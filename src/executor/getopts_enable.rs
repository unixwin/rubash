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

        let optstring = &cmd.words[1];
        if optstring == "--" {
            let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
            return 2;
        }
        if optstring.starts_with('-') && optstring.len() > 1 {
            let option = optstring.chars().nth(1).unwrap_or('-');
            let _ = writeln!(
                stderr,
                "{}getopts: -{option}: invalid option",
                self.diagnostic_prefix()
            );
            let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg ...]");
            return 2;
        }

        let variable = &cmd.words[2];
        if !is_shell_name(variable) {
            let _ = writeln!(
                stderr,
                "{}getopts: `{variable}': not a valid identifier",
                self.diagnostic_prefix()
            );
            return 2;
        }

        let args: Vec<String> = if cmd.words.len() > 3 {
            cmd.words[3..].to_vec()
        } else {
            self.positional_params.clone()
        };

        let silent = optstring.starts_with(':');
        let optspec = if silent {
            &optstring[1..]
        } else {
            optstring.as_str()
        };
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
            self.finish_getopts_scan(variable, optind, 1);
            return 1;
        };
        if offset == 1 {
            if current == "--" {
                self.finish_getopts_scan(variable, optind + 1, 1);
                return 1;
            }
            if current == "-" || !current.starts_with('-') {
                self.finish_getopts_scan(variable, optind, 1);
                return 1;
            }
        }

        let option_chars: Vec<char> = current.chars().collect();
        let Some(option) = option_chars.get(offset).copied() else {
            self.finish_getopts_scan(variable, optind + 1, 1);
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
            self.env_vars
                .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
            self.set_optind(optind);
            self.remove_env("OPTARG");
            self.apply_shell_assignment(variable, "?".to_string());
            if !silent && self.env_vars.get("OPTERR").map(String::as_str) != Some("0") {
                let _ = writeln!(
                    stderr,
                    "{}illegal option -- {option}",
                    self.script_name_value()
                );
            } else if silent {
                self.apply_shell_assignment("OPTARG", option.to_string());
            }
            return 0;
        };

        let requires_arg = optspec[spec_index + option.len_utf8()..].starts_with(':');
        if requires_arg {
            let argument = if !consumed_arg {
                let value = option_chars[offset - 1..].iter().collect::<String>();
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
                self.env_vars
                    .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
                self.set_optind(optind);
                if silent {
                    self.apply_shell_assignment(variable, ":".to_string());
                    self.apply_shell_assignment("OPTARG", option.to_string());
                    return 0;
                }
                self.remove_env("OPTARG");
                self.apply_shell_assignment(variable, "?".to_string());
                if self.env_vars.get("OPTERR").map(String::as_str) != Some("0") {
                    let _ = writeln!(
                        stderr,
                        "{}option requires an argument -- {option}",
                        self.script_name_value()
                    );
                }
                return 0;
            };

            self.apply_shell_assignment(variable, option.to_string());
            self.apply_shell_assignment("OPTARG", argument);
        } else {
            self.apply_shell_assignment(variable, option.to_string());
            self.remove_env("OPTARG");
        }

        self.env_vars
            .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
        self.set_optind(optind);
        0
    }

    pub(in crate::executor) fn finish_getopts_scan(
        &mut self,
        variable: &str,
        optind: usize,
        offset: usize,
    ) {
        self.apply_shell_assignment(variable, "?".to_string());
        self.remove_env("OPTARG");
        self.set_optind(optind);
        self.env_vars
            .insert("__RUBASH_GETOPTS_OFFSET".to_string(), offset.to_string());
    }

    pub(in crate::executor) fn set_optind(&mut self, optind: usize) {
        let value = optind.to_string();
        self.env_vars.insert("OPTIND".to_string(), value.clone());
        set_process_env("OPTIND", value);
    }

    pub(in crate::executor) fn execute_enable(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::enable::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::enable::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::enable::execute_with_io(
                    &cmd.words[1..],
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::enable::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::enable::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::enable::execute(
            &cmd.words[1..],
            &mut self.env_vars,
        )?)
    }
}
