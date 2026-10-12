//! pushd module.
//!
//! GNU Bash source ownership:
// - builtins/pushd.def

mod parse;
mod stack;

pub(crate) use stack::{load_stack, save_stack, set_stack_value, stack_value, stack_words};

use parse::{parse_popd_operand, parse_pushd_operand, PopdOperand, PushdOperand};
use stack::{
    is_stack_index, logical_dir_exists, normalize_stack_dir, resolved_index, set_pwd_from_stack,
    stack_index, strip_double_dash,
};
use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
/// GNU error.h EX_USAGE: a builtin's invalid number / usage error exits 2
/// (builtins/common.c get_numeric_arg -> builtin_error("%s: invalid number")
/// + return EX_USAGE). dirs/popd/pushd share that path for every ±N operand.
const EX_USAGE: i32 = 2;
pub(crate) const DIR_STACK: &str = "__RUBASH_DIR_STACK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackBuiltin {
    Pushd,
    Popd,
    Dirs,
}

pub fn execute(
    builtin: StackBuiltin,
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    diagnostic_prefix: &str,
) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        builtin,
        args.iter().map(String::as_str),
        env_vars,
        diagnostic_prefix,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    builtin: StackBuiltin,
    args: I,
    env_vars: &mut HashMap<String, String>,
    diagnostic_prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut stack = load_stack(env_vars);

    match builtin {
        StackBuiltin::Dirs => {
            let args = strip_double_dash(&args);
            if args.first().copied() == Some("-c") {
                save_stack(env_vars, &[]);
                return Ok(EXECUTION_SUCCESS);
            }
            // GNU dirs.def walks the display flags off the front and they
            // combine (`dirs -l -v` prints the numbered long form); the
            // first remaining word is the optional ±N operand, anything
            // else after the flags is rejected below. A ±-shaped word goes
            // through get_numeric_arg (builtins/common.c): a non-number is
            // `%s: invalid number` + usage, EX_USAGE — and dirs.def's
            // caller turns EX_USAGE into exit status 2. `dirs -l +q` must
            // NOT silently consume the operand (issue #509: it printed the
            // stack and exited 0 where GNU exits 2).
            let mut flag_end = 0;
            let mut printable = false;
            let mut numbered = false;
            while let Some(arg) = args.get(flag_end).copied() {
                match arg {
                    // -l is consumed for validation parity; rubash's stack
                    // always stores full paths, so the long form has no
                    // separate rendering (GNU's ~-collapsing is the only
                    // difference and rubash never collapses).
                    "-l" | "-p" => printable = true,
                    "-v" => numbered = true,
                    _ => break,
                }
                flag_end += 1;
            }
            let rest = &args[flag_end..];
            if let Some(arg) = rest.first().copied() {
                if arg.starts_with('+') || arg.starts_with('-') {
                    if !is_stack_index(arg) {
                        writeln!(stderr, "{diagnostic_prefix}dirs: {arg}: invalid number")?;
                        writeln!(stderr, "dirs: usage: dirs [-clpv] [+N] [-N]")?;
                        return Ok(EX_USAGE);
                    }
                    let Some(index) = stack_index(arg, stack.len()) else {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}dirs: {}: directory stack index out of range",
                            arg.trim_start_matches(['+', '-'])
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    };
                    if numbered {
                        writeln!(stdout, "{index:2}  {}", stack[index])?;
                    } else {
                        writeln!(stdout, "{}", stack[index])?;
                    }
                    return Ok(EXECUTION_SUCCESS);
                }
                writeln!(stderr, "{diagnostic_prefix}dirs: {arg}: invalid option")?;
                writeln!(stderr, "dirs: usage: dirs [-clpv] [+N] [-N]")?;
                return Ok(EX_USAGE);
            }
            if numbered {
                for (index, dir) in stack.iter().enumerate() {
                    writeln!(stdout, "{index:2}  {dir}")?;
                }
            } else if printable {
                for dir in &stack {
                    writeln!(stdout, "{dir}")?;
                }
            } else {
                // load_stack always exposes the cwd as element 0, so the
                // plain (and -l) listing joins the stack; the old dead
                // empty-stack arm only ever echoed $PWD.
                writeln!(stdout, "{}", stack.join(" "))?;
            }
            Ok(EXECUTION_SUCCESS)
        }
        StackBuiltin::Pushd => {
            let operand = parse_pushd_operand(&args, diagnostic_prefix, stderr)?;
            let Some(operand) = operand else {
                // The parse printed its usage diagnostic: GNU pushd.def
                // returns get_numeric_arg's EX_USAGE (2), not a plain
                // builtin failure (issue #509).
                return Ok(EX_USAGE);
            };

            match operand {
                PushdOperand::Swap => {
                    if stack.len() < 2 {
                        writeln!(stderr, "{diagnostic_prefix}pushd: no other directory")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.swap(0, 1);
                    set_pwd_from_stack(env_vars, &stack, true);
                }
                PushdOperand::Index {
                    index,
                    from_right,
                    no_cd,
                } => {
                    let index =
                        resolved_index(index, from_right, stack.len()).unwrap_or(usize::MAX);
                    if index >= stack.len() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}pushd: {}: directory stack index out of range",
                            args.last().copied().unwrap_or_default()
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    if no_cd {
                        save_stack(env_vars, &stack);
                        return Ok(EXECUTION_SUCCESS);
                    }
                    stack.rotate_left(index);
                    set_pwd_from_stack(env_vars, &stack, true);
                }
                PushdOperand::Dir { dir, no_cd } => {
                    let resolved_dir = normalize_stack_dir(&dir, env_vars);
                    if !logical_dir_exists(&resolved_dir, env_vars) {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}pushd: {dir}: No such file or directory"
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    let old_pwd = stack
                        .first()
                        .cloned()
                        .or_else(|| env_vars.get("PWD").cloned())
                        .unwrap_or_else(|| "/".to_string());
                    if stack.is_empty() {
                        stack.push(old_pwd.clone());
                    }
                    if no_cd && !stack.is_empty() {
                        stack.insert(1, resolved_dir);
                    } else {
                        stack.insert(0, resolved_dir.clone());
                        env_vars.insert("OLDPWD".to_string(), old_pwd);
                        env_vars.insert("PWD".to_string(), resolved_dir);
                    }
                }
            }

            save_stack(env_vars, &stack);
            writeln!(stdout, "{}", stack.join(" "))?;
            Ok(EXECUTION_SUCCESS)
        }
        StackBuiltin::Popd => {
            let operand = parse_popd_operand(&args, diagnostic_prefix, stderr)?;
            let Some(operand) = operand else {
                // Same EX_USAGE contract as pushd above (issue #509).
                return Ok(EX_USAGE);
            };

            match operand {
                PopdOperand::Top { no_cd } => {
                    if stack.len() <= 1 {
                        writeln!(stderr, "{diagnostic_prefix}popd: directory stack empty")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.remove(if no_cd { 1 } else { 0 });
                    if no_cd {
                        save_stack(env_vars, &stack);
                        writeln!(stdout, "{}", stack.join(" "))?;
                        return Ok(EXECUTION_SUCCESS);
                    }
                }
                PopdOperand::Index {
                    index,
                    from_right,
                    no_cd,
                } => {
                    let index =
                        resolved_index(index, from_right, stack.len()).unwrap_or(usize::MAX);
                    if index >= stack.len() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}popd: {}: directory stack index out of range",
                            args.last().copied().unwrap_or_default()
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.remove(index);
                    if no_cd {
                        save_stack(env_vars, &stack);
                        writeln!(stdout, "{}", stack.join(" "))?;
                        return Ok(EXECUTION_SUCCESS);
                    }
                }
            }

            let pwd = stack.first().cloned().unwrap_or_else(|| "/".to_string());
            env_vars.insert("PWD".to_string(), pwd);
            save_stack(env_vars, &stack);
            writeln!(stdout, "{}", stack.join(" "))?;
            Ok(EXECUTION_SUCCESS)
        }
    }
}
