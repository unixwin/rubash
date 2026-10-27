//! The history builtin over the session history list (builtins/history.def
//! operating on the shell's own list, not a host provider).
//!
//! GNU source ownership: builtins/history.def (history_builtin), with the
//! -p expansion going through histexpand.c history_expand.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use super::Executor;
use crate::history::SessionHistory;
use crate::history_expand::{HistChars, HistCtx};

/// The listing subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryMode {
    List,
    Delete,
    Print,
    Save,
    Append,
    Write,
    Read,
    ReadNew,
}

/// Build the expansion context (histchars plus posix mode) from the shell env.
pub(in crate::executor) fn hist_ctx(executor: &Executor) -> HistCtx {
    let chars = executor.get_env("histchars").unwrap_or("!^#");
    let mut it = chars.chars();
    HistCtx {
        chars: HistChars {
            expand: it.next().unwrap_or('!'),
            subst: it.next().unwrap_or('^'),
            comment: it.next().unwrap_or('#'),
        },
        posix: executor
            .get_env("__RUBASH_POSIX_MODE")
            .map(|v| v == "1")
            .unwrap_or(false),
    }
}

/// variables.c sv_histsize: unset/empty/negative/non-numeric HISTSIZE means
/// unstifled; a valid non-negative number stifles the list.
fn histsize_of(executor: &Executor) -> Option<usize> {
    SessionHistory::size_limit(executor.get_env("HISTSIZE"))
}

/// histfile.c:121 + variables.c:6168 sv_histtimefmt: timestamps are written
/// whenever HISTTIMEFORMAT exists as a shell variable (even set-but-empty).
fn write_timestamps_of(executor: &Executor) -> bool {
    executor.get_env("HISTTIMEFORMAT").is_some()
}

