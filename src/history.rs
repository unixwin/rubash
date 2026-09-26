//! Session command history (the in-memory history list) plus the
//! host-injected command history contracts.
//!
//! GNU Bash source ownership:
//! - lib/readline/history.c: the list (entries + history_base), stifling
//! - bashhist.c: check_history_control, history_should_ignore,
//!   expand_histignore_pattern, load_history (HISTFILE read)
//! - builtins/history.def: file I/O entry points (-a -w -r -n)

use std::cell::RefCell;
use std::fmt::Debug;
use std::fs;
use std::io;
use std::rc::Rc;

use crate::history_expand::{
    history_expand, HistCtx, HistEngineState, HistExpandResult, HistLookup,
};

/// A host-owned command history used by Rubash's history-facing builtins.
pub trait HistoryProvider: Debug {
    /// Return commands in oldest-to-newest order.
    fn entries(&mut self) -> io::Result<Vec<String>>;
    /// Remove all commands from the host history.
    fn clear(&mut self) -> io::Result<()>;
    /// Append one command to the host history.
    fn append(&mut self, command: String) -> io::Result<()>;
    /// Replace all commands while preserving the host storage implementation.
    fn replace(&mut self, entries: Vec<String>) -> io::Result<()>;
    /// Write all current history entries to the given file (truncate if exists).
    fn write_history(&mut self, path: &str) -> io::Result<()>;
    /// Read the given file and replace history with its contents.
    fn read_history(&mut self, path: &str) -> io::Result<()>;
    /// Append all current history entries to the given file.
    fn append_history(&mut self, path: &str) -> io::Result<()>;
    /// Read lines from the given file that are not already in history.
    fn read_new_history(&mut self, path: &str) -> io::Result<()>;
}

/// Shared provider handle suitable for injecting into an Executor.
pub type SharedHistoryProvider = Rc<RefCell<dyn HistoryProvider>>;

/// The shell's own history list for one session. Mirrors history.c's list
/// plus the bash-level bookkeeping: history_base, lines added this session,
/// the last-line-added flag that "history -s" consults, and the expansion
/// engine state that persists across expansions.
#[derive(Debug, Default, Clone)]
pub struct SessionHistory {
    /// Oldest-to-newest entry lines. Entry N (1-based) lives at
    /// entries[N - base].
    pub entries: Vec<String>,
    /// Parallel to `entries`: the `#<unix-seconds>` stamp GNU stores on every
    /// add_history (readline/history.c hist_inittime). Empty string when the
    /// entry carries none (loaded from a file without timestamps). Written to
    /// the history file only when history_write_timestamps is set, i.e. when
    /// HISTTIMEFORMAT exists as a variable (variables.c sv_histtimefmt).
    pub timestamps: Vec<String>,
    /// history_base: the list number of entries[0].
    pub base: usize,
    /// history_lines_this_session: lines added via the recording path.
    pub lines_this_session: usize,
    /// hist_last_line_added: the line currently executing added an entry.
    pub last_line_added: bool,
    /// hist_last_line_pushed: a `history -s` already pushed an entry for
    /// the command line currently executing (bashhist.c); once set, later
    /// `-s`/`-p` calls in the same line skip the current-line pop.
    pub last_line_pushed: bool,
    /// The HISTFILE load triggered by the first "set -o history" ran.
    pub histfile_loaded: bool,
    /// Entries already flushed by "history -a".
    pub entries_written: usize,
    /// Expansion state (search string, last substitution) that survives
    /// across expansions, like the histexpand.c statics.
    pub engine: HistEngineState,
}

impl SessionHistory {
    pub fn new() -> Self {
        Self {
            base: 1,
            ..Default::default()
        }
    }

    /// Expand one line with this session's list and engine state
    /// (histexpand.c history_expand).
    pub fn expand(&mut self, line: &str, ctx: HistCtx) -> HistExpandResult {
        let snapshot = HistSnapshot {
            base: self.base,
            lines: self.entries.clone(),
        };
        history_expand(line, &snapshot, &mut self.engine, ctx)
    }

