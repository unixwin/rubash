use super::*;
use crate::executor::markers::DATA_DOLLAR;
use crate::executor::path::shell_directory_entries;

impl Executor {
    /// Expands a command-substitution argument word. When the word was
    /// quoted in the source and starts with `~`, prefix the quote-protection
    /// marker so tilde expansion is skipped (Bash: `$(printf '%s' "~/repo")`
    /// prints `~/repo`, not the home directory).
    /// GNU bash runs brace expansion on each command word before any other
    /// expansion (subst.c expand_words -> brace expansion on the raw word).
    /// The single-command substitution shortcuts expand the raw words
    /// directly, so splice brace-expansion results in place, preserving the
    /// parent word's quote flag for the remaining per-word passes.
    fn brace_expanded_substitution_args(
        &self,
        words: &[String],
        word_parts: &[(String, bool)],
    ) -> Vec<String> {
        let mut expanded_args = Vec::new();
        for (index, word) in words[1..].iter().enumerate() {
            let quote = word_parts.get(index + 1).map(|(_, q)| *q);
            let unquoted = quote != Some(true);
            let braced = crate::expand::braces::expand_braces(word);
            if braced.len() > 1 {
                for item in braced {
                    let expanded = self.expand_protected_tilde(&item, quote);
                    // GNU subst.c expand_words runs pathname expansion on
                    // each brace-expanded word when the original was unquoted
                    // (`$(echo {a,b}*)` expands `a*` and `b*` separately).
                    if unquoted {
                        expanded_args.extend(
                            self.expand_command_substitution_arg_values_quoted(&item, false),
                        );
                    } else {
                        expanded_args.push(expanded);
                    }
                }
            } else {
                // GNU expand_words field-splits an unquoted expansion word on
                // $IFS (subst.c), so the substitution body's `echo $a` hands
                // echo one arg per IFS field, not the raw joined value
                // (nquote5.tests: `$(echo $a)` with IFS=$'\001'). A fully
                // quoted word (`echo "$a"`) is one field, never split.
                let expanded = self.expand_protected_tilde(word, quote);
                if unquoted && for_word_has_unquoted_expansion(word, None) {
                    let split = self.field_split_values(&expanded);
                    // Pathname expansion after field splitting (subst.c
                    // expand_words -> pathname expansion): each field is
                    // expanded independently. `$(echo *)` yields the
                    // directory listing, `$(echo $a)` yields the IFS fields.
                    for value in split {
                        expanded_args
                            .extend(self.apply_command_substitution_pathname_expansion(&value));
                    }
                } else if unquoted {
                    // No unquoted parameter expansion, but the word may still
                    // be a literal glob pattern (`echo *`, `echo *.sh`).
                    expanded_args
                        .extend(self.apply_command_substitution_pathname_expansion(&expanded));
                } else {
                    expanded_args.push(expanded);
                }
            }
        }
        expanded_args
    }

    fn expand_protected_tilde(&self, word: &str, was_quoted: Option<bool>) -> String {
        let expanded = if was_quoted == Some(true) && word.starts_with('~') {
            self.expand_word(&format!(
                "{}{word}",
                crate::executor::markers::QUOTED_WORD_PREFIX_STR
            ))
        } else {
            self.expand_word(word)
        };
        let unescaped = unescape_remaining_shell_escapes(&expanded);
        let protected = if command_substitution_value_needs_payload_protection(word, &unescaped) {
            protect_command_substitution_output(&unescaped)
        } else {
            unescaped
        };
        decode_command_substitution_payload(&restore_old_style_backtick_markers(&protected))
    }

    pub(in crate::executor) fn expand_command_substitution(&self, source: &str) -> String {
        self.expand_command_substitution_with_context(source, SubstitutionQuoteContext::Unquoted)
    }

