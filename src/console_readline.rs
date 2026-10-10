//! Raw console line editing for the standalone interactive REPL (rubash#419
//! follow-up, the readline leg).
//!
//! The engine's interactive reader ([`crate::script_driver::run_interactive_stdin`])
//! already renders the expanded PS1/PS2 and dispatches readline edit keys
//! (C-a/C-e/C-b/C-f/C-k/C-u, history C-p/C-n and the arrow-key CSI
//! sequences, C-r isearch, C-o operate-and-get-next). Under a pipe those
//! keys arrive in the byte stream; on a real Windows console stdin was
//! left in cooked line-input mode, where conhost owns the editing and
//! every arrow key drives conhost's own line buffer — the shell's history
//! and cursor bindings never fire.
//!
//! This module is the minimal readline integration for that console case:
//! between the prompt and the accepted line the console input is switched
//! to raw mode (`ENABLE_LINE_INPUT`/`ENABLE_ECHO_INPUT`/
//! `ENABLE_PROCESSED_INPUT` off), keystrokes are translated to the exact
//! character stream the engine's edit dispatcher already understands, and
//! the edit line is redrawn after each key (readline.c rl_redisplay — in
//! raw mode nothing else echoes). Before an accepted command executes the
//! console is restored to its cooked mode, so children (`cat`, editors)
//! see a normal line-discipline console, exactly like readline's
//! save_tty_state/unix:tty restoration (bind.c prepare_terminal).

/// Turn one console key event into the character stream the edit
/// dispatcher consumes. Pure and platform-independent so the mapping is
/// unit-testable off-Windows.
///
/// * `vk` — the virtual-key code (`wVirtualKeyCode`);
/// * `ch` — the decoded UTF-16 character (`uChar`), when the event carries
///   one. Control keystrokes arrive pre-encoded in `uChar` by conhost
///   (Ctrl-A = 0x01), so the control bindings fall out of the character,
///   not the key.
pub fn translate_console_key(vk: u16, ch: Option<char>) -> Vec<char> {
    match vk {
        // VK_RETURN — accept-line; the dispatcher treats '\r' like '\n'.
        0x0D => vec!['\r'],
        // VK_BACK — readline binds Backspace to DEL (0x7f); the
        // dispatcher rubs 0x7f and 0x08 through the same rubout arm.
        0x08 => vec!['\x7f'],
        // VK_TAB — completion is a follow-up; keep the byte so the
        // dispatcher sees a shape it can safely drop.
        0x09 => vec!['\t'],
        // VK_ESCAPE — bare ESC introducer (the dispatcher consumes the
        // following sequence bytes).
        0x1B => vec!['\x1b'],
        // VK_UP / VK_DOWN / VK_RIGHT / VK_LEFT — the same CSI sequences a
        // unix terminal emits, wired to the dispatcher's existing
        // previous/next-history and forward/backward-char arms.
        0x26 => vec!['\x1b', '[', 'A'],
        0x28 => vec!['\x1b', '[', 'B'],
        0x27 => vec!['\x1b', '[', 'C'],
        0x25 => vec!['\x1b', '[', 'D'],
        // VK_HOME / VK_END — beginning/end of line (C-a / C-e).
        0x24 => vec!['\x01'],
        0x23 => vec!['\x05'],
        // VK_DELETE — delete-char at the cursor (readline's Delete binds
        // rl_delete, the C-d arm of the dispatcher).
        0x2E => vec!['\x04'],
        _ => match ch {
            Some(c) if c != '\0' => vec![c],
            _ => Vec::new(),
        },
    }
}

