//! shopt module.
//!
//! GNU Bash source ownership:
// - builtins/shopt.def

mod compat;
mod support;

pub(crate) use compat::{
    binds_bashcompat_operand, derived_level, flag_for_level, is_compat_flag, level_for_flag,
    parse_bash_compat_value, DEFAULT_COMPAT_LEVEL,
};
pub(crate) use support::is_supported_option;
pub(crate) use support::SHOPT_OPTIONS;
use support::{default_state, print_all_shopts, print_shopt, print_shopts_by_state};

/// rubash#780: the startup `-O`/`+O` list form (`bash -O` with no option
/// argument). GNU shell.c routes the argument-less case to the shopt table,
/// so the bytes are the builtin's: `-O` renders `name<TAB>on|off` (the bare
/// `shopt` listing, reusable=false), `+O` the `shopt -s|-u name` recreate
/// list (the `shopt -p` listing, reusable=true).
pub(crate) fn print_shopt_listing<W: Write>(
    env_vars: &HashMap<String, String>,
    reusable: bool,
    stdout: &mut W,
) -> io::Result<()> {
    print_all_shopts(env_vars, reusable, stdout)
}

use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const SHOPT_STATE: &str = "__RUBASH_SHOPT_STATE";

static XPG_ECHO: AtomicBool = AtomicBool::new(false);
static SOURCEPATH: AtomicBool = AtomicBool::new(true);
static CHECKHASH: AtomicBool = AtomicBool::new(false);

pub(crate) fn xpg_echo_enabled() -> bool {
    XPG_ECHO.load(Ordering::Relaxed)
}

pub(crate) fn sourcepath_enabled() -> bool {
    SOURCEPATH.load(Ordering::Relaxed)
}

pub(crate) fn checkhash_enabled() -> bool {
    CHECKHASH.load(Ordering::Relaxed)
}

pub(crate) fn cdable_vars_enabled(env_vars: &HashMap<String, String>) -> bool {
    option_enabled(env_vars, "cdable_vars")
}

