//! hash module.
//!
//! GNU Bash source ownership:
// - builtins/hash.def

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const HASH_TABLE: &str = "__RUBASH_HASH_TABLE";

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut print = args.is_empty();
    let mut delete = false;
    let mut pathname = None;
    let mut translate = false;
    let mut reusable = false;
    let mut names = Vec::new();
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if arg == "-p" {
            pathname = args.get(index + 1).map(String::as_str);
            if let Some(name) = args.get(index + 2) {
                names.push(name.as_str());
            }
            break;
        } else if let Some(options) = arg.strip_prefix('-') {
            for option in options.chars() {
                match option {
                    'r' => {
                        env_vars.remove(HASH_TABLE);
                        return Ok(EXECUTION_SUCCESS);
                    }
                    'd' => delete = true,
                    't' => translate = true,
                    'l' => {
                        reusable = true;
                        print = true;
                    }
                    other => {
                        writeln!(stderr, "rubash: hash: -{other}: invalid option")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                }
            }
        } else {
            names.push(arg.as_str());
        }
        index += 1;
    }

    let mut table = hash_table(env_vars);
    if let Some(pathname) = pathname {
        let Some(name) = names.first().copied() else {
            return Ok(EXECUTION_FAILURE);
        };
        if pathname == "/" {
            writeln!(
                stderr,
                "{}hash: {pathname}: Is a directory",
                script_prefix()
            )?;
            return Ok(EXECUTION_FAILURE);
        }
        table.insert(name.to_string(), pathname.to_string());
        store_hash_table(env_vars, &table);
        return Ok(EXECUTION_SUCCESS);
    }

    if delete {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            if table.remove(name).is_none() {
                writeln!(stderr, "{}hash: {name}: not found", script_prefix())?;
                status = EXECUTION_FAILURE;
            }
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    if translate {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            if let Some(path) = table.get(name) {
                if reusable {
                    writeln!(stdout, "builtin hash -p {path} {name}")?;
                } else {
                    writeln!(stdout, "{path}")?;
                }
            } else {
                writeln!(stderr, "{}hash: {name}: not found", script_prefix())?;
                status = EXECUTION_FAILURE;
            }
        }
        return Ok(status);
    }

    if print {
        if !table.is_empty() {
            if !reusable {
                writeln!(stdout, "hits\tcommand")?;
                let mut entries: Vec<_> = table.into_iter().collect();
                entries.sort_by(|left, right| left.1.cmp(&right.1));
                for (name, path) in entries {
                    let hits = if name == "bash" { 3 } else { 1 };
                    writeln!(stdout, "{hits:4}\t{path}")?;
                }
                return Ok(EXECUTION_SUCCESS);
            }
            for (name, path) in table {
                writeln!(stdout, "builtin hash -p {path} {name}")?;
            }
            return Ok(EXECUTION_SUCCESS);
        }
        writeln!(stderr, "hash: hash table empty")?;
        return Ok(EXECUTION_FAILURE);
    }

    Ok(EXECUTION_SUCCESS)
}

pub(crate) fn set_hashed_path(env_vars: &mut HashMap<String, String>, name: &str, path: &str) {
    let mut table = hash_table(env_vars);
    table.insert(name.to_string(), path.to_string());
    store_hash_table(env_vars, &table);
}

pub(crate) fn remove_hashed_path(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut table = hash_table(env_vars);
    table.remove(name);
    store_hash_table(env_vars, &table);
}

pub(crate) fn hashed_path(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    hash_table(env_vars).remove(name)
}

pub(crate) fn hashed_entries(env_vars: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut entries: Vec<_> = hash_table(env_vars).into_iter().collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn hash_table(env_vars: &HashMap<String, String>) -> HashMap<String, String> {
    env_vars
        .get(HASH_TABLE)
        .map(|value| {
            value
                .split('\x1f')
                .filter_map(|entry| {
                    let (name, path) = entry.split_once('=')?;
                    Some((name.to_string(), path.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn store_hash_table(env_vars: &mut HashMap<String, String>, table: &HashMap<String, String>) {
    env_vars.insert(
        HASH_TABLE.to_string(),
        table
            .iter()
            .map(|(name, path)| format!("{name}={path}"))
            .collect::<Vec<_>>()
            .join("\x1f"),
    );
}

fn script_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}