    /// bashhist.c check_history_control + history_should_ignore, then
    /// add_history with HISTSIZE stifling. Returns true when recorded.
    pub fn record(
        &mut self,
        line: &str,
        histcontrol: &str,
        histignore: &str,
        histsize: Option<usize>,
    ) -> bool {
        if line.trim().is_empty() {
            return false;
        }
        if history_should_ignore(line, histignore, self.entries.last().map(String::as_str)) {
            return false;
        }
        let mut control = 0u8;
        for part in histcontrol.split(':') {
            match part.trim() {
                "ignorespace" => control |= 1,
                "ignoredups" => control |= 2,
                "ignoreboth" => control |= 3,
                "erasedups" => control |= 4,
                _ => {}
            }
        }
        if control & 1 != 0 && (line.starts_with(' ') || line.starts_with('\t')) {
            return false;
        }
        if control & 2 != 0 && self.entries.last().map(String::as_str) == Some(line) {
            return false;
        }
        if control & 4 != 0 {
            // erasedups removes every matching entry; keep timestamps aligned.
            let mut kept_ts: Vec<String> = Vec::with_capacity(self.timestamps.len());
            let mut kept: Vec<String> = Vec::with_capacity(self.entries.len());
            for (i, e) in self.entries.iter().enumerate() {
                if e != line {
                    kept.push(e.clone());
                    kept_ts.push(self.timestamps.get(i).cloned().unwrap_or_default());
                }
            }
            self.entries = kept;
            self.timestamps = kept_ts;
        }
        self.entries.push(line.to_string());
        // readline/history.c:322-333 hist_inittime: every add_history stores
        // "#<unix-seconds>" on the entry regardless of whether timestamps are
        // later written to the file.
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.timestamps.push(format!("#{epoch}"));
        self.stifle(histsize);
        self.lines_this_session += 1;
        true
    }

    /// history.c stifle_history: drop oldest entries beyond HISTSIZE while
    /// keeping the numbering continuous (history_base advances). `None`
    /// is the unstifled state (GNU sv_histsize unstifles when HISTSIZE is
    /// unset, empty, negative, or non-numeric); `Some(0)` empties the list.
    pub fn stifle(&mut self, histsize: Option<usize>) {
        let Some(histsize) = histsize else {
            return;
        };
        while self.entries.len() > histsize {
            self.entries.remove(0);
            if !self.timestamps.is_empty() {
                self.timestamps.remove(0);
            }
            self.base += 1;
        }
    }

    /// variables.c sv_histsize: map a HISTSIZE/HISTFILESIZE value to the
    /// stifle/truncate limit. Unset, empty, negative, or non-numeric values
    /// mean "no limit" (unstifle / no file truncation).
    pub fn size_limit(value: Option<&str>) -> Option<usize> {
        let value = value?;
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        match value.parse::<i64>() {
            Ok(n) if n >= 0 => Some(n as usize),
            _ => None,
        }
    }

