//! `export` and `readonly` attribute builtins.
//!
//! GNU Bash source ownership:
//! - builtins/setattr.def (`export_builtin`, `readonly_builtin`)

use std::io::{self, Write};

mod apply;
mod marks;
mod print;
pub(crate) mod value;

use crate::executor::builtin_diag::{Diag, Prolog};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::builtin_table_row;
use apply::{apply_export_arg, apply_readonly_arg};
use print::{print_exported, print_readonly};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
pub(crate) const EX_USAGE: i32 = 2;
/// The lib-level `Diag` prolog, mirroring
/// `builtins::declare::diag_prolog`: the same env cells the deleted
/// `value::diagnostic_prefix` read. The `rubash: ` fallback the old
/// prefix used is replaced by the prolog's `error_name` shape, the
/// same simplification declare took in Phase 4 (E1/E2 run script
/// files, where script + line are present).
fn diag_prolog(env_vars: &crate::shell::var_table::VarTable) -> Prolog {
    Prolog {
        interactive: env_vars.get("__RUBASH_INTERACTIVE").is_some(),
        shell_basename: env_vars
            .get("__RUBASH_SHELL_NAME")
            .cloned()
            .unwrap_or_else(|| "rubash".to_string()),
        error_name: "rubash".to_string(),
        script_file: env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .cloned()
            .or_else(|| std::env::var("__RUBASH_SCRIPT_NAME").ok()),
        line: env_vars
            .get("__RUBASH_CURRENT_LINE")
            .cloned()
            .or_else(|| std::env::var("__RUBASH_CURRENT_LINE").ok()),
    }
}

/// A diagnostic line that carries the prolog but NO builtin name —
/// the `error.c` report family (`err_badarraysub`, the unqualified
/// `warning:` lines). Recorded diagnostics are flushed first so the
/// line keeps its position relative to the standard
/// `{prolog}{name}:` skeleton lines queued on the same handle.
fn bare_diag_line(diag: &mut Diag, prolog: &Prolog, text: &str) {
    diag.flush_with(|| prolog.clone());
    let head = prolog.render();
    diag.sink().write_str(&format!("{head}{text}\n"));
}

/// The error.c:453 `err_readonly` prefix: `gnu_errfmt`-aware
/// (`{script}:{line}: ` with the option on), else the plain prolog
/// render. This is the deleted `value::readonly_error_prefix`,
/// re-homed on the `Diag` (the caller supplies the rendered prefix
/// to `Diag::err_readonly`).
fn readonly_error_prefix(env_vars: &crate::shell::var_table::VarTable, prolog: &Prolog) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        if crate::builtins::shopt::option_enabled(env_vars, "gnu_errfmt") {
            return format!("{script}:{line}: ");
        }
    }
    prolog.render()
}

const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
use crate::executor::types::COMPOUND_ASSIGNMENT_MARKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExportMode {
    Set,
    Unset,
}

pub fn export(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    let mut oob_word = None;
    let status = export_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
        &mut oob_word,
        None,
        &crate::shell::var_table::TempenvListingUnderlay::new(),
    )?;
    // No abort channel here (test-only path in practice): keep the bare
    // report_error diagnostic plus the failure status so a direct caller
    // still sees GNU-identical output.
    if let Some(word) = oob_word {
        let prolog = diag_prolog(env_vars);
        writeln!(stderr, "{}{word}: bad array subscript", prolog.render())?;
        return Ok(EXECUTION_FAILURE);
    }
    Ok(status)
}

/// GNU variables.c:5760 { "BASH_COMPAT", sv_shcompat }: a builtin operand
/// rebinding BASH_COMPAT re-syncs the compatNN shopt flags. The hook runs
/// on every rebinding — even a value-identical one (verified oracle:
/// repeated `export BASH_COMPAT=99` re-reports) — and its error carries no
/// command prefix and never fails the builtin.
fn sync_bash_compat_flags(
    env_vars: &mut crate::shell::var_table::VarTable,
    operands: &[&str],
    diag: &mut Diag,
    prolog: &Prolog,
) -> io::Result<()> {
    let binds = operands
        .iter()
        .any(|operand| crate::builtins::shopt::binds_bashcompat_operand(operand));
    if binds {
        if let Err(diagnostic) =
            crate::builtins::shopt::sync_compat_flags_from_bash_compat(env_vars)
        {
            bare_diag_line(diag, prolog, &diagnostic);
        }
    }
    Ok(())
}

