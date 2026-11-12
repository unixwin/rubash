use super::{
    ARRAY_VARS, ASSOC_VARS, EXECUTION_FAILURE, EXECUTION_SUCCESS, NAMEREF_VARS, READONLY_VARS,
};
use crate::executor::builtin_diag::{Diag, DiagKind, Prolog};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::builtin_table_row;
use std::io::{self, Write};

/// GNU shell.h:76 `EX_UTILERROR = 263` — a Posix special builtin utility error
/// (> EX_SHERRBASE = 256). `unset_builtin` returns this when `posix_utility_error`
/// is set (readonly variable, invalid identifier, non-unsettable variable).
/// execute_cmd.c:4888 then sets `special_builtin_failed`, causing a
/// non-interactive POSIX shell to exit.
pub(crate) const EX_UTILERROR: i32 = 263;

/// GNU shell.h:71 `EX_USAGE = 258` — the pre-builtin_status usage error
/// status (> EX_SHERRBASE = 256). `unset_builtin` returns this for invalid
/// options; execute_cmd.c:4888 sets `special_builtin_failed`.
const EX_USAGE_ORIG: i32 = 258;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct UnsetOptions {
    functions: bool,
    variables: bool,
    nameref: bool,
}

/// The lib-level `Diag` prolog, mirroring the declare layer's
/// `PrologParts::Lib` composition: the same env cells the deleted
/// local `diagnostic_prefix` read.
fn diag_prolog(env_vars: &crate::shell::var_table::VarTable) -> Prolog {
    // The name cells derive from the single argv[0] carrier chain
    // (builtins::prefix, rubash#2791): error_name follows the
    // get_name_for_error chain via builtin_diag::error_report_name
    // (rubash#3355: the presence-based $0 step — an empty $0
    // prints an empty name, not the hard-coded "rubash" brand),
    // while shell_basename keeps the shared basename derivation
    // (the old cells printed the shell-name carrier VERBATIM as
    // the interactive name and a hard-coded "rubash" as the error
    // name, rubash#3256).
    let script_file = env_vars
        .get("__RUBASH_SCRIPT_NAME")
        .cloned()
        .or_else(|| std::env::var("__RUBASH_SCRIPT_NAME").ok());
    let get = |key: &str| env_vars.get(key).cloned();
    Prolog {
        interactive: env_vars.get("__RUBASH_INTERACTIVE").is_some(),
        shell_basename: crate::builtins::prefix::shell_basename(&get),
        error_name: crate::executor::builtin_diag::error_report_name(&script_file, env_vars),
        script_file,
        line: env_vars
            .get("__RUBASH_CURRENT_LINE")
            .cloned()
            .or_else(|| std::env::var("__RUBASH_CURRENT_LINE").ok()),
    }
}

