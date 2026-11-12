//! alias module.
//!
//! GNU Bash source ownership:
//! - alias.c (`add_alias`, `remove_alias`, `all_aliases`)
//! - alias.h (`alias_t`, `AL_EXPANDNEXT`, `AL_BEINGEXPANDED`)
//! - builtins/alias.def (`alias_builtin`, `unalias_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

use crate::executor::builtin_diag::{Diag, Prolog};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::builtin_table_row;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

/// The lib-level `Diag` prolog for the standalone entries below.
/// `alias`/`unalias` take no variable table, so this reads the
/// process-env cells the deleted `diagnostic_prefix` read
/// (`__RUBASH_SCRIPT_NAME`, `__RUBASH_CURRENT_LINE`); the name
/// cells derive from the single argv[0] carrier chain
/// (builtins::prefix, rubash#2791) — the old cells printed a
/// hard-coded "rubash" for both (rubash#3256). The executor
/// dispatch does NOT use this prolog: it hands its own
/// `Executor::diagnostic_prolog()` into the `with_io` entries, so
/// a session's diagnostics carry the executor's composed frame
/// (including the interactive arm, whose carrier never reaches
/// the process env — rubash#2259).
fn diag_prolog() -> Prolog {
    // rubash#3355 + the #3356 carrier-chain convergence, merged:
    // error_name follows the get_name_for_error chain, not the
    // hardcoded "rubash" brand — an empty $0 prints an empty name.
    // alias takes no variable table, so the chain is composed here
    // from the process-env carriers (the script-file arm, else
    // prefix::fallback_error_name's presence-based $0 step), while
    // shell_basename keeps the shared basename derivation (the
    // verbatim carrier printed a renamed shell's full path).
    let get = |key: &str| std::env::var(key).ok();
    let script_file = std::env::var("__RUBASH_SCRIPT_NAME").ok();
    let error_name = match script_file.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => name.to_string(),
        None => crate::builtins::prefix::fallback_error_name(&get),
    };
    Prolog {
        interactive: std::env::var("__RUBASH_INTERACTIVE").is_ok(),
        shell_basename: crate::builtins::prefix::shell_basename(&get),
        error_name,
        script_file,
        line: std::env::var("__RUBASH_CURRENT_LINE").ok(),
    }
}

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
    alias_with_io(
        args,
        aliases,
        &mut stdout,
        &mut stderr,
        false,
        diag_prolog(),
    )
}

pub fn unalias(args: &[String], aliases: &mut HashMap<String, Alias>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    unalias_with_io(args, aliases, &mut stdout, &mut stderr, diag_prolog())
}

pub(crate) fn alias_with_io<W>(
    args: &[String],
    aliases: &mut HashMap<String, Alias>,
    stdout: &mut W,
    stderr: &mut dyn crate::executor::builtin_diag::DiagSink,
    posix: bool,
    prolog: Prolog,
) -> io::Result<i32>
where
    W: Write,
{
    // The Diag is built here, over the caller's stderr (the same
    // shape as setattr's with_io fns): the executor passes this
    // command's own buffered stderr.
    let row = builtin_table_row("alias").expect("alias row exists");
    let mut diag = Diag::new(row, stderr);
    // GNU alias.def scans with internal_getopt over "p" (the row's
    // spec); the single BuiltinOpt machine replaces the hand scan
    // here (Stage-2 ④ Phase 4).
    let mut pflag = false;
    let args: Vec<&str> = {
        let mut opt = BuiltinOpt::new(row, args, &mut diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    crate::builtins::help::print_builtin_help(row.name, stdout)?;
                    return Ok(EX_USAGE);
                }
                OptEvent::End => break,
                OptEvent::Opt(c) => {
                    if c == b'p' {
                        pflag = true;
                    }
                }
                OptEvent::OptArg(_, _) => {}
            }
        }
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        drop(opt);
        if diag.status() != 0 {
            diag.usage();
            diag.flush_with(|| prolog.clone());
            return Ok(diag.status());
        }
        operands
    };
    // Bash posix-mode notes (item 47): the alias listing omits the leading
    // "alias" unless -p is given. `alias -p` keeps the full prefix in every
    // mode; the bare list and the name-query form drop it only in POSIX
    // mode (the default mode keeps `alias name='value'` everywhere). With
    // operand names the -p flag only picks the prefixed spelling — the
    // listing branch runs solely for the nameless invocation, exactly like
    // GNU setattr.def's loptend split.
    let include_prefix = pflag || !posix;

    // rubash#549 (cell 2): GNU bash 5.3 turned `alias -p NAME` into a pure
    // whole-table listing — the NAME operands are ignored entirely (probe
    // 2026-10-12: `alias zz1=: zz2=:; alias -p zz1` prints exactly the
    // zz1/zz2 pair, rc 0; the old rubash path printed the table AND then
    // the named line, duplicating zz1). `alias NAME` without -p still
    // prints just the named alias.
    if args.is_empty() || pflag {
        print_aliases(aliases, stdout, include_prefix)?;
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in args {
        if let Some((name, value)) = arg.split_once('=') {
            if !valid_alias_name(name) {
                diag.free(format_args!("`{name}': invalid alias name"));
                status = EXECUTION_FAILURE;
                continue;
            }
            aliases.insert(name.to_string(), Alias::new(strip_quote_marker(value)));
        } else if let Some(alias) = aliases.get(arg) {
            print_alias(arg, alias, stdout, include_prefix)?;
        } else {
            diag.free(format_args!("{arg}: not found"));
            status = EXECUTION_FAILURE;
        }
    }

    diag.flush_with(|| prolog.clone());
    Ok(status)
}

