//! The `read -e` line editor (rubash#2339).
//!
//! GNU ownership: builtins/read.def read_builtin — `-e` sets `edit`
//! (read.def:323; `-E` sets it too, with bash's completion function,
//! read.def:328), and when the read descriptor is a terminal the raw
//! scanner is replaced by edit_line (read.def:694-710): readline reads
//! the line with the current keymap, the `-i` text pre-inserted by the
//! startup hook (read.def:1282 rl_insert_text via set_itext), the `-p`
//! prompt as its prompt, rl_num_chars_to_read armed by `-n`
//! (read.def:577-578) and the `-d` character installed as the
//! end-of-line delimiter (read.def:583 set_eol_delim). On a
//! non-terminal descriptor all of `-p`/`-e`/`-s`/`-i` are cleared
//! (read.def:489-497) — the executor's gate reproduces that; this
//! module is only ever entered for a real terminal.
//!
//! The engine has no in-process editor host on this face (niubash's
//! reedline serves its own prompts; the standalone engine serves
//! `read -e` itself), so the editor lives here: a pure character-fed
//! state machine ([`LineEditor`]) implementing the emacs keymap's
//! editing semantics (lib/readline/text.c, kill.c, misc.c), plus the
//! terminal glue (termios on Unix via [`TtyRawGuard`], the raw console
//! on Windows via [`crate::console_readline::RawConsole`]) in the
//! executor's read path.
//!
//! Deliberate scope limits (host/editor backlog, not silent gaps):
//! TAB completion is inert (the engine has no completion session to
//! offer a `read` builtin; GNU would run filename completion), C-r
//! incremental search and undo (C-_, C-x C-u) are inert, and the
//! yank-pop cycle covers the session kill ring only as far as
//! consecutive-kill merging goes (kill.c: forward kills append,
//! backward kills prepend to the newest entry).

/// How one editing session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditEnd {
    /// Enter, the `-d` delimiter character, or the `-n` character
    /// limit accepted the line; the payload is the edited text (the
    /// delimiter itself is never part of it).
    Accepted(String),
    /// ^D on an empty buffer, or the input stream ended with nothing
    /// in the buffer: readline returns NULL and the builtin fails
    /// (read.def:705-710 eof path).
    Eof,
    /// The input stream ended with text in the buffer: that text is
    /// the (unterminated) record (read.def:949 partial-record rule).
    EofPartial(String),
}

/// What feeding one character did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feed {
    /// Editing continues.
    Continue,
    /// The display needs a full clear before the next repaint (C-l,
    /// rl_clear_screen).
    ClearScreen,
    /// The session ended.
    End(EditEnd),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Esc {
    None,
    /// A bare ESC arrived: the next character is a meta command or a
    /// sequence introducer.
    Meta,
    /// Inside `\e[` …: parameter bytes until a final byte (0x40-0x7E).
    Csi,
    /// Inside `\eO`: the next character is the final byte.
    Ss3,
    /// Inside `\C-x`: the next character is a ctlx-map command.
    Ctlx,
    /// C-v quoted-insert: the next character is literal.
    QuotedInsert,
}

/// The kill/yank bookkeeping (readline/kill.c): consecutive kills
/// merge into the newest ring entry — forward kills append, backward
/// kills prepend — and yank/yank-pop track the span they inserted so
/// yank-pop can replace it with the next older entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
    Other,
    Kill,
    Yank { ring_index: usize },
}

/// The pure editor state machine. Character offsets are byte offsets
/// kept on char boundaries, the [`crate::script_driver`] EditState
/// convention.
pub struct LineEditor {
    buffer: String,
    cursor: usize,
    esc: Esc,
    csi_params: String,
    history: Vec<String>,
    history_index: Option<usize>,
    saved_line: String,
    last_action: LastAction,
    yank_start: usize,
    yank_len: usize,
    /// Chars consumed so far, for the `-n` limit
    /// (rl_num_chars_to_read counts input characters).
    chars_read: usize,
    max_chars: Option<usize>,
    eol_delim: Option<char>,
}

