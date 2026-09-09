//! History expansion engine (the event/word-designator/modifier language).
//!
//! GNU Bash source ownership (vendored 5.3 sources):
//! - lib/readline/histexpand.c: history_expand (pass 1 and pass 2),
//!   history_expand_internal, get_history_event, get_history_word_specifier,
//!   history_arg_extract, get_subst_pattern, postproc_subst_rhs,
//!   quote_breaks, hist_error, hist_string_extract_single_quoted,
//!   history_tokenize/_internal/_word, history_find_word
//! - bashhist.c: bash_history_inhibit_expansion, bash_history_no_expand_chars
//! - subst.c: skip_to_histexp (quoting/command-substitution aware scan)
//!
//! Expansion happens once on the raw text before the shell parses it, exactly
//! like GNU's pre_process_line call path. histchars maps to (expansion char,
//! quick-substitution char, comment char).

/// histexpand.c HISTORY_WORD_DELIMITERS.
pub const HISTORY_WORD_DELIMITERS: &str = " \t\n;&()|<>";
/// histexpand.c HISTORY_EVENT_DELIMITERS.
const HISTORY_EVENT_DELIMITERS: &str = "^$*%-";
/// bashhist.c bash_history_no_expand_chars (bash override of the library set).
const NO_EXPAND_CHARS: &str = " \t\n\r=;&|()<>";
/// bashhist.c bash_initialize_history: history_search_delimiter_chars.
const SEARCH_DELIMITER_CHARS: &str = ";&()|<>";
/// histexpand.c HISTORY_QUOTE_CHARACTERS (double quote, quote, backtick).
const HISTORY_QUOTE_CHARACTERS: &str = "\"'\u{60}";
/// histexpand.c slashify_in_quotes (backslash, backtick, quote, dollar).
const SLASHIFY_IN_QUOTES: &str = "\\\u{60}\"$";

const EVENT_NOT_FOUND: i32 = 1;
const BAD_WORD_SPEC: i32 = 2;
const SUBST_FAILED: i32 = 3;
const BAD_MODIFIER: i32 = 4;
const NO_PREV_SUBST: i32 = 5;

/// Backslash as a char constant.
const BS: char = '\\';
/// Backtick as a char constant (escape form for source hygiene).
const BT: char = '\u{60}';

/// Expansion state persisting across expansions within one session
/// (histexpand.c statics: search_string, search_match, subst_lhs, subst_rhs).
#[derive(Debug, Default)]
pub struct HistEngineState {
    pub search_string: Option<String>,
    pub search_match: Option<String>,
    pub subst_lhs: Option<String>,
    pub subst_rhs: Option<String>,
}

/// histchars=(expansion, quick-substitution, comment) — defaults ! ^ #.
#[derive(Debug, Clone, Copy)]
pub struct HistChars {
    pub expand: char,
    pub subst: char,
    pub comment: char,
}

impl Default for HistChars {
    fn default() -> Self {
        Self { expand: '!', subst: '^', comment: '#' }
    }
}

/// Expansion context threaded through the engine (histchars plus the posix
/// mode flag that changes double-quote handling in the scan).
#[derive(Debug, Clone, Copy)]
pub struct HistCtx {
    pub chars: HistChars,
    pub posix: bool,
}

/// history_expand return: GNU codes -1/0/1/2.
#[derive(Debug)]
pub struct HistExpandResult {
    /// -1 error (text holds the engine message), 0 no expansion, 1 expanded,
    /// 2 print-only (the p modifier).
    pub status: i32,
    pub text: String,
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

fn member(c: char, set: &str) -> bool {
    set.chars().any(|s| s == c)
}

fn whitespace(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\u{0b}' || c == '\u{0c}'
}

fn fielddelim(c: char) -> bool {
    whitespace(c) || c == '\n'
}

/// histexpand.c hist_error: "spec: message".
fn hist_error(spec: &str, start: usize, current: usize, errtype: i32) -> String {
    let emsg = match errtype {
        EVENT_NOT_FOUND => "event not found",
        BAD_WORD_SPEC => "bad word specifier",
        SUBST_FAILED => "substitution failed",
        BAD_MODIFIER => "unrecognized history modifier",
        NO_PREV_SUBST => "no previous substitution",
        _ => "unknown expansion error",
    };
    let chars: Vec<char> = spec.chars().collect();
    let current = current.min(chars.len());
    let start = start.min(current);
    let head: String = chars[start..current].iter().collect();
    if head.is_empty() {
        format!(": {emsg}")
    } else {
        format!("{head}: {emsg}")
    }
}

/// histexpand.c quote_breaks (the x modifier quoting).
fn quote_breaks(s: &str) -> String {
    let mut ret = String::with_capacity(s.len() + 2);
    ret.push('\'');
    for c in s.chars() {
        if c == '\'' {
            ret.push_str("'\\''");
        } else if whitespace(c) || c == '\n' {
            ret.push('\'');
            ret.push(c);
            ret.push('\'');
        } else {
            ret.push(c);
        }
    }
    ret.push('\'');
    ret
}

/// sh_single_quote equivalent (the q modifier quoting).
fn sh_single_quote(s: &str) -> String {
    let mut ret = String::with_capacity(s.len() + 2);
    ret.push('\'');
    for c in s.chars() {
        if c == '\'' {
            ret.push_str("'\\''");
        } else {
            ret.push(c);
        }
    }
    ret.push('\'');
    ret
}

/// histexpand.c hist_string_extract_single_quoted: index of the closing quote.
fn hist_string_extract_single_quoted(string: &str, sindex: usize, flags: u32) -> usize {
    let chars: Vec<char> = string.chars().collect();
    let mut i = sindex;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            break;
        }
        if (flags & 1) != 0 && c == BS && i + 1 < chars.len() {
            i += 1;
        }
        i += 1;
    }
    i
}

