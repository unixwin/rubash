//! bind module — pure parsing and listing formats.
//!
//! GNU Bash source ownership:
//! - builtins/bind.def (option grammar, query_bindings/unbind_command/
//!   unbind_keyseq)
//! - bashline.c:4814 isolate_sequence / :4850 bind_keyseq_to_unix_command
//!   (the `-x` argument grammar)
//! - bashline.c:4720 print_unix_command + print_unix_command_map (`-X`
//!   listing format)
//! - The stateful half lives in the executor (`Executor::execute_bind`,
//!   the registry in [`crate::shell::bind_registry`]); hosts mirror the
//!   registry into their own line-editor keymaps.
//!
//! Probe evidence (WSL GNU bash 5.3.0, target/issue-suites/results/
//! wt100-i185/): `bind -X` prints `"\C-g" "echo bound-c-g"`; `bind -s`
//! prints `"\C-t": "ins-macro"`; `bind -S` prints `\C-t outputs
//! ins-macro`; `-q` answers `NAME can be invoked via "seq", ...` (rc 0)
//! or `NAME is not bound to any keys.` (rc 1) or
//! ``bind: `NAME': unknown function name`` (rc 1); `-p`/`-P`/`-l`/`-v`
//! print to stdout; `-x noquotes` errors with `first non-whitespace
//! character is not '"'`.

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

/// The readline function names the engine's `bind` world knows: the names
/// the niubash reedline bridge maps (crates/niubash-runtime repl.rs
/// `native_widget_event`) plus the classic names carried by reedline's
/// default emacs/vi keymaps. GNU's table has 176 names (bind -l probe);
/// niu's editor implements this subset — listing more would claim
/// bindings no host can deliver.
pub const READLINE_FUNCTIONS: &[&str] = &[
    "abort",
    "accept-line",
    "backward-char",
    "backward-delete-char",
    "backward-kill-line",
    "backward-kill-word",
    "backward-word",
    "beginning-of-buffer-or-history",
    "beginning-of-history",
    "beginning-of-line",
    "call-last-kbd-macro",
    "capitalize-word",
    "clear-screen",
    "complete-word",
    "delete-char",
    "downcase-word",
    "down-line-or-history",
    "emacs-editing-mode",
    "end-of-buffer-or-history",
    "end-of-history",
    "end-of-line",
    "exchange-point-and-mark",
    "expand-or-complete",
    "forward-char",
    "forward-search-history",
    "forward-word",
    "history-incremental-search-backward",
    "history-incremental-search-forward",
    "history-substring-search-down",
    "history-substring-search-up",
    "kill-line",
    "kill-whole-line",
    "kill-word",
    "menu-complete",
    "menu-previous",
    "next-history",
    "operate-and-get-next",
    "overwrite-mode",
    "previous-history",
    "quoted-insert",
    "re-read-init-file",
    "redisplay",
    "redo",
    "reverse-search-history",
    "self-insert",
    "transpose-chars",
    "transpose-words",
    "undo",
    "unix-line-discard",
    "unix-word-rubout",
    "upcase-word",
    "up-line-or-history",
    "vi-editing-mode",
    "yank",
    "yank-pop",
];

