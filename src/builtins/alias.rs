//! alias module.
//!
//! GNU Bash source ownership:
//! - alias.c (`add_alias`, `remove_alias`, `all_aliases`)
//! - alias.h (`alias_t`, `AL_EXPANDNEXT`, `AL_BEINGEXPANDED`)
//! - builtins/alias.def (`alias_builtin`, `unalias_builtin`)

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub value: String,
    pub expand_next: bool,
}

impl Alias {
    pub fn new(value: &str) -> Self {
        // TODO(parse.y/alias.c): Bash re-reads alias replacement text through
        // the parser, where backslash-newline pairs are removed before token
        // recognition. Keep this narrow normalization until alias expansion is
        // fully parser-stream based.
        let value = value.replace("\\\n", "");
        Self {
            value: value.to_string(),
            expand_next: value.ends_with(' ') || value.ends_with('\t'),
        }
    }
}

pub fn alias(args: &[String], aliases: &mut HashMap<String, Alias>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    alias_with_io(args, aliases, &mut stdout, &mut stderr)
}

pub fn unalias(args: &[String], aliases: &mut HashMap<String, Alias>) -> io::Result<i32> {
    let mut stderr = io::stderr();
    unalias_with_io(args, aliases, &mut stderr)
}

pub(crate) fn alias_with_io<W, E>(
    args: &[String],
    aliases: &mut HashMap<String, Alias>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut pflag = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'p' => pflag = true,
                other => {
                    writeln!(
                        stderr,
                        "{}alias: -{other}: invalid option",
                        diagnostic_prefix()
                    )?;
                    writeln!(stderr, "alias: usage: alias [-p] [name[=value] ... ]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }
    let args = &args[index..];

    if args.is_empty() || pflag {
        print_aliases(aliases, stdout)?;
        if args.is_empty() {
            return Ok(EXECUTION_SUCCESS);
        }
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in args {
        if let Some((name, value)) = arg.split_once('=') {
            if !valid_alias_name(name) {
                writeln!(
                    stderr,
                    "{}alias: `{name}': invalid alias name",
                    diagnostic_prefix()
                )?;
                status = EXECUTION_FAILURE;
                continue;
            }
            aliases.insert(name.to_string(), Alias::new(strip_quote_marker(value)));
        } else if let Some(alias) = aliases.get(arg) {
            print_alias(arg, alias, stdout)?;
        } else {
            writeln!(stderr, "{}alias: {}: not found", diagnostic_prefix(), arg)?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

pub fn unalias_with_io<E>(
    args: &[String],
    aliases: &mut HashMap<String, Alias>,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    if args.is_empty() {
        writeln!(stderr, "unalias: usage: unalias [-a] name [name ...]")?;
        return Ok(EXECUTION_FAILURE);
    }

    let mut index = 0;
    let mut clear_all = false;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'a' => clear_all = true,
                other => {
                    writeln!(
                        stderr,
                        "{}unalias: -{other}: invalid option",
                        diagnostic_prefix()
                    )?;
                    writeln!(stderr, "unalias: usage: unalias [-a] name [name ...]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    if clear_all {
        aliases.clear();
        return Ok(EXECUTION_SUCCESS);
    }

    let args = &args[index..];
    if args.is_empty() {
        writeln!(stderr, "unalias: usage: unalias [-a] name [name ...]")?;
        return Ok(EXECUTION_FAILURE);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in args {
        if aliases.remove(arg).is_none() {
            writeln!(stderr, "{}unalias: {}: not found", diagnostic_prefix(), arg)?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn print_aliases<W>(aliases: &HashMap<String, Alias>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let mut names: Vec<_> = aliases.keys().collect();
    names.sort();
    for name in names {
        if let Some(alias) = aliases.get(name) {
            print_alias(name, alias, stdout)?;
        }
    }
    Ok(())
}

fn print_alias<W>(name: &str, alias: &Alias, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let prefix = if name.starts_with('-') { "-- " } else { "" };
    writeln!(
        stdout,
        "alias {prefix}{}='{}'",
        name,
        quote_single(&alias.value)
    )
}

fn valid_alias_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '/' | '$' | '`' | '"' | '\'' | '\\' | '(' | ')' | '<' | '>' | '&' | '|'
                )
        })
}

fn quote_single(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn strip_quote_marker(value: &str) -> &str {
    value.strip_prefix('\x1c').unwrap_or(value)
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        env::var("__RUBASH_SCRIPT_NAME"),
        env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_internal_quote_marker_from_alias_value() {
        let mut aliases = HashMap::new();
        let mut out = Vec::new();
        let mut err = Vec::new();

        let status = alias_with_io(
            &["a=\x1cunalias -a\nv=2".to_string()],
            &mut aliases,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(aliases.get("a").unwrap().value, "unalias -a\nv=2");
    }
}
