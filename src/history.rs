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

use crate::history_expand::{history_expand, HistCtx, HistExpandResult, HistEngineState, HistLookup};

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
#[derive(Debug, Default)]
pub struct SessionHistory {
    /// Oldest-to-newest entry lines. Entry N (1-based) lives at
    /// entries[N - base].
    pub entries: Vec<String>,
    /// history_base: the list number of entries[0].
    pub base: usize,
    /// history_lines_this_session: lines added via the recording path.
    pub lines_this_session: usize,
    /// hist_last_line_added: the line currently executing added an entry.
    pub last_line_added: bool,
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
        Self { base: 1, ..Default::default() }
    }

    /// Expand one line with this session's list and engine state
    /// (histexpand.c history_expand).
    pub fn expand(&mut self, line: &str, ctx: HistCtx) -> HistExpandResult {
        let snapshot = HistSnapshot { base: self.base, lines: self.entries.clone() };
        history_expand(line, &snapshot, &mut self.engine, ctx)
    }

    /// bashhist.c check_history_control + history_should_ignore, then
    /// add_history with HISTSIZE stifling. Returns true when recorded.
    pub fn record(&mut self, line: &str, histcontrol: &str, histignore: &str, histsize: usize) -> bool {
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
            self.entries.retain(|e| e != line);
        }
        self.entries.push(line.to_string());
        self.stifle(histsize);
        self.lines_this_session += 1;
        true
    }

    /// history.c stifle_history: drop oldest entries beyond HISTSIZE while
    /// keeping the numbering continuous (history_base advances).
    pub fn stifle(&mut self, histsize: usize) {
        if histsize == 0 {
            return;
        }
        while self.entries.len() > histsize {
            self.entries.remove(0);
            self.base += 1;
        }
    }

    /// builtins/history.def -c: clear the list and reset the base and the
    /// session line counter.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.base = 1;
        self.lines_this_session = 0;
        self.last_line_added = false;
        self.entries_written = 0;
    }

    /// bashhist.c load_history: append HISTFILE lines to the list.
    pub fn load_file(&mut self, path: &str, histsize: usize) -> io::Result<usize> {
        let content = fs::read_to_string(path)?;
        let mut count = 0usize;
        for line in content.lines() {
            self.entries.push(line.to_string());
            self.stifle(histsize);
            count += 1;
        }
        Ok(count)
    }

    /// builtins/history.def -w: write every entry (truncate).
    pub fn write_file(&self, path: &str) -> io::Result<usize> {
        let joined = self.entries.iter().map(|e| e.as_str()).collect::<Vec<_>>().join("\n");
        fs::write(path, joined + "\n")?;
        Ok(self.entries.len())
    }

    /// builtins/history.def -a: append entries added since the last flush.
    pub fn append_file(&mut self, path: &str) -> io::Result<usize> {
        use std::io::Write;
        let start = self.entries_written.min(self.entries.len());
        let pending: Vec<String> = self.entries[start..].to_vec();
        if !pending.is_empty() {
            let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
            for entry in &pending {
                writeln!(file, "{entry}")?;
            }
        }
        self.entries_written = self.entries.len();
        Ok(pending.len())
    }

    /// builtins/history.def -r: append file lines to the list.
    pub fn read_file(&mut self, path: &str, histsize: usize) -> io::Result<usize> {
        self.load_file(path, histsize)
    }

    /// builtins/history.def -n: append file lines not already in the list.
    pub fn read_new_file(&mut self, path: &str, histsize: usize) -> io::Result<usize> {
        let content = fs::read_to_string(path)?;
        let mut count = 0usize;
        for line in content.lines() {
            if self.entries.iter().any(|e| e == line) {
                continue;
            }
            self.entries.push(line.to_string());
            self.stifle(histsize);
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
