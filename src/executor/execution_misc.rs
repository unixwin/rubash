use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

/// GNU general.c:1019 printable_filename: if the string contains non-
/// printable characters, return it as a $'...' ANSI-C quoted form; otherwise
/// return it unchanged. Used by execute_disk_command (execute_cmd.c:5909)
/// before printing "command not found" so a CR-only command name shows as
/// `$'\r'` instead of a raw control character.
pub(crate) fn printable_filename(name: &str) -> String {
    if word_needs_ansic_quote(name) {
        ansic_quote_with_markers(name)
    } else {
        name.to_string()
    }
}

/// True when the word contains non-printable characters OR raw-byte markers
/// (U+E000 series). GNU strtrans.c:341 ansic_shouldquote tests printability
/// in the active locale; Rubash additionally must quote when internal
/// raw-byte markers are present so they don't leak as PUA chars.
pub(crate) fn word_needs_ansic_quote(word: &str) -> bool {
    if word.contains(char::from_u32(super::substitution_metadata::RAW_BYTE_MARKER_ESCAPE).unwrap())
    {
        return true;
    }
    word.chars().any(|ch| {
        if ch.is_ascii() {
            !(0x20..=0x7e).contains(&(ch as u8))
        } else {
            ch.is_control()
        }
    })
}

/// strtrans.c ansic_quote (230-308) over the decoded byte stream: raw-byte
/// markers (U+E000 series) are decoded back to their original bytes before
/// quoting, so diagnostics show `$'\247\100...'` instead of PUA replacement
/// characters. Printable ASCII and printable wide chars stay literal.
pub(crate) fn ansic_quote_with_markers(word: &str) -> String {
    let bytes = super::substitution_metadata::shell_text_to_raw_bytes(word);
    let mut out = String::with_capacity(4 * bytes.len() + 4);
    out.push_str("$'");
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            0x1b => {
                out.push_str("\\E");
                index += 1;
            }
            0x07 => {
                out.push_str("\\a");
                index += 1;
            }
            0x08 => {
                out.push_str("\\b");
                index += 1;
            }
            0x09 => {
                out.push_str("\\t");
                index += 1;
            }
            0x0a => {
                out.push_str("\\n");
                index += 1;
            }
            0x0b => {
                out.push_str("\\v");
                index += 1;
            }
            0x0c => {
                out.push_str("\\f");
                index += 1;
            }
            0x0d => {
                out.push_str("\\r");
                index += 1;
            }
            b'\\' => {
                out.push_str("\\\\");
                index += 1;
            }
            b'\'' => {
                out.push_str("\\'");
                index += 1;
            }
            0x20..=0x7e => {
                out.push(byte as char);
                index += 1;
            }
            _ if byte >= 0x80 => {
                // Try to decode a full UTF-8 character (GNU strtrans.c:266-282
                // emits a printable wide char verbatim under a UTF-8 locale).
                match std::str::from_utf8(&bytes[index..]) {
                    Ok(text) => {
                        let ch = text.chars().next().unwrap();
                        if !ch.is_control() {
                            out.push(ch);
                            index += ch.len_utf8();
                        } else {
                            push_octal_escape(&mut out, byte);
                            index += 1;
                        }
                    }
                    Err(error) => {
                        let valid = error.valid_up_to();
                        if valid > 0 {
                            let text = std::str::from_utf8(&bytes[index..index + valid]).unwrap();
                            for ch in text.chars() {
                                if !ch.is_control() {
                                    out.push(ch);
                                } else {
                                    for b in ch.encode_utf8(&mut [0u8; 4]).as_bytes() {
                                        push_octal_escape(&mut out, *b);
                                    }
                                }
                            }
                            index += valid;
                        } else {
                            push_octal_escape(&mut out, byte);
                            index += 1;
                        }
                    }
                }
            }
            _ => {
                push_octal_escape(&mut out, byte);
                index += 1;
            }
        }
    }
    out.push('\'');
    out
}

