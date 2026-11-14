//! fc module.
//!
//! GNU Bash source ownership:
//! - builtins/fc.def (fc_builtin, fc_gethnum, fc_list)

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

/// Result of running the fc builtin: either a plain status, or a
/// re-execution request (`fc -s`) whose substituted command the executor
/// must run (fc.def:308 echoes it to stderr, then parse_and_execute runs it).
pub enum FcResult {
    Status(i32),
    Reexec {
        command: String,
    },
    /// fc -e: open the selected entries in an editor, then execute the
    /// edited result (fc.def edit_and_execute_command). `start`/`end`
    /// index the history list AFTER the fc-line entry has been dropped
    /// (fc.def:403-421); `drop_last_entry` tells the executor to perform
    /// that deletion on the live session before selecting.
    EditWith {
        editor: Option<String>,
        start: usize,
        end: usize,
        rev: bool,
        drop_last_entry: bool,
    },
}

enum SpecErr {
    NotFound,
    Erange,
}

/// Returns true if arg looks like a history number (possibly negative),
/// matching bash's fc_number() so that e.g. "-1" is not misread as an option.
fn is_number_arg(arg: &str) -> bool {
    let s = if arg.starts_with('-') && arg.len() > 1 {
        &arg[1..]
    } else {
        arg
    };
    !s.is_empty() && s.parse::<isize>().is_ok()
}

