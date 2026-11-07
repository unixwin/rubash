//! Faithful ports of the `pushd`/`popd` option walks in
//! builtins/pushd.def (pushd.def:199-260, :340-368).
//!
//! These are NOT `internal_getopt` loops: `dirs`/`pushd`/`popd` walk
//! exact words (`ISOPTION`) and CONTINUE after `+N` words — the last
//! `+N` wins, a `-n` after a `+N` still accumulates, and an invalid
//! word after a `+N` still errors. A plain `BuiltinOpt` spec would
//! wrongly accept clusters like `-nl`, which GNU rejects with
//! `sh_invalidnum` (the `+`/`-` arm validates the digits inline).
//! Diagnostics speak through the caller's `Diag` (D3); the outcome
//! tells the caller which status the already-printed diagnostic
//! carries, so the parse itself never returns a status.

use crate::executor::builtin_diag::Diag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PushdOperand {
    Swap,
    /// Option words with nothing left (`pushd -n`, `pushd -n --`) are a
    /// silent no-op: GNU's option walk consumes the flags and leaves
    /// `list == 0`, reaching pushd.def:288-291 `return
    /// EXECUTION_SUCCESS` — never the bare-`pushd` swap arm (:195-213,
    /// which errors "no other directory" when the stack is short).
    /// rubash#859, #896.
    NoOp,
    Index {
        /// The POST-conversion rotation count: `+N` is N, `-N` is
        /// `offset - N` (pushd.def:235-244). The range check already
        /// fired inline during the walk, so this is always in
        /// `0..=offset`; the display stack rotates left by it.
        num: usize,
        no_cd: bool,
        /// The raw ±N word. GNU hands the word itself to pushd_error
        /// (pushd.def:242 walks it inline), so the range diagnostic echoes
        /// the offending operand verbatim — never a trailing option word.
        word: String,
    },
    Dir {
        dir: String,
        no_cd: bool,
    },
    /// GNU pushd.def:298 routes the Dir operand through cd_builtin, whose
    /// CD_COMPLAINS argc check (cd.def:326-330, active via config-top.h)
    /// rejects a multi-word list with `too many arguments` and no usage
    /// line; pushd maps the non-zero verdict to EXECUTION_FAILURE
    /// (:310-315).
    TooManyArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PopdOperand {
    Top {
        no_cd: bool,
    },
    Index {
        /// The display-stack slot to free (no chdir ever runs for a
        /// non-top pop, pushd.def:404-421). The `""` empty-word quirk
        /// lands here too: GNU's loop test assigns `direction =
        /// word[0]` — `'\0'` for `""` — and the post-loop else-branch
        /// removes `list[0]`, the OLDEST entry, so the display index
        /// is `len - 1`.
        display_index: usize,
        no_cd: bool,
    },
}

/// The outcome of a stack option walk. A usage diagnostic
/// (`invalid number` / `invalid option` / `invalid argument` + the
/// usage line) was already printed for `Usage`; a failure diagnostic
/// (the out-of-range `pushd_error`) was already printed for
/// `Failed`. The caller only supplies the status.
pub(super) enum ParseOutcome<T> {
    Ok(T),
    Usage,
    Failed,
}

/// GNU pushd.def:199-260, verbatim in structure. `offset` is
/// `directory_list_offset` — the display stack length minus one
/// (rubash models the cwd as display slot 0).
pub(super) fn parse_pushd_operand<'a>(
    args: &[&'a str],
    offset: usize,
    diag: &mut Diag<'a>,
) -> ParseOutcome<PushdOperand> {
    // GNU pushd.def:183-190: a LEADING `--` sets skipopt and skips the whole
    // option walk — the remainder goes verbatim to cd_builtin, so `-- -n`
    // is a cd-to-`-n` failure (rc 1), `-- +1` never rotates, and a lone
    // `--` still swaps. More than one word is cd's argc failure.
    if args.first().copied() == Some("--") {
        match &args[1..] {
            [] => return ParseOutcome::Ok(PushdOperand::Swap),
            [dir] => {
                return ParseOutcome::Ok(PushdOperand::Dir {
                    dir: dir.to_string(),
                    no_cd: false,
                });
            }
            _ => return ParseOutcome::Ok(PushdOperand::TooManyArgs),
        }
    }
    if args.is_empty() {
        // Bare `pushd` (no words at all; a leading `--` returned above) is
        // the no-argument swap. Consumed-options no-ops (`pushd -n`,
        // `pushd -n --`) are judged after the walk via `rest.is_empty()`.
        return ParseOutcome::Ok(PushdOperand::Swap);
    }

    // GNU pushd.def:214-260 option walk: `-n` accumulates NOCD, `--` ends
    // the walk with the remainder verbatim, a lone `-` breaks out as a
    // Dir operand (cd $OLDPWD), and a `+N`/`-N` word converts, range-
    // checks INLINE (naming the word), sets ROTATE, and the loop
    // CONTINUES — the last `+N` wins and a `-n` after it still applies.
    let mut no_cd = false;
    let mut rotate: Option<(usize, String)> = None;
    let mut index = 0;
    while let Some(arg) = args.get(index).copied() {
        if arg == "-n" {
            no_cd = true;
            index += 1;
        } else if arg == "--" {
            index += 1;
            break;
        } else if arg == "-" {
            break;
        } else if arg.starts_with('+') || arg.starts_with('-') {
            // pushd.def:232-244: `direction = word[0]`,
            // `valid_number (word + 1, &num)` else `sh_invalidnum` +
            // usage + EX_USAGE; `-N` converts to `offset - N`;
            // `num > offset || num < 0` is `pushd_error (word)` +
            // EXECUTION_FAILURE, fired here, before any later word is
            // examined — `pushd +9 -n` reports "+9" even though the
            // walk never reaches the -n.
            let Ok(digits) = arg[1..].parse::<i64>() else {
                diag.free(format_args!("{arg}: invalid number"));
                diag.usage();
                return ParseOutcome::Usage;
            };
            let mut num = digits;
            if arg.starts_with('-') {
                num = offset as i64 - num;
            }
            if num > offset as i64 || num < 0 {
                super::pushd_error(offset, arg, diag);
                return ParseOutcome::Failed;
            }
            rotate = Some((num as usize, arg.to_string()));
            index += 1;
        } else if arg.starts_with('-') {
            // Dead in GNU: every dash word reaches the `+`/`-` arm
            // first (its test assigns `direction = word[0]`, so `-`
            // always enters it). Kept so the loop keeps GNU's shape.
            diag.free(format_args!("{arg}: invalid option"));
            diag.usage();
            return ParseOutcome::Usage;
        } else {
            break;
        }
    }
    if let Some((num, word)) = rotate {
        return ParseOutcome::Ok(PushdOperand::Index { num, no_cd, word });
    }
    let rest = &args[index..];
    if rest.is_empty() {
        // pushd.def:288: options with nothing left are a silent success.
        return ParseOutcome::Ok(PushdOperand::NoOp);
    }
    let arg = rest[0];
    // The argc check lives in cd_builtin (CD_COMPLAINS) ahead of the null
    // and `-` judgments, so `pushd "" extra` and `pushd - -n` are `too many
    // arguments`, never null-directory or OLDPWD diagnostics. NOCD pushes
    // word one and ignores the rest; ROTATE ignores leftovers.
    if !no_cd && rest.len() > 1 {
        return ParseOutcome::Ok(PushdOperand::TooManyArgs);
    }
    // A lone `-` is the verbatim Dir operand, not a ±-shaped word.
    // Past `--` the walk already ended: the word is verbatim for
    // cd_builtin or the NOCD push (`pushd -n -- -n` stores a literal
    // `-n`), never a number-or-usage judgment.
    ParseOutcome::Ok(PushdOperand::Dir {
        dir: arg.to_string(),
        no_cd,
    })
}

/// GNU pushd.def:340-368 (popd_builtin's option walk) plus the
/// post-loop range check and branch selection (:374-421). `offset` is
/// `directory_list_offset` — the display stack length minus one.
pub(super) fn parse_popd_operand<'a>(
    args: &[&'a str],
    offset: usize,
    diag: &mut Diag<'a>,
) -> ParseOutcome<PopdOperand> {
    // No skipopt pre-check in popd: the loop's ISOPTION '-' arm skips
    // the word and breaks, and the `#if 0` block is DISABLED — words
    // after `--` are IGNORED entirely, so `popd -n -- +1` pops the
    // top and `popd --` pops the top too.
    let mut no_cd = false;
    let mut direction = '+';
    let mut which: i64 = 0;
    let mut which_word: Option<&str> = None;
    let mut index = 0;
    while let Some(arg) = args.get(index).copied() {
        if arg == "-n" {
            no_cd = true;
            index += 1;
        } else if arg == "--" {
            index += 1;
            break;
        } else if arg.starts_with('+') || arg.starts_with('-') {
            // pushd.def:344-352: `direction = word[0]` (assigned in
            // the test itself), `valid_number (word + 1, &which)` else
            // `sh_invalidnum` + usage + EX_USAGE; `which_word` records
            // the word for the post-loop range check. The loop
            // CONTINUES — the last ±N wins.
            direction = arg.chars().next().unwrap_or('+');
            let Ok(digits) = arg[1..].parse::<i64>() else {
                diag.free(format_args!("{arg}: invalid number"));
                diag.usage();
                return ParseOutcome::Usage;
            };
            which = digits;
            which_word = Some(arg);
            index += 1;
        } else if arg.starts_with('-') {
            // Dead in GNU (see pushd above).
            diag.free(format_args!("{arg}: invalid option"));
            diag.usage();
            return ParseOutcome::Usage;
        } else if !arg.is_empty() {
            // pushd.def:356-363: a non-empty non-dash word in the
            // operand position is `%s: invalid argument` + usage.
            diag.free(format_args!("{arg}: invalid argument"));
            diag.usage();
            return ParseOutcome::Usage;
        } else {
            // Empty word: the loop test assigns `direction =
            // word[0]` — `'\0'` — and the `else break` fires. The
            // post-loop else-branch then removes `list[0]`, the
            // OLDEST entry, without chdir (pushd.def:404-421).
            direction = '\0';
            break;
        }
    }

    // pushd.def:374-377: `which > offset || which < -offset ||
    // (offset == 0 && which == 0)` hands `which_word` (or "" when no
    // ±N word was seen) to pushd_error. `which` is the raw digits,
    // always >= 0, so the `which < -offset` arm is dead in practice.
    if which > offset as i64 || (offset == 0 && which == 0) {
        super::pushd_error(offset, which_word.unwrap_or(""), diag);
        return ParseOutcome::Failed;
    }

    // pushd.def:383-391: `(direction == '+' && which == 0) ||
    // (direction == '-' && which == offset)` is the top-of-stack
    // branch (cd to the new top, then free it); anything else frees
    // `list[i]` with `i = direction == '+' ? offset - which : which`
    // and shifts the remainder left, no chdir. `list[i]` is display
    // slot `len - 1 - i` (the list holds the display tail oldest-
    // first, without the cwd).
    if (direction == '+' && which == 0) || (direction == '-' && which == offset as i64) {
        return ParseOutcome::Ok(PopdOperand::Top { no_cd });
    }
    let list_index = if direction == '+' {
        offset as i64 - which
    } else {
        which
    };
    ParseOutcome::Ok(PopdOperand::Index {
        display_index: offset - list_index as usize,
        no_cd,
    })
}