    pub(in crate::executor) fn expand_command_substitution_with_context(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        self.last_command_substitution_status.set(Some(0));
        self.last_command_substitution_parse_error.set(false);
        // A command substitution is a subshell boundary: GNU runs the body
        // in a forked child (subst.c:7143 command_substitute ->
        // execute_cmd.c:1576 execute_in_subshell), so no mutation of shell
        // state — the arithmetic/bad-substitution latches (subst.c:10277),
        // subshell_level, or the DEBUG-trap command text — can reach the
        // parent. Fast-path builtins expand on this shared executor under
        // `&self`, so the boundary is expressed as the typed interior
        // snapshot/restore (issue #67: `x=$(echo $((b)))` under `set -u`
        // prints the diagnostic, leaves x empty, keeps running).
        let saved_state = self.shell_state.snapshot_interior();
        self.shell_state
            .subshell_depth
            .set(saved_state.subshell_depth() + 1);
        // execute_cmd.c:1576 execute_in_subshell marks SUBSHELL_COMSUB —
        // start_job (jobs.c:3837) refuses fg/bg inside it.
        self.shell_state.in_command_substitution.set(true);
        self.shell_state.parameter_bad_substitution.set(false);
        // Bash evaluates BASH_COMMAND in a command substitution against the
        // substitution's own command source, rather than the outer word.
        *self.shell_state.debug_trap_command.borrow_mut() = Some(source.trim().to_string());
        let result = self.expand_command_substitution_inner(source, context);
        self.shell_state.restore_interior(&saved_state);
        result
    }

    pub(in crate::executor) fn expand_command_substitution_readback_with_context(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> SubstitutionOutput {
        let output = self.expand_command_substitution_with_context(source, context);
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        // expand_command_substitution_with_context returns shell TEXT (marker
        // pairs for carrier/raw bytes). readback takes raw capture bytes, so
        // decode the markers once — into_bytes() would re-encode them as
        // literal PUA glyphs at the next bytes_to_shell_text boundary.
        SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
            status,
            context,
        )
    }

