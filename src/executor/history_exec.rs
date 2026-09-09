//! The history builtin over the session history list (builtins/history.def
//! operating on the shell's own list, not a host provider).
//!
//! GNU source ownership: builtins/history.def (history_builtin), with the
//! -p expansion going through histexpand.c history_expand.

use std::cell::RefCell;
use std::rc::Rc;
use std::io::Write;

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
        posix: executor.get_env("__RUBASH_POSIX_MODE").map(|v| v == "1").unwrap_or(false),
    }
}

fn histsize_of(executor: &Executor) -> usize {
    executor
        .get_env("HISTSIZE")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
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
    let mut file: Option<String> = None;
    let mut operands: Vec<String> = Vec::new();
    let mut expecting_offset = false;
    let mut expecting_file = false;
    let mut no_more_opts = false;

    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        if expecting_offset {
            delete_offset = Some(arg.clone());
            expecting_offset = false;
            i += 1;
            continue;
        }
        if expecting_file {
            file = Some(arg.clone());
            expecting_file = false;
            i += 1;
            continue;
        }
        if arg == "--" {
            no_more_opts = true;
            i += 1;
            continue;
        }
        if !no_more_opts && arg.len() >= 2 && arg.starts_with('-') {
            // A bare negative number is a listing count (history -5).
            if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                count = arg[1..].parse::<usize>().ok();
                i += 1;
                continue;
            }
            let mut bad: Option<char> = None;
            for c in arg[1..].chars() {
                match c {
                    'c' => clear = true,
                    'd' => {
                        mode = HistoryMode::Delete;
                        expecting_offset = true;
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
                        expecting_file = true;
                    }
                    'w' => {
                        mode = HistoryMode::Write;
                        expecting_file = true;
                    }
                    'r' => {
                        mode = HistoryMode::Read;
                        expecting_file = true;
                    }
                    'n' => {
                        mode = HistoryMode::ReadNew;
                        expecting_file = true;
                    }
                    other => bad = Some(other),
                }
            }
            if let Some(c) = bad {
                let _ = writeln!(stderr, "history: -{c}: invalid option");
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

    let ctx = hist_ctx(executor);
    let histsize = histsize_of(executor);
    let mut shell = session.borrow_mut();

    if clear {
        shell.clear();
    }

    match mode {
        HistoryMode::Delete => {
            if let Some(offset) = delete_offset.as_deref().and_then(|v| v.parse::<i64>().ok()) {
                let len = shell.entries.len();
                let index: Option<usize> = if offset >= 0 {
                    let offset = offset as usize;
                    if offset >= shell.base && offset < shell.base + len {
                        Some(offset - shell.base)
                    } else {
                        None
                    }
                } else {
                    let back = (-offset) as usize;
                    if back >= 1 && back <= len {
                        Some(len - back)
                    } else {
                        None
                    }
                };
                if let Some(index) = index {
                    shell.entries.remove(index);
                }
            }
        }
        HistoryMode::Save => {
            if shell.last_line_added && !shell.entries.is_empty() {
                shell.entries.pop();
            }
            let command = operands.join(" ");
            if !command.is_empty() {
                shell.entries.push(command);
                shell.stifle(histsize);
                shell.lines_this_session += 1;
                shell.last_line_added = true;
            }
        }
        HistoryMode::Print => {
            for operand in &operands {
                let result = shell.expand(operand, ctx);
                if result.status < 0 {
                    let _ = writeln!(stderr, "history: {operand}: history expansion failed");
                    continue;
                }
                let _ = writeln!(stdout, "{}", result.text);
            }
        }
        HistoryMode::Append | HistoryMode::Write | HistoryMode::Read | HistoryMode::ReadNew => {
            let Some(path) = file.or_else(|| executor.get_env("HISTFILE").map(String::from)) else {
                let _ = writeln!(stderr, "history: filename not specified");
                return Ok(2);
            };
            let outcome = match mode {
                HistoryMode::Append => shell.append_file(&path).map(|_| 0),
                HistoryMode::Write => shell.write_file(&path).map(|_| {
                    shell.entries_written = shell.entries.len();
                    0
                }),
                HistoryMode::Read => shell.read_file(&path, histsize).map(|_| 0),
                HistoryMode::ReadNew => shell.read_new_file(&path, histsize).map(|_| 0),
                _ => Ok(0),
            };
            if let Err(err) = outcome {
                let _ = writeln!(stderr, "history: {path}: cannot open: {err}");
                return Ok(1);
            }
        }
        HistoryMode::List => {
            let base = shell.base;
            let entries = shell.entries.clone();
            let start = match count {
                Some(n) => entries.len().saturating_sub(n),
                None => 0,
            };
            for (index, entry) in entries.iter().enumerate().skip(start) {
                let _ = writeln!(stdout, "{:>5}  {}", base + index, entry);
            }
        }
    }

    Ok(0)
}

use crate::executor::ExecuteError as ExecuteErrorAlias;