/// Default function bindings niu's editor ships (reedline
/// default_emacs_keybindings mapped to their readline function names;
/// arrow/prefix bindings from the engine's historical default table).
/// `bind -p`/`-P`/`-q` answers beyond this table come from the registry.
pub const DEFAULT_FUNCTION_BINDINGS: &[(&str, &str)] = &[
    (r#"\e[1~"#, "beginning-of-line"),
    (r#"\e[4~"#, "end-of-line"),
    (r#"\e[5~"#, "beginning-of-history"),
    (r#"\e[6~"#, "end-of-history"),
    (r#"\e[A"#, "previous-history"),
    (r#"\e[B"#, "next-history"),
    (r#"\e[C"#, "forward-char"),
    (r#"\e[D"#, "backward-char"),
    (r#"\e[3~"#, "delete-char"),
    (r#"\e"#, "emacs-editing-mode"),
    (r#"\C-a"#, "beginning-of-line"),
    (r#"\C-b"#, "backward-char"),
    (r#"\C-e"#, "end-of-line"),
    (r#"\C-f"#, "forward-char"),
    (r#"\C-k"#, "kill-line"),
    (r#"\C-l"#, "clear-screen"),
    (r#"\C-n"#, "next-history"),
    (r#"\C-p"#, "previous-history"),
    (r#"\C-r"#, "reverse-search-history"),
    (r#"\C-t"#, "transpose-chars"),
    (r#"\C-u"#, "unix-line-discard"),
    (r#"\C-w"#, "unix-word-rubout"),
    (r#"\C-y"#, "yank"),
    (r#"\C-z"#, "undo"),
    (r#"\C-g"#, "redo"),
];

/// GNU 5.3's readline variable table in `bind -v` output order, with each
/// variable's default value (WT probe: WSL GNU bash 5.3.0 `bind -v`,
/// 48 lines). Booleans hold `on`/`off`, numbers decimal integers, strings
/// verbatim. `editing-mode` and `keymap` are the two variables GNU keeps
/// cross-consistent: `set editing-mode vi` also switches `keymap` to
/// `vi-insert` (GNU probe) — [`apply_readline_variable`] maintains that.
pub const READLINE_VARIABLE_DEFAULTS: &[(&str, &str)] = &[
    ("bind-tty-special-chars", "off"),
    ("blink-matching-paren", "off"),
    ("byte-oriented", "off"),
    ("colored-completion-prefix", "off"),
    ("colored-stats", "off"),
    ("completion-ignore-case", "off"),
    ("completion-map-case", "off"),
    ("convert-meta", "off"),
    ("disable-completion", "off"),
    ("echo-control-characters", "on"),
    ("enable-active-region", "off"),
    ("enable-bracketed-paste", "off"),
    ("enable-keypad", "off"),
    ("enable-meta-key", "on"),
    ("expand-tilde", "off"),
    ("force-meta-prefix", "off"),
    ("history-preserve-point", "off"),
    ("horizontal-scroll-mode", "off"),
    ("input-meta", "on"),
    ("mark-directories", "on"),
    ("mark-modified-lines", "off"),
    ("mark-symlinked-directories", "off"),
    ("match-hidden-files", "on"),
    ("menu-complete-display-prefix", "off"),
    ("meta-flag", "on"),
    ("output-meta", "on"),
    ("page-completions", "on"),
    ("prefer-visible-bell", "on"),
    ("print-completions-horizontally", "off"),
    ("revert-all-at-newline", "off"),
    ("search-ignore-case", "off"),
    ("show-all-if-ambiguous", "off"),
    ("show-all-if-unmodified", "off"),
    ("show-mode-in-prompt", "off"),
    ("skip-completed-text", "off"),
    ("visible-stats", "off"),
    ("bell-style", "audible"),
    ("comment-begin", "#"),
    ("completion-display-width", "-1"),
    ("completion-prefix-display-length", "0"),
    ("completion-query-items", "100"),
    ("editing-mode", "emacs"),
    ("emacs-mode-string", "@"),
    ("history-size", "-1"),
    ("keymap", "emacs"),
    ("keyseq-timeout", "500"),
    ("vi-cmd-mode-string", "(cmd)"),
    ("vi-ins-mode-string", "(ins)"),
];

/// How one `set NAME VALUE` resolved against the readline variable table
/// (readline bind.c rl_variable_bind):
/// - [`SetVariableOutcome::Applied`] — stored (booleans normalize
///   non-`on` values to `off` and numbers non-numeric text to `0`
///   silently, GNU probes: `set completion-ignore-case maybe` → `off`,
///   `set keyseq-timeout abc` → `0`; `set keymap bogus` keeps the old
///   keymap silently).
/// - [`SetVariableOutcome::UnknownVariable`] — GNU prints
///   ``readline: NAME: unknown variable name`` (probe) and continues.
/// - [`SetVariableOutcome::CouldNotSetValue`] — only `editing-mode`
///   rejects values loudly: ``readline: editing-mode: could not set
///   value to `bogus'`` (probe), variable unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetVariableOutcome {
    Applied,
    UnknownVariable,
    CouldNotSetValue,
}

/// Apply `set NAME VALUE` to a `bind -v` variable table (name → value, in
/// [`READLINE_VARIABLE_DEFAULTS`] order). Returns the outcome plus the
/// normalized value/keymap a caller may need to mirror (editing-mode
/// lowercases and switches `keymap`; GNU probe: `set editing-mode VI`
/// stores `vi` and `set editing-mode vi` stores `keymap vi-insert`).
pub fn apply_readline_variable(
    variables: &mut Vec<(String, String)>,
    name: &str,
    value: &str,
) -> (SetVariableOutcome, Option<(&'static str, &'static str)>) {
    let Some((_, kind)) = variable_kind(name) else {
        return (SetVariableOutcome::UnknownVariable, None);
    };
    match kind {
        VariableKind::EditingMode => {
            let lowered = value.to_ascii_lowercase();
            let mode = match lowered.as_str() {
                "emacs" => "emacs",
                "vi" => "vi",
                _ => {
                    return (SetVariableOutcome::CouldNotSetValue, None);
                }
            };
            let keymap = if mode == "vi" { "vi-insert" } else { "emacs" };
            store_variable(variables, name, mode);
            store_variable(variables, "keymap", keymap);
            (
                SetVariableOutcome::Applied,
                Some(("editing-mode", if mode == "vi" { "vi" } else { "emacs" })),
            )
        }
        VariableKind::Keymap => {
            // A bad keymap name keeps the current value silently (GNU
            // probe: `set keymap bogus` → keymap stays emacs, no message).
            if crate::shell::bind_registry::KEYMAP_NAMES.contains(&value) {
                store_variable(variables, name, value);
            }
            (SetVariableOutcome::Applied, None)
        }
        VariableKind::Boolean => {
            let normalized = if value.eq_ignore_ascii_case("on") {
                "on"
            } else {
                "off"
            };
            store_variable(variables, name, normalized);
            (SetVariableOutcome::Applied, None)
        }
        VariableKind::Number => {
            let normalized = strtol_i64(value);
            store_variable(variables, name, &normalized.to_string());
            (SetVariableOutcome::Applied, None)
        }
        VariableKind::String => {
            store_variable(variables, name, value);
            (SetVariableOutcome::Applied, None)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum VariableKind {
    Boolean,
    Number,
    String,
    EditingMode,
    Keymap,
}

fn variable_kind(name: &str) -> Option<(&'static str, VariableKind)> {
    READLINE_VARIABLE_DEFAULTS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(candidate, _)| (*candidate, kind_of(candidate)))
}

fn kind_of(name: &str) -> VariableKind {
    match name {
        "editing-mode" => VariableKind::EditingMode,
        "keymap" => VariableKind::Keymap,
        "bell-style" | "comment-begin" | "emacs-mode-string" | "vi-cmd-mode-string"
        | "vi-ins-mode-string" => VariableKind::String,
        "completion-display-width"
        | "completion-prefix-display-length"
        | "completion-query-items"
        | "history-size"
        | "keyseq-timeout" => VariableKind::Number,
        _ => VariableKind::Boolean,
    }
}

fn store_variable(variables: &mut Vec<(String, String)>, name: &str, value: &str) {
    match variables
        .iter_mut()
        .find(|(candidate, _)| candidate == name)
    {
        Some(slot) => slot.1 = value.to_string(),
        None => variables.push((name.to_string(), value.to_string())),
    }
}

/// strtol semantics readline uses for numeric variables: optional
/// whitespace and sign, leading decimal digits, `500abc` → 500, garbage →
/// 0 (GNU probe: `set keyseq-timeout abc` stores 0).
fn strtol_i64(text: &str) -> i64 {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars().peekable();
    let mut sign = 1i64;
    match chars.peek() {
        Some('+') => {
            chars.next();
        }
        Some('-') => {
            sign = -1;
            chars.next();
        }
        _ => {}
    }
    let mut value: i64 = 0;
    let mut any = false;
    for ch in chars {
        match ch.to_digit(10) {
            Some(digit) => {
                any = true;
                value = value.saturating_mul(10).saturating_add(digit as i64);
            }
            None => break,
        }
    }
    if any {
        sign * value
    } else {
        0
    }
}

/// Grammar failure of a `-x` or inputrc-form binding argument. `message`
/// is the GNU text without the `bind: ` prefix and without the shell's
/// `name: line N: ` diagnostic prefix.
///
/// The two argument channels report failures differently in GNU:
/// - `-x` (bind_keyseq_to_unix_command, bashline.c:4850) fails the
///   builtin with rc 1 under the shell's `bind: ` prefix (GNU probe:
///   `bind -x '"\C-g'` → `bash: bind: no closing `"' in "\C-g`, rc 1).
///   That is `message` alone.
/// - a plain positional goes through rl_parse_and_bind, whose errors
///   print as `readline: …` WITHOUT the shell prefix and do NOT fail the
///   builtin — bind.def:304-333 ignores the return value (GNU probes:
///   `bind '"\C-t'` → ``readline: "\C-t: no closing `"' in key binding``
///   rc 0; `bind '"\C-t"'` → ``readline: "\C-t": no key sequence
///   terminator`` rc 0). `readline_message` carries that text; the
///   caller prints it and leaves the status alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindSpecError {
    pub message: String,
    /// The quote character that never closed (no-closing errors).
    pub delimiter: Option<char>,
    /// The `readline: …` text for the plain-positional channel, when this
    /// error class reports there.
    pub readline_message: Option<String>,
}

impl BindSpecError {
    pub fn message(message: impl Into<String>) -> Self {
        BindSpecError {
            message: message.into(),
            delimiter: None,
            readline_message: None,
        }
    }
}

/// The inputrc-form binding value: an unquoted value names a readline
/// function, a quoted value is a macro (bind.def SHORT_DOC "keyseq:
/// readline-function or readline-command"; the wt100 GNU probe shows
/// `bind '"\C-t": ins-macro'` binds nothing — unknown function name,
/// silently ignored by rl_parse_and_bind — while the quoted form
/// `bind '"\C-t": "ins-macro"'` binds a macro `bind -s` lists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingSpec {
    Function { name: String },
    Macro { text: String },
}

/// One parsed plain binding argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBinding {
    pub keyseq: String,
    pub spec: BindingSpec,
}

/// Port of bashline.c:4814 isolate_sequence: skip leading whitespace,
/// require `need_dquote` when told, find the closing unescaped delimiter
/// (`"`, `'`, or none), return the RAW interior plus the index just past
/// the closing quote. Callers decide which escape translation applies
/// (key sequences keep the written `\C-g` form for the host key parser;
/// macro text and whitespace-separated commands go through
/// [`unescape_macro_text`], matching rl_macro_bind's translate path at
/// bashline.c:4893-4896).
fn isolate_sequence(
    line: &str,
    start: usize,
    need_dquote: bool,
) -> Result<(String, usize), BindSpecError> {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() {
        if need_dquote {
            return Err(BindSpecError {
                message: format!("{line}: first non-whitespace character is not `\"'"),
                delimiter: None,
                readline_message: None,
            });
        }
        return Err(BindSpecError {
            message: format!("{line}: missing separator"),
            delimiter: None,
            readline_message: None,
        });
    }
    let delim = matches!(bytes[i], b'"' | b'\'').then(|| bytes[i]);
    if need_dquote && delim != Some(b'"') {
        return Err(BindSpecError {
            message: format!("{line}: first non-whitespace character is not `\"'"),
            delimiter: None,
            readline_message: None,
        });
    }
    let content_start;
    if let Some(d) = delim {
        i += 1;
        content_start = i;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == d {
                break;
            }
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != d {
            return Err(BindSpecError {
                message: format!("no closing `{}' in {line}", d as char),
                delimiter: Some(d as char),
                readline_message: None,
            });
        }
        Ok((line[content_start..i].to_string(), i + 1))
    } else {
        content_start = i;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            i += 1;
        }
        Ok((line[content_start..].to_string(), i))
    }
}

/// Resolve only the delimiter escapes a stored KEY SEQUENCE keeps
/// written: `\\` → `\`, `\"` → `"`; every other sequence (including
/// `\C-g`, `\e[A`, `\M-x`) stays in written form because the host key
/// parser (reedline bridge `parse_key_sequence`) reads that form. GNU
/// translates the sequence to raw bytes for matching and renders it back
/// in this same backslash form on dump (wt100 probe: `bind -X` prints
/// `"\C-g" "echo bound-c-g"`).
pub fn unescape_keyseq(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('\\') => {
                out.push('\\');
                chars.next();
            }
            Some('"') => {
                out.push('"');
                chars.next();
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// readline's \-escape translation for MACRO text and translated command
/// values (rl_translate_keyseq): \n \t \r \e \a \b \f \v, `\\` and quote
/// escapes, \C-x control characters, \M- ESC meta prefixes, and octal
/// \NNN. Unknown escapes keep the backslash.
pub fn unescape_macro_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let next = *chars.peek().unwrap_or(&'\\');
        match next {
            'n' => {
                out.push('\n');
                chars.next();
            }
            't' => {
                out.push('\t');
                chars.next();
            }
            'r' => {
                out.push('\r');
                chars.next();
            }
            'e' => {
                out.push('\x1b');
                chars.next();
            }
            'a' => {
                out.push('\x07');
                chars.next();
            }
            'b' => {
                out.push('\x08');
                chars.next();
            }
            'f' => {
                out.push('\x0c');
                chars.next();
            }
            'v' => {
                out.push('\x0b');
                chars.next();
            }
            '\\' => {
                out.push('\\');
                chars.next();
            }
            '"' => {
                out.push('"');
                chars.next();
            }
            '\'' => {
                out.push('\'');
                chars.next();
            }
            'C' => {
                // \C-x: control character (rl_translate_keyseq's CTL
                // handling). \C-? is DEL.
                chars.next();
                match chars.next() {
                    Some('-') => match chars.next() {
                        Some('?') => out.push('\x7f'),
                        Some(letter) => {
                            let upper = letter.to_ascii_uppercase();
                            out.push((upper as u8 & 0x1f) as char);
                        }
                        None => {}
                    },
                    other => {
                        // Not \C-x — keep written form.
                        out.push_str("\\C");
                        if let Some(ch) = other {
                            out.push(ch);
                        }
                    }
                }
            }
            'M' => {
                // \M-x: meta prefix = ESC before x.
                chars.next();
                match chars.next() {
                    Some('-') => {
                        out.push('\x1b');
                    }
                    other => {
                        out.push_str("\\M");
                        if let Some(ch) = other {
                            out.push(ch);
                        }
                    }
                }
            }
            '0'..='7' => {
                // \NNN octal.
                let mut value: u32 = 0;
                let mut digits = 0;
                while digits < 3 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            value = value * 8 + digit;
                            chars.next();
                            digits += 1;
                        }
                        None => break,
                    }
                }
                if let Some(byte) = char::from_u32(value) {
                    out.push(byte);
                }
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// Parse a `bind -x` argument: `"keyseq"[:|ws]["']command["']`
/// (bashline.c:4850 bind_keyseq_to_unix_command). The keyseq MUST be
/// double-quoted; colon or whitespace separates it from the command.
/// Colon separator stores the command verbatim (rl_generic_bind);
/// whitespace separator requires the command to be quoted and gets the
/// readline escape translation (rl_macro_bind path, translate=true at
/// bashline.c:4893-4896).
pub fn parse_unix_command_spec(line: &str) -> Result<(String, String), BindSpecError> {
    let (keyseq_raw, mut i) = isolate_sequence(line, 0, true)?;
    let keyseq = unescape_keyseq(&keyseq_raw);
    // Scan forward to the separator: colon or whitespace
    // (bashline.c:4877-4879).
    let bytes = line.as_bytes();
    while i < bytes.len() && bytes[i] != b':' && bytes[i] != b' ' && bytes[i] != b'\t' {
        i += 1;
    }
    if i >= bytes.len() {
        return Err(BindSpecError {
            message: format!("{line}: missing separator"),
            delimiter: None,
            readline_message: None,
        });
    }
    if bytes[i] == b':' {
        // Colon separator: the command is stored verbatim
        // (rl_generic_bind, bashline.c:4898-4900).
        let (command, _) = isolate_sequence(line, i + 1, false)?;
        Ok((keyseq, command))
    } else {
        // Whitespace separator: command must be quoted and gets the
        // readline escape translation (rl_macro_bind path,
        // bashline.c:4893-4896).
        let (command_raw, _) = isolate_sequence(line, i, true)?;
        Ok((keyseq, unescape_macro_text(&command_raw)))
    }
}

/// Parse a plain inputrc-form binding argument `"keyseq": value`
/// (rl_parse_and_bind). Quoted value → macro; unquoted value → function
/// name (GNU: `bind '"\C-t": ins-macro'` binds nothing — an unknown
/// function name is silently dropped; see wt100 probe). Returns None
/// when the argument is not this form (non-quote, non-`set` text —
/// rl_parse_and_bind ignores it silently, GNU probe `bind 'ls -l'` rc 0).
///
/// Failures carry `readline_message` (see [`BindSpecError`]): GNU shapes
/// from the wt100-era probes — ``"\C-t: no closing `"' in key binding``,
/// `"\C-t": no key sequence terminator`, ``'ins: missing closing quote
/// for macro`` — printed after `readline: ` with no effect on status.
pub fn parse_binding_line(line: &str) -> Option<Result<ParsedBinding, BindSpecError>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('"') {
        return None;
    }
    let (keyseq_raw, mut i) = match isolate_sequence(line, 0, true) {
        Ok(parsed) => parsed,
        Err(err) => {
            let readline_message = err
                .delimiter
                .map(|delimiter| format!("{line}: no closing `{delimiter}' in key binding"));
            return Some(Err(BindSpecError {
                message: err.message,
                delimiter: err.delimiter,
                readline_message,
            }));
        }
    };
    let keyseq = unescape_keyseq(&keyseq_raw);
    let bytes = line.as_bytes();
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b':' {
        i += 1;
    }
    // Quoted value → macro (rl_parse_and_bind's ISMACR path); anything
    // else is a function name, taken verbatim to end of line.
    let mut probe = i;
    while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
        probe += 1;
    }
    if probe < bytes.len() && (bytes[probe] == b'"' || bytes[probe] == b'\'') {
        let macro_text_start = probe;
        // isolate_sequence auto-detects the quote character (GNU passes
        // need_dquote=0 here; either quote works as the macro delimiter).
        let (text_raw, _) = match isolate_sequence(line, i, false) {
            Ok(parsed) => parsed,
            Err(err) => {
                let readline_message = if err.delimiter.is_some() {
                    Some(format!(
                        "{}: missing closing quote for macro",
                        line[macro_text_start..].trim_end()
                    ))
                } else {
                    None
                };
                return Some(Err(BindSpecError {
                    message: err.message,
                    delimiter: err.delimiter,
                    readline_message,
                }));
            }
        };
        Some(Ok(ParsedBinding {
            keyseq,
            spec: BindingSpec::Macro {
                text: unescape_macro_text(&text_raw),
            },
        }))
    } else {
        let (name, _) = match isolate_sequence(line, i, false) {
            Ok(parsed) => parsed,
            Err(err) => {
                // The keyseq closed but no `: value` followed — GNU
                // readline: `"\C-t": no key sequence terminator`.
                let readline_message = (err.message.ends_with("missing separator"))
                    .then(|| format!("{line}: no key sequence terminator"));
                return Some(Err(BindSpecError {
                    message: err.message,
                    delimiter: None,
                    readline_message,
                }));
            }
        };
        Some(Ok(ParsedBinding {
            keyseq,
            spec: BindingSpec::Function { name },
        }))
    }
}

/// True when `name` is in the supported function table.
pub fn is_known_function(name: &str) -> bool {
    READLINE_FUNCTIONS.contains(&name)
}

/// `bind` usage text (builtins/bind.def SHORT_DOC).
pub fn usage_line() -> &'static str {
    "bind: usage: bind [-lpsvPSVX] [-m keymap] [-f filename] [-q name] [-u name] [-r keyseq] [-x keyseq:shell-command] [keyseq:readline-function or readline-command]"
}

/// Write the usage line (bind.def default: builtin_usage → EX_USAGE).
pub fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(stderr, "{}", usage_line())
}

/// `bind -X` line: `"\C-g" "command"` (print_unix_command,
/// bashline.c:4720-4727, readable form).
pub fn format_unix_command_readable(keyseq: &str, command: &str) -> String {
    format!("\"{keyseq}\" \"{command}\"")
}

/// `bind -s` line: `"\C-t": "ins-macro"` (rl_macro_dumper readable).
pub fn format_macro_readable(keyseq: &str, text: &str) -> String {
    format!("\"{keyseq}\": \"{text}\"")
}

/// `bind -S` line: `\C-t outputs ins-macro` (GNU probe; rl_macro_dumper
/// plain form).
pub fn format_macro_plain(keyseq: &str, text: &str) -> String {
    format!("{keyseq} outputs {text}")
}

/// `bind -p` line: `"\C-a": beginning-of-line` (rl_function_dumper(1)).
pub fn format_function_readable(keyseq: &str, name: &str) -> String {
    format!("\"{keyseq}\": {name}")
}

/// `bind -v` line: `set completion-ignore-case on`
/// (rl_variable_dumper(1) readable form; WSL GNU 5.3 probe).
pub fn format_variable_readable(name: &str, value: &str) -> String {
    format!("set {name} {value}")
}

/// `bind -V` line: ``completion-ignore-case is set to `off'``
/// (rl_variable_dumper(0) plain form; WSL GNU 5.3 probe).
pub fn format_variable_plain(name: &str, value: &str) -> String {
    format!("{name} is set to `{value}'")
}

/// `bind -P` line: `abort can be found on "\C-g", "\C-x\C-g".`
pub fn format_function_plain(name: &str, keyseqs: &[String]) -> String {
    let quoted: Vec<String> = keyseqs.iter().map(|seq| format!("\"{seq}\"")).collect();
    format!("{name} can be found on {}.", quoted.join(", "))
}

/// `bind -q` positive answer (query_bindings, bind.def:344-367): at most
/// five key sequences, `...` when truncated.
pub fn format_query(name: &str, keyseqs: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let shown = keyseqs.len().min(5);
    for seq in keyseqs.iter().take(shown) {
        parts.push(format!("\"{seq}\""));
    }
    let mut out = format!("{name} can be invoked via ");
    if keyseqs.len() > shown {
        out.push_str(&parts.join(", "));
        out.push_str(", ...");
    } else {
        out.push_str(&parts.join(", "));
        out.push('.');
    }
    out
}

/// `bind -q` negative answer: `NAME is not bound to any keys.`
pub fn format_query_unbound(name: &str) -> String {
    format!("{name} is not bound to any keys.")
}

/// ``bind: `NAME': unknown function name`` (bind.def:351).
pub fn format_unknown_function(name: &str) -> String {
    format!("`{name}': unknown function name")
}

/// ``bind: `NAME': invalid keymap name`` (bind.def:255).
pub fn format_invalid_keymap(name: &str) -> String {
    format!("`{name}': invalid keymap name")
}

/// `bind: FILE: cannot read: ERRNO` (bind.def:274-276).
pub fn format_cannot_read(file: &str, reason: &str) -> String {
    format!("{file}: cannot read: {reason}")
}

/// Parse one positional binding argument. `bind -x` values arrive through
/// [`parse_unix_command_spec`]; this handles the plain inputrc form.
pub fn parse_positional_binding(arg: &str) -> Result<ParsedBinding, BindSpecError> {
    match parse_binding_line(arg) {
        Some(Ok(parsed)) => Ok(parsed),
        Some(Err(err)) => Err(err),
        None => Err(BindSpecError::message(format!("{arg}: missing separator"))),
    }
}

/// Strip a case-insensitive `set` keyword with its separating whitespace
/// (readline matches the keyword case-insensitively); `None` when the
/// line is not a variable assignment.
pub fn strip_set_keyword(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.len() >= 3 && trimmed[..3].eq_ignore_ascii_case("set") {
        let rest = &trimmed[3..];
        if rest.starts_with(char::is_whitespace) {
            return Some(rest);
        }
    }
    None
}

/// The environment an inputrc file's conditionals evaluate against:
/// `$if term=VALUE` compares `$TERM` verbatim (case-sensitively — GNU
/// probe: `$if term=XTERM` does not match TERM=xterm) and `$if mode=VALUE`
/// compares the current editing mode (`set editing-mode` lines earlier in
/// the same file update it for later conditionals — GNU probe: a
/// `mode=vi` block after `set editing-mode vi` applies). A bare
/// `$if word` or unknown key is an error whose block never applies
/// (GNU probe: `readline: ...: equality comparison operator expected,
/// found `end-of-line'`).
#[derive(Debug, Clone, Default)]
pub struct InputrcEnv<'a> {
    pub term: Option<&'a str>,
    pub editing_mode: Option<&'a str>,
}

/// One `readline: ...` diagnostic from parsing. `file` is the file the
/// line came from ($include files carry their own name), `line` its
/// 1-based number, and `core` the message body GNU prints after the
/// `file: line N: ` middle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputrcDiagnostic {
    pub file: String,
    pub line: usize,
    pub core: String,
}

/// Everything one inputrc parse produced, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputrcParsed {
    /// `"keyseq": value` bindings from active regions.
    pub bindings: Vec<(String, BindingSpec)>,
    /// Validated `set NAME VALUE` lines (value normalized the way
    /// [`apply_readline_variable`] normalizes; `editing-mode` normalized
    /// lowercase).
    pub variables: Vec<(String, String)>,
    pub diagnostics: Vec<InputrcDiagnostic>,
}

/// Parse an inputrc body (`bind -f` / the startup ~/.inputrc read):
/// bindings, `set NAME VALUE` variables, `$include FILE`, and
/// `$if term=…/mode=…` / `$else` / `$endif` conditionals (nesting
/// supported; a `$if`-family line is recognized even inside inactive
/// regions so its `$endif` still matches). `load` fetches `$include`
/// content — a missing include is silently skipped (GNU probe:
/// `bind -f` over a file with `$include nope.rc` proceeds, rc 0).
pub fn parse_inputrc_with_env(
    text: &str,
    file: &str,
    env: &InputrcEnv,
    load: &mut dyn FnMut(&str) -> Option<String>,
) -> InputrcParsed {
    let mut state = InputrcParseState {
        result: InputrcParsed::default(),
        env: env.clone(),
        file: file.to_string(),
        // GNU/readline caps conditional nesting (INIT_NEST_DEPTH 10 in
        // readline's encapsulations); deeper `$if`s are ignored.
        conditionals: Vec::new(),
        include_depth: 0,
    };
    state.parse_text(text, load);
    state.result
}

struct InputrcParseState<'a> {
    result: InputrcParsed,
    env: InputrcEnv<'a>,
    file: String,
    conditionals: Vec<ConditionalRegion>,
    include_depth: u8,
}

#[derive(Clone, Copy)]
struct ConditionalRegion {
    /// Whether lines in this region are active.
    active: bool,
    /// Whether any branch of this region has already been active (an
    /// `$else` is only active when no earlier branch was).
    taken: bool,
}

impl<'a> InputrcParseState<'a> {
    fn active(&self) -> bool {
        self.conditionals.iter().all(|region| region.active)
    }

    fn diagnostic(&mut self, line: usize, core: String) {
        self.result.diagnostics.push(InputrcDiagnostic {
            file: self.file.clone(),
            line,
            core,
        });
    }

    fn parse_text(&mut self, text: &str, load: &mut dyn FnMut(&str) -> Option<String>) {
        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            let number = index + 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('$') {
                self.parse_directive(rest.trim(), number, load);
                continue;
            }
            if !self.active() {
                continue;
            }
            if let Some(rest) = strip_set_keyword(line) {
                self.parse_set_line(rest, number);
                continue;
            }
            match parse_binding_line(line) {
                Some(Ok(parsed)) => self.result.bindings.push((parsed.keyseq, parsed.spec)),
                // rl_parse_and_bind errors do not fail the bind builtin
                // (bind.def:304-333 ignores the return value; GNU probe:
                // `bind '"\C-t'` prints
                // `readline: "\C-t: no closing `"' in key binding`, rc 0).
                Some(Err(_)) => {}
                // Other non-quote text is silently ignored (GNU probe:
                // `bind 'ls -l'` → rc 0, no output).
                None => {}
            }
        }
    }

    fn parse_directive(
        &mut self,
        rest: &str,
        number: usize,
        load: &mut dyn FnMut(&str) -> Option<String>,
    ) {
        if let Some(condition) = rest.strip_prefix("if") {
            let condition = condition.trim();
            let active = if let Some((key, value)) = condition.split_once('=') {
                match key.trim() {
                    // `$if term=…` compares $TERM verbatim (case
                    // sensitive, GNU probe: XTERM ≠ xterm).
                    "term" => self.env.term == Some(value),
                    // `$if mode=…` compares the CURRENT editing mode —
                    // a `set editing-mode` earlier in the file already
                    // updated env.editing_mode (GNU probe: the mode=vi
                    // block after `set editing-mode vi` applies).
                    "mode" => self.env.editing_mode == Some(value),
                    _ => false,
                }
            } else {
                self.diagnostic(
                    number,
                    "equality comparison operator expected, found `end-of-line'".to_string(),
                );
                false
            };
            self.conditionals.push(ConditionalRegion {
                active,
                taken: active,
            });
            return;
        }
        if rest == "else" {
            if let Some(region) = self.conditionals.last_mut() {
                region.active = !region.taken;
                region.taken |= region.active;
            }
            return;
        }
        if rest == "endif" {
            self.conditionals.pop();
            return;
        }
        if let Some(name) = rest.strip_prefix("include") {
            let name = name.trim();
            if self.active() && self.include_depth < 10 {
                if let Some(content) = load(name) {
                    let saved_file = std::mem::replace(&mut self.file, name.to_string());
                    self.include_depth += 1;
                    self.parse_text(&content, load);
                    self.include_depth -= 1;
                    self.file = saved_file;
                }
            }
        }
    }

    fn parse_set_line(&mut self, rest: &str, number: usize) {
        let rest = rest.trim_start();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or_default().trim();
        let value = parts.next().unwrap_or_default().trim();
        if name.is_empty() {
            return;
        }
        // Validate against the variable table now (the diagnostics carry
        // the file:line position GNU reports), store the normalized pair.
        let mut scratch: Vec<(String, String)> = Vec::new();
        let (outcome, editing_mode) = apply_readline_variable(&mut scratch, name, value);
        match outcome {
            SetVariableOutcome::Applied => {
                for (stored, normalized) in scratch {
                    store_variable(&mut self.result.variables, &stored, &normalized);
                }
                if let Some((mode_name, mode_value)) = editing_mode {
                    // Later `mode=` conditionals in this file see the new
                    // mode (GNU probe).
                    if mode_name == "editing-mode" {
                        self.env.editing_mode = Some(mode_value);
                    }
                }
            }
            SetVariableOutcome::UnknownVariable => {
                self.diagnostic(number, format!("{name}: unknown variable name"))
            }
            SetVariableOutcome::CouldNotSetValue => {
                self.diagnostic(number, format!("{name}: could not set value to `{value}'"))
            }
        }
    }
}

/// Parse an inputrc file body (`bind -f`): returns the keyseq bindings
/// and the `set editing-mode` value. Compatibility wrapper over
/// [`parse_inputrc_with_env`] with no term and no include loading.
pub fn parse_inputrc(text: &str) -> (Vec<(String, BindingSpec)>, Option<String>) {
    let parsed = parse_inputrc_with_env(text, "", &InputrcEnv::default(), &mut |_| None);
    (
        parsed.bindings,
        parsed
            .variables
            .iter()
            .find(|(name, _)| name == "editing-mode")
            .map(|(_, value)| value.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unix_command_spec_colon_form() {
        let (keyseq, command) =
            parse_unix_command_spec(r#""\C-g": echo bound-c-g"#).expect("parses");
        assert_eq!(keyseq, "\\C-g");
        assert_eq!(command, "echo bound-c-g");
    }

    #[test]
    fn parse_unix_command_spec_space_form_with_quotes() {
        let (keyseq, command) =
            parse_unix_command_spec(r#""\C-r" "fzf --query $READLINE_LINE""#).expect("parses");
        assert_eq!(keyseq, "\\C-r");
        assert_eq!(command, "fzf --query $READLINE_LINE");
    }

    #[test]
    fn parse_unix_command_spec_keeps_nested_quotes() {
        let (keyseq, command) =
            parse_unix_command_spec(r#""\C-g": sh -c "echo inner""#).expect("parses");
        assert_eq!(command, r#"sh -c "echo inner""#);
    }

    #[test]
    fn parse_unix_command_spec_errors_match_gnu() {
        let err = parse_unix_command_spec("noquotes").expect_err("rejected");
        assert_eq!(
            err.message,
            "noquotes: first non-whitespace character is not `\"'"
        );
        let err = parse_unix_command_spec(r#""\C-g"#).expect_err("rejected");
        assert!(err.message.contains("no closing"));
    }

    #[test]
    fn parse_binding_line_macro_and_function() {
        let parsed = parse_binding_line(r#""\C-t": "ins-macro""#)
            .expect("this form")
            .expect("parses");
        assert_eq!(parsed.keyseq, "\\C-t");
        assert_eq!(
            parsed.spec,
            BindingSpec::Macro {
                text: "ins-macro".to_string()
            }
        );
        let parsed = parse_binding_line(r#""\C-o": accept-line"#)
            .expect("this form")
            .expect("parses");
        assert_eq!(
            parsed.spec,
            BindingSpec::Function {
                name: "accept-line".to_string()
            }
        );
    }

    #[test]
    fn formats_match_gnu_probe_shapes() {
        assert_eq!(
            format_unix_command_readable("\\C-g", "echo bound-c-g"),
            "\"\\C-g\" \"echo bound-c-g\""
        );
        assert_eq!(
            format_macro_readable("\\C-t", "ins-macro"),
            "\"\\C-t\": \"ins-macro\""
        );
        assert_eq!(
            format_macro_plain("\\C-t", "ins-macro"),
            "\\C-t outputs ins-macro"
        );
        assert_eq!(
            format_function_plain("abort", &["\\C-g".to_string(), "\\C-x\\C-g".to_string()]),
            "abort can be found on \"\\C-g\", \"\\C-x\\C-g\"."
        );
        assert_eq!(
            format_query("self-insert", &[" ".to_string()]),
            "self-insert can be invoked via \" \"."
        );
        assert_eq!(
            format_query("self-insert", &[" ".to_string(), "!".to_string()]),
            "self-insert can be invoked via \" \", \"!\"."
        );
        assert_eq!(
            format_query(
                "self-insert",
                &[" ".to_string(), "!".to_string(), "?".to_string()]
            ),
            "self-insert can be invoked via \" \", \"!\", \"?\"."
        );
        assert_eq!(
            format_query_unbound("vi-editing-mode"),
            "vi-editing-mode is not bound to any keys."
        );
    }

    #[test]
    fn query_truncates_at_five_with_ellipsis() {
        let seqs: Vec<String> = (0..7).map(|i| format!("\\C-{i}")).collect();
        let out = format_query("self-insert", &seqs);
        assert!(out.ends_with(", ..."), "got: {out}");
        assert!(out.contains("\\C-4"));
        assert!(!out.contains("\\C-5"), "only five shown: {out}");
    }

    #[test]
    fn inputrc_parsing_extracts_bindings_and_editing_mode() {
        let text = "\n# comment\n\"\\C-k\": kill-whole-line\n\"\\C-t\": \"macro text\"\nset editing-mode vi\nset blink-matching-paren off\n$if term=xterm\n\"\\C-x\": beginning-of-line\n$endif\n";
        let (bindings, mode) = parse_inputrc(text);
        // `$if term=xterm` with no TERM in the wrapper env: the block is
        // skipped (GNU probe: term conditions compare $TERM verbatim).
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].0, "\\C-k");
        assert_eq!(mode, Some("vi".to_string()));
    }

    #[test]
    fn inputrc_term_condition_applies_when_term_matches() {
        let text = "$if term=xterm\n\"\\C-x\": beginning-of-line\n$else\n\"\\C-y\": yank\n$endif\n";
        let env = InputrcEnv {
            term: Some("xterm"),
            editing_mode: None,
        };
        let parsed = parse_inputrc_with_env(text, "", &env, &mut |_| None);
        assert_eq!(parsed.bindings.len(), 1);
        assert_eq!(parsed.bindings[0].0, "\\C-x");
        // $else branch when the term does not match (case-sensitive: GNU
        // probe `term=XTERM` does not match TERM=xterm).
        let env = InputrcEnv {
            term: Some("XTERM"),
            editing_mode: None,
        };
        let parsed = parse_inputrc_with_env(text, "", &env, &mut |_| None);
        assert_eq!(parsed.bindings.len(), 1);
        assert_eq!(parsed.bindings[0].0, "\\C-y");
    }

    #[test]
    fn inputrc_mode_condition_follows_in_file_set() {
        let text = "set editing-mode vi\n$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n";
        let parsed = parse_inputrc_with_env(text, "", &InputrcEnv::default(), &mut |_| None);
        // The `set editing-mode vi` earlier in the file arms the later
        // mode= conditional (GNU probe).
        assert_eq!(parsed.bindings.len(), 1);
        assert_eq!(parsed.variables[0].0, "editing-mode");
        assert_eq!(parsed.variables[0].1, "vi");
        assert_eq!(parsed.variables[1].0, "keymap");
        assert_eq!(parsed.variables[1].1, "vi-insert");
    }

    #[test]
    fn inputrc_set_variables_validate_and_normalize() {
        let text = "set completion-ignore-case on\nset completion-ignore-case maybe\nset keyseq-timeout abc\nset no-such-var on\nset editing-mode bogus\nset editing-mode VI\n";
        let parsed = parse_inputrc_with_env(text, "rc", &InputrcEnv::default(), &mut |_| None);
        // Non-`on` booleans normalize to `off`, garbage numbers to 0
        // (GNU probes: silent). `editing-mode VI` lowercases.
        let value = |name: &str| {
            parsed
                .variables
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        assert_eq!(value("completion-ignore-case"), "off");
        assert_eq!(value("keyseq-timeout"), "0");
        assert_eq!(value("editing-mode"), "vi");
        // Diagnostics: the unknown name and the bad editing-mode value
        // (`readline: rc: line N: …` shaped by the caller).
        assert_eq!(parsed.diagnostics.len(), 2);
        assert_eq!(parsed.diagnostics[0].line, 4);
        assert_eq!(
            parsed.diagnostics[0].core,
            "no-such-var: unknown variable name"
        );
        assert_eq!(parsed.diagnostics[1].line, 5);
        assert_eq!(
            parsed.diagnostics[1].core,
            "editing-mode: could not set value to `bogus'"
        );
    }

    #[test]
    fn inputrc_include_and_nesting() {
        let text = "$if term=vt100\n$if term=vt100\n\"\\C-a\": beginning-of-line\n$endif\n$endif\n\"\\C-b\": backward-char\n";
        let parsed = parse_inputrc_with_env(text, "", &InputrcEnv::default(), &mut |_| None);
        // Nested $if with no TERM: inactive; the outer binding applies.
        assert_eq!(
            parsed
                .bindings
                .iter()
                .map(|(seq, _)| seq.as_str())
                .collect::<Vec<_>>(),
            vec!["\\C-b"]
        );
    }

    #[test]
    fn variable_table_defaults_match_gnu_dump() {
        // `bind -v` order and defaults pinned against WSL GNU bash 5.3.0
        // (48 lines).
        assert_eq!(READLINE_VARIABLE_DEFAULTS.len(), 48);
        assert_eq!(
            READLINE_VARIABLE_DEFAULTS[0],
            ("bind-tty-special-chars", "off")
        );
        assert_eq!(READLINE_VARIABLE_DEFAULTS[35], ("visible-stats", "off"));
        assert_eq!(READLINE_VARIABLE_DEFAULTS[36], ("bell-style", "audible"));
        assert_eq!(
            READLINE_VARIABLE_DEFAULTS[47],
            ("vi-ins-mode-string", "(ins)")
        );
        let registry = crate::shell::bind_registry::BindRegistry::default();
        assert_eq!(registry.variable_value("editing-mode"), Some("emacs"));
        assert_eq!(registry.variable_value("keymap"), Some("emacs"));
    }

    #[test]
    fn set_variable_outcomes_match_gnu_probes() {
        let mut registry = crate::shell::bind_registry::BindRegistry::default();
        // `bind 'set completion-ignore-case on'` → visible in `bind -v`.
        assert_eq!(
            registry.set_readline_variable("completion-ignore-case", "on"),
            SetVariableOutcome::Applied
        );
        assert_eq!(
            registry.variable_value("completion-ignore-case"),
            Some("on")
        );
        // Unknown name: ``readline: no-such-var: unknown variable name``,
        // command keeps running.
        assert_eq!(
            registry.set_readline_variable("no-such-var", "on"),
            SetVariableOutcome::UnknownVariable
        );
        // Bad editing-mode value: loud, variable unchanged.
        assert_eq!(
            registry.set_readline_variable("editing-mode", "bogus"),
            SetVariableOutcome::CouldNotSetValue
        );
        assert_eq!(registry.variable_value("editing-mode"), Some("emacs"));
        // Valid vi switches editing-mode AND keymap (GNU probe).
        assert_eq!(
            registry.set_readline_variable("editing-mode", "vi"),
            SetVariableOutcome::Applied
        );
        assert_eq!(registry.variable_value("editing-mode"), Some("vi"));
        assert_eq!(registry.variable_value("keymap"), Some("vi-insert"));
        assert_eq!(registry.editing_mode.as_deref(), Some("vi"));
        // Bad keymap keeps the old value silently.
        assert_eq!(
            registry.set_readline_variable("keymap", "bogus"),
            SetVariableOutcome::Applied
        );
        assert_eq!(registry.variable_value("keymap"), Some("vi-insert"));
    }

    #[test]
    fn positional_errors_carry_readline_channel() {
        // Unclosed keyseq quote: `readline: …` text, no status effect.
        let err = parse_binding_line(r#""\C-t"#)
            .expect("this form")
            .expect_err("rejected");
        assert_eq!(
            err.readline_message.as_deref(),
            Some("\"\\C-t: no closing `\"' in key binding")
        );
        // Keyseq with no `: value`: no key sequence terminator.
        let err = parse_binding_line(r#""\C-t""#)
            .expect("this form")
            .expect_err("rejected");
        assert_eq!(
            err.readline_message.as_deref(),
            Some("\"\\C-t\": no key sequence terminator")
        );
        // Unclosed macro quote.
        let err = parse_binding_line(r#""\C-t": 'ins"#)
            .expect("this form")
            .expect_err("rejected");
        assert_eq!(
            err.readline_message.as_deref(),
            Some("'ins: missing closing quote for macro")
        );
        // -x errors stay in the `bind:` channel (rc 1 there).
        let err = parse_unix_command_spec("noquotes").expect_err("rejected");
        assert!(err.readline_message.is_none());
        // Non-quote non-set text: not this form at all.
        assert!(parse_binding_line("ls -l").is_none());
    }

    #[test]
    fn known_function_table_covers_common_names() {
        for name in [
            "self-insert",
            "accept-line",
            "beginning-of-line",
            "history-incremental-search-backward",
        ] {
            assert!(is_known_function(name), "{name} should be known");
        }
        assert!(!is_known_function("no-such-fn"));
    }
}
