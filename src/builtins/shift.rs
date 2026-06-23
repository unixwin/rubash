//! shift module.
//!
//! GNU Bash source ownership:
// - builtins/shift.def

use std::io::{self, Write};

pub enum ShiftAction {
    Complete(i32),
    Shift(usize),
}

pub fn execute(args: &[String]) -> io::Result<ShiftAction> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<ShiftAction>
where
    W: Write,
    E: Write,
{
    if args.first().map(String::as_str) == Some("--help") {
        crate::builtins::help::print_shift_help_with_io(stdout)?;
        return Ok(ShiftAction::Complete(2));
    }

    if args.len() > 1 {
        writeln!(stderr, "{}shift: too many arguments", diagnostic_prefix())?;
        return Ok(ShiftAction::Complete(1));
    }

    let amount = match args.first() {
        Some(arg) => match arg.parse::<isize>() {
            Ok(amount) if amount >= 0 => amount as usize,
            Ok(_) => {
                writeln!(
                    stderr,
                    "{}shift: {arg}: shift count out of range",
                    diagnostic_prefix()
                )?;
                return Ok(ShiftAction::Complete(1));
            }
            Err(_) => {
                writeln!(
                    stderr,
                    "{}shift: {arg}: numeric argument required",
                    diagnostic_prefix()
                )?;
                return Ok(ShiftAction::Complete(1));
            }
        },
        None => 1,
    };
    Ok(ShiftAction::Shift(amount))
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
