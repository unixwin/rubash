//! `set` and `unset` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/set.def (`set_builtin`, `unset_builtin`)

mod options;
mod unset;

pub(crate) use options::{
    is_flag_only_shell_option, is_shell_option, print_shell_option, print_shell_options,
    print_shell_options_by_state, set_shell_option, shell_option_enabled, shell_option_names,
    shellopts_value, sync_shell_option_flag, SET_O_PRINT_WIDTH, SHOPT_O_PRINT_WIDTH,
};
pub use unset::unset;
pub(crate) use unset::unset_with_stderr;
use unset::valid_identifier;

use crate::executor::builtin_diag::{Diag, DiagKind, Prolog};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;
use std::collections::HashMap;
use std::io::{self, Write};

pub(super) const EXECUTION_SUCCESS: i32 = 0;
pub(super) const EXECUTION_FAILURE: i32 = 1;
pub(crate) const EX_USAGE: i32 = 2;

/// The apply loop's admission set is change_flag's table (flags.c:168-200
/// shell_flags[]), which unlike the pre-scan DOES include 'i' — a character
/// hidden inside an inline `o` operand (`set -opipefail`) reaches the apply
/// loop, where 'i' succeeds (forced_interactive) and the first genuinely
/// unknown char ('l') produces sh_invalidopt + EXECUTION_FAILURE
/// (set.def:763-771). Rubash models no interactive-mode flip for that
/// corner, so 'i' applies as a no-op there.
const APPLY_FLAGS: &str = "abefhikmnprtuvxBCEHPT";

/// GNU flags.c:168-200 `shell_flags[]` letters that have no dedicated
/// branch below, mapped to their `-o` option names (set.def o_options /
/// flags.c find_flag). Single truth table for BOTH the executor fast paths
/// (`apply_simple_set_flags` / `apply_set_flag_updates`) and `set_with_io`,
/// so the historical two-path split cannot drift again (rubash#358: the
/// slow path used to validate these letters without ever applying them).
pub(crate) fn short_flag_option_name(flag: char) -> Option<&'static str> {
    match flag {
        'a' => Some("allexport"),
        'b' => Some("notify"),
        'B' => Some("braceexpand"),
        'E' => Some("errtrace"),
        'h' => Some("hashall"),
        'H' => Some("histexpand"),
        'k' => Some("keyword"),
        'm' => Some("monitor"),
        'P' => Some("physical"),
        'p' => Some("privileged"),
        'r' => Some("restricted"),
        't' => Some("onecmd"),
        'T' => Some("functrace"),
        'v' => Some("verbose"),
        _ => None,
    }
}

/// GNU set_builtin's per-character dispatch (builtins/set.def:716-772): a
/// `-`/`+` word is applied one char at a time, every char that is not `o`
/// (or the `r` refusal handled by callers) goes through change_flag
/// (flags.c:226), which writes the ONE flag variable flags.c:171 names for
/// the letter. Rubash's single counterpart of that variable is the option
/// table entry; exit_immediately_on_error's derived state is modeled by
/// the suppression counter and the comsub entry adjustments instead (the
/// old `__RUBASH_ERREXIT`/`__RUBASH_XTRACE` mirror markers are gone — a
/// second encoding that only this short form wrote went stale after
/// `set +o errexit` and resurrected the flag).
pub(crate) fn apply_short_set_flag(
    env_vars: &mut crate::shell::var_table::VarTable,
    flag: char,
    enabled: bool,
) {
    match flag {
        'e' => set_shell_option(env_vars, "errexit", enabled),
        'x' => set_shell_option(env_vars, "xtrace", enabled),
        'f' => set_shell_option(env_vars, "noglob", enabled),
        'n' => set_shell_option(env_vars, "noexec", enabled),
        'C' => set_shell_option(env_vars, "noclobber", enabled),
        'u' => set_shell_option(env_vars, "nounset", enabled),
        other => {
            if let Some(option) = short_flag_option_name(other) {
                set_shell_option(env_vars, option, enabled);
            }
        }
    }
}

