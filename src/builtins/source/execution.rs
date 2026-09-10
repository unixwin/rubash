use crate::executor::{ExecuteError, Executor};
use crate::parser::{Ast, CommandNode};

pub fn execute_text(executor: &mut Executor, source: &str) -> Result<(), ExecuteError> {
    execute_text_with_args(executor, source, &[])
}

pub fn execute_text_with_args(
    executor: &mut Executor,
    source: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    let ast = parse_source_ast(source);
    execute_ast_with_args(executor, ast, args, None)
}

pub(super) fn execute_text_maybe_redirected(
    executor: &mut Executor,
    source: &str,
    args: &[String],
    redirect_cmd: Option<&CommandNode>,
    source_name: Option<&str>,
) -> Result<(), ExecuteError> {
    let mut ast = parse_source_ast(source);
    if let Some(redirect_cmd) = redirect_cmd {
        executor.apply_command_output_redirects(redirect_cmd, &mut ast)?;
    }
    execute_ast_with_args(executor, ast, args, source_name)
}

fn parse_source_ast(source: &str) -> Ast {
    let tokens = crate::lexer::tokenize(source);
    crate::parser::parse(&tokens)
}

fn execute_ast_with_args(
    executor: &mut Executor,
    ast: Ast,
    args: &[String],
    source_name: Option<&str>,
) -> Result<(), ExecuteError> {
    // GNU builtins/source.def: the return status of `.` is the exit status of
    // the last command executed in the file, or **zero** when no commands run
    // (builtins.tests sources a zero-length file and expects $? == 0). An
    // empty AST must therefore succeed without touching any shell state —
    // positional params, BASH_SOURCE frames, and traps stay exactly as the
    // sourcer left them.
    if ast.commands.is_empty() {
        executor.set_exit_code(0);
        return Ok(());
    }
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
    let old_current_line = executor.get_env("__RUBASH_CURRENT_LINE").map(str::to_string);
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

    let old_dollar_vars_changed = executor.dollar_vars_changed_by_set;
    executor.dollar_vars_changed_by_set = false;
    // GNU builtins/source.def:208-216 unsets the DEBUG trap for the duration
    // of a sourced file when function_trace_mode is off; the unwind-protect
    // restores it only after source_file's run_return_trap (evalfile.c:395),
    // so the sourced file's top-level commands and the RETURN-trap action's
    // own DEBUG fire are suppressed together (dbg-support.tests:98 emits
    // only `debug lineno: 98 main`, no fires inside dbg-support.sub).
    let functrace =
        crate::builtins::set::shell_option_enabled(&executor.env_vars(), "functrace");
    let old_source_debug_suppressed = executor.source_debug_suppressed();
    if !functrace {
        executor.set_source_debug_suppressed(true);
    }
    let result = executor.execute_ast(&ast);

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
        if executor.dollar_vars_changed_by_set && executor.function_depth == 0 {
            // keep the sourced script's new positionals
        } else {
            executor.set_positional_params(old_positional_params);
        }
    }
    executor.dollar_vars_changed_by_set = old_dollar_vars_changed;

    match result {
        Err(ExecuteError::Return(status)) => {
            executor.set_exit_code(status);
            Ok(())
        }
        other => other,
    }
}