/// Execute `unset` with arguments after the command name.
pub fn unset(args: &[String], env_vars: &mut crate::shell::var_table::VarTable) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    unset_with_stderr(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn unset_with_stderr<'a, I, W>(
    args: I,
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut dyn crate::executor::builtin_diag::DiagSink,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    // The Diag is built here, over the caller's stderr (the same
    // shape as setattr's with_io fns): the executor passes this
    // command's own buffered stderr, so a diagnostic lands in the
    // command's redirection (A3-R1, rubash#2707).
    let row = builtin_table_row("unset").expect("unset row exists");
    let mut diag = Diag::new(row, stderr);
    let args: Vec<String> = args.into_iter().map(str::to_string).collect();
    // GNU set.def unset_builtin scans with internal_getopt over
    // "fnv" (the row's spec); the single BuiltinOpt machine replaces
    // the hand scan here (Stage-2 ④ Phase 4). An invalid option
    // returns EX_USAGE_ORIG (258, set.def:855) — NOT the Diag's
    // EX_USAGE (2): the machine records the error, but the status
    // this builtin returns is the .def's own.
    let mut options = UnsetOptions::default();
    let operands: Vec<&str> = {
        let mut opt = BuiltinOpt::new(row, &args, &mut diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    // GNU set.def CASE_HELPOPT: the long help goes to
                    // stdout with the .def's EX_USAGE (258).
                    crate::builtins::help::print_builtin_help(row.name, stdout)?;
                    return Ok(EX_USAGE_ORIG);
                }
                OptEvent::End => break,
                OptEvent::Opt(c) => match c {
                    b'f' => options.functions = true,
                    b'v' => options.variables = true,
                    b'n' => options.nameref = true,
                    _ => {}
                },
                OptEvent::OptArg(_, _) => {}
            }
        }
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        drop(opt);
        if diag.status() != 0 {
            diag.usage();
            diag.flush_with(|| diag_prolog(env_vars));
            // GNU set.def:855 returns EX_USAGE (258 > EX_SHERRBASE).
            return Ok(EX_USAGE_ORIG);
        }
        operands
    };

    if options.functions && options.variables {
        diag.free(format_args!(
            "cannot simultaneously unset a function and a variable"
        ));
        diag.flush_with(|| diag_prolog(env_vars));
        return Ok(EXECUTION_FAILURE);
    }

    let mut status = EXECUTION_SUCCESS;
    for name in &operands {
        let name_status = unset_name(name, options, env_vars, &mut diag)?;
        if name_status != EXECUTION_SUCCESS {
            // Preserve error statuses > EX_SHERRBASE (256) so the executor can
            // set special_builtin_failed; fall back to EXECUTION_FAILURE.
            status = if name_status > 256 {
                name_status
            } else {
                EXECUTION_FAILURE
            };
        }
    }

    diag.flush_with(|| diag_prolog(env_vars));
    Ok(status)
}

