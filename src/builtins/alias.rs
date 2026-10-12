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
    alias_with_io(args, aliases, &mut stdout, &mut stderr, false)
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
    posix: bool,
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
    // Bash posix-mode notes (item 47): the alias listing omits the leading
    // "alias" unless -p is given. `alias -p` keeps the full prefix in every
    // mode; the bare list and the name-query form drop it only in POSIX
    // mode (the default mode keeps `alias name='value'` everywhere). With
    // operand names the -p flag only picks the prefixed spelling — the
    // listing branch runs solely for the nameless invocation, exactly like
    // GNU setattr.def's loptend split.
    let include_prefix = pflag || !posix;

    if args.is_empty() {
        print_aliases(aliases, stdout, include_prefix)?;
        return Ok(EXECUTION_SUCCESS);
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
            print_alias(arg, alias, stdout, include_prefix)?;
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

fn print_aliases<W>(
    aliases: &HashMap<String, Alias>,
    stdout: &mut W,
    include_prefix: bool,
) -> io::Result<()>
where
    W: Write,
{
    let mut names: Vec<_> = aliases.keys().collect();
    names.sort();
    for name in names {
        if let Some(alias) = aliases.get(name) {
            print_alias(name, alias, stdout, include_prefix)?;
        }
    }
    Ok(())
}

fn print_alias<W>(name: &str, alias: &Alias, stdout: &mut W, include_prefix: bool) -> io::Result<()>
where
    W: Write,
{
    // The `-- ` guard only belongs to the prefixed spelling: a bare
    // POSIX-mode listing has no option word to disambiguate.
    let prefix = if include_prefix {
        if name.starts_with('-') {
            "alias -- "
        } else {
            "alias "
        }
    } else {
        ""
    };
    writeln!(stdout, "{prefix}{}='{}'", name, quote_single(&alias.value))
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
    value
        .strip_prefix(crate::executor::markers::IFS_GLUE)
        .unwrap_or(value)
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
            &[format!(
                "{}{}{}",
                "a=",
                crate::executor::markers::QUOTED_WORD_VALUE_PREFIX_STR,
                "unalias -a\nv=2"
            )
            .to_string()],
            &mut aliases,
            &mut out,
            &mut err,
            false,
        )
        .unwrap();

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(aliases.get("a").unwrap().value, "unalias -a\nv=2");
    }

    /// Bash posix-mode notes item 47: the bare posix listing and the name
    /// query omit the leading "alias " (and the `-- ` guard never applies —
    /// there is no option word to disambiguate); `alias -p` keeps the
    /// prefixed spelling in every mode, and the default mode keeps
    /// `alias name='value'` everywhere.
    #[test]
    fn posix_listing_omits_prefix_unless_p_flag() {
        let mut aliases = HashMap::new();
        aliases.insert("ll".to_string(), Alias::new("echo LL"));

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(&[], &mut aliases, &mut out, &mut err, true).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(&["ll".to_string()], &mut aliases, &mut out, &mut err, true).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(&["-p".to_string()], &mut aliases, &mut out, &mut err, true).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "alias ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(&[], &mut aliases, &mut out, &mut err, false).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "alias ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(&["ll".to_string()], &mut aliases, &mut out, &mut err, false).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "alias ll='echo LL'\n");
    }
}
