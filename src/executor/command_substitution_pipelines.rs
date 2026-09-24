use super::*;

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

/// True when the command-substitution source's first heredoc header line
/// also carries the closing `)` (`cat << EOF)`). GNU treats this as an
/// unterminated here-document inside the substitution (parse.y:4563-4567)
/// and warns before gathering it (heredoc7.sub).
///
/// The `)` must be AFTER the heredoc delimiter word, not part of it.
/// `cat <<\)` has `)` as the delimiter (backslash-quoted), so the `)` is
/// NOT the command-substitution closer.  `cat <<EOF)` has `EOF` as the
/// delimiter and `)` as the closer.
fn heredoc_header_closes_command_substitution(source: &str) -> bool {
    let Some(header) = source.lines().next() else {
        return false;
    };
    let Some(ll_pos) = header.find("<<") else {
        return false;
    };
    let after = &header[ll_pos + 2..];
    // Skip optional '-' for <<-
    let after = after.strip_prefix('-').unwrap_or(after);
    // Skip leading whitespace
    let after = after.trim_start();
    // Parse the delimiter word: characters until unquoted whitespace, `;`,
    // `|`, `&`, or `)`.  Backslash escapes the next character.  Single and
    // double quotes delimit quoted sections that are part of the delimiter.
    // Byte-level scan: the delimiter grammar is pure ASCII structure
    // (backslash, quotes, whitespace, ;|&)). Comparing raw bytes avoids the
    // byte->char widening hazard — a UTF-8 continuation byte 0x85 widened to
    // char would read as U+0085 and satisfy char::is_whitespace(),
    // truncating a multibyte delimiter mid-character (see
    // scripts/check-utf8-boundary-hygiene.sh).
    let bytes = after.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'\\' && i + 1 < bytes.len() {
            // Escaped character is part of the delimiter
            i += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            // Quoted section: skip to matching quote
            let quote = byte;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip closing quote
            }
            continue;
        }
        if byte.is_ascii_whitespace() || matches!(byte, b';' | b'|' | b'&' | b')') {
            break;
        }
        i += 1;
    }
    // Check if there's a `)` after the delimiter word
    after[i..].trim_start().starts_with(')')
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
            .shell_state
            .env_vars
            .get("TMPDIR")
            .filter(|value| !value.contains('\0'))
            .cloned()
            .unwrap_or_else(safe_temp_dir_string);
        let dir = shell_path_to_windows(&dir, &self.shell_state.env_vars);
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
        let closed_by_paren = source.contains(crate::executor::markers::IFS_GLUE);
        let source = source.replace(crate::executor::markers::IFS_GLUE, "");
        let source = self.comsub_body_alias_splice_extracted(&source);
        if heredoc_header_closes_command_substitution(&source) {
            // GNU parse.y:4563-4567: when the `)` that closes a command
            // substitution sits on the heredoc header line (`cat << EOF)`),
            // the heredoc has not been gathered yet, so bash warns and then
            // gathers it anyway (heredoc7.sub line 17).
            let start_line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1);
            eprintln!(
                "{}warning: command substitution: 1 unterminated here-document",
                self.diagnostic_prefix_for_line(start_line)
            );
        }
        let comsub_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .unwrap_or(1)
            + self.comsub_leading_newlines.get();
        let tokens = crate::lexer::tokenize_comsub_body(
            &source,
            self.posix_mode_enabled(),
            comsub_start_line,
            true,
        );
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
        if closed_by_paren || command_has_warned_heredoc(first) {
            // Deduplicate: the same comsub can be expanded through multiple
            // paths (expand_assignment_value_inner and
            // expand_embedded_parameters_mut), which would emit the warning
            // twice. Track the last warning source and skip duplicates.
            let should_emit = self
                .last_heredoc_warning_source
                .borrow()
                .as_deref()
                .map(|last| last != source)
                .unwrap_or(true);
            if should_emit {
                *self.last_heredoc_warning_source.borrow_mut() = Some(source.to_string());
                self.report_command_substitution_heredoc_warning(&source, first);
            }
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
        output = output.trim_capture_terminator().to_string();
        self.last_command_substitution_status.set(Some(0));
        Some(SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
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

        let closed_by_paren = source.contains(crate::executor::markers::IFS_GLUE);
        let source = source.replace(crate::executor::markers::IFS_GLUE, "");
        let source = self.comsub_body_alias_splice_extracted(&source);
        let comsub_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .unwrap_or(1)
            + self.comsub_leading_newlines.get();
        let tokens = crate::lexer::tokenize_comsub_body(
            &source,
            self.posix_mode_enabled(),
            comsub_start_line,
            true,
        );
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

        // Warning is emitted by command_substitution_heredoc_output_mut_typed
        // (the typed path) to avoid duplicate warnings when both paths are
        // called for the same comsub.
        let _ = closed_by_paren;
        let _ = command_has_warned_heredoc(first);

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
            return Some(output.trim_capture_terminator().to_string());
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

        Some(output.trim_capture_terminator().to_string())
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
            if self.shell_state.functions.contains_key(head) {
                return None;
            }
        }
        let mut output = self.command_substitution_pipeline_first_stage(stages.first()?)?;
        let mut status = 0;
        for stage in stages.iter().skip(1) {
            (output, status) = self.command_substitution_pipeline_filter(stage, &output)?;
        }
        Some((output.trim_capture_terminator().to_string(), status))
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
            &self.shell_state.env_vars,
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
                let mut env_vars = self.shell_state.env_vars.clone();
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
                        .trim_capture_terminator()
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
                // Bare `cat` (or flag/`-` operands) reads fd 0 — the shared
                // FUNCTION_STDIN cursor — which this shortcut cannot model;
                // fall back to real execution (external_cat owns it).
                if words.len() <= 1 || words[1..].iter().any(|word| word.starts_with('-')) {
                    return None;
                }
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    if let Ok(value) =
                        fs::read_to_string(shell_path_to_windows(&path, &self.shell_state.env_vars))
                    {
                        output.push_str(&value);
                    }
                }
                self.last_command_substitution_status.set(Some(0));
                Some(output.trim_capture_terminator().to_string())
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
                // GNU subst.c expand_words runs pathname expansion on each
                // word of a pipeline stage inside a command substitution
                // (`$(echo * | cat)` yields the directory listing). The
                // printf arm beside this one already routes through
                // expand_command_substitution_arg_values for that reason;
                // the echo arm was still calling plain expand_word, so a
                // literal glob pattern in the first stage stayed a literal.
                let args: Vec<String> = words[1..]
                    .iter()
                    .flat_map(|word| self.expand_command_substitution_arg_values(word))
                    .collect();
                Some(echo_raw_output(&args))
            }
            "printf" => {
                let expanded_args: Vec<String> = words[1..]
                    .iter()
                    .flat_map(|word| self.expand_command_substitution_arg_values(word))
                    .collect();
                let mut env_vars = self.shell_state.env_vars.clone();
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
                if words.len() <= 1 || words[1..].iter().any(|word| word.starts_with('-')) {
                    return None;
                }
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    if let Ok(value) =
                        fs::read_to_string(shell_path_to_windows(&path, &self.shell_state.env_vars))
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
                // Generic external command first stage. Do NOT spawn it here.
                // This substitution fast path would need the real executor's
                // full child environment (apply_env_command_environment sets
                // every shell var); apply_child_environment only forwards
                // exported/marked vars, and with that stripped environment a
                // winuxcmd command such as `find` misbehaves and writes its
                // output to stderr, which the capture discards — so
                // `$(find ... | wc -l)` silently yields 0 (issue #76). Falling
                // through lets command_list_substitution_output run the
                // pipeline through the real executor, which captures it
                // correctly. Builtin first stages (echo/printf/cat/command)
                // above keep using this fast path.
                return None;
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