fn push_octal_escape(out: &mut String, byte: u8) {
    out.push('\\');
    out.push((b'0' + ((byte >> 6) & 0o7)) as char);
    out.push((b'0' + ((byte >> 3) & 0o7)) as char);
    out.push((b'0' + (byte & 0o7)) as char);
}

pub(in crate::executor) fn is_arithmetic_command_words(words: &[String]) -> bool {
    matches!(words, [open, _, close] if open == "((" && close == "))")
}

pub(in crate::executor) fn echo_args_without_background_marker(args: &[String]) -> Vec<String> {
    // TODO(parse.y/jobs.c): `&` is a command terminator that launches the
    // preceding command asynchronously. The parser now consumes a source-level
    // `&` (token_actions sets CommandNode::background), so args ending in `&`
    // are field-split expansion data (e.g. echo ${s//?/\\& }) and must be
    // printed. Keep the hook for legacy word shapes that still carry `&`.
    args.to_vec()
}

pub(in crate::executor) fn is_null_device(path: &str) -> bool {
    crate::executor::path::is_shell_null_device(path)
}

pub(in crate::executor) fn is_closed_redirect_target(path: &str) -> bool {
    path == "&-"
}

pub(in crate::executor) fn redirect_target_fd(target: &str) -> Option<u32> {
    redirect_target_fd_and_move(target).and_then(|(fd, move_fd)| (!move_fd).then_some(fd))
}

pub(in crate::executor) fn redirect_target_fd_and_move(target: &str) -> Option<(u32, bool)> {
    let target = target.trim_start_matches([
        crate::executor::markers::QUOTED_WORD_PREFIX,
        STORAGE_WORD_PREFIX,
    ]);
    let Some(fd) = target.strip_prefix('&') else {
        return dev_stdio_redirect_fd(target).map(|fd| (fd, false));
    };
    let fd = fd.trim_matches(|ch| ch == '"' || ch == STORAGE_WORD_PREFIX);
    let (fd, move_fd) = fd
        .strip_suffix('-')
        .map(|fd| (fd, true))
        .unwrap_or((fd, false));
    (!fd.is_empty() && fd.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| fd.parse::<u32>().ok().map(|fd| (fd, move_fd)))
        .flatten()
}

/// GNU redir.c:1401 stdin_redirection + :1435 stdin_redirects — whether a
/// redirection alters the standard input for the async-stdin decision
/// (execute_cmd.c:828). Plain input forms (`<`, `<<`, `<<<`, `<>`) count at
/// ANY redirector fd — `3<file` sets the flag too; `N<&M` dups and `N<&-`
/// closes count only when N is 0; `N<&M-` moves and output forms never do.
/// REDIR_VARASSIGN (`{var}<f`) entries are skipped like the C code does.
pub(in crate::executor) fn redirect_updates_stdin_redir(
    redirect: &crate::parser::Redirect,
) -> bool {
    if redirect.fd_var.is_some() {
        return false;
    }
    use crate::parser::RedirectKind;
    match redirect.kind {
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereString => true,
        RedirectKind::DuplicateInput => {
            if redirect_target_fd_and_move(&redirect.target).is_some_and(|(_, move_fd)| move_fd) {
                false
            } else {
                redirect.fd.unwrap_or(0) == 0
            }
        }
        RedirectKind::CloseInput => redirect.fd.unwrap_or(0) == 0,
        _ => false,
    }
}

/// GNU execute_cmd.c:474 shell_control_structure — the command types whose
/// redirects update the global stdin_redir (execute_cmd.c:828). `( )`
/// subshells and coprocs are deliberately absent: a subshell recomputes
/// stdin_redir from its own redirects inside execute_in_subshell
/// (execute_cmd.c:1733) instead of feeding the parent's flag.
pub(in crate::executor) fn command_is_shell_control_structure(cmd: &CommandNode) -> bool {
    cmd.for_command.is_some()
        || cmd.arithmetic_command.is_some()
        || cmd.if_command.is_some()
        || cmd.loop_command.is_some()
        || cmd.select_command.is_some()
        || cmd.case_command.is_some()
        || cmd.conditional_command.is_some()
        || cmd.brace_group.is_some()
        || cmd.function_command.is_some()
}

