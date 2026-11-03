//! caller module.
//!
//! GNU Bash source ownership:
//! - builtins/caller.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

pub fn execute_with_io<W, E>(
    args: &[String],
    funcname: &[String],
    lineno: &[String],
    source: &[String],
    diagnostic_prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // GNU builtins/caller.def caller_builtin, in its own order
    // (deep-audit B6-F3):
    // 1. CHECK_HELPOPT (builtins/common.h:29): a leading `--help' word
    //    prints the long help and returns EX_USAGE, before anything else.
    if args.first().is_some_and(|arg| arg == "--help") {
        crate::builtins::help::print_caller_help_with_io(stdout)?;
        return Ok(EX_USAGE);
    }

    // 2. caller.def:94-98: an empty BASH_LINENO or BASH_SOURCE array fails
    //    the builtin silently (EXECUTION_FAILURE) BEFORE any argument is
    //    looked at — at the top level of a -c string every argument form,
    //    valid or not, is a silent status 1.
    if lineno.is_empty() || source.is_empty() {
        return Ok(EXECUTION_FAILURE);
    }

    // 3. no_options (builtins/common.c:157-173): internal_getopt with an
    //    empty optstring scans the words before `--' / the first operand;
    //    any option word is an error. getopt names the option as `-'
    //    plus the word's second character (`-zx' -> `-z', `--foo' ->
    //    `--'); a lone `-' is not an option word, it is the operand.
    let mut index = 0;
    let mut operand: Option<&str> = None;
    while let Some(arg) = args.get(index) {
        let arg = arg.as_str();
        if arg == "--" {
            operand = args.get(index + 1).map(String::as_str);
            break;
        }
        if arg.len() > 1 && arg.starts_with('-') {
            let option = arg.chars().nth(1).unwrap_or('-');
            writeln!(
                stderr,
                "{diagnostic_prefix}caller: -{option}: invalid option"
            )?;
            writeln!(stderr, "caller: usage: caller [expr]")?;
            return Ok(EX_USAGE);
        }
        operand = Some(arg);
        break;
    }

    // 4. No argument: the short form (caller.def:103-105).
    let Some(arg) = operand else {
        return print_current_call(funcname, lineno, source, stdout);
    };

    // 5. valid_number (caller.def:111) gates the frame lookup. The
    //    caller.def:108 FUNCNAME-emptiness check needs no arm of its
    //    own here: it tests the RAW funcname_a, which open_shell_script
    //    (shell.c:1636-1648) keeps non-empty with an invisible "main"
    //    element whenever a script file is being read — so in script
    //    mode the expression IS validated even at the top level, where
    //    the visible FUNCNAME is empty — while the -c/-s top level is
    //    already silenced by the BASH_LINENO/BASH_SOURCE checks above
    //    (both arrays are empty there). print_call_frame's own absence
    //    checks produce the silent status 1 for any valid number whose
    //    frame is not visible.
    //    sh_invalidnum (builtins/common.c:214-224) reports the failure.
    match parse_frame_number(arg) {
        Some(level) if level >= 0 => {
            print_call_frame(level as usize, funcname, lineno, source, stdout)
        }
        // A valid negative number selects no frame: every array
        // reference is absent, so the failure is silent.
        Some(_) => Ok(EXECUTION_FAILURE),
        None => {
            let kind =
                if arg.starts_with('0') && arg.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
                    "invalid octal number"
                } else if arg.starts_with("0x") {
                    "invalid hex number"
                } else {
                    "invalid number"
                };
            writeln!(stderr, "{diagnostic_prefix}caller: {arg}: {kind}")?;
            writeln!(stderr, "caller: usage: caller [expr]")?;
            Ok(EX_USAGE)
        }
    }
}

/// GNU valid_number for the caller expression: strtoimax semantics —
/// leading whitespace is skipped, an optional sign follows, the rest
/// must be decimal digits (trailing whitespace tolerated); overflow is
/// not a valid number.
fn parse_frame_number(arg: &str) -> Option<i64> {
    let trimmed = arg
        .trim_start_matches(|c: char| c.is_whitespace())
        .trim_end_matches(|c: char| c.is_whitespace());
    let digits = trimmed
        .strip_prefix('+')
        .or_else(|| trimmed.strip_prefix('-'))
        .unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<i64>().ok()
}

fn print_current_call<W>(
    funcname: &[String],
    lineno: &[String],
    source: &[String],
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    // GNU builtins/caller.def:89-105: the no-argument form never consults
    // FUNCNAME. It fails only when the BASH_LINENO or BASH_SOURCE array is
    // empty, then prints BASH_LINENO[0] and BASH_SOURCE[1] with a literal
    // NULL for absent elements. At the top level that renders "0 NULL"
    // (dbg-support.tests line 71); inside a function called from line N of
    // file F it renders "N F".
    let _ = funcname;
    if lineno.is_empty() || source.is_empty() {
        return Ok(EXECUTION_FAILURE);
    }
    let line = lineno.first().map(String::as_str).unwrap_or("NULL");
    let caller_source = source.get(1).map(String::as_str).unwrap_or("NULL");
    writeln!(stdout, "{line} {caller_source}")?;
    Ok(EXECUTION_SUCCESS)
}

fn print_call_frame<W>(
    level: usize,
    funcname: &[String],
    lineno: &[String],
    source: &[String],
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    // GNU builtins/caller.def:108-121: with EXPR the frame is
    // BASH_LINENO[expr] FUNCNAME[expr+1] BASH_SOURCE[expr+1]; any absent
    // element fails the builtin with status 1 and no output (no "0" /
    // "environment" fallbacks).
    if funcname.is_empty() {
        return Ok(EXECUTION_FAILURE);
    }
    let Some(line) = lineno.get(level) else {
        return Ok(EXECUTION_FAILURE);
    };
    let Some(function) = funcname.get(level + 1) else {
        return Ok(EXECUTION_FAILURE);
    };
    let Some(caller_source) = source.get(level + 1) else {
        return Ok(EXECUTION_FAILURE);
    };
    writeln!(stdout, "{line} {function} {caller_source}")?;
    Ok(EXECUTION_SUCCESS)
}