/// Display width of one scalar value, the small subset of wcwidth the
/// prompt/line redraw needs: zero for C0 controls, two for the common
/// East Asian Wide/Fullwidth ranges, zero for the common combining ranges,
/// one otherwise.
fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 {
        return 0;
    }
    if matches!(u,
        0x0300..=0x036F   // combining diacritics
        | 0x200B..=0x200F // zero-width joiners/marks
        | 0xFE00..=0xFE0F // variation selectors
    ) {
        return 0;
    }
    let wide = matches!(u,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E // CJK radicals .. CJK symbols (excl. 0x303F)
        | 0x3041..=0x33FF // kana .. CJK compatibility
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE4F // CJK compatibility forms
        | 0xFF00..=0xFF60 // fullwidth forms
        | 0xFFE0..=0xFFE6 // fullwidth signs
        | 0x20000..=0x3FFFD
    );
    if wide {
        2
    } else {
        1
    }
}

/// Display width of a rendered prompt line: ESC sequences (CSI/OSC and the
/// two-char escapes) carry no columns; visible characters count through
/// [`char_width`]. Theme prompts are color-laden, so skipping the
/// sequences is what keeps the redraw cursor honest.
pub fn prompt_display_width(line: &str) -> usize {
    let bytes: Vec<char> = line.chars().collect();
    let mut width = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        if c != '\x1b' {
            width += char_width(c);
            continue;
        }
        // ESC: consume the sequence. CSI (\x1b[ ... final 0x40-0x7E) or
        // OSC (\x1b] ... BEL or ESC \); anything else is a two-char escape.
        match bytes.get(i) {
            Some('[') => {
                i += 1;
                while i < bytes.len() && !('\x40'..='\x7e').contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            Some(']') => {
                i += 1;
                while i < bytes.len() && bytes[i] != '\x07' {
                    if bytes[i] == '\x1b' && i + 1 < bytes.len() && bytes[i + 1] == '\\' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            Some(_) => i += 1,
            None => {}
        }
    }
    width
}

/// The per-keystroke repaint (readline.c rl_redisplay, reduced): CR to the
/// prompt column, the prompt tail, the buffer, erase-to-end, then reposition
/// the cursor at the edit point. The caller prints the full prompt once per
/// read; this only refreshes the shared last line.
pub fn redraw_line(prompt_tail: &str, buffer: &str, cursor: usize) -> String {
    let col = prompt_display_width(prompt_tail)
        + buffer[..cursor.min(buffer.len())]
            .chars()
            .map(char_width)
            .sum::<usize>();
    let mut out = String::with_capacity(prompt_tail.len() + buffer.len() + 24);
    out.push('\r');
    out.push_str(prompt_tail);
    out.push_str(buffer);
    out.push_str("\x1b[K");
    out.push('\r');
    if col > 0 {
        out.push_str(&format!("\x1b[{col}C"));
    }
    out
}

/// A raw-mode console line editor over the process's stdin handle. `None`
/// everywhere the console path does not apply: non-Windows targets and any
/// stdin that is not a console (pipes, files, winpty/ConPTY relays that
/// present a pipe — those keep the cooked reader, whose byte-stream keys
/// the dispatcher already honors).
#[cfg(windows)]
pub struct RawConsole {
    handle: std::os::windows::io::RawHandle,
    cooked_mode: u32,
    raw: bool,
    /// High half of a UTF-16 surrogate pair awaiting its low half.
    pending_high_surrogate: Option<u16>,
}

#[cfg(windows)]
impl RawConsole {
    /// Take over the console stdin for raw-mode editing. Returns `None`
    /// when stdin is not a console handle.
    pub fn acquire() -> Option<RawConsole> {
        use std::os::windows::io::AsRawHandle;
        let stdin = std::io::stdin();
        let handle = stdin.as_raw_handle();
        let mut mode = 0u32;
        // GetConsoleMode succeeds only on console handles — the same probe
        // fd/windows_impl.rs uses to recognize a console.
        if unsafe { GetConsoleMode(handle as _, &mut mode) } == 0 {
            return None;
        }
        Some(RawConsole {
            handle: handle as _,
            cooked_mode: mode,
            raw: false,
            pending_high_surrogate: None,
        })
    }

    /// Enter raw mode for one edit line. No-op when already raw.
    pub fn enable(&mut self) {
        if self.raw {
            return;
        }
        // readline/terminal.c prepare_terminal: line discipline off during
        // the read. ENABLE_PROCESSED_INPUT off is what makes Ctrl-C a
        // dispatcher byte (0x03, line discard) instead of a console event;
        // ENABLE_WINDOW_INPUT keeps resize events flowing so they can be
        // drained instead of piling up.
        let raw_mode = (self.cooked_mode
            & !(ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT))
            | ENABLE_WINDOW_INPUT;
        if unsafe { SetConsoleMode(self.handle, raw_mode) } != 0 {
            self.raw = true;
        }
        self.pending_high_surrogate = None;
    }

    /// Restore the console's original mode — run before every accepted
    /// command so children see a cooked console, and on session teardown.
    pub fn restore(&mut self) {
        if !self.raw {
            return;
        }
        unsafe {
            SetConsoleMode(self.handle, self.cooked_mode);
        }
        self.raw = false;
        self.pending_high_surrogate = None;
    }

    /// Write directly to the console's output side (stderr, the readline
    /// rl_outstream): the repaint stream.
    pub fn write(&self, text: &str) {
        use std::io::Write;
        let mut err = std::io::stderr();
        let _ = err.write_all(text.as_bytes());
        let _ = err.flush();
    }

    /// Read one key event and return its translated character stream.
    /// `Ok(None)` marks the console EOF (the handle closed); non-key
    /// records (focus, resize, menu) are drained.
    pub fn next_key(&mut self) -> std::io::Result<Option<Vec<char>>> {
        loop {
            let mut record = INPUT_RECORD {
                event_type: 0,
                key: KEY_EVENT_RECORD {
                    key_down: 0,
                    repeat: 0,
                    vk: 0,
                    scan: 0,
                    ch: 0,
                    state: 0,
                },
            };
            let mut read = 0u32;
            let ok = unsafe { ReadConsoleInputW(self.handle, &mut record, 1, &mut read) };
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(6 /* ERROR_INVALID_HANDLE */) {
                    return Ok(None);
                }
                return Err(err);
            }
            if read == 0 || record.event_type != KEY_EVENT {
                continue;
            }
            let key = &record.key;
            if key.key_down == 0 || key.repeat == 0 {
                continue;
            }
            // UTF-16 assembly: a high surrogate parks until its low half.
            let ch = if (0xD800..0xDC00).contains(&key.ch) {
                self.pending_high_surrogate = Some(key.ch);
                None
            } else if (0xDC00..0xE000).contains(&key.ch) {
                match self.pending_high_surrogate.take() {
                    Some(high) => {
                        let c =
                            0x10000 + (((high as u32) - 0xD800) << 10) + ((key.ch as u32) - 0xDC00);
                        char::from_u32(c)
                    }
                    None => None,
                }
            } else {
                self.pending_high_surrogate = None;
                char::from_u32(key.ch as u32).filter(|c| *c != '\0')
            };
            let translated = translate_console_key(key.vk, ch);
            if translated.is_empty() {
                continue;
            }
            // A held key repeats: honor the count like a terminal's
            // autorepeat would (ReadConsoleInputW reports it per record).
            let mut out = Vec::with_capacity(translated.len() * key.repeat as usize);
            for _ in 0..key.repeat {
                out.extend_from_slice(&translated);
            }
            return Ok(Some(out));
        }
    }
}

#[cfg(windows)]
impl Drop for RawConsole {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(not(windows))]
pub struct RawConsole {
    _private: (),
}

#[cfg(not(windows))]
impl RawConsole {
    pub fn acquire() -> Option<RawConsole> {
        None
    }
    pub fn enable(&mut self) {}
    pub fn restore(&mut self) {}
    pub fn write(&self, _text: &str) {}
    pub fn next_key(&mut self) -> std::io::Result<Option<Vec<char>>> {
        Ok(None)
    }
}

const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
const ENABLE_LINE_INPUT: u32 = 0x0002;
const ENABLE_ECHO_INPUT: u32 = 0x0004;
const ENABLE_WINDOW_INPUT: u32 = 0x0008;
const KEY_EVENT: u16 = 0x0001;

#[cfg(windows)]
type HANDLE = *mut std::ffi::c_void;

#[cfg(windows)]
#[repr(C)]
struct KEY_EVENT_RECORD {
    key_down: i32,
    repeat: u16,
    vk: u16,
    scan: u16,
    ch: u16,
    state: u32,
}

#[cfg(windows)]
/// wincon.h INPUT_RECORD: the 4-byte-aligned event union follows the 2-byte
/// event type (repr(C) inserts the pad; the record is 20 bytes on x86 and
/// x64 alike). Only the key-event arm is materialized — this shell never
/// reads the other console event kinds, it drains them.
#[repr(C)]
struct INPUT_RECORD {
    event_type: u16,
    key: KEY_EVENT_RECORD,
}

#[cfg(windows)]
extern "system" {
    fn GetConsoleMode(hConsoleInput: HANDLE, mode: *mut u32) -> i32;
    fn SetConsoleMode(hConsoleInput: HANDLE, mode: u32) -> i32;
    fn ReadConsoleInputW(
        hConsoleInput: HANDLE,
        record: *mut INPUT_RECORD,
        length: u32,
        read: *mut u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keys_translate_to_csi_sequences() {
        assert_eq!(translate_console_key(0x26, None), vec!['\x1b', '[', 'A']);
        assert_eq!(translate_console_key(0x28, None), vec!['\x1b', '[', 'B']);
        assert_eq!(translate_console_key(0x25, None), vec!['\x1b', '[', 'D']);
        assert_eq!(translate_console_key(0x27, None), vec!['\x1b', '[', 'C']);
    }

    #[test]
    fn enter_backspace_home_end_delete_translate() {
        assert_eq!(translate_console_key(0x0D, Some('\r')), vec!['\r']);
        assert_eq!(translate_console_key(0x08, None), vec!['\x7f']);
        assert_eq!(translate_console_key(0x24, None), vec!['\x01']);
        assert_eq!(translate_console_key(0x23, None), vec!['\x05']);
        assert_eq!(translate_console_key(0x2E, None), vec!['\x04']);
    }

    #[test]
    fn control_and_printable_characters_pass_through() {
        // conhost pre-encodes control keys in uChar (Ctrl-A = 0x01).
        assert_eq!(translate_console_key(0x41, Some('\x01')), vec!['\x01']);
        assert_eq!(translate_console_key(0x52, Some('\x12')), vec!['\x12']);
        assert_eq!(translate_console_key(0x43, Some('\x03')), vec!['\x03']);
        assert_eq!(translate_console_key(0x00, Some('x')), vec!['x']);
        assert_eq!(translate_console_key(0x00, None), Vec::<char>::new());
    }

    #[test]
    fn prompt_width_skips_ansi_sequences() {
        assert_eq!(prompt_display_width("$ "), 2);
        assert_eq!(prompt_display_width("\x1b[1;32mRC> \x1b[0m"), 4);
        assert_eq!(prompt_display_width("\x1b]0;title\x07$ "), 2);
        assert_eq!(prompt_display_width("\x1b]0;title\x1b\\$ "), 2);
        // East Asian Wide counts two columns.
        assert_eq!(prompt_display_width("中> "), 4);
    }

    #[test]
    fn redraw_positions_cursor_at_the_edit_point() {
        let out = redraw_line("RC> ", "echo hi", 5);
        assert!(out.starts_with('\r'));
        assert!(out.contains("RC> echo hi\x1b[K"));
        // after the buffer (7 cols) the cursor returns to 5+4=9
        assert!(out.ends_with("\r\x1b[9C"));

        let out = redraw_line("$ ", "abc", 0);
        // cursor sits after the 2-column prompt even at buffer start
        assert!(out.ends_with("\r\x1b[2C"));
    }

    #[test]
    fn redraw_counts_wide_characters_to_the_cursor() {
        // 中 = 2 columns; cursor after it is at 2+2 = 4.
        let out = redraw_line("> ", "中x", 3);
        assert!(out.ends_with("\r\x1b[4C"));
    }
}