/// GNU redir.c opens `/dev/stdin`/`/dev/stdout`/`/dev/stderr`,
/// `/dev/fd/N`, and `/proc/self/fd/N` through the OS's fd-alias device
/// files, which the kernel resolves to a dup of fd N — behaviorally the
/// same as `>&N`/`<&N`. Windows has no such filesystem, so the names are
/// recognized here and flow through the same fd-dup machinery (niubash#118:
/// `>> /dev/stdout` used to land on the CONOUT$ device path and fail with
/// Permission denied).
pub(in crate::executor) fn dev_stdio_redirect_fd(target: &str) -> Option<u32> {
    match target {
        "/dev/stdin" => return Some(0),
        "/dev/stdout" => return Some(1),
        "/dev/stderr" => return Some(2),
        _ => {}
    }
    let fd = target
        .strip_prefix("/dev/fd/")
        .or_else(|| target.strip_prefix("/proc/self/fd/"))?;
    (!fd.is_empty() && fd.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| fd.parse::<u32>().ok())
        .flatten()
}

pub(in crate::executor) fn redirect_target_is_ambiguous(raw: &str, expanded: &str) -> bool {
    if !expanded.chars().any(char::is_whitespace) {
        return false;
    }

    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut has_unquoted_expansion = false;

    for ch in raw.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single_quoted {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            '$' if !single_quoted && !double_quoted => {
                has_unquoted_expansion = true;
            }
            _ => {}
        }
    }

    has_unquoted_expansion
}

pub(in crate::executor) fn stdio_output_target(fd: u32) -> Option<&'static str> {
    match fd {
        1 => Some(FD_STDOUT_TARGET),
        2 => Some(FD_STDERR_TARGET),
        _ => None,
    }
}

pub(in crate::executor) fn command_has_unterminated_heredoc(cmd: &CommandNode) -> bool {
    cmd.heredoc
        .as_deref()
        .is_some_and(|body| strip_quoted_heredoc_marker(body).starts_with(DATA_DOLLAR))
}

/// True when the here-document delimiter was found but not on a line by
/// itself (e.g. `EOF)` inside a command substitution).  GNU make_cmd.c:606-627
/// sets `full_line = 0` via the PST_EOFTOKEN path and issues the same warning
/// as a truly unterminated heredoc.  The `\x1e` marker is inserted by the
/// lexer for this case.
pub(in crate::executor) fn command_has_warned_heredoc(cmd: &CommandNode) -> bool {
    cmd.heredoc.as_deref().is_some_and(|body| {
        strip_quoted_heredoc_marker(body)
            .starts_with(crate::executor::markers::HEREDOC_WARNED_BODY_PREFIX)
    })
}

pub(in crate::executor) fn strip_unterminated_heredoc_marker(body: &str) -> &str {
    let stripped = body
        .strip_prefix(DATA_DOLLAR)
        .or_else(|| body.strip_prefix(crate::executor::markers::HEREDOC_WARNED_BODY_PREFIX));
    match stripped {
        Some(s) => s,
        None => body,
    }
}

pub(in crate::executor) fn strip_quoted_heredoc_marker(body: &str) -> &str {
    body.strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
        .unwrap_or(body)
}

/// Marks a here-document body or here-string word already expanded by the
/// command dispatch (execute_command). GNU expands them inside
/// do_redirections — after word expansion, before the command runs — so
/// the executor expands them at the same point and stores the result with the
/// StdinBody::Preexpanded typed carrier; the stdin paths return it verbatim
/// instead of re-running embedded substitutions a second time. A raw 0x05 CAN
/// appear at the start of a user heredoc body or here-string word (script
/// files are arbitrary byte streams), so the parser encodes such bytes as
/// raw-byte marker pairs at collection time (parser/redirections.rs
/// encode_stdin_body_enq) and the expand/emit boundary decodes them back
/// (decode_stdin_body_enq).
pub(in crate::executor) const PREEXPANDED_STDIN_BODY: char =
    crate::executor::markers::PREEXPANDED_STDIN_BODY;

