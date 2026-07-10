use super::*;

impl Executor {
    pub(in crate::executor) fn create_redirect_output(
        &self,
        target: &str,
        clobber: bool,
    ) -> io::Result<File> {
        let path = shell_path_to_windows(target, &self.env_vars);
        if !clobber && crate::builtins::set::shell_option_enabled(&self.env_vars, "noclobber") {
            return OpenOptions::new().write(true).create_new(true).open(path);
        }
        File::create(path)
    }

    pub(in crate::executor) fn apply_simple_set_flags(&mut self, args: &[String]) -> bool {
        if args.is_empty() {
            return false;
        }

        for arg in args {
            let Some(prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                return false;
            };
            let flags = &arg[1..];
            if flags.is_empty()
                || flags
                    .chars()
                    .any(|flag| !self.is_supported_short_set_flag(flag))
            {
                return false;
            }

            let enabled = prefix == '-';
            for flag in flags.chars() {
                match (flag, enabled) {
                    ('e', true) => {
                        self.env_vars
                            .insert("__RUBASH_ERREXIT".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "errexit", true);
                    }
                    ('e', false) => {
                        self.env_vars.remove("__RUBASH_ERREXIT");
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "errexit",
                            false,
                        );
                    }
                    ('x', true) => {
                        self.env_vars
                            .insert("__RUBASH_XTRACE".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", true);
                    }
                    ('x', false) => {
                        self.env_vars.remove("__RUBASH_XTRACE");
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", false);
                    }
                    ('u', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "nounset",
                            enabled,
                        );
                    }
                    ('C', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noclobber",
                            enabled,
                        );
                    }
                    ('f', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noglob",
                            enabled,
                        );
                    }
                    ('n', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noexec",
                            enabled,
                        );
                    }
                    (flag, _) => {
                        if let Some(option) = short_set_flag_option(flag) {
                            crate::builtins::set::set_shell_option(
                                &mut self.env_vars,
                                option,
                                enabled,
                            );
                        }
                    }
                }
            }
        }

        true
    }

    pub(in crate::executor) fn apply_set_positional_operands(&mut self, args: &[String]) -> bool {
        if args.is_empty() {
            return false;
        }

        let mut flag_updates = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            if arg == "--" {
                self.apply_set_flag_updates(&flag_updates);
                self.positional_params = args[index + 1..].to_vec();
                return true;
            }

            if arg == "-" {
                self.apply_set_flag_updates(&flag_updates);
                self.env_vars.remove("__RUBASH_XTRACE");
                crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", false);
                if index + 1 < args.len() {
                    self.positional_params = args[index + 1..].to_vec();
                }
                return true;
            }

            let Some(prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                self.apply_set_flag_updates(&flag_updates);
                self.positional_params = args[index..].to_vec();
                return true;
            };

            let flags = &arg[1..];
            if flags.is_empty()
                || flags
                    .chars()
                    .any(|flag| !self.is_supported_short_set_flag(flag))
            {
                return false;
            }

            flag_updates.push((prefix, flags.to_string()));
        }

        false
    }

    pub(in crate::executor) fn apply_set_flag_updates(&mut self, flag_updates: &[(char, String)]) {
        for (prefix, flags) in flag_updates {
            let enabled = *prefix == '-';
            for flag in flags.chars() {
                match (flag, enabled) {
                    ('e', true) => {
                        self.env_vars
                            .insert("__RUBASH_ERREXIT".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "errexit", true);
                    }
                    ('e', false) => {
                        self.env_vars.remove("__RUBASH_ERREXIT");
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "errexit",
                            false,
                        );
                    }
                    ('x', true) => {
                        self.env_vars
                            .insert("__RUBASH_XTRACE".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", true);
                    }
                    ('x', false) => {
                        self.env_vars.remove("__RUBASH_XTRACE");
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", false);
                    }
                    ('u', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "nounset",
                            enabled,
                        );
                    }
                    ('C', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noclobber",
                            enabled,
                        );
                    }
                    ('f', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noglob",
                            enabled,
                        );
                    }
                    (flag, _) => {
                        if let Some(option) = short_set_flag_option(flag) {
                            crate::builtins::set::set_shell_option(
                                &mut self.env_vars,
                                option,
                                enabled,
                            );
                        }
                    }
                }
            }
        }
    }

    pub(in crate::executor) fn is_supported_short_set_flag(&self, flag: char) -> bool {
        matches!(flag, 'e' | 'x' | 'u' | 'C' | 'f' | 'n') || short_set_flag_option(flag).is_some()
    }

    pub(in crate::executor) fn expand_case_word(&self, word: &str) -> String {
        if let Some(value) = tilde_expand::expand_word_prefix(word, &self.env_vars) {
            return value;
        }

        self.expand_word(word)
    }

    pub(in crate::executor) fn stdin_string_for_command(
        &self,
        cmd: &CommandNode,
    ) -> Option<String> {
        if let Some(body) = &cmd.heredoc {
            let quoted = body.starts_with('\x1e');
            let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
            if quoted {
                return Some(body.to_string());
            }
            return Some(self.expand_embedded_parameters(body));
        }

        if let Some(redirect) = &cmd.redirect_in {
            let target = self.expand_word(&redirect.target);
            let path = shell_path_to_windows(&target, &self.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path);
            }
            return fs::read_to_string(path).ok();
        }

        let word = cmd.here_string.as_ref()?;
        let mut input = decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
        input.push('\n');
        Some(input)
    }
}
