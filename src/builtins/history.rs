//! history module.
//!
//! GNU Bash source ownership:
// - builtins/history.def
// - builtins/common.c get_numeric_arg() / no_args() / sh_neednumarg() /
//   sh_invalidnum() / sh_erange()

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<(i32, bool)>
where
    E: Write,
{
    let mut surplus = false;
    let status = execute_with_history(
        args,
        diagnostic_prefix,
        &[],
        1,
        &mut io::sink(),
        stderr,
        &mut surplus,
    )?;
    Ok((status, surplus))
}

pub fn execute_with_history<E, O>(
    args: &[String],
    diagnostic_prefix: &str,
    entries: &[String],
    history_base: usize,
    stdout: &mut O,
    stderr: &mut E,
    surplus: &mut bool,
) -> io::Result<i32>
where
    E: Write,
    O: Write,
{
    let mut index = 0;
    let mut file_flags = 0_u8;
    let mut delete_arg: Option<String> = None;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        for (position, option) in arg[1..].char_indices() {
            match option {
                'a' => file_flags |= 0x01,
                'n' => file_flags |= 0x02,
                'r' => file_flags |= 0x04,
                'w' => file_flags |= 0x08,
                'c' => {}
                'd' => {
                    // internal_getopt("acd:npsrw"): -d's optarg is the rest
                    // of the cluster when attached (-d5), else the next word.
                    let rest = &arg[position + 2..];
                    if rest.is_empty() {
                        index += 1;
                        match args.get(index) {
                            Some(value) => delete_arg = Some(value.clone()),
                            None => {
                                writeln!(
                                    stderr,
                                    "{diagnostic_prefix}history: -d: option requires an argument"
                                )?;
                                write_usage(stderr)?;
                                return Ok(EX_USAGE);
                            }
                        }
                    } else {
                        delete_arg = Some(rest.to_string());
                    }
                    break;
                }
                'p' | 's' => {
                    return Ok(EXECUTION_SUCCESS);
                }
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}history: -{other}: invalid option"
                    )?;
                    write_usage(stderr)?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    // history.def:145: more than one of -anrw is a hard failure.
    if file_flags.count_ones() > 1 {
        writeln!(
            stderr,
            "{diagnostic_prefix}history: cannot use more than one of -anrw"
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    // history.def:164-264: -d validates its offset or inclusive range
    // before deleting. The executor's provider path performs in-range
    // deletions; this layer reports malformed arguments (sh_erange /
    // sh_invalidnum) and lets well-formed ones through.
    if let Some(arg) = &delete_arg {
        let body = arg.strip_prefix('-').unwrap_or(arg);
        if let Some(separator) = body.find('-') {
            let (start, end) = (&body[..separator], &body[separator + 1..]);
            if !is_valid_number(start) || !is_valid_number(end) {
                writeln!(
                    stderr,
                    "{diagnostic_prefix}history: {arg}: history position out of range"
                )?;
                return Ok(EXECUTION_FAILURE);
            }
        } else if !is_valid_number(arg) {
            writeln!(stderr, "{diagnostic_prefix}history: {arg}: invalid number")?;
            return Ok(EXECUTION_FAILURE);
        }
        return Ok(EXECUTION_SUCCESS);
    }

    if file_flags != 0 {
        return Ok(EXECUTION_SUCCESS);
    }

    // display_history -> get_numeric_arg(list, 0, &limit): the operand is
    // validated as a number first (sh_neednumarg), then no_args() rejects
    // any further operands with `too many arguments`. no_args jumps to
    // DISCARD, so the listing never runs on the error paths.
    if let Some(arg) = args.get(index) {
        if !is_valid_number(arg) {
            writeln!(
                stderr,
                "{diagnostic_prefix}history: {arg}: numeric argument required"
            )?;
            return Ok(EX_USAGE);
        }
        if args.get(index + 1).is_some() {
            writeln!(stderr, "{diagnostic_prefix}history: too many arguments")?;
            // rubash#1660: the no_args class — the executor arms the
            // reader-level DISCARD for this surplus (see the session
            // path in history_exec.rs); the other EX_USAGE paths
            // (numeric argument required, invalid option) keep a
            // plain status with no discard.
            *surplus = true;
            return Ok(EX_USAGE);
        }
    }

    let limit = args
        .get(index)
        .and_then(|arg| arg.trim_start().parse::<i64>().ok())
        .map(|n| n.unsigned_abs() as usize);
    let start = limit.map(|n| entries.len().saturating_sub(n)).unwrap_or(0);
    for (number, entry) in entries.iter().enumerate().skip(start) {
        // display_history numbers from history_base (lib/readline/history.c):
        // the session path mirrors it with shell.base (history_exec.rs); the
        // provider path carries the host's base through this parameter — a
        // stifled host store keeps its GLOBAL numbers (rubash#2007).
        writeln!(stdout, "{:>5}  {}", number + history_base, entry)?;
    }

    Ok(EXECUTION_SUCCESS)
}

/// builtins/common.c valid_number(): strtoimax semantics — leading
/// whitespace and a sign are accepted, the whole remainder must parse.
fn is_valid_number(text: &str) -> bool {
    !text.is_empty() && text.trim_start().parse::<i64>().is_ok()
}

fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(
        stderr,
        "history: usage: history [-c] [-d offset] [n] or history -anrw [filename] or history -ps arg [arg...]"
    )
}