    pub(in crate::executor) fn expand_command_substitution_inner(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        // TODO(subst.c/parse.y/execute_cmd.c): Bash command substitution runs a
        // subshell, captures stdout, removes trailing newlines, and performs
        // full parsing/execution. This handles the alias4.sub form
        // `$(eval echo b)` so alias-expanded command substitutions participate
        // in word expansion.
        let source = source.trim();
        if source.is_empty() {
            self.last_command_substitution_status.set(Some(0));
            return String::new();
        }
        // NOTE: an earlier revision stripped a leading `eval ` here and ran
        // the remainder as a plain command. That breaks eval semantics for
        // multi-word strings: `x=$(eval "echo hi")` must re-parse the string
        // into two commands words, not execute a command named "echo hi"
        // (issue #69). Command substitutions that begin with `eval` now fall
        // through to the real parser/executor fallback below, which runs the
        // eval builtin with full re-parse semantics (alias4.sub
        // `$(eval echo b)` included).
        if let Some(inner) = strip_wrapping_subshell_group(source) {
            return self.expand_command_substitution_inner(inner, context);
        }
        if source == "false" {
            self.last_command_substitution_status.set(Some(1));
            return String::new();
        }
        if matches!(source, "true" | ":") {
            self.last_command_substitution_status.set(Some(0));
            return String::new();
        }
        if let Some(path) = source.strip_prefix('<') {
            let raw_path = path.trim();
            let allow_glob = !readfile_path_is_quoted(raw_path);
            let expanded = self.expand_word(raw_path);
            let path = strip_matching_quotes(&expanded);
            if let Some(path) = self.command_substitution_read_path(&path, allow_glob) {
                return fs::read_to_string(path)
                    .map(|value| {
                        self.last_command_substitution_status.set(Some(0));
                        value.trim_capture_terminator().to_string()
                    })
                    .unwrap_or_else(|_| {
                        self.last_command_substitution_status.set(Some(1));
                        String::new()
                    });
            }
            self.last_command_substitution_status.set(Some(1));
            return String::new();
        }
        // GNU trap.c reset_or_restore_signal_handlers (~1588): a command
        // substitution child keeps the DEBUG trap only when
        // function_trace_mode is set. When it is inherited, every inner
        // command's run_debug_trap must fire — with the trap output captured
        // into the substitution result — which none of the word-level
        // shortcuts below can express. Disqualify the whole shortcut family
        // and route the body through the real parser/executor, matching
        // subst.c:7143 command_substitute -> parse_and_execute.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "functrace")
            && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                .is_some_and(|action| !action.is_empty())
        {
            if let Some(output) = self.command_list_substitution_output(source, context) {
                return output;
            }
            return String::new();
        }
        if let Some(output) = self.command_substitution_cd_pwd_output(source) {
            return output;
        }
        if let Some(output) = self.command_substitution_heredoc_output(source) {
            return output;
        }
        // GNU applies alias expansion while reading the substitution body
        // (parse.y alias_expand_token + push_string): expand the body text
        // at stream level once here so the whitelist below judges the same
        // words the real parser would see — an alias body can inject
        // operators or quotes the raw source did not carry.
        let word_source = strip_command_substitution_comments(source);
        let word_source = self.comsub_body_alias_splice_extracted(&word_source);

        // rubash#117 whitelist admission: GNU subst.c:7143
        // command_substitute routes every body through parse_and_execute —
        // there are no word-level shortcuts upstream. A shortcut here is
        // only equivalent when the body is provably a single simple command
        // of literal words (no quoting, no expansion, no operators), where
        // parsing cannot change the outcome. Everything else falls through
        // to the real parser/executor: a false positive only costs speed,
        // while the blacklist guards this replaced kept producing silent
        // semantic bugs for the next uncovered character class.
        if !command_substitution_body_is_trivial(&word_source) {
            if let Some(output) = self.command_list_substitution_output(source, context) {
                return output;
            }
            return String::new();
        }

        let word_parts = split_shell_words_with_quote_info(&word_source);
        let words: Vec<String> = word_parts.iter().map(|(word, _)| word.clone()).collect();

        if let Some(output) = self.timed_command_substitution_output(&words) {
            return output;
        }

        if words.first().map(String::as_str) == Some("echo") {
            let expanded_args = self.brace_expanded_substitution_args(&words, &word_parts);
            return echo_command_substitution_output(&expanded_args);
        }

        if words.first().map(String::as_str) == Some("recho") {
            let expanded_args = self.brace_expanded_substitution_args(&words, &word_parts);
            return self
                .recho_output(&expanded_args)
                .trim_capture_terminator()
                .to_string();
        }

        if words.first().map(String::as_str) == Some("zecho") {
            let expanded_args = self.brace_expanded_substitution_args(&words, &word_parts);
            return self
                .zecho_output(&expanded_args)
                .trim_capture_terminator()
                .to_string();
        }

        if words.first().map(String::as_str) == Some("printf") {
            let expanded_args: Vec<String> = words[1..]
                .iter()
                .enumerate()
                .flat_map(|(index, word)| {
                    if let Some(values) = self.array_at_word_values(word) {
                        return values;
                    }
                    if let Some(values) = self.quoted_positional_at_word_values(word, None) {
                        return values;
                    }
                    let was_quoted = word_parts.get(index + 1).map(|(_, q)| *q);
                    let unquoted = was_quoted != Some(true);
                    let expanded =
                        strip_matching_quotes(&self.expand_protected_tilde(word, was_quoted))
                            .to_string();
                    // Same expand_words semantics as the echo/recho/zecho
                    // paths: unquoted expansion words split on $IFS, fully
                    // quoted words stay one field.
                    let values = if unquoted && for_word_has_unquoted_expansion(word, None) {
                        self.field_split_values(&expanded)
                    } else {
                        vec![expanded]
                    };
                    // Pathname expansion (subst.c expand_words): each
                    // unquoted field is expanded independently.
                    if unquoted {
                        values
                            .into_iter()
                            .flat_map(|v| self.apply_command_substitution_pathname_expansion(&v))
                            .collect::<Vec<_>>()
                    } else {
                        values
                    }
                })
                .collect();
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
            return bytes_to_shell_text(&stdout)
                .trim_capture_terminator()
                .to_string();
        }

        // The file-operand fast path must not claim a bare `cat` (or flag/
        // `-` operands): those read fd 0, which lives in FUNCTION_STDIN and
        // shares the caller's cursor — external_cat owns that semantics.
        if words.first().map(String::as_str) == Some("cat")
            && words[1..].iter().all(|word| !word.starts_with('-'))
            && words.len() > 1
        {
            let mut output = String::new();
            let mut status = 0;
            for word in &words[1..] {
                // Process substitution `<(...)`: Bash materializes it to a
                // temporary file holding the command's output, so `cat` reads
                // that output. Run the inner command directly instead of
                // treating the literal `<(...)` text as a file path.
                if let Some(source) = word
                    .strip_prefix("<(")
                    .and_then(|rest| rest.strip_suffix(')'))
                {
                    let mut executor = self.command_substitution_executor();
                    crate::builtins::trap::reset_for_subshell(&mut executor.shell_state.env_vars);
                    output.push_str(&executor.expand_command_substitution(source));
                    continue;
                }
                let path = self.expand_word(word);
                match fs::read_to_string(shell_path_to_windows(&path, &self.shell_state.env_vars)) {
                    Ok(value) => output.push_str(&value),
                    Err(_) => {
                        status = 1;
                        eprintln!("cat: '{}': No such file or directory", path);
                    }
                }
            }
            self.last_command_substitution_status.set(Some(status));
            return output.trim_capture_terminator().to_string();
        }

        if words.first().map(String::as_str) == Some("basename") {
            let Some(path) = words.get(1).map(|word| self.expand_word(word)) else {
                self.last_command_substitution_status.set(Some(1));
                return String::new();
            };
            let trimmed = path.trim_end_matches(['/', '\\']);
            let name = trimmed
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(trimmed);
            let suffix = words.get(2).map(|word| self.expand_word(word));
            let output = suffix
                .as_deref()
                .and_then(|suffix| name.strip_suffix(suffix))
                .unwrap_or(name);
            self.last_command_substitution_status.set(Some(0));
            return output.to_string();
        }

        if let Some(output) = self.command_describe_substitution_output(&words) {
            return output;
        }

        if words.first().map(String::as_str) == Some("umask") {
            return self
                .shell_state
                .env_vars
                .get("__RUBASH_UMASK")
                .cloned()
                .unwrap_or_else(|| "0022".to_string());
        }

        if words.first().map(String::as_str) == Some("ulimit") {
            return crate::builtins::ulimit::command_substitution(
                &words[1..],
                &self.shell_state.env_vars,
            );
        }

        if words.first().map(String::as_str) == Some("pwd") {
            if words.get(1).map(String::as_str) == Some("-P") {
                return std::env::current_dir()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
            }
            return self
                .shell_state
                .env_vars
                .get("PWD")
                .cloned()
                .unwrap_or_default();
        }

        if words.first().map(String::as_str) == Some("type")
            && words.get(1).map(String::as_str) == Some("-t")
            && words.len() >= 3
        {
            let mut subshell = self.command_substitution_executor();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Ok(status) = subshell.execute_type_with_io(&words[1..], &mut stdout, &mut stderr)
            {
                self.last_command_substitution_status.set(Some(status));
                return String::from_utf8_lossy(&stdout)
                    .trim_capture_terminator()
                    .to_string();
            }
        }

        if words.first().map(String::as_str) == Some("kill")
            && words.get(1).map(String::as_str) == Some("-l")
        {
            if let Some(word) = words.get(2) {
                // The spec may reference variables set by a running trap
                // action ($(kill -l $BASH_TRAPSIG) in trap9.sub), so expand
                // before translating; a literal that names no signal keeps
                // the historical empty-output behavior.
                let expanded = self.expand_word(word);
                if let Some(signal) = crate::builtins::kill::translate_signal(&expanded) {
                    self.last_command_substitution_status.set(Some(0));
                    return signal.to_string();
                }
                self.last_command_substitution_status.set(Some(1));
                return String::new();
            }
        }

        if words.first().map(String::as_str) == Some("mktemp") {
            if let Some(path) = self.mktemp_command_substitution(&words) {
                return path;
            }
        }

        if let Some(output) = self.run_external_command_substitution(&words) {
            return output;
        }

        if words.first().map(String::as_str) == Some("mktemp") {
            if let Some(path) = self.mktemp_command_substitution(&words) {
                return path;
            }
        }

        // Fallback: run the source through the real parser/executor in a
        // subshell and capture stdout (function calls, pipelines, compound
        // commands that the special-case dispatch above does not cover).
        if let Some(output) = self.command_list_substitution_output(source, context) {
            return output;
        }

        String::new()
    }

