//! `set` and `unset` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/set.def (`set_builtin`, `unset_builtin`)

mod options;
mod unset;

pub(crate) use options::{
    is_shell_option, print_shell_option, print_shell_options, print_shell_options_by_state,
    set_shell_option, shell_option_enabled, shellopts_value,
};
pub use unset::unset;
pub(crate) use unset::unset_with_stderr;

use std::collections::HashMap;
use std::io::{self, Write};

pub(super) const EXECUTION_SUCCESS: i32 = 0;
pub(super) const EXECUTION_FAILURE: i32 = 1;
pub(super) const EX_USAGE: i32 = 2;

const SET_FLAGS: &str = "abefhkmnprtuvxBCEHPT";
pub(super) const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
pub(super) const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
pub(super) const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub(super) const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
pub(super) const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
pub(super) const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
pub(super) const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
pub(super) const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
pub(super) const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";

/// Execute `set` with arguments after the command name.
pub fn set(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
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
fn builtin_error_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        if env_vars.contains_key("__RUBASH_EVAL_CONTEXT") {
            return format!("{script}: eval: line {line}: ");
        }
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}

pub(crate) fn set_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
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

        let mut chars = options.chars().peekable();
        while let Some(option) = chars.next() {
            if option == 'o' {
                if chars.peek().is_some() {
                    writeln!(stderr, "rubash: set: {}: invalid option", arg)?;
                    writeln!(
                        stderr,
                        "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [arg ...]"
                    )?;
                    return Ok(EX_USAGE);
                }

                match args.get(index + 1) {
                    Some(name)
                        if !name.is_empty() && !name.starts_with('-') && !name.starts_with('+') =>
                    {
                        if !is_shell_option(name) {
                            writeln!(stderr, "rubash: set: {}: invalid option name", name)?;
                            // GNU set.def treats an invalid `-o` name as a
                            // usage error, distinct from a valid option that
                            // simply reports failure.
                            return Ok(EX_USAGE);
                        }
                        if *name == "restricted"
                            && prefix == '+'
                            && shell_option_enabled(env_vars, "restricted")
                        {
                            // GNU set.def set_minus_o_option (set.def:476-501)
                            // -> change_flag FLAG_ERROR (flags.c:227-235) ->
                            // sh_invalidoptname: "invalid option name" with
                            // EX_USAGE; the restriction stays on.
                            writeln!(
                                stderr,
                                "{}set: restricted: invalid option name",
                                builtin_error_prefix(env_vars)
                            )?;
                            return Ok(EX_USAGE);
                        }
                        set_shell_option(env_vars, name, prefix == '-');
                        index += 1;
                    }
                    _ => print_shell_options(env_vars, prefix == '+', stdout)?,
                }
                break;
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
            }

            if !SET_FLAGS.contains(option) {
                writeln!(stderr, "rubash: set: {}{}: invalid option", prefix, option)?;
                writeln!(
                    stderr,
                    "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [arg ...]"
                )?;
                return Ok(EX_USAGE);
            }
        }

        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

fn print_shell_variables<W>(env_vars: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
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

    fn run(args: &[&str], env_vars: &mut HashMap<String, String>) -> (i32, String) {
        let mut stderr = Vec::new();
        let status = unset_with_stderr(args.iter().copied(), env_vars, &mut stderr).unwrap();
        (status, String::from_utf8(stderr).unwrap())
    }

    fn run_set(args: &[&str], env_vars: &HashMap<String, String>) -> (i32, String, String) {
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
        let env_vars = HashMap::from([("NAME".to_string(), "value".to_string())]);
        let (status, stdout, stderr) = run_set(&[], &env_vars);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "NAME=value\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn set_rejects_unknown_flag() {
        let env_vars = HashMap::new();
        let (status, _stdout, stderr) = run_set(&["-Z"], &env_vars);

        assert_eq!(status, EX_USAGE);
        assert!(stderr.contains("invalid option"));
    }

    #[test]
    fn set_o_invalid_name_is_usage_error() {
        let env_vars = HashMap::new();
        let (status, _stdout, stderr) = run_set(&["-o", "no_such"], &env_vars);

        assert_eq!(status, EX_USAGE);
        assert!(stderr.contains("invalid option name"));
    }

    #[test]
    fn restricted_short_option_is_enabled() {
        let mut env_vars = HashMap::new();
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
        let mut env_vars = HashMap::new();
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
        let mut env_vars = HashMap::from([("NAME".to_string(), "value".to_string())]);

        assert_eq!(run(&["NAME"], &mut env_vars).0, EXECUTION_SUCCESS);
        assert!(!env_vars.contains_key("NAME"));
    }

    #[test]
    fn rejects_invalid_identifier_for_variable_unset() {
        // GNU builtins/set.def:899-920: `unset -v 1BAD` reports sh_invalidid
        // ("`1BAD': not a valid identifier") and fails; without -v the invalid
        // name is treated as a potential function name and unset silently.
        let mut env_vars = HashMap::new();
        let (status, stderr) = run(&["-v", "1BAD"], &mut env_vars);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stderr.contains("`1BAD': not a valid identifier"));

        let (silent_status, silent_stderr) = run(&["1BAD"], &mut env_vars);
        assert_eq!(silent_status, EXECUTION_SUCCESS);
        assert!(silent_stderr.is_empty());
    }

    #[test]
    fn rejects_function_and_variable_modes_together() {
        let mut env_vars = HashMap::new();
        let (status, stderr) = run(&["-fv", "NAME"], &mut env_vars);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stderr.contains("cannot simultaneously"));
    }
}
