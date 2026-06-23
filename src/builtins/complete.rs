//! complete module.
//!
//! GNU Bash source ownership:
// - builtins/complete.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionBuiltin {
    Complete,
    Compgen,
    Compopt,
}

pub fn execute_with_io<E>(
    builtin: CompletionBuiltin,
    args: &[String],
    diagnostic_prefix: &str,
    stdout: &mut E,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    match builtin {
        CompletionBuiltin::Complete => execute_complete(args, diagnostic_prefix, stdout, stderr),
        CompletionBuiltin::Compgen => execute_compgen(args, diagnostic_prefix, stdout, stderr),
        CompletionBuiltin::Compopt => execute_compopt(args, diagnostic_prefix, stderr),
    }
}

fn execute_complete<E>(
    args: &[String],
    diagnostic_prefix: &str,
    _stdout: &mut E,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    parse_options(
        CompletionBuiltin::Complete,
        args,
        "abcdefgjksuvprDEI",
        "oAGWFCXPS",
        diagnostic_prefix,
        stderr,
    )
}

fn execute_compgen<E>(
    args: &[String],
    diagnostic_prefix: &str,
    _stdout: &mut E,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    parse_options(
        CompletionBuiltin::Compgen,
        args,
        "abcdefgjksuv",
        "oAGWFCXPS",
        diagnostic_prefix,
        stderr,
    )
}

fn execute_compopt<E>(args: &[String], diagnostic_prefix: &str, stderr: &mut E) -> io::Result<i32>
where
    E: Write,
{
    let status = parse_compopt_options(args, diagnostic_prefix, stderr)?;
    if status != EXECUTION_SUCCESS {
        return Ok(status);
    }

    writeln!(
        stderr,
        "{diagnostic_prefix}compopt: not currently executing completion function"
    )?;
    Ok(EXECUTION_FAILURE)
}

fn parse_options<E>(
    builtin: CompletionBuiltin,
    args: &[String],
    flag_options: &str,
    arg_options: &str,
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let name = builtin.name();
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        let mut chars = arg[1..].chars().peekable();
        while let Some(option) = chars.next() {
            if flag_options.contains(option) {
                continue;
            }
            if arg_options.contains(option) {
                if chars.peek().is_some() {
                    break;
                }
                index += 1;
                if args.get(index).is_none() {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}{name}: -{option}: option requires an argument"
                    )?;
                    write_usage(builtin, stderr)?;
                    return Ok(EX_USAGE);
                }
                break;
            }

            writeln!(
                stderr,
                "{diagnostic_prefix}{name}: -{option}: invalid option"
            )?;
            write_usage(builtin, stderr)?;
            return Ok(EX_USAGE);
        }
        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

fn parse_compopt_options<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if !arg.starts_with('-') && !arg.starts_with("+o") || arg == "-" {
            break;
        }

        if let Some(rest) = arg.strip_prefix("+o") {
            if rest.is_empty() {
                index += 1;
                if args.get(index).is_none() {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}compopt: +o: option requires an argument"
                    )?;
                    write_usage(CompletionBuiltin::Compopt, stderr)?;
                    return Ok(EX_USAGE);
                }
            }
            index += 1;
            continue;
        }

        let mut chars = arg[1..].chars().peekable();
        while let Some(option) = chars.next() {
            match option {
                'D' | 'E' | 'I' => {}
                'o' => {
                    if chars.peek().is_none() {
                        index += 1;
                        if args.get(index).is_none() {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}compopt: -o: option requires an argument"
                            )?;
                            write_usage(CompletionBuiltin::Compopt, stderr)?;
                            return Ok(EX_USAGE);
                        }
                    }
                    break;
                }
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}compopt: -{other}: invalid option"
                    )?;
                    write_usage(CompletionBuiltin::Compopt, stderr)?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

impl CompletionBuiltin {
    fn name(self) -> &'static str {
        match self {
            CompletionBuiltin::Complete => "complete",
            CompletionBuiltin::Compgen => "compgen",
            CompletionBuiltin::Compopt => "compopt",
        }
    }
}

fn write_usage<E>(builtin: CompletionBuiltin, stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    let usage = match builtin {
        CompletionBuiltin::Complete => {
            "complete: usage: complete [-abcdefgjksuv] [-pr] [-DEI] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [name ...]"
        }
        CompletionBuiltin::Compgen => {
            "compgen: usage: compgen [-abcdefgjksuv] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [word]"
        }
        CompletionBuiltin::Compopt => {
            "compopt: usage: compopt [-o|+o option] [-DEI] [name ...]"
        }
    };
    writeln!(stderr, "{usage}")
}