pub(crate) fn bashopts_value(env_vars: &HashMap<String, String>) -> String {
    SHOPT_OPTIONS
        .iter()
        .copied()
        .filter(|name| option_enabled(env_vars, name))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn execute(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_with_io_tracked(args, env_vars, stdout, stderr, None, None)
}

/// [`execute_with_io`] plus a sink for the `-o` binary-option WRITES
/// (name, enabled) in operand order. GNU builtins/shopt.def:594-606
/// set_shopt_o_options routes every `-o` operand through
/// builtins/set.def:476-491 set_minus_o_option, whose vi/emacs setter
/// (set.def:424-447 set_edit_mode) fires on EVERY FLAG_ON write — even
/// a redundant `shopt -s -o vi` with the option already on rebinds
/// `editing-mode` (oracle-verified) — so a caller mirroring the option
/// funnel's side effects needs the writes themselves, not a before/
/// after state diff (rubash#2435). Print/list modes never write and
/// record nothing.
///
/// The second sink reports a `login_shell` set/unset ATTEMPT (either
/// spelling, any resulting value): GNU's login_shell entry carries a
/// custom setter (builtins/shopt.def:244, set_login_shell at
/// shopt.def:762-766) that ignores the requested value and re-copies
/// the CURRENT login_shell global, which this pure layer cannot see —
/// the executor re-derives the stored bit when the flag is set
/// (rubash#2457). An attempt that lands on the already-reported value
/// still counts: `shopt -s login_shell` inside a `( )` region writes
/// "on" here but must end off, so a before/after diff cannot detect
/// it.
pub(crate) fn execute_with_io_and_o_writes<W, E>(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
    o_writes: &mut Vec<(String, bool)>,
    login_shell_write: &mut bool,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_with_io_tracked(
        args,
        env_vars,
        stdout,
        stderr,
        Some(o_writes),
        Some(login_shell_write),
    )
}

fn execute_with_io_tracked<W, E>(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
    o_writes: Option<&mut Vec<(String, bool)>>,
    mut login_shell_write: Option<&mut bool>,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // `shopt` lists human-readable states by default; `-p` switches to
    // reusable `shopt -s/-u` commands.  An empty argument list is not `-p`.
    let mut print = false;
    let mut use_set_options = false;
    let mut mode = ShoptMode::List;
    let mut names = Vec::new();
    let mut status = EXECUTION_SUCCESS;
    let mut saw_set = false;
    let mut saw_unset = false;
    // GNU builtins/shopt.def:313: `-q` is QFLAG (quiet), independent of the
    // set/unset mode — `shopt -s -q opt` SETS opt silently (rubash#1357).
    let mut quiet = false;
    // GNU builtins/bashgetopt.c:84-89: a bare `--` ends option parsing.
    let mut parsing_options = true;

    for arg in args {
        if parsing_options && arg == "--" {
            parsing_options = false;
            continue;
        }
        if parsing_options && arg.starts_with('-') && arg != "-" {
            for option in arg[1..].chars() {
                match option {
                    's' => {
                        mode = ShoptMode::Set;
                        saw_set = true;
                    }
                    'u' => {
                        mode = ShoptMode::Unset;
                        saw_unset = true;
                    }
                    'q' => quiet = true,
                    'p' => print = true,
                    'o' => use_set_options = true,
                    other => {
                        writeln!(
                            stderr,
                            "{}shopt: -{}: invalid option",
                            diagnostic_prefix(),
                            other
                        )?;
                        writeln!(stderr, "shopt: usage: shopt [-pqsu] [-o] [optname ...]")?;
                        return Ok(EX_USAGE);
                    }
                }
            }
        } else {
            parsing_options = false;
            names.push(arg.as_str());
        }
    }

    // GNU builtins/shopt.def:331-334: if both -s and -u are given, report
    // "cannot set and unset shell options simultaneously" via builtin_error.
    if saw_set && saw_unset {
        writeln!(
            stderr,
            "{}shopt: cannot set and unset shell options simultaneously",
            diagnostic_prefix()
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    if use_set_options {
        return execute_set_option_mode(
            mode, print, quiet, &names, env_vars, stdout, stderr, o_writes,
        );
    }

    if names.is_empty() {
        // GNU shopt.def list_shopts/list_some_shopts with QFLAG: no output.
        if quiet {
            return Ok(EXECUTION_SUCCESS);
        }
        match mode {
            ShoptMode::Set if print => {
                print_shopts_by_state(env_vars, true, true, stdout)?;
            }
            ShoptMode::Unset if print => {
                print_shopts_by_state(env_vars, false, true, stdout)?;
            }
            ShoptMode::Unset => {
                print_shopts_by_state(env_vars, false, false, stdout)?;
            }
            ShoptMode::List => {
                print_all_shopts(env_vars, print, stdout)?;
            }
            // GNU builtins/shopt.def shopt_builtin: bare `shopt -s` (no
            // operands, no -p) lists the SET options in the plain
            // name/on-off format, exactly like bare `shopt -u` lists the
            // unset ones (rubash#1107 item 5: `shopt -s compat44;
            // shopt -s | grep -c compat` is 1 in GNU, was 0).
            ShoptMode::Set => {
                print_shopts_by_state(env_vars, true, false, stdout)?;
            }
        }
        return Ok(status);
    }

    for name in names {
        if !is_supported_option(name) {
            writeln!(
                stderr,
                "{}shopt: {name}: invalid shell option name",
                diagnostic_prefix()
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }

        // rubash#2457: a login_shell write attempt is reported for
        // the executor's re-derivation (see execute_with_io_and_o_writes);
        // the write itself proceeds with the requested value here and
        // the executor's correction, when it runs, overwrites it.
        if name == "login_shell" && matches!(mode, ShoptMode::Set | ShoptMode::Unset) {
            if let Some(flag) = login_shell_write.as_deref_mut() {
                *flag = true;
            }
        }

        match mode {
            ShoptMode::Set => {
                if let Some(diagnostic) = set_option(env_vars, name, true) {
                    writeln!(stderr, "{}{diagnostic}", diagnostic_prefix())?;
                }
            }
            ShoptMode::Unset => {
                if let Some(diagnostic) = set_option(env_vars, name, false) {
                    writeln!(stderr, "{}{diagnostic}", diagnostic_prefix())?;
                }
            }
            // GNU shopt.def list_shopts with QFLAG: no output, rc reflects
            // whether every queried option is on.
            ShoptMode::List if quiet => {
                if !option_enabled(env_vars, name) {
                    status = EXECUTION_FAILURE;
                }
            }
            ShoptMode::List if print => {
                if !option_enabled(env_vars, name) {
                    status = EXECUTION_FAILURE;
                }
                print_shopt(env_vars, name, true, stdout)?;
            }
            ShoptMode::List => {
                if !option_enabled(env_vars, name) {
                    status = EXECUTION_FAILURE;
                }
                print_shopt(env_vars, name, false, stdout)?;
            }
        }
    }

    Ok(status)
}

#[allow(clippy::too_many_arguments)]
fn execute_set_option_mode<W, E>(
    mode: ShoptMode,
    print: bool,
    quiet: bool,
    names: &[&str],
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
    mut o_writes: Option<&mut Vec<(String, bool)>>,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut status = EXECUTION_SUCCESS;
    // Operand-less `shopt -o` listing goes through GNU's minus-o print path:
    // builtins/shopt.def:553-559 list_shopt_o_options(list=0) ->
    // list_minus_o_opts -> builtins/set.def:281 print_minus_o_option with
    // MINUS_O_FORMAT "%-15s\t%s\n". That is width 15, NOT the OPTFMT
    // "%-20s\t%s\n" of shopt.def:73 used by plain `shopt` and by
    // `shopt -o <name>` (shopt.def:579). Verified against the WSL GNU Bash
    // 5.3.0 oracle (/usr/local/bin/bash): tab lands at byte 15 for `set -o`
    // and `shopt -o`, at byte 20 for plain `shopt`.
    const MINUS_O_PRINT_WIDTH: usize = crate::builtins::set::SET_O_PRINT_WIDTH;
    if names.is_empty() {
        // GNU shopt.def list_shopt_o_options/list_some_o_options with QFLAG:
        // no output.
        if quiet {
            return Ok(EXECUTION_SUCCESS);
        }
        match mode {
            ShoptMode::Set if print => {
                crate::builtins::set::print_shell_options_by_state(
                    env_vars,
                    true,
                    true,
                    MINUS_O_PRINT_WIDTH,
                    stdout,
                )?;
            }
            ShoptMode::Unset if print => {
                crate::builtins::set::print_shell_options_by_state(
                    env_vars,
                    false,
                    true,
                    MINUS_O_PRINT_WIDTH,
                    stdout,
                )?;
            }
            // GNU shopt.def list_some_o_options: `shopt -s -o` with no
            // names lists only the options currently SET (the -s filter),
            // in the plain display format — it is not the full table
            // (rubash#1487).
            ShoptMode::Set => {
                crate::builtins::set::print_shell_options_by_state(
                    env_vars,
                    true,
                    false,
                    MINUS_O_PRINT_WIDTH,
                    stdout,
                )?;
            }
            ShoptMode::Unset => {
                crate::builtins::set::print_shell_options_by_state(
                    env_vars,
                    false,
                    false,
                    MINUS_O_PRINT_WIDTH,
                    stdout,
                )?;
            }
            _ => crate::builtins::set::print_shell_options(
                env_vars,
                print,
                MINUS_O_PRINT_WIDTH,
                stdout,
            )?,
        }
        return Ok(status);
    }

    for name in names {
        if !crate::builtins::set::is_shell_option(name) {
            writeln!(
                stderr,
                "{}shopt: {name}: invalid option name",
                diagnostic_prefix()
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }
        match mode {
            ShoptMode::Set if !print => {
                crate::builtins::set::set_shell_option(env_vars, name, true);
                if let Some(writes) = o_writes.as_deref_mut() {
                    writes.push((name.to_string(), true));
                }
            }
            ShoptMode::Unset if !print => {
                crate::builtins::set::set_shell_option(env_vars, name, false);
                if let Some(writes) = o_writes.as_deref_mut() {
                    writes.push((name.to_string(), false));
                }
            }
            _ if print || mode == ShoptMode::List => {
                if mode == ShoptMode::List
                    && !crate::builtins::set::shell_option_enabled(env_vars, name)
                {
                    status = EXECUTION_FAILURE;
                }
                // GNU shopt.def list_shopt_o_options with QFLAG: no output.
                if !quiet {
                    crate::builtins::set::print_shell_option(
                        env_vars,
                        name,
                        print,
                        crate::builtins::set::SHOPT_O_PRINT_WIDTH,
                        stdout,
                    )?;
                }
            }
            ShoptMode::List | ShoptMode::Set | ShoptMode::Unset => {}
        }
    }

    Ok(status)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShoptMode {
    List,
    Set,
    Unset,
}

/// GNU builtins/shopt.def:180-181 binds `array_expand_once` and
/// `assoc_expand_once` to the SAME variable (expand_once_flag) through the
/// same set_array_expand handler, so the two names always report and toggle
/// ONE shared flag; doc/bashref.info:5208 calls `assoc_expand_once`
/// "Deprecated; a synonym for array_expand_once". The canonical state entry
/// is `array_expand_once`, and the alias never persists in the state set.
fn canonical_shopt_name(name: &str) -> &str {
    if name == "assoc_expand_once" {
        "array_expand_once"
    } else {
        name
    }
}

pub(crate) fn option_enabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    let name = canonical_shopt_name(name);
    match name {
        "xpg_echo" => xpg_echo_enabled(),
        "checkhash" => checkhash_enabled(),
        "sourcepath" => sourcepath_enabled(),
        _ => state_contains(env_vars, name),
    }
}

/// The single derived `shell_compatibility_level` (see `compat.rs`): a set
/// compatNN shopt flag wins, else the strict `BASH_COMPAT` parse, else the
/// 5.3 default. Every level-keyed behavior (`unset` `VA_ALLOWALL`,
/// `test -v` `AV_ATSTARKEYS`, `=~` quoted-RHS mode) reads this — never
/// `BASH_COMPAT` or a compatNN flag directly.
pub(crate) fn compatibility_level(env_vars: &HashMap<String, String>) -> u32 {
    derived_level(
        |flag| option_enabled(env_vars, flag),
        env_vars.get("BASH_COMPAT").map(String::as_str),
    )
}

/// GNU `variables.c:6460-6508 sv_shcompat()` port for `BASH_COMPAT`
/// assignments (bare, `declare`/`export`/`local`, prefix, `read`, ...):
/// re-derive the shopt compat flags from the bound value. The raw value is
/// kept verbatim (GNU rebinds nothing on this path — even `"99"` stays
/// while the level resets to default). Returns the
/// `compatibility value out of range` diagnostic for the caller to emit;
/// the assignment itself still succeeds (status 0, verified oracle).
pub(crate) fn sync_compat_flags_from_bash_compat(
    env_vars: &mut crate::shell::var_table::VarTable,
) -> Result<(), String> {
    // Fast path (executor spawns hit this on every construction): with no
    // BASH_COMPAT binding and no compat flag set there is nothing to sync.
    // The flag scan stays on the allocation-free static path while
    // SHOPT_STATE is absent (state_contains's None arm).
    if !env_vars.contains_key("BASH_COMPAT")
        && !compat::COMPAT_FLAGS
            .iter()
            .any(|(flag, _)| option_enabled(env_vars, flag))
    {
        return Ok(());
    }
    let level = match env_vars.get("BASH_COMPAT").cloned() {
        None => DEFAULT_COMPAT_LEVEL,
        Some(ref raw) if raw.is_empty() => DEFAULT_COMPAT_LEVEL,
        Some(raw) => match parse_bash_compat_value(&raw) {
            Some(level) => level,
            None => {
                set_active_compat_flag(env_vars, None);
                return Err(format!(
                    "BASH_COMPAT: {raw}: compatibility value out of range"
                ));
            }
        },
    };
    set_active_compat_flag(env_vars, flag_for_level(level));
    Ok(())
}

/// `unset BASH_COMPAT` — `sv_shcompat()`'s `v == 0` arm: default level,
/// all compat flags off. The variable itself stays removed (no rebind).
pub(crate) fn clear_compat_flags(env_vars: &mut crate::shell::var_table::VarTable) {
    set_active_compat_flag(env_vars, None);
}

/// Write exactly one active compat flag (or none), clearing the rest —
/// the `set_compatibility_opts()` half of the sync.
fn set_active_compat_flag(env_vars: &mut crate::shell::var_table::VarTable, active: Option<&str>) {
    let mut current = state(env_vars);
    for (flag, _) in compat::COMPAT_FLAGS {
        current.remove(*flag);
    }
    if let Some(flag) = active {
        current.insert(flag.to_string());
    }
    store_state(env_vars, current, true);
}

/// First set compat flag in GNU priority order (`shopt.def:676-689`).
fn first_set_compat_level(env_vars: &HashMap<String, String>) -> Option<u32> {
    compat::COMPAT_FLAGS
        .iter()
        .find(|(flag, _)| option_enabled(env_vars, flag))
        .map(|(_, level)| *level)
}

/// Allocation-free membership test against the serialized shopt state.
/// Equivalent to `state(env_vars).contains(name)`: the set is exactly the
/// nonempty `$__RUBASH_SHOPT_STATE` segments, so splitting the serialized
/// string and comparing slices answers the same question without building
/// a HashSet. The former per-query rebuild allocated one String per
/// enabled option on every lookup — a measured hot cost of each `(( ))`
/// evaluation and each glob word (rubash#156/#157).
fn state_contains(env_vars: &HashMap<String, String>, name: &str) -> bool {
    match env_vars.get(SHOPT_STATE) {
        Some(value) => value.split(DATA_DOLLAR).any(|entry| entry == name),
        // rubash#237: while the serialized state is unset (a script that
        // never ran `shopt`), this arm runs on EVERY option_enabled call —
        // use the allocation-free equivalent instead of rebuilding the
        // 63-entry HashSet per lookup.
        None => support::default_state_contains(name),
    }
}

pub(crate) fn set_option(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    enabled: bool,
) -> Option<String> {
    set_option_inner(env_vars, name, enabled, true)
}

/// GNU general.c:103-128 posix_initialize writes the posix_vars C
/// variables directly; it does NOT go through shopt.def toggle_shopts,
/// so a posix-mode flip never rebinds $BASHOPTS (rubash#1055: runtime
/// `set -o posix` leaves $BASHOPTS at its startup/shopt-builtin snapshot
/// even though `shopt` queries show the flipped values). The posix walk
/// in builtins/set/options.rs therefore sets its options through this
/// entry, which skips the set_bashopts rebind at the bottom.
pub(crate) fn set_option_from_posix_walk(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    enabled: bool,
) -> Option<String> {
    set_option_inner(env_vars, name, enabled, false)
}

fn set_option_inner(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    enabled: bool,
    rebind_bashopts: bool,
) -> Option<String> {
    // rubash#131: keep the parse-time extglob gate in step with the runtime
    // shopt (GNU builtins/shopt.def updates extglob_flag; reset_parser
    // parse.y:3502 propagates it into the parser's extended_glob). Later
    // in-process parses (`eval`, ${THIS_SH} scripts) then see the same
    // value GNU's parser would.
    if name == "extglob" {
        crate::lexer::set_parse_extended_glob(enabled);
        // rubash#1093: record the line whose execution enabled extglob so
        // the executor can tell a runtime flip on an EARLIER line (GNU's
        // read-execute cadence parses later lines with the gate on) from
        // a same-line `shopt -s extglob; case ...` unit (GNU parses the
        // whole line first and rejects the pattern).
        if enabled {
            let line = env_vars
                .get("__RUBASH_CURRENT_LINE")
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            env_vars.insert("__RUBASH_EXTGLOB_ENABLED_LINE".to_string(), line);
        } else {
            env_vars.remove("__RUBASH_EXTGLOB_ENABLED_LINE");
        }
    }
    match name {
        "xpg_echo" => XPG_ECHO.store(enabled, Ordering::Relaxed),
        "sourcepath" => SOURCEPATH.store(enabled, Ordering::Relaxed),
        "checkhash" => {
            CHECKHASH.store(enabled, Ordering::Relaxed);
            // G-03: route through the mirror funnel so the write is
            // journaled to the running executor's lineage.
            if enabled {
                crate::executor::local_helpers::set_process_env("__RUBASH_SHOPT_CHECKHASH", "1");
            } else {
                crate::executor::local_helpers::unset_process_env("__RUBASH_SHOPT_CHECKHASH");
            }
        }
        // GNU builtins/shopt.def:617-627 (shopt_set_debug_mode):
        // error_trace_mode = function_trace_mode = debugging_mode, so turning
        // extdebug on also enables the functrace and errtrace flags, and
        // turning it off clears them (a later `set +T` then stops
        // DEBUG/RETURN inheritance even with extdebug active,
        // dbg-support.tests:19 vs :91).
        "extdebug" => {
            for option in ["functrace", "errtrace"] {
                crate::builtins::set::set_shell_option(env_vars, option, enabled);
            }
        }
        _ => {}
    }

    // The expand-once pair shares one flag (canonical_shopt_name); the
    // alias name never persists in the state set so both names always
    // report the same value.
    let name = canonical_shopt_name(name);

    // GNU `builtins/shopt.def:654-708 set_compatibility_level()`: compatNN
    // flags are mutually exclusive and rebind BASH_COMPAT — a plain state
    // insert would leave siblings on and the level stale (rubash#1107).
    // Returns the bind-hook diagnostic for the caller to emit (a readonly
    // BASH_COMPAT refuses the rebind but the flag is still set, and the
    // status stays 0 — verified oracle).
    if is_compat_flag(name) {
        return apply_shopt_compat_change(env_vars, name, enabled);
    }

    let mut state = state(env_vars);
    state.remove("assoc_expand_once");
    if enabled {
        state.insert(name.to_string());
    } else {
        state.remove(name);
    }
    store_state(env_vars, state, rebind_bashopts);
    None
}

/// GNU `builtins/shopt.def:654-708 set_compatibility_level()`: setting a
/// compat flag clears every sibling and rebinds `BASH_COMPAT` to the flag's
/// level; unsetting falls back through the remaining flags in priority
/// order, keeps a flag-less 45..52 level (`oldval > 44 && level < default`
/// — e.g. `BASH_COMPAT=51; shopt -u compat44` stays at 51), else restores
/// the default and rebinds `BASH_COMPAT` to it (even when unsetting an
/// already-off flag, verified oracle).
fn apply_shopt_compat_change(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    enabled: bool,
) -> Option<String> {
    // `oldval` predates the toggle (`shopt.def:661` — toggle_shopts already
    // flipped the C int before the set-func runs).
    let old_level = compatibility_level(env_vars);
    let mut current = state(env_vars);
    if enabled {
        for (flag, _) in compat::COMPAT_FLAGS {
            current.remove(*flag);
        }
        current.insert(name.to_string());
    } else {
        current.remove(name);
    }
    store_state(env_vars, current, true);
    // GNU `shopt.def:675-693` priority chain over what remains.
    let new_level = if enabled {
        level_for_flag(name).unwrap_or(DEFAULT_COMPAT_LEVEL)
    } else {
        first_set_compat_level(env_vars).unwrap_or_else(|| {
            if old_level > 44 && old_level < DEFAULT_COMPAT_LEVEL {
                old_level
            } else {
                DEFAULT_COMPAT_LEVEL
            }
        })
    };
    // GNU bind_variable refuses a readonly BASH_COMPAT (sh_readonly): the
    // rebind is skipped but the flag change above stands and the status
    // stays 0 (toggle_shopts ignores the set-func return; verified oracle
    // reports `BASH_COMPAT: readonly variable` with rc 0 and the flag on).
    if env_vars.is_marked(crate::executor::types::READONLY_VARS, "BASH_COMPAT") {
        return Some("BASH_COMPAT: readonly variable".to_string());
    }
    env_vars.insert("BASH_COMPAT".to_string(), new_level.to_string());
    None
}

fn state(env_vars: &HashMap<String, String>) -> HashSet<String> {
    let Some(value) = env_vars.get(SHOPT_STATE) else {
        return default_state();
    };
    value
        .split(DATA_DOLLAR)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Persist a mutated shopt state: normalize back to ABSENT when it equals
/// the defaults (perf: keeps every option_enabled lookup on the
/// allocation-free static path — a single redundant startup write used to
/// flip ALL lookups to the serialized split-scan, ~1us per simple
/// command), then rebind BASHOPTS (GNU shopt.def toggle_shopts ->
/// set_bashopts, so an exported BASHOPTS carries live state to child
/// shells — invocation1.sub:28-31).
fn store_state(
    env_vars: &mut crate::shell::var_table::VarTable,
    state: HashSet<String>,
    rebind_bashopts: bool,
) {
    // Normalization (perf): a state equal to the defaults is stored as
    // ABSENT, keeping every option_enabled lookup on the allocation-free
    // static path (state_contains's None arm). A single redundant startup
    // write — e.g. a host pinning expand_aliases off — used to flip ALL
    // lookups to the serialized split-scan for the whole session, a
    // measured ~1us per simple command. Reads treat absent as default, so
    // nothing observable changes (same shopt -p/BASHOPTS rendering).
    if state == default_state() {
        env_vars.remove(SHOPT_STATE);
    } else {
        env_vars.insert(SHOPT_STATE.to_string(), serialize_state(&state));
    }
    // GNU shopt.def toggle_shopts -> set_bashopts: every shopt change
    // rebinds the BASHOPTS variable, so an export of BASHOPTS carries the
    // live state to child shells (invocation1.sub:28-31). The posix walk
    // (set_option_from_posix_walk) is the one caller that must not
    // rebind: GNU posix_initialize bypasses toggle_shopts (rubash#1055).
    if rebind_bashopts {
        let bashopts = bashopts_value(env_vars);
        env_vars.insert("BASHOPTS".to_string(), bashopts);
    }
}

fn serialize_state(state: &HashSet<String>) -> String {
    let mut names: Vec<&str> = state.iter().map(String::as_str).collect();
    names.sort();
    names.join(DATA_DOLLAR_STR)
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_env() -> crate::shell::var_table::VarTable {
        crate::shell::var_table::VarTable::default()
    }

    #[test]
    fn assoc_expand_once_aliases_array_expand_once() {
        let mut env_vars = fresh_env();

        assert!(!option_enabled(&env_vars, "assoc_expand_once"));
        assert!(!option_enabled(&env_vars, "array_expand_once"));

        let _ = set_option(&mut env_vars, "assoc_expand_once", true);

        assert!(option_enabled(&env_vars, "assoc_expand_once"));
        assert!(option_enabled(&env_vars, "array_expand_once"));
        // The canonical entry persists, the alias does not.
        assert!(state(&env_vars).contains("array_expand_once"));
        assert!(!state(&env_vars).contains("assoc_expand_once"));

        // Unsetting either name clears the shared flag for both.
        let _ = set_option(&mut env_vars, "array_expand_once", false);

        assert!(!option_enabled(&env_vars, "assoc_expand_once"));
        assert!(!option_enabled(&env_vars, "array_expand_once"));
    }

    #[test]
    fn redundant_writes_normalize_to_absent_state() {
        // Perf: a state equal to the defaults must not materialize the
        // serialized entry — its presence flips every option_enabled
        // lookup from the static path to the split-scan. A host pinning
        // expand_aliases off at startup (its default) must leave the
        // fast path intact.
        let mut env_vars = fresh_env();
        assert!(env_vars.get(SHOPT_STATE).is_none());

        let _ = set_option(&mut env_vars, "expand_aliases", false);
        assert!(env_vars.get(SHOPT_STATE).is_none());
        assert!(!option_enabled(&env_vars, "expand_aliases"));
        // BASHOPTS still renders (from the default state, identically).
        assert!(!bashopts_value(&env_vars).contains("expand_aliases"));

        // A real deviation still materializes, and clearing it removes
        // the entry again.
        let _ = set_option(&mut env_vars, "expand_aliases", true);
        assert!(option_enabled(&env_vars, "expand_aliases"));
        assert!(env_vars.get(SHOPT_STATE).is_some());

        let _ = set_option(&mut env_vars, "expand_aliases", false);
        assert!(!option_enabled(&env_vars, "expand_aliases"));
        assert!(env_vars.get(SHOPT_STATE).is_none());
    }

    #[test]
    fn bashopts_lists_both_expand_once_names_when_enabled() {
        let mut env_vars = fresh_env();

        assert!(!bashopts_value(&env_vars)
            .split(':')
            .any(|name| name == "assoc_expand_once" || name == "array_expand_once"));

        let _ = set_option(&mut env_vars, "assoc_expand_once", true);

        let bashopts = bashopts_value(&env_vars);
        assert!(bashopts.split(':').any(|name| name == "assoc_expand_once"));
        assert!(bashopts.split(':').any(|name| name == "array_expand_once"));

        let _ = set_option(&mut env_vars, "array_expand_once", false);

        let bashopts = bashopts_value(&env_vars);
        assert!(!bashopts.split(':').any(|name| name == "assoc_expand_once"));
        assert!(!bashopts.split(':').any(|name| name == "array_expand_once"));
    }

    #[test]
    fn other_shopts_keep_independent_names() {
        let mut env_vars = fresh_env();

        let _ = set_option(&mut env_vars, "dotglob", true);

        assert!(option_enabled(&env_vars, "dotglob"));
        assert!(state(&env_vars).contains("dotglob"));
        assert!(!option_enabled(&env_vars, "assoc_expand_once"));
    }
}