/// Returns the pre-expanded text when `body` carries
/// PREEXPANDED_STDIN_BODY (legacy sentinel check for parser/compat paths).
/// New code should use StdinBody::Preexpanded typed carrier instead.
pub(in crate::executor) fn preexpanded_stdin_body(body: &str) -> Option<&str> {
    body.strip_prefix(PREEXPANDED_STDIN_BODY)
}

/// Returns the pre-expanded text from a StdinBody typed carrier.
pub(in crate::executor) fn stdin_body_carrier_to_text(
    carrier: &Option<crate::parser::StdinBody>,
) -> Option<String> {
    match carrier {
        Some(crate::parser::StdinBody::Preexpanded(text)) => Some(text.clone()),
        Some(crate::parser::StdinBody::NeedsExpansion(_)) => None,
        None => None,
    }
}

/// Inverse of parser encode_stdin_body_enq: once stdin text leaves the
/// heredoc/here-string transport fields, a raw-byte marker pair for 0x05
/// decodes back to the literal ENQ char — the canonical in-value
/// representation, since 0x05 is not a carrier byte. Other marker pairs
/// (carriers, >=0x80 bytes) must stay encoded, so only the 0x05 pair is
/// touched.
pub(in crate::executor) fn decode_stdin_body_enq(text: &str) -> String {
    if !text.contains(
        char::from_u32(crate::executor::markers::RAW_BYTE_MARKER_ESCAPE)
            .expect("sentinel is valid"),
    ) {
        return text.to_string();
    }
    let pair = [
        char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
            .expect("sentinel is valid"),
        char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_FIRST + 0x05)
            .expect("marker char is valid"),
    ]
    .iter()
    .collect::<String>();
    text.replace(&pair, "\u{5}")
}

pub(in crate::executor) fn unterminated_heredoc_body_line_count(body: &str) -> usize {
    let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
    body.lines().count()
}

pub(in crate::executor) fn copy_command_substitution_heredoc(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    source: &mut String,
) {
    source.push('<');
    source.push('<');
    chars.next();

    let strip_tabs = if chars.peek().copied() == Some('-') {
        source.push('-');
        chars.next();
        true
    } else {
        false
    };

    while chars.peek().is_some_and(|ch| matches!(ch, ' ' | '\t')) {
        let ch = chars.next().unwrap();
        source.push(ch);
    }

    let mut raw_delimiter = String::new();
    // GNU read_token_word: quoting inside the delimiter word makes
    // metacharacters literal — `<< ')'` names `)` as the delimiter, so a
    // quoted `)` (or `;`, `|`, `&`) is delimiter text, not the
    // substitution closer (comsub-posix.tests).
    let mut delimiter_single = false;
    let mut delimiter_double = false;
    while let Some(next) = chars.peek().copied() {
        match next {
            '\'' if !delimiter_double => delimiter_single = !delimiter_single,
            '"' if !delimiter_single => delimiter_double = !delimiter_double,
            _ if !delimiter_single
                && !delimiter_double
                && (next.is_whitespace() || matches!(next, ';' | '|' | '&' | ')')) =>
            {
                break;
            }
            // A backslash quotes the next delimiter byte (`<<\)` uses a
            // literal `)` delimiter); consume the escape pair as one unit.
            '\\' if !delimiter_single && !delimiter_double => {
                let ch = chars.next().unwrap();
                raw_delimiter.push(ch);
                source.push(ch);
                if let Some(escaped) = chars.peek().copied() {
                    chars.next();
                    raw_delimiter.push(escaped);
                    source.push(escaped);
                }
                continue;
            }
            _ => {}
        }
        let ch = chars.next().unwrap();
        raw_delimiter.push(ch);
        source.push(ch);
    }
    let mut delimiter = raw_delimiter.replace(['\'', '"', '\\'], "");
    if strip_tabs {
        delimiter = delimiter.trim_start_matches('\t').to_string();
    }
    if delimiter.is_empty() {
        return;
    }

    while let Some(ch) = chars.next() {
        source.push(ch);
        if ch == '\n' {
            break;
        }
    }

    loop {
        let mut line = String::new();
        while let Some(ch) = chars.peek().copied() {
            let comparable = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            };
            // GNU make_cmd.c:605-611 (PST_EOFTOKEN): any body line that
            // starts with the delimiter and reaches `)` later ends the
            // document; the `)` is left for the caller so the command
            // substitution sees its closer (`EOFx)` counts too -- the `x`
            // stays in the collected source).
            if comparable.starts_with(delimiter.as_str()) && ch == ')' {
                source.push(crate::executor::markers::IFS_GLUE);
                return;
            }
            if ch == '\n' {
                break;
            }
            chars.next();
            line.push(ch);
            source.push(ch);
        }

        let comparable = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if comparable == delimiter {
            if chars.peek().copied() == Some('\n') {
                source.push('\n');
                chars.next();
            }
            return;
        }

        match chars.next() {
            Some('\n') => source.push('\n'),
            Some(ch) => source.push(ch),
            None => return,
        }
    }
}