    pub(in crate::executor) fn command_substitution_cd_pwd_output(
        &self,
        source: &str,
    ) -> Option<String> {
        let (left, right) =
            split_unquoted_and_and(source).or_else(|| split_unquoted_semicolon(source))?;
        let right_words = split_shell_words(right.trim());
        if !matches!(right_words.as_slice(), [cmd] if cmd == "pwd")
            && !matches!(right_words.as_slice(), [cmd, option] if cmd == "pwd" && option == "-P")
        {
            return None;
        }

        let left_words = split_shell_words(left.trim());
        if left_words.first().map(String::as_str) != Some("cd") || left_words.len() > 2 {
            return None;
        }
        let target = if let Some(word) = left_words.get(1) {
            // GNU subst.c expands the cd target with expand_string (parameter,
            // tilde) but not pathname expansion: `cd` takes a single directory,
            // so a glob pattern would be a literal path, not a match list.
            self.expand_word(word)
        } else {
            self.home_value()
        };
        let target = shell_path_to_windows(&target, &self.shell_state.env_vars);
        let Ok(path) = fs::canonicalize(target) else {
            self.last_command_substitution_status.set(Some(1));
            return Some(String::new());
        };
        if !path.is_dir() {
            self.last_command_substitution_status.set(Some(1));
            return Some(String::new());
        }

        self.last_command_substitution_status.set(Some(0));
        let display = path.to_string_lossy().replace('\\', "/");
        Some(display.strip_prefix("//?/").unwrap_or(&display).to_string())
    }