/// Execute the history builtin against the session list.
pub(in crate::executor) fn execute_history_session(
    executor: &Executor,
    args: &[String],
    session: Rc<RefCell<SessionHistory>>,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
) -> Result<i32, ExecuteErrorAlias> {
    let mut clear = false;
    let mut mode = HistoryMode::List;
    let mut count: Option<usize> = None;
    let mut delete_offset: Option<String> = None;
    let mut operands: Vec<String> = Vec::new();
    let mut expecting_offset = false;
    let mut no_more_opts = false;
    // history.def:161-166: track how many of -a/-r/-w/-n were given; GNU
    // rejects "cannot use more than one of -anrw" when 2+ are set.
    let mut anrw_count: u8 = 0;

    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        if expecting_offset {
            delete_offset = Some(arg.clone());
            expecting_offset = false;
            i += 1;
            continue;
        }
        if arg == "--" {
            no_more_opts = true;
            i += 1;
            continue;
        }
        if !no_more_opts && arg.len() >= 2 && arg.starts_with('-') {
            let mut bad: Option<char> = None;
            for (position, c) in arg[1..].char_indices() {
                match c {
                    'c' => clear = true,
                    'd' => {
                        mode = HistoryMode::Delete;
                        // internal_getopt: -d's optarg is the rest of the
                        // cluster when attached (-d5), else the next word.
                        let rest = &arg[position + 2..];
                        if rest.is_empty() {
                            expecting_offset = true;
                        } else {
                            delete_offset = Some(rest.to_string());
                        }
                        break;
                    }
                    'p' => {
                        mode = HistoryMode::Print;
                        no_more_opts = true;
                    }
                    's' => {
                        mode = HistoryMode::Save;
                        no_more_opts = true;
                    }
                    'a' => {
                        mode = HistoryMode::Append;
                        anrw_count += 1;
                    }
                    'w' => {
                        mode = HistoryMode::Write;
                        anrw_count += 1;
                    }
                    'r' => {
                        mode = HistoryMode::Read;
                        anrw_count += 1;
                    }
                    'n' => {
                        mode = HistoryMode::ReadNew;
                        anrw_count += 1;
                    }
                    other => bad = Some(other),
                }
            }
            if let Some(c) = bad {
                let _ = writeln!(
                    stderr,
                    "{}history: -{c}: invalid option",
                    executor.diagnostic_prefix()
                );
                let _ = writeln!(stderr, "history: usage: history [-c] [-d offset] [n] or history -anrw [filename] or history -ps arg [arg...]");
                return Ok(2);
            }
            i += 1;
            continue;
        }
        if mode == HistoryMode::List && count.is_none() && arg.chars().all(|c| c.is_ascii_digit()) {
            count = arg.parse::<usize>().ok();
            i += 1;
            continue;
        }
        operands.push(arg.clone());
        i += 1;
    }
    // internal_getopt reports a missing -d optarg before the builtin runs.
    if expecting_offset {
        let _ = writeln!(
            stderr,
            "{}history: -d: option requires an argument",
            executor.diagnostic_prefix()
        );
        let _ = writeln!(stderr, "history: usage: history [-c] [-d offset] [n] or history -anrw [filename] or history -ps arg [arg...]");
        return Ok(2);
    }
    // history.def:161-166: reject when more than one of -a/-r/-w/-n is set.
    if anrw_count > 1 {
        let _ = writeln!(
            stderr,
            "{}history: cannot use more than one of -anrw",
            executor.diagnostic_prefix()
        );
        return Ok(1);
    }

    // GNU display_history -> get_numeric_arg(list, 0, &limit): the operand
    // is validated as a number first (sh_neednumarg, EX_USAGE), then
    // no_args() rejects any further operands with `too many arguments`.
    // no_args jumps to DISCARD, so the listing never runs on error paths.
    if mode == HistoryMode::List {
        let mut rest = operands.as_slice();
        if count.is_none() {
            if let Some(first) = rest.first() {
                if first.parse::<i64>().is_err() {
                    let _ = writeln!(
                        stderr,
                        "{}history: {first}: numeric argument required",
                        executor.diagnostic_prefix()
                    );
                    return Ok(2);
                }
                count = first.parse::<usize>().ok();
                rest = &rest[1..];
            }
        }
        if !rest.is_empty() {
            let _ = writeln!(
                stderr,
                "{}history: too many arguments",
                executor.diagnostic_prefix()
            );
            // rubash#1660: the no_args DISCARD channel, shared with
            // `exit 1 2` and `shift 1 2` (rubash#1650): EX_BADUSAGE
            // plus the reader-level discard — the list loop skips
            // the rest of the failing logical line, a function body
            // unwinds with status 2 and its caller continues.
            executor.raise_evalerror_abort();
            executor.evalerror_exit_toomany_pending.set(true);
            return Ok(2);
        }
    }

    let ctx = hist_ctx(executor);
    let histsize = histsize_of(executor);
    let mut shell = session.borrow_mut();

    if clear {
        shell.clear();
    }

    match mode {
        HistoryMode::Delete => {
            // GNU 5.3 history.def:190-262: `history -d start-end` deletes
            // the INCLUSIVE range. The separator scan starts after a
            // leading '-' (`-2-4` = start -2, end 4); negative numbers
            // count back from the end of the list (-1 is the last entry);
            // positive numbers are displayed numbers, offset by
            // history_base (bashhist.c). valid_number accepts strtoimax
            // base-0 spellings (0xaf). The C code NUL-terminates the
            // separator in place, so erange diagnostics print the
            // offending SIDE (start text on start errors, end text on
            // end errors); first > last fails silently
            // (remove_history_range, readline/history.c).
            let len = shell.entries.len() as i128;
            // GNU valid_number (general.c:248) uses strtoimax base 10 --
            // no hex/octal spellings ("0xaf" is INVALID and reports the
            // whole argument after restoring the '-' separator).
            let parse_pos = |text: &str| -> Option<i128> {
                let value: i128 = text.parse().ok()?;
                if text.starts_with('-') && value < 0 {
                    Some(value + len)
                } else if value > 0 {
                    Some(value - shell.base as i128)
                } else {
                    Some(0)
                }
            };
            // rubash#1210: the SINGLE-offset delete follows history.def
            // delete_history_offset -> history_offset_arg: a bare 0 (or -0)
            // names no entry — GNU errors `history: 0: history position
            // out of range` — while a RANGE may legitimately start at 0
            // (remove_history_range maps it to the first entry). Keep the
            // range arms on parse_pos; the single-offset arm uses this
            // strict variant where 0 parses to no position.
            let parse_single = |text: &str| -> Option<i128> {
                let value: i128 = text.parse().ok()?;
                if value == 0 {
                    None
                } else {
                    parse_pos(text)
                }
            };
            if let Some(arg) = delete_offset.as_deref() {
                let search_from = if arg.starts_with('-') { 1 } else { 0 };
                let range_pos = arg[search_from..].find('-').map(|pos| pos + search_from);
                let ok = if let Some(pos) = range_pos {
                    let (start_text, end_text) = (&arg[..pos], &arg[pos + 1..]);
                    match (parse_pos(start_text), parse_pos(end_text)) {
                        (Some(start), Some(end))
                            if start >= 0
                                && end >= 0
                                && (start as usize) < shell.entries.len()
                                && (end as usize) < shell.entries.len()
                                && start <= end =>
                        {
                            shell.entries.drain(start as usize..=end as usize);
                            shell.timestamps.drain(start as usize..=end as usize);
                            true
                        }
                        (Some(start), Some(end)) if start >= 0 && start < len => {
                            // start in range, end bad (or first > last
                            // with a bad end): GNU reports the end text
                            let bad = if end < 0 || end >= len {
                                end_text
                            } else {
                                // first > last, both in range: silent
                                return Ok(1);
                            };
                            let _ = writeln!(
                                stderr,
                                "{}history: {bad}: history position out of range",
                                executor.diagnostic_prefix()
                            );
                            return Ok(1);
                        }
                        (Some(start), _) if start < 0 || start >= len => {
                            let _ = writeln!(
                                stderr,
                                "{}history: {start_text}: history position out of range",
                                executor.diagnostic_prefix()
                            );
                            return Ok(1);
                        }
                        _ => {
                            // unparseable sides: GNU restores the '-' and
                            // reports the WHOLE argument
                            let _ = writeln!(
                                stderr,
                                "{}history: {arg}: history position out of range",
                                executor.diagnostic_prefix()
                            );
                            return Ok(1);
                        }
                    }
                } else {
                    match parse_single(arg) {
                        Some(index) if index >= 0 && (index as usize) < shell.entries.len() => {
                            shell.entries.remove(index as usize);
                            shell.timestamps.remove(index as usize);
                            true
                        }
                        _ => {
                            if arg.parse::<i64>().is_err() {
                                let _ = writeln!(
                                    stderr,
                                    "{}history: {arg}: invalid number",
                                    executor.diagnostic_prefix()
                                );
                            } else {
                                let _ = writeln!(
                                    stderr,
                                    "{}history: {arg}: history position out of range",
                                    executor.diagnostic_prefix()
                                );
                            }
                            return Ok(1);
                        }
                    }
                };
                if !ok {
                    return Ok(1);
                }
            }
        }
        HistoryMode::Save => {
            // history.def:425-448: `-s` pops the last entry only when it
            // was added by the command line currently executing —
            // remember_on_history (set -o history) && hist_last_line_added
            // && !hist_last_line_pushed. A non-interactive shell never
            // pops, and a second -s in one line never pops the first -s's
            // entry. The push goes through check_add_history (FORCE=1),
            // i.e. record() with HISTCONTROL/HISTIGNORE honored.
            let remember = crate::builtins::set::shell_option_enabled(
                &executor.shell_state.env_vars,
                "history",
            );
            if remember
                && !shell.last_line_pushed
                && shell.last_line_added
                && !shell.entries.is_empty()
            {
                shell.entries.pop();
                shell.timestamps.pop();
            }
            let command = operands.join(" ");
            if !command.is_empty() {
                let control = executor.get_env("HISTCONTROL").unwrap_or_default();
                let ignore = executor.get_env("HISTIGNORE").unwrap_or_default();
                let was_recorded = shell.record(&command, &control, &ignore, histsize);
                // push_history goes through check_add_history ->
                // really_add_history (bashhist.c:961), which sets
                // hist_last_line_added = 1 for the pushed entry, and
                // push_history then marks it pushed (history.def:451).
                // fc's last_hist arithmetic (fc.def:335) depends on the
                // added flag being set here (#1117).
                if was_recorded {
                    shell.last_line_added = true;
                }
                shell.last_line_pushed = true;
            }
        }
        HistoryMode::Print => {
            // history.def:462-463: `-p` pops the current-line entry under
            // the same conditions as -s; failure to delete is a hard error.
            let remember = crate::builtins::set::shell_option_enabled(
                &executor.shell_state.env_vars,
                "history",
            );
            if remember && !shell.last_line_pushed && shell.last_line_added {
                if shell.entries.is_empty() {
                    return Ok(1);
                }
                shell.entries.pop();
                shell.timestamps.pop();
            }
            // builtins/history.def expand_and_print_history: every
            // operand is processed in order and successful expansions
            // still print, but any failure flips the overall result to
            // EXECUTION_FAILURE (rubash#1627).
            let mut any_failed = false;
            for operand in &operands {
                let result = shell.expand(operand, ctx);
                if result.status < 0 {
                    let _ = writeln!(
                        stderr,
                        "{}history: {operand}: history expansion failed",
                        executor.diagnostic_prefix()
                    );
                    any_failed = true;
                    continue;
                }
                let _ = writeln!(stdout, "{}", result.text);
            }
            if any_failed {
                return Ok(1);
            }
        }
        HistoryMode::Append | HistoryMode::Write | HistoryMode::Read | HistoryMode::ReadNew => {
            // history.def:159,273: filename is the first non-option argument
            // (loptend); fall back to $HISTFILE when none is given.
            let from_histfile = operands.is_empty();
            // builtins/history.def history_builtin: the null-filename
            // block (unset/empty $HISTFILE, or an explicitly empty
            // operand) prints its diagnostic and returns
            // `interactive ? EXECUTION_SUCCESS : EXECUTION_FAILURE'.
            // `interactive' is shell.c:143's global — set together with
            // interactive_shell, and clear for a -c unit even under -i
            // (shell.c init_noninteractive) — which is the engine's
            // interactive_reader_session predicate (rubash#1830).
            let null_filename_status = if executor.interactive_reader_session() {
                0
            } else {
                1
            };
            let Some(path) = operands
                .first()
                .cloned()
                .or_else(|| executor.get_env("HISTFILE").map(String::from))
            else {
                // builtins/history.def history_builtin file branch: with
                // no operand the filename comes from $HISTFILE, and an
                // unset $HISTFILE draws the `parameter null or not set'
                // diagnostic — for -a/-n/-r/-w alike (rubash#1633).
                let _ = writeln!(
                    stderr,
                    "{}history: HISTFILE: parameter null or not set",
                    executor.diagnostic_prefix()
                );
                return Ok(null_filename_status);
            };
            // GNU history.def: a null $HISTFILE is its own diagnostic
            // (rubash#1289 B3), for both -w and -a.
            if from_histfile && path.is_empty() {
                let _ = writeln!(
                    stderr,
                    "{}history: HISTFILE: parameter null or not set",
                    executor.diagnostic_prefix()
                );
                return Ok(null_filename_status);
            }
            // builtins/history.def: an explicitly empty operand is the
            // separate `empty filename' diagnostic (rubash#1633).
            if !from_histfile && path.is_empty() {
                let _ = writeln!(
                    stderr,
                    "{}history: empty filename",
                    executor.diagnostic_prefix()
                );
                return Ok(null_filename_status);
            }
            // Translate Git-Bash/POSIX spellings (/c/..., /dev/null) to the
            // Windows forms the file APIs need; children see the same file.
            let path =
                crate::executor::path::shell_path_to_windows(&path, &executor.shell_state.env_vars)
                    .to_string_lossy()
                    .to_string();
            // variables.c:6168 sv_histtimefmt: this same condition (the
            // variable exists) is what arms readline's timestamp comment
            // char; the read paths take their arming state from it
            // (rubash#1669).
            let write_ts = write_timestamps_of(executor);
            let outcome = match mode {
                HistoryMode::Append => shell.append_file(&path, write_ts).map(|_| 0),
                // GNU builtins/history.def WFLAG branch: write_history
                // does NOT reset history_lines_this_session — only -a
                // (maybe_append_history) consumes the session counter.
                // Resetting here starved the exit-time append in
                // finish_shell of lines already written by -w
                // (rubash#1049: GNU re-appends them at exit).
                HistoryMode::Write => shell.write_file(&path, write_ts).map(|_| 0),
                HistoryMode::Read => shell
                    .read_file_with_arming(&path, histsize, write_ts)
                    .map(|_| 0),
                HistoryMode::ReadNew => shell
                    .read_new_file_with_arming(&path, histsize, write_ts)
                    .map(|_| 0),
                _ => Ok(0),
            };
            if let Err(err) = outcome {
                // GNU's diagnostic surface for -w/-a file errors is
                // narrow (rubash#1289, verified on GNU bash 5.3.20):
                // -w failures are silent; -a prints exactly one
                // `cannot create' line and only when the target does
                // not exist — bashhist.c:450 maybe_append_history runs
                // its pre-create open exactly on the stat-ENOENT branch,
                // and every later append failure (a directory target,
                // EISDIR) stays silent. The portable form of that
                // condition is the error's NotFound kind: std maps both
                // ENOENT and Windows' ERROR_PATH_NOT_FOUND to it, and
                // posix_errors renders both with the same GNU text —
                // the old raw_os_error()==2 gate dropped the line on
                // Windows (rubash#1830). Read modes keep the existing
                // `cannot open' report.
                match mode {
                    HistoryMode::Write => {}
                    HistoryMode::Append => {
                        if err.kind() == std::io::ErrorKind::NotFound {
                            let _ = writeln!(
                                stderr,
                                "{}history: {path}: cannot create: {}",
                                executor.diagnostic_prefix(),
                                crate::posix_errors::message(&err)
                            );
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            stderr,
                            "{}history: {path}: cannot open: {err}",
                            executor.diagnostic_prefix()
                        );
                    }
                }
                return Ok(1);
            }
        }
        HistoryMode::List => {
            let base = shell.base;
            let entries = shell.entries.clone();
            let timestamps = shell.timestamps.clone();
            // GNU builtins/history.def (display_history): with
            // HISTTIMEFORMAT set and non-null, each line carries the
            // entry's timestamp rendered by histtime between the
            // marker and the entry text (rubash#1070, #1669).
            let time_format = executor
                .get_env("HISTTIMEFORMAT")
                .filter(|value| !value.is_empty());
            let start = match count {
                Some(n) => entries.len().saturating_sub(n),
                None => 0,
            };
            for (index, entry) in entries.iter().enumerate().skip(start) {
                // GNU history.def:407-408: %5d%c %s with the continuation
                // marker "*" for entries carrying multi-line histdata.
                // histdata is only set for interactive continuations;
                // script entries always print the space marker (history.def:408).
                let mark = " ";
                match &time_format {
                    Some(format) => {
                        // builtins/history.def histtime via
                        // lib/readline/history.c history_get_time
                        // (rubash#1669): strtol stamp parsing, the
                        // `<text>: invalid timestamp` form, and `??`
                        // for stamp-less entries.
                        let rendered = crate::history::render_timestamp(
                            timestamps.get(index).map(String::as_str),
                            format,
                            &executor.shell_state.env_vars,
                        );
                        let _ =
                            writeln!(stdout, "{:>5}{} {}{}", base + index, mark, rendered, entry);
                    }
                    None => {
                        let _ = writeln!(stdout, "{:>5}{} {}", base + index, mark, entry);
                    }
                }
            }
        }
    }

    Ok(0)
}

use crate::executor::ExecuteError as ExecuteErrorAlias;
