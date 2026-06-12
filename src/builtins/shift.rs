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

    let amount = args
        .first()
        .and_then(|arg| arg.parse::<usize>().ok())
        .unwrap_or(1);
    Ok(ShiftAction::Shift(amount))
}