/// subst.c skip_single_quoted (byte-oriented in GNU; char-oriented here):
/// return the index after the closing single quote, or end of string.
/// return the index of the closing single quote or end of string.
fn skip_single_quoted(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // subst.c returns the index after the closing quote.
            return i + 1;
        }
        if c == BS && i + 1 < chars.len() {
            i += 1;
        }
        i += 1;
    }
    chars.len()
}

/// subst.c skip_double_quoted (simplified for the scan): return the index of
/// the closing double quote, or end of string.
fn skip_double_quoted(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() {
        let c = chars[i];
        if c == BS {
            i += 1;
        } else if c == '"' {
            // subst.c returns the index after the closing quote.
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// subst.c skip_to_histexp: scan for the history expansion char, honoring
/// backslash, backticks, single quotes (skipped wholesale), double quotes
/// (toggled outside posix mode; in posix mode double-quoted strings are
/// skipped wholesale), and dollar/angle-paren substitution (which resets
/// double-quote state). Returns the first expansion char index.
fn skip_to_histexp(chars: &[char], start: usize, ctx: HistCtx) -> Option<usize> {
    let histchar = ctx.chars.expand;
    let mut i = start;
    let mut pass_next = false;
    let mut backq = false;
    let mut dquote = false;
    let mut histexp_comsub = 0usize;
    let mut old_dquote = false;

    while i < chars.len() {
        let c = chars[i];
        if pass_next {
            pass_next = false;
            i += 1;
            continue;
        }
        if c == BS {
            pass_next = true;
            i += 1;
            continue;
        }
        if backq && c == BT {
            backq = false;
            dquote = old_dquote;
            i += 1;
            continue;
        }
        if c == BT {
            backq = true;
            old_dquote = dquote;
            dquote = false;
            i += 1;
            continue;
        }
        // In double quotes, expansion-char-then-quote is not an expansion
        // (like the library's no-expand handling).
        if dquote && c == histchar && chars.get(i + 1) == Some(&'"') {
            i += 1;
            continue;
        }
        if c == histchar {
            return Some(i);
        }
        if dquote && c == '\'' {
            i += 1;
            continue;
        }
        if c == '\'' {
            i = skip_single_quoted(chars, i + 1);
            continue;
        }
        // The posixly_correct test makes posix-mode shells allow double
        // quotes to quote the history expansion character (subst.c:2445).
        if c == '"' {
            if ctx.posix {
                i = skip_double_quoted(chars, i + 1).min(chars.len());
            } else {
                dquote = !dquote;
                i += 1;
            }
            continue;
        }
        if (c == '$' || c == '<' || c == '>')
            && chars.get(i + 1) == Some(&'(')
            && chars.get(i + 2) != Some(&'(')
        {
            i += 2;
            histexp_comsub += 1;
            old_dquote = dquote;
            dquote = false;
            continue;
        }
        if histexp_comsub > 0 && c == ')' {
            histexp_comsub -= 1;
            dquote = old_dquote;
            i += 1;
            continue;
        }
        i += 1;
    }
    // subst.c returns the end index when the scan runs off the string (for
    // example an unterminated posix double-quote skip); the caller and its
    // t > i test then report a candidate inside that skipped region as
    // inhibited. Do not model this as not found.
    Some(chars.len())
}

/// bashhist.c bash_history_inhibit_expansion (minus the extglob and
/// cross-line quoting-state rules, which histexp.tests does not exercise):
/// the expansion char in globbing bracket expressions, in indirect
/// expansion, and after a dollar is not expanded; quoting and
/// command-substitution regions mirror skip_to_histexp. A candidate that
/// falls inside a region the scan skips wholesale (t > i) is inhibited.
pub fn bash_history_inhibit_expansion(chars: &[char], i: usize, ctx: HistCtx) -> bool {
    let histchar = ctx.chars.expand;
    // The shell uses ! as a pattern negation character in [...] expressions.
    if i > 0 && chars.get(i - 1) == Some(&'[') && chars[i + 1..].contains(&']') {
        return true;
    }
    // The shell uses ! as the indirect expansion character.
    if i > 1
        && chars.get(i - 1) == Some(&'{')
        && chars.get(i - 2) == Some(&'$')
        && chars[i + 1..].contains(&'}')
    {
        return true;
    }
    // The shell uses the dollar-bang parameter expansion.
    if i > 1 && chars.get(i - 1) == Some(&'$') && chars.get(i) == Some(&histchar) {
        return true;
    }

    // Quoting / command-substitution scan (bashhist.c:255-268).
    match skip_to_histexp(chars, 0, ctx) {
        Some(mut t) => {
            while t < i {
                match skip_to_histexp(chars, t + 1, ctx) {
                    Some(next) => t = next,
                    None => return false,
                }
            }
            t > i
        }
        None => false,
    }
}

/// History list access used by the event resolver (decouples the engine from
/// the storage representation; mirrors history.c history_get).
pub trait HistLookup {
    /// history_base
    fn base(&self) -> usize;
    /// history_length
    fn length(&self) -> usize;
    /// history_get(offset): offset is absolute (relative to history_base).
    fn get(&self, offset: usize) -> Option<&str>;
}

/// histexpand.c get_history_event: resolve the event at caller_index.
/// Returns the event line (None on failure) and the post-parse index in both
/// cases (the failure index feeds hist_error, like the C *caller_index).
fn get_history_event(
    string: &str,
    caller_index: usize,
    delimiting_quote: Option<char>,
    hist: &dyn HistLookup,
    state: &mut HistEngineState,
    ctx: HistCtx,
) -> (Option<String>, usize) {
    let chars: Vec<char> = string.chars().collect();
    let histchar = ctx.chars.expand;
    let mut i = caller_index;
    if chars.get(i).copied() != Some(histchar) {
        return (None, i);
    }
    i += 1;

    // Double expansion char: the previous command.
    if chars.get(i).copied() == Some(histchar) {
        i += 1;
        let which = hist.base().saturating_add(hist.length().saturating_sub(1));
        return (hist.get(which).map(|line| line.to_string()), i);
    }

    let mut sign = 1;
    if chars.get(i) == Some(&'-') && chars.get(i + 1).map(|c| is_digit(*c)) == Some(true) {
        sign = -1;
        i += 1;
    }

    // Numeric specification.
    if chars.get(i).map(|c| is_digit(*c)) == Some(true) {
        let mut which: usize = 0;
        while chars.get(i).map(|c| is_digit(*c)) == Some(true) {
            which = which * 10 + chars[i].to_digit(10).unwrap() as usize;
            i += 1;
        }
        if sign < 0 {
            let total = hist.length() + hist.base();
            if which >= total {
                return (None, i);
            }
            which = total - which;
        }
        return (hist.get(which).map(|line| line.to_string()), i);
    }

    // question-mark substring search.
    let mut substring_okay = false;
    if chars.get(i) == Some(&'?') {
        substring_okay = true;
        i += 1;
    }

    let local_index = i;
    while i < chars.len() {
        let c = chars[i];
        let breaks = (!substring_okay
            && (whitespace(c)
                || c == ':'
                || (i > local_index && c == '-')
                || (c != '-' && member(c, HISTORY_EVENT_DELIMITERS))
                || member(c, SEARCH_DELIMITER_CHARS)
                || delimiting_quote == Some(c)))
            || c == '\n'
            || (substring_okay && c == '?');
        if breaks {
            break;
        }
        i += 1;
    }

    let which = i - local_index;
    let mut temp: String = chars[local_index..local_index + which].iter().collect();

    if substring_okay && chars.get(i) == Some(&'?') {
        i += 1;
    }

    // Empty substring search reuses the last search string.
    if temp.is_empty() && substring_okay {
        match state.search_string.clone() {
            Some(s) if !s.is_empty() => temp = s,
            _ => return (None, i),
        }
    }

    // Search backwards through the history list (histsearch.c
    // history_search_internal with listdir=-1).
    let mut position = hist.length().saturating_sub(1);
    loop {
        if hist.length() == 0 {
            return (None, i);
        }
        let Some(entry) = hist.get(hist.base() + position) else {
            if position == 0 {
                return (None, i);
            }
            position -= 1;
            continue;
        };
        let found = if substring_okay {
            entry.rfind(&temp)
        } else {
            entry.starts_with(&temp).then_some(0usize)
        };
        if let Some(line_index) = found {
            if substring_okay {
                state.search_string = Some(temp.clone());
                state.search_match = history_find_word(entry, line_index, ctx);
            }
            return (Some(entry.to_string()), i);
        }
        if position == 0 {
            return (None, i);
        }
        position -= 1;
    }
}

/// histexpand.c get_subst_pattern: extract the pattern delimited by
/// the given char from index i; backslash quotes the delimiter.
fn get_subst_pattern(chars: &[char], i: usize, delimiter: char, is_rhs: bool) -> (Option<String>, usize) {
    let mut si = i;
    while si < chars.len() && chars[si] != delimiter {
        if chars[si] == BS && chars.get(si + 1) == Some(&delimiter) {
            si += 1;
        }
        si += 1;
    }
    let closed = si < chars.len();
    let mut s: Option<String> = None;
    if si > i || is_rhs {
        let mut buf = String::new();
        let mut k = i;
        while k < si {
            if chars[k] == BS && chars.get(k + 1) == Some(&delimiter) {
                k += 1;
            }
            buf.push(chars[k]);
            k += 1;
        }
        s = Some(buf);
    }
    let mut ni = si;
    if closed {
        ni += 1;
    }
    (s, ni)
}

/// histexpand.c postproc_subst_rhs: replace unquoted ampersand in rhs with lhs.
fn postproc_subst_rhs(state: &mut HistEngineState) {
    let lhs = state.subst_lhs.clone().unwrap_or_default();
    let rhs = state.subst_rhs.clone().unwrap_or_default();
    let chars: Vec<char> = rhs.chars().collect();
    let mut new = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '&' {
            new.push_str(&lhs);
        } else {
            if chars[i] == BS && chars.get(i + 1) == Some(&'&') {
                i += 1;
            }
            new.push(chars[i]);
        }
        i += 1;
    }
    state.subst_rhs = Some(new);
}

/// histexpand.c history_expand_internal: expand one event occurrence.
#[allow(clippy::too_many_arguments)]
fn history_expand_internal(
    string: &str,
    start: usize,
    qc: Option<char>,
    end_index_ptr: &mut usize,
    ret_string: &mut Option<String>,
    current_line: &str,
    hist: &dyn HistLookup,
    state: &mut HistEngineState,
    ctx: HistCtx,
) -> i32 {
    let chars: Vec<char> = string.chars().collect();
    let mut i = start;
    let mut substitute_globally = false;
    let mut subst_bywords = false;
    let mut want_quotes: Option<char> = None;
    let mut print_only = false;

    // Expansion char followed by a word-specifier starter implies the
    // previous event (histexpand.c:549-557).
    let event: String = if chars.get(i + 1).map(|c| member(*c, ":$*%^")) == Some(true) {
        let fake = format!("{}{}", ctx.chars.expand, ctx.chars.expand);
        let (resolved, _) = get_history_event(&fake, 0, None, hist, state, ctx);
        match resolved {
            Some(line) => {
                i += 1;
                line
            }
            None => {
                *ret_string = Some(hist_error(string, start, start + 1, EVENT_NOT_FOUND));
                return -1;
            }
        }
    } else if chars.get(i + 1) == Some(&'#') {
        // The text entered so far (histexpand.c:558-562).
        i += 2;
        current_line.to_string()
    } else {
        let (resolved, ni) = get_history_event(string, i, qc, hist, state, ctx);
        i = ni;
        match resolved {
            Some(line) => line,
            None => {
                *ret_string = Some(hist_error(string, start, i, EVENT_NOT_FOUND));
                return -1;
            }
        }
    };

    // Word designator (histexpand.c:574-589).
    let starting_index = i;
    let word_spec = get_history_word_specifier(string, &event, &mut i, state, ctx);
    let mut temp: String = match word_spec {
        Err(bad) => {
            *ret_string = Some(hist_error(string, starting_index, bad, BAD_WORD_SPEC));
            return -1;
        }
        Ok(spec) => spec.unwrap_or_else(|| event.clone()),
    };

    // Modifiers (histexpand.c:592-836).
    let mut starting_index = i;
    while chars.get(i) == Some(&':') {
        let mut c = match chars.get(i + 1) {
            Some(c) => *c,
            None => break,
        };

        if c == 'g' || c == 'a' {
            substitute_globally = true;
            i += 1;
            c = chars.get(i + 1).copied().unwrap_or('\0');
        } else if c == 'G' {
            subst_bywords = true;
            i += 1;
            c = chars.get(i + 1).copied().unwrap_or('\0');
        }

        match c {
            'q' => want_quotes = Some('q'),
            'x' => want_quotes = Some('x'),
            'p' => print_only = true,
            't' => {
                if let Some(pos) = temp.rfind('/') {
                    temp = temp[pos + 1..].to_string();
                }
            }
            'h' => {
                if let Some(pos) = temp.rfind('/') {
                    temp.truncate(pos);
                }
            }
            'r' => {
                if let Some(pos) = temp.rfind('.') {
                    temp.truncate(pos);
                }
            }
            'e' => {
                if let Some(pos) = temp.rfind('.') {
                    temp = temp[pos..].to_string();
                }
            }
            '&' | 's' => {
                if c == 's' {
                    if chars.get(i + 2).is_none() {
                        break;
                    }
                    let delimiter = chars[i + 2];
                    i += 3;
                    let (lhs, ni) = get_subst_pattern(&chars, i, delimiter, false);
                    i = ni;
                    match lhs {
                        Some(lhs) => state.subst_lhs = Some(lhs),
                        None => {
                            if state.subst_lhs.is_none() {
                                if let Some(ss) = &state.search_string {
                                    if !ss.is_empty() {
                                        state.subst_lhs = Some(ss.clone());
                                    }
                                }
                            }
                        }
                    }
                    let (rhs, ni) = get_subst_pattern(&chars, i, delimiter, true);
                    i = ni;
                    state.subst_rhs = rhs;
                    if state.subst_lhs.is_some()
                        && state.subst_rhs.as_deref().map(|r| r.contains('&')).unwrap_or(false)
                    {
                        postproc_subst_rhs(state);
                    }
                } else {
                    i += 2;
                }

                let lhs_len = state.subst_lhs.as_ref().map(|s| s.chars().count()).unwrap_or(0);
                if lhs_len == 0 {
                    *ret_string = Some(hist_error(string, starting_index, i, NO_PREV_SUBST));
                    return -1;
                }
                let lhs: Vec<char> = state.subst_lhs.clone().unwrap_or_default().chars().collect();
                let rhs: Vec<char> = state.subst_rhs.clone().unwrap_or_default().chars().collect();
                let mut work: Vec<char> = temp.chars().collect();
                if lhs_len > work.len() {
                    *ret_string = Some(hist_error(string, starting_index, i, SUBST_FAILED));
                    return -1;
                }

                // Substitute RHS for LHS in temp. Three cases (histexpand.c
                // 759-769): first occurrence only; every occurrence (global);
                // the first occurrence of each word (by-words).
                let mut si = 0usize;
                let mut we = 0usize;
                let mut failed = true;
                while si + lhs_len <= work.len() {
                    if subst_bywords && si > we {
                        let text: String = work.iter().collect();
                        let mut skip = si;
                        while skip < text.chars().count() && fielddelim(text.chars().nth(skip).unwrap()) {
                            skip += 1;
                        }
                        we = history_tokenize_word(&text, skip, ctx.chars.comment);
                    }
                    if work[si..si + lhs_len] == lhs[..] {
                        let mut next: Vec<char> = Vec::new();
                        next.extend_from_slice(&work[..si]);
                        next.extend_from_slice(&rhs);
                        next.extend_from_slice(&work[si + lhs_len..]);
                        work = next;
                        failed = false;
                        if substitute_globally {
                            // histexpand.c:804: si += subst_rhs_len - 1 (an
                            // empty rhs walks back one, then the loop's si++
                            // re-examines the same position).
                            if rhs.is_empty() {
                                si = si.saturating_sub(1);
                            } else {
                                si += rhs.len() - 1;
                            }
                            continue;
                        } else if subst_bywords {
                            si = we;
                            continue;
                        } else {
                            break;
                        }
                    }
                    si += 1;
                }
                temp = work.iter().collect();
                if failed {
                    *ret_string = Some(hist_error(string, starting_index, i, SUBST_FAILED));
                    return -1;
                }
                continue;
            }
            _ => {
                *ret_string = Some(hist_error(string, i + 1, i + 2, BAD_MODIFIER));
                return -1;
            }
        }
        i += 2;
    }
    // histexpand.c:839 does "--i" because its caller re-increments i after
    // taking *end_index_ptr; this engine's caller does not re-increment, so
    // i already points at the first unconsumed character.

    let temp = match want_quotes {
        Some('q') => sh_single_quote(&temp),
        Some('x') => quote_breaks(&temp),
        _ => temp,
    };

    *end_index_ptr = i;
    *ret_string = Some(temp);
    if print_only {
        1
    } else {
        0
    }
}

/// Sentinel for the dollar word designator inside history_arg_extract.
const DOLLAR: i64 = -2;

/// histexpand.c get_history_word_specifier. Err holds the updated index for
/// the bad-word-specifier error.
fn get_history_word_specifier(
    spec: &str,
    from: &str,
    caller_index: &mut usize,
    state: &mut HistEngineState,
    ctx: HistCtx,
) -> Result<Option<String>, usize> {
    let chars: Vec<char> = spec.chars().collect();
    let mut i = *caller_index;
    let mut first: i64 = 0;
    let mut last: i64 = 0;
    let mut expecting_word_spec = false;

    if chars.get(i) == Some(&':') {
        i += 1;
        expecting_word_spec = true;
    }

    // Percent: the word last matched by the substring search
    // (histexpand.c:1335-1339).
    if chars.get(i) == Some(&'%') {
        *caller_index = i + 1;
        return Ok(Some(state.search_match.clone().unwrap_or_default()));
    }
    // Star: all arguments; an empty result is still a word spec
    // (histexpand.c:1342-1347).
    if chars.get(i) == Some(&'*') {
        *caller_index = i + 1;
        return Ok(Some(history_arg_extract(1, DOLLAR, from, ctx).unwrap_or_default()));
    }
    // Dollar: last argument (histexpand.c:1350-1354).
    if chars.get(i) == Some(&'$') {
        *caller_index = i + 1;
        return Ok(history_arg_extract(DOLLAR, DOLLAR, from, ctx));
    }

    if chars.get(i) == Some(&'-') {
        first = 0;
    } else if chars.get(i) == Some(&'^') {
        first = 1;
        i += 1;
    } else if chars.get(i).map(|c| is_digit(*c)) == Some(true) && expecting_word_spec {
        let mut value: i64 = 0;
        while chars.get(i).map(|c| is_digit(*c)) == Some(true) {
            value = value * 10 + chars[i].to_digit(10).unwrap() as i64;
            i += 1;
        }
        first = value;
    } else {
        return Ok(None);
    }

    if chars.get(i) == Some(&'^') || chars.get(i) == Some(&'*') {
        last = if chars.get(i) == Some(&'^') { 1 } else { DOLLAR };
        i += 1;
    } else if chars.get(i) != Some(&'-') {
        last = first;
    } else {
        i += 1;
        if chars.get(i).map(|c| is_digit(*c)) == Some(true) {
            let mut value: i64 = 0;
            while chars.get(i).map(|c| is_digit(*c)) == Some(true) {
                value = value * 10 + chars[i].to_digit(10).unwrap() as i64;
                i += 1;
            }
            last = value;
        } else if chars.get(i) == Some(&'$') {
            i += 1;
            last = DOLLAR;
        } else if chars.get(i) == Some(&'^') {
            i += 1;
            last = 1;
        } else {
            last = -1; // x- abbreviates x through second-to-last
        }
    }

    *caller_index = i;

    if last >= first || last == DOLLAR || last < 0 {
        match history_arg_extract(first, last, from, ctx) {
            Some(result) => Ok(Some(result)),
            None => Err(i),
        }
    } else {
        Err(i)
    }
}

/// histexpand.c history_arg_extract: extract words first..last (inclusive).
pub fn history_arg_extract(first: i64, last: i64, string: &str, ctx: HistCtx) -> Option<String> {
    let list = history_tokenize(string, Some(ctx.chars.comment))?;
    let len = list.len() as i64;

    let mut first = first;
    let mut last = last;
    if last < 0 && last != DOLLAR {
        last = len + last - 1;
    }
    if first < 0 && first != DOLLAR {
        first = len + first - 1;
    }
    if last == DOLLAR {
        last = len - 1;
    }
    if first == DOLLAR {
        first = len - 1;
    }
    last += 1;

    if first >= len || last > len || first < 0 || last < 0 || first > last {
        return None;
    }

    let mut parts: Vec<&str> = Vec::new();
    let mut index = first;
    while index < last {
        parts.push(&list[index as usize]);
        index += 1;
    }
    Some(parts.join(" "))
}

/// histexpand.c history_tokenize_word: end of the word at char index ind,
/// honoring quotes, command/process substitution, and extglob parens.
pub fn history_tokenize_word(string: &str, ind: usize, comment_char: char) -> usize {
    let chars: Vec<char> = string.chars().collect();
    let len = chars.len();
    let mut i = ind;
    let mut delimiter: char = '\0';
    let mut nestdelim = 0usize;
    let mut delimopen: char = '\0';

    if i < len && member(chars[i], "()\n") {
        return i + 1;
    }

    // Digit sequence that may start a file descriptor redirection
    // (histexpand.c:1497-1511).
    if i < len && is_digit(chars[i]) {
        let mut j = i;
        while j < len && is_digit(chars[j]) {
            j += 1;
        }
        if j == len {
            return j;
        }
        if chars[j] == '<' || chars[j] == '>' {
            i = j;
        } else {
            return get_word_tail(&chars, i, delimiter, nestdelim, delimopen);
        }
    }

    if i < len && member(chars[i], "<>;&|") {
        let peek = chars.get(i + 1).copied();
        if peek == Some(chars[i]) {
            if peek == Some('<') && chars.get(i + 2) == Some(&'-') {
                i += 1;
            } else if peek == Some('<') && chars.get(i + 2) == Some(&'<') {
                i += 1;
            }
            return i + 2;
        } else if peek == Some('&') && (chars[i] == '>' || chars[i] == '<') {
            let mut j = i + 2;
            while j < len && is_digit(chars[j]) {
                j += 1;
            }
            if j < len && chars[j] == '-' {
                j += 1;
            }
            return j;
        } else if (peek == Some('>') && chars[i] == '&') || (peek == Some('|') && chars[i] == '>') {
            return i + 2;
        } else if peek == Some('(') && (chars[i] == '>' || chars[i] == '<') {
            i += 2;
            delimopen = '(';
            delimiter = ')';
            nestdelim = 1;
            return get_word_tail(&chars, i, delimiter, nestdelim, delimopen);
        }
        return i + 1;
    }

    let _ = comment_char;
    get_word_tail(&chars, i, delimiter, nestdelim, delimopen)
}

fn get_word_tail(
    chars: &[char],
    mut i: usize,
    mut delimiter: char,
    mut nestdelim: usize,
    mut delimopen: char,
) -> usize {
    let len = chars.len();

    if delimiter == '\0' && i < len && member(chars[i], HISTORY_QUOTE_CHARACTERS) {
        delimiter = chars[i];
        i += 1;
    }

    while i < len {
        let c = chars[i];
        if c == BS && chars.get(i + 1) == Some(&'\n') {
            i += 1;
            continue;
        }
        if c == BS && delimiter != '\'' && (delimiter != '"' || member(c, SLASHIFY_IN_QUOTES)) {
            i += 1;
            if i == len {
                break;
            }
            continue;
        }
        if nestdelim > 0 && c == delimopen {
            nestdelim += 1;
            i += 1;
            continue;
        }
        if nestdelim > 0 && c == delimiter {
            nestdelim -= 1;
            if nestdelim == 0 {
                delimiter = '\0';
            }
            i += 1;
            continue;
        }
        if delimiter != '\0' && c == delimiter {
            delimiter = '\0';
            i += 1;
            continue;
        }
        // Command/process substitution and extended glob stay one word
        // (histexpand.c:1599-1608).
        if nestdelim == 0
            && delimiter == '\0'
            && member(c, "<>$!@?+*")
            && chars.get(i + 1) == Some(&'(')
        {
            i += 1;
            if chars.get(i + 1).is_none() {
                break;
            }
            delimopen = '(';
            delimiter = ')';
            nestdelim = 1;
            i += 1;
            continue;
        }
        if delimiter == '\0' && member(c, HISTORY_WORD_DELIMITERS) {
            break;
        }
        if delimiter == '\0' && member(c, HISTORY_QUOTE_CHARACTERS) {
            delimiter = c;
        }
        i += 1;
    }
    i
}

/// histexpand.c history_tokenize_internal: a token starting with the history
/// comment char terminates tokenization (histexpand.c:1656).
fn history_tokenize_internal(string: &str, wind: i64, comment_char: Option<char>) -> (Vec<String>, i64) {
    let mut result: Vec<String> = Vec::new();
    let mut result_index: i64 = -1;
    let mut i = 0usize;
    let chars: Vec<char> = string.chars().collect();
    let len = chars.len();

    while i < len {
        while i < len && fielddelim(chars[i]) {
            i += 1;
        }
        if i >= len || Some(chars[i]) == comment_char {
            break;
        }
        let start = i;
        i = history_tokenize_word(string, start, comment_char.unwrap_or('#'));
        if i == start && i < len {
            i += 1;
            while i < len && member(chars[i], HISTORY_WORD_DELIMITERS) {
                i += 1;
            }
        }
        if wind != -1 && wind >= start as i64 && wind < i as i64 {
            result_index = result.len() as i64;
        }
        result.push(chars[start..i].iter().collect());
    }
    (result, result_index)
}

/// histexpand.c history_tokenize.
pub fn history_tokenize(string: &str, comment_char: Option<char>) -> Option<Vec<String>> {
    let (result, _) = history_tokenize_internal(string, -1, comment_char);
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// histexpand.c history_find_word: the word containing char index ind.
fn history_find_word(line: &str, ind: usize, ctx: HistCtx) -> Option<String> {
    let (words, wind) = history_tokenize_internal(line, ind as i64, None);
    if wind < 0 {
        return None;
    }
    words.get(wind as usize).cloned()
}

/// histexpand.c history_expand (with bash configuration: quotes inhibit
/// expansion, bash no-expand char set, comment char from histchars[2]).
pub fn history_expand(
    hstring: &str,
    hist: &dyn HistLookup,
    state: &mut HistEngineState,
    ctx: HistCtx,
) -> HistExpandResult {
    let mut modified = false;
    let mut only_printing = false;

    // Quick substitution: caret-x-caret-y equals double-bang :s with carets
    // (histexpand.c:952-961).
    if hstring.starts_with(ctx.chars.subst) {
        let string = format!("{}{}:s{}", ctx.chars.expand, ctx.chars.expand, hstring);
        let result = expand_pass(&string, hist, state, ctx, &mut modified, &mut only_printing);
        if result.status < 0 {
            return result;
        }
        if only_printing {
            return HistExpandResult { status: 2, text: result.text };
        }
        return HistExpandResult { status: i32::from(modified), text: result.text };
    }

    // Pass 1: find the first expansion candidate (histexpand.c:990-1079).
    let chars: Vec<char> = hstring.chars().collect();
    let len = chars.len();
    let mut dquote = false;
    let mut i = 0usize;
    let mut candidate: Option<usize> = None;
    while i < len {
        let c = chars[i];
        let cc = chars.get(i + 1).copied();
        if ctx.chars.comment != '\0'
            && c == ctx.chars.comment
            && !dquote
            && (i == 0 || member(chars[i - 1], HISTORY_WORD_DELIMITERS))
        {
            // The rest of the line is a comment: no expansion.
            break;
        } else if c == ctx.chars.expand {
            if cc.is_none() || member(cc.unwrap(), NO_EXPAND_CHARS) {
                i += 1;
                continue;
            } else if dquote && cc == Some('"') {
                i += 1;
                continue;
            } else if bash_history_inhibit_expansion(&chars, i, ctx) {
                i += 1;
                continue;
            } else {
                candidate = Some(i);
                break;
            }
        } else if dquote && c == BS && cc == Some('"') {
            i += 1;
        } else if c == '"' {
            dquote = !dquote;
        } else if !dquote && c == '\'' {
            // Single quotes inhibit history expansion (outside double quotes;
            // histexpand.c:1052-1063).
            let flag = i > 0 && chars[i - 1] == '$';
            i += 1;
            i = hist_string_extract_single_quoted(hstring, i, u32::from(flag));
            if i >= len {
                i = len;
                break;
            }
        } else if c == BS && (cc == Some('\'') || cc == Some(ctx.chars.expand)) {
            i += 1;
        }
        i += 1;
    }

    if candidate.is_none() {
        return HistExpandResult { status: 0, text: hstring.to_string() };
    }

    // Pass 2: perform the substitutions (histexpand.c:1105-1290).
    let result = expand_pass(hstring, hist, state, ctx, &mut modified, &mut only_printing);
    if result.status < 0 {
        return result;
    }
    if only_printing {
        return HistExpandResult { status: 2, text: result.text };
    }
    HistExpandResult { status: i32::from(modified), text: result.text }
}

/// The expansion pass (histexpand.c:1105-1290).
fn expand_pass(
    string: &str,
    hist: &dyn HistLookup,
    state: &mut HistEngineState,
    ctx: HistCtx,
    modified: &mut bool,
    only_printing: &mut bool,
) -> HistExpandResult {
    let chars: Vec<char> = string.chars().collect();
    let len = chars.len();
    let mut result = String::new();
    let mut i = 0usize;
    let mut dquote = false;
    let mut passc = false;

    while i < len {
        let tchar = chars[i];
        if passc {
            passc = false;
            result.push(tchar);
            i += 1;
            continue;
        }

        if tchar == ctx.chars.expand {
            let cc = chars.get(i + 1).copied();
            if cc.is_none() || member(cc.unwrap(), NO_EXPAND_CHARS) || (dquote && cc == Some('"')) {
                result.push(tchar);
                i += 1;
                continue;
            }
            // Inhibit check against the expansion-so-far (histexpand.c:1229):
            // append the expansion char and the following char, check, then
            // un-add the following char on inhibition.
            let save_len = result.chars().count();
            let mut probe = result.clone();
            probe.push(tchar);
            if let Some(ccc) = cc {
                probe.push(ccc);
            }
            let probe_chars: Vec<char> = probe.chars().collect();
            let inhibited = bash_history_inhibit_expansion(&probe_chars, save_len, ctx);
            if inhibited {
                result.push(tchar);
                i += 1;
                continue;
            }

            let qc = if dquote { Some('"') } else { None };
            let mut eindex = i;
            let mut temp: Option<String> = None;
            let current_line = result.clone();
            let r = history_expand_internal(
                string,
                i,
                qc,
                &mut eindex,
                &mut temp,
                &current_line,
                hist,
                state,
                ctx,
            );
            if r < 0 {
                return HistExpandResult {
                    status: -1,
                    text: temp.unwrap_or_else(|| "history expansion failed".to_string()),
                };
            }
            if let Some(temp) = temp {
                *modified = true;
                if !temp.is_empty() {
                    result.push_str(&temp);
                }
            }
            *only_printing |= r == 1;
            i = eindex;
            continue;
        }

        match tchar {
            BS => {
                passc = true;
                result.push(tchar);
            }
            '"' => {
                dquote = !dquote;
                result.push(tchar);
            }
            '\'' if !dquote => {
                // Single quotes inhibit history expansion: copy verbatim,
                // including the opening quote, through the closing quote
                // (histexpand.c:1171-1185).
                let flag = i > 0 && chars[i - 1] == '$';
                result.push(chars[i]);
                i += 1;
                let end = hist_string_extract_single_quoted(string, i, u32::from(flag));
                let upto = (end + 1).min(len);
                while i < upto {
                    result.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            c if ctx.chars.comment != '\0'
                && c == ctx.chars.comment
                && !dquote
                && (i == 0 || member(chars[i - 1], HISTORY_WORD_DELIMITERS)) =>
            {
                // The comment char at a word start: copy the rest verbatim
                // (histexpand.c:1196-1208).
                while i < len {
                    result.push(chars[i]);
                    i += 1;
                }
                break;
            }
            _ => result.push(tchar),
        }
        i += 1;
    }

    HistExpandResult { status: 0, text: result }
}