    /// builtins/history.def -c: clear the list and reset the base and the
    /// session line counter.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.timestamps.clear();
        self.base = 1;
        self.lines_this_session = 0;
        self.last_line_added = false;
        self.last_line_pushed = false;
        self.entries_written = 0;
    }

    /// bashhist.c load_history: append HISTFILE lines to the list.
    /// lib/readline/histfile.c:267-476 read_history_range: when the file
    /// has timestamps (lines starting with #digit), blank lines are not
    /// skipped (they may be part of multi-line entries), and lines after
    /// a timestamp that are not themselves timestamps are appended to
    /// the previous entry when the file has multi-line entries.
    pub fn load_file(&mut self, path: &str, histsize: Option<usize>) -> io::Result<usize> {
        let content = fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().collect();
        // histfile.c:377-381: detect timestamps (# followed by digit).
        let has_timestamps = lines.first().is_some_and(|l| {
            l.starts_with('#') && l[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
        });
        // histfile.c:387: default_skipblanks = 0 when multiline entries.
        let has_multiline = has_timestamps;
        let default_skipblanks = !has_multiline;
        let mut skipblanks = default_skipblanks;
        let mut last_ts: Option<String> = None;
        let mut count = 0usize;
        for line in &lines {
            let is_timestamp = line.starts_with('#')
                && line[1..].chars().next().is_some_and(|c| c.is_ascii_digit());
            if is_timestamp {
                // histfile.c:448-453: save timestamp, skip leading blanks.
                last_ts = Some(line.to_string());
                skipblanks = true;
                continue;
            }
            let is_blank = line.is_empty();
            if is_blank && skipblanks {
                // histfile.c:426: skip blank lines when skipblanks is set.
                continue;
            }
            // histfile.c:435: reset skipblanks to default.
            skipblanks = default_skipblanks;
            if last_ts.is_none() && has_multiline && !self.entries.is_empty() {
                // histfile.c:436-437: append to previous entry (multiline).
                let last = self.entries.last_mut().unwrap();
                last.push('\n');
                last.push_str(line);
            } else {
                // histfile.c:439-453: add as new entry carrying the saved
                // timestamp (add_history_time).
                self.entries.push(line.to_string());
                self.timestamps.push(last_ts.clone().unwrap_or_default());
                self.stifle(histsize);
                count += 1;
            }
            last_ts = None;
        }
        Ok(count)
    }

    /// builtins/history.def -w / histfile.c history_write: write every entry
    /// (truncate). When write_timestamps is set each entry is preceded by its
    /// `#<epoch>` line (histfile.c:738-739 history_write_slow).
    pub fn write_file(&self, path: &str, write_timestamps: bool) -> io::Result<usize> {
        let mut out = String::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if write_timestamps {
                if let Some(ts) = self.timestamps.get(i).filter(|ts| !ts.is_empty()) {
                    out.push_str(ts);
                    out.push('\n');
                }
            }
            out.push_str(entry);
            out.push('\n');
        }
        fs::write(path, out)?;
        Ok(self.entries.len())
    }

    /// builtins/history.def -a: append the last history_lines_this_session
    /// entries (the ones added since the session baseline) and reset the
    /// session counter on success.
    pub fn append_file(&mut self, path: &str, write_timestamps: bool) -> io::Result<usize> {
        use std::io::Write;
        let n = self.lines_this_session.min(self.entries.len());
        let start = self.entries.len() - n;
        if n > 0 {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            for i in start..self.entries.len() {
                if write_timestamps {
                    if let Some(ts) = self.timestamps.get(i).filter(|ts| !ts.is_empty()) {
                        writeln!(file, "{ts}")?;
                    }
                }
                writeln!(file, "{}", self.entries[i])?;
            }
        }
        self.lines_this_session = 0;
        Ok(n)
    }

    /// histfile.c:540-668 history_truncate_file: keep only the last `lines`
    /// command lines of the file. When timestamps are being written, a
    /// command's `#<epoch>` line travels with it and does not count toward
    /// the limit (`lines += history_write_timestamps` plus the
    /// HIST_TIMESTAMP_START backtrack at lines 633-657).
    pub fn truncate_file(
        &self,
        path: &str,
        lines: usize,
        write_timestamps: bool,
    ) -> io::Result<()> {
        let Ok(content) = fs::read_to_string(path) else {
            return Ok(());
        };
        if lines == 0 {
            fs::write(path, "")?;
            return Ok(());
        }
        let file_lines: Vec<&str> = content.lines().collect();
        let is_ts = |line: &str| {
            write_timestamps
                && line.starts_with('#')
                && line[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
        };
        let command_lines: Vec<usize> = file_lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !is_ts(line))
            .map(|(i, _)| i)
            .collect();
        if command_lines.len() <= lines {
            return Ok(());
        }
        let first_kept = command_lines[command_lines.len() - lines];
        // Back up over the kept command's timestamp line.
        let start = if first_kept > 0 && is_ts(file_lines[first_kept - 1]) {
            first_kept - 1
        } else {
            first_kept
        };
        // Recompute the byte offset of the kept tail inside the original
        // text so the file's exact bytes (including any trailing newlines)
        // are preserved.
        let mut offset = 0usize;
        for (i, line) in file_lines.iter().enumerate() {
            if i == start {
                break;
            }
            offset += line.len() + 1;
        }
        fs::write(path, &content[offset..])?;
        Ok(())
    }

    /// builtins/history.def -r: append file lines to the list.
    pub fn read_file(&mut self, path: &str, histsize: Option<usize>) -> io::Result<usize> {
        self.load_file(path, histsize)
    }

    /// builtins/history.def -n: append file lines not already in the list.
    pub fn read_new_file(&mut self, path: &str, histsize: Option<usize>) -> io::Result<usize> {
        let content = fs::read_to_string(path)?;
        // Same #<digit> detection as load_file (histfile.c:377-381).
        let has_timestamps = content.lines().next().is_some_and(|l| {
            l.starts_with('#') && l[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
        });
        let mut count = 0usize;
        let mut last_ts: Option<String> = None;
        for line in content.lines() {
            if has_timestamps
                && line.starts_with('#')
                && line[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
            {
                last_ts = Some(line.to_string());
                continue;
            }
            if self.entries.iter().any(|e| e == line) {
                last_ts = None;
                continue;
            }
            self.entries.push(line.to_string());
            self.timestamps.push(last_ts.clone().unwrap_or_default());
            self.stifle(histsize);
            last_ts = None;
            count += 1;
        }
        Ok(count)
    }
}

/// A point-in-time view of the list, satisfying the engine's HistLookup
/// (keeps the expand() borrow split clean).
struct HistSnapshot {
    base: usize,
    lines: Vec<String>,
}

impl HistLookup for HistSnapshot {
    fn base(&self) -> usize {
        self.base
    }

    fn length(&self) -> usize {
        self.lines.len()
    }

    fn get(&self, offset: usize) -> Option<&str> {
        if offset >= self.base {
            self.lines.get(offset - self.base).map(String::as_str)
        } else {
            None
        }
    }
}

/// bashhist.c history_should_ignore: HISTIGNORE patterns are fnmatched
/// against the whole line; a literal ampersand in a pattern expands to the
/// previous entry before matching.
fn history_should_ignore(line: &str, histignore: &str, previous: Option<&str>) -> bool {
    if histignore.is_empty() {
        return false;
    }
    for pattern in histignore.split(':') {
        let mut pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            continue;
        }
        if pattern.contains('&') {
            let Some(previous) = previous else { continue };
            pattern = pattern.replace('&', previous);
        }
        if fnmatch(&pattern, line) {
            return true;
        }
    }
    false
}

