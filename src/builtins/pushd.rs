//! pushd module.
//!
//! GNU Bash source ownership:
// - builtins/pushd.def

mod parse;
mod stack;

pub(crate) use stack::{load_stack, save_stack, set_stack_value, stack_value, stack_words};

use crate::executor::markers::DATA_DOLLAR;
use crate::executor::types::READONLY_VARS;
use parse::{parse_popd_operand, parse_pushd_operand, PopdOperand, PushdOperand};
use stack::{
    is_stack_index, logical_dir_exists, normalize_stack_dir, resolved_index, stack_index,
    strip_double_dash,
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
                    // rubash#759: -l is the LONG form, not the one-per-line
                    // form. GNU dirs.def only widens the rendering (it stops
                    // collapsing $HOME to ~); the separator stays a single
                    // space, and -p alone selects one entry per line. rubash's
                    // stack always stores full paths, so -l has no separate
                    // rendering at all and must not set `printable`:
                    // `dirs -l` printed three lines where GNU prints one.
                    "-l" => {}
                    "-p" => printable = true,
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
                    // pushd.def:203-215: the no-argument swap lands in
                    // pushd_directory_list BEFORE change_to_temp runs, so a
                    // bindpwd readonly failure keeps the swapped list — but
                    // change_to_temp (pushd.def:577-586) displays and succeeds
                    // only behind cd_to_string's EXECUTION_SUCCESS, so the
                    // failed swap prints nothing and returns cd_builtin's
                    // verdict with the cwd moved (the display's cwd slot
                    // follows it).
                    let old_pwd = env_vars
                        .get("PWD")
                        .cloned()
                        .unwrap_or_else(|| "/".to_string());
                    stack.swap(0, 1);
                    let new_pwd = stack[0].clone();
                    if bindpwd_write(env_vars, &old_pwd, &new_pwd, diagnostic_prefix, stderr)? {
                        save_stack(env_vars, &stack);
                        return Ok(EXECUTION_FAILURE);
                    }
                }
                PushdOperand::NoOp => {
                    // GNU pushd.def:288-291 `if (list == 0) return
                    // (EXECUTION_SUCCESS)`: consumed options with no operand
                    // left (`pushd -n`, `pushd -n --`) are a silent success —
                    // the stack is untouched and nothing prints.
                    // rubash#859, #896.
                    return Ok(EXECUTION_SUCCESS);
                }
                PushdOperand::TooManyArgs => {
                    // GNU cd.def:326-330 CD_COMPLAINS via pushd.def:298, whose
                    // non-zero verdict pushd maps to EXECUTION_FAILURE
                    // (:310-315): the message only, no usage line, rc 1.
                    writeln!(stderr, "{diagnostic_prefix}pushd: too many arguments")?;
                    return Ok(EXECUTION_FAILURE);
                }
                PushdOperand::Index {
                    index,
                    from_right,
                    no_cd,
                    word,
                } => {
                    let index =
                        resolved_index(index, from_right, stack.len()).unwrap_or(usize::MAX);
                    if index >= stack.len() {
                        // pushd.def:235-244: pushd's option walk range-checks
                        // the operand inline, handing the word itself to
                        // pushd_error — `pushd +9 -n` reports "+9" even though
                        // the walk never reaches the -n.
                        stack_range_error("pushd", stack.len(), &word, diagnostic_prefix, stderr)?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    if no_cd {
                        save_stack(env_vars, &stack);
                        return Ok(EXECUTION_SUCCESS);
                    }
                    // pushd.def:256-286: the rotation lands in the list
                    // before change_to_temp, so a bindpwd readonly failure
                    // keeps the rotated list while the display and the
                    // success status stay behind change_to_temp's
                    // EXECUTION_SUCCESS gate (pushd.def:577-586).
                    let old_pwd = env_vars
                        .get("PWD")
                        .cloned()
                        .unwrap_or_else(|| "/".to_string());
                    stack.rotate_left(index);
                    let new_pwd = stack[0].clone();
                    if bindpwd_write(env_vars, &old_pwd, &new_pwd, diagnostic_prefix, stderr)? {
                        save_stack(env_vars, &stack);
                        return Ok(EXECUTION_FAILURE);
                    }
                }
                PushdOperand::Dir { dir, no_cd } => {
                    // GNU pushd.def:296-302 delegates the Dir operand to
                    // cd_builtin (`j = ((flags & NOCD) == 0) ? cd_builtin
                    // (...) : EXECUTION_SUCCESS`), so an operand that
                    // expanded to the empty string inherits cd.def:335's
                    // verdict — `builtin_error ("null directory")` +
                    // EXECUTION_FAILURE, with the prolog naming pushd, the
                    // executing builtin. (`HOME=; pushd ~` used to silently
                    // succeed as a no-op, rubash#754 family.)
                    if dir.is_empty() && !no_cd {
                        writeln!(stderr, "{diagnostic_prefix}pushd: null directory")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    // GNU pushd.def:223-225: a lone `-` breaks the option walk
                    // as a verbatim Dir operand, and pushd.def:298 routes it
                    // through cd_builtin — whose `cd -` arm (cd.def:339-349)
                    // substitutes $OLDPWD. An unset OLDPWD is `OLDPWD not set`
                    // (rc 1); a set-but-empty one canonicalizes to the cwd
                    // (change_to_directory("") via make_absolute + canon), a
                    // no-op success. LCD_PRINTPATH (cd.def:371-374) echoes the
                    // RAW value on success only — a stale OLDPWD prints no
                    // echo — and the failure diagnostic names the raw value
                    // too (cd.def:427), e.g. `pushd: /gone: No such file...`.
                    // With -n the word stays a raw push (`pushd -n -`
                    // stores "-"), never an OLDPWD lookup.
                    let resolved_dir = if dir == "-" && !no_cd {
                        let Some(oldpwd) = env_vars.get("OLDPWD").cloned() else {
                            writeln!(stderr, "{diagnostic_prefix}pushd: OLDPWD not set")?;
                            return Ok(EXECUTION_FAILURE);
                        };
                        let resolved = if oldpwd.is_empty() {
                            stack
                                .first()
                                .cloned()
                                .or_else(|| env_vars.get("PWD").cloned())
                                .unwrap_or_else(|| "/".to_string())
                        } else {
                            normalize_stack_dir(&oldpwd, env_vars)
                        };
                        if !logical_dir_exists(&resolved, env_vars) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}pushd: {oldpwd}: No such file or directory"
                            )?;
                            return Ok(EXECUTION_FAILURE);
                        }
                        writeln!(stdout, "{oldpwd}")?;
                        resolved
                    // pushd.def:301: with -n, cd_builtin is never called and
                    // `add_dirstack_element (savestring (list->word->word))`
                    // pushes the RAW operand word unconditionally — no
                    // normalization, no existence probe (that probe lives on
                    // the cd path only). An empty expansion therefore stays
                    // "" on the stack (`HOME=; pushd -n ~` yields
                    // `<cwd> ""`), and `pushd -n ./sub` stores "./sub".
                    } else if no_cd {
                        dir.clone()
                    } else {
                        let resolved = normalize_stack_dir(&dir, env_vars);
                        if !logical_dir_exists(&resolved, env_vars) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}pushd: {dir}: No such file or directory"
                            )?;
                            return Ok(EXECUTION_FAILURE);
                        }
                        resolved
                    };
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
                        // pushd.def:296-316: cd_builtin runs BEFORE
                        // add_dirstack_element, so a bindpwd readonly failure
                        // (cd.def:136-175) is the whole command's verdict:
                        // the element never joins the stack and dirs_builtin
                        // never runs. The chdir itself stands — GNU's cwd
                        // moved to the operand — so the display's cwd slot
                        // follows it, the same verdict popd inherits.
                        if bindpwd_write(
                            env_vars,
                            &old_pwd,
                            &resolved_dir,
                            diagnostic_prefix,
                            stderr,
                        )? {
                            if stack.is_empty() {
                                stack.push(resolved_dir);
                            } else {
                                stack[0] = resolved_dir;
                            }
                            save_stack(env_vars, &stack);
                            return Ok(EXECUTION_FAILURE);
                        }
                        stack.insert(0, resolved_dir);
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

            // GNU pushd.def:383-391 resolves every ±N operand to one array
            // slot before branching, and slot 0 (`popd`, `popd +0`,
            // `popd -N` naming the bottom of the display) is the top-of
            // -stack branch. The empty-stack verdict is pushd_error's
            // (pushd.def:540-546): "directory stack empty" whenever the
            // offset is 0, "directory stack index out of range" otherwise.
            let (index, no_cd) = match operand {
                PopdOperand::Top { no_cd } => (0usize, no_cd),
                PopdOperand::Index {
                    index,
                    from_right,
                    no_cd,
                    word,
                } => match resolved_index(index, from_right, stack.len()) {
                    Some(index) => (index, no_cd),
                    None => {
                        // pushd.def:382/:414 hand which_word — the ±N word the
                        // walk recorded — to pushd_error, so the diagnostic
                        // names the out-of-range operand even when options
                        // follow it (`popd +5 -n` reports "+5").
                        stack_range_error("popd", stack.len(), &word, diagnostic_prefix, stderr)?;
                        return Ok(EXECUTION_FAILURE);
                    }
                },
            };

            if index == 0 {
                if stack.len() <= 1 {
                    writeln!(stderr, "{diagnostic_prefix}popd: directory stack empty")?;
                    return Ok(EXECUTION_FAILURE);
                }
                // With -n the element is freed and nothing else happens
                // (pushd.def:397: cd_to_string is skipped entirely).
                if no_cd {
                    stack.remove(1);
                    save_stack(env_vars, &stack);
                    writeln!(stdout, "{}", stack.join(" "))?;
                    return Ok(EXECUTION_SUCCESS);
                }
                // pushd.def:394-403: cd_to_string(new top) runs BEFORE the
                // element is freed, so a failed chdir leaves the stack, PWD
                // and the display untouched — cd_builtin's diagnostic (under
                // the invoking builtin's prolog: `popd: word: strerror`) and
                // its EXECUTION_FAILURE are the only effects (rubash#830;
                // GNU bash 5.3.0 probes: rc 1, `dirs` still shows the entry).
                let target = stack[1].clone();
                let Some(chdir) = popd_chdir(&target, env_vars, stderr, diagnostic_prefix)? else {
                    return Ok(EXECUTION_FAILURE);
                };
                let old_pwd = env_vars
                    .get("PWD")
                    .cloned()
                    .unwrap_or_else(|| "/".to_string());
                // cd_builtin's bindpwd (cd.def:136-175) may fail on a
                // readonly OLDPWD/PWD; pushd.def:400-403 turns cd_to_string's
                // EXECUTION_FAILURE into an early return BEFORE the free.
                // The chdir verdict stands — GNU's cwd physically moved — so
                // the display's cwd slot still promotes to the chdir target
                // while the stack keeps both entries and nothing prints
                // (GNU 5.3.0: `readonly OLDPWD; popd` -> rc 1, dirs shows
                // "<new top> <new top>", $PWD already the target).
                let bind_failed =
                    bindpwd_write(env_vars, &old_pwd, &target, diagnostic_prefix, stderr)?;
                match chdir {
                    // A real chdir drops the physical-pwd override so
                    // `pwd -P` follows the real process cwd (cd.rs does the
                    // same after a chdir).
                    PopdChdir::Real => {
                        env_vars.remove("__RUBASH_PHYSICAL_PWD");
                    }
                    // cd's Windows-host bridge for the POSIX system names
                    // (cd.rs logical_posix_test_dir) pins the physical pwd to
                    // the logical name instead of chdir(2); popd's inner cd
                    // inherits that verdict verbatim.
                    PopdChdir::Bridged => {
                        env_vars.insert("__RUBASH_PHYSICAL_PWD".to_string(), target.clone());
                    }
                }
                if bind_failed {
                    stack[0] = target;
                    save_stack(env_vars, &stack);
                    return Ok(EXECUTION_FAILURE);
                }
                // Success: cd_builtin's bindpwd refreshed OLDPWD/PWD and the
                // display promotes the chdir target to the cwd slot — the
                // same net stack the GNU free() of the popped word leaves.
                stack.remove(0);
                save_stack(env_vars, &stack);
                writeln!(stdout, "{}", stack.join(" "))?;
                return Ok(EXECUTION_SUCCESS);
            }

            // Any other slot (pushd.def:404-421): the element is freed and
            // the remainder shifted; no chdir ever runs and PWD stays as it
            // was (GNU 5.3.0 probe: `popd +1` leaves pwd -P and $PWD alone).
            stack.remove(index);
            save_stack(env_vars, &stack);
            writeln!(stdout, "{}", stack.join(" "))?;
            Ok(EXECUTION_SUCCESS)
        }
    }
}