pub(super) const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
pub(super) const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
pub(super) const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub(super) const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
pub(super) const ASSOC_128_VARS: &str = "__RUBASH_ASSOC_128_VARS";
pub(super) const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
pub(super) const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
pub(super) const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
pub(super) const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
pub(super) const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
pub(super) const INVISIBLE_VARS: &str = "__RUBASH_INVISIBLE_VARS";

/// Execute `set` with arguments after the command name.
pub fn set(args: &[String], env_vars: &mut crate::shell::var_table::VarTable) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    let row =
        crate::executor::builtin_table::builtin_table_row("set").expect("set row in BUILTIN_TABLE");
    let mut diag = Diag::new(row, &mut stderr);
    let status = set_with_io(
        row,
        args,
        env_vars,
        &mut diag,
        &mut stdout,
        &crate::shell::var_table::TempenvListingUnderlay::new(),
    );
    diag.flush_with(|| prolog_for_env(env_vars));
    status
}

/// The flush prolog for the stdio entry points above, built from
/// the same env cells `builtin_error_prefix` reads (the executor
/// path builds its own via `Executor::diagnostic_prolog`).
fn prolog_for_env(env_vars: &crate::shell::var_table::VarTable) -> Prolog {
    Prolog {
        interactive: env_vars.get("__RUBASH_INTERACTIVE").is_some(),
        shell_basename: env_vars
            .get("__RUBASH_SHELL_NAME")
            .cloned()
            .unwrap_or_else(|| "rubash".to_string()),
        error_name: "rubash".to_string(),
        script_file: env_vars.get("__RUBASH_SCRIPT_NAME").cloned(),
        line: env_vars.get("__RUBASH_CURRENT_LINE").cloned(),
    }
}

/// GNU builtin_error prolog (builtins/common.c:83-94): the shell name, the
/// executing line for scripts, then the builtin name. The builtin name is
/// added by each call site, so this only carries the script/line prolog.
/// Reads the executor env map (same sources as
/// Executor::diagnostic_prefix), matching GNU's script-relative prolog.
/// Interactive mode (shell reading from a terminal) omits the line segment.
pub fn builtin_error_prefix(env_vars: &HashMap<String, String>) -> String {
    if env_vars.contains_key("__RUBASH_INTERACTIVE") {
        // Interactive mode: report only the shell name, no line segment.
        // GNU error.c:88-120 (get_name_for_error) for interactive shells
        // returns base_pathname(shell_name) with no line number.
        if let Some(shell_name) = env_vars.get("__RUBASH_SHELL_NAME") {
            return format!("{shell_name}: ");
        }
        return "rubash: ".to_string();
    }

    // Script/-c mode: line segment present
    let line = env_vars.get("__RUBASH_CURRENT_LINE");
    let script = env_vars.get("__RUBASH_SCRIPT_NAME");
    match (script, line) {
        (Some(script), Some(line)) => {
            if env_vars.contains_key("__RUBASH_EVAL_CONTEXT") {
                format!("{script}: eval: line {line}: ")
            } else {
                // Script mode: "script: line N:"
                format!("{script}: line {line}: ")
            }
        }
        (None, Some(line)) => {
            // -c mode: "rubash: line N:"
            format!("rubash: line {line}: ")
        }
        _ => "rubash: ".to_string(),
    }
}

pub(crate) fn set_with_io<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    env_vars: &mut crate::shell::var_table::VarTable,
    diag: &mut Diag<'a>,
    stdout: &mut W,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
) -> io::Result<i32>
where
    W: Write,
{
    set_with_io_tracked(row, args, env_vars, diag, stdout, underlay, &[], None)
}

/// [`set_with_io`] plus a sink for the `-o` option WRITES (name,
/// enabled) in operand order. GNU builtins/set.def:476-491
/// set_minus_o_option invokes the option's setter on EVERY write via
/// SET_BINARY_O_OPTION_VALUE (set.def:247-249) — for vi/emacs the
/// setter is set_edit_mode (set.def:424-447), whose FLAG_ON arm runs
/// rl_variable_bind("editing-mode", name) even when the option was
/// already on — so a caller mirroring the funnel's side effects needs
/// the writes themselves, not a before/after state diff
/// (rubash#2448; the shopt door's tracked twin is rubash#2435). The
/// `-o` print form (no operand) writes nothing and records nothing.
pub(crate) fn set_with_io_and_o_writes<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    env_vars: &mut crate::shell::var_table::VarTable,
    diag: &mut Diag<'a>,
    stdout: &mut W,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
    dynamic: &[DynamicSetView],
    o_writes: &mut Vec<(String, bool)>,
) -> io::Result<i32>
where
    W: Write,
{
    set_with_io_tracked(
        row,
        args,
        env_vars,
        diag,
        stdout,
        underlay,
        dynamic,
        Some(o_writes),
    )
}