/// Execute `readonly` with arguments after the command name.
pub fn readonly(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    let mut oob_word = None;
    let status = readonly_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
        None,
        &mut oob_word,
        None,
        &crate::shell::var_table::TempenvListingUnderlay::new(),
    )?;
    // Same direct-path diagnostic as export above (no Executor to abort).
    if let Some(word) = oob_word {
        let prolog = diag_prolog(env_vars);
        writeln!(stderr, "{}{word}: bad array subscript", prolog.render())?;
        return Ok(EXECUTION_FAILURE);
    }
    Ok(status)
}

pub(crate) fn export_with_io<'a, I, W>(
    args: I,
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut dyn crate::executor::builtin_diag::DiagSink,
    oob_word: &mut Option<String>,
    traced: Option<&mut Vec<(String, bool, String)>>,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    // The Diag is built here, over the caller's stderr: the executor
    // passes this command's own buffered stderr, so a diagnostic
    // lands in the command's redirection (A3-R1, rubash#2707), and
    // the flush below runs before the buffer is read.
    let row = builtin_table_row("export").expect("export row exists");
    let mut diag = Diag::new(row, stderr);
    let prolog = diag_prolog(env_vars);
    let args: Vec<String> = args.into_iter().map(str::to_string).collect();
    let mut mode = ExportMode::Set;
    let mut print = false;
    let mut array = false;
    let mut assoc = false;
    let mut func_export = false;
    // GNU setattr.def ATTROPTS "aAfnp" (setattr.def:112): the spec
    // carries no leading `+`, so `+` words are operands, never
    // options (`export +n v` reports `` `+n': not a valid
    // identifier ``, oracle-verified). The single BuiltinOpt machine
    // replaces the hand scan here (Stage-2 ④ Phase 4).
    let operands: Vec<&str> = {
        let mut opt = BuiltinOpt::new(row, &args, &mut diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    crate::builtins::help::print_builtin_help(row.name, stdout)?;
                    return Ok(EX_USAGE);
                }
                OptEvent::End => break,
                OptEvent::Opt(c) => match c {
                    b'n' => mode = ExportMode::Unset,
                    b'p' => print = true,
                    b'a' => array = true,
                    b'A' => assoc = true,
                    // export -f: mark function for export. The
                    // executor diverts -f invocations to its own
                    // function face (whole-word lookup) before this
                    // builtin runs; the flag only survives on direct
                    // lib entries, which keep the stub below.
                    b'f' => func_export = true,
                    _ => {}
                },
                OptEvent::OptArg(_, _) => {}
            }
        }
        // operands() borrows the args slice, not the machine, so the
        // names outlive it; drop the machine first to release the
        // &mut Diag borrow before reading the scan status.
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        drop(opt);
        if diag.status() != 0 {
            diag.usage();
            diag.flush_with(|| prolog.clone());
            return Ok(diag.status());
        }
        operands
    };

    // GNU setattr.def set_or_show_attributes: the listing branch runs only
    // when NO operand names remain (loptend == NULL, the `else` arm at
    // setattr.def:291-345). With names present the -p flag is ignored —
    // each name is marked/assigned silently (`export -p NAME` prints
    // nothing), exactly like readonly.
    if operands.is_empty() {
        let _ = print;
        print_exported(
            env_vars,
            env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1"),
            array,
            assoc,
            underlay,
            stdout,
        )?;
        return Ok(EXECUTION_SUCCESS);
    }

    // Handle export -f: mark functions for export
    if func_export {
        for arg in &operands {
            let func_name = arg.to_string();
            // Store the function name in BASH_FUNC_<name>%% environment variable
            // This is the standard way bash exports functions
            let env_key = format!("BASH_FUNC_{}%%", func_name);
            env_vars.insert(env_key, "() { :; }".to_string());
        }
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    // The traced-assignment sink is consumed by the first apply call;
    // hand each call a reborrow so later operands keep recording.
    let mut traced = traced;
    for arg in &operands {
        if apply_export_arg(
            arg,
            mode,
            array,
            assoc,
            env_vars,
            &mut diag,
            &prolog,
            oob_word,
            traced.as_deref_mut(),
        )? != EXECUTION_SUCCESS
        {
            status = EXECUTION_FAILURE;
        }
        // GNU setattr.def:240-258 delegates `-a/-A name=value` to a nested
        // declare_builtin, whose OOB DISCARDs out of the whole export
        // command (arrayfunc.c:804-809): later operands never run, and the
        // Executor aborts the list. Stop here like declare's assign loop.
        if oob_word.is_some() {
            return Ok(EXECUTION_FAILURE);
        }
    }

    sync_bash_compat_flags(env_vars, &operands, &mut diag, &prolog)?;
    diag.flush_with(|| prolog.clone());

    Ok(status)
}