impl LineEditor {
    /// A fresh editor. `initial_text` is the `-i` text (read.def:334
    /// itext), pre-inserted with the cursor after it — readline's
    /// startup hook inserts it into an empty buffer.
    /// `history` is the session history snapshot (newest last).
    pub fn new(
        initial_text: &str,
        history: Vec<String>,
        max_chars: Option<usize>,
        eol_delim: Option<char>,
    ) -> LineEditor {
        LineEditor {
            buffer: initial_text.to_string(),
            cursor: initial_text.len(),
            esc: Esc::None,
            csi_params: String::new(),
            history,
            history_index: None,
            saved_line: String::new(),
            last_action: LastAction::Other,
            yank_start: 0,
            yank_len: 0,
            chars_read: 0,
            max_chars: max_chars.filter(|&n| n > 0),
            eol_delim: eol_delim.filter(|&d| d != '\n' && d != '\0'),
        }
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn feed_end(&mut self, end: EditEnd) -> Feed {
        Feed::End(end)
    }

    fn after_insert_check(&mut self) -> Option<Feed> {
        if let Some(max) = self.max_chars {
            if self.chars_read >= max {
                let line = std::mem::take(&mut self.buffer);
                return Some(self.feed_end(EditEnd::Accepted(line)));
            }
        }
        None
    }

    /// Feed one input character. `kill_ring` is the process kill ring
    /// (newest entry last), shared with the caller so it survives
    /// across `read` invocations like readline's.
    pub fn feed(&mut self, c: char, kill_ring: &mut Vec<String>) -> Feed {
        self.chars_read += 1;
        let out = self.dispatch(c, kill_ring);
        if !matches!(out, Feed::End(_)) {
            if let Some(end) = self.after_insert_check() {
                return end;
            }
        }
        out
    }

    fn dispatch(&mut self, c: char, kill_ring: &mut Vec<String>) -> Feed {
        match self.esc {
            Esc::QuotedInsert => {
                self.esc = Esc::None;
                self.insert_char(c);
                return Feed::Continue;
            }
            Esc::Meta => {
                self.esc = Esc::None;
                return self.meta_dispatch(c, kill_ring);
            }
            Esc::Csi => {
                if ('\x40'..='\x7e').contains(&c) {
                    self.esc = Esc::None;
                    let params = std::mem::take(&mut self.csi_params);
                    self.csi_dispatch(&params, c);
                } else {
                    self.csi_params.push(c);
                }
                return Feed::Continue;
            }
            Esc::Ss3 => {
                self.esc = Esc::None;
                self.ss3_dispatch(c);
                return Feed::Continue;
            }
            Esc::Ctlx => {
                // The ctlx map's editing payload (undo, macros) is
                // outside this editor's scope; every ctlx key is inert
                // here except that it is consumed as one command.
                self.esc = Esc::None;
                self.last_action = LastAction::Other;
                return Feed::Continue;
            }
            Esc::None => {}
        }

        // The -d delimiter character ends the line wherever it is
        // typed (set_eol_delim, read.def:583); it is not inserted.
        if Some(c) == self.eol_delim {
            let line = std::mem::take(&mut self.buffer);
            return self.feed_end(EditEnd::Accepted(line));
        }

        match c {
            '\n' | '\r' => {
                let line = std::mem::take(&mut self.buffer);
                self.feed_end(EditEnd::Accepted(line))
            }
            '\x01' => {
                self.cursor = 0;
                self.note_other();
                Feed::Continue
            }
            '\x05' => {
                self.cursor = self.buffer.len();
                self.note_other();
                Feed::Continue
            }
            '\x02' => {
                self.cursor = prev_char_boundary(&self.buffer, self.cursor);
                self.note_other();
                Feed::Continue
            }
            '\x06' => {
                self.cursor = next_char_boundary(&self.buffer, self.cursor);
                self.note_other();
                Feed::Continue
            }
            '\x08' | '\x7f' => {
                // rl_rubout (C-h in the emacs map is backward-delete-char;
                // DEL likewise).
                self.rubout();
                Feed::Continue
            }
            '\x04' => {
                // rl_delete: EOF only when the buffer is empty;
                // otherwise delete the char at the point (at the end of
                // a non-empty buffer it does nothing).
                if self.buffer.is_empty() {
                    self.feed_end(EditEnd::Eof)
                } else {
                    self.delete_char_at_cursor();
                    Feed::Continue
                }
            }
            '\x0b' => {
                // rl_kill_line: kill [point, end).
                let killed: String = self.buffer.drain(self.cursor..).collect();
                self.kill(kill_ring, killed, true);
                Feed::Continue
            }
            '\x15' => {
                // rl_unix_line_discard: kill [0, point).
                let killed: String = self.buffer.drain(..self.cursor).collect();
                self.cursor = 0;
                self.kill(kill_ring, killed, false);
                Feed::Continue
            }
            '\x17' => {
                // rl_unix_word_rubout: kill the whitespace-delimited
                // word before the point (plus any blanks immediately
                // before it, the tty WERASE shape).
                let end = self.cursor;
                let mut start = end;
                let bytes = self.buffer.as_bytes();
                while start > 0 && bytes[start - 1].is_ascii_whitespace() {
                    start -= 1;
                }
                while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
                    start -= 1;
                }
                let killed: String = self.buffer.drain(start..end).collect();
                self.cursor = start;
                self.kill(kill_ring, killed, false);
                Feed::Continue
            }
            '\x14' => {
                self.transpose_chars();
                Feed::Continue
            }
            '\x19' => {
                self.yank(kill_ring);
                Feed::Continue
            }
            '\x10' => {
                self.history_previous();
                Feed::Continue
            }
            '\x0e' => {
                self.history_next();
                Feed::Continue
            }
            '\x0f' => {
                // rl_operate_and_get_next accepts the line; the
                // get-next preload is a prompt-session concern, not a
                // read builtin's.
                let line = std::mem::take(&mut self.buffer);
                self.feed_end(EditEnd::Accepted(line))
            }
            '\x0c' => {
                self.note_other();
                Feed::ClearScreen
            }
            '\x16' => {
                self.esc = Esc::QuotedInsert;
                Feed::Continue
            }
            '\x18' => {
                self.esc = Esc::Ctlx;
                Feed::Continue
            }
            '\x1b' => {
                self.esc = Esc::Meta;
                Feed::Continue
            }
            '\t' => {
                // rl_complete: inert in the engine editor (no
                // completion session behind a read builtin).
                self.note_other();
                Feed::Continue
            }
            c if c >= ' ' => {
                self.insert_char(c);
                Feed::Continue
            }
            _ => {
                // Remaining control characters (C-c reaches us only on
                // the Windows console path — Unix delivers it as
                // SIGINT; C-g abort, C-z, C-q/C-s flow control, ...)
                // are inert here, matching the standalone REPL
                // dispatcher's treatment of unbound controls.
                self.note_other();
                Feed::Continue
            }
        }
    }