pub(in crate::executor) fn contains_windows_forbidden_posix_filename_char(path: &str) -> bool {
    path.chars()
        .any(|ch| matches!(ch, '*' | '?' | '<' | '>' | '|'))
}

pub(in crate::executor) fn word_has_unquoted_command_substitution(word: &str) -> bool {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double && ch == '`' {
            return true;
        }
        if !single && !double && ch == '$' && chars.get(index + 1) == Some(&'(') {
            return true;
        }
        if !single && !double && ch == '$' && chars.get(index + 1) == Some(&'{') {
            // Bash 5.3 (parser.h FUNSUB_CHAR): a whitespace-led `${ command; }`
            // is a nofork command substitution. Its unquoted output word-splits
            // and vanishes when empty, exactly like $() output
            // (comsub2.tests: `echo ${ printf '%s\n' aa bb; }` -> one line).
            // The `${| command; }` funsub form keeps field integrity and must
            // not trigger splitting here.
            if chars
                .get(index + 2)
                .is_some_and(|next| next.is_whitespace())
            {
                return true;
            }
        }
        index += 1;
    }
    false
}

pub(in crate::executor) fn for_word_has_unquoted_expansion(word: &str, raw: Option<&str>) -> bool {
    if word.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
        || word.starts_with(STORAGE_WORD_PREFIX)
    {
        return false;
    }
    let source = raw.unwrap_or(word);
    word_has_unquoted_parameter_expansion(source) || word_has_unquoted_command_substitution(source)
}

fn word_has_unquoted_parameter_expansion(word: &str) -> bool {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double && ch == '$' {
            match chars.get(index + 1).copied() {
                Some('{') => return true,
                Some(next) if is_shell_name_start(next) => return true,
                Some(next)
                    if next.is_ascii_digit()
                        || matches!(next, '@' | '*' | '#' | '?' | '$' | '!' | '-') =>
                {
                    return true;
                }
                _ => {}
            }
        }
        index += 1;
    }
    false
}

pub(in crate::executor) fn bash_aliases_assignment_name(word: &str) -> Option<String> {
    // TODO(variables.c/alias.c): BASH_ALIASES is a dynamic associative array
    // backed by the alias table. This narrow path reports invalid alias names
    // for upstream alias.tests.
    let rest = word.strip_prefix("BASH_ALIASES[")?;
    let (name, _) = rest.split_once("]=")?;
    Some(name.trim_matches('\'').to_string())
}

