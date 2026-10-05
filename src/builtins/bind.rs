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

/// Grammar failure of a `-x` or inputrc-form binding argument. `message`
/// is the GNU text without the `bind: ` prefix and without the shell's
/// `name: line N: ` diagnostic prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindSpecError {
    pub message: String,
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
            });
        }
        return Err(BindSpecError {
            message: format!("{line}: missing separator"),
        });
    }
    let delim = matches!(bytes[i], b'"' | b'\'').then(|| bytes[i]);
    if need_dquote && delim != Some(b'"') {
        return Err(BindSpecError {
            message: format!("{line}: first non-whitespace character is not `\"'"),
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
/// when the argument is not this form.
pub fn parse_binding_line(line: &str) -> Option<Result<ParsedBinding, BindSpecError>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('"') {
        return None;
    }
    let (keyseq_raw, mut i) = match isolate_sequence(line, 0, true) {
        Ok(parsed) => parsed,
        Err(err) => return Some(Err(err)),
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
        // isolate_sequence auto-detects the quote character (GNU passes
        // need_dquote=0 here; either quote works as the macro delimiter).
        let (text_raw, _) = match isolate_sequence(line, i, false) {
            Ok(parsed) => parsed,
            Err(err) => return Some(Err(err)),
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
            Err(err) => return Some(Err(err)),
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
        None => Err(BindSpecError {
            message: format!("{arg}: missing separator"),
        }),
    }
}

/// Parse an inputrc file body (`bind -f`): returns the keyseq bindings
/// and the `set editing-mode` value. `$if`/`$endif`/`$else`/`$include`
/// and unrecognized `set` lines are tolerated (skipped) — GNU applies
/// them through readline's variable table, which niu's editor only
/// implements for editing-mode; documented gap (wt100 matrix, row
/// inputrc).
pub fn parse_inputrc(text: &str) -> (Vec<(String, BindingSpec)>, Option<String>) {
    let mut bindings = Vec::new();
    let mut editing_mode = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('$') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("set ") {
            let mut parts = rest.split_whitespace();
            let key = parts.next().unwrap_or_default();
            let value = parts.next().unwrap_or_default();
            if key == "editing-mode" {
                editing_mode = Some(value.to_string());
            }
            continue;
        }
        match parse_binding_line(line) {
            Some(Ok(parsed)) => bindings.push((parsed.keyseq, parsed.spec)),
            _ => continue,
        }
    }
    (bindings, editing_mode)
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
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].0, "\\C-k");
        assert_eq!(mode, Some("vi".to_string()));
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