    fn meta_dispatch(&mut self, c: char, kill_ring: &mut Vec<String>) -> Feed {
        match c {
            '[' => {
                self.esc = Esc::Csi;
                self.csi_params.clear();
            }
            'O' => {
                self.esc = Esc::Ss3;
            }
            'b' => {
                self.cursor = word_start_before(&self.buffer, self.cursor);
                self.note_other();
            }
            'f' => {
                self.cursor = word_end_after(&self.buffer, self.cursor);
                self.note_other();
            }
            'd' => {
                let start = self.cursor;
                let end = word_end_after(&self.buffer, self.cursor);
                let killed: String = self.buffer.drain(start..end).collect();
                self.kill(kill_ring, killed, true);
            }
            '\x7f' | '\x08' => {
                // backward-kill-word
                let end = self.cursor;
                let start = word_start_before(&self.buffer, self.cursor);
                let killed: String = self.buffer.drain(start..end).collect();
                self.cursor = start;
                self.kill(kill_ring, killed, false);
            }
            'u' => {
                self.change_word_case(true, false);
            }
            'l' => {
                self.change_word_case(false, false);
            }
            'c' => {
                self.change_word_case(true, true);
            }
            't' => {
                self.transpose_words();
            }
            'y' => {
                self.yank_pop(kill_ring);
            }
            '<' => {
                if !self.history.is_empty() {
                    if self.history_index.is_none() {
                        self.saved_line = self.buffer.clone();
                    }
                    self.history_index = Some(0);
                    self.buffer = self.history[0].clone();
                    self.cursor = self.buffer.len();
                }
                self.note_other();
            }
            '>' => {
                if self.history_index.is_some() {
                    self.history_index = None;
                    self.buffer = self.saved_line.clone();
                    self.cursor = self.buffer.len();
                }
                self.note_other();
            }
            _ => {
                // ESC ESC is rl_complete (inert, like TAB); digits would
                // open a numeric argument; the rest of the meta map is
                // inert in this editor.
                self.note_other();
            }
        }
        Feed::Continue
    }