pub(in crate::executor) fn valid_alias_assignment_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '/' | '$' | '`' | '"' | '\'' | '\\' | '(' | ')' | '<' | '>' | '&' | '|'
                )
        })
}

pub(in crate::executor) fn shell_display_path(path: &str) -> String {
    if cfg!(windows) {
        let path = path.strip_prefix("//?/").unwrap_or(path);
        let path = crate::executor::path::shell_path_display_from_windows(path);
        // GNU process_substitute (subst.c) emits forward-slash paths
        // (/dev/fd/N, or sh_mktmpname /-separated names) so the substituted
        // word survives eval/source re-parsing: a literal backslash would be
        // consumed as a shell escape. Normalize Windows backslashes to slashes
        // before the drive-letter check so bytes[2]=='/' fires too.
        let path = path.replace('\\', "/");
        return windows_native_to_slash_drive_display(&path);
    }
    path.to_string()
}

fn windows_native_to_slash_drive_display(path: &str) -> String {
    if path.len() >= 3 && path.as_bytes()[1] == b':' && path.as_bytes()[2] == b'/' {
        let drive = (path.as_bytes()[0] as char).to_ascii_lowercase();
        return format!("/{drive}/{}", &path[3..]);
    }
    path.to_string()
}

pub(in crate::executor) fn current_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(in crate::executor) fn current_epoch_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_micros() as i64)
        .unwrap_or(0)
}

pub(in crate::executor) fn eval_source_for_reparse(source: &str) -> String {
    let source = source
        .replace(crate::lexer::QUOTED_HEREDOC_MARKER, "")
        .replace(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER, "")
        .replace(crate::executor::markers::IFS_GLUE, "")
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::DATA_SQUOTE, "'")
        // GNU expand_word_internal's single-quote arm (subst.c:11882) takes the
        // region body from string_extract_single_quoted (subst.c:1088), which
        // only substrings the raw text, and then calls remove_quoted_escapes
        // (subst.c:11902 -> 4901 -> dequote_escapes:4692), which strips just
        // CTLESC-CTLESC and CTLESC-CTLNUL pairs. A `"` written inside single
        // quotes therefore reaches eval as a bare quote character, and eval
        // re-reads its argument as parser input (builtins/eval.def:49 ->
        // evalstring -> parse_and_execute) where that bare quote delimits
        // again. Rubash carries the same character as the data-double-quote
        // marker in the word value, so it has to be rendered back to source
        // here; otherwise eval re-parses the marker as literal data.
        // niubash #124: `eval 'd='\''x'\''; mkdir -p "$d"'` handed the quotes
        // to the child (`mkdir: cannot create directory '"/x"'`).
        .replace(crate::executor::markers::DATA_DQUOTE, "\"")
        .replace(crate::lexer::ANSI_C_QUOTE_MARKER_STR, "'")
        .replace(crate::lexer::ANSI_C_DQUOTE_MARKER_STR, "\"");
    protect_unmatched_double_quoted_backticks(&source)
}

fn protect_unmatched_double_quoted_backticks(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.char_indices().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while let Some((_index, ch)) = chars.next() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            output.push(ch);
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                output.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                output.push(ch);
            }
            '`' if in_double && !in_single => output.push(crate::executor::markers::DATA_BACKTICK),
            _ => output.push(ch),
        }
    }
    output
}

/// GNU lib/sh/random.c state: `rseed` is RANDOM's Park-Miller seed
/// (random.c:51), `last_value` backs get_random_number's resample-on-repeat
/// loop (variables.c:1428-1431, reset to 0 by sbrand on every `RANDOM=`
/// assignment), and `rseed32` is SRANDOM's separate seed (random.c:132) so
/// SRANDOM draws never perturb the RANDOM sequence.
#[derive(Debug, Clone)]
pub(crate) struct RandomGen {
    pub rseed: Cell<u32>,
    pub last_value: Cell<u32>,
    pub rseed32: Cell<u32>,
}

