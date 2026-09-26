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
fn is_valid_read_name(name: &str, w_arrayref: bool, env_vars: &HashMap<String, String>) -> bool {
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
        // GNU execute_cmd.c do_redirections applies redirects before the
        // builtin runs: a failed input open aborts the command — read's
        // variables stay unset and the diagnostic is the redirect's
        // (probe: `read a < /nonexist` leaves `a` unset, status 1). The
        // in-reader open treats failure as EOF, which wrongly assigns "".
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 && redirect.fd_var.is_none() {
                let target = self.expand_redirect_target(redirect);
                if !is_closed_redirect_target(&target)
                    && redirect_target_fd(&target).is_none()
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
                        let _ = self.write_default_stderr(&line);
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        scalar_field_count += 1;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
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
                    } else {
                        report_read_invalid_identifier(
                            &mut stderr,
                            &self.diagnostic_prefix(),
                            name,
                        );
                        invalid_name = true;
                        stop_scalar_names = true;
                    }
                    index += 1;
                }
                "-d" => {
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
                    index += 2;
                }
                "-n" => {
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    initial_text = cmd.words.get(index + 1).cloned();
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
                    prompt = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                "-e" => {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    prompt = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-sp") && word.len() > 3 => {
                    prompt = Some(word[3..].to_string());
                    index += 1;
                }
                "-ep" => {
                    prompt = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-ep") && word.len() > 3 => {
                    prompt = Some(word[3..].to_string());
                    index += 1;
                }
                "-rep" | "-erp" => {
                    raw = true;
                    prompt = cmd.words.get(index + 1).cloned();
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
                    prompt = cmd.words.get(index + 1).cloned();
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
                    initial_text = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-ei") && word.len() > 3 => {
                    initial_text = Some(word[3..].to_string());
                    index += 1;
                }
                "-rei" | "-eri" => {
                    raw = true;
                    initial_text = cmd.words.get(index + 1).cloned();
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
                    initial_text = cmd.words.get(index + 1).cloned();
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
                    initial_text = cmd.words.get(index + 1).cloned();
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
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
                    index += 2;
                }
                word if word.starts_with("-ed") && word.len() > 3 => {
                    delimiter = read_delimiter_char(&word[3..]);
                    index += 1;
                }
                "-red" | "-erd" => {
                    raw = true;
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
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
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
                    index += 2;
                }
                word if word.starts_with("-sd") && word.len() > 3 => {
                    delimiter = read_delimiter_char(&word[3..]);
                    index += 1;
                }
                word if word.starts_with('-')
                    && word.len() > 2
                    && word[1..].chars().all(|ch| matches!(ch, 'e' | 'r' | 's')) =>
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
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
                    index += 2;
                }
                word if word.starts_with("-rd") && word.len() > 3 => {
                    raw = true;
                    delimiter = read_delimiter_char(&word[3..]);
                    index += 1;
                }
                "-rsd" | "-srd" => {
                    raw = true;
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
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
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .map(|word| read_delimiter_char(word))
                        .unwrap_or('\u{0}');
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
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
                    prompt = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                word if word.starts_with("-rp") && word.len() > 3 => {
                    raw = true;
                    prompt = Some(word[3..].to_string());
                    index += 1;
                }
                "-rsp" | "-srp" => {
                    raw = true;
                    prompt = cmd.words.get(index + 1).cloned();
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
                    index += 2;
                }
                word if word.starts_with("-ri") && word.len() > 3 => {
                    raw = true;
                    index += 1;
                }
                "-rsi" | "-sri" => {
                    raw = true;
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

        // Bash displays read -p prompts only for interactive stdin.
        if prompt.is_some() && self.read_prompt_should_display(cmd, read_fd) {
            let expanded = self.expand_word(prompt.as_deref().unwrap_or_default());
            eprint!("{}", expanded);
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }

        if let Some(fd) = read_fd {
            if !self.read_fd_is_available(cmd, fd) {
                let _ = writeln!(
                    &mut stderr,
                    "{}read: {fd}: invalid file descriptor: Bad file descriptor",
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

        if timeout_zero {
            let status = self.read_timeout_zero_status(cmd, read_fd);
            if let Some(name) = array_name {
                let _ = self
                    .shell_state
                    .variables
                    .replace_indexed_array(&name, std::iter::empty::<String>());
                self.shell_state
                    .env_vars
                    .insert(name.clone(), read_array_storage(&[]));
                mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
                return if invalid_name {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    status
                };
            }

            let scalar_names = if scalar_names.is_empty() {
                if invalid_name {
                    Vec::new()
                } else {
                    vec!["REPLY".to_string()]
                }
            } else {
                scalar_names.clone()
            };
            if !scalar_names.is_empty() {
                let assign_status = self.assign_read_scalar_names(
                    &scalar_names,
                    initial_text.as_deref().unwrap_or(""),
                    raw,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
            }
            return if invalid_name {
                self.finish_read_error(cmd, &stderr, 1)
            } else {
                status
            };
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
                return self.finish_read_error(cmd, &stderr, 2);
            }
            if char_limit == Some(0) {
                let _ = self
                    .shell_state
                    .variables
                    .replace_indexed_array(&name, std::iter::empty::<String>());
                self.shell_state
                    .env_vars
                    .insert(name.clone(), read_array_storage(&[]));
                mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
                return 0;
            }

            self.arm_read_deadline(cmd, read_fd, timeout_secs);
            let line = match self.read_input_for_command(
                cmd,
                read_fd,
                delimiter,
                char_limit,
                exact_char_limit,
            ) {
                Some(l) if l.is_empty() && initial_text.is_some() => None,
                Some(l) => Some(l),
                None => None,
            };
            let read_timed_out = self.take_read_timed_out();
            let final_line = match (&line, &initial_text) {
                (Some(line), _) => line.clone(),
                (None, Some(text)) => text.clone(),
                (None, None) => {
                    let _ = self
                        .shell_state
                        .variables
                        .replace_indexed_array(&name, std::iter::empty::<String>());
                    self.shell_state
                        .env_vars
                        .insert(name.clone(), read_array_storage(&[]));
                    mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
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
            let value = read_array_storage(&values);
            let _ = self
                .shell_state
                .variables
                .replace_indexed_array(&name, values);
            self.shell_state.env_vars.insert(name.clone(), value);
            mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
            return if invalid_name {
                self.finish_read_error(cmd, &stderr, 1)
            } else if read_timed_out {
                142
            } else {
                0
            };
        }

        let scalar_names = if scalar_names.is_empty() {
            if invalid_name {
                Vec::new()
            } else {
                scalar_field_count = 0;
                vec!["REPLY".to_string()]
            }
        } else {
            scalar_names
        };
        if !scalar_names.is_empty() {
            if char_limit == Some(0) {
                let assign_status = self.assign_read_scalar_names(&scalar_names, "", raw);
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                return if invalid_name {
                    self.finish_read_error(cmd, &stderr, 1)
                } else {
                    0
                };
            }

            self.arm_read_deadline(cmd, read_fd, timeout_secs);
            let status = if let Some(line) =
                self.read_input_for_command(cmd, read_fd, delimiter, char_limit, exact_char_limit)
            {
                let line = if !raw && delimiter == '\n' && char_limit.is_none() && !exact_char_limit
                {
                    self.continue_read_line_after_backslash(cmd, read_fd, line)
                } else {
                    line
                };
                let line = if line.is_empty() && initial_text.is_some() {
                    initial_text.as_deref().unwrap()
                } else {
                    &line
                };
                let assign_status = self.assign_read_scalar_names_with_field_count(
                    &scalar_names,
                    line,
                    raw,
                    scalar_field_count,
                );
                if assign_status != 0 {
                    return self.finish_read_error(cmd, &stderr, assign_status);
                }
                0
            } else if command_closes_stdin(cmd) || self.fd_table.is_closed(0) {
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
                    "{}read: read error: 0: Bad file descriptor",
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
            } else {
                match read_stdin_until(delimiter, char_limit, exact_char_limit) {
                    Ok((0, _)) => {
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
                    Ok((_, line)) => {
                        let line = if !raw
                            && delimiter == '\n'
                            && char_limit.is_none()
                            && !exact_char_limit
                        {
                            self.continue_read_line_after_backslash(cmd, read_fd, line)
                        } else {
                            line
                        };
                        let line = if line.is_empty() && initial_text.is_some() {
                            initial_text.as_deref().unwrap()
                        } else {
                            &line
                        };
                        let assign_status = self.assign_read_scalar_names(&scalar_names, line, raw);
                        if assign_status != 0 {
                            return self.finish_read_error(cmd, &stderr, assign_status);
                        }
                        0
                    }
                    Err(_) => 1,
                }
            };
            // GNU read.def:539-562: on timeout the vars keep the partial
            // assignment above and the builtin returns 128+SIGALRM=142.
            let status = if self.take_read_timed_out() {
                142
            } else {
                status
            };
            return if invalid_name {
                self.finish_read_error(cmd, &stderr, 1)
            } else {
                status
            };
        }
        if invalid_name {
            return self.finish_read_error(cmd, &stderr, 1);
        }
        let _ = writeln!(
            &mut stderr,
            "{}read: command not found",
            self.diagnostic_prefix()
        );
        self.finish_read_error(cmd, &stderr, 127)
    }

    fn read_prompt_should_display(&self, cmd: &CommandNode, read_fd: Option<u32>) -> bool {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_SCRIPT_NAME")
            || self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
            || command_redirects_stdin(cmd)
            || command_closes_stdin(cmd)
        {
            return false;
        }

        let fd = read_fd.unwrap_or(0);
        !matches!(
            self.fd_table.read_endpoint(fd),
            Some(
                FdReadEndpoint::Text(_)
                    | FdReadEndpoint::ProcessSubstitution(_)
                    | FdReadEndpoint::CoprocStdout { .. }
                    | FdReadEndpoint::InheritedProcessStdin
            )
        )
    }

    fn read_fd_is_available(&self, cmd: &CommandNode, fd: u32) -> bool {
        if self.coproc_read_file(fd).is_some() || (fd == 0 && self.first_coproc_read().is_some()) {
            return true;
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
        cmd.redirect_in.as_ref().is_some_and(|redirect| {
            redirect.fd == Some(fd)
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        })
    }

    fn read_timeout_zero_status(&mut self, cmd: &CommandNode, read_fd: Option<u32>) -> i32 {
        if let Some(fd) = read_fd {
            if matches!(
                self.fd_table.read_endpoint(fd),
                Some(FdReadEndpoint::InheritedProcessStdin)
            ) {
                return 1;
            }
            if self.fd_table.is_open_for_read(fd) {
                return 0;
            }
            if cmd
                .heredoc_redirects
                .iter()
                .any(|redirect| redirect.fd == Some(fd) && redirect.body.is_some())
            {
                return 0;
            }
            if cmd.redirect_in.as_ref().is_some_and(|redirect| {
                redirect.fd == Some(fd)
                    && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
            }) {
                return 0;
            }
            return 1;
        }

        if command_closes_stdin(cmd) || self.fd_table.is_closed(0) {
            return 1;
        }
        if cmd.redirect_in.as_ref().is_some_and(|redirect| {
            redirect.fd.unwrap_or(0) == 0
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        }) {
            return 0;
        }
        if cmd
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.fd.unwrap_or(0) == 0 && redirect.body.is_some())
        {
            return 0;
        }
        if self.stdin_string_for_command_mut(cmd).is_some()
            || matches!(
                self.fd_table.read_endpoint(0),
                Some(
                    FdReadEndpoint::Text(_)
                        | FdReadEndpoint::ProcessSubstitution(_)
                        | FdReadEndpoint::CoprocStdout { .. }
                )
            )
            || self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
        {
            return 0;
        }
        // GNU read.def polls fd 0 with select(): a non-terminal inherited
        // stdin (regular file, /dev/null, a pipe at EOF) is always readable,
        // so `-t 0` succeeds. Only a live terminal with no pending input
        // fails the poll.
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            1
        } else {
            0
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
        self.fd_table.read_deadline = deadline;
        self.fd_table.read_timed_out = false;
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
                    let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
                    return std::fs::metadata(&path)
                        .map(|m| m.is_file())
                        .unwrap_or(false);
                }
            }
        }
        false
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
            'a' | 'd' | 'e' | 'i' | 'n' | 'N' | 'p' | 'r' | 's' | 't' | 'u'
        )
    })
}

fn command_closes_stdin(cmd: &CommandNode) -> bool {
    cmd.redirect_in
        .as_ref()
        .is_some_and(|redirect| redirect.fd.unwrap_or(0) == 0 && redirect.target == "&-")
}

fn command_redirects_stdin(cmd: &CommandNode) -> bool {
    cmd.redirect_in
        .as_ref()
        .is_some_and(|redirect| redirect.fd.unwrap_or(0) == 0)
}

fn redirected_input_fd(executor: &Executor, cmd: &CommandNode) -> Option<u32> {
    let redirect = cmd.redirect_in.as_ref()?;
    if !redirect.operator.contains('&') {
        return None;
    }
    let target = executor.expand_redirect_target(redirect);
    let target = target.strip_prefix('&').unwrap_or(&target);
    (target != "-")
        .then(|| target.parse::<u32>().ok())
        .flatten()
}