    fn csi_dispatch(&mut self, params: &str, final_byte: char) {
        self.note_other();
        // Modifier parameter: `1;5` = Ctrl, `1;3` = Alt (word motion);
        // bare / `1;2` etc. keep the plain meaning for this editor.
        let modifier = if params.contains(';') {
            params.rsplit(';').next().unwrap_or("")
        } else {
            ""
        };
        let word = modifier == "5" || modifier == "3";
        match final_byte {
            'A' => self.history_previous(),
            'B' => self.history_next(),
            'C' => {
                if word {
                    self.cursor = word_end_after(&self.buffer, self.cursor);
                } else {
                    self.cursor = next_char_boundary(&self.buffer, self.cursor);
                }
            }
            'D' => {
                if word {
                    self.cursor = word_start_before(&self.buffer, self.cursor);
                } else {
                    self.cursor = prev_char_boundary(&self.buffer, self.cursor);
                }
            }
            'H' => self.cursor = 0,
            'F' => self.cursor = self.buffer.len(),
            '~' => match params.split(';').next().unwrap_or("") {
                "1" | "7" => self.cursor = 0,
                "4" | "8" => self.cursor = self.buffer.len(),
                "3" => {
                    if word {
                        // \e[3;5~ is rl_kill_word in the initialized map.
                        let start = self.cursor;
                        let end = word_end_after(&self.buffer, self.cursor);
                        self.buffer.drain(start..end);
                    } else {
                        self.delete_char_at_cursor();
                    }
                }
                "5" => {
                    // PageUp: beginning-of-history in the emacs map.
                    if !self.history.is_empty() {
                        if self.history_index.is_none() {
                            self.saved_line = self.buffer.clone();
                        }
                        self.history_index = Some(0);
                        self.buffer = self.history[0].clone();
                        self.cursor = self.buffer.len();
                    }
                }
                "6" => {
                    if self.history_index.is_some() {
                        self.history_index = None;
                        self.buffer = self.saved_line.clone();
                        self.cursor = self.buffer.len();
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn ss3_dispatch(&mut self, c: char) {
        self.note_other();
        match c {
            'A' => self.history_previous(),
            'B' => self.history_next(),
            'C' => self.cursor = next_char_boundary(&self.buffer, self.cursor),
            'D' => self.cursor = prev_char_boundary(&self.buffer, self.cursor),
            'H' => self.cursor = 0,
            'F' => self.cursor = self.buffer.len(),
            _ => {}
        }
    }

    fn note_other(&mut self) {
        self.last_action = LastAction::Other;
    }

    fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.note_other();
    }

    fn rubout(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = prev_char_boundary(&self.buffer, self.cursor);
        self.buffer.drain(start..self.cursor);
        self.cursor = start;
        self.note_other();
    }

    fn delete_char_at_cursor(&mut self) {
        if self.cursor < self.buffer.len() {
            let end = next_char_boundary(&self.buffer, self.cursor);
            self.buffer.drain(self.cursor..end);
        }
        self.note_other();
    }

    /// Record one kill (kill.c): an empty kill touches nothing; a kill
    /// immediately following another kill merges into the newest ring
    /// entry (appended for a forward kill, prepended for a backward
    /// one); anything else pushes a new entry.
    fn kill(&mut self, ring: &mut Vec<String>, killed: String, forward: bool) {
        if killed.is_empty() {
            self.last_action = LastAction::Kill;
            return;
        }
        if self.last_action == LastAction::Kill {
            if let Some(top) = ring.last_mut() {
                if forward {
                    top.push_str(&killed);
                } else {
                    top.insert_str(0, &killed);
                }
                self.last_action = LastAction::Kill;
                return;
            }
        }
        ring.push(killed);
        if ring.len() > 60 {
            // rl_kill_ring_max is unbounded by default in GNU; the cap
            // only keeps a pathological session bounded — 60 is far
            // past anything the read face can observe.
            ring.remove(0);
        }
        self.last_action = LastAction::Kill;
    }

    fn yank(&mut self, ring: &[String]) {
        let Some(top) = ring.last() else {
            self.note_other();
            return;
        };
        let text = top.clone();
        self.yank_start = self.cursor;
        self.yank_len = text.len();
        self.buffer.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = LastAction::Yank {
            ring_index: ring.len() - 1,
        };
    }

    fn yank_pop(&mut self, ring: &[String]) {
        let LastAction::Yank { ring_index } = self.last_action else {
            return;
        };
        if ring.is_empty() {
            return;
        }
        let next = if ring_index == 0 {
            ring.len() - 1
        } else {
            ring_index - 1
        };
        let text = ring[next].clone();
        self.buffer
            .drain(self.yank_start..self.yank_start + self.yank_len);
        self.buffer.insert_str(self.yank_start, &text);
        self.cursor = self.yank_start + text.len();
        self.yank_len = text.len();
        self.last_action = LastAction::Yank { ring_index: next };
    }

    fn transpose_chars(&mut self) {
        // rl_transpose_chars (text.c): at the end of the line swap the
        // last two chars; in the middle swap the char before the point
        // with the char at it and advance one.
        let len = self.buffer.len();
        if self.cursor == 0 || len < 2 {
            return;
        }
        let (first, second) = if self.cursor >= len {
            let second = prev_char_boundary(&self.buffer, len);
            let first = prev_char_boundary(&self.buffer, second);
            (first, second)
        } else {
            let first = prev_char_boundary(&self.buffer, self.cursor);
            (first, self.cursor)
        };
        let second_end = next_char_boundary(&self.buffer, second);
        let ch_first: String = self.buffer[first..second].to_string();
        let ch_second: String = self.buffer[second..second_end].to_string();
        self.buffer
            .replace_range(first..second_end, &format!("{ch_second}{ch_first}"));
        if self.cursor < len {
            self.cursor = second_end;
        }
        self.note_other();
    }

    fn transpose_words(&mut self) {
        // rl_transpose_words (text.c): swap the word before the point
        // with the word after it; the point ends after both.
        let w1_end = {
            let mut e = self.cursor.min(self.buffer.len());
            while e > 0 && !is_word_char(char_at(&self.buffer, e - 1)) {
                e = prev_char_boundary(&self.buffer, e);
            }
            e
        };
        let w1_start = word_start_before(&self.buffer, w1_end);
        if w1_start == w1_end {
            return;
        }
        let mut w2_start = w1_end;
        while w2_start < self.buffer.len() && !is_word_char(char_at(&self.buffer, w2_start)) {
            w2_start = next_char_boundary(&self.buffer, w2_start);
        }
        if w2_start >= self.buffer.len() {
            return;
        }
        let w2_end = word_end_after(&self.buffer, w2_start);
        let word1 = self.buffer[w1_start..w1_end].to_string();
        let gap = self.buffer[w1_end..w2_start].to_string();
        let word2 = self.buffer[w2_start..w2_end].to_string();
        self.buffer
            .replace_range(w1_start..w2_end, &format!("{word2}{gap}{word1}"));
        self.cursor = w2_end;
        self.note_other();
    }

    fn change_word_case(&mut self, up: bool, capitalize: bool) {
        // rl_upcase_word / rl_downcase_word / rl_capitalize_word
        // (text.c): operate on the word at/after the point; the point
        // ends after the word.
        let start = {
            let mut s = self.cursor;
            while s < self.buffer.len() && !is_word_char(char_at(&self.buffer, s)) {
                s = next_char_boundary(&self.buffer, s);
            }
            s
        };
        let end = word_end_after(&self.buffer, start);
        if start >= end {
            return;
        }
        let word = self.buffer[start..end].to_string();
        let changed: String = if capitalize {
            let mut out = String::with_capacity(word.len());
            for (i, ch) in word.chars().enumerate() {
                if i == 0 {
                    out.extend(ch.to_uppercase());
                } else {
                    out.extend(ch.to_lowercase());
                }
            }
            out
        } else if up {
            word.to_uppercase()
        } else {
            word.to_lowercase()
        };
        self.buffer.replace_range(start..end, &changed);
        self.cursor = start + changed.len();
        self.note_other();
    }

    fn history_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => {
                self.saved_line = self.buffer.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_index = Some(idx);
        self.buffer = self.history[idx].clone();
        self.cursor = self.buffer.len();
        self.note_other();
    }

    fn history_next(&mut self) {
        let Some(idx) = self.history_index else {
            return;
        };
        if idx + 1 < self.history.len() {
            self.history_index = Some(idx + 1);
            self.buffer = self.history[idx + 1].clone();
        } else {
            self.history_index = None;
            self.buffer = self.saved_line.clone();
        }
        self.cursor = self.buffer.len();
        self.note_other();
    }
}

/// readline's word character for the emacs word commands
/// (rl_alphabetic: alphanumeric or `_`).
fn is_word_char(c: Option<char>) -> bool {
    matches!(c, Some(ch) if ch.is_alphanumeric() || ch == '_')
}

fn char_at(text: &str, byte: usize) -> Option<char> {
    text[byte..].chars().next()
}

fn prev_char_boundary(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    if i == 0 {
        return 0;
    }
    i -= 1;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    if i >= text.len() {
        return text.len();
    }
    i += 1;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Start of the word ending at or before `at` (rl_backward_word:
/// skip non-word characters back, then word characters back).
fn word_start_before(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    while i > 0 && !is_word_char(char_at(text, prev_char_boundary(text, i))) {
        i = prev_char_boundary(text, i);
    }
    while i > 0 && is_word_char(char_at(text, prev_char_boundary(text, i))) {
        i = prev_char_boundary(text, i);
    }
    i
}

/// End of the word starting at or after `at` (rl_forward_word: skip
/// non-word characters forward, then word characters forward; the
/// point lands at the word's end).
fn word_end_after(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    while i < text.len() && !is_word_char(char_at(text, i)) {
        i = next_char_boundary(text, i);
    }
    while i < text.len() && is_word_char(char_at(text, i)) {
        i = next_char_boundary(text, i);
    }
    i
}

/// RAII termios guard for the Unix editor session: canonical mode,
/// echo and extended processing off (readline's rl_prep_terminal
/// shape); ISIG stays ON so ^C/^Z keep their signal meaning and the
/// executor's SIGINT machinery (not a byte) ends the read. Restores
/// the saved attributes on drop, on every exit path. TCSANOW, never
/// TCSAFLUSH: input the harness or user already typed must not be
/// discarded.
#[cfg(unix)]
pub struct TtyRawGuard {
    fd: std::os::unix::io::RawFd,
    saved: libc::termios,
}

#[cfg(unix)]
impl TtyRawGuard {
    pub fn new(fd: std::os::unix::io::RawFd) -> Option<TtyRawGuard> {
        let mut saved = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return None;
        }
        let mut raw = saved;
        raw.c_iflag &= !(libc::INLCR | libc::ICRNL | libc::IXON);
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        Some(TtyRawGuard { fd, saved })
    }
}

#[cfg(unix)]
impl Drop for TtyRawGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(initial: &str) -> LineEditor {
        LineEditor::new(initial, Vec::new(), None, None)
    }

    /// Feed a whole string; returns the end once the session finishes.
    fn feed_str(ed: &mut LineEditor, ring: &mut Vec<String>, input: &str) -> Option<EditEnd> {
        for c in input.chars() {
            if let Feed::End(end) = ed.feed(c, ring) {
                return Some(end);
            }
        }
        None
    }

    #[test]
    fn beginning_of_line_insert() {
        // Hunt rs03: `hello world` + C-a + typing lands at the front.
        let mut ring = Vec::new();
        let mut ed = editor("");
        assert_eq!(feed_str(&mut ed, &mut ring, "hello world\x01"), None);
        ed.feed('X', &mut ring);
        assert_eq!(ed.buffer(), "Xhello world");
    }

    #[test]
    fn kill_and_yank_round_trip() {
        // Hunt rs07/rs25: C-u kills the line into the ring, C-y yanks
        // it back; text typed after the yank appends.
        let mut ring = Vec::new();
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "junk\x15\x19real");
        assert_eq!(ed.buffer(), "junkreal");
        assert_eq!(ring, vec!["junk".to_string()]);
    }

    #[test]
    fn kill_line_then_yank_restores_tail() {
        // Hunt rs05 shape: C-k on "abc" at the start leaves "" and
        // the ring holds "abc".
        let mut ring = Vec::new();
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "abc\x01\x0b\x19");
        assert_eq!(ed.buffer(), "abc");
    }

    #[test]
    fn meta_word_motions_and_kill() {
        // Hunt rs10/rs11: ESC-b moves to the word start, ESC-d kills
        // the word after the point.
        let mut ring = Vec::new();
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "foo bar\x1bb");
        assert_eq!(ed.cursor(), 4);
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "foo bar\x01\x1bd");
        assert_eq!(ed.buffer(), " bar");
    }

