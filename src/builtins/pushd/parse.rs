use super::stack::is_stack_index;
use std::io::{self, Write};

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
        index: usize,
        from_right: bool,
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
    /// line; pushd maps the non-zero verdict to EXECUTION_FAILURE (:310-315).
    TooManyArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PopdOperand {
    Top {
        no_cd: bool,
    },
    Index {
        index: usize,
        from_right: bool,
        no_cd: bool,
        /// The raw ±N word: popd's walk records it as which_word
        /// (pushd.def:374-377) and hands it to pushd_error at :382/:414,
        /// so `popd +5 -n` reports "+5", not the option that follows it.
        word: String,
    },
}

pub(super) fn parse_pushd_operand<W>(
    args: &[&str],
    diagnostic_prefix: &str,
    stderr: &mut W,
) -> io::Result<Option<PushdOperand>>
where
    W: Write,
{
    // GNU pushd.def:183-190: a LEADING `--` sets skipopt and skips the whole
    // option walk — the remainder goes verbatim to cd_builtin, so `-- -n`
    // is a cd-to-`-n` failure (rc 1), `-- +1` never rotates, and a lone
    // `--` still swaps. More than one word is cd's argc failure.
    if args.first().copied() == Some("--") {
        match &args[1..] {
            [] => return Ok(Some(PushdOperand::Swap)),
            [dir] => {
                return Ok(Some(PushdOperand::Dir {
                    dir: dir.to_string(),
                    no_cd: false,
                }));
            }
            _ => return Ok(Some(PushdOperand::TooManyArgs)),
        }
    }
    if args.is_empty() {
        // Bare `pushd` (no words at all; a leading `--` returned above) is
        // the no-argument swap. Consumed-options no-ops (`pushd -n`,
        // `pushd -n --`) are judged after the walk via `rest.is_empty()`.
        return Ok(Some(PushdOperand::Swap));
    }

    // GNU pushd.def:214-260 option walk: `-n` accumulates NOCD, `--` ends
    // the walk with the remainder verbatim, and a lone `-` breaks out as a
    // Dir operand (cd $OLDPWD) instead of an option or rotation.
    let mut no_cd = false;
    let mut index = 0;
    let mut saw_dashdash = false;
    while let Some(arg) = args.get(index).copied() {
        match arg {
            "-n" => {
                no_cd = true;
                index += 1;
            }
            "--" => {
                index += 1;
                saw_dashdash = true;
                break;
            }
            "-" => break,
            _ => break,
        }
    }
    let rest = &args[index..];
    if rest.is_empty() {
        // pushd.def:288: options with nothing left are a silent success.
        return Ok(Some(PushdOperand::NoOp));
    }
    let arg = rest[0];
    // A ±N word rotates only straight out of the option walk: past `--`
    // it is a verbatim directory word (`pushd -n -- +1` pushes a literal
    // `+1`, never rotates).
    if !saw_dashdash && is_stack_index(arg) {
        return Ok(Some(PushdOperand::Index {
            index: arg[1..].parse::<usize>().unwrap_or(usize::MAX),
            from_right: arg.starts_with('-'),
            no_cd,
            word: arg.to_string(),
        }));
    }
    // The argc check lives in cd_builtin (CD_COMPLAINS) ahead of the null
    // and `-` judgments, so `pushd "" extra` and `pushd - -n` are `too many
    // arguments`, never null-directory or OLDPWD diagnostics. NOCD pushes
    // word one and ignores the rest; ROTATE ignores leftovers.
    if !no_cd && rest.len() > 1 {
        return Ok(Some(PushdOperand::TooManyArgs));
    }
    // A lone `-` is the verbatim Dir operand, not a ±-shaped word.
    if arg == "-" {
        return Ok(Some(PushdOperand::Dir {
            dir: arg.to_string(),
            no_cd,
        }));
    }
    // GNU pushd.def validates the operand through get_numeric_arg when it
    // is ±-shaped: EITHER sign failing to parse is
    // `%s: invalid number` + usage (builtins/common.c), not a directory
    // operand — `pushd +q` must not fall through to the Dir arm (issue
    // #509).
    // Past `--` the walk already ended: the word is verbatim for cd_builtin
    // or the NOCD push (`pushd -n -- -n` stores a literal `-n`), never a
    // number-or-usage judgment.
    if !saw_dashdash && (arg.starts_with('-') || arg.starts_with('+')) && !is_stack_index(arg) {
        writeln!(stderr, "{diagnostic_prefix}pushd: {arg}: invalid number")?;
        writeln!(stderr, "pushd: usage: pushd [-n] [+N | -N | dir]")?;
        return Ok(None);
    }

    Ok(Some(PushdOperand::Dir {
        dir: arg.to_string(),
        no_cd,
    }))
}

pub(super) fn parse_popd_operand<W>(
    args: &[&str],
    diagnostic_prefix: &str,
    stderr: &mut W,
) -> io::Result<Option<PopdOperand>>
where
    W: Write,
{
    if args.first().copied() == Some("--") {
        return Ok(Some(PopdOperand::Top { no_cd: false }));
    }

    let (no_cd, args) = parse_stack_options(args);
    if args.is_empty() {
        return Ok(Some(PopdOperand::Top { no_cd }));
    }

    let arg = args[0];
    // GNU builtins/pushd.def:318 popd_builtin()'s option walk treats an
    // operand word that expanded to the empty string as the END of the
    // argument list (`else if (*list->word->word) { invalid argument } else
    // break;`, pushd.def:356-363) — `HOME=; popd ~` lands here with "" and
    // performs the default top-of-stack pop instead of erroring (#754
    // family: an empty tilde expansion is a real word, and each builtin
    // judges it by its own operand rules).
    if arg.is_empty() {
        return Ok(Some(PopdOperand::Top { no_cd }));
    }
    if !is_stack_index(arg) {
        // GNU popd.def: a ±-shaped non-number is get_numeric_arg's
        // `%s: invalid number`; anything else in the operand position is
        // `invalid argument` (both EX_USAGE — the caller returns 2).
        if arg.starts_with('-') || arg.starts_with('+') {
            writeln!(stderr, "{diagnostic_prefix}popd: {arg}: invalid number")?;
        } else {
            writeln!(stderr, "{diagnostic_prefix}popd: {arg}: invalid argument")?;
        }
        writeln!(stderr, "popd: usage: popd [-n] [+N | -N]")?;
        return Ok(None);
    }

    let index = arg[1..].parse::<usize>().unwrap_or(usize::MAX);
    Ok(Some(PopdOperand::Index {
        index,
        from_right: arg.starts_with('-'),
        no_cd,
        word: arg.to_string(),
    }))
}

fn parse_stack_options<'a>(args: &'a [&str]) -> (bool, &'a [&'a str]) {
    let mut no_cd = false;
    let mut index = 0;
    while let Some(arg) = args.get(index).copied() {
        match arg {
            "--" => return (no_cd, &args[index + 1..]),
            "-n" => {
                no_cd = true;
                index += 1;
            }
            _ => break,
        }
    }
    (no_cd, &args[index..])
}