impl RandomGen {
    /// GNU variables.c:663-664 seeds both generators from genseed()
    /// (time/uid/pid mix) at startup; epoch microseconds is the rubash
    /// equivalent of that entropy.
    pub(crate) fn seeded() -> Self {
        let seed = current_epoch_micros() as u32;
        Self {
            rseed: Cell::new(seed),
            last_value: Cell::new(0),
            rseed32: Cell::new(seed ^ 0x9e37_79b9),
        }
    }

    /// Command/process substitution clones carry the seeds over (GNU
    /// reseeds from genseed on pid change, which rubash cannot observe).
    pub(crate) fn clone_state(&self) -> Self {
        Self {
            rseed: Cell::new(self.rseed.get()),
            last_value: Cell::new(self.last_value.get()),
            rseed32: Cell::new(self.rseed32.get()),
        }
    }
}

/// GNU lib/sh/random.c:57-78 intrand32 — the Park-Miller "minimal standard"
/// generator x(n+1) = 16807*x(n) mod 2147483647 with Schrage splitting
/// (q=127773, r=2836); a zero seed is replaced by 123459876.
fn intrand32(last: u32) -> u32 {
    let ret = if last == 0 { 123_459_876u32 } else { last };
    let h = ret / 127_773;
    let l = ret % 127_773;
    let t = 16_807i64 * i64::from(l) - 2_836i64 * i64::from(h);
    if t < 0 {
        (t + 0x7fff_ffff) as u32
    } else {
        t as u32
    }
}

/// GNU lib/sh/random.c:99-113 brand() + variables.c:1424-1431
/// get_random_number: fold the 31-bit seed to 15 bits via
/// `(rseed >> 16) ^ (rseed & 65535)` (the shell_compatibility_level > 50
/// path) and resample while the draw repeats the previous value.
pub(in crate::executor) fn next_random_from_state(state: &RandomGen) -> u32 {
    loop {
        let rseed = intrand32(state.rseed.get());
        state.rseed.set(rseed);
        let ret = ((rseed >> 16) ^ (rseed & 65_535)) & 32_767;
        if ret != state.last_value.get() {
            state.last_value.set(ret);
            return ret;
        }
    }
}

/// GNU lib/sh/random.c:140-146 brand32. SRANDOM on Linux reads
/// getrandom(2) (random.c:226-240 get_urandom32) so its values are entropy
/// and unseedable; this deterministic fallback only needs to stay off
/// RANDOM's rseed.
pub(in crate::executor) fn next_srandom_from_state(state: &RandomGen) -> u32 {
    let rseed = intrand32(state.rseed32.get());
    state.rseed32.set(rseed);
    rseed & 0x7fff_ffff
}

pub(in crate::executor) fn strip_shebang(source: &str) -> &str {
    source
        .strip_prefix("#!")
        .and_then(|rest| rest.split_once('\n').map(|(_, body)| body))
        .unwrap_or(source)
}

