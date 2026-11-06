//! pushd module.
//!
//! GNU Bash source ownership:
// - builtins/pushd.def

mod parse;
mod stack;

pub(crate) use stack::{
    load_stack, load_stack_carrier, save_stack, set_stack_value, stack_value, stack_words,
};

use crate::executor::builtin_diag::Diag;
use crate::executor::builtin_table::BuiltinRow;
use crate::executor::types::READONLY_VARS;
use crate::shell::var_table::VarTable;
use parse::{parse_popd_operand, parse_pushd_operand, ParseOutcome, PopdOperand, PushdOperand};
use stack::{dir_operand_error_text, logical_dir_exists, normalize_stack_dir};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
/// GNU general.c:953 polite_directory_format: when $HOME is longer than one
/// byte and `name` is exactly $HOME or starts with `$HOME/`, the prefix is
/// replaced with `~`. `dirs`/`pushd`/`popd` displays
/// (builtins/pushd.def:493 DIRSTACK_FORMAT, :514 DIRSTACK_ENTRY) apply it
/// unless -l (LONGFORM) is set; the DIRSTACK array itself stays raw
/// (pushd.def:669 get_directory_stack(0) via variables.c get_dirstack).
fn polite_directory_format(
    name: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(home) = env_vars.get("HOME") {
        if home.len() > 1 && (name == home.as_str() || name.starts_with(&format!("{home}/"))) {
            return format!("~{}", &name[home.len()..]);
        }
    }
    name.to_string()
}

/// Join the stack the way the default `dirs` display does: every entry
/// through [`polite_directory_format`] (GNU pushd.def calls
/// `dirs_builtin(NULL)` after pushd/popd, so those trailing listings are
/// polite too).
fn join_for_display(
    stack: &[String],
    env_vars: &std::collections::HashMap<String, String>,
) -> String {
    stack
        .iter()
        .map(|entry| polite_directory_format(entry, env_vars))
        .collect::<Vec<_>>()
        .join(" ")
}
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

