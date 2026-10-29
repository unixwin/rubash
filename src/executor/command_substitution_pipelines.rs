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

/// An admitted `cat` heredoc/here-string comsub body: the parsed AST
/// of a body that passed the shared admission whitelist in
/// `Executor::heredoc_comsub_plan`. Production (expanding each
/// command's stdin body and concatenating) is parameterized by the
/// caller's fetch — the typed path expands with side effects, the
/// readback path without — so both entry points share this one
/// plan/produce pair and a body is admission-parsed at most once per
/// expansion chain.
struct HeredocComsubPlan {
    ast: crate::parser::Ast,
}

/// Key for the decline-only heredoc-admission memo: the source with
/// the lexer-spliced IFS_GLUE marker removed. The typed entry point
/// sees the raw source (marker present); the readback retry inside
/// the mut dispatcher sees the cleaned source — normalizing on the
/// stripped text lets the two spellings of one body share a record.
fn heredoc_admission_memo_key(source: &str) -> String {
    if source.contains(crate::executor::markers::IFS_GLUE) {
        source.replace(crate::executor::markers::IFS_GLUE, "")
    } else {
        source.to_string()
    }
}

/// Produce the captured output of an admitted heredoc comsub plan:
/// a sequence of plain `cat` commands concatenates each command's
/// stdin body; a single `cat` may feed the `[sort, -u]` pipeline arm.
/// `fetch` supplies one command's stdin text (typed or readback
/// expansion, per the entry point).
fn heredoc_comsub_produce(
    plan: &HeredocComsubPlan,
    fetch: &mut dyn FnMut(&CommandNode) -> Option<String>,
) -> Option<String> {
    let ast = &plan.ast;
    let first = ast.commands.first()?;
    let (first, piped_next) = if let Some(pipeline_command) = &first.pipeline_command {
        (
            pipeline_command.stages.first()?,
            pipeline_command.stages.get(1),
        )
    } else {
        (first, ast.commands.get(1))
    };
    let mut output = if first.pipe.is_none() && ast.commands.len() > 1 {
        let mut output = String::new();
        for command in &ast.commands {
            if command.words.first().map(String::as_str) != Some("cat")
                || command.pipe.is_some()
                // rubash#320: redirect-carrying cats are not
                // shortcut-eligible (see command_has_file_redirect).
                || command_has_file_redirect(command)
            {
                return None;
            }
            output.push_str(&fetch(command)?);
        }
        output
    } else {
        let mut output = fetch(first)?;
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
    Some(output)
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
        // coreutils default template: tmp.XXXXXXXXXX (10 X's, gnulib
        // __gen_tempname base "tmp").
        let mut template = "tmp.XXXXXXXXXX";
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
        // coreutils mktemp name generation (gnulib try_tempname_len /
        // mktemp(1)): the trailing run of X's in the template (minimum 3)
        // is replaced character-by-character from the 62-char
        // [a-zA-Z0-9] alphabet — never a pid/timestamp string (rubash#232:
        // `mktemp /tmp/t.XXXXXX` output must look like `TnVGKu`, and
        // scripts parse/validate the shape). Randomness comes from a
        // splitmix of (nanos, pid, attempt); the create_new retry loop
        // below is the collision backstop.
        let x_run = template.chars().rev().take_while(|c| *c == 'X').count();
        for attempt in 0..32u64 {
            let mut state = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0)
                ^ ((std::process::id() as u64) << 32)
                ^ attempt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut random_suffix = |state: &mut u64| {
                *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = *state;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                const ALPHABET: &[u8; 62] =
                    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                char::from(ALPHABET[(z ^ (z >> 31)) as usize % 62])
            };
            let suffix: String = (0..x_run).map(|_| random_suffix(&mut state)).collect();
            let filename = if x_run >= 3 {
                format!("{}{}", &template[..template.len() - x_run], suffix)
            } else {
                // No X-run in the template: GNU mktemp appends its own
                // 10-char random suffix (`tmp.XXXXXXXXXX` shape); keep the
                // historical dotted-append with the same alphabet.
                let dot_suffix: String = (0..10).map(|_| random_suffix(&mut state)).collect();
                format!("{template}.{dot_suffix}")
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

    /// Admission for the `cat` heredoc/here-string comsub shortcut,
    /// shared by the typed (&mut) and readback (&self) entry points
    /// below: tokenize + parse the body ONCE and run the full admission
    /// whitelist (a leading `cat` that does not stand down, rubash#320
    /// no file redirection, rubash#1130 no expansion error in the body).
    /// `emit_warnings` is set only by the typed entry point, which owns
    /// the delimited-by-end-of-file here-document warning; the readback
    /// entry stays silent for that one so a comsub expanded through
    /// both paths warns once. The header-close warning at the top of
    /// this function is entry-independent (rubash#1982).
    fn heredoc_comsub_plan(&self, source: &str, emit_warnings: bool) -> Option<HeredocComsubPlan> {
        if !source.contains("<<") {
            return None;
        }
        // rubash#1982: the reader's comsub-heredoc rotation
        // (lexer::relocate_comsub_heredoc_paren) splices an IFS_GLUE
        // marker immediately after the owning `$(`, so a marker
        // inside the source's FIRST line records "the closing paren
        // sat on the heredoc header line" even though the reprinted
        // source no longer contains that `)`. A marker at or after
        // the first newline is the older delimiter-line record
        // (`EOF)`) and keeps its closed_by_paren meaning for the
        // delimited-by-end-of-file warning below.
        let first_line_end = source.find('\n').unwrap_or(source.len());
        let header_close_marked =
            source[..first_line_end].contains(crate::executor::markers::IFS_GLUE);
        let closed_by_paren = source[first_line_end..].contains(crate::executor::markers::IFS_GLUE);
        let source = source.replace(crate::executor::markers::IFS_GLUE, "");
        let source = self.comsub_body_alias_splice_extracted(&source);
        if header_close_marked || heredoc_header_closes_command_substitution(&source) {
            // GNU parse.y:4563-4567: when the `)` that closes a command
            // substitution sits on the heredoc header line (`cat << EOF)`),
            // the heredoc has not been gathered yet, so bash warns and then
            // gathers it anyway (heredoc7.sub line 17).
            // rubash#1982: this warning belongs to the substitution's
            // parse, not to one expansion face — GNU emits it whichever
            // way the word is expanded, so it must not ride
            // `emit_warnings` (some shapes are produced entirely by the
            // readback entry, which used to silence it completely).
            // The textual check covers sources that still carry the
            // header-line `)`; reader-produced sources carry the
            // rotation's marker instead (header_close_marked above).
            // Once-ness across faces is already the decline memo's
            // contract (a typed decline is recorded and the readback
            // retry never re-plans the same source), so no second
            // emission site exists for one expansion episode.
            // `emit_warnings` now owns only the
            // delimited-by-end-of-file warning further below.
            let start_line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CMD_START_LINE")
                .or_else(|| self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE"))
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1);
            eprintln!(
                "{}warning: command substitution: 1 unterminated here-document",
                self.error_prefix_for_line(start_line)
            );
        }
        // The cat/heredoc-optimized comsub paths re-derive the here-document
        // warning lines, which GNU reports from the outer script's physical
        // parse (parse_comsub yyparse + gather_here_documents), so seed the
        // body numbering from the command's START line, not the
        // execution-time line_number (rubash#201: comsub-eof0.sub).
        let comsub_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CMD_START_LINE")
            .or_else(|| self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE"))
            .and_then(|line| line.parse::<usize>().ok())
            .unwrap_or(1)
            + self.comsub_leading_newlines.get();
        let tokens = crate::lexer::tokenize_comsub_body(
            &source,
            self.posix_mode_enabled(),
            comsub_start_line,
            true,
        );
        let mut ast = crate::parser::parse(&tokens);
        super::command_substitution::normalize_comsub_body_statement_lines(&mut ast, &tokens);
        let first = ast.commands.first()?;
        let (first, piped_next) = if let Some(pipeline_command) = &first.pipeline_command {
            (
                pipeline_command.stages.first()?,
                pipeline_command.stages.get(1),
            )
        } else {
            (first, ast.commands.get(1))
        };
        if first.words.first().map(String::as_str) != Some("cat")
            || self.emulated_external_stands_down("cat")
        {
            // When cat does not resolve (PATH unset, rubash#1126; or a
            // set PATH without it, rubash#1545), the heredoc shortcut
            // must not produce the content.
            return None;
        }
        // GNU redir.c expands the heredoc / here-string word in the
        // child with the full error machinery: under `set -u` an
        // unbound variable there is fatal to the substitution
        // (rubash#1130: `$(cat <<< $p)` reported the diagnostic and
        // left the assignment status 1). This shortcut expands the
        // body itself and would silently substitute the empty string,
        // so decline any command whose body carries an expansion
        // error and let the real executor raise it.
        let body_has_expansion_error = ast.commands.iter().any(|command| {
            let mut blocked = false;
            if let Some(word) = &command.here_string {
                let trimmed = word.trim_start();
                if !trimmed.starts_with('\'') && !trimmed.starts_with("$'") {
                    blocked |= self.parameter_expansion_error_in_word(word).is_some();
                }
            }
            for redirect in &command.heredoc_redirects {
                if let Some(body) = &redirect.body {
                    blocked |= self
                        .parameter_expansion_error_in_heredoc_body(body)
                        .is_some();
                }
            }
            blocked
        });
        if body_has_expansion_error {
            return None;
        }
        // rubash#320: admission whitelist — this fast path models only
        // `cat <<EOF` reading its heredoc and echoing it to the captured
        // stdout. GNU executes `cat <<EOF > /dev/null` with the redirect
        // bound first (redir.c do_redirection_internal before exec), so the
        // capture is EMPTY. A command carrying any file redirection (or an
        // fd duplication) must fall through to the real executor instead of
        // losing the redirect here — a false admission is a silent semantic
        // bug; declining only costs the shortcut.
        if command_has_file_redirect(first) {
            return None;
        }
        if piped_next.is_some_and(command_has_file_redirect) {
            return None;
        }
        // rubash#1981: admission must verify the whole shape the
        // produce step can model. heredoc_comsub_produce concatenates
        // each admitted cat's stdin body and nothing else: it cannot
        // honor file operands (GNU cat interleaves operand files with
        // the heredoc in argument order — `cat file <<EOF` prints the
        // file, `cat - file <<EOF` prints body then file) and it
        // cannot honor option flags. Every cat the plan can fetch
        // (the first command and, in the sequence arm, every top-level
        // command) must therefore carry no operands beyond a sole
        // bare `-`, cat's name for the stdin the heredoc supplies.
        // The pipeline arm models at most [cat, sort -u]; a third
        // stage would be silently dropped, so longer pipelines
        // decline too. Declining only costs the shortcut — the real
        // executor produces GNU's output for every declined shape.
        let cat_operands_admissible = |command: &CommandNode| {
            if command.words.first().map(String::as_str) != Some("cat") {
                return true;
            }
            let operands = &command.words[1..];
            operands.is_empty() || (operands.len() == 1 && operands[0] == "-")
        };
        if !cat_operands_admissible(first)
            || ast
                .commands
                .iter()
                .any(|command| !cat_operands_admissible(command))
        {
            return None;
        }
        if ast
            .commands
            .first()
            .and_then(|command| command.pipeline_command.as_ref())
            .is_some_and(|pipeline| pipeline.stages.len() > 2)
        {
            return None;
        }
        if emit_warnings && (closed_by_paren || command_has_warned_heredoc(first)) {
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
        Some(HeredocComsubPlan { ast })
    }

    pub(in crate::executor) fn command_substitution_heredoc_output_mut_typed(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<SubstitutionOutput> {
        let output = self.heredoc_comsub_typed_output(source)?;
        self.last_command_substitution_status.set(Some(0));
        Some(SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
            0,
            context,
        ))
    }

    fn heredoc_comsub_typed_output(&mut self, source: &str) -> Option<String> {
        let plan = self.heredoc_comsub_plan(source, true);
        let result = match &plan {
            Some(plan) => heredoc_comsub_produce(plan, &mut |command| {
                self.stdin_string_for_command_mut(command)
            }),
            None => None,
        };
        // Decline-only memo (see the field doc on
        // heredoc_admission_declined_source): when this typed attempt
        // declines, the readback retry of the same source inside the
        // same expansion episode skips a second admission parse; a
        // success clears any stale record.
        *self.heredoc_admission_declined_source.borrow_mut() = if result.is_none() {
            Some(heredoc_admission_memo_key(source))
        } else {
            None
        };
        result
    }

    pub(in crate::executor) fn command_substitution_heredoc_output(
        &self,
        source: &str,
    ) -> Option<String> {
        {
            let mut memo = self.heredoc_admission_declined_source.borrow_mut();
            if memo.as_deref() == Some(heredoc_admission_memo_key(source).as_str()) {
                // A typed attempt already declined this exact source in
                // this expansion episode: fall through to the real
                // executor without re-running the admission parse.
                return None;
            }
            // An unrelated heredoc attempt ends the recorded episode.
            *memo = None;
        }
        let Some(plan) = self.heredoc_comsub_plan(source, false) else {
            *self.heredoc_admission_declined_source.borrow_mut() =
                Some(heredoc_admission_memo_key(source));
            return None;
        };
        heredoc_comsub_produce(&plan, &mut |command| self.stdin_string_for_command(command))
    }

    /// Shared byte-read ladder for the two `cat` file-operand comsub
    /// arms (the word-level arm in command_substitution.rs and the
    /// `time` arm in this file): registry first, in the quick-read /
    /// external_cat order (rubash#1917) — synthetic /proc content has
    /// no filesystem file behind the mapped path on Windows, and a
    /// SyntheticLink operand (rubash#1900) resolves through the
    /// registry to its native target (the session cwd answers for
    /// self/cwd). Only the byte SOURCE is shared here: what a failed
    /// or non-UTF-8 read means is each arm's own policy (the word-level
    /// arm diagnoses and sets status 1, rubash#772; the timed arm
    /// silently skips and keeps status 0) and stays in the arms.
    pub(in crate::executor) fn comsub_cat_operand_bytes(
        &self,
        path: &str,
    ) -> std::io::Result<Vec<u8>> {
        if let Some(bytes) = crate::special_paths::synthetic_proc_content(path) {
            Ok(bytes)
        } else if let Some(link) = crate::special_paths::synthetic_link_target(
            path,
            Some(&crate::proc_vfs::self_cwd_forms(self.session_cwd())),
        ) {
            fs::read(&link.native)
        } else {
            // niubash#177: same operand resolution as the argv funnel —
            // a virtual-root file operand may exist only under its `.exe`
            // spelling on disk.
            fs::read(
                self.session_path(crate::executor::path::windows_operand_file_path(
                    shell_path_to_windows(path, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                )),
            )
        }
    }

    /// An fd-0 `cat` operand in a comsub shortcut reads the shared
    /// FUNCTION_STDIN cursor: record the writeback both cat arms owe
    /// (the child drains the stream from the cursor onward) and return
    /// the remaining text.
    pub(in crate::executor) fn comsub_cat_function_stdin_text(&self) -> String {
        if let Some(text) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
            self.comsub_stdin_writeback
                .set(Some((text.len(), Self::function_stdin_fingerprint(text))));
        }
        self.function_stdin_remaining().unwrap_or_default()
    }

    pub(in crate::executor) fn timed_command_substitution_output(
        &self,
        words: &[String],
    ) -> Option<String> {
        if words.first().map(String::as_str) != Some("time") {
            return None;
        }
        // rubash#1218: posix mode + dash word — not the time keyword;
        // decline the shortcut so the body runs as an ordinary command.
        // The live mode is correct HERE (unlike the node-based sites,
        // rubash#1520): these words were split from the substitution
        // body text at this moment, so this body's parse time is now.
        if posix_time_dash_word(words, self.posix_mode_enabled()) {
            return None;
        }

        // rubash#1548: the operand window follows parse.y
        // special_case_tokens — at most one `-p`, then at most one
        // `--`; a `-p` beyond the window is the timed command's name
        // and must not be skipped (or counted for the report format).
        // Nested `time` words stay transparent, as before. These words
        // were split from the substitution body text, so the reading
        // is value-based. The window's `!` case has no arm here
        // (redundancy R7): this shortcut's only caller sits behind
        // command_substitution_body_is_trivial, whose byte set excludes
        // `!`, so an inverted timed body always takes full execution.
        let mut index = 1;
        let mut posix_format = false;
        let mut seen_opt = false;
        let mut seen_ign = false;
        while let Some(word) = words.get(index).map(String::as_str) {
            match word {
                "time" => index += 1,
                "-p" if !seen_opt && !seen_ign => {
                    seen_opt = true;
                    posix_format = true;
                    index += 1;
                }
                "--" if !seen_ign => {
                    seen_ign = true;
                    posix_format = true;
                    index += 1;
                }
                _ => break,
            }
        }

        let started = time_command_started();
        let output = self.timed_command_substitution_inner(&words[index..])?;
        // &self context: the executor's stderr endpoint needs &mut, so this
        // site keeps the raw stream write. A command substitution binds only
        // stdout — its stderr IS the shell's stderr in practice — so the
        // item-6 posix routing has no observable difference to close here.
        print_time(&self.shell_state.env_vars, posix_format, started);
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        self.last_command_substitution_status.set(Some(status));
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
            Some("tty") | Some("/bin/tty") | Some("/usr/bin/tty") => {
                if words.first().map(String::as_str) == Some("tty")
                    && self.emulated_external_stands_down("tty")
                {
                    return None;
                }
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
                // When cat does not resolve (PATH unset, rubash#1126; or
                // a set PATH without it, rubash#1545), decline.
                if self.emulated_external_stands_down("cat") {
                    return None;
                }
                // Bare `cat` (or flag/`-` operands) reads fd 0 — the shared
                // FUNCTION_STDIN cursor — which this shortcut cannot model;
                // fall back to real execution (external_cat owns it).
                if words.len() <= 1 || words[1..].iter().any(|word| word.starts_with('-')) {
                    return None;
                }
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    // `/dev/fd/N` (N>0) needs the real descriptor table —
                    // full execution owns it. fd 0 is the FUNCTION_STDIN
                    // cursor when that channel exists; without it fd 0 is
                    // the inherited process stdin, which full execution
                    // drains (subst.c:7143).
                    if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&path) {
                        if fd != 0 || !self.shell_state.env_vars.contains_key(FUNCTION_STDIN) {
                            return None;
                        }
                        output.push_str(&self.comsub_cat_function_stdin_text());
                        continue;
                    }
                    // Byte source: the ladder shared with the word-level
                    // cat arm (comsub_cat_operand_bytes). This arm's own
                    // semantics are preserved exactly: the bytes must be
                    // valid UTF-8 and a failed read is silently skipped,
                    // status stays 0 — unlike the word-level arm, which
                    // keeps raw bytes as content and diagnoses failures
                    // (that fork is deliberate and stays unfused; see T2
                    // in the redundancy audit).
                    if let Ok(bytes) = self.comsub_cat_operand_bytes(&path) {
                        if let Ok(value) = String::from_utf8(bytes) {
                            output.push_str(&value);
                        }
                    }
                }
                self.last_command_substitution_status.set(Some(0));
                Some(output.trim_capture_terminator().to_string())
            }
            Some(_) => self.run_external_command_substitution(words),
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

/// rubash#320 admission predicate for the `cat` heredoc comsub shortcut:
/// GNU binds file redirections (and fd duplications) before the command
/// runs (redir.c do_redirection_internal), so `cat <<EOF > /dev/null`
/// captures nothing. The shortcut only models heredoc/here-string input
/// (`<<`, `<<-`, `<<<` operators — input plumbing), so any other redirect
/// operator disqualifies the command and it falls through to the real
/// executor.
fn command_has_file_redirect(command: &CommandNode) -> bool {
    command
        .redirects
        .iter()
        .any(|r| !r.operator.starts_with("<<"))
        || command.redirect_in.is_some()
        || command.redirect_out.is_some()
        || command.append.is_some()
        || command.redirect_err.is_some()
        || command.redirect_err_append.is_some()
}
