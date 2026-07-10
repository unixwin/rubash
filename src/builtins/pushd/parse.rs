use super::stack::is_stack_index;
use std::io::{self, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PushdOperand {
    Swap,
    Index {
        index: usize,
        from_right: bool,
        no_cd: bool,
    },
    Dir {
        dir: String,
        no_cd: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PopdOperand {
    Top {
        no_cd: bool,
    },
    Index {
        index: usize,
        from_right: bool,
        no_cd: bool,
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
    let (no_cd, args) = parse_stack_options(args);
    if args.is_empty() {
        return Ok(Some(PushdOperand::Swap));
    }

    let arg = args[0];
    if arg.starts_with('-') && !is_stack_index(arg) {
        writeln!(stderr, "{diagnostic_prefix}pushd: {arg}: invalid number")?;
        writeln!(stderr, "pushd: usage: pushd [-n] [+N | -N | dir]")?;
        return Ok(None);
    }

    if is_stack_index(arg) {
        return Ok(Some(PushdOperand::Index {
            index: arg[1..].parse::<usize>().unwrap_or(usize::MAX),
            from_right: arg.starts_with('-'),
            no_cd,
        }));
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
    if !is_stack_index(arg) {
        if arg.starts_with('-') {
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
