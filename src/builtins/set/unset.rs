use super::{
    ARRAY_VARS, ASSOC_VARS, DECLARED_UNSET_VARS, EXECUTION_FAILURE, EXECUTION_SUCCESS,
    EXPORTED_VARS, EX_USAGE, INTEGER_VARS, LOWERCASE_VARS, NAMEREF_VARS, READONLY_VARS,
    UPPERCASE_VARS,
};
use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct UnsetOptions {
    functions: bool,
    variables: bool,
    nameref: bool,
}

/// Execute `unset` with arguments after the command name.
pub fn unset(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stderr = io::stderr().lock();
    unset_with_stderr(args.iter().map(String::as_str), env_vars, &mut stderr)
}

pub(crate) fn unset_with_stderr<'a, I, W>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let (options, first_name) = match parse_unset_options(&args, stderr)? {
        Ok(parsed) => parsed,
        Err(status) => return Ok(status),
    };

    if options.functions && options.variables {
        writeln!(
            stderr,
            "rubash: unset: cannot simultaneously unset a function and a variable"
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    let mut status = EXECUTION_SUCCESS;
    for name in &args[first_name..] {
        if unset_name(name, options, env_vars, stderr)? != EXECUTION_SUCCESS {
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn parse_unset_options<W>(
    args: &[&str],
    stderr: &mut W,
) -> io::Result<Result<(UnsetOptions, usize), i32>>
where
    W: Write,
{
    let mut options = UnsetOptions::default();
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            return Ok(Ok((options, index + 1)));
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'f' => options.functions = true,
                'v' => options.variables = true,
                'n' => options.nameref = true,
                other => {
                    writeln!(stderr, "rubash: unset: -{}: invalid option", other)?;
                    writeln!(stderr, "unset: usage: unset [-f] [-v] [-n] [name ...]")?;
                    return Ok(Err(EX_USAGE));
                }
            }
        }

        index += 1;
    }

    Ok(Ok((options, index)))
}

fn unset_name<W>(
    name: &str,
    options: UnsetOptions,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
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
        writeln!(
            stderr,
            "{}unset: {base}: not an array variable",
            diagnostic_prefix(env_vars)
        )?;
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
        // Single write: GNU emits the prologue + message as one stderr
        // record; splitting the write lets a concurrent stdout flush (e.g.
        // under WSL-interop combined capture) tear the prefix off the line.
        let diagnostic = format!(
            "{}unset: `{name}': not a valid identifier\n",
            diagnostic_prefix(env_vars)
        );
        stderr.write_all(diagnostic.as_bytes())?;
        return Ok(EXECUTION_FAILURE);
    }

    // GNU builtins/set.def:990-1010 (unset_builtin): `unset -v` of a nameref
    // follows the reference: a plain identifier cell unbinds the referenced
    // variable and keeps the nameref itself; an array-reference cell unbinds
    // the referenced element and keeps the nameref; a valueless or invalid
    // cell falls back to unbinding the nameref variable itself.
    let nameref_cell: Option<String> = if !options.nameref
        && is_marked_variable(env_vars, NAMEREF_VARS, name)
    {
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
        writeln!(
            stderr,
            "{}unset: {unset_name}: cannot unset",
            diagnostic_prefix(env_vars)
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    if is_marked_variable(env_vars, READONLY_VARS, unset_name) {
        writeln!(
            stderr,
            "{}unset: {unset_name}: cannot unset: readonly variable",
            diagnostic_prefix(env_vars)
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    let unset_name = unset_name.to_string();
    env_vars.remove(&unset_name);
    env::remove_var(&unset_name);
    unmark_variable(env_vars, EXPORTED_VARS, &unset_name);
    unmark_variable(env_vars, READONLY_VARS, &unset_name);
    unmark_variable(env_vars, ARRAY_VARS, &unset_name);
    unmark_variable(env_vars, ASSOC_VARS, &unset_name);
    unmark_variable(env_vars, INTEGER_VARS, &unset_name);
    unmark_variable(env_vars, UPPERCASE_VARS, &unset_name);
    unmark_variable(env_vars, LOWERCASE_VARS, &unset_name);
    unmark_variable(env_vars, NAMEREF_VARS, &unset_name);
    unmark_variable(env_vars, DECLARED_UNSET_VARS, &unset_name);
    Ok(EXECUTION_SUCCESS)
}

fn is_marked_variable(env_vars: &HashMap<String, String>, key: &str, name: &str) -> bool {
    env_vars
        .get(key)
        .map(|value| value.split('\x1f').any(|marked| marked == name))
        .unwrap_or(false)
}

fn unmark_variable(env_vars: &mut HashMap<String, String>, key: &str, name: &str) {
    let Some(value) = env_vars.get(key).cloned() else {
        return;
    };
    let marked = value
        .split('\x1f')
        .filter(|marked| !marked.is_empty() && *marked != name)
        .collect::<Vec<_>>()
        .join("\x1f");
    if marked.is_empty() {
        env_vars.remove(key);
    } else {
        env_vars.insert(key.to_string(), marked);
    }
}

fn valid_identifier(name: &str) -> bool {
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

fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