pub(in crate::executor) fn command_substitution_word_split(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

const COMMAND_SUBSTITUTION_PAYLOAD_PREFIX: &str = crate::executor::markers::COMSUB_PAYLOAD_PREFIX;

pub(in crate::executor) fn contains_command_substitution_payload(value: &str) -> bool {
    value.contains(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX)
}

pub(in crate::executor) fn command_substitution_value_needs_payload_protection(
    source: &str,
    value: &str,
) -> bool {
    source.contains('$')
        && !source.contains('`')
        && !value.contains(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX)
        && value.chars().any(|ch| {
            (crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER..=DATA_DOLLAR).contains(&ch)
        })
}

pub(in crate::executor) fn protect_command_substitution_output(value: &str) -> String {
    let escaped_value = value.replace(
        COMMAND_SUBSTITUTION_PAYLOAD_PREFIX,
        &format!("{COMMAND_SUBSTITUTION_PAYLOAD_PREFIX}{COMMAND_SUBSTITUTION_PAYLOAD_PREFIX}"),
    );
    let mut output = String::with_capacity(escaped_value.len());
    for ch in escaped_value.chars() {
        match ch {
            crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER..=DATA_DOLLAR => output.push_str(
                &format!("{COMMAND_SUBSTITUTION_PAYLOAD_PREFIX}{:02x};", ch as u32),
            ),
            '`' => output.push(crate::executor::markers::DATA_BACKTICK),
            '$' => output.push(DATA_DOLLAR),
            '\\' => output.push(crate::executor::markers::PROTECTED_BACKSLASH),
            _ => output.push(ch),
        }
    }
    output
}

pub(in crate::executor) fn restore_command_substitution_output(value: &str) -> String {
    value
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
        .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
}

/// The command-substitution dispatchers return transport text; a body that
/// ran the real parser/executor path can still carry marker pairs and C0
/// DATA_* sentinels. A consumer materializing final text (heredoc bodies,
/// embedded expansion) must decode to visible text — subst.c
/// command_substitute yields the command's raw stdout bytes — before
/// re-protecting for its own downstream boundary, or the carriers leak
/// into the output (e.g. `cat <<EOF` with `` `echo '\`'` ``).
pub(in crate::executor) fn substitution_result_visible_text(value: &str) -> String {
    // The dispatcher's output can nest the raw-byte marker pair inside
    // literal-char escapes (E400+E000/E4xx), which the char-level decoder
    // resolves to a live E000+payload pair again.
    //
    // Ordering matters: restore transport carriers FIRST — a bare C0 carrier
    // char in `value` is syntax (DATA_BACKTICK -> `, PROTECTED_BACKSLASH ->
    // \). Only then decode marker pairs; a decoded pair byte is DATA and must
    // be re-tagged by bytes_to_shell_text so no later carrier restore claims
    // it (unicode1.sub `$(printf '\x15')` in a heredoc body printed `\` —
    // the decoded 0x15 was read as PROTECTED_BACKSLASH).
    let restored = restore_command_substitution_output(value);
    let visible = crate::locale::decode_to_visible_text(&restored);
    let bytes = crate::executor::substitution_metadata::decode_raw_byte_markers(visible.as_bytes());
    crate::executor::substitution_metadata::bytes_to_shell_text(&bytes)
}

pub(in crate::executor) fn decode_command_substitution_payload(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX) {
        output.push_str(&rest[..index]);
        rest = &rest[index + COMMAND_SUBSTITUTION_PAYLOAD_PREFIX.len()..];
        if let Some(escaped) = rest.strip_prefix(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX) {
            output.push_str(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX);
            rest = escaped;
        } else if rest.len() >= 3 && rest.as_bytes()[2] == b';' {
            if let Ok(byte) = u8::from_str_radix(&rest[..2], 16) {
                output.push(char::from(byte));
                rest = &rest[3..];
            } else {
                output.push_str(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX);
            }
        } else {
            output.push_str(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX);
        }
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod command_substitution_payload_tests {
    use super::decode_command_substitution_payload;

    #[test]
    fn decodes_c0_payload_without_utf8_loss() {
        assert_eq!(
            decode_command_substitution_payload("a__RUBASH_CSB1_15;b"),
            format!(
                "{}{}{}",
                "a",
                crate::executor::markers::PROTECTED_BACKSLASH_STR,
                "b"
            )
        );
    }

    #[test]
    fn preserves_escaped_payload_prefix() {
        assert_eq!(
            decode_command_substitution_payload("__RUBASH_CSB1___RUBASH_CSB1_"),
            "__RUBASH_CSB1_"
        );
    }

    #[test]
    fn leaves_malformed_payload_literal() {
        assert_eq!(
            decode_command_substitution_payload("x__RUBASH_CSB1_no_semicolon"),
            "x__RUBASH_CSB1_no_semicolon"
        );
    }
}