pub fn unalias_with_io<W>(
    args: &[String],
    aliases: &mut HashMap<String, Alias>,
    stdout: &mut W,
    stderr: &mut dyn crate::executor::builtin_diag::DiagSink,
    prolog: Prolog,
) -> io::Result<i32>
where
    W: Write,
{
    // Same Diag-over-caller's-stderr shape as alias_with_io above.
    // The no-arguments usage carries NO prolog (common.c:128
    // builtin_usage), so it is `diag.usage()`, not a free line.
    let row = builtin_table_row("unalias").expect("unalias row exists");
    let mut diag = Diag::new(row, stderr);
    if args.is_empty() {
        // GNU builtins/alias.def unalias_builtin: no arguments is a
        // usage error — sh_usage + EX_USAGE (2), the same status the
        // invalid-option arm below returns (rubash#2975). Only a
        // missing alias NAME is EXECUTION_FAILURE. The usage line
        // itself is the Diag one above (no prolog, per the comment
        // on this function); only the STATUS is #2975's.
        diag.usage();
        diag.flush_with(|| prolog.clone());
        return Ok(EX_USAGE);
    }

    let mut clear_all = false;
    let args: Vec<&str> = {
        let mut opt = BuiltinOpt::new(row, args, &mut diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    // GNU unalias.def CASE_HELPOPT: the long help
                    // goes to stdout with EX_USAGE.
                    crate::builtins::help::print_builtin_help(row.name, stdout)?;
                    return Ok(EX_USAGE);
                }
                OptEvent::End => break,
                OptEvent::Opt(c) => {
                    if c == b'a' {
                        clear_all = true;
                    }
                }
                OptEvent::OptArg(_, _) => {}
            }
        }
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        drop(opt);
        if diag.status() != 0 {
            diag.usage();
            diag.flush_with(|| prolog.clone());
            return Ok(diag.status());
        }
        operands
    };

    if clear_all {
        aliases.clear();
        return Ok(EXECUTION_SUCCESS);
    }

    if args.is_empty() {
        // Same usage status as the no-arguments entry above: an
        // option-only invocation (`unalias --`) names no alias
        // (rubash#2975).
        diag.usage();
        diag.flush_with(|| prolog.clone());
        return Ok(EX_USAGE);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in args {
        if aliases.remove(arg).is_none() {
            diag.free(format_args!("{arg}: not found"));
            status = EXECUTION_FAILURE;
        }
    }

    diag.flush_with(|| prolog.clone());
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
            diag_prolog(),
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
        alias_with_io(&[], &mut aliases, &mut out, &mut err, true, diag_prolog()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(
            &["ll".to_string()],
            &mut aliases,
            &mut out,
            &mut err,
            true,
            diag_prolog(),
        )
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(
            &["-p".to_string()],
            &mut aliases,
            &mut out,
            &mut err,
            true,
            diag_prolog(),
        )
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "alias ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(&[], &mut aliases, &mut out, &mut err, false, diag_prolog()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "alias ll='echo LL'\n");

        let mut out = Vec::new();
        let mut err = Vec::new();
        alias_with_io(
            &["ll".to_string()],
            &mut aliases,
            &mut out,
            &mut err,
            false,
            diag_prolog(),
        )
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "alias ll='echo LL'\n");
    }
}