pub fn execute_with_io<'a, I, W>(
    row: &'a BuiltinRow,
    builtin: StackBuiltin,
    args: I,
    env_vars: &mut crate::shell::var_table::VarTable,
    cwd: &mut PathBuf,
    diag: &mut Diag<'a>,
    stdout: &mut W,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    // CHECK_HELPOPT (pushd.def:183/:330/:433): the FIRST word `--help`
    // prints help and returns EX_USAGE, per the row's `help_entry` bit.
    // The stack family are NoScan rows — the option walks are faithful
    // GNU ISOPTION loops, not BuiltinOpt scans — so the check lives
    // here, not in the scan. A `--help` anywhere else is a loop
    // operand (`dirs +1 --help` is `invalid number`).
    if row.help_entry && args.first().copied() == Some("--help") {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(EX_USAGE);
    }
    let mut stack = load_stack(env_vars, cwd);

    match builtin {
        StackBuiltin::Dirs => {
            // GNU pushd.def:424-464 dirs_builtin: the ISOPTION loop
            // CONTINUES after ±N words (last wins) and after -c anywhere;
            // a non-flag word is `sh_invalidopt`, a ±-shaped non-number
            // is `sh_invalidnum`. CLEARSTAK is checked BEFORE the index
            // print, so `dirs -l -c` clears (rc 0) and `dirs +1 -c`
            // clears instead of printing the entry (rubash#2937).
            // These are NOT internal_getopt loops — a plain BuiltinOpt
            // spec would wrongly accept clusters like `-lv`.
            let mut long_form = false;
            let mut clear = false;
            let mut numbered = false;
            let mut printable = false;
            let mut desired: Option<DirsIndex> = None;
            let mut w = "";
            let mut index = 0;
            while let Some(word) = args.get(index).copied() {
                match word {
                    "-l" => long_form = true,
                    "-c" => clear = true,
                    "-v" => numbered = true,
                    "-p" => printable = true,
                    // GNU pushd.def:445-448: ISOPTION '-' skips the
                    // word and breaks; the rest is ignored entirely
                    // (rubash#1380: `dirs -- -l`/`-- -c` must not apply).
                    "--" => {
                        index += 1;
                        break;
                    }
                    _ if word.starts_with('+') || word.starts_with('-') => {
                        // pushd.def:449-461: `valid_number (word + 1)`
                        // else `sh_invalidnum (word)` + usage + EX_USAGE;
                        // `w` keeps the digits part for the range error;
                        // `get_dirstack_index` converts to a display slot
                        // (last ±N word wins).
                        let Ok(digits) = word[1..].parse::<i64>() else {
                            diag.free(format_args!("{word}: invalid number"));
                            diag.usage();
                            return Ok(EX_USAGE);
                        };
                        w = &word[1..];
                        desired = Some(dirs_display_index(
                            digits,
                            word.starts_with('+'),
                            stack.len(),
                        ));
                    }
                    _ => {
                        diag.free(format_args!("{word}: invalid option"));
                        diag.usage();
                        return Ok(EX_USAGE);
                    }
                }
                index += 1;
            }
            if clear {
                save_stack(env_vars, &[]);
                return Ok(EXECUTION_SUCCESS);
            }
            if let Some(desired) = desired {
                let shown = |dir: &str| {
                    if long_form {
                        dir.to_string()
                    } else {
                        polite_directory_format(dir, env_vars)
                    }
                };
                return match desired {
                    DirsIndex::OutOfRange => {
                        pushd_error("dirs", stack.len() - 1, w, diag);
                        Ok(EXECUTION_FAILURE)
                    }
                    DirsIndex::Cwd => {
                        if numbered {
                            writeln!(stdout, "{:2}  {}", 0, shown(&stack[0]))?;
                        } else {
                            writeln!(stdout, "{}", shown(&stack[0]))?;
                        }
                        Ok(EXECUTION_SUCCESS)
                    }
                    DirsIndex::Entry(display_index) => {
                        let shown = shown(&stack[display_index]);
                        if numbered {
                            writeln!(stdout, "{display_index:2}  {shown}")?;
                        } else {
                            writeln!(stdout, "{shown}")?;
                        }
                        Ok(EXECUTION_SUCCESS)
                    }
                };
            }
            if numbered {
                for (index, dir) in stack.iter().enumerate() {
                    let shown = if long_form {
                        dir.clone()
                    } else {
                        polite_directory_format(dir, env_vars)
                    };
                    writeln!(stdout, "{index:2}  {shown}")?;
                }
            } else if printable {
                for dir in &stack {
                    let shown = if long_form {
                        dir.clone()
                    } else {
                        polite_directory_format(dir, env_vars)
                    };
                    writeln!(stdout, "{shown}")?;
                }
            } else {
                // load_stack always exposes the cwd as element 0, so the
                // plain (and -l) listing joins the stack; the old dead
                // empty-stack arm only ever echoed $PWD.
                if long_form {
                    writeln!(stdout, "{}", stack.join(" "))?;
                } else {
                    writeln!(stdout, "{}", join_for_display(&stack, env_vars))?;
                }
            }
            Ok(EXECUTION_SUCCESS)
        }
        StackBuiltin::Pushd => {
            // The walk already printed any usage/range diagnostic; the
            // outcome only carries the status it implies (EX_USAGE for a
            // usage diagnostic, EXECUTION_FAILURE for pushd_error).
            let operand = match parse_pushd_operand(&args, stack.len() - 1, diag) {
                ParseOutcome::Ok(operand) => operand,
                ParseOutcome::Usage => return Ok(EX_USAGE),
                ParseOutcome::Failed => return Ok(EXECUTION_FAILURE),
            };

            match operand {
                PushdOperand::Swap => {
                    if stack.len() < 2 {
                        diag.free(format_args!("no other directory"));
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
                    // G-02: change_to_temp's chdir moves the session
                    // cwd field; it stands even if bindpwd fails.
                    *cwd = physical_cwd_for(&new_pwd, env_vars, cwd);
                    if bindpwd_write(env_vars, &old_pwd, &new_pwd, diag) {
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
                    diag.free(format_args!("too many arguments"));
                    return Ok(EXECUTION_FAILURE);
                }
                PushdOperand::Index { num, no_cd, .. } => {
                    // The walk already range-checked `num` inline
                    // (pushd.def:235-244), so `num` is in `0..=offset`
                    // and the rotation below cannot fail.
                    if no_cd {
                        // pushd.def:262-286: the rotation lands in the
                        // list even with NOCD — only the rotated-to
                        // entry (temp) is discarded and nothing prints.
                        // In display terms the rotation is a left
                        // rotation by num with slot 0 re-seeded to the
                        // old cwd: `pushd +1 -n` turns
                        // `[d2, d1, d0, W]` into `[d2, d0, W, d2]`
                        // (rubash#2937). The old code skipped the
                        // rotation entirely.
                        let old_pwd = stack[0].clone();
                        stack.rotate_left(num);
                        stack[0] = old_pwd;
                        save_stack(env_vars, &stack);
                        return Ok(EXECUTION_SUCCESS);
                    }
                    // pushd.def:256-286: the rotation lands in the list
                    // before change_to_temp, so a bindpwd readonly failure
                    // keeps the rotated list while the display and the
                    // success status stay behind change_to_temp's
                    // EXECUTION_SUCCESS gate (pushd.def:577-586). In
                    // display terms the rotation is a left rotation by
                    // num: `[d2, d1, d0, W]` rotated by 1 is
                    // `[d1, d0, W, d2]`, which is GNU's
                    // `[temp] ++ reverse (list')` (rubash#2937).
                    let old_pwd = env_vars
                        .get("PWD")
                        .cloned()
                        .unwrap_or_else(|| "/".to_string());
                    stack.rotate_left(num);
                    let new_pwd = stack[0].clone();
                    // G-02: the rotation's chdir moves the session cwd
                    // field; it stands even if bindpwd fails.
                    *cwd = physical_cwd_for(&new_pwd, env_vars, cwd);
                    if bindpwd_write(env_vars, &old_pwd, &new_pwd, diag) {
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
                        diag.free(format_args!("null directory"));
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
                            diag.free(format_args!("OLDPWD not set"));
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
                            diag.free(format_args!(
                                "{oldpwd}: {}",
                                dir_operand_error_text(&resolved, env_vars)
                            ));
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
                            diag.free(format_args!(
                                "{dir}: {}",
                                dir_operand_error_text(&resolved, env_vars)
                            ));
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
                        // G-02: that chdir is the session cwd field move.
                        *cwd = physical_cwd_for(&resolved_dir, env_vars, cwd);
                        if bindpwd_write(env_vars, &old_pwd, &resolved_dir, diag) {
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
            writeln!(stdout, "{}", join_for_display(&stack, env_vars))?;
            Ok(EXECUTION_SUCCESS)
        }
        StackBuiltin::Popd => {
            // The walk already printed any usage/range diagnostic; the
            // outcome only carries the status it implies.
            let operand = match parse_popd_operand(&args, stack.len() - 1, diag) {
                ParseOutcome::Ok(operand) => operand,
                ParseOutcome::Usage => return Ok(EX_USAGE),
                ParseOutcome::Failed => return Ok(EXECUTION_FAILURE),
            };

            // GNU pushd.def:383-391 resolves every ±N operand to one array
            // slot before branching, and slot 0 (`popd`, `popd +0`,
            // `popd -N` naming the bottom of the display) is the top-of
            // -stack branch. The empty-stack verdict is pushd_error's
            // (pushd.def:540-546) — the walk already reported it for the
            // empty-stack shapes, so the `len <= 1` guard below is
            // unreachable through the parse; it stays as a defensive
            // bound for the `stack[1]` read.
            let (index, no_cd) = match operand {
                PopdOperand::Top { no_cd } => (0usize, no_cd),
                PopdOperand::Index {
                    display_index,
                    no_cd,
                } => (display_index, no_cd),
            };

            if index == 0 {
                if stack.len() <= 1 {
                    diag.free(format_args!("directory stack empty"));
                    return Ok(EXECUTION_FAILURE);
                }
                // With -n the element is freed and nothing else happens
                // (pushd.def:397: cd_to_string is skipped entirely).
                if no_cd {
                    stack.remove(1);
                    save_stack(env_vars, &stack);
                    writeln!(stdout, "{}", join_for_display(&stack, env_vars))?;
                    return Ok(EXECUTION_SUCCESS);
                }
                // pushd.def:394-403: cd_to_string(new top) runs BEFORE the
                // element is freed, so a failed chdir leaves the stack, PWD
                // and the display untouched — cd_builtin's diagnostic (under
                // the invoking builtin's prolog: `popd: word: strerror`) and
                // its EXECUTION_FAILURE are the only effects (rubash#830;
                // GNU bash 5.3.0 probes: rc 1, `dirs` still shows the entry).
                let target = stack[1].clone();
                let Some(chdir) = popd_chdir(&target, env_vars, cwd, diag)? else {
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
                let bind_failed = bindpwd_write(env_vars, &old_pwd, &target, diag);
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
                writeln!(stdout, "{}", join_for_display(&stack, env_vars))?;
                return Ok(EXECUTION_SUCCESS);
            }

            // Any other slot (pushd.def:404-421): the element is freed and
            // the remainder shifted; no chdir ever runs and PWD stays as it
            // was (GNU 5.3.0 probe: `popd +1` leaves pwd -P and $PWD alone).
            stack.remove(index);
            save_stack(env_vars, &stack);
            writeln!(stdout, "{}", join_for_display(&stack, env_vars))?;
            Ok(EXECUTION_SUCCESS)
        }
    }
}

/// Which display slot a `dirs ±N` word names, after GNU's
/// `get_dirstack_index` (pushd.def:600-627) — expressed in rubash's
/// display terms, where slot 0 is the cwd and the list holds the
/// display tail oldest-first, without the cwd.
enum DirsIndex {
    /// `+0`, or `-offset`: the cwd itself (index_flag 1, desired 0).
    Cwd,
    /// A display slot (`+N` is N, `-N` is `len - 1 - N`, `+offset`
    /// and `-0` are the oldest entry, `len - 1`).
    Entry(usize),
    /// `N > offset`: `pushd_error` names the digits part `w`.
    OutOfRange,
}

/// GNU pushd.def:600-627 `get_dirstack_index (i, sign, &index_flag)`:
/// `+0` returns 0 with index_flag 1 (cwd); `-0` returns 0 with
/// index_flag 2 (list[0], the oldest); `ind == offset` returns 0
/// with the flag flipped (`+offset` is list[0], `-offset` is the
/// cwd); `0 <= ind <= offset` returns `offset - ind` for `+` and
/// `ind` for `-`; anything else returns -1 (out of range). `len` is
/// the display length (`offset + 1`).
fn dirs_display_index(digits: i64, plus: bool, len: usize) -> DirsIndex {
    let offset = len as i64 - 1;
    if digits == 0 && plus {
        return DirsIndex::Cwd;
    }
    if digits == offset {
        return if plus {
            DirsIndex::Entry(len - 1)
        } else {
            DirsIndex::Cwd
        };
    }
    if digits >= 0 && digits <= offset {
        if plus {
            DirsIndex::Entry(digits as usize)
        } else {
            DirsIndex::Entry((offset - digits) as usize)
        }
    } else {
        DirsIndex::OutOfRange
    }
}

/// GNU pushd_error (pushd.def:540-546): an out-of-range operand names
/// "directory stack empty" while the offset is 0 — rubash's stack
/// models the cwd as slot 0, so `len <= 1` — and sh_erange's
/// "<word>: directory stack index out of range" (builtins/common.c:252)
/// otherwise, echoing the offending ±N word itself: the option walks
/// hand the word to pushd_error (pushd.def:242 inline for pushd,
/// pushd.def:382/:414 via which_word for popd), never a trailing
/// option word. `dirs` hands the digits part `w` instead (the loop
/// assigns `w = word + 1` before calling it).
pub(super) fn pushd_error(builtin: &str, offset: usize, word: &str, diag: &mut Diag<'_>) {
    if offset == 0 {
        diag.free(format_args!("{builtin}: directory stack empty"));
    } else {
        diag.free(format_args!(
            "{builtin}: {word}: directory stack index out of range"
        ));
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
///
/// Stage-2 ④ Phase 2: the readonly diagnostics write through the
/// `Diag` sink (`Diag::sink`), NOT through `Diag::free` — they are
/// `err_readonly` lines (`{prefix}VAR: readonly variable`, error.c:453)
/// with no builtin name and a gnu_errfmt-aware prefix of their own.
fn bindpwd_write(
    env_vars: &mut crate::shell::var_table::VarTable,
    old_pwd: &str,
    new_pwd: &str,
    diag: &mut Diag<'_>,
) -> bool {
    // GNU error.c:453 err_readonly (report_error family): the bare
    // `VAR: readonly variable` bind diagnostic honors gnu_errfmt
    // (rubash#1038), unlike the builtin_error lines sharing this file.
    // The no-script fallback matches the executor-supplied
    // diagnostic_prefix ("bash: "), not this file's local "rubash: ".
    let error_prefix = if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        if crate::builtins::shopt::option_enabled(env_vars, "gnu_errfmt") {
            format!("{script}:{line}: ")
        } else {
            format!("{script}: line {line}: ")
        }
    } else {
        "bash: ".to_string()
    };
    let mut failed = false;
    if var_is_readonly(env_vars, "OLDPWD") {
        diag.sink()
            .write_str(&format!("{error_prefix}OLDPWD: readonly variable\n"));
        failed = true;
    } else {
        env_vars.insert("OLDPWD".to_string(), old_pwd.to_string());
    }
    if var_is_readonly(env_vars, "PWD") {
        diag.sink()
            .write_str(&format!("{error_prefix}PWD: readonly variable\n"));
        failed = true;
    } else {
        env_vars.insert("PWD".to_string(), new_pwd.to_string());
    }
    failed
}

/// The same readonly-marker membership cd.rs reads for its own bindpwd mirror
/// (`__RUBASH_READONLY_VARS`, DATA_DOLLAR-separated names).
fn var_is_readonly(env_vars: &crate::shell::var_table::VarTable, name: &str) -> bool {
    env_vars.is_marked(READONLY_VARS, name)
}

/// How the top-of-stack popd's inner cd (GNU cd_to_string) resolved.
enum PopdChdir {
    /// The session cwd field moved to the target.
    Real,
    /// cd's Windows-host bridge (cd.rs logical_posix_test_dir): with no
    /// configured logical root, `/bin`-class POSIX names are not physical
    /// directories, so cd rewrites PWD and pins __RUBASH_PHYSICAL_PWD
    /// without a chdir — popd's inner cd inherits the same verdict.
    Bridged,
}

/// G-02: the physical session-cwd value for a stack directory word
/// (the logical display form the stack and $PWD carry): map to the
/// native form, join the session cwd for relative words, canonicalize
/// — the same physical resolution cd's move probe performs.
fn physical_cwd_for(
    word: &str,
    env_vars: &std::collections::HashMap<String, String>,
    cwd: &Path,
) -> PathBuf {
    let native = crate::executor::path::shell_path_to_windows(word, env_vars);
    let joined = if native.is_absolute() {
        native
    } else {
        cwd.join(native)
    };
    crate::executor::path::canonicalize_native(&joined).unwrap_or(joined)
}

/// The error chdir would have reported for a target that exists but
/// is not a directory (ENOTDIR / ERROR_DIRECTORY), in a form
/// `posix_errors::message` renders as "Not a directory".
fn not_a_directory_error() -> io::Error {
    #[cfg(windows)]
    {
        io::Error::from_raw_os_error(267)
    }
    #[cfg(not(windows))]
    {
        io::Error::from_raw_os_error(20)
    }
}

/// The top-of-stack popd chdir, GNU cd_to_string (pushd.def:559-572): a
/// cd_builtin call with the raw stack word. Returns None when the chdir
/// failed (diagnostic already printed).
fn popd_chdir(
    target: &str,
    env_vars: &mut VarTable,
    cwd: &mut PathBuf,
    diag: &mut Diag<'_>,
) -> io::Result<Option<PopdChdir>> {
    if target.is_empty() {
        // cd.def:331-336 judges an operand that expanded to "" before any
        // path work; through cd_to_string the prolog names popd.
        diag.free(format_args!("null directory"));
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
    // G-02: the chdir is a session cwd field move, probed the way cd
    // probes (canonicalize carries chdir's error codes; a plain file
    // canonicalizes fine but is ENOTDIR to chdir). Raw stack words can
    // be relative (`pushd -n ./sub` stores them verbatim) — they anchor
    // at the session cwd.
    let joined = if native.is_absolute() {
        native
    } else {
        cwd.join(&native)
    };
    let probed = match crate::executor::path::canonicalize_native(&joined) {
        Ok(path) if path.is_dir() => Ok(path),
        Ok(_) => Err(not_a_directory_error()),
        Err(error) => Err(error),
    };
    if let Err(error) = probed {
        // cd.def:427: builtin_error("%s: %s", dirname, strerror(e)) reports
        // the stored word, not the Windows-mapped path.
        diag.free(format_args!(
            "{target}: {}",
            crate::posix_errors::message(&error)
        ));
        return Ok(None);
    }
    *cwd = probed.unwrap();
    Ok(Some(PopdChdir::Real))
}
