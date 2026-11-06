//! wait module.
//!
//! GNU Bash source ownership:
// - builtins/wait.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const EXECUTION_NOTFOUND: i32 = 127;

/// rubash#2898: how an all-digit wait operand classifies under GNU's
/// rule (builtins/wait.def:272-291): a digit-leading word is a pid
/// only when `valid_number` succeeds AND the value round-trips
/// through `pid_t` (a 32-bit int) unchanged — `pid_value ==
/// (pid_t)pid_value`. A value above INT_MAX (2147483648) or a digit
/// string too long for intmax is NOT a pid at all: wait reports it
/// with sh_badpid (`` `N': not a pid or valid job spec ``) and returns
/// EXECUTION_FAILURE at once. An in-range value is the pid — and the
/// pid is what wait_for_single_pid prints in its not-a-child report
/// (jobs.c:2753), so `wait 007` names `pid 7`, never the raw word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitPidClass {
    /// Not an all-digit operand (jobspec / other text forms).
    NotPid,
    /// All digits, in pid_t range: the parsed pid value.
    Pid(u32),
    /// All digits but outside pid_t range (or beyond intmax).
    BadPid,
}

pub(crate) fn classify_wait_pid_operand(operand: &str) -> WaitPidClass {
    if operand.is_empty() || !operand.bytes().all(|byte| byte.is_ascii_digit()) {
        return WaitPidClass::NotPid;
    }
    match operand.parse::<u64>() {
        Ok(value) if value <= i32::MAX as u64 => WaitPidClass::Pid(value as u32),
        _ => WaitPidClass::BadPid,
    }
}

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    error_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let mut index = 0;
    let mut wait_any = false;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        for (offset, option) in arg[1..].char_indices() {
            match option {
                'f' => {}
                'n' => wait_any = true,
                'p' => {
                    let value_start = 1 + offset + option.len_utf8();
                    if value_start < arg.len() {
                        break;
                    }
                    index += 1;
                    if args.get(index).is_none() {
                        // GNU wait.def internal_getopt "fnp:": a dangling
                        // `-p` is a getopt error -> usage + EX_USAGE
                        // (rubash#1156).
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}wait: -p: option requires an argument"
                        )?;
                        writeln!(stderr, "wait: usage: wait [-fn] [-p var] [id ...]")?;
                        return Ok(EX_USAGE);
                    }
                    break;
                }
                other => {
                    writeln!(stderr, "{diagnostic_prefix}wait: -{other}: invalid option")?;
                    writeln!(stderr, "wait: usage: wait [-fn] [-p var] [id ...]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    let operands = &args[index..];
    if operands.is_empty() {
        return Ok(if wait_any {
            EXECUTION_NOTFOUND
        } else {
            EXECUTION_SUCCESS
        });
    }

    let mut status = EXECUTION_SUCCESS;
    // GNU wait.def set_waitlist / jobs.c get_job_spec: an operand is a pid
    // only when it is a nonempty digit string; the empty string (like any
    // other non-numeric text) is "`': not a pid or valid job spec" with
    // EXECUTION_FAILURE, never the 127 "not a child" arm (rubash#1118 —
    // `chars().all(..)` on an empty operand is vacuously true and took the
    // digit arm).
    for operand in operands {
        if operand.starts_with('%') {
            writeln!(stderr, "{diagnostic_prefix}wait: {operand}: no such job")?;
            status = EXECUTION_NOTFOUND;
        } else if !operand.is_empty() && operand.chars().all(|ch| ch.is_ascii_digit()) {
            // rubash#2898: an all-digit operand outside pid_t range is
            // not a pid — wait.def:285-291 answers sh_badpid and
            // WAIT_RETURNs EXECUTION_FAILURE immediately, without
            // looking at any later operands.
            let pid = match classify_wait_pid_operand(operand) {
                WaitPidClass::BadPid => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}wait: `{operand}': not a pid or valid job spec"
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
                WaitPidClass::Pid(pid) => pid,
                WaitPidClass::NotPid => unreachable!("all-digit operand classified NotPid"),
            };
            if wait_any {
                // GNU builtins/wait.def set_waitlist: under `-n' operands
                // are validated against the JOB TABLE (get_job_by_pid /
                // get_job_spec), so a pid that is not a current job takes
                // sh_badpid (builtins/common.c:234) — never the
                // wait_for_single_pid "not a child" report (deep-audit
                // B6-F4).
                writeln!(
                    stderr,
                    "{diagnostic_prefix}wait: `{operand}': not a pid or valid job spec"
                )?;
            } else {
                // GNU jobs.c:2753-2754 wait_for_single_pid JWAIT_PERROR:
                // internal_error (error_prolog family) honors gnu_errfmt.
                // The report names the parsed pid value, so leading
                // zeros collapse (`wait 007` -> `pid 7`, rubash#2898).
                writeln!(
                    stderr,
                    "{error_prefix}wait: pid {pid} is not a child of this shell"
                )?;
            }
            status = EXECUTION_NOTFOUND;
        } else {
            writeln!(
                stderr,
                "{diagnostic_prefix}wait: `{operand}': not a pid or valid job spec"
            )?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}