    fn command_list_substitution_output_typed(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<SubstitutionOutput> {
        // The body was extracted from the current word; keep GNU's in-place
        // line counter so body diagnostics report the original script line
        // instead of restarting at 1 (subst.c comsub handling).
        let body_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let source = &self.comsub_body_alias_splice_extracted(source);
        let tokens = crate::lexer::tokenize_comsub_body(
            source,
            self.posix_mode_enabled(),
            body_start_line,
            true,
        );
        let ast = crate::parser::parse(&tokens);

        if ast.commands.iter().any(command_has_parse_error) {
            // GNU reports the body's syntax error (parse.y yyerror) even
            // though the enclosing command is what dies — emit the stored
            // diagnostic instead of silently swallowing it (case-pattern
            // `$(esac;x)` in comsub-posix6.sub).
            if let Some(command) = ast.commands.iter().find_map(command_parse_error_node) {
                self.report_command_parse_error(command);
            }
            self.last_command_substitution_parse_error.set(true);
            self.last_command_substitution_status.set(Some(2));
            return Some(SubstitutionOutput::readback(Vec::new(), 2, context));
        }

        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // The body was alias-expanded at stream level by
        // comsub_body_alias_splice above (parse.y alias_expand_token on the
        // fresh input); the child must not expand those words a second time.
        if self.alias_expansion_enabled() || self.posix_mode_enabled() {
            subshell
                .shell_state
                .env_vars
                .insert("__RUBASH_ALIAS_STREAMED".to_string(), "1".to_string());
        }
        // Trap mutation needs the Bash command-substitution trap lifecycle.
        // Keep the compatibility adjustment scoped to parsed bodies that
        // actually invoke trap, preserving specialized substitution modes.
        let has_trap_command = source.split_whitespace().any(|word| word == "trap");
        if has_trap_command {
            crate::builtins::trap::reset_for_subshell(&mut subshell.shell_state.env_vars);
        }
        // Keep the command source visible to BASH_COMMAND while the parsed
        // substitution body runs, including DEBUG trap actions.
        *subshell.shell_state.debug_trap_command.borrow_mut() = Some(source.trim().to_string());
        subshell.stdout_capture = Some(Vec::new());
        // GNU subst.c:7143 command_substitute: the substitution child's
        // stdout is the capture pipe, never the caller's fd 1. The forked
        // executor clones the parent's fd table, and since the compound
        // redirect binds landed there (4fe2a48f) an enclosing for/group
        // redirect surfaces as an fd-1 file binding that
        // apply_external_stdout_redirect delivered to external children —
        // their output bypassed the capture and landed in the outer
        // redirect target while the substitution read an empty pipe
        // (niubash shell-quirks Q16: inside
        // `for …; do out=$(cargo test 2>&1); …; done > summary.txt` the
        // failing round's output reached summary.txt and $out stayed
        // empty). Rebind fd 1 to the default Stdout endpoint — it resolves
        // to the active stdout_capture in write_fd_endpoint — rather than
        // removing the entry: in-shell writes consult fd_table[1] first and
        // a missing entry silently drops output. External children then hit
        // the stdout_capture pipe branch; body-level redirects rebind fd 1
        // on the child's own table. fd 2 stays inherited — $( ) does not
        // capture stderr (GNU subst.c:7149).
        subshell.fd_table.entries.insert(
            1,
            crate::executor::fd_table::FdEntry {
                read: None,
                write: Some(FdWriteEndpoint::Stdout),
                closed: false,
                dynamic: false,
            },
        );

        // GNU subst.c:7356-7359 command_substitute: without inherit_errexit
        // the substitution child runs `builtin_ignoring_errexit = 0` and
        // `change_flag ('e', FLAG_OFF)` — it clears the -e *flag itself*, so
        // an explicit `set -e` inside the body re-enables it (set-e.tests
        // `x=$(set -e; false; echo bad)` prints nothing). A suppression
        // counter would keep -e dead even after `set -e`. POSIX mode
        // enables inherit_errexit (set-e1.sub).
        let posix_mode = subshell
            .shell_state
            .env_vars
            .get("__RUBASH_POSIX_MODE")
            .map(String::as_str)
            == Some("1");
        let inherit_errexit = crate::builtins::shopt::option_enabled(
            &subshell.shell_state.env_vars,
            "inherit_errexit",
        );
        // Builtins inside the body that write the process stdout directly
        // consult the thread-local capture — which belongs to an enclosing
        // pipeline stage when this substitution runs inside one, leaking
        // the substitution's output into the stage's pipe. Give the body
        // its own thread-local capture and merge both buffers.
        let (captured, status) = crate::executor::shell_options::capture_stdout(|| {
            let result = if posix_mode || inherit_errexit {
                subshell.execute_ast(&ast)
            } else {
                subshell.suppress_errexit = 0;
                subshell.shell_state.env_vars.remove("__RUBASH_ERREXIT");
                crate::builtins::set::set_shell_option(
                    &mut subshell.shell_state.env_vars,
                    "errexit",
                    false,
                );
                subshell.execute_ast(&ast)
            };
            // GNU parse.y: a syntax error inside the substitution body is a
            // read-time failure of the ENCLOSING command — after this command
            // finishes the reader stops (`$( esac ; ...)` in a case pattern:
            // the `*)` arm prints, `echo we should not see this` is skipped).
            // The body ast carried a __RUBASH_PARSE_ERROR__ node past the
            // early command_has_parse_error screen, so propagate the child's
            // parse_error latch onto the parent's abort flag here.
            if subshell.parse_error_occurred {
                self.last_command_substitution_parse_error.set(true);
            }
            let mut status = command_substitution_result_status(result, subshell.exit_code);
            // Bash runs EXIT in the command-substitution child, so an
            // EXIT trap installed by the body contributes its output to
            // captured stdout.
            if has_trap_command {
                if let Ok(exit_status) = subshell.run_exit_trap_for_status(status) {
                    status = exit_status;
                }
            }
            status
        });
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&captured);

        // GNU subst.c:7143 command_substitute forks sharing the parent's
        // fd 0 — input the body consumed is gone for the caller too. The
        // child's cursor lives in its env clone; fold it back when both
        // sides still name the same FUNCTION_STDIN buffer.
        if let (Some(parent_input), Some(child_input)) = (
            self.shell_state.env_vars.get(FUNCTION_STDIN).cloned(),
            subshell.shell_state.env_vars.get(FUNCTION_STDIN).cloned(),
        ) {
            if parent_input == child_input {
                if let Some(child_offset) = subshell
                    .shell_state
                    .env_vars
                    .get(FUNCTION_STDIN_OFFSET)
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    self.comsub_stdin_writeback.set(Some((
                        child_offset,
                        Self::function_stdin_fingerprint(&parent_input),
                    )));
                }
            }
        }

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }

        let readback = SubstitutionOutput::readback(output, status, context);
        self.last_command_substitution_status
            .set(Some(readback.status));
        Some(readback)
    }

    /// Legacy String boundary for callers that still build AST words as text.
    fn command_list_substitution_output(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<String> {
        self.command_list_substitution_output_typed(source, context)
            .map(|output| output.text_lossy())
    }

    pub(in crate::executor) fn command_substitution_executor(&self) -> Executor {
        // The fork copy: every mutable shell datum arrives through the
        // cloned state boundary (execute_cmd.c:1576 execute_in_subshell /
        // subst.c:7143 command_substitute), so new semantic fields are
        // isolated automatically — no per-field list to maintain.
        let mut shell_state = self.shell_state.clone();
        // GNU forks carry the parent's loop_level, but a `break` in a real
        // child can only end the child. In-process the loop-break flag
        // would reach the parent's live loop, so the boundary resets it —
        // the same rule the flat `( )` region applies at entry.
        shell_state.loop_depth = 0;
        shell_state
            .subshell_depth
            .set(self.shell_state.subshell_depth.get() + 1);
        shell_state.in_command_substitution.set(true);
        // Mid-expansion error latches belong to the parent's in-flight word
        // expansion; the forked child starts with a clean slate (same as a
        // real fork, where no such rubash-internal latch exists).
        shell_state.arithmetic_expansion_error.set(false);
        shell_state.arithmetic_nonfatal_error.set(false);
        shell_state.arithmetic_fatal_error.set(false);
        shell_state.arithmetic_nounset_error.set(false);
        shell_state.arithmetic_last_error_category.set(None);
        shell_state.parameter_bad_substitution.set(false);
        // The substitution body is fresh parser input (subst.c:7143
        // command_substitute re-reads the collected text): GNU expands its
        // aliases at that read, so the driver's streamed-batch marker must
        // not reach the child's executor-level expansion.
        shell_state.env_vars.remove("__RUBASH_ALIAS_STREAMED");
        Executor {
            shell_state,
            fd_table: self.fd_table.clone(),
            exit_code: self.exit_code,
            parse_error_occurred: false,
            // GNU exit.def bash_logout: subshells never source ~/.bash_logout
            // (subshell_environment check); inherit the parent's latch so a
            // subshell logout cannot double-source it either.
            bash_logout_sourced: true,
            shell_pid: self.shell_pid,
            owns_signal_mailbox: false,
            arithmetic_last_error_expression: std::cell::RefCell::new(String::new()),
            arithmetic_last_eval_input: std::cell::RefCell::new(String::new()),
            assignment_command_name: None,
            buffer_assignment_diagnostics: false,
            pending_assignment_diagnostics: Vec::new(),
            parameter_assignment_failure: Cell::new(false),
            tempenv_names: Vec::new(),
            tempenv_marks: Vec::new(),
            tempenv_promoted_names: Vec::new(),
            tempenv_previous: HashMap::new(),
            tempenv_propagated_names: Vec::new(),
            function_tempenv_names: Vec::new(),
            evalerror_pending: Cell::new(false),
            evalerror_line: Cell::new(None),
            evalerror_exec_depth: Cell::new(0),
            reader_command_line: Cell::new(None),
            ambient_line: Cell::new(None),
            conditional_invert_pending: Cell::new(false),
            inside_compound_condition: Cell::new(false),
            inside_assignment_rhs: Cell::new(false),
            background_children: HashMap::new(),
            coproc_stderr_forwarders: HashMap::new(),
            assignment_output_process_substitutions: HashMap::new(),
            pending_scalar_assignment: false,
            suppress_errexit: self.suppress_errexit,
            debug_trap_running: false,
            return_trap_running: false,
            signal_trap_running: false,
            error_trap_running: false,
            sigchld_notifications_pending: std::cell::Cell::new(0),
            source_debug_suppressed: false,
            host_internal_depth: std::cell::Cell::new(self.host_internal_depth.get()),
            debug_trap_function_line: None,
            last_command_substitution_status: Cell::new(None),
            comsub_stdin_writeback: Cell::new(None),
            pipeline_stdin_consumed: Cell::new(None),
            last_heredoc_warning_source: RefCell::new(None),
            comsub_leading_newlines: Cell::new(0),
            current_shell_substitution_exit: Cell::new(self.current_shell_substitution_exit.get()),
            last_command_substitution_parse_error: Cell::new(false),
            last_command_inverted: Cell::new(false),
            exit_jump_pending: Cell::new(false),
            upstream_script_consumed: Cell::new(false),
            special_builtin_failed: Cell::new(false),
            last_builtin_write_failed: Cell::new(false),
            redirect_target_memo: RefCell::new(HashMap::new()),
            fd_var_external_undo: Vec::new(),
            read_deadline: None,
            read_timed_out: false,
            stdout_capture: None,
            stderr_capture: None,
            host_external_command_handler: None,
            #[cfg(windows)]
            elevation_handler: None,
            external_file_builtins_enabled: self.external_file_builtins_enabled,
            process_env_snapshot: self.process_env_snapshot.clone(),
            history_provider: self.history_provider.clone(),
        }
    }

    pub(in crate::executor) fn command_substitution_read_path(
        &self,
        path: &str,
        allow_glob: bool,
    ) -> Option<PathBuf> {
        if !allow_glob || !path.contains('*') || self.posix_mode_enabled() {
            return Some(shell_path_to_windows(path, &self.shell_state.env_vars));
        }

        let normalized = path.replace('\\', "/");
        let (dir, pattern) = normalized
            .rsplit_once('/')
            .map(|(dir, pattern)| (if dir.is_empty() { "/" } else { dir }, pattern))
            .unwrap_or((".", normalized.as_str()));
        let mut matches = shell_directory_entries(dir, &self.shell_state.env_vars)
            .ok()?
            .into_iter()
            .filter_map(|entry| case_pattern_matches(pattern, &entry.name).then_some(entry.path))
            .collect::<Vec<_>>();
        matches.sort();
        matches.into_iter().next()
    }
}

