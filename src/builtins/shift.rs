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
    execute_with_io(args, &mut stdout)
}

pub(crate) fn execute_with_io<W>(args: &[String], stdout: &mut W) -> io::Result<ShiftAction>
where
    W: Write,
{
    if args.first().map(String::as_str) == Some("--help") {
        crate::builtins::help::print_shift_help_with_io(stdout)?;
        return Ok(ShiftAction::Complete(2));
    }

    let amount = match args.first() {
        Some(arg) => match arg.parse::<isize>() {
            Ok(amount) if amount >= 0 => amount as usize,
            _ => return Ok(ShiftAction::Complete(1)),
        },
        None => 1,
    };
    Ok(ShiftAction::Shift(amount))
}
