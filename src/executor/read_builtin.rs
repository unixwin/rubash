use super::*;

const READ_USAGE: &str =
    "read: usage: read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]";

/// GNU read.def:405: `read` accepts a name that is either a valid identifier
/// or a valid array reference `name[subscript]` (array.tests:80 `read x[1]`).
/// The array-reference check is `valid_array_reference(name, arrayflags)`
/// (arrayfunc.c:1350 -> tokenize_array_reference arrayfunc.c:1288) with the
/// `SET_VFLAGS` flag set (builtins/common.h:279): `VA_NOEXPAND` follows
/// `array_expand_once`; `VA_ONEWORD` is added only when the option is on
/// AND the raw word carried `W_ARRAYREF` (execute_cmd.c:4370). The double
/// flag set takes the LAST `]` for an assoc (`read "A[]]"` keys on `]`),
/// `VA_NOEXPAND` alone takes the first (`read 'a[80's]'` keys on `80's`
/// but `A[]]` is invalid), and flag-0 runs the quote-aware matched-pair
/// scan (`read a[80's]` reports not-a-valid-identifier).
fn is_valid_read_name(
    name: &str,
    w_arrayref: bool,
    env_vars: &crate::shell::var_table::VarTable,
) -> bool {
    // W_ARRAYREF arrives in-band as an ARRAYREF_FLAG prefix on the operand
    // text; it is a word flag, not name bytes, so strip it before the
    // identifier/reference checks (execute_cmd.c:4366 fix_arrayref_words).
    let name = crate::builtins::arrayref::take_arrayref_flag(name).1;
    if is_shell_name(name) {
        return true;
    }

    let expand_once = crate::builtins::shopt::option_enabled(env_vars, "array_expand_once");
    crate::executor::subscript_expansion::valid_array_reference_env(
        name,
        expand_once,
        expand_once && w_arrayref,
        env_vars,
    )
}