    #[test]
    fn transpose_chars_at_end() {
        // Hunt rs12: "ab" + C-t -> "ba".
        let mut ring = Vec::new();
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "ab\x14");
        assert_eq!(ed.buffer(), "ba");
    }

    #[test]
    fn initial_text_is_preloaded_with_cursor_after_it() {
        // Hunt rs13/rs14: -i text is in the buffer; DELs eat it.
        let mut ring = Vec::new();
        let mut ed = editor("INIT");
        assert_eq!(ed.buffer(), "INIT");
        feed_str(&mut ed, &mut ring, "\x7f\x7fOK");
        assert_eq!(ed.buffer(), "INOK");
    }

    #[test]
    fn ctrl_d_empty_is_eof_nonempty_deletes() {
        // Hunt rs24: C-d on an empty buffer ends the read as EOF;
        // with text it deletes the char at the point.
        let mut ring = Vec::new();
        let mut ed = editor("");
        assert_eq!(ed.feed('\x04', &mut ring), Feed::End(EditEnd::Eof));
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "ab\x01\x04");
        assert_eq!(ed.buffer(), "b");
    }

    #[test]
    fn char_limit_accepts() {
        // Hunt rs19: -n 3 accepts after the third character.
        let mut ring = Vec::new();
        let mut ed = LineEditor::new("", Vec::new(), Some(3), None);
        assert_eq!(
            feed_str(&mut ed, &mut ring, "abcdef"),
            Some(EditEnd::Accepted("abc".to_string()))
        );
    }

    #[test]
    fn eol_delim_accepts_without_inserting() {
        // Hunt rs20: with -d ':', typing ':' ends the line.
        let mut ring = Vec::new();
        let mut ed = LineEditor::new("", Vec::new(), None, Some(':'));
        assert_eq!(
            feed_str(&mut ed, &mut ring, "ab:cd"),
            Some(EditEnd::Accepted("ab".to_string()))
        );
    }

    #[test]
    fn history_recall_and_empty_history() {
        // Hunt rs26: Up recalls session history; with an empty
        // history (read lines are not added) nothing changes.
        let mut ring = Vec::new();
        let mut ed = LineEditor::new("", vec!["one".to_string()], None, None);
        feed_str(&mut ed, &mut ring, "\x10");
        assert_eq!(ed.buffer(), "one");
        feed_str(&mut ed, &mut ring, "\x0e");
        assert_eq!(ed.buffer(), "");
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "\x1b[A");
        assert_eq!(ed.buffer(), "");
    }

    #[test]
    fn arrow_keys_move_and_delete() {
        let mut ring = Vec::new();
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "ab\x1b[D");
        assert_eq!(ed.cursor(), 1);
        feed_str(&mut ed, &mut ring, "\x1b[3~");
        assert_eq!(ed.buffer(), "a");
        feed_str(&mut ed, &mut ring, "\x1bOH");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn yank_pop_cycles_the_ring() {
        let mut ring = vec!["first".to_string(), "second".to_string()];
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "\x19");
        assert_eq!(ed.buffer(), "second");
        feed_str(&mut ed, &mut ring, "\x1by");
        assert_eq!(ed.buffer(), "first");
    }

    #[test]
    fn word_rubout_keeps_the_separator() {
        // GNU rl_unix_word_rubout on "foo bar" leaves "foo ".
        let mut ring = Vec::new();
        let mut ed = editor("");
        feed_str(&mut ed, &mut ring, "foo bar\x17");
        assert_eq!(ed.buffer(), "foo ");
        assert_eq!(ring, vec!["bar".to_string()]);
    }
}