pub(crate) fn readonly_with_io<'a, I, W>(
    args: I,
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut dyn crate::executor::builtin_diag::DiagSink,
    context_name: Option<&str>,
    oob_word: &mut Option<String>,
    traced: Option<&mut Vec<(String, bool, String)>>,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    // Same Diag-over-caller's-stderr shape as export_with_io above.
    let row = builtin_table_row("readonly").expect("readonly row exists");
    let mut diag = Diag::new(row, stderr);
    let prolog = diag_prolog(env_vars);
    let args: Vec<String> = args.into_iter().map(str::to_string).collect();
    let mut print = false;
    let mut array = false;
    let mut assoc = false;
    // GNU setattr.def:112 ATTROPTS "aAfnp" + :174-176: `-n' says to
    // remove the attribute from the remaining names — and for
    // readonly the removal itself is silently disallowed, so the
    // attribute is dropped from the request: an assignment still
    // binds (plainly), a name-only operand marks nothing, and an
    // already-readonly variable keeps its mark. The old code treated
    // `-n` as a no-op and marked readonly anyway (`readonly -n
    // rv=6` wrongly left `rv` readonly, oracle-verified).
    let mut undo = false;
    let operands: Vec<&str> = {
        let mut opt = BuiltinOpt::new(row, &args, &mut diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    crate::builtins::help::print_builtin_help(row.name, stdout)?;
                    return Ok(EX_USAGE);
                }
                OptEvent::End => break,
                OptEvent::Opt(c) => match c {
                    b'p' => print = true,
                    b'a' => array = true,
                    b'A' => assoc = true,
                    // The -f face is diverted by the executor
                    // (whole-word lookup) before this builtin runs.
                    b'f' => {}
                    b'n' => undo = true,
                    _ => {}
                },
                OptEvent::OptArg(_, _) => {}
            }
        }
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        drop(opt);
        if diag.status() != 0 {
            diag.usage();
            diag.flush_with(|| prolog.clone());
            return Ok(diag.status());
        }
        operands
    };

    // GNU setattr.def set_or_show_attributes: with operand names the -p
    // flag is ignored — the operand branch (setattr.def:174-291) only calls
    // set_var_attribute, which never prints. Only the nameless invocation
    // (the else arm at :291-345) lists; without names, -a/-A restrict the
    // printed set to readonly indexed/associative arrays.
    if operands.is_empty() {
        let _ = print;
        print_readonly(
            env_vars,
            array,
            assoc,
            env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1"),
            underlay,
            stdout,
        )?;
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    let mut traced = traced;
    for arg in &operands {
        if apply_readonly_arg(
            arg,
            array,
            assoc,
            undo,
            env_vars,
            &mut diag,
            &prolog,
            context_name,
            oob_word,
            traced.as_deref_mut(),
        )? != EXECUTION_SUCCESS
        {
            status = EXECUTION_FAILURE;
        }
        // Same nested-declare DISCARD stop as the export loop above.
        if oob_word.is_some() {
            return Ok(EXECUTION_FAILURE);
        }
    }
    sync_bash_compat_flags(env_vars, &operands, &mut diag, &prolog)?;
    diag.flush_with(|| prolog.clone());
    Ok(status)
}

#[cfg(test)]
#[path = "setattr_tests.rs"]
mod tests;