impl Executor {
    pub(in crate::executor) fn execute_read(&mut self, cmd: &CommandNode) -> i32 {
        // GNU redir.c do_redirections opens EVERY redirection before
        // the builtin runs: output targets are created/truncated and a
        // failed open aborts the command (rubash#1593 — `read l >f1`
        // created nothing and `read l >sub/f1` failed silently). The
        // fd-0 input probe below owns only the input side; this walk
        // performs the remaining opens in list order.
        match self.apply_redirect_setup_side_effects(cmd) {
            Ok(true) => return 1,
            Ok(false) => {}
            Err(_) => return 1,
        }
        // GNU execute_cmd.c do_redirections applies redirects before the
        // builtin runs: a failed input open aborts the command — read's
        // variables stay unset and the diagnostic is the redirect's
        // (probe: `read a < /nonexist` leaves `a` unset, status 1). The
        // in-reader open treats failure as EOF, which wrongly assigns "".
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 && redirect.fd_var.is_none() {
                let target = self.expand_redirect_target(redirect);
                if !is_closed_redirect_target(&target)
                    && redirect_target_fd_and_move(&target).is_none()
                    && !target.starts_with("<(")
                {
                    if let Err(error) = self.probe_input_redirect(&target) {
                        let mut line = Vec::new();
                        let _ = writeln!(
                            &mut line,
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        // GNU redir.c:135 redirection_error via do_redirections
                        // (redir.c:260): the diagnostic goes through the fd 2
                        // the command's earlier redirections already bound, so
                        // `read x 2>/dev/null < /missing` stays silent while
                        // the reversed order leaks (issue #250).
                        let _ = self.write_redirect_diagnostic_routed(cmd, &line);
                        return 1;
                    }
                }
            }
        }
        let mut stderr = Vec::new();
        let mut array_name = None;
        let mut delimiter = '\n';
        let mut char_limit = None;
        let mut exact_char_limit = false;
        let mut raw = false;
        let mut scalar_names = Vec::new();
        let mut scalar_field_count = 0usize;
        let mut invalid_name = false;
        // rubash#1301: an invalid -a argument is recorded here, not
        // reported inline — GNU read.def validates the plain names
        // first and the -a argument only after all of them pass.
        let mut invalid_array_name: Option<String> = None;
        // Whether a plain (non-option) name word has been scanned yet.
        // GNU's option phase ends at the first plain name; an option
        // letter after it is a NAME, not an option (see the -a arms).
        let mut seen_plain_name = false;
        let mut stop_scalar_names = false;
        let mut prompt: Option<String> = None;
        let mut initial_text: Option<String> = None;
        let mut read_fd: Option<u32> = None;
        let mut timeout_zero = false;
        // GNU read.def:302-351/443-458: the -t operand is a floating-point
        // second count (have_timeout); TMOUT supplies the default when -t
        // is absent.
        let mut timeout_secs: Option<f64> = None;
        let mut index = 1;
        while index < cmd.words.len() {
            match cmd.words[index].as_str() {
                "--" => {
                    index += 1;
                    while index < cmd.words.len() {
                        if stop_scalar_names {
                            index += 1;
                            continue;
                        }

                        if is_valid_read_name(
                            &cmd.words[index],
                            self.word_is_arrayref(cmd, index),
                            &self.shell_state.env_vars,
                        ) {
                            scalar_names.push(
                                crate::builtins::arrayref::take_arrayref_flag(&cmd.words[index])
                                    .1
                                    .to_string(),
                            );

                            scalar_field_count += 1;
                        } else {
                            report_read_invalid_identifier(
                                &mut stderr,
                                &self.diagnostic_prefix(),
                                &cmd.words[index],
                            );
                            invalid_name = true;
                            scalar_field_count += 1;
                            stop_scalar_names = true;
                        }
                        index += 1;
                    }
                }
                "-a" => {
                    let Some(name) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -a: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                "-ar" | "-ra" => {
                    raw = true;
                    let Some(name) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                word if word.starts_with("-ra") && word.len() > 3 => {
                    raw = true;
                    let name = &word[3..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                "-rsa" | "-sra" => {
                    raw = true;
                    let Some(name) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                word if (word.starts_with("-rsa") || word.starts_with("-sra"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let name = &word[4..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                "-ersa" | "-esra" | "-resa" | "-rsea" | "-sera" | "-srea" => {
                    raw = true;
                    let Some(name) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                word if (word.starts_with("-ersa")
                    || word.starts_with("-esra")
                    || word.starts_with("-resa")
                    || word.starts_with("-rsea")
                    || word.starts_with("-sera")
                    || word.starts_with("-srea"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    let name = &word[5..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                "-sa" => {
                    let Some(name) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -sa: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                word if word.starts_with("-sa") && word.len() > 3 => {
                    let name = &word[3..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                "-ea" => {
                    let Some(name) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -ea: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                word if word.starts_with("-ea") && word.len() > 3 => {
                    let name = &word[3..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                "-rea" | "-era" => {
                    raw = true;
                    let Some(name) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 2;
                }
                word if (word.starts_with("-rea") || word.starts_with("-era"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let name = &word[4..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                word if word.starts_with("-a") && word.len() > 2 => {
                    let name = &word[2..];
                    if is_shell_name(name) {
                        array_name = Some(
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                        );
                        invalid_array_name = None;
                    } else {
                        // rubash#1301: deferred — reported after the
                        // plain names, only if they all pass (see the
                        // post-parse validation below).
                        if seen_plain_name {
                            // An option letter after the first plain name is a
                            // name in GNU (the option phase has ended), so its
                            // invalid argument folds into the plain-name failure
                            // instead of the deferred -a path.
                            invalid_name = true;
                            stop_scalar_names = true;
                        } else {
                            invalid_array_name = Some(name.to_string());
                        }
                        array_name = None;
                    }
                    index += 1;
                }
                "-d" => {
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                "-n" => {
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                "-N" => {
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                "-u" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -u: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                "-i" => {
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(next_word.clone());
                    index += 2;
                }
                "-t" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -t: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                "-p" => {
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                "-e" => {
                    index += 1;
                }
                // GNU read.def:308: `-E` disables readline (inverse of `-e`).
                // Rubash has no readline editor for read, so it's a no-op
                // (rubash#1407).
                "-E" => {
                    index += 1;
                }
                "-r" => {
                    raw = true;
                    index += 1;
                }
                "-s" => {
                    index += 1;
                }
                "-sn" => {
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                word if word.starts_with("-sn") && word.len() > 3 => {
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                "-sN" => {
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                word if word.starts_with("-sN") && word.len() > 3 => {
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                "-en" => {
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                word if word.starts_with("-en") && word.len() > 3 => {
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                "-ren" | "-ern" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                word if (word.starts_with("-ren") || word.starts_with("-ern"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    char_limit = match read_char_limit_argument(Some(&word[4..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                "-eN" => {
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                word if word.starts_with("-eN") && word.len() > 3 => {
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                "-reN" | "-erN" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                word if (word.starts_with("-reN") || word.starts_with("-erN"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    char_limit = match read_char_limit_argument(Some(&word[4..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                "-sp" => {
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                word if word.starts_with("-sp") && word.len() > 3 => {
                    prompt = Some(word[3..].to_string());
                    index += 1;
                }
                "-ep" => {
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                word if word.starts_with("-ep") && word.len() > 3 => {
                    prompt = Some(word[3..].to_string());
                    index += 1;
                }
                "-rep" | "-erp" => {
                    raw = true;
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                word if (word.starts_with("-rep") || word.starts_with("-erp"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    prompt = Some(word[4..].to_string());
                    index += 1;
                }
                "-ersp" | "-esrp" | "-resp" | "-rsep" | "-serp" | "-srep" => {
                    raw = true;
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                word if (word.starts_with("-ersp")
                    || word.starts_with("-esrp")
                    || word.starts_with("-resp")
                    || word.starts_with("-rsep")
                    || word.starts_with("-serp")
                    || word.starts_with("-srep"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    prompt = Some(word[5..].to_string());
                    index += 1;
                }
                "-ei" => {
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(next_word.clone());
                    index += 2;
                }
                word if word.starts_with("-ei") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-rei" | "-eri" => {
                    raw = true;
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(next_word.clone());
                    index += 2;
                }
                word if (word.starts_with("-rei") || word.starts_with("-eri"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    initial_text = Some(word[4..].to_string());
                    index += 1;
                }
                "-ersi" | "-esri" | "-resi" | "-rsei" | "-seri" | "-srei" => {
                    raw = true;
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(next_word.clone());
                    index += 2;
                }
                word if (word.starts_with("-ersi")
                    || word.starts_with("-esri")
                    || word.starts_with("-resi")
                    || word.starts_with("-rsei")
                    || word.starts_with("-seri")
                    || word.starts_with("-srei"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    initial_text = Some(word[5..].to_string());
                    index += 1;
                }
                "-et" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -et: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                word if word.starts_with("-et") && word.len() > 3 => {
                    let value = &word[3..];
                    match parse_read_timeout(value) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 1;
                }
                "-ret" | "-ert" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                word if (word.starts_with("-ret") || word.starts_with("-ert"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let value = &word[4..];
                    match parse_read_timeout(value) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 1;
                }
                "-eu" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -eu: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                word if word.starts_with("-eu") && word.len() > 3 => {
                    let value = &word[3..];
                    read_fd = match parse_read_fd(value) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 1;
                }
                "-reu" | "-eru" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                word if (word.starts_with("-reu") || word.starts_with("-eru"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let value = &word[4..];
                    read_fd = match parse_read_fd(value) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 1;
                }
                "-id" => {
                    initial_text = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-id") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-di" => {
                    initial_text = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-di") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-ie" => {
                    initial_text = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-ie") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-si" => {
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(next_word.clone());
                    index += 2;
                }
                word if word.starts_with("-si") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-is" => {
                    initial_text = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-is") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-ni" => {
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {}: invalid character count specification",
                                self.diagnostic_prefix(),
                                word
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    initial_text = cmd.words.get(index + 2).cloned();
                    index += 3;
                }
                word if word.starts_with("-ni") && word.len() > 3 => {
                    if let Ok(limit) = word[3..].parse::<usize>() {
                        char_limit = Some(limit);
                    }
                    index += 1;
                }
                "-in" => {
                    initial_text = cmd.words.get(index + 1).cloned();
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 2)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {}: invalid character count specification",
                                self.diagnostic_prefix(),
                                word
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 3;
                }
                word if word.starts_with("-in") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-edi" => {
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
                    initial_text = cmd.words.get(index + 2).cloned();
                    index += 3;
                }
                word if word.starts_with("-edi") && word.len() > 4 => {
                    delimiter = read_delimiter_char(&word[3..]);
                    initial_text = Some(word[4..].to_string());
                    index += 1;
                }
                "-dei" => {
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
                    initial_text = cmd.words.get(index + 2).cloned();
                    index += 3;
                }
                word if word.starts_with("-dei") && word.len() > 4 => {
                    delimiter = read_delimiter_char(&word[3..]);
                    initial_text = Some(word[4..].to_string());
                    index += 1;
                }
                "-ed" => {
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                word if word.starts_with("-ed") && word.len() > 3 => {
                    delimiter = read_delimiter_char(&word[3..]);
                    index += 1;
                }
                "-red" | "-erd" => {
                    raw = true;
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                word if (word.starts_with("-red") || word.starts_with("-erd"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    delimiter = read_delimiter_char(&word[4..]);
                    index += 1;
                }
                "-st" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -st: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                word if word.starts_with("-st") && word.len() > 3 => {
                    let value = &word[3..];
                    match parse_read_timeout(value) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 1;
                }
                "-su" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -su: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                word if word.starts_with("-su") && word.len() > 3 => {
                    let value = &word[3..];
                    read_fd = match parse_read_fd(value) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 1;
                }
                "-sd" => {
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                word if word.starts_with("-sd") && word.len() > 3 => {
                    delimiter = read_delimiter_char(&word[3..]);
                    index += 1;
                }
                word if word.starts_with('-')
                    && word.len() > 2
                    && word[1..]
                        .chars()
                        .all(|ch| matches!(ch, 'e' | 'E' | 'r' | 's')) =>
                {
                    raw |= word[1..].contains('r');
                    index += 1;
                }
                word if word.starts_with("-d") && word.len() > 2 => {
                    delimiter = read_delimiter_char(&word[2..]);
                    index += 1;
                }
                "-rd" => {
                    raw = true;
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                word if word.starts_with("-rd") && word.len() > 3 => {
                    raw = true;
                    delimiter = read_delimiter_char(&word[3..]);
                    index += 1;
                }
                "-rsd" | "-srd" => {
                    raw = true;
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                word if (word.starts_with("-rsd") || word.starts_with("-srd"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    delimiter = read_delimiter_char(&word[4..]);
                    index += 1;
                }
                "-ersd" | "-esrd" | "-resd" | "-rsed" | "-serd" | "-sred" => {
                    raw = true;
                    let Some(delimiter_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'd');
                    };
                    delimiter = read_delimiter_char(delimiter_word);
                    index += 2;
                }
                word if (word.starts_with("-ersd")
                    || word.starts_with("-esrd")
                    || word.starts_with("-resd")
                    || word.starts_with("-rsed")
                    || word.starts_with("-serd")
                    || word.starts_with("-sred"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    delimiter = read_delimiter_char(&word[5..]);
                    index += 1;
                }
                "-rsn" | "-srn" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                word if (word.starts_with("-rsn") || word.starts_with("-srn"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let value = &word[4..];
                    char_limit = match read_char_limit_argument(Some(value)) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                "-ersn" | "-esrn" | "-resn" | "-rsen" | "-sern" | "-sren" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                word if (word.starts_with("-ersn")
                    || word.starts_with("-esrn")
                    || word.starts_with("-resn")
                    || word.starts_with("-rsen")
                    || word.starts_with("-sern")
                    || word.starts_with("-sren"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    let value = &word[5..];
                    char_limit = match read_char_limit_argument(Some(value)) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                "-rsN" | "-srN" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                word if (word.starts_with("-rsN") || word.starts_with("-srN"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let value = &word[4..];
                    char_limit = match read_char_limit_argument(Some(value)) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                "-ersN" | "-esrN" | "-resN" | "-rseN" | "-serN" | "-sreN" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                word if (word.starts_with("-ersN")
                    || word.starts_with("-esrN")
                    || word.starts_with("-resN")
                    || word.starts_with("-rseN")
                    || word.starts_with("-serN")
                    || word.starts_with("-sreN"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    let value = &word[5..];
                    char_limit = match read_char_limit_argument(Some(value)) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                "-rn" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'n');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                word if word.starts_with("-rn") && word.len() > 3 => {
                    raw = true;
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                "-rN" => {
                    raw = true;
                    let Some(limit_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'N');
                    };
                    char_limit = match read_char_limit_argument(Some(limit_word)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                word if word.starts_with("-rN") && word.len() > 3 => {
                    raw = true;
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                "-ru" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -ru: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                word if word.starts_with("-ru") && word.len() > 3 => {
                    raw = true;
                    let value = &word[3..];
                    read_fd = match parse_read_fd(value) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 1;
                }
                "-rsu" | "-sru" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                word if (word.starts_with("-rsu") || word.starts_with("-sru"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let value = &word[4..];
                    read_fd = match parse_read_fd(value) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 1;
                }
                "-ersu" | "-esru" | "-resu" | "-rseu" | "-seru" | "-sreu" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    read_fd = match parse_read_fd(word) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 2;
                }
                word if (word.starts_with("-ersu")
                    || word.starts_with("-esru")
                    || word.starts_with("-resu")
                    || word.starts_with("-rseu")
                    || word.starts_with("-seru")
                    || word.starts_with("-sreu"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    let value = &word[5..];
                    read_fd = match parse_read_fd(value) {
                        Ok(fd) => Some(fd),
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid file descriptor specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    index += 1;
                }
                "-rp" => {
                    raw = true;
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                word if word.starts_with("-rp") && word.len() > 3 => {
                    raw = true;
                    prompt = Some(word[3..].to_string());
                    index += 1;
                }
                "-rsp" | "-srp" => {
                    raw = true;
                    let Some(next_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'p');
                    };
                    prompt = Some(next_word.clone());
                    index += 2;
                }
                word if (word.starts_with("-rsp") || word.starts_with("-srp"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    prompt = Some(word[4..].to_string());
                    index += 1;
                }
                "-ri" => {
                    raw = true;
                    let Some(initial_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(initial_word.clone());
                    index += 2;
                }
                word if word.starts_with("-ri") && word.len() > 3 => {
                    raw = true;
                    index += 1;
                }
                "-rsi" | "-sri" => {
                    raw = true;
                    let Some(initial_word) = cmd.words.get(index + 1) else {
                        return self.read_missing_argument(cmd, &mut stderr, 'i');
                    };
                    initial_text = Some(initial_word.clone());
                    index += 2;
                }
                word if (word.starts_with("-rsi") || word.starts_with("-sri"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    index += 1;
                }
                "-rst" | "-srt" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                word if (word.starts_with("-rst") || word.starts_with("-srt"))
                    && word.len() > 4 =>
                {
                    raw = true;
                    let value = &word[4..];
                    match parse_read_timeout(value) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 1;
                }
                "-erst" | "-esrt" | "-rest" | "-rset" | "-sert" | "-sret" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let option = &cmd.words[index][1..];
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -{option}: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                word if (word.starts_with("-erst")
                    || word.starts_with("-esrt")
                    || word.starts_with("-rest")
                    || word.starts_with("-rset")
                    || word.starts_with("-sert")
                    || word.starts_with("-sret"))
                    && word.len() > 5 =>
                {
                    raw = true;
                    let value = &word[5..];
                    match parse_read_timeout(value) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 1;
                }
                "-rt" => {
                    raw = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        let _ = writeln!(
                            &mut stderr,
                            "{}read: -rt: option requires an argument",
                            self.diagnostic_prefix()
                        );
                        let _ = writeln!(&mut stderr, "{READ_USAGE}");
                        return self.finish_read_error(cmd, &stderr, 2);
                    };
                    match parse_read_timeout(word) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 2;
                }
                word if word.starts_with("-rt") && word.len() > 3 => {
                    raw = true;
                    let value = &word[3..];
                    match parse_read_timeout(value) {
                        Ok(secs) => {
                            timeout_zero = secs == 0.0;
                            timeout_secs = Some(secs);
                        }
                        Err(()) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid timeout specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    index += 1;
                }
                word if word.starts_with("-n") && word.len() > 2 => {
                    char_limit = match read_char_limit_argument(Some(&word[2..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                word if word.starts_with("-N") && word.len() > 2 => {
                    char_limit = match read_char_limit_argument(Some(&word[2..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                word if word.starts_with('-')
                    && matches!(
                        word.as_bytes().get(1).copied(),
                        Some(b'i' | b'n' | b'N' | b't' | b'u')
                    )
                    && word.len() > 2 =>
                {
                    if let Some(value) = word.strip_prefix("-i") {
                        initial_text = Some(value.to_string());
                    } else if let Some(value) = word.strip_prefix("-N") {
                        if let Ok(limit) = value.parse::<usize>() {
                            char_limit = Some(limit);
                            exact_char_limit = true;
                        } else {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid character count specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    } else if let Some(value) = word.strip_prefix("-n") {
                        if let Ok(limit) = value.parse::<usize>() {
                            char_limit = Some(limit);
                        } else {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid character count specification",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    } else if let Some(value) = word.strip_prefix("-u") {
                        read_fd = match parse_read_fd(value) {
                            Ok(fd) => Some(fd),
                            Err(()) => {
                                let _ = writeln!(
                                    &mut stderr,
                                    "{}read: {value}: invalid file descriptor specification",
                                    self.diagnostic_prefix()
                                );
                                return self.finish_read_error(cmd, &stderr, 1);
                            }
                        };
                    } else if let Some(value) = word.strip_prefix("-t") {
                        match parse_read_timeout(value) {
                            Ok(secs) => {
                                timeout_zero = secs == 0.0;
                                timeout_secs = Some(secs);
                            }
                            Err(()) => {
                                let _ = writeln!(
                                    &mut stderr,
                                    "{}read: {value}: invalid timeout specification",
                                    self.diagnostic_prefix()
                                );
                                return self.finish_read_error(cmd, &stderr, 1);
                            }
                        }
                    }
                    index += 1;
                }
                word if word.starts_with("-p") && word.len() > 2 => {
                    index += 1;
                }
                word if word.starts_with('-') && word.len() > 1 => {
                    let option = first_invalid_read_option(word).unwrap_or('?');
                    let _ = writeln!(
                        &mut stderr,
                        "{}read: -{option}: invalid option",
                        self.diagnostic_prefix()
                    );
                    let _ = writeln!(&mut stderr, "{READ_USAGE}");
                    return self.finish_read_error(cmd, &stderr, 2);
                }
                word if !stop_scalar_names => {
                    seen_plain_name = true;
                    if is_valid_read_name(
                        word,
                        self.word_is_arrayref(cmd, index),
                        &self.shell_state.env_vars,
                    ) {
                        scalar_names.push(
                            crate::builtins::arrayref::take_arrayref_flag(word)
                                .1
                                .to_string(),
                        );

                        scalar_field_count += 1;
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            word,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
                        stop_scalar_names = true;
                    }
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        // GNU read.def, as measured against bash 5.3.20: the FIRST
        // plain name is validated before any input is read — an
        // invalid first plain name fails the builtin with the record
        // unread and nothing assigned. Later plain names are validated
        // during assignment: the record is read and earlier names keep
        // their values. The -a argument is validated last of all: its
        // diagnostic is deferred behind the plain names (rubash#1301),
        // and when it fails the record is still read but nothing is
        // assigned (the fail_after_read path below: the name lists are
        // emptied and the scalar read runs for consumption only).
        // k6's #1301 aborted before the read for EVERY invalid name,
        // which lost the earlier assignments GNU keeps (`read ok 1bad
        // later` left ok unset) and skipped the record GNU consumes
        // for an invalid -a argument.
        if invalid_name && scalar_names.is_empty() && !timeout_zero {
            return self.finish_read_error(cmd, &stderr, 1);
        }
        let fail_after_read = invalid_array_name.is_some();
        if fail_after_read {
            if !invalid_name {
                let bad = invalid_array_name.as_deref().unwrap_or_default();
                report_read_invalid_identifier(&mut stderr, &self.diagnostic_prefix(), bad);
            }
            scalar_names.clear();
            scalar_field_count = 0;
        }

        // rubash#1300: without -r and with a non-newline delimiter, a
        // backslash-escaped delimiter is record data, not the record
        // terminator (GNU read.def zread escape consumption). The
        // newline delimiter is exempt: \<newline> is the continuation
        // case continue_read_line_after_backslash owns.
        let escape_aware = !raw && delimiter != '\n';

        // GNU read.def:331-336: `-i text` only fills the readline edit
        // buffer — `itext = list_optarg` is consumed exclusively by the
        // edit path (-e/-E set `edit`, read.def:316-327), which itself only
        // engages on an interactive terminal. Without a real readline
        // editor the initial text never reaches the variable
        // (`printf '\n' | { read -i init v; }` leaves v empty). Rubash's
        // read builtin has no readline editor, so the option parses and
        // stays inert (rubash#270).
        initial_text = None;

        // Bash displays read -p prompts only for interactive stdin.
        // A -t 0 read returns before the prompt would print (read.def
        // early return), so it is suppressed there too.
        if prompt.is_some() && !timeout_zero && self.read_prompt_should_display(cmd, read_fd) {
            let expanded = self.expand_word(prompt.as_deref().unwrap_or_default());
            eprint!("{}", expanded);
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }

        if let Some(fd) = read_fd {
            if !self.read_fd_is_available(cmd, fd) {
                // GNU read.def:378-380: sh_validfd failing means the
                // descriptor is not open at all — "invalid file
                // descriptor". A descriptor that IS open but not
                // readable (write-only) passes that check and fails in
                // the read itself — read.def:911 "read error"
                // (rubash#1592).
                let wording = if self.fd_table.is_open(fd) {
                    "read error"
                } else {
                    "invalid file descriptor"
                };
                let _ = writeln!(
                    &mut stderr,
                    "{}read: {fd}: {wording}: Bad file descriptor",
                    self.diagnostic_prefix()
                );
                return self.finish_read_error(cmd, &stderr, 1);
            }
        }

        if read_fd.is_none() {
            if let Some(fd) = redirected_input_fd(self, cmd) {
                if !self.read_fd_is_available(cmd, fd) {
                    let _ = writeln!(
                        &mut stderr,
                        "{}{fd}: Bad file descriptor",
                        self.diagnostic_prefix()
                    );
                    return self.finish_read_error(cmd, &stderr, 1);
                }
            }
        }

        // GNU builtins/common.c:988 builtin_find_indexed_array ->
        // arrayfunc.c:454 find_or_make_array_variable: `read -a` resolves a
        // nameref operand through find_variable_last_nameref — an empty cell
        // is invisible so the attribute is stripped and the name itself
        // becomes the array, a valid bare-name cell redirects the bind to
        // the target, and an element/invalid cell fails sh_invalidid
        // (nameref18.sub `read -a ref` where ref -> `XXX[0]`).
        if let Some(name) = array_name.clone() {
            if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, &name) {
                match self.nameref_resolution(&name) {
                    NamerefResolution::Target(target) => {
                        if parse_array_subscript(&target).is_some() {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: `{target}': not a valid identifier",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                        array_name = Some(target);
                    }
                    NamerefResolution::Unresolved => {
                        let cell = self
                            .shell_state
                            .env_vars
                            .get(&name)
                            .cloned()
                            .unwrap_or_default();
                        if cell.is_empty() {
                            let _ = writeln!(
                                &mut stderr,
                                "{}warning: {name}: removing nameref attribute",
                                self.diagnostic_prefix()
                            );
                            unmark_env_name(&mut self.shell_state.env_vars, NAMEREF_VARS, &name);
                            // The buffered stderr is only emitted on the
                            // error path; GNU writes the warning at once, so
                            // flush it before continuing to a successful read.
                            let _ = self.write_buffered_builtin_output(cmd, &[], &stderr);
                            stderr.clear();
                        } else {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: `{cell}': not a valid identifier",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    }
                    // Circular/over-depth chains: find_variable_last_nameref
                    // returns NULL and the array binds to the name itself.
                    NamerefResolution::Circular
                    | NamerefResolution::MaxDepth
                    | NamerefResolution::NotNameref => {}
                }
            }
        }

        // read.def -s: silent input. The combined-option arms above consume
        // the letters without tracking them, so derive the flag from the
        // option words themselves — every form carrying `s` (-s, -sn4,
        // -rsa, -ersa, ...) arms it, and a `--`-escaped name never can.
        let silent = cmd
            .words
            .iter()
            .skip(1)
            .take_while(|word| word.as_str() != "--")
            .any(|word| word.starts_with('-') && word.contains('s'));

        if timeout_zero {
            // GNU builtins/read.def:395-398: a `-t 0` read returns
            // input_avail(fd) ? 0 : 1 immediately after option parsing —
            // it consumes nothing and assigns NOTHING (scalars keep
            // their values, a -a array is not replaced, REPLY is
            // untouched), and neither the read targets' name validity
            // nor the deferred -a failure is ever consulted. Only the
            // -u descriptor itself was validated during option parsing,
            // before this early return.
            return self.read_timeout_zero_status(cmd, read_fd);
        }

        if let Some(name) = array_name {
            // GNU builtins/common.c:1003-1006: `read -a` on a variable that
            // is not an indexed array (e.g., an associative array) reports
            // `read: A: not an indexed array` and does not read into it
            // (array33.sub:52 `read -a A` on `declare -A A`).
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &name) {
                let _ = writeln!(
                    &mut stderr,
                    "{}read: {name}: not an indexed array",
                    self.diagnostic_prefix()
                );
                // GNU read.def returns EXECUTION_FAILURE (1) here, not the
                // usage-error status 2 (rubash#1298).
                return self.finish_read_error(cmd, &stderr, 1);
            }
            if char_limit == Some(0) {
                // GNU read.def assign_vars runs even for -n 0 (zero
                // characters read, then the array is located and the
                // assignment attempted), so a readonly -a target fails
                // here too (rubash#1457; see the post-read guard below).
                if !invalid_name && is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &name)
                {
                    let _ = writeln!(
                        &mut stderr,
                        "{}{name}: readonly variable",
                        self.diagnostic_prefix()
                    );
                    return self.finish_read_error(cmd, &stderr, 1);
                }
                if !invalid_name {
                    let _ = replace_typed_indexed_array(
                        &mut self.shell_state.variables,
                        &name,
                        std::iter::empty::<String>(),
                    );
                    self.shell_state
                        .env_vars
                        .insert(name.clone(), read_array_storage(&[]));
                    mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
                }
                return if invalid_name {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    0
                };
            }

            self.arm_read_deadline(cmd, read_fd, timeout_secs);
            let line = match self.read_input_for_command(
                cmd,
                read_fd,
                silent,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ) {
                Some(l) if l.is_empty() && initial_text.is_some() => None,
                Some(l) => Some(l),
                None => None,
            };
            let read_timed_out = self.take_read_timed_out();
            // rubash#1250: an untrapped SIGINT aborted the acquisition —
            // GNU assigns nothing and the builtin returns 128+SIGINT.
            if let Some(signal) = self.take_read_interrupted_signal() {
                return if invalid_name {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    // rubash#1639 (read arm): an interrupted read is an
                    // untrapped-SIGINT foreground wait — the flag is
                    // only ever set under the armed guard (interactive
                    // + untrapped) — so the rest of the line is
                    // discarded like the sleep/external waits. Shape
                    // ported from PR #1702 (a-rookie-of-C-language).
                    self.fg_sigint_unwind = true;
                    128 + signal
                };
            }
            let read_eof = self.take_read_eof_no_delimiter();
            // GNU read.def assign_vars (post-read): the -a target is
            // located through builtins/common.c:988
            // builtin_find_indexed_array -> arrayfunc.c:454
            // find_or_make_array_variable, whose assignment flag makes a
            // readonly target fail with err_readonly AFTER the input has
            // been consumed; the array keeps its old value and the
            // builtin returns EXECUTION_FAILURE. The scalar path already
            // refuses through the assignment machinery. This guard covers
            // the EOF-without-line store and the main store below; the
            // -t 0 branch above is deliberately NOT covered
            // (read.def:395-398 returns the input-availability result
            // before the array is ever located, so `read -t 0 -a
            // readonly` stays silent) and -n 0 has its own guard at its
            // early return (rubash#1457).
            if !invalid_name && is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &name) {
                let _ = writeln!(
                    &mut stderr,
                    "{}{name}: readonly variable",
                    self.diagnostic_prefix()
                );
                return self.finish_read_error(cmd, &stderr, 1);
            }
            let final_line = match (&line, &initial_text) {
                (Some(line), _) => line.clone(),
                (None, Some(text)) => text.clone(),
                (None, None) => {
                    if !invalid_name {
                        let _ = replace_typed_indexed_array(
                            &mut self.shell_state.variables,
                            &name,
                            std::iter::empty::<String>(),
                        );
                        self.shell_state
                            .env_vars
                            .insert(name.clone(), read_array_storage(&[]));
                        mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
                    }
                    return if read_timed_out { 142 } else { 1 };
                }
            };
            let values = if raw {
                split_read_array_words(
                    &final_line,
                    self.shell_state.env_vars.get("IFS").map(String::as_str),
                )
            } else {
                split_read_array_words_with_backslashes(
                    &final_line,
                    self.shell_state.env_vars.get("IFS").map(String::as_str),
                )
            };
            // An invalid plain name anywhere in the list keeps the
            // array unassigned too (GNU validates every name before
            // the array store; see the post-scan validation comment).
            if !invalid_name {
                let value = read_array_storage(&values);
                let _ = replace_typed_indexed_array(&mut self.shell_state.variables, &name, values);
                self.shell_state.env_vars.insert(name.clone(), value);
                mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
            }
            return if invalid_name {
                self.finish_read_error(cmd, &stderr, 1)
            } else if read_timed_out {
                142
            } else if read_eof {
                // read.def:949: partial record at EOF still assigns but
                // fails the builtin.
                1
            } else {
                0
            };
        }

        let scalar_names = if fail_after_read {
            // The record is still read below (for consumption, as GNU
            // does), but the emptied name list assigns nothing.
            Vec::new()
        } else if scalar_names.is_empty() {
            if invalid_name {
                Vec::new()
            } else {
                scalar_field_count = 0;
                vec!["REPLY".to_string()]
            }
        } else {
            scalar_names
        };
        if !scalar_names.is_empty() || fail_after_read {
            if char_limit == Some(0) {
                let assign_status = self.assign_read_scalar_names(&scalar_names, "", raw);
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                return if invalid_name || fail_after_read {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    0
                };
            }

            self.arm_read_deadline(cmd, read_fd, timeout_secs);
            let status = if let Some(line) = self.read_input_for_command(
                cmd,
                read_fd,
                silent,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ) {
                // GNU read.def: input is a C string, truncate at first NUL
                // (rubash#1419). The `-d ''` case stops at NUL (delimiter),
                // so only truncate when delimiter is not NUL.
                let line = if delimiter != '\0' {
                    match line.find('\0') {
                        Some(pos) => line[..pos].to_string(),
                        None => line,
                    }
                } else {
                    line
                };
                let line = if !raw && delimiter == '\n' && char_limit.is_none() && !exact_char_limit
                {
                    self.continue_read_line_after_backslash(cmd, read_fd, silent, line)
                } else {
                    line
                };
                let assign_status = if exact_char_limit {
                    // rubash#776: `-N` binds the record whole (see
                    // assign_read_exact_names); `-n` keeps IFS splitting.
                    self.assign_read_exact_names(&scalar_names, &line, raw)
                } else {
                    self.assign_read_scalar_names_with_field_count(
                        &scalar_names,
                        &line,
                        raw,
                        scalar_field_count,
                    )
                };
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                // read.def:949: eof (input exhausted before the delimiter)
                // still assigns the partial record but returns failure.
                if self.take_read_eof_no_delimiter() {
                    1
                } else {
                    0
                }
            } else if self.read_interrupted_signal.is_some() {
                // rubash#1250: an untrapped SIGINT aborted the
                // acquisition — GNU assigns NOTHING and returns
                // 128+SIGINT; the final status mapping below produces
                // it from the flag. Do not fall through to the EOF /
                // timeout / fallback arms (the fallback would block
                // on a second raw read).
                // rubash#1639 (read arm): the same untrapped-SIGINT
                // foreground-wait discard as the sleep/external waits
                // (ported from PR #1702's shape set).
                self.fg_sigint_unwind = true;
                130
            } else if self.fd_table.read_error.is_some() {
                // GNU read.def:911: a hard read(2) failure (EISDIR when
                // the input descriptor names a directory) is reported as
                // `read: <fd>: read error: <strerror>` and the builtin
                // fails — it is not plain EOF (rubash#1495). The fd-table
                // reader recorded the strerror text; the closed-fd arm
                // below is the EBADF special case of the same shape.
                let message = self.fd_table.read_error.take().unwrap_or_default();
                let fd = read_fd.unwrap_or(0);
                let assign_status = self.assign_read_scalar_names(
                    &scalar_names,
                    initial_text.as_deref().unwrap_or(""),
                    raw,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                let _ = writeln!(
                    &mut stderr,
                    "{}read: {fd}: read error: {message}",
                    self.diagnostic_prefix()
                );
                if initial_text.is_none() {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    0
                }
            } else if command_closes_stdin(cmd) || self.fd_table.is_closed(0) {
                // GNU builtins/read.def: an OS-level read error (EBADF
                // here) is not an EOF — read.def:911 reports it and the
                // builtin fails WITHOUT binding the names; only a real
                // EOF binds the (empty/partial) record (rubash#1477:
                // `read x <&-` left x set-but-empty). The -i prefill arm
                // keeps binding the initial text (readline behavior).
                let assign_status = if initial_text.is_some() {
                    self.assign_read_scalar_names(
                        &scalar_names,
                        initial_text.as_deref().unwrap_or(""),
                        raw,
                    )
                } else {
                    0
                };
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                // GNU read.def:911: builtin_error ("%d: %s: %s", fd,
                // "read error", strerror(errno)) — the fd number comes
                // before the "read error" tag (probe 2026-09-27:
                // `read value <&-` → `bash: line 1: read: 0: read error:
                // Bad file descriptor`).
                let _ = writeln!(
                    &mut stderr,
                    "{}read: 0: read error: Bad file descriptor",
                    self.diagnostic_prefix()
                );
                if initial_text.is_none() {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    0
                }
            } else if read_fd.is_some() || command_redirects_stdin(cmd) {
                let assign_status = self.assign_read_scalar_names(
                    &scalar_names,
                    initial_text.as_deref().unwrap_or(""),
                    raw,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                if initial_text.is_none() {
                    1
                } else {
                    0
                }
            } else if self.shell_state.env_vars.contains_key(FUNCTION_STDIN) {
                let assign_status = self.assign_read_scalar_names(
                    &scalar_names,
                    initial_text.as_deref().unwrap_or(""),
                    raw,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                if initial_text.is_none() {
                    1
                } else {
                    0
                }
            } else if self.read_timed_out || self.fd_table.read_timed_out {
                // GNU read.def:539-562: a timeout still assigns the partial
                // input (empty here) and returns 128+SIGALRM — do not fall
                // through to another blocking read.
                let assign_status = self.assign_read_scalar_names(
                    &scalar_names,
                    initial_text.as_deref().unwrap_or(""),
                    raw,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                142
            } else if self.fd_table.fd0_bound_away_from_process_stdin() {
                // The fd table owns fd 0 as a real endpoint (the enclosing
                // compound's `done < file` binding or an `exec 0<` fd), and
                // every layer of read_input_for_command is exhausted: that
                // is EOF on THAT descriptor. GNU read has no other input —
                // read.def:762-768 sets eof and breaks, :949 returns
                // EXECUTION_FAILURE — so read_stdin_until (the raw process
                // stdin) must not run here: it would block forever on the
                // parent's still-open stdin pipe (readhang regression:
                // `while read -r f; do :; done < list.txt` under a piped
                // stdin never finished the loop).
                let assign_status = self.assign_read_scalar_names(
                    &scalar_names,
                    initial_text.as_deref().unwrap_or(""),
                    raw,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                if initial_text.is_none() {
                    1
                } else {
                    0
                }
            } else {
                // rubash#1250: hold the foreground SIGINT guard across
                // the raw fallback read as well (see read_input_for_
                // command); its loops poll for the abort in slices.
                #[cfg(unix)]
                let _sigint_guard = super::external_inner::ForegroundBuiltinSigintGuard::hold_if_interactive_untrapped(self);
                let poll_sigint = {
                    #[cfg(unix)]
                    {
                        _sigint_guard.is_armed()
                    }
                    #[cfg(not(unix))]
                    {
                        false
                    }
                };
                match read_stdin_until(
                    self.read_deadline,
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    poll_sigint,
                    escape_aware,
                    raw,
                ) {
                    Ok((_, _, _, _, true)) => {
                        // Interrupted (rubash#1250): assign nothing; the
                        // final status mapping produces 128+SIGINT.
                        #[cfg(unix)]
                        {
                            self.read_interrupted_signal = Some(libc::SIGINT);
                        }
                        #[cfg(not(unix))]
                        {
                            self.read_interrupted_signal = Some(2);
                        }
                        130
                    }
                    Ok((0, _, false, false, _)) => {
                        let assign_status = self.assign_read_scalar_names(
                            &scalar_names,
                            initial_text.as_deref().unwrap_or(""),
                            raw,
                        );
                        if assign_status != 0 {
                            return self.finish_read_error(cmd, &stderr, assign_status);
                        }
                        if initial_text.is_none() {
                            1
                        } else {
                            0
                        }
                    }
                    Ok((_, line, stdin_eof, timed_out, _)) => {
                        // read.def:553: the deadline expired — the partial
                        // record is still assigned and the status becomes
                        // 128+SIGALRM via take_read_timed_out below.
                        self.read_timed_out |= timed_out;
                        // rubash#1419: this last-resort transport bypasses
                        // read_input_for_command, so apply its NUL skip
                        // here too (read.def discards NUL bytes as the
                        // record is read).
                        let line = if line.contains('\0') {
                            line.replace('\0', "")
                        } else {
                            line
                        };
                        self.read_eof_no_delimiter |= stdin_eof;
                        let line = if !raw
                            && delimiter == '\n'
                            && char_limit.is_none()
                            && !exact_char_limit
                        {
                            self.continue_read_line_after_backslash(cmd, read_fd, silent, line)
                        } else {
                            line
                        };
                        let assign_status =
                            self.assign_read_scalar_names(&scalar_names, &line, raw);
                        if assign_status != 0 {
                            return self.finish_read_error(cmd, &stderr, assign_status);
                        }
                        // read.def:949: input exhausted before the delimiter
                        // still assigns the partial record but fails.
                        if self.take_read_eof_no_delimiter() {
                            1
                        } else {
                            0
                        }
                    }
                    Err(_) => 1,
                }
            };
            // GNU read.def: an interrupted read returns 128+SIGINT and
            // assigns nothing (rubash#1250); on timeout the vars keep
            // the partial assignment above and the builtin returns
            // 128+SIGALRM=142 (read.def:539-562).
            let mut interrupt_signal = None;
            let status = if let Some(signal) = self.take_read_interrupted_signal() {
                interrupt_signal = Some(signal);
                128 + signal
            } else if self.take_read_timed_out() {
                142
            } else {
                status
            };
            return if invalid_name || fail_after_read {
                self.finish_read_error(cmd, &stderr, 1)
            } else {
                // rubash#1639 (read arm): arm the line discard only
                // when the interrupted status is the one actually
                // returned (the error arms above return 1 instead).
                // Ported from PR #1702 (a-rookie-of-C-language).
                if interrupt_signal.is_some() {
                    self.fg_sigint_unwind = true;
                }
                status
            };
        }
        if invalid_name || fail_after_read {
            return self.finish_read_error(cmd, &stderr, 1);
        }
        let _ = writeln!(
            &mut stderr,
            "{}read: command not found",
            self.diagnostic_prefix()
        );
        self.finish_read_error(cmd, &stderr, 127)
    }

    /// GNU builtins/read.def: an argument-taking option letter appearing
    /// last in its word with no following word is a usage error —
    /// `read: -X: option requires an argument` plus the usage line,
    /// status 2 (the shape the -a/-t/-u arms already emit). rubash#1470:
    /// the -d/-n/-N/-i/-p arms used to swallow the missing argument
    /// silently (delimiter defaulted to NUL, char limit and prompt and
    /// initial text to none) and start reading — a trailing `read -n` in
    /// a script hung on stdin instead of failing.
    fn read_missing_argument(
        &mut self,
        cmd: &CommandNode,
        stderr: &mut Vec<u8>,
        letter: char,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}read: -{letter}: option requires an argument",
            self.diagnostic_prefix()
        );
        let _ = writeln!(stderr, "{READ_USAGE}");
        self.finish_read_error(cmd, stderr, 2)
    }

    fn read_prompt_should_display(&self, cmd: &CommandNode, read_fd: Option<u32>) -> bool {
        if self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
            || command_redirects_stdin(cmd)
            || command_closes_stdin(cmd)
        {
            return false;
        }

        // GNU builtins/read.def:476-491: the -p prompt is printed iff
        // the descriptor `read` reads from is a terminal — the shell's
        // own interactivity is irrelevant (pty-verified GNU 5.3.20: a
        // script FILE whose stdin is a tty shows the prompt; rubash
        // #1255). The old blanket ban on script runs
        // (__RUBASH_SCRIPT_NAME) inverted that rule. Probe the bound
        // endpoint the way redirect_target_is_terminal does.
        let fd = read_fd.unwrap_or(0);
        match self.fd_table.read_endpoint(fd) {
            Some(FdReadEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
            Some(FdReadEndpoint::InheritedProcessStdin) => {
                crate::fd::is_console_handle(crate::fd::process_std_handle(0))
            }
            Some(_) => false,
            None => crate::fd::is_console_handle(fd as crate::fd::HANDLE),
        }
    }

    fn read_fd_is_available(&self, cmd: &CommandNode, fd: u32) -> bool {
        if self.coproc_read_file(fd).is_some() || (fd == 0 && self.first_coproc_read().is_some()) {
            return true;
        }
        // A move redirection `<&N-`/`>&N-` closed fd N for this command's
        // duration (make_cmd.c:704-718 → redir.c:1159-1166 close), so
        // `read -u 5 x <&5-` sees fd 5 invalid exactly like GNU's
        // read.def:377-381 sh_validfd check (verified vs WSL GNU 5.3.0).
        // The close lives inside the `redir_fd != redirector` guard
        // (redir.c:1128): `read x <&0-` is a complete no-op and leaves the
        // descriptor open (rubash#938).
        if let Some(redirect) = cmd.redirect_in.as_deref() {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd_and_move(&target) == Some((fd, true))
                && redirect.fd.unwrap_or(0) != fd
            {
                return false;
            }
        }
        // fd 0 is a descriptor whenever the table recorded no explicit
        // close — the same F_GETFD rule sh_validfd applies (and the same
        // stdio-always-open rule validate_builtin_dup_source uses). A
        // pipeline stage's fd 0 can carry no fd-table endpoint yet still
        // be a perfectly valid read target on the transports below.
        if fd == 0 {
            return !self.fd_table.is_closed(0);
        }
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
        if cmd.redirect_in.as_deref().is_some_and(|redirect| {
            redirect.fd == Some(fd)
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        }) {
            return true;
        }
        // rubash#1296: a numbered input redirect attached to the read
        // command itself (`read -u 3 x 3< file`) lives in cmd.redirects,
        // not redirect_in — the same dual-store lookup mapfile performs.
        cmd.redirects.iter().rev().any(|redirect| {
            redirect.fd == Some(fd)
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
                )
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        })
    }

    fn read_timeout_zero_status(&mut self, cmd: &CommandNode, read_fd: Option<u32>) -> i32 {
        // GNU builtins/read.def:395-398 + lib/sh/input_avail.c: the
        // status is input_avail(fd) on the very descriptor the read
        // would use — a zero-timeout select(). EOF, HUP and error
        // conditions all report readable, exactly as poll() reports
        // them Ready; a live pipe or terminal with nothing ready is
        // the only failure. `-u 0` polls fd 0 itself.
        let fd = read_fd.unwrap_or(0);
        if self.fd_table.is_closed(fd) || (fd == 0 && command_closes_stdin(cmd)) {
            return 1;
        }
        // Materialized command-attached inputs carry their bytes
        // in-process (heredoc body, herestring/stdin string, function
        // stdin): the descriptor GNU would poll already holds them.
        if cmd
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.fd.unwrap_or(0) == fd && redirect.body.is_some())
        {
            return 0;
        }
        if fd == 0
            && (self.stdin_string_for_command_mut(cmd).is_some()
                || self.shell_state.env_vars.contains_key(FUNCTION_STDIN))
        {
            return 0;
        }
        if let Some(status) = self.read_timeout_zero_poll(fd) {
            return status;
        }
        // Command-attached redirects that never entered the fd table
        // (rubash#1296 dual-store): an opened source is readable, a
        // `<&-` target is not.
        if cmd.redirect_in.as_deref().is_some_and(|redirect| {
            redirect.fd.unwrap_or(0) == fd
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        }) {
            return 0;
        }
        if cmd.redirects.iter().rev().any(|redirect| {
            redirect.fd == Some(fd)
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
                )
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        }) {
            return 0;
        }
        if fd == 0 {
            // No table entry for fd 0: the descriptor is the process
            // stdin itself.
            return read_timeout_zero_poll_handle(crate::fd::process_std_handle(0));
        }
        1
    }

    /// The select()/poll() half of read_timeout_zero_status. Endpoints
    /// backed by a kernel object are polled for real: a live pipe with
    /// no data is NOT available even though the fd is open, and a
    /// terminal reports ready only when the line discipline holds a
    /// complete line, so pending typeahead is neither consumed nor
    /// misjudged. Text endpoints are materialized — always ready.
    fn read_timeout_zero_poll(&self, fd: u32) -> Option<i32> {
        match self.fd_table.read_endpoint(fd) {
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => Some(0),
            Some(FdReadEndpoint::File(file)) => Some(read_timeout_zero_poll_handle(file.handle)),
            Some(FdReadEndpoint::CoprocStdout { fd, .. }) => {
                Some(read_timeout_zero_poll_handle(fd.handle))
            }
            Some(FdReadEndpoint::InheritedProcessStdin) => Some(read_timeout_zero_poll_handle(
                crate::fd::process_std_handle(0),
            )),
            None => None,
        }
    }

    /// GNU read.def:443-458 + 499-519: arm the absolute read deadline for
    /// `-t secs` (or TMOUT when -t is absent). A regular-file input fd
    /// disables the timeout outright (fstat S_ISREG → tmsec = tmusec = 0).
    /// Buffered sources (heredoc, herestring, text endpoints) keep the
    /// deadline armed: GNU 5.3 sources them from a pipe, so
    /// check_read_timeout still fires if the deadline already elapsed.
    fn arm_read_deadline(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        timeout_secs: Option<f64>,
    ) {
        let secs = timeout_secs.or_else(|| {
            self.shell_state
                .env_vars
                .get("TMOUT")
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|t| t.is_finite() && *t >= 0.0)
        });
        let deadline = secs
            .filter(|s| *s > 0.0)
            .filter(|_| !self.read_input_is_regular_file(cmd, read_fd))
            .map(|s| std::time::Instant::now() + std::time::Duration::from_secs_f64(s));
        self.read_deadline = deadline;
        self.read_timed_out = false;
        self.read_interrupted_signal = None;
        self.read_sigint_guard_armed = false;
        self.fd_table.read_deadline = deadline;
        self.fd_table.read_timed_out = false;
    }

    /// Merge and clear the rubash#1250 interrupt flag: an untrapped
    /// SIGINT consumed by a blocking read's slice poll. The caller maps
    /// it to 128+signal and assigns nothing (GNU read.def interrupted
    /// read: no `goto assign_vars`).
    fn take_read_interrupted_signal(&mut self) -> Option<i32> {
        self.read_interrupted_signal.take()
    }

    /// Merge the fd-table timeout flag, clear the armed deadline, and report
    /// whether this read hit its timeout (caller maps that to 128+SIGALRM).
    fn take_read_timed_out(&mut self) -> bool {
        let timed_out = self.read_timed_out || self.fd_table.read_timed_out;
        self.read_deadline = None;
        self.read_timed_out = false;
        self.fd_table.read_deadline = None;
        self.fd_table.read_timed_out = false;
        timed_out
    }

    /// Merge and clear the read.def:949 eof flag: the record read exhausted
    /// its input before the delimiter. The caller still assigns the partial
    /// record but returns EXECUTION_FAILURE.
    fn take_read_eof_no_delimiter(&mut self) -> bool {
        let eof = self.read_eof_no_delimiter || self.fd_table.read_eof_no_delimiter;
        self.read_eof_no_delimiter = false;
        self.fd_table.read_eof_no_delimiter = false;
        eof
    }

    /// GNU read.def:499-505: `fstat(fd)` S_ISREG turns the timeout off —
    /// regular files are always readable, so `-t` can never expire on one.
    fn read_input_is_regular_file(&self, cmd: &CommandNode, read_fd: Option<u32>) -> bool {
        let fd = read_fd.unwrap_or(0);
        match self.fd_table.read_endpoint(fd) {
            Some(FdReadEndpoint::File(file)) => return crate::fd::is_disk_file(file.handle),
            Some(FdReadEndpoint::InheritedProcessStdin) => {
                return crate::fd::is_disk_file(crate::fd::process_std_handle(0));
            }
            Some(_) => return false,
            None => {}
        }
        if fd == 0 {
            if let Some(redirect) = &cmd.redirect_in {
                let target = self.expand_redirect_target(redirect);
                if !target.starts_with('&')
                    && !target.starts_with("<(")
                    && !is_closed_redirect_target(&target)
                {
                    // niubash#177: same operand resolution as
                    // open_input_redirect so `-t` sees the same regular
                    // file the read itself opens.
                    let path = self.session_path(crate::executor::path::windows_operand_file_path(
                        shell_path_to_windows(&target, &self.shell_state.env_vars),
                        &self.shell_state.env_vars,
                    ));
                    return std::fs::metadata(&path)
                        .map(|m| m.is_file())
                        .unwrap_or(false);
                }
            }
        }
        false
    }
}

/// One zero-timeout readability poll, GNU input_avail's shape:
/// Ready (data, EOF, HUP, or an error select() would also surface)
/// is success; a genuinely unready descriptor is failure.
fn read_timeout_zero_poll_handle(handle: crate::fd::HANDLE) -> i32 {
    match crate::fd::wait_readable(handle, std::time::Duration::ZERO) {
        crate::fd::ReadWait::Ready => 0,
        crate::fd::ReadWait::Timeout => 1,
    }
}

fn parse_read_fd(value: &str) -> Result<u32, ()> {
    let fd = value.parse::<i32>().map_err(|_| ())?;
    u32::try_from(fd).map_err(|_| ())
}

fn parse_read_timeout(value: &str) -> Result<f64, ()> {
    let timeout = value.parse::<f64>().map_err(|_| ())?;
    if !timeout.is_finite() || timeout < 0.0 {
        return Err(());
    }
    Ok(timeout)
}

fn report_read_invalid_identifier(stderr: &mut Vec<u8>, diagnostic_prefix: &str, name: &str) {
    // GNU sh_invalidid prints the operand's text; W_ARRAYREF's in-band
    // ARRAYREF_FLAG prefix (execute_cmd.c:4366) is a word flag, not text.
    let name = crate::builtins::arrayref::take_arrayref_flag(name).1;
    let _ = writeln!(
        stderr,
        "{diagnostic_prefix}read: `{name}': not a valid identifier"
    );
}

fn first_invalid_read_option(word: &str) -> Option<char> {
    let mut chars = word.chars();
    chars.next()?;
    chars.find(|ch| {
        !matches!(
            ch,
            // GNU read.def:308 optstring "Eersa:d:i:n:p:t:u:N:" (bash 5.3).
            // `-E` disables readline editing (inverse of `-e`); rubash#1407.
            'a' | 'd' | 'e' | 'E' | 'i' | 'n' | 'N' | 'p' | 'r' | 's' | 't' | 'u'
        )
    })
}

fn command_closes_stdin(cmd: &CommandNode) -> bool {
    cmd.redirect_in
        .as_deref()
        .is_some_and(|redirect| redirect.fd.unwrap_or(0) == 0 && redirect.target == "&-")
}

fn command_redirects_stdin(cmd: &CommandNode) -> bool {
    cmd.redirect_in
        .as_deref()
        .is_some_and(|redirect| redirect.fd.unwrap_or(0) == 0)
}

fn redirected_input_fd(executor: &Executor, cmd: &CommandNode) -> Option<u32> {
    let redirect = cmd.redirect_in.as_deref()?;
    if !redirect.operator.contains('&') {
        return None;
    }
    let target = executor.expand_redirect_target(redirect);
    let target = target.strip_prefix('&').unwrap_or(&target);
    (target != "-")
        .then(|| target.parse::<u32>().ok())
        .flatten()
}