fn set_with_io_tracked<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    env_vars: &mut crate::shell::var_table::VarTable,
    diag: &mut Diag<'a>,
    stdout: &mut W,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
    dynamic: &[DynamicSetView],
    mut o_writes: Option<&mut Vec<(String, bool)>>,
) -> io::Result<i32>
where
    W: Write,
{
    if args.is_empty() {
        print_shell_variables(env_vars, underlay, dynamic, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    // GNU set.def:671-691 pre-scan: internal_getopt (bashgetopt.c) walks the
    // words with optflags (flags.c:372-382 = shell_flags letters + "o;")
    // and validates every flag word BEFORE set_builtin's apply loop runs,
    // so `set -e -Z` reports the error with errexit still off.
    {
        // Stage-2 ④ Phase 3: the validity scan is the (table row +
        // BuiltinOpt + Diag) triple — the row's spec is the optflags
        // string itself ('+' + shell_flags letters + "o;",
        // flags.c:372-382). The machine stops at the first
        // non-option word and at `--`/`-`, swallows an inline `o`
        // operand without validating its characters (`-opipefail`:
        // the scan sees only 'o'), and consumes a following
        // non-option word (`-o pipefail`) so a bad flag AFTER the
        // operand is still a scan error with nothing applied.
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    // builtins/bashgetopt.c:80-85: internal_getopt
                    // returns GETOPT_HELP for a `--help` word in
                    // option position, and set.def:684 CASE_HELPOPT
                    // prints the long help and returns EX_USAGE —
                    // from this VALIDITY scan, before the apply loop,
                    // so nothing is applied (`set -e --help` leaves
                    // errexit off). Words consumed as a `-o` operand
                    // never reach this check (rubash#2128).
                    crate::builtins::help::print_builtin_help("set", stdout)?;
                    return Ok(EX_USAGE);
                }
                OptEvent::Opt(b'i') => {
                    // set.def:677-683: `set -i` is explicitly
                    // refused; the errstr carries the sign of the
                    // word that carried the `i` (list_opttype).
                    let sign = if opt.opttype() == b'-' { "-i" } else { "+i" };
                    diag.error(DiagKind::InvalidOpt(sign.to_string()));
                    diag.usage();
                    return Ok(EX_USAGE);
                }
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {}
                OptEvent::End => break,
            }
        }
        let optopt = opt.optopt();
        drop(opt);
        if diag.status() != 0 {
            // The machine already reported the character
            // (bashgetopt.c:99-101 sh_invalidopt + '?' return);
            // set.def:684-686 adds the usage and maps an error on
            // '?' itself (list_optopt) to EXECUTION_SUCCESS.
            diag.usage();
            return Ok(if optopt == Some(b'?') {
                EXECUTION_SUCCESS
            } else {
                EX_USAGE
            });
        }
    }

    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if *arg == "--" || *arg == "-" {
            return Ok(EXECUTION_SUCCESS);
        }

        let Some(prefix) = arg.chars().next().filter(|ch| *ch == '-' || *ch == '+') else {
            return Ok(EXECUTION_SUCCESS);
        };

        let options = &arg[1..];
        if options.is_empty() {
            return Ok(EXECUTION_SUCCESS);
        }

        let mut chars = options.chars();
        while let Some(option) = chars.next() {
            if option == 'o' {
                match args.get(index + 1) {
                    Some(name)
                        if !name.is_empty() && !name.starts_with('-') && !name.starts_with('+') =>
                    {
                        if !is_shell_option(name) {
                            diag.free(format_args!("{name}: invalid option name"));
                            // GNU set.def treats an invalid `-o` name as a
                            // usage error, distinct from a valid option that
                            // simply reports failure.
                            return Ok(EX_USAGE);
                        }
                        set_shell_option(env_vars, name, prefix == '-');
                        if let Some(writes) = o_writes.as_deref_mut() {
                            writes.push((name.to_string(), prefix == '-'));
                        }
                        index += 1;
                    }
                    _ => print_shell_options(env_vars, prefix == '+', SET_O_PRINT_WIDTH, stdout)?,
                }
                // set.def:726-766: the `o` branch continues the character
                // loop, so trailing chars of the word (`-eox pipefail` → x,
                // `-opipefail` → p i p e f a i l) are still applied.
                continue;
            }

            if option == '?' {
                // set.def:724-728: the apply loop's own '?' arm —
                // builtin_usage + EXECUTION_SUCCESS, with NO
                // invalid-option line. Reachable only for a '?'
                // hidden inside an inline `o` operand (`set -o?`):
                // a standalone '?' is the pre-scan's (rubash#2957).
                diag.usage();
                return Ok(EXECUTION_SUCCESS);
            }

            if option == 'r' {
                if prefix == '+' && shell_option_enabled(env_vars, "restricted") {
                    // GNU flags.c change_flag (flags.c:227-235) refuses
                    // "set +r" in a restricted shell with FLAG_ERROR;
                    // set.def:763-771 reports sh_invalidopt("+r") plus the
                    // builtin usage line and returns EXECUTION_FAILURE.
                    diag.error(DiagKind::InvalidOpt(format!("{prefix}r")));
                    diag.usage();
                    return Ok(EXECUTION_FAILURE);
                }
                set_shell_option(env_vars, "restricted", prefix == '-');
                continue;
            }

            if !APPLY_FLAGS.contains(option) {
                // set.def:763-771: change_flag's FLAG_ERROR path. Only
                // reachable for characters hidden from the pre-scan inside
                // an inline `o` operand (`set -opipefail` dies at 'l').
                diag.error(DiagKind::InvalidOpt(format!("{prefix}{option}")));
                diag.usage();
                return Ok(EXECUTION_FAILURE);
            }

            // set.def:767 change_flag: apply every remaining flag char of
            // the word — the branch whose absence dropped `-e`/`-u` bundled
            // before `-o` (rubash#358).
            apply_short_set_flag(env_vars, option, prefix == '-');
            // flags.c:257: a monitor write also drives the job_control
            // variable through set_job_control; record it with the
            // `-o` writes so the executor dispatch can replay it onto
            // the variable's home after the builtin returns
            // (rubash#2487).
            if short_flag_option_name(option) == Some("monitor") {
                if let Some(writes) = o_writes.as_deref_mut() {
                    writes.push(("monitor".to_string(), prefix == '-'));
                }
            }
            // flags.c:226: change_flag has no special case for `i` —
            // reached only inside an inline `-o` operand (`set
            // -ovi`; a standalone ±i is refused by the pre-scan), it
            // flips forced_interactive, the state `$-` renders the
            // letter from. That state lives on ShellState, not in
            // the env table, so — exactly like monitor's job_control
            // replay above (rubash#2487) — record it with the `-o`
            // writes for the executor dispatch to replay onto its
            // home (rubash#2961).
            if option == 'i' {
                if let Some(writes) = o_writes.as_deref_mut() {
                    writes.push(("forced_interactive".to_string(), prefix == '-'));
                }
            }
        }

        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