/// GNU pushd_error (pushd.def:540-546): an out-of-range operand names
/// "directory stack empty" while the offset is 0 — rubash's stack models the
/// cwd as slot 0, so `len <= 1` — and sh_erange's
/// "<word>: directory stack index out of range" (builtins/common.c:252)
/// otherwise, echoing the offending ±N word itself: the option walks hand the
/// word to pushd_error (pushd.def:242 inline for pushd, pushd.def:382/:414
/// via which_word for popd), never a trailing option word.
fn stack_range_error<E: Write>(
    builtin: &str,
    stack_len: usize,
    word: &str,
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<()> {
    if stack_len <= 1 {
        writeln!(
            stderr,
            "{diagnostic_prefix}{builtin}: directory stack empty"
        )
    } else {
        writeln!(
            stderr,
            "{diagnostic_prefix}{builtin}: {word}: directory stack index out of range"
        )
    }
}

/// GNU cd.def:136-175 bindpwd: cd_builtin's post-chdir refresh binds OLDPWD
/// to the pre-cd $PWD and PWD to the new directory; a readonly variable fails
/// its bind with err_readonly's "VAR: readonly variable" (error.c:453-456,
/// reached from bind_variable, variables.c:3131-3136 — one diagnostic per
/// variable, OLDPWD first) and the failure surfaces as cd_builtin's
/// EXECUTION_FAILURE. pushd.def routes every cd through cd_builtin —
/// cd_to_string (pushd.def:562-574), change_to_temp (pushd.def:577-586) and
/// the Dir branch's direct call (pushd.def:301) — so popd/pushd inherit the
/// verdict: the chdir stands, the diagnostic names the readonly variable
/// under the invoking builtin's prolog, and the caller skips its stack
/// mutation. Returns true when any bind failed.
fn bindpwd_write<E: Write>(
    env_vars: &mut HashMap<String, String>,
    old_pwd: &str,
    new_pwd: &str,
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<bool> {
    let mut failed = false;
    if var_is_readonly(env_vars, "OLDPWD") {
        writeln!(stderr, "{diagnostic_prefix}OLDPWD: readonly variable")?;
        failed = true;
    } else {
        env_vars.insert("OLDPWD".to_string(), old_pwd.to_string());
    }
    if var_is_readonly(env_vars, "PWD") {
        writeln!(stderr, "{diagnostic_prefix}PWD: readonly variable")?;
        failed = true;
    } else {
        env_vars.insert("PWD".to_string(), new_pwd.to_string());
    }
    Ok(failed)
}

/// The same readonly-marker membership cd.rs reads for its own bindpwd mirror
/// (`__RUBASH_READONLY_VARS`, DATA_DOLLAR-separated names).
fn var_is_readonly(env_vars: &HashMap<String, String>, name: &str) -> bool {
    env_vars
        .get(READONLY_VARS)
        .map(|vars| vars.split(DATA_DOLLAR).any(|var| var == name))
        .unwrap_or(false)
}

/// How the top-of-stack popd's inner cd (GNU cd_to_string) resolved.
enum PopdChdir {
    /// set_current_dir moved the shell process.
    Real,
    /// cd's Windows-host bridge (cd.rs logical_posix_test_dir): with no
    /// configured logical root, `/bin`-class POSIX names are not physical
    /// directories, so cd rewrites PWD and pins __RUBASH_PHYSICAL_PWD
    /// without a chdir — popd's inner cd inherits the same verdict.
    Bridged,
}

/// The top-of-stack popd chdir, GNU cd_to_string (pushd.def:559-572): a
/// cd_builtin call with the raw stack word. Returns None when the chdir
/// failed (diagnostic already printed).
fn popd_chdir<E: Write>(
    target: &str,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut E,
    diagnostic_prefix: &str,
) -> io::Result<Option<PopdChdir>> {
    if target.is_empty() {
        // cd.def:331-336 judges an operand that expanded to "" before any
        // path work; through cd_to_string the prolog names popd.
        writeln!(stderr, "{diagnostic_prefix}popd: null directory")?;
        return Ok(None);
    }
    // cd.rs's Windows-host bridge, same predicate and same match list: on an
    // unconfigured host the POSIX system names have no physical directory,
    // and cd reports success by rewriting the logical state only.
    if cfg!(windows)
        && !crate::executor::path::shell_root_configured(env_vars)
        && matches!(target, "/" | "/bin" | "/etc" | "/tmp" | "/usr")
    {
        return Ok(Some(PopdChdir::Bridged));
    }
    let native = crate::executor::path::shell_path_to_windows(target, env_vars);
    if let Err(error) = std::env::set_current_dir(&native) {
        // cd.def:427: builtin_error("%s: %s", dirname, strerror(e)) reports
        // the stored word, not the Windows-mapped path.
        writeln!(
            stderr,
            "{diagnostic_prefix}popd: {target}: {}",
            crate::posix_errors::message(&error)
        )?;
        stderr.flush()?;
        return Ok(None);
    }
    Ok(Some(PopdChdir::Real))
}
