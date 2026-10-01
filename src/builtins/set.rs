//! `set` and `unset` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/set.def (`set_builtin`, `unset_builtin`)

mod options;
mod unset;

pub(crate) use options::{
    is_shell_option, print_shell_option, print_shell_options, print_shell_options_by_state,
    set_shell_option, shell_option_enabled, shell_option_names, shellopts_value,
    sync_shell_option_flag, SET_O_PRINT_WIDTH, SHOPT_O_PRINT_WIDTH,
};
pub use unset::unset;
pub(crate) use unset::unset_with_stderr;
use unset::valid_identifier;

use std::collections::HashMap;
use std::io::{self, Write};

pub(super) const EXECUTION_SUCCESS: i32 = 0;
pub(super) const EXECUTION_FAILURE: i32 = 1;
pub(crate) const EX_USAGE: i32 = 2;

const SET_FLAGS: &str = "abefhkmnprtuvxBCEHPT";
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

/// Execute `set` with arguments after the command name.
pub fn set(args: &[String], env_vars: &mut crate::shell::var_table::VarTable) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    set_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
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

pub(crate) fn set_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();

    if args.is_empty() {
        print_shell_variables(env_vars, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    // GNU set.def:671-691 pre-scan: internal_getopt (bashgetopt.c) walks the
    // words with optflags (flags.c:372-382 = shell_flags letters + "o;")
    // and validates every flag word BEFORE set_builtin's apply loop runs,
    // so `set -e -Z` reports the error with errexit still off. The scan
    // stops at the first non-option word and at `--`/`-` (bashgetopt.c:76-90
    // NOTOPT / "--" handling), swallows an inline `o` operand without
    // validating its characters (`-opipefail`: the scan sees only 'o'), and
    // consumes a following non-option word (`-o pipefail`) so a bad flag
    // AFTER the operand is still a scan error with nothing applied.
    {
        let mut scan = args.iter().peekable();
        while let Some(arg) = scan.next() {
            let arg: &str = arg;
            if arg == "--" || arg == "-" {
                break;
            }
            let Some(prefix) = arg.chars().next().filter(|ch| *ch == '-' || *ch == '+') else {
                break;
            };
            let flags = &arg[1..];
            if flags.is_empty() {
                break;
            }
            let mut chars = flags.chars();
            while let Some(flag) = chars.next() {
                if flag == 'o' {
                    // bashgetopt.c:111-137: an inline operand consumes the
                    // rest of the word unvalidated; otherwise a following
                    // NOTOPT word is the operand.
                    if chars.next().is_some() {
                        break;
                    }
                    if let Some(next) = scan.peek() {
                        let notopt =
                            (!next.starts_with('-') && !next.starts_with('+')) || next.len() == 1;
                        if notopt {
                            scan.next();
                        }
                    }
                    continue;
                }
                if flag == 'i' {
                    // set.def:677-683: `set -i` is explicitly refused.
                    writeln!(
                        stderr,
                        "{}set: {}i: invalid option",
                        builtin_error_prefix(env_vars),
                        prefix
                    )?;
                    writeln!(
                        stderr,
                        "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                    )?;
                    return Ok(EX_USAGE);
                }
                if flag == '?' {
                    // bashgetopt.c:99-101 prints the invalid-option line for
                    // '?' itself; set.def:684-686 turns list_optopt=='?'
                    // into EXECUTION_SUCCESS.
                    writeln!(
                        stderr,
                        "{}set: {}?: invalid option",
                        builtin_error_prefix(env_vars),
                        prefix
                    )?;
                    writeln!(
                        stderr,
                        "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                    )?;
                    return Ok(EXECUTION_SUCCESS);
                }
                if !SET_FLAGS.contains(flag) {
                    // bashgetopt.c:99-101 sh_invalidopt + '?' return;
                    // set.def:685-686 maps it to EX_USAGE.
                    writeln!(
                        stderr,
                        "{}set: {}{}: invalid option",
                        builtin_error_prefix(env_vars),
                        prefix,
                        flag
                    )?;
                    writeln!(
                        stderr,
                        "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                    )?;
                    return Ok(EX_USAGE);
                }
            }
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
                            writeln!(
                                stderr,
                                "{}set: {}: invalid option name",
                                builtin_error_prefix(env_vars),
                                name
                            )?;
                            // GNU set.def treats an invalid `-o` name as a
                            // usage error, distinct from a valid option that
                            // simply reports failure.
                            return Ok(EX_USAGE);
                        }
                        set_shell_option(env_vars, name, prefix == '-');
                        index += 1;
                    }
                    _ => print_shell_options(env_vars, prefix == '+', SET_O_PRINT_WIDTH, stdout)?,
                }
                // set.def:726-766: the `o` branch continues the character
                // loop, so trailing chars of the word (`-eox pipefail` → x,
                // `-opipefail` → p i p e f a i l) are still applied.
                continue;
            }

            if option == 'r' {
                if prefix == '+' && shell_option_enabled(env_vars, "restricted") {
                    // GNU flags.c change_flag (flags.c:227-235) refuses
                    // "set +r" in a restricted shell with FLAG_ERROR;
                    // set.def:763-771 reports sh_invalidopt("+r") plus the
                    // builtin usage line and returns EXECUTION_FAILURE.
                    writeln!(
                        stderr,
                        "{}set: {}{}: invalid option",
                        builtin_error_prefix(env_vars),
                        prefix,
                        option
                    )?;
                    writeln!(
                        stderr,
                        "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
                set_shell_option(env_vars, "restricted", prefix == '-');
                continue;
            }

            if !APPLY_FLAGS.contains(option) {
                // set.def:763-771: change_flag's FLAG_ERROR path. Only
                // reachable for characters hidden from the pre-scan inside
                // an inline `o` operand (`set -opipefail` dies at 'l').
                writeln!(
                    stderr,
                    "{}set: {}{}: invalid option",
                    builtin_error_prefix(env_vars),
                    prefix,
                    option
                )?;
                writeln!(
                    stderr,
                    "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                )?;
                return Ok(EXECUTION_FAILURE);
            }

            // set.def:767 change_flag: apply every remaining flag char of
            // the word — the branch whose absence dropped `-e`/`-u` bundled
            // before `-o` (rubash#358).
            apply_short_set_flag(env_vars, option, prefix == '-');
        }

        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

fn print_shell_variables<W>(
    env_vars: &crate::shell::var_table::VarTable,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let mut vars: Vec<(&String, &String)> = env_vars.iter().collect();
    vars.sort_by(|left, right| left.0.cmp(right.0));

    for (name, value) in vars {
        // Internal bookkeeping variables (readonly/array/assoc marks, shell
        // option state) are invisible in listings, like GNU's invisible_p
        // vars in print_var_list (variables.c).
        if name.starts_with("__RUBASH_") {
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
        writeln!(stdout, "{}={}", name, shell_quote(value))?;
    }

    Ok(())
}

fn shell_quote(value: &str) -> String {
    // GNU variables.c print_var_value: dollar-quote for values containing
    // non-printing characters (ansic_shouldquote), sh_single_quote for
    // values containing shell metacharacters (sh_contains_shell_metas),
    // bare otherwise. sh_single_quote special-cases a value that is
    // exactly one quote and prints it as backslash-quote (posix2.tests
    // variable quoting 1/3).
    if ansic_shouldquote(value) {
        return ansic_quote_value(value);
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

/// GNU ansic_shouldquote: any non-printing character forces the dollar-quote
/// form. Printable ASCII and printable Unicode text stay unquoted here.
fn ansic_shouldquote(value: &str) -> bool {
    value.chars().any(|ch| ch.is_control())
}

/// GNU ansic_quote with flags=0: dollar-quote wrapping, escaping ESC, the
/// C-style escapes, backslash, and single quote; other non-printing
/// characters become three-digit octal escapes.
fn ansic_quote_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 4);
    out.push_str("$'");
    for ch in value.chars() {
        match ch {
            '\u{1b}' => out.push_str("\\E"),
            '\u{7}' => out.push_str("\\a"),
            '\u{b}' => out.push_str("\\v"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            _ if ch.is_control() => {
                let byte = ch as u32;
                out.push_str(&format!("\\{:03o}", byte));
            }
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
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
        let status = set_with_io(
            args.iter().copied(),
            &mut env_vars,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
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
            set_with_io(["-r"], &mut env_vars, &mut stdout, &mut stderr).unwrap(),
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
            set_with_io(["+r"], &mut env_vars, &mut stdout, &mut stderr).unwrap(),
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