/// A small full-string glob matcher (fnmatch without FNM_PATHNAME): star,
/// question mark, and bracket classes.
fn fnmatch(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fnmatch_at(&p, 0, &t, 0)
}

fn fnmatch_at(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
    while pi < p.len() {
        match p[pi] {
            '*' => {
                // collapse consecutive stars
                while pi < p.len() && p[pi] == '*' {
                    pi += 1;
                }
                if pi == p.len() {
                    return true;
                }
                for skip in ti..=t.len() {
                    if fnmatch_at(p, pi, t, skip) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= t.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            '[' => {
                if ti >= t.len() {
                    return false;
                }
                let mut j = pi + 1;
                let mut negate = false;
                if j < p.len() && (p[j] == '!' || p[j] == '^') {
                    negate = true;
                    j += 1;
                }
                let mut matched = false;
                let mut first = true;
                while j < p.len() && (p[j] != ']' || first) {
                    first = false;
                    if j + 2 < p.len() && p[j + 1] == '-' && p[j + 2] != ']' {
                        if t[ti] >= p[j] && t[ti] <= p[j + 2] {
                            matched = true;
                        }
                        j += 3;
                    } else {
                        if t[ti] == p[j] {
                            matched = true;
                        }
                        j += 1;
                    }
                }
                if j >= p.len() {
                    // unterminated class: treat the bracket literally
                    if t[ti] != '[' {
                        return false;
                    }
                    pi += 1;
                    ti += 1;
                    continue;
                }
                if matched == negate {
                    return false;
                }
                pi = j + 1;
                ti += 1;
            }
            c => {
                if ti >= t.len() || t[ti] != c {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == t.len()
}