fn command_has_parse_error(command: &CommandNode) -> bool {
    command_parse_error_node(command).is_some()
}

fn command_parse_error_node(command: &CommandNode) -> Option<&CommandNode> {
    if command.has_assignment("__RUBASH_PARSE_ERROR__") {
        return Some(command);
    }
    command
        .and_or_list
        .as_ref()
        .and_then(|list| list.commands.iter().find_map(command_parse_error_node))
        .or_else(|| {
            command
                .pipeline_command
                .as_ref()
                .and_then(|pipeline| pipeline.stages.iter().find_map(command_parse_error_node))
        })
}

/// rubash#117 whitelist admission for the word-level command-substitution
/// shortcuts. GNU subst.c:7143 command_substitute sends every body through
/// parse_and_execute; a shortcut is only equivalent when the body is
/// provably a single simple command of literal words — no quoting
/// (`'` `"` `\` `` ` ``), no expansion (`$` `~` glob `[` `*` `?`), no
/// operators or redirections (`;` `|` `&` `<` `>` `(` `)` `{` `}`), no
/// comment/negation introducers (`#` `!`), and no control or non-ASCII
/// bytes (which can carry in-band markers). Anything else must reach the
/// real parser/executor.
fn command_substitution_body_is_trivial(source: &str) -> bool {
    let source = source.trim();
    !source.is_empty()
        && source.bytes().all(|byte| {
            matches!(byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
                | b' ' | b'\t'
                | b'_' | b'-' | b'+' | b'=' | b'.' | b',' | b'/' | b':' | b'@'
                | b'%' | b'^')
        })
}

