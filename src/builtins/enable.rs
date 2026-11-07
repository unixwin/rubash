//! `enable` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/enable.def (`enable_builtin`)

use crate::builtins::dynamic;
use crate::executor::builtin_diag::Diag;
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;
use crate::shell::var_table::VarTable;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
/// GNU shell.h EX_BADUSAGE: usage errors exit 2.
const EX_USAGE: i32 = 2;

const DISABLED_BUILTINS: &str = "__RUBASH_DISABLED_BUILTINS";

pub(crate) fn execute_with_io<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    env_vars: &mut VarTable,
    diag: &mut Diag<'a>,
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let cwd = std::env::current_dir().unwrap_or_default();
    execute_with_io_in_dir(row, args, env_vars, &cwd, diag, stdout)
}

/// `enable` entry point with the shell's physical cwd.  Loadable objects
/// are resolved against the SHELL cwd, not the host process cwd — rubash
/// `cd` moves `ShellState::cwd` without calling `chdir` (see
/// `src/shell/state.rs`), while GNU resolves `make_absolute(filename,
/// ".")` against the process cwd because its `cd` really chdirs.
///
/// Stage-2 ④ Phase 2: the option scan is the (table row + `BuiltinOpt` +
/// `Diag`) triple (`internal_getopt (list, "adnpsf:")`,
/// builtins/enable.def:126-127); every diagnostic speaks through `diag`.
pub(crate) fn execute_with_io_in_dir<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    env_vars: &mut VarTable,
    cwd: &Path,
    diag: &mut Diag<'a>,
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (list_all, disable, reusable, special_only, delete, load_file, help, operands) = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut list_all = false;
        let mut disable = false;
        let mut reusable = false;
        let mut special_only = false;
        let mut delete = false;
        let mut load_file: Option<String> = None;
        let mut help = false;
        loop {
            match opt.next() {
                OptEvent::Opt(b'a') => list_all = true,
                OptEvent::Opt(b'n') => disable = true,
                OptEvent::Opt(b'p') => reusable = true,
                OptEvent::Opt(b's') => special_only = true,
                OptEvent::Opt(b'd') => delete = true,
                // GNU: `-f` takes the rest of the word or the next
                // word as its filename argument (bashgetopt.c).
                OptEvent::OptArg(b'f', value) => load_file = Some(value.to_string()),
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // The row's spec is `"adnpsf:"`: no other option
                // character (and no other argument-taking option)
                // can ever arrive.
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("enable spec is \"adnpsf:\"")
                }
            }
        }
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        (
            list_all,
            disable,
            reusable,
            special_only,
            delete,
            load_file,
            help,
            operands,
        )
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(EX_USAGE);
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok(diag.status());
    }

    // GNU builtins/enable.def (`if (list == 0 || (flags & PFLAG))`): this
    // branch is checked BEFORE -f/-d, so `enable -f file` with no NAME
    // and `enable -p -f file NAME` both list instead of loading. `-p`
    // lists even when operands are present — the operands are ignored
    // (rubash#1397). Plain `enable` with no operands lists in every mode.
    if operands.is_empty() || reusable {
        // GNU builtins/enable.def:172-180: the list filter is
        // `AFLAG ? (ENABLED|DISABLED) : NFLAG ? DISABLED : ENABLED`,
        // with SPECIAL OR'd in when `-s` is given — and it applies
        // UNIFORMLY to the full, disabled-only, and special listings
        // alike. The old code had three separate listing branches
        // whose filter rules disagreed: `-an` listed only disabled
        // builtins (empty in a fresh shell) instead of the full
        // listing, `-s` with `-n` ignored the filter, and a disabled
        // special builtin printed as enabled under `-s`
        // (rubash#2938). One filter, applied to every row:
        //   list_some_builtins walks the table; a row prints
        //   `enable name` when it is enabled and ENABLED is in the
        //   filter, `enable -n name` when it is disabled and
        //   DISABLED is in the filter (enable.def:449-466).
        let disabled = disabled_builtins(env_vars);
        let show_enabled = list_all || !disable;
        let show_disabled = list_all || disable;
        // Stage-2 ④ Phase 0: the listing walks the one declaration
        // table (D1) instead of the registry ledger. The two sets are
        // pinned equal by the table's unit tests
        // (`builtin_table::tests::names_match_builtin_names` and
        // `introspection_matches_registry`); the dynamic-load dedup
        // below still consults the registry, unchanged.
        for name in crate::executor::builtin_table::table_builtin_names() {
            if special_only && !is_table_special_builtin(name) {
                continue;
            }
            if disabled.iter().any(|disabled| disabled == name) {
                if show_disabled {
                    writeln!(stdout, "enable -n {name}")?;
                }
            } else if show_enabled {
                writeln!(stdout, "enable {name}")?;
            }
        }
        // GNU appends dynamically loaded builtins after the compiled
        // table (enable.def:449-466).  Skip a loaded name that already
        // has a static row: a shadowing load replaces that one slot.
        for name in dynamic::loaded_names() {
            if crate::executor::builtin_table::is_table_builtin_name(&name) {
                continue;
            }
            if special_only && !dynamic::is_special(&name) {
                continue;
            }
            if disabled.iter().any(|disabled| disabled == &name) {
                if show_disabled {
                    writeln!(stdout, "enable -n {name}")?;
                }
            } else if show_enabled {
                writeln!(stdout, "enable {name}")?;
            }
        }
        return Ok(EXECUTION_SUCCESS);
    }

    if let Some(file) = load_file {
        let names: Vec<String> = operands.iter().map(|name| name.to_string()).collect();
        let loadables_path = env_vars.get("BASH_LOADABLES_PATH").map(String::as_str);
        match dynamic::load_builtin(&file, &names, loadables_path, cwd, special_only, disable) {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    diag.free(format_args!("warning: {warning}"));
                }
                for error in &outcome.errors {
                    diag.free(format_args!("{error}"));
                }
                for name in &outcome.loaded {
                    set_builtin_enabled(env_vars, name, !disable);
                }
                // GNU enable.def:420-446: diagnostics for individual
                // names do not turn the whole load into a failure when
                // at least one builtin was installed.
                if outcome.loaded.is_empty() {
                    return Ok(EXECUTION_FAILURE);
                }
                return Ok(EXECUTION_SUCCESS);
            }
            Err(error) => {
                diag.free(format_args!("{error}"));
                return Ok(EXECUTION_FAILURE);
            }
        }
    }

    if delete {
        // GNU enable.def:536-586 (`dyn_unload_builtin`): NAME must be a
        // builtin and must have been loaded with -f; a static builtin is
        // reported "not dynamically loaded".
        let mut status = EXECUTION_SUCCESS;
        for name in operands {
            if !is_builtin(name) {
                diag.free(format_args!("{name}: not a shell builtin"));
                status = EXECUTION_FAILURE;
            } else if !dynamic::is_loaded(name) {
                diag.free(format_args!("{name}: not dynamically loaded"));
                status = EXECUTION_FAILURE;
            } else {
                match dynamic::unload_builtin(name) {
                    Ok(()) => set_builtin_enabled(env_vars, name, true),
                    Err(error) => {
                        diag.free(format_args!("{error}"));
                        status = EXECUTION_FAILURE;
                    }
                }
            }
        }
        return Ok(status);
    }

    let mut disabled = disabled_builtins(env_vars);
    let mut status = EXECUTION_SUCCESS;
    for name in operands {
        if !is_builtin(name) {
            diag.free(format_args!("{name}: not a shell builtin"));
            status = EXECUTION_FAILURE;
            continue;
        }

        if disable {
            if !disabled.iter().any(|disabled| disabled == name) {
                disabled.push(name.to_string());
            }
        } else {
            disabled.retain(|disabled| disabled != name);
        }
        // Keep a loaded struct's BUILTIN_ENABLED bit in sync.
        dynamic::set_enabled(name, !disable);
    }
    set_disabled_builtins(env_vars, &disabled);
    Ok(status)
}

