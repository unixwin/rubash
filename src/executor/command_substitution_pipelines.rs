use super::*;

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

/// True when the command-substitution source's first heredoc header line
/// also carries the closing `)` (`cat << EOF)`). GNU treats this as an
/// unterminated here-document inside the substitution (parse.y:4563-4567)
/// and warns before gathering it (heredoc7.sub).
fn heredoc_header_closes_command_substitution(source: &str) -> bool {
    let Some(header) = source.lines().next() else {
        return false;
    };
    if !header.contains("<<") {
        return false;
    }
    header
        .split("<<")
        .nth(1)
        .is_some_and(|after| after.trim().ends_with(')'))
}

fn mktemp_command_substitution_display_path(path: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        return windows_mktemp_display_path(path);
    }

    #[cfg(not(windows))]
    {
        let display = path.to_string_lossy().replace('\\', "/");
        shell_display_path(&display)
    }
}

#[cfg(windows)]
fn windows_mktemp_display_path(path: &std::path::Path) -> String {
    let native = windows_slash_drive_to_native(path);
    let long = windows_long_path(&native);
    windows_display_path(&long)
}

#[cfg(windows)]
fn windows_slash_drive_to_native(path: &std::path::Path) -> std::path::PathBuf {
    let display = path.to_string_lossy().replace('\\', "/");
    if display.len() >= 3
        && display.as_bytes()[0] == b'/'
        && display.as_bytes()[2] == b'/'
        && display.as_bytes()[1].is_ascii_alphabetic()
    {
        let drive = display.as_bytes()[1] as char;
        return std::path::PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &display[3..]).replace('/', "\\"),
        );
    }
    path.to_path_buf()
}