/// One dynamic variable's assignment-form listing entry: the name and
/// its storage in the table's serialized array form. GNU's dynamic
/// variables are ordinary cells in the variable table
/// (variables.c:1844 initialize_dynamic_variables), so print_var_list
/// enumerates them with live getter values; rubash composes the stack
/// and group arrays outside the table, and the executor supplies their
/// current storage here so the assignment-form listings (`set`, bare
/// `declare`) render the live composition instead of a stale mirror
/// cell — or nothing, for cell-less GROUPS (rubash#2700).
pub(crate) struct DynamicSetView {
    pub name: String,
    pub storage: String,
}

/// GNU `set` with no arguments: print_var_list (variables.c) — assignment
/// form with print_var_value quoting. `declare` with no operands and no
/// minus-form attribute options dispatches here too (declare.def:373-380
/// `set_builtin ((WORD_LIST *)NULL)`), so the two listings stay identical.
pub(crate) fn print_shell_variables<W>(
    env_vars: &crate::shell::var_table::VarTable,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
    dynamic: &[DynamicSetView],
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let entry_storage: std::collections::HashMap<&str, &str> = dynamic
        .iter()
        .map(|entry| (entry.name.as_str(), entry.storage.as_str()))
        .collect();
    // rubash#2702 (owner decision D7): the injected THIS_SH spelling
    // (the running executable's own path) is an internal face, not a
    // user variable — GNU has no such name — so it stays out of this
    // listing, which backs plain `declare`/`typeset` as well as
    // `set`. A caller-supplied value kept by init (the sh/bash-named
    // wrapper exception) lists, under the same value-shaped rule
    // compgen and the declare -p listing apply (rubash#1799).
    let this_sh_injected = crate::builtins::complete::auto_this_sh_spelling();
    // GNU variables.c:1126-1142 print_var_value: the ansic ($'...')
    // branch is taken only when posixly_correct == 0; POSIX mode falls
    // to sh_single_quote with the raw bytes (audit B4-F11).
    let posix_mode = shell_option_enabled(env_vars, "posix");
    let mut vars: Vec<(&str, Option<&str>)> = env_vars
        .iter()
        .map(|(name, value)| (name.as_str(), Some(value.as_str())))
        .collect();
    for entry in dynamic {
        if !vars.iter().any(|(name, _)| *name == entry.name.as_str()) {
            vars.push((entry.name.as_str(), None));
        }
    }
    vars.sort_by(|left, right| left.0.cmp(right.0));

    for (name, value) in vars {
        // Internal bookkeeping variables (readonly/array/assoc marks, shell
        // option state) are invisible in listings, like GNU's invisible_p
        // vars in print_var_list (variables.c).
        if name.starts_with("__RUBASH_") {
            continue;
        }
        // rubash#2702: the injected THIS_SH spelling (see above).
        if name == "THIS_SH"
            && env_vars.get("THIS_SH").map(String::as_str) == Some(this_sh_injected.as_str())
        {
            continue;
        }
        // GNU att_invisible (variables.c print_var_list skips
        // invisible_p): COMP_WORDBREAKS is bound invisible at startup, so
        // `set` never lists it even though the variable exists
        // (rubash#1325).
        if unset::is_marked_variable(env_vars, INVISIBLE_VARS, name) {
            continue;
        }
        // GNU variables.c:511-526 (initialize_shell_variables): environment
        // entries whose names are not valid identifiers (e.g.
        // CommonProgramFiles(x86)) are bound into the invisible invalid_env
        // table instead of shell_variables, so print_var_list never shows
        // them. They still reach child processes through the export
        // environment (maybe_make_export_env, variables.c:5064).
        if !valid_identifier(name) {
            continue;
        }
        // rubash#1569: a name bound by the current command's own
        // tempenv frame renders its pre-frame binding: frame-created
        // names vanish, and a pre-frame binding without env text is
        // var_isset-false (variables.c:1912) so print_assignment skips
        // it. Array/assoc form comes from the snapshot attributes.
        if let Some(under) = underlay.get(name) {
            let Some((under_value, under_attrs)) = under else {
                continue;
            };
            let Some(value) = under_value else {
                continue;
            };
            if value.is_empty() && (under_attrs.assoc || under_attrs.array) {
                continue;
            }
            if under_attrs.assoc {
                let nbuckets = crate::executor::assoc_nbuckets(env_vars, name);
                writeln!(
                    stdout,
                    "{}={}",
                    name,
                    crate::builtins::declare::format_assoc_for_output(value, nbuckets)
                )?;
                continue;
            }
            if under_attrs.array {
                writeln!(
                    stdout,
                    "{}={}",
                    name,
                    crate::builtins::declare::format_array_for_output(value)
                )?;
                continue;
            }
            writeln!(stdout, "{}={}", name, shell_quote(value, posix_mode))?;
            continue;
        }
        // A dynamic registry entry supersedes the mirror cell: the
        // composition is the live value (rubash#2700). The visible
        // registry class is array-valued, so it renders through the
        // same array_to_assign form as an array cell.
        if let Some(storage) = entry_storage.get(name) {
            writeln!(
                stdout,
                "{}={}",
                name,
                crate::builtins::declare::format_array_for_output(storage)
            )?;
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        // GNU variables.c:1096-1104 print_assignment: array and assoc cells
        // print through array_to_assign/assoc_to_assign (hash-bucket order,
        // per-element quoting, unquoted parens), not the scalar
        // sh_single_quote path — `set` prints myarray=(["a]a"]="abc" ).
        // var_isset (variables.c:1912) is `var->value != 0`: `declare -A x`
        // marks x assoc without binding a cell, so print_assignment skips it
        // entirely — only an explicit `x=()` binds the empty table.
        if value.is_empty()
            && (unset::is_marked_variable(env_vars, ASSOC_VARS, name)
                || unset::is_marked_variable(env_vars, ARRAY_VARS, name))
        {
            continue;
        }
        if unset::is_marked_variable(env_vars, ASSOC_VARS, name) {
            let nbuckets = crate::executor::assoc_nbuckets(env_vars, name);
            writeln!(
                stdout,
                "{}={}",
                name,
                crate::builtins::declare::format_assoc_for_output(value, nbuckets)
            )?;
            continue;
        }
        if unset::is_marked_variable(env_vars, ARRAY_VARS, name) {
            writeln!(
                stdout,
                "{}={}",
                name,
                crate::builtins::declare::format_array_for_output(value)
            )?;
            continue;
        }
        writeln!(stdout, "{}={}", name, shell_quote(value, posix_mode))?;
    }

    Ok(())
}

fn shell_quote(value: &str, posix_mode: bool) -> String {
    // GNU variables.c print_var_value: dollar-quote for values containing
    // non-printing characters (ansic_shouldquote), sh_single_quote for
    // values containing shell metacharacters (sh_contains_shell_metas),
    // bare otherwise. sh_single_quote special-cases a value that is
    // exactly one quote and prints it as backslash-quote (posix2.tests
    // variable quoting 1/3). The dollar-quote branch is gated on
    // posixly_correct == 0 (variables.c:1131): POSIX mode single-quotes
    // the raw value instead (audit B4-F11).
    //
    // The ansic pair must be the shared byte-stream ports
    // (executor::ansic_shouldquote / ansic_quote, strtrans.c:341/230):
    // GNU tests the raw bytes, and in rubash storage part of the C0
    // range and raw high bytes ride as U+E000 raw-byte marker pairs,
    // which a char-level scan of the stored text reads as printable
    // PUA — the ports decode the pairs before judging (rubash#3017).
    // The declare/array faces already render through the same ports.
    if !posix_mode && crate::executor::ansic_shouldquote(value) {
        return crate::executor::ansic_quote(value);
    }
    if !contains_shell_metas(value) {
        return value.to_string();
    }
    if value == "'" {
        return "\\'".to_string();
    }
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// GNU sh_contains_shell_metas: IFS whitespace, quoting characters, shell
/// metacharacters, reserved-word braces, globbing characters, expansion
/// characters; tilde only at the start or after =/:; hash only at the start.
fn contains_shell_metas(value: &str) -> bool {
    let bytes = value.as_bytes();
    for (index, ch) in value.char_indices() {
        match ch {
            ' ' | '\t' | '\n' | '\'' | '"' | '\\' | '|' | '&' | ';' | '(' | ')' | '<' | '>'
            | '!' | '{' | '}' | '*' | '[' | '?' | ']' | '^' | '$' | '\u{60}' => return true,
            '~' => {
                if index == 0 || (index > 0 && matches!(bytes[index - 1], b'=' | b':')) {
                    return true;
                }
            }
            '#' => {
                if index == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], env_vars: &mut crate::shell::var_table::VarTable) -> (i32, String) {
        let mut stderr = Vec::new();
        let status = unset_with_stderr(args.iter().copied(), env_vars, &mut stderr).unwrap();
        (status, String::from_utf8(stderr).unwrap())
    }

    fn run_set(
        args: &[&str],
        env_vars: &crate::shell::var_table::VarTable,
    ) -> (i32, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut env_vars = env_vars.clone();
        let status = run_set_with_io(args, &mut env_vars, &mut stdout, &mut stderr);
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    /// The migrated call shape: build the row + Diag, call, flush
    /// with a fixed test prolog (`t.sh: line 1: `).
    fn run_set_with_io(
        args: &[&str],
        env_vars: &mut crate::shell::var_table::VarTable,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let row = crate::executor::builtin_table::builtin_table_row("set").unwrap();
        let words: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut diag = Diag::new(row, stderr);
        let status = set_with_io(
            row,
            &words,
            env_vars,
            &mut diag,
            stdout,
            &crate::shell::var_table::TempenvListingUnderlay::new(),
        )
        .unwrap();
        diag.flush_with(|| Prolog {
            interactive: false,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: Some("t.sh".to_string()),
            line: Some("1".to_string()),
        });
        status
    }

    #[test]
    fn set_without_arguments_prints_variables() {
        let env_vars = crate::shell::var_table::VarTable::from_values(HashMap::from([(
            "NAME".to_string(),
            "value".to_string(),
        )]));
        let (status, stdout, stderr) = run_set(&[], &env_vars);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "NAME=value\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn set_rejects_unknown_flag() {
        let env_vars = crate::shell::var_table::VarTable::default();
        let (status, _stdout, stderr) = run_set(&["-Z"], &env_vars);

        assert_eq!(status, EX_USAGE);
        assert!(stderr.contains("invalid option"));
    }

    #[test]
    fn set_o_invalid_name_is_usage_error() {
        let env_vars = crate::shell::var_table::VarTable::default();
        let (status, _stdout, stderr) = run_set(&["-o", "no_such"], &env_vars);

        assert_eq!(status, EX_USAGE);
        assert!(stderr.contains("invalid option name"));
    }

    #[test]
    fn restricted_short_option_is_enabled() {
        let mut env_vars = crate::shell::var_table::VarTable::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_set_with_io(&["-r"], &mut env_vars, &mut stdout, &mut stderr),
            EXECUTION_SUCCESS
        );
        assert!(shell_option_enabled(&env_vars, "restricted"));
    }

    #[test]
    fn restricted_option_cannot_be_unset() {
        let mut env_vars = crate::shell::var_table::VarTable::default();
        set_shell_option(&mut env_vars, "restricted", true);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_set_with_io(&["+r"], &mut env_vars, &mut stdout, &mut stderr),
            EXECUTION_FAILURE
        );
        assert!(shell_option_enabled(&env_vars, "restricted"));
    }

    #[test]
    fn unsets_variable() {
        let mut env_vars = crate::shell::var_table::VarTable::from_values(HashMap::from([(
            "NAME".to_string(),
            "value".to_string(),
        )]));

        assert_eq!(run(&["NAME"], &mut env_vars).0, EXECUTION_SUCCESS);
        assert!(!env_vars.contains_key("NAME"));
    }

    #[test]
    fn rejects_invalid_identifier_for_variable_unset() {
        // GNU builtins/set.def:899-920: `unset -v 1BAD` reports sh_invalidid
        // ("`1BAD': not a valid identifier") and fails; without -v the invalid
        // name is treated as a potential function name and unset silently.
        // The raw status is EX_UTILERROR (263 > EX_SHERRBASE); the executor
        // converts it to EXECUTION_FAILURE (1) via builtin_status.
        let mut env_vars = crate::shell::var_table::VarTable::default();
        let (status, stderr) = run(&["-v", "1BAD"], &mut env_vars);

        assert_eq!(status, super::unset::EX_UTILERROR);
        assert!(stderr.contains("`1BAD': not a valid identifier"));

        let (silent_status, silent_stderr) = run(&["1BAD"], &mut env_vars);
        assert_eq!(silent_status, EXECUTION_SUCCESS);
        assert!(silent_stderr.is_empty());
    }

    #[test]
    fn rejects_function_and_variable_modes_together() {
        let mut env_vars = crate::shell::var_table::VarTable::default();
        let (status, stderr) = run(&["-fv", "NAME"], &mut env_vars);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stderr.contains("cannot simultaneously"));
    }
}