/// Change a builtin's enabled state without parsing an `enable` command.
/// Embedding hosts use this for policy defaults before user startup code runs.
pub fn set_disabled(env_vars: &mut VarTable, name: &str, disabled: bool) {
    let mut builtins = disabled_builtins(env_vars);
    if disabled {
        if !builtins.iter().any(|builtin| builtin == name) {
            builtins.push(name.to_string());
        }
    } else {
        builtins.retain(|builtin| builtin != name);
    }
    set_disabled_builtins(env_vars, &builtins);
    dynamic::set_enabled(name, !disabled);
}

pub fn is_disabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    disabled_builtins(env_vars)
        .iter()
        .any(|disabled| disabled == name)
}

/// The one reader for the disabled-builtin carrier (the completion
/// candidates in `complete.rs` read through here too).
///
/// rubash#2942: the carrier is NEWLINE-joined, not colon-joined. A
/// builtin name is a single shell word from the static table or a
/// loadable-builtin symbol name; neither can contain a newline — but
/// `:` is itself a builtin name, so the colon-joined form stored the
/// single-element list `[":"]` as `":"` and the split read it back as
/// zero names, silently un-disabling `:`. (A value inherited from an
/// older shell in the colon-joined form reads as one inert name that
/// matches no builtin; the carrier is rubash-internal and is rewritten
/// on the first `enable` change.)
pub(crate) fn disabled_builtins(env_vars: &HashMap<String, String>) -> Vec<String> {
    env_vars
        .get(DISABLED_BUILTINS)
        .map(|value| {
            value
                .split('\n')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn set_disabled_builtins(env_vars: &mut VarTable, disabled: &[String]) {
    if disabled.is_empty() {
        env_vars.remove(DISABLED_BUILTINS);
    } else {
        env_vars.insert(DISABLED_BUILTINS.to_string(), disabled.join("\n"));
    }
}

fn is_builtin(name: &str) -> bool {
    // Stage-2 ④ Phase 2: the introspection face reads the DERIVED
    // table (D1) — `env` is a builtin here too (KD-1's third face).
    // Loadable builtins join the same table from the process-global
    // dynamic registry.
    crate::executor::builtin_table::is_table_builtin_name(name) || dynamic::is_loaded(name)
}

fn is_table_special_builtin(name: &str) -> bool {
    crate::executor::builtin_table::is_table_special_builtin_name(name)
}

/// Set NAME's enabled state in the existing `enable -n` carrier used by
/// every dispatch path.  Loading with `-n` records the same disabled
/// state as `enable -n NAME` (GNU clears BUILTIN_ENABLED, enable.def:430).
fn set_builtin_enabled(env_vars: &mut VarTable, name: &str, enabled: bool) {
    let mut disabled = disabled_builtins(env_vars);
    if enabled {
        disabled.retain(|disabled| disabled != name);
    } else if !disabled.iter().any(|disabled| disabled == name) {
        disabled.push(name.to_string());
    }
    set_disabled_builtins(env_vars, &disabled);
    dynamic::set_enabled(name, enabled);
}