/// Verbatim port of fc.def fc_gethnum: resolve a numeric or prefix-string
/// history specification to a 0-based index. Positive numbers are absolute
/// history numbers (offsets from history_base); negative numbers are offsets
/// from the most recent entry; strings are most-recent prefix matches.
fn fc_gethnum(
    spec: &str,
    entries: &[String],
    history_base: usize,
    last_hist: i64,
    real_last: usize,
    listing: bool,
    hn_first: bool,
) -> Result<usize, SpecErr> {
    let mut s = spec;
    let mut sign: i64 = 1;
    if let Some(rest) = spec.strip_prefix('-') {
        sign = -1;
        s = rest;
    }
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        let mut n: i64 = s.parse::<i64>().unwrap_or(0) * sign;
        if n < 0 {
            // fc.def: "If the value is negative or zero, then it is an
            // offset from the current history item."
            n += last_hist + 1;
            return Ok(if n < 0 { 0 } else { n as usize });
        } else if n == 0 {
            return if sign == -1 {
                if listing {
                    Ok(real_last)
                } else {
                    Err(SpecErr::Erange)
                }
            } else {
                Ok(last_hist as usize)
            };
        }
        let n = n - history_base as i64;
        if n < 0 {
            return Ok(if hn_first { 0 } else { last_hist as usize });
        }
        if n >= last_hist {
            return Ok(if hn_first { 0 } else { last_hist as usize });
        }
        return Ok(n as usize);
    }
    // String prefix search, most recent match first (fc.def STREQN loop).
    let mut j: i64 = last_hist;
    while j >= 0 {
        if entries[j as usize].starts_with(spec) {
            return Ok(j as usize);
        }
        j -= 1;
    }
    Err(SpecErr::NotFound)
}

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<FcResult>
where
    E: Write,
{
    execute_with_history(
        args,
        diagnostic_prefix,
        Vec::new(),
        1,
        false,
        false,
        false,
        &mut io::sink(),
        stderr,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_with_history<E, O>(
    args: &[String],
    diagnostic_prefix: &str,
    mut entries: Vec<String>,
    history_base: usize,
    last_line_added: bool,
    remember_on_history: bool,
    posix_mode: bool,
    stdout: &mut O,
    stderr: &mut E,
) -> io::Result<FcResult>
where
    E: Write,
    O: Write,
{
    let mut index = 0;
    let mut numbering = true;
    let mut reverse = false;
    let mut listing = false;
    let mut editor: Option<String> = None;
    let mut execute = false;

    while let Some(arg) = args.get(index) {
        if arg == "--" {
            // loptend points PAST the "--"; a bare "--" is not a spec.
            index += 1;
            break;
        }
        if arg == "--help" {
            // Historical hand-rolled face, kept for direct callers:
            // through the shell, the dispatch gate (rubash#2753)
            // intercepts `--help` in option position before this
            // parser runs and prints the long help with EX_USAGE,
            // per CASE_HELPOPT (builtins/common.h:38-41). GNU has
            // no `-h` option for fc (fc.def's spec is ":e:lnrs"),
            // so `-h` is deliberately NOT aliased here — it falls
            // into the cluster scan below and reports the invalid
            // option with EX_USAGE, as GNU does.
            write_help(stdout)?;
            return Ok(FcResult::Status(EXECUTION_SUCCESS));
        }
        if !arg.starts_with('-') || arg == "-" || is_number_arg(arg) {
            break;
        }

        for (offset, option) in arg[1..].char_indices() {
            match option {
                'l' => {
                    listing = true;
                }
                'n' => {
                    numbering = false;
                }
                'r' => {
                    reverse = true;
                }
                's' => {
                    execute = true;
                }
                'e' => {
                    let value_start = 1 + offset + option.len_utf8();
                    if value_start < arg.len() {
                        editor = Some(arg[value_start..].to_string());
                        break;
                    }
                    if value_start < arg.len() {
                        break;
                    }
                    index += 1;
                    editor = args.get(index).cloned();
                    if args.get(index).is_none() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}fc: -e: option requires an argument"
                        )?;
                        write_usage(stderr)?;
                        return Ok(FcResult::Status(EX_USAGE));
                    }
                }
                other => {
                    writeln!(stderr, "{diagnostic_prefix}fc: -{other}: invalid option")?;
                    write_usage(stderr)?;
                    return Ok(FcResult::Status(EX_USAGE));
                }
            }
        }
        index += 1;
    }

    // fc.def: "fc -e -" means execute without an editor -- same as fc -s.
    if editor.as_deref() == Some("-") {
        execute = true;
        editor = None;
    }

    let count = entries.len();
    if count == 0 {
        // GNU fc.def splits the empty-history verdict by form: the
        // EXECUTE form (fc -s, and fc -e - which the conversion above
        // already turned into execute=1, fc.def:243-246) resolves its
        // command through fc_gethist (fc.def:280-286), which returns
        // NULL over an empty list — "fc: no command found",
        // EXECUTION_FAILURE (rubash#1123, rubash#1178). The list/edit
        // forms instead check history_list() itself (fc.def:317-319:
        // `if (hlist == 0) return (EXECUTION_SUCCESS)`) and succeed
        // silently, for `fc -l` and the editor path alike.
        if execute {
            writeln!(stderr, "{diagnostic_prefix}fc: no command found")?;
            return Ok(FcResult::Status(EXECUTION_FAILURE));
        }
        return Ok(FcResult::Status(EXECUTION_SUCCESS));
    }

    // fc.def:334-348 (fc_builtin) / fc_gethist: last_hist = i - rh -
    // hist_last_line_added, where rh is remember_on_history (set -o
    // history / interactive). With rh == 0 and the current line not
    // added, last_hist lands one past the end and backs up over the
    // empty tail slot to the last real entry (#1117).
    let mut last_hist: i64 = count as i64 - remember_on_history as i64 - last_line_added as i64;
    if last_hist >= count as i64 {
        last_hist = count as i64 - 1;
    }
    let real_last: usize = count.saturating_sub(1);

    let pos: Vec<&String> = args[index..].iter().collect();

    // fc -s: leading pat=rep arguments are global substitutions; the first
    // argument without '=' is the command specification (fc.def fc -s).
    if execute {
        let mut subs: Vec<(String, String)> = Vec::new();
        let mut spec: Option<&String> = None;
        let mut spec_index: Option<usize> = None;
        for (position, a) in pos.iter().enumerate() {
            if spec.is_none() {
                if let Some(eq) = a.find('=') {
                    if eq > 0 {
                        subs.push((a[..eq].to_string(), a[eq + 1..].to_string()));
                        continue;
                    }
                }
                spec = Some(a);
                spec_index = Some(position);
            }
        }
        // GNU builtins/fc.def (POSIX mode, bash posix-mode notes item 56):
        // operands after the command specification are an error ("fc: too
        // many arguments", EXECUTION_FAILURE) instead of being ignored the
        // way the default mode treats them.
        if posix_mode && spec_index.is_some_and(|index| index + 1 < pos.len()) {
            writeln!(stderr, "{diagnostic_prefix}fc: too many arguments")?;
            return Ok(FcResult::Status(EXECUTION_FAILURE));
        }
        let resolved = match spec {
            None => Ok(last_hist.max(0) as usize),
            Some(s) => fc_gethnum(
                s,
                &entries,
                history_base,
                last_hist,
                real_last,
                false,
                false,
            ),
        };
        let idx = match resolved {
            Ok(i) => i,
            // fc_gethist returns NULL for every negative sentinel
            // (HIST_INVALID / HIST_ERANGE / HIST_NOTFOUND), so fc -s
            // reports "no command found" for ALL resolution failures.
            Err(SpecErr::NotFound) | Err(SpecErr::Erange) => {
                writeln!(stderr, "{diagnostic_prefix}fc: no command found")?;
                return Ok(FcResult::Status(EXECUTION_FAILURE));
            }
        };
        let idx = idx.min(count - 1);
        let mut command = entries[idx].clone();
        for (pat, rep) in &subs {
            command = command.replace(pat.as_str(), rep.as_str());
        }
        // fc.def:308: the substituted command is echoed to stderr.
        writeln!(stderr, "{command}")?;
        return Ok(FcResult::Reexec { command });
    }

    // fc.def:345-360: resolve histbeg/histend from one or two arguments.
    let resolved: Result<(usize, usize), SpecErr> = match (pos.first(), pos.get(1)) {
        (Some(f), Some(l)) => fc_gethnum(
            f,
            &entries,
            history_base,
            last_hist,
            real_last,
            listing,
            true,
        )
        .and_then(|b| {
            fc_gethnum(
                l,
                &entries,
                history_base,
                last_hist,
                real_last,
                listing,
                false,
            )
            .map(|e| (b, e))
        }),
        (Some(f), None) => fc_gethnum(
            f,
            &entries,
            history_base,
            last_hist,
            real_last,
            listing,
            true,
        )
        .map(|b| {
            let e = if b == real_last {
                if listing {
                    real_last
                } else {
                    b
                }
            } else if listing {
                last_hist.max(0) as usize
            } else {
                b
            };
            (b, e)
        }),
        (None, _) => {
            if listing {
                // fc.def: "The default for listing is the last 16 history items."
                let e = last_hist.max(0) as usize;
                let b = (e as i64 - 16 + 1).max(0) as usize;
                Ok((b, e))
            } else {
                // For editing it is the last history command.
                let e = last_hist.max(0) as usize;
                Ok((e, e))
            }
        }
    };
    let (mut histbeg, mut histend) = match resolved {
        Ok(pair) => pair,
        Err(SpecErr::NotFound) => {
            writeln!(stderr, "{diagnostic_prefix}fc: no command found")?;
            return Ok(FcResult::Status(EXECUTION_FAILURE));
        }
        Err(SpecErr::Erange) => {
            writeln!(
                stderr,
                "{diagnostic_prefix}fc: history specification out of range"
            )?;
            return Ok(FcResult::Status(EXECUTION_FAILURE));
        }
    };

    // fc.def:403-421: when not listing and the current line was added,
    // "the fc command that caused the editing shall not be entered into
    // the history list" -- the last entry is deleted BEFORE the edited
    // text is produced, and a range that pointed at the deleted slot
    // backs off to the new last entry (hlist[last_hist] == 0 after the
    // delete). This is why `history -s X; fc -e :` edits an empty file
    // in a non-interactive shell: the -s entry is the deleted slot and
    // the range collapses onto the (empty) remainder (#1117).
    let mut drop_last_entry = false;
    if !listing && last_line_added && !entries.is_empty() {
        entries.pop();
        drop_last_entry = true;
        let mut beg = histbeg as i64;
        let mut end = histend as i64;
        let mut last = last_hist;
        // hlist[last_hist] == 0 once the slot at last_hist was deleted.
        if beg == end && end == last && last >= entries.len() as i64 {
            last -= 1;
            end -= 1;
            beg = end;
        }
        if last >= entries.len() as i64 || last < 0 {
            last -= 1;
        }
        if end >= last {
            end = last;
        } else if beg >= last {
            beg = last;
        }
        histbeg = beg.max(0) as usize;
        histend = end.max(0) as usize;
        last_hist = last;
    }

    // fc.def: an inverted range swaps and forces reverse order.
    let mut rev = reverse;
    if histend < histbeg {
        std::mem::swap(&mut histbeg, &mut histend);
        rev = true;
    }

    // fc.def:476-483: a bare number (i + history_base), then when listing
    // a tab plus the continuation marker ("*" when the entry carries
    // multi-line histdata, else a space); posix mode prints a bare tab.
    let emit = |stdout: &mut O, history_num: usize, entry: &str| -> io::Result<()> {
        if numbering {
            write!(stdout, "{}", history_num)?;
        }
        if listing {
            if posix_mode {
                write!(stdout, "\t")?;
            } else {
                // histdata(i) is only set for interactive continuation
                // entries; script-recorded entries always print a space marker.
                write!(stdout, "\t ")?;
            }
        }
        writeln!(stdout, "{}", entry)
    };

    let indices: Vec<usize> = if rev {
        (histbeg..=histend).rev().collect()
    } else {
        (histbeg..=histend).collect()
    };
    if !listing {
        // fc.def: the edit path opens the range in the editor instead of
        // printing it.
        return Ok(FcResult::EditWith {
            editor,
            start: histbeg,
            end: histend,
            rev,
            drop_last_entry,
        });
    }

    for idx in indices {
        if idx >= count {
            continue; // fc.def: hlist[i] == 0 -> continue
        }
        emit(stdout, idx + history_base, &entries[idx])?;
    }

    Ok(FcResult::Status(EXECUTION_SUCCESS))
}

fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(
        stderr,
        "fc: usage: fc [-e ename] [-lnr] [first] [last] or fc -s [pat=rep] [command]"
    )
}

fn write_help<O>(stdout: &mut O) -> io::Result<()>
where
    O: Write,
{
    writeln!(
        stdout,
        "fc: display or execute commands from the history list"
    )?;
    writeln!(stdout, "")?;
    writeln!(stdout, "Usage: fc [-e ename] [-lnr] [first] [last]")?;
    writeln!(stdout, "       fc -s [pat=rep ...] [command]")?;
    writeln!(stdout, "")?;
    writeln!(stdout, "Display or execute commands from the history list.")?;
    writeln!(stdout, "")?;
    writeln!(stdout, "Options:")?;
    writeln!(stdout, "  -e ename    Select which editor to use.")?;
    writeln!(stdout, "  -l          List lines instead of editing.")?;
    writeln!(stdout, "  -n          Omit line numbers when listing.")?;
    writeln!(stdout, "  -r          Reverse the order of the lines.")?;
    writeln!(
        stdout,
        "  -s          Re-execute command after substitution."
    )?;
    writeln!(stdout, "")?;
    writeln!(stdout, "FIRST and LAST can be numbers or strings.")?;
    writeln!(
        stdout,
        "Negative numbers count back from the most recent command."
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_fc(args: &[&str], entries: &[&str]) -> (String, String, FcResult) {
        // Interactive model: the fc line itself is the last entry and
        // remember_on_history is on.
        run_fc_model(args, entries, true, true)
    }

    fn run_fc_model(
        args: &[&str],
        entries: &[&str],
        last_line_added: bool,
        remember_on_history: bool,
    ) -> (String, String, FcResult) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let entries: Vec<String> = entries.iter().map(|s| s.to_string()).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = execute_with_history(
            &args,
            "",
            entries,
            1,
            last_line_added,
            remember_on_history,
            false,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        (
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
            result,
        )
    }

    fn expect_lines(out: &str, lines: &[&str]) {
        let actual: Vec<&str> = out.lines().collect();
        assert_eq!(actual, lines);
    }

    fn expect_status(result: &FcResult, status: i32) {
        match result {
            FcResult::Status(s) => assert_eq!(*s, status),
            FcResult::Reexec { .. } | FcResult::EditWith { .. } => panic!("expected Status"),
        }
    }

    // --- -n (no numbering) ---

    #[test]
    fn test_fc_no_numbering() {
        let (out, _, result) = run_fc(&["-ln"], &["a", "b", "c", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t a", "\t b", "\t c"]);
    }

    // --- -r (reverse) ---

    #[test]
    fn test_fc_reverse_all() {
        let (out, _, result) = run_fc(&["-lr"], &["a", "b", "c", "d", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["4\t d", "3\t c", "2\t b", "1\t a"]);
    }

    #[test]
    fn test_fc_reverse_with_range() {
        let (out, _, result) = run_fc(&["-lr", "2", "4"], &["a", "b", "c", "d", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["4\t d", "3\t c", "2\t b"]);
    }

    #[test]
    fn test_fc_reverse_no_numbers() {
        let (out, _, result) = run_fc(&["-lnr"], &["a", "b", "c", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t c", "\t b", "\t a"]);
    }

    // --- negative first ---

    #[test]
    fn test_fc_negative_first_minus_one() {
        let (out, _, result) = run_fc(&["-ln", "-1"], &["a", "b", "c", "git status", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t git status"]);
    }

    #[test]
    fn test_fc_negative_first_minus_two() {
        let (out, _, result) = run_fc(&["-ln", "-2"], &["a", "b", "c", "git status", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t c", "\t git status"]);
    }

    #[test]
    fn test_fc_negative_first_clamped_to_zero() {
        let (out, _, result) = run_fc(&["-ln", "-10"], &["a", "b", "c", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t a", "\t b", "\t c"]);
    }

    // --- last argument ---

    #[test]
    fn test_fc_positive_range() {
        let (out, _, result) = run_fc(&["-l", "2", "4"], &["a", "b", "c", "d", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["2\t b", "3\t c", "4\t d"]);
    }

    #[test]
    fn test_fc_negative_first_and_last() {
        let (out, _, result) = run_fc(&["-ln", "-3", "-1"], &["a", "b", "c", "d", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t b", "\t c", "\t d"]);
    }

    #[test]
    fn test_fc_last_clamped() {
        let (out, _, result) = run_fc(&["-l", "1", "100"], &["a", "b", "c", "d", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["1\t a", "2\t b", "3\t c", "4\t d"]);
    }

    #[test]
    fn test_fc_out_of_range_first_clamps_to_start() {
        // fc.def: n - base >= i with HN_FIRST clamps to index 0, and the
        // inverted range swaps to ascending order with reverse forced.
        let (out, _, result) = run_fc(&["-l", "5", "3"], &["a", "b", "c", "d", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["1\t a", "2\t b", "3\t c"]);
    }

    // --- defaults ---

    #[test]
    fn test_fc_default_all_entries() {
        let (out, _, result) = run_fc(&["-l"], &["a", "b", "c", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["1\t a", "2\t b", "3\t c"]);
    }

    #[test]
    fn test_fc_empty_entries() {
        let (out, _, result) = run_fc(&["-l"], &[]);
        expect_status(&result, EXECUTION_SUCCESS);
        assert_eq!(out, "");
    }

    #[test]
    fn test_fc_s_empty_history_no_command_found() {
        // rubash#1123: GNU fc.def's execute form reports "no command
        // found" (rc 1) on an empty history; the list form above stays 0.
        let (_, stderr, result) = run_fc(&["-s"], &[]);
        expect_status(&result, EXECUTION_FAILURE);
        assert_eq!(stderr, "fc: no command found\n");
        let (_, stderr, result) = run_fc(&["-e", "-"], &[]);
        expect_status(&result, EXECUTION_FAILURE);
        assert_eq!(stderr, "fc: no command found\n");
    }

    // --- string specification ---

    #[test]
    fn test_fc_string_spec_not_found() {
        let (_, stderr, result) = run_fc(&["-l", "zzz"], &["a", "b", "fc"]);
        expect_status(&result, EXECUTION_FAILURE);
        assert_eq!(stderr, "fc: no command found\n");
    }

    #[test]
    fn test_fc_string_spec_prefix_match() {
        let (out, _, result) = run_fc(&["-ln", "git"], &["a", "git status", "fc"]);
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t git status"]);
    }

    // --- fc -s (re-execution) ---

    #[test]
    fn test_fc_s_substitution() {
        let (_, stderr, result) = run_fc(&["-s", "a=x"], &["echo aa ab ac", "fc"]);
        match result {
            FcResult::Reexec { command } => assert_eq!(command, "echo xx xb xc"),
            other => panic!(
                "expected Reexec, got {:?}",
                match other {
                    FcResult::Status(s) => s.to_string(),
                    FcResult::Reexec { .. } | FcResult::EditWith { .. } => String::new(),
                }
            ),
        }
        assert_eq!(stderr, "echo xx xb xc\n");
    }

    #[test]
    fn test_fc_execute_form_empty_history() {
        // rubash#1178: the execute form (fc -s / fc -e -) over an empty
        // history list is "no command found" (fc.def fc_gethist NULL);
        // the list and edit forms succeed silently (fc.def: hlist == 0).
        let (_, stderr, result) = run_fc(&["-e", "-", "5"], &[]);
        expect_status(&result, EXECUTION_FAILURE);
        assert_eq!(stderr, "fc: no command found\n");

        let (_, stderr, result) = run_fc(&["-s"], &[]);
        expect_status(&result, EXECUTION_FAILURE);
        assert_eq!(stderr, "fc: no command found\n");

        let (_, _, result) = run_fc(&["5"], &[]);
        expect_status(&result, EXECUTION_SUCCESS);
    }

    #[test]
    fn test_fc_s_no_command_found() {
        let (_, stderr, result) = run_fc(&["-s", "cc"], &["echo aa ab ac", "fc"]);
        expect_status(&result, EXECUTION_FAILURE);
        assert_eq!(stderr, "fc: no command found\n");
    }

    // --- non-interactive model (remember_on_history off) ---

    #[test]
    fn test_fc_list_after_s_pushes_uses_raw_last_hist() {
        // rubash#1117: entries fed only by `history -s` (each push sets
        // hist_last_line_added via really_add_history, bashhist.c:961)
        // with remember_on_history off: last_hist = count - 0 - 1, so
        // -2 covers both pushed entries.
        let (out, _, result) = run_fc_model(
            &["-ln", "-2"],
            &["echo FIRST_CMD", "echo SECOND_CMD"],
            true,
            false,
        );
        expect_status(&result, EXECUTION_SUCCESS);
        expect_lines(&out, &["\t echo FIRST_CMD", "\t echo SECOND_CMD"]);
    }

    #[test]
    fn test_fc_edit_drops_last_pushed_entry() {
        // fc.def:403-421 + fc16 probe: the default edit range points at
        // the last entry; the not-listing deletion removes that entry
        // first, so the range backs off to the previous one and the
        // executor must drop the entry from the live session.
        let (_, _, result) = run_fc_model(&["-e", ":"], &["echo U1", "echo U2"], true, false);
        match result {
            FcResult::EditWith {
                start,
                end,
                drop_last_entry,
                ..
            } => {
                assert_eq!((start, end), (0, 0));
                assert!(drop_last_entry);
            }
            _ => panic!("expected EditWith"),
        }
    }

    #[test]
    fn test_fc_s_substitution_on_pushed_entries() {
        // fc15 probe: `fc -s BBB=CCC` resolves the last pushed entry
        // (last_hist = count - rh - added = 1), not the one before it.
        let (_, _, result) =
            run_fc_model(&["-s", "BBB=CCC"], &["echo AAA", "echo BBB"], true, false);
        match result {
            FcResult::Reexec { command } => assert_eq!(command, "echo CCC"),
            _ => panic!("expected Reexec"),
        }
    }

    // --- --help ---

    #[test]
    fn test_fc_help_flag() {
        let (out, _, result) = run_fc(&["--help"], &[]);
        expect_status(&result, EXECUTION_SUCCESS);
        assert!(out.contains("display or execute commands"));
    }
}
