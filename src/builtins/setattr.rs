//! `export` and `readonly` attribute builtins.
//!
//! GNU Bash source ownership:
//! - builtins/setattr.def (`export_builtin`)

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportMode {
    Set,
    Unset,
}

/// Execute `export` with arguments after the command name.
pub fn export(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    export_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

fn export_with_io<'a, I, W, E>(
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
    let mut mode = ExportMode::Set;
    let mut print = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'n' => mode = ExportMode::Unset,
                'p' => print = true,
                'f' => {
                    writeln!(
                        stderr,
                        "rubash: export: -f: shell functions are not supported yet"
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
                other => {
                    writeln!(stderr, "rubash: export: -{}: invalid option", other)?;
                    writeln!(
                        stderr,
                        "export: usage: export [-fn] [name[=value] ...] or export -p"
                    )?;
                    return Ok(EX_USAGE);
                }
            }
        }

        index += 1;
    }

    if index >= args.len() || print {
        print_exported(env_vars, stdout)?;
        if index >= args.len() {
            return Ok(EXECUTION_SUCCESS);
        }
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in &args[index..] {
        if apply_export_arg(arg, mode, env_vars, stderr)? != EXECUTION_SUCCESS {
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn apply_export_arg<W>(
    arg: &str,
    mode: ExportMode,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, value) = split_assignment(arg);
    if !valid_identifier(name) {
        writeln!(stderr, "rubash: export: `{}`: not a valid identifier", arg)?;
        return Ok(EXECUTION_FAILURE);
    }

    match mode {
        ExportMode::Set => {
            let value = value
                .map(str::to_string)
                .or_else(|| env_vars.get(name).cloned())
                .or_else(|| env::var(name).ok())
                .unwrap_or_default();
            env_vars.insert(name.to_string(), value.clone());
            env::set_var(name, value);
            mark_exported(env_vars, name);
        }
        ExportMode::Unset => {
            env_vars.remove(name);
            env::remove_var(name);
            unmark_exported(env_vars, name);
        }
    }

    Ok(EXECUTION_SUCCESS)
}

fn print_exported<W>(env_vars: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let mut vars: Vec<(&String, &String)> = env_vars.iter().collect();
    vars.sort_by(|left, right| left.0.cmp(right.0));

    for (name, value) in vars {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        writeln!(
            stdout,
            "declare -x {}=\"{}\"",
            name,
            quote_export_value(value)
        )?;
    }

    Ok(())
}

fn mark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = exported_vars(env_vars);
    exported.insert(name.to_string());
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

fn unmark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = exported_vars(env_vars);
    exported.remove(name);
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

fn exported_vars(env_vars: &HashMap<String, String>) -> std::collections::HashSet<String> {
    env_vars
        .get(EXPORTED_VARS)
        .map(|value| {
            value
                .split('\x1f')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn split_assignment(arg: &str) -> (&str, Option<&str>) {
    match arg.find('=') {
        Some(index) => (&arg[..index], Some(&arg[index + 1..])),
        None => (arg, None),
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

fn quote_export_value(value: &str) -> String {
    let mut quoted = String::new();
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn exports_assignment() {
        let mut vars = env_map();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let status = export_with_io(["NAME=value"], &mut vars, &mut stdout, &mut stderr).unwrap();

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(vars.get("NAME"), Some(&"value".to_string()));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn rejects_invalid_identifier() {
        let mut vars = env_map();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let status = export_with_io(["1BAD=value"], &mut vars, &mut stdout, &mut stderr).unwrap();

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(String::from_utf8(stderr).unwrap().contains("not a valid identifier"));
    }

    #[test]
    fn prints_exported_variables() {
        let mut vars = env_map();
        vars.insert("NAME".to_string(), "value".to_string());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let status = export_with_io(["-p"], &mut vars, &mut stdout, &mut stderr).unwrap();

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "declare -x NAME=\"value\"\n"
        );
        assert!(stderr.is_empty());
    }
}
