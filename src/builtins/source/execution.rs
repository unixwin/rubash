use crate::executor::{ExecuteError, Executor};
use crate::parser::CommandNode;

pub fn execute_text(executor: &mut Executor, source: &str) -> Result<(), ExecuteError> {
    execute_text_with_args(executor, source, &[])
}

pub fn execute_text_with_args(
    executor: &mut Executor,
    source: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    execute_source_with_args(executor, source, args, None, None)
}

pub(super) fn execute_text_maybe_redirected(
    executor: &mut Executor,
    source: &str,
    args: &[String],
    redirect_cmd: Option<&CommandNode>,
    source_name: Option<&str>,
) -> Result<(), ExecuteError> {
    if let Some(redirect_cmd) = redirect_cmd {
        // GNU execute_cmd.c applies the command's redirections once for the
        // whole sourced text (redir.c do_redirection_internal). The outer
        // scope opens/anchors the targets; each executed group then carries
        // the inherited per-leaf redirects like the script driver.
        return executor.with_compound_output_redirects(redirect_cmd, |executor| {
            execute_source_with_args(executor, source, args, source_name, Some(redirect_cmd))
        });
    }
    execute_source_with_args(executor, source, args, source_name, None)
}

/// GNU builtins/evalfile.c source_file -> evalstring.c parse_and_execute:
/// sourced text is read and executed one complete command group at a time,
/// so `alias`/`shopt -s expand_aliases` take effect for later groups — a
/// whole-file pre-parse would leave group-internal aliases (like modernish's
/// `forever do ... done` inside function bodies) unexpanded. Each group's
/// text is alias-expanded with the live table before parsing, and the
/// STREAMED marker suspends executor-level re-expansion, matching
/// run_history_group's contract.
fn run_source_groups(
    executor: &mut Executor,
    source: &str,
    redirect_cmd: Option<&CommandNode>,
) -> Result<(), ExecuteError> {
    let raw_lines: Vec<&str> = source.split_inclusive('\n').collect();
    let mut index = 0usize;
    let mut ran_any = false;
    while let Some((pending, start_line, _group_lines, feeder_tokens)) =
        crate::script_driver::read_next_source_group(executor, &raw_lines, &mut index)
    {
        let line_offset = start_line.saturating_sub(1);
        let pre_alias_text = pending.clone();
        let exec_text = crate::script_driver::expand_group_aliases(executor, &pending);
        if exec_text.trim().is_empty() {
            continue;
        }
        let parse_posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
        // A group that ran to EOF while still incomplete (unclosed quote,
        // unterminated compound) needs run_source_with_line_offset's
        // unclosed-input diagnostics and prefix execution.
        if !index_complete_group(&exec_text, parse_posix) {
            // The incomplete group's text is still the output of
            // expand_group_aliases above — aliases already ran at stream
            // level (parse.y:3249 alias_expand_token fires while reading,
            // complete or not), so the same __RUBASH_ALIAS_STREAMED
            // contract as the complete branch applies: executor-level word
            // expansion must not fire a second time over pre-expanded text
            // (`alias let='let --'` inside a sourced module's $( ... )
            // became `let -- --' and killed modernish _IN/sig, rubash#282).
            let old_streamed = executor
                .shell_state
                .env_vars
                .get("__RUBASH_ALIAS_STREAMED")
                .cloned();
            executor
                .shell_state
                .env_vars
                .insert("__RUBASH_ALIAS_STREAMED".to_string(), "1".to_string());
            let status = crate::script_driver::run_source_with_line_offset(
                executor,
                &exec_text,
                false,
                line_offset,
                redirect_cmd,
                if exec_text == pre_alias_text {
                    None
                } else {
                    Some(pre_alias_text.as_str())
                },
            );
            match old_streamed {
                Some(value) => executor
                    .shell_state
                    .env_vars
                    .insert("__RUBASH_ALIAS_STREAMED".to_string(), value),
                None => executor
                    .shell_state
                    .env_vars
                    .remove("__RUBASH_ALIAS_STREAMED"),
            };
            ran_any = true;
            let parse_error = executor.take_parse_error();
            if parse_error {
                executor.set_exit_code(status);
                return Ok(());
            }
            continue;
        }
        // perf10: reuse the gather feeder's committed tokens when the gather
        // certified them equivalent to a fresh whole-group re-lex
        // (read_next_source_group's doc: non-alias arm — exec_text is the
        // pending verbatim there —, complete break, no effective extglob
        // toggle). GNU anchor: parse.y:3557 read_token streams input once
        // and never re-tokenizes consumed text; the previous unconditional
        // `tokenize_with_initial_posix(&exec_text, ...)` re-read every byte
        // the feeder had just scanned (perf7: ~1 s of `. ./nvm.sh`, huq
        // 16009 calls vs 5764 parse-only). The pop applies the exact rule
        // `tokenize_comsub_body_with_origin` applies to the fresh stream
        // (the feeder's committed output ends with the per-logical-line
        // separator); position/column/leading_ws/heredoc_end_line semantics
        // are identical because both streams are produced by the same
        // feeder code from the same pushes.
        let mut tokens = match feeder_tokens {
            Some(mut tokens) => {
                if tokens
                    .last()
                    .is_some_and(|token| token.kind == crate::lexer::TokenKind::Semicolon)
                {
                    tokens.pop();
                }
                tokens
            }
            None => crate::lexer::tokenize_with_initial_posix(&exec_text, parse_posix),
        };
        if let Some(line) = crate::lexer::heredoc_overflow_line() {
            executor.mark_parse_error();
            eprintln!(
                "{}maximum here-document count exceeded",
                executor.parser_diagnostic_prefix_for_line(line)
            );
            executor.set_exit_code(2);
            return Ok(());
        }
        if line_offset != 0 {
            for token in &mut tokens {
                token.position += line_offset;
                token.column += line_offset;
            }
        }
        // GNU builtins/evalfile.c:296: a sourced file runs through
        // parse_and_execute — the SAME parser as the top-level driver, not a
        // lenient reparse. A stray `)` / `;;` at command position is a syntax
        // error that aborts the remaining sourced text (rubash#203: `echo )`
        // was silently accepted, the argument swallowed, rc=0, and the rest
        // of the file still ran). parse.y yyerror reports `syntax error near
        // unexpected token `)''; the group's pre-alias text is supplied so
        // the diagnostic can echo the offending physical line
        // (parse.y:6866 print_offending_line).
        let mut ast = crate::parser::parse_with_options(
            &tokens,
            crate::parser::ParseLoopOptions {
                stray_close_is_error: true,
                source_text: Some(exec_text.as_str().into()),
                diagnostic_text: if exec_text == pre_alias_text {
                    None
                } else {
                    Some(pre_alias_text.as_str().into())
                },
                source_line_offset: line_offset,
            },
        );
        if let Some(cmd) = redirect_cmd {
            executor.apply_inherited_command_output_redirects(cmd, &mut ast)?;
        }
        // The group's words are final: executor-level alias expansion would
        // expand them a second time (run_history_group precedent). Save the
        // marker so a nested `.` inside an already-streamed context restores
        // the caller's value instead of clearing it.
        let old_streamed = executor
            .shell_state
            .env_vars
            .get("__RUBASH_ALIAS_STREAMED")
            .cloned();
        executor
            .shell_state
            .env_vars
            .insert("__RUBASH_ALIAS_STREAMED".to_string(), "1".to_string());
        let group_result = executor.execute_ast(&ast);
        match old_streamed {
            Some(value) => executor
                .shell_state
                .env_vars
                .insert("__RUBASH_ALIAS_STREAMED".to_string(), value),
            None => executor
                .shell_state
                .env_vars
                .remove("__RUBASH_ALIAS_STREAMED"),
        };
        ran_any = true;
        let parse_error = executor.take_parse_error();
        match group_result {
            Ok(()) if !parse_error => {}
            Ok(()) => {
                // GNU evalstring.c:585-606: a syntax error aborts the
                // remaining file; `.` itself returns 2.
                executor.set_exit_code(2);
                return Ok(());
            }
            Err(ExecuteError::ExitCode(code)) if parse_error => {
                executor.set_exit_code(code);
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    // GNU builtins/source.def: the return status of `.` is the exit status
    // of the last command executed in the file, or **zero** when no commands
    // run (builtins.tests sources a zero-length file and expects $? == 0).
    if !ran_any {
        executor.set_exit_code(0);
    }
    Ok(())
}

/// Whether the accumulated group text ended syntactically complete. At EOF
/// an incomplete tail means the file itself ends inside an unclosed
/// construct, which is the run_source_with_line_offset unclosed-input path.
fn index_complete_group(text: &str, posix: bool) -> bool {
    !crate::lexer::has_unclosed_input_syntax_posix(text, posix)
        && !crate::script_driver::stdin_source_needs_more_posix(text, posix)
}

fn execute_source_with_args(
    executor: &mut Executor,
    source: &str,
    args: &[String],
    source_name: Option<&str>,
    redirect_cmd: Option<&CommandNode>,
) -> Result<(), ExecuteError> {
    let old_positional_params = executor.positional_params();
    let source_positional_params: Vec<String> = args.to_vec();
    let had_source_args = !source_positional_params.is_empty();
    let old_source_marker = executor.get_env("__RUBASH_IN_SOURCE").map(str::to_string);
    let old_script_name = executor.get_env("__RUBASH_SCRIPT_NAME").map(str::to_string);
    let old_bash_argv0 = executor.get_env("BASH_ARGV0").map(str::to_string);
    executor.set_env("__RUBASH_IN_SOURCE", "1");
    if executor.get_env("__RUBASH_TOP_LEVEL_NAME").is_none() {
        let top_level_name = old_bash_argv0
            .as_deref()
            .or(old_script_name.as_deref())
            .unwrap_or("rubash");
        executor.set_env("__RUBASH_TOP_LEVEL_NAME", top_level_name);
    }
    let old_current_line = executor
        .get_env("__RUBASH_CURRENT_LINE")
        .map(str::to_string);
    if let Some(source_name) = source_name {
        // GNU builtins/evalfile.c:253-257 pushes a "source" frame for a
        // sourced file: BASH_SOURCE += filename, BASH_LINENO += the source
        // command's line, FUNCNAME += "source". The frame is popped (and
        // line_number restored) by run_unwind_frame before source_file's
        // run_return_trap (evalfile.c:395), which is why the RETURN trap at
        // sourced-file exit reports the sourcer's context with the source
        // call line (dbg-support.tests: "return lineno: 59 fn3").
        let source_call_line = executor
            .get_env("__RUBASH_CURRENT_LINE")
            .unwrap_or("0")
            .to_string();
        executor.push_source_call_frame(source_call_line);
        executor.push_bash_source(source_name.to_string());
        if let Some(top_level_name) = old_script_name.as_deref().or(old_bash_argv0.as_deref()) {
            executor.set_env("BASH_ARGV0", top_level_name);
        }
        executor.set_env("__RUBASH_SCRIPT_NAME", source_name);
    }
    if had_source_args {
        executor.set_positional_params(source_positional_params.clone());
    }

    let old_dollar_vars_changed = executor.shell_state.dollar_vars_changed_by_set;
    executor.shell_state.dollar_vars_changed_by_set = false;
    // GNU builtins/source.def:208-216 unsets the DEBUG trap for the duration
    // of a sourced file when function_trace_mode is off; the unwind-protect
    // restores it only after source_file's run_return_trap (evalfile.c:395),
    // so the sourced file's top-level commands and the RETURN-trap action's
    // own DEBUG fire are suppressed together (dbg-support.tests:98 emits
    // only `debug lineno: 98 main`, no fires inside dbg-support.sub).
    let functrace = crate::builtins::set::shell_option_enabled(&executor.env_vars(), "functrace");
    let old_source_debug_suppressed = executor.source_debug_suppressed();
    if !functrace {
        executor.set_source_debug_suppressed(true);
    }
    let result = run_source_groups(executor, source, redirect_cmd);

    if source_name.is_some() {
        // evalfile_internal's run_unwind_frame pops the "source" frame and
        // restores line_number before source_file runs the RETURN trap.
        executor.pop_bash_source();
        executor.pop_source_call_frame();
        match &old_current_line {
            Some(line) => executor.set_env("__RUBASH_CURRENT_LINE", line),
            None => executor.remove_env("__RUBASH_CURRENT_LINE"),
        }
    }

    // Restore the sourcer's shell state while __RUBASH_IN_SOURCE is still
    // set, so set_env's top-level BASH_SOURCE rebinding stays skipped and
    // the sourcer's BASH_SOURCE frames survive (GNU evalfile_internal's
    // run_unwind_frame restores its own state without touching the
    // sourcer's stack). The in-source marker itself must be removed last.
    match old_script_name {
        Some(value) => executor.set_env("__RUBASH_SCRIPT_NAME", &value),
        None => executor.remove_env("__RUBASH_SCRIPT_NAME"),
    }
    match old_bash_argv0 {
        Some(value) => executor.set_env("BASH_ARGV0", &value),
        None => executor.remove_env("BASH_ARGV0"),
    }
    match old_source_marker {
        Some(value) => executor.set_env("__RUBASH_IN_SOURCE", &value),
        None => executor.remove_env("__RUBASH_IN_SOURCE"),
    }

    // GNU Bash runs the RETURN trap when a sourced script finishes
    // (builtins/evalfile.c source_file: run_return_trap after
    // evalfile_internal). Inside a function that does not inherit the
    // RETURN trap it was already restored to default (execute_cmd.c:5295),
    // so nothing fires (dbg-support.tests:96/97 have no `return lineno`
    // inside the functrace-off fn3, while the top-level tests:98 exit
    // still reports `return lineno: 98 main`). The DEBUG suppression above
    // stays active through the trap action itself, matching GNU's
    // unwind-protect ordering.
    if executor.return_trap_in_scope() {
        executor.run_return_trap()?;
    }
    executor.set_source_debug_suppressed(old_source_debug_suppressed);

    if had_source_args {
        // GNU source.def uw_maybe_pop_dollar_vars: when the sourced script
        // reassigned the dollar vars through the set builtin and we are not
        // inside a shell function, the new values stay and the saved copy is
        // discarded; otherwise the saved positionals are restored.
        if executor.shell_state.dollar_vars_changed_by_set
            && executor.shell_state.function_depth == 0
        {
            // keep the sourced script's new positionals
        } else {
            executor.set_positional_params(old_positional_params);
        }
    }
    executor.shell_state.dollar_vars_changed_by_set = old_dollar_vars_changed;

    match result {
        Err(ExecuteError::Return(status)) => {
            executor.set_exit_code(status);
            Ok(())
        }
        // GNU builtins/evalstring.c:585-606 (parse_and_execute): a syntax
        // error aborts the remaining string (`break` — "syntax errors in a
        // script abort the execution of the script") and the caller gets
        // EX_BADUSAGE (shell.h:56 = 2) as an ORDINARY return value — no
        // longjmp, so the sourcing script continues (rubash#203). `exit N`
        // inside the file is a different unwinder (jump_to_top_level
        // EXITPROG) and must still propagate, hence the parse-error flag
        // gate rather than catching every ExitCode.
        Err(ExecuteError::ExitCode(status)) if executor.take_parse_error() => {
            executor.set_exit_code(status);
            Ok(())
        }
        other => other,
    }
}