fn unset_name(
    name: &str,
    options: UnsetOptions,
    env_vars: &mut crate::shell::var_table::VarTable,
    diag: &mut Diag,
) -> io::Result<i32> {
    if options.functions {
        return Ok(EXECUTION_SUCCESS);
    }

    // `unset name[subscript]`: bash only removes an element from an actual
    // array/associative variable. A subscript on an undeclared name is a
    // silent no-op (GNU exit 0); on a declared scalar it errors
    // "unset: <name>: not an array variable". Real array/assoc element
    // removal is handled upstream in the executor; this covers the
    // scalar/undeclared cases and the builtin-only diagnostic path.
    if let Some((base, _subscript)) = parse_unset_subscript(name) {
        let declared = env_vars.contains_key(base);
        let is_array = is_marked_variable(env_vars, ARRAY_VARS, base)
            || is_marked_variable(env_vars, ASSOC_VARS, base);
        if !declared || is_array {
            return Ok(EXECUTION_SUCCESS);
        }
        diag.free(format_args!("{base}: not an array variable"));
        return Ok(EXECUTION_FAILURE);
    }

    if !valid_identifier(name) {
        // GNU builtins/set.def:899-920: when neither -f nor -v was given, a
        // name that is not a valid identifier is treated as a potential
        // function name and unset silently (no diagnostic, success). Only
        // `unset -v NAME` reports sh_invalidid ("`NAME': not a valid
        // identifier") through the builtin_error prologue.
        if !options.variables {
            return Ok(EXECUTION_SUCCESS);
        }
        diag.error(DiagKind::InvalidId(name.to_string()));
        // GNU set.def:917-918 increments posix_utility_error → EX_UTILERROR.
        return Ok(EX_UTILERROR);
    }

    // GNU builtins/set.def:990-1010 (unset_builtin): `unset -v` of a nameref
    // follows the reference: a plain identifier cell unbinds the referenced
    // variable and keeps the nameref itself; an array-reference cell unbinds
    // the referenced element and keeps the nameref; a valueless or invalid
    // cell falls back to unbinding the nameref variable itself.
    let nameref_cell: Option<String> =
        if !options.nameref && is_marked_variable(env_vars, NAMEREF_VARS, name) {
            env_vars.get(name).cloned()
        } else {
            None
        };
    let (unset_name, _keep_nameref) = match nameref_cell {
        Some(ref cell) if valid_identifier(cell) => (cell.as_str(), true),
        Some(ref cell) if parse_unset_subscript(cell).is_some() => {
            // Element unbinding through the reference happens in the
            // executor path (execute_unset_with_stderr); the builtin-only
            // path keeps the nameref and reports success.
            return Ok(EXECUTION_SUCCESS);
        }
        _ => (name, false),
    };

    if is_unsettable_bash_variable(unset_name) {
        diag.free(format_args!("{unset_name}: cannot unset"));
        // GNU set.def:930-931 increments posix_utility_error → EX_UTILERROR.
        return Ok(EX_UTILERROR);
    }

    if is_marked_variable(env_vars, READONLY_VARS, unset_name) {
        diag.free(format_args!(
            "{unset_name}: cannot unset: readonly variable"
        ));
        // GNU set.def:965-966 increments posix_utility_error → EX_UTILERROR.
        return Ok(EX_UTILERROR);
    }

    // GNU builtins/set.def:1024 + variables.c:3807-3815 unbind_nameref:
    // under -n the variable is removed only when it is itself a nameref;
    // other names are a silent no-op (the readonly/non-unsettable checks
    // above still apply to them).
    if options.nameref && !options.functions && !is_marked_variable(env_vars, NAMEREF_VARS, name) {
        return Ok(EXECUTION_SUCCESS);
    }

    // Borrowed (no alloc): every consumer below takes `&str`, and the
    // owned cell (when the operand was a nameref) outlives the call.
    let unset_name: &str = unset_name;
    // GNU variables.c:5778 { "GLOBIGNORE", sv_globignore } ->
    // pathexp.c:507-515 setup_glob_ignore: unsetting GLOBIGNORE takes the
    // `v == 0` branch and turns dotfile matching OFF — the effect shares
    // the dotglob cell with the shopt, so the write lands in the same
    // option storage (it clobbers even an explicit `shopt -s dotglob`).
    if unset_name == "GLOBIGNORE" {
        let _ = crate::builtins::shopt::set_option(env_vars, "dotglob", false);
    }
    // GNU variables.c:6460-6483 sv_shcompat: `unset BASH_COMPAT` takes the
    // `v == 0` branch — default level, all compatNN flags off. The variable
    // stays removed (unlike the shopt path, nothing rebinds it).
    if unset_name == "BASH_COMPAT" {
        crate::builtins::shopt::clear_compat_flags(env_vars);
    }
    env_vars.remove(&unset_name);
    // GNU variables.c:5819/5919 sv_path -> phash_flush (rubash#1551):
    // unbinding PATH empties the command hash table too (verified
    // oracle: `unset PATH` leaves `hash` empty).
    if unset_name == "PATH" {
        crate::builtins::hash::flush_command_hash_table(env_vars);
    }
    // Process-env removal is existence-gated, not name-gated (#916/#917):
    // ordinary names were never mirrored by writes, but an inherited copy
    // predates the mirror and must be dropped so table-miss fallbacks do
    // not resurrect it. Runs before the unmark calls below.
    crate::executor::local_helpers::unset_process_env(&unset_name);
    // #916: one attrs.remove clears all SHELL_VAR bits instead of ten
    // attr_of + write_flag round-trips (ASSOC_128 string dropped inside).
    env_vars.remove_unset_marks(&unset_name);
    Ok(EXECUTION_SUCCESS)
}

pub(super) fn is_marked_variable(
    env_vars: &crate::shell::var_table::VarTable,
    key: &str,
    name: &str,
) -> bool {
    // Structured flags are authoritative (lazy strings are not maintained);
    // is_marked falls back to the string only for names with no flag entry,
    // which a lazy write can never leave behind.
    env_vars.is_marked(key, name)
}

pub(super) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Split `name[subscript]` into `(base, subscript)`, returning `None` when
/// the name is not a subscripted form or the base is not a valid identifier
/// (so genuinely invalid names still fall through to the `valid_identifier`
/// rejection above).
fn parse_unset_subscript(name: &str) -> Option<(&str, &str)> {
    let (base, rest) = name.split_once('[')?;
    let subscript = rest.strip_suffix(']')?;
    if !valid_identifier(base) {
        return None;
    }
    Some((base, subscript))
}

fn is_unsettable_bash_variable(name: &str) -> bool {
    matches!(name, "BASH_LINENO" | "BASH_SOURCE")
}