#[cfg(windows)]
fn windows_long_path(path: &std::path::Path) -> std::path::PathBuf {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut buffer = vec![0u16; 32768];
    let written =
        unsafe { GetLongPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
    if written == 0 || written as usize >= buffer.len() {
        return path.to_path_buf();
    }
    buffer.truncate(written as usize);
    std::path::PathBuf::from(OsString::from_wide(&buffer))
}

#[cfg(windows)]
fn windows_display_path(path: &std::path::Path) -> String {
    let display = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = display.strip_prefix("//?/UNC/") {
        return format!("//{rest}");
    }
    if let Some(rest) = display.strip_prefix("//?/") {
        return rest.to_string();
    }
    display
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetLongPathNameW(short_path: *const u16, long_path: *mut u16, buffer_length: u32) -> u32;
}

impl Executor {
    pub(in crate::executor) fn mktemp_command_substitution(
        &self,
        words: &[String],
    ) -> Option<String> {
        // TODO(subst.c/execute_cmd.c): command substitution should fork a
        // subshell and capture external command stdout. This covers common
        // script prologues like `tmp=$(mktemp -t name.XXXXXX) || exit`.
        if words.first().map(String::as_str) != Some("mktemp") {
            return None;
        }
        let mut directory = false;
        let mut template = "rubash-mktemp.XXXXXX";
        let mut index = 1;
        while index < words.len() {
            match words[index].as_str() {
                "-d" => {
                    directory = true;
                    index += 1;
                }
                "-t" => {
                    template = words.get(index + 1)?.as_str();
                    index += 2;
                }
                "<" | ">" | ">>" | ">|" | "1>" | "1>>" | "1>|" | "2>" | "2>>" | "2>|" => {
                    words.get(index + 1)?;
                    index += 2;
                }
                value if value.starts_with('-') => return None,
                value => {
                    template = value;
                    index += 1;
                }
            }
        }
        let dir = self
            .env_vars
            .get("TMPDIR")
            .filter(|value| !value.contains('\0'))
            .cloned()
            .unwrap_or_else(safe_temp_dir_string);
        let dir = shell_path_to_windows(&dir, &self.env_vars);
        std::fs::create_dir_all(&dir).ok()?;
        let mut path = None;
        for attempt in 0..32 {
            let unique = format!(
                "{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0),
                attempt
            );
            let filename = if template.contains("XXXXXX") {
                template.replace("XXXXXX", &unique)
            } else {
                format!("{template}.{unique}")
            };
            let candidate = dir.join(filename);
            let created = if directory {
                std::fs::create_dir_all(&candidate).is_ok()
            } else {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                    .is_ok()
            };
            if created {
                path = Some(candidate);
                break;
            }
        }
        let path = path?;
        self.last_command_substitution_status.set(Some(0));
        Some(mktemp_command_substitution_display_path(&path))
    }

    pub(in crate::executor) fn command_substitution_heredoc_output_mut_typed(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<SubstitutionOutput> {
        if !source.contains("<<") {
            return None;
        }
        let closed_by_paren = source.contains('\x1c');
        let source = source.replace('\x1c', "");
        if heredoc_header_closes_command_substitution(&source) {
            // GNU parse.y:4563-4567: when the `)` that closes a command
            // substitution sits on the heredoc header line (`cat << EOF)`),
            // the heredoc has not been gathered yet, so bash warns and then
            // gathers it anyway (heredoc7.sub line 17).
            let start_line = self
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1);
            eprintln!(
                "{}warning: command substitution: 1 unterminated here-document",
                self.diagnostic_prefix_for_line(start_line)
            );
        }
        let tokens =
            crate::lexer::tokenize_comsub_body(&source, self.posix_mode_enabled(), 1, true);
        let ast = crate::parser::parse(&tokens);
        let first = ast.commands.first()?;
        let (first, piped_next) = if let Some(pipeline_command) = &first.pipeline_command {
            (
                pipeline_command.stages.first()?,
                pipeline_command.stages.get(1),
            )
        } else {
            (first, ast.commands.get(1))
        };
        if first.words.first().map(String::as_str) != Some("cat") {
            return None;
        }
        if closed_by_paren {
            self.report_command_substitution_heredoc_warning(&source, first);
        }
        let mut output = if first.pipe.is_none() && ast.commands.len() > 1 {
            let mut output = String::new();
            for command in &ast.commands {
                if command.words.first().map(String::as_str) != Some("cat")
                    || command.pipe.is_some()
                {
                    return None;
                }
                output.push_str(&self.stdin_string_for_command_mut(command)?);
            }
            output
        } else {
            let mut output = self.stdin_string_for_command_mut(first)?;
            if first.pipe.is_some() {
                let next = piped_next?;
                match next.words.as_slice() {
                    [cmd, option] if cmd == "sort" && option == "-u" => {
                        let mut lines = output.lines().map(str::to_string).collect::<Vec<_>>();
                        lines.sort();
                        lines.dedup();
                        output = lines.join("\n");
                        output.push('\n');
                    }
                    _ => return None,
                }
            }
            output
        };
        output = output.trim_end_matches('\n').to_string();
        self.last_command_substitution_status.set(Some(0));
        Some(SubstitutionOutput::readback(
            output.into_bytes(),
            0,
            context,
        ))
    }

    pub(in crate::executor) fn command_substitution_heredoc_output(
        &self,
        source: &str,
    ) -> Option<String> {
        if !source.contains("<<") {
            return None;
        }

        let closed_by_paren = source.contains('\x1c');
        let source = source.replace('\x1c', "");
        let tokens =
            crate::lexer::tokenize_comsub_body(&source, self.posix_mode_enabled(), 1, true);
        let ast = crate::parser::parse(&tokens);
        let first = ast.commands.first()?;
        let (first, piped_next) = if let Some(pipeline_command) = &first.pipeline_command {
            (
                pipeline_command.stages.first()?,
                pipeline_command.stages.get(1),
            )
        } else {
            (first, ast.commands.get(1))
        };
        if first.words.first().map(String::as_str) != Some("cat") {
            return None;
        }

        if closed_by_paren {
            self.report_command_substitution_heredoc_warning(&source, first);
        }

        if first.pipe.is_none() && ast.commands.len() > 1 {
            let mut output = String::new();
            for command in &ast.commands {
                if command.words.first().map(String::as_str) != Some("cat")
                    || command.pipe.is_some()
                {
                    return None;
                }
                output.push_str(&self.stdin_string_for_command(command)?);
            }
            return Some(output.trim_end_matches('\n').to_string());
        }

        let mut output = self.stdin_string_for_command(first)?;
        if first.pipe.is_some() {
            let next = piped_next?;
            match next.words.as_slice() {
                [cmd, option] if cmd == "sort" && option == "-u" => {
                    let mut lines = output.lines().map(str::to_string).collect::<Vec<_>>();
                    lines.sort();
                    lines.dedup();
                    output = lines.join("\n");
                    output.push('\n');
                }
                _ => return None,
            }
        }

        Some(output.trim_end_matches('\n').to_string())
    }

    pub(in crate::executor) fn command_substitution_pipeline_output(
        &self,
        words: &[String],
    ) -> Option<(String, i32)> {
        if !words.iter().any(|word| word == "|") {
            return None;
        }

        let stages = split_pipeline_words(words)?;
        // GNU subst.c parses and executes the whole pipeline; this word-level
        // shortcut only covers plain producer/filter stages. A stage carrying
        // redirection syntax (`git symbolic-ref HEAD 2>/dev/null | sed ...`)
        // would receive the redirect as a literal argument, and a stage whose
        // head names a shell function must run the function, not an external
        // program of the same name. Both fall back to the real parser
        // (issue #70).
        if stages
            .iter()
            .any(|stage| stage.iter().any(|word| command_word_carries_redirect(word)))
        {
            return None;
        }
        if let Some(head) = stages.first().and_then(|stage| stage.first()) {
            if self.functions.contains_key(head) {
                return None;
            }
        }
        let mut output = self.command_substitution_pipeline_first_stage(stages.first()?)?;
        let mut status = 0;
        for stage in stages.iter().skip(1) {
            (output, status) = self.command_substitution_pipeline_filter(stage, &output)?;
        }
        Some((output.trim_end_matches('\n').to_string(), status))
    }

    pub(in crate::executor) fn timed_command_substitution_output(
        &self,
        words: &[String],
    ) -> Option<String> {
        if words.first().map(String::as_str) != Some("time") {
            return None;
        }

        let mut index = 1;
        let mut inverted = false;
        while let Some(word) = words.get(index).map(String::as_str) {
            match word {
                "-p" | "--" | "time" => index += 1,
                "!" => {
                    inverted = !inverted;
                    index += 1;
                }
                _ => break,
            }
        }

        let started = time_command_started();
        let output = self.timed_command_substitution_inner(&words[index..])?;
        print_time(
            &self.env_vars,
            words
                .iter()
                .skip(1)
                .take_while(|word| word.as_str() != "!")
                .any(|word| word == "-p"),
            started,
        );
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        self.last_command_substitution_status.set(Some(if inverted {
            invert_exit_status(status)
        } else {
            status
        }));
        Some(output)
    }

    fn timed_command_substitution_inner(&self, words: &[String]) -> Option<String> {
        match words.first().map(String::as_str) {
            None | Some(":") | Some("true") => {
                self.last_command_substitution_status.set(Some(0));
                Some(String::new())
            }
            Some("false") => {
                self.last_command_substitution_status.set(Some(1));
                Some(String::new())
            }
            Some("echo") => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                self.last_command_substitution_status.set(Some(0));
                Some(echo_command_substitution_output(&args))
            }
            Some("printf") => {
                let expanded_args = words[1..]
                    .iter()
                    .flat_map(|word| self.expand_command_substitution_arg_values(word))
                    .collect::<Vec<_>>();
                let mut env_vars = self.env_vars.clone();
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::printf::execute_with_io(
                    expanded_args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut stdout,
                    &mut stderr,
                )
                .unwrap_or(1);
                self.last_command_substitution_status.set(Some(status));
                Some(
                    bytes_to_shell_text(&stdout)
                        .trim_end_matches('\n')
                        .to_string(),
                )
            }
            Some(_) if words.iter().any(|word| word == "|") => {
                let (output, status) = self.command_substitution_pipeline_output(words)?;
                self.last_command_substitution_status.set(Some(status));
                Some(output)
            }
            Some("tty") | Some("/bin/tty") | Some("/usr/bin/tty") => {
                let silent = words
                    .iter()
                    .skip(1)
                    .any(|arg| arg == "-s" || arg == "--silent" || arg == "--quiet");
                self.last_command_substitution_status.set(Some(1));
                Some(if silent {
                    String::new()
                } else {
                    "not a tty".to_string()
                })
            }
            Some("cat") => {
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    if let Ok(value) =
                        fs::read_to_string(shell_path_to_windows(&path, &self.env_vars))
                    {
                        output.push_str(&value);
                    }
                }
                self.last_command_substitution_status.set(Some(0));
                Some(output.trim_end_matches('\n').to_string())
            }
            Some(_) => self.run_external_command_substitution(words),
        }
    }

    pub(in crate::executor) fn command_substitution_pipeline_first_stage(
        &self,
        words: &[String],
    ) -> Option<String> {
        match words.first().map(String::as_str)? {
            "echo" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                Some(echo_raw_output(&args))
            }
            "printf" => {
                let expanded_args: Vec<String> = words[1..]
                    .iter()
                    .flat_map(|word| self.expand_command_substitution_arg_values(word))
                    .collect();
                let mut env_vars = self.env_vars.clone();
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = crate::builtins::printf::execute_with_io(
                    expanded_args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut stdout,
                    &mut stderr,
                );
                Some(bytes_to_shell_text(&stdout))
            }
            "cat" => {
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    if let Ok(value) =
                        fs::read_to_string(shell_path_to_windows(&path, &self.env_vars))
                    {
                        output.push_str(&value);
                    }
                }
                Some(output)
            }
            "command" => self
                .command_describe_substitution_output(words)
                .map(|mut output| {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output
                }),
            _ => {
                // Generic external command first stage
                let cmd_name = self.expand_word(&words[0]);
                let expanded_args: Vec<String> =
                    words[1..].iter().map(|w| self.expand_word(w)).collect();
                use std::process::{Command, Stdio};
                let output = Command::new(&cmd_name)
                    .args(&expanded_args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                    .ok()?;
                Some(bytes_to_shell_text(&output.stdout))
            }
        }
    }
}

#[cfg(test)]
mod mktemp_display_path_tests {
    use super::*;

    #[test]
    fn mktemp_display_path_uses_native_windows_drive() {
        let display = mktemp_command_substitution_display_path(std::path::Path::new(
            "/c/Users/example/AppData/Local/Temp/rubash-mktemp.1",
        ));

        if cfg!(windows) {
            assert_eq!(
                display,
                "C:/Users/example/AppData/Local/Temp/rubash-mktemp.1"
            );
        } else {
            assert_eq!(
                display,
                "/c/Users/example/AppData/Local/Temp/rubash-mktemp.1"
            );
        }
    }
}
