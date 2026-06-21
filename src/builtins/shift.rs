//! shift module.
//!
//! GNU Bash source ownership:
// - builtins/shift.def

use std::io;

pub enum ShiftAction {
    Complete(i32),
    Shift(usize),
}

pub fn execute(args: &[String]) -> io::Result<ShiftAction> {
    if args.first().map(String::as_str) == Some("--help") {
        crate::builtins::help::print_shift_help();
        return Ok(ShiftAction::Complete(0));
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
