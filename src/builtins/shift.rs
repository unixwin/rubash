//! shift module.
//!
//! GNU Bash source ownership:
//! - builtins/shift.def

use crate::executor::builtin_diag::{Diag, DiagKind, NumericFatal, Prolog};
use crate::executor::builtin_table::BuiltinRow;
use std::io::{self, Write};

pub enum ShiftAction {
    Complete(i32),
    /// A validated count plus the word it came from (`None` for
    /// the implicit count). The executor's gated out-of-range
    /// diagnostic renders GNU's `sh_erange` argument — the raw
    /// word, or no word at all for the implicit count
    /// (builtins/shift.def:74,86 pass
    /// `list ? list->word->word : NULL`; rubash#2966) — so the
    /// word has to travel with the amount.
    Shift(usize, Option<String>),
    /// Surplus operands: GNU builtins/common.c no_args() — the
    /// diagnostic, EX_BADUSAGE, then jump_to_top_level (DISCARD).
    /// Kept distinct from Complete(2) because the other EX_USAGE
    /// paths (non-numeric operand, --help) return a plain status
    /// with no discard; only the no_args class abandons the rest
    /// of the current list (rubash#1650).
    TooManyArguments,
}

/// GNU error.h: EX_USAGE (2) marks a usage error, EXECUTION_FAILURE (1) a
/// plain runtime failure. shift.def routes a non-numeric operand through
/// get_numeric_arg (builtins/common.c:501-505 `sh_neednumarg` + `return 0`
/// → `return (EX_USAGE)`) and a surplus operand through `no_args`
/// (builtins/common.c `set_exit_status (EX_BADUSAGE)`), so both exit 2;
/// only the shift-count-out-of-range erange path returns
/// EXECUTION_FAILURE.
pub(crate) const EX_USAGE: i32 = 2;
const EXECUTION_FAILURE: i32 = 1;

pub fn execute(args: &[String]) -> io::Result<ShiftAction> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let row = crate::executor::builtin_table::builtin_table_row("shift")
        .expect("shift row in BUILTIN_TABLE");
    let mut diag = Diag::new(row, &mut stderr);
    let action = execute_with_io(row, args, &mut diag, &mut stdout)?;
    diag.flush_with(|| Prolog {
        interactive: false,
        shell_basename: "rubash".to_string(),
        error_name: "rubash".to_string(),
        script_file: std::env::var("__RUBASH_SCRIPT_NAME").ok(),
        line: std::env::var("__RUBASH_CURRENT_LINE").ok(),
    });
    Ok(action)
}

/// Stage-2 ④ Phase 3: the count goes through `Diag::numeric_arg`
/// (builtins/common.c:488-520 `get_numeric_arg (list, 0, &times)`)
/// — the head word's validity is judged BEFORE the no_args
/// surplus tail (rubash#2956). `numeric_arg` folds the surplus
/// report into the same call, so the head is validated on its own
/// first (the optional `--` plus the first word) and the surplus
/// is reported separately: the two failure classes land in
/// different executor channels (plain EX_USAGE return vs the
/// DISCARD vehicle), exactly as shift.def splits them.
pub(crate) fn execute_with_io<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut W,
) -> io::Result<ShiftAction>
where
    W: Write,
{
    if args.first().map(String::as_str) == Some("--help") {
        // CHECK_HELPOPT (builtins/common.h:29): prints the long
        // help and the builtin RETURNS EX_USAGE — no shift.
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(ShiftAction::Complete(EX_USAGE));
    }

    // get_numeric_arg skips ONE leading `--` (ISOPTION,
    // common.h:26) before the count word; the head slice handed
    // to it is that word (with its `--`).
    let start = usize::from(args.first().map(String::as_str) == Some("--"));
    let head_end = (start + 1).min(args.len());
    let amount = match diag.numeric_arg(&args[..head_end], NumericFatal::Soft) {
        Some(amount) => amount,
        // sh_neednumarg was already reported by numeric_arg.
        // shift.def: `get_numeric_arg (list, 0, &times) == 0`
        // (builtins/common.c:501-505) returns EX_USAGE, not
        // EXECUTION_FAILURE (issue #759 item 3: GNU 5.3.0
        // `( set -- 1 2; shift x )` exits 2).
        None => return Ok(ShiftAction::Complete(EX_USAGE)),
    };
    if args.len() > start + 1 {
        // no_args (builtins/common.c:511): the surplus report; the
        // executor arms the DISCARD for this action (rubash#1650).
        diag.report(DiagKind::TooManyArgs);
        return Ok(ShiftAction::TooManyArguments);
    }
    if amount < 0 {
        // builtins/shift.def:77-88: sh_erange (word, "shift count")
        // and EXECUTION_FAILURE. The word is the one after the
        // `--` skip. (The > $# half of the range check needs the
        // positional count, so it stays with the executor's apply
        // step, as before.)
        let word = args.get(start).map(String::as_str).unwrap_or_default();
        diag.error(DiagKind::Erange(
            Some(word.to_string()),
            "shift count".to_string(),
        ));
        return Ok(ShiftAction::Complete(EXECUTION_FAILURE));
    }
    // The count word for the executor's gated erange: the word
    // after the optional `--`, or None when the count is
    // implicit (no word, or a lone `--`).
    let word = if head_end > start {
        args.get(start).cloned()
    } else {
        None
    };
    Ok(ShiftAction::Shift(amount as usize, word))
}