fn strip_command_substitution_comments(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut single = false;
    let mut double = false;
    let mut comment = false;
    let mut boundary = true;
    let mut escaped = false;

    for ch in source.chars() {
        if comment {
            if ch == '\n' {
                comment = false;
                boundary = true;
                output.push(ch);
            }
            continue;
        }
        if escaped {
            escaped = false;
            boundary = false;
            output.push(ch);
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            output.push(ch);
            continue;
        }
        if !double && ch == '\'' {
            single = !single;
            boundary = false;
            output.push(ch);
            continue;
        }
        if !single && ch == '"' {
            double = !double;
            boundary = false;
            output.push(ch);
            continue;
        }
        if !single && !double && ch == '#' && boundary {
            comment = true;
            continue;
        }
        // GNU read_token: `#` starts a comment only at a token boundary —
        // after whitespace or a separator, not mid-word (`$#`, `a#b`).
        boundary = ch.is_whitespace()
            || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        output.push(ch);
    }

    output
}

fn restore_old_style_backtick_markers(value: &str) -> String {
    value
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
        .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
}

fn readfile_path_is_quoted(path: &str) -> bool {
    path.chars().any(|ch| matches!(ch, '\'' | '"' | '\\'))
}

fn command_substitution_result_status(result: Result<(), ExecuteError>, exit_code: i32) -> i32 {
    match result {
        Ok(()) => exit_code,
        Err(ExecuteError::Return(status)) => status,
        Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => status,
        Err(_) => 1,
    }
}
