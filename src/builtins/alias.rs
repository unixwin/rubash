//! alias module.
//!
//! GNU Bash source ownership:
// - builtins/alias.def

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

pub fn alias(args: &[String], aliases: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    alias_with_io(args, aliases, &mut stdout, &mut stderr)
}

pub fn unalias(args: &[String], aliases: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stderr = io::stderr();
    unalias_with_io(args, aliases, &mut stderr)
}

fn alias_with_io<W, E>(
    args: &[String],
    aliases: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    if args.is_empty() {
        print_aliases(aliases, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in args {
        if let Some((name, value)) = arg.split_once('=') {
            if name.is_empty() {
                writeln!(stderr, "rubash: alias: `{arg}': invalid alias name")?;
                status = EXECUTION_FAILURE;
                continue;
            }
            aliases.insert(name.to_string(), value.to_string());
        } else if let Some(value) = aliases.get(arg) {
            writeln!(stdout, "alias {}='{}'", arg, quote_single(value))?;
        } else {
            writeln!(stderr, "rubash: alias: {}: not found", arg)?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn unalias_with_io<E>(
    args: &[String],
    aliases: &mut HashMap<String, String>,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    if args.is_empty() {
        writeln!(stderr, "unalias: usage: unalias [-a] name [name ...]")?;
        return Ok(EXECUTION_FAILURE);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in args {
        if arg == "-a" {
            aliases.clear();
            continue;
        }

        if aliases.remove(arg).is_none() {
            writeln!(stderr, "rubash: unalias: {}: not found", arg)?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn print_aliases<W>(aliases: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let mut names: Vec<_> = aliases.keys().collect();
    names.sort();
    for name in names {
        if let Some(value) = aliases.get(name) {
            writeln!(stdout, "alias {}='{}'", name, quote_single(value))?;
        }
    }
    Ok(())
}

fn quote_single(value: &str) -> String {
    value.replace('\'', "'\\''")
}

