use super::brace_scan_cache::{BraceScanCache, BraceScanEntry, BraceScanResume};
use super::dolbrace::{scan_braced_parameter, BraceContext, DolbraceState};
use super::scanner::Lexer;

/// How a `{ ... }' brace-group scan ended (see `Lexer::skip_brace`).
pub(super) struct BraceScan {
    /// True when the matching `}' was consumed.
    pub(super) closed: bool,
    /// Byte offset of the first word-initial `#' comment that began at the
    /// group's top level, when the scan ran past one. Everything from that
    /// offset on is comment text, not group text.
    pub(super) comment_start: Option<usize>,
}

impl<'a> Lexer<'a> {
    pub(super) fn skip_cmd_subst(&mut self) {
        let mut depth = 1;
        let mut case_depth = 0usize;
        let mut word = String::new();
        let mut word_boundary = true;
        let mut current_word_boundary = true;
        let mut parameter_depth = 0usize;
        // `case WORD in' chain tracker (GNU special_case_tokens,
        // parse.y:3369-3386 + 3433-3441) — see
        // update_command_substitution_case_depth.
        let mut case_in_stage = 0u8;
        let mut case_pattern_region = false;
        // rubash#380: the last significant unquoted char fed to the case
        // word machine — the previous-token witness for the `esac`
        // pattern-text rule (parse.y:3181/3183).
        let mut prev_sig: Option<char> = None;
        // GNU read_token (parse.y:3630-3643): `#` introduces a comment only
        // at a token boundary — after whitespace, a separator (`;&|()<>`),
        // or at the start. `word.is_empty()` alone is wrong: `$`, quotes and
        // other non-alphanumeric word characters never reach `word`, so
        // `$(echo $#)` and `$(echo 'a'#b)` would misread `#` as a comment.
        let mut token_boundary = true;
        while let Some(c) = self.advance() {
            if c == '\\' {
                // GNU read_token_word: a backslash-quoted character is word
                // text (`\;#` keeps `#` mid-word — comsub1.sub). Only a
                // quoted newline is a line continuation, not word content.
                // Push a placeholder rather than the literal char: `c\ase`
                // is not the `case` reserved word.
                if let Some(next) = self.advance() {
                    if next != '\n' {
                        word.push('\u{1}');
                    }
                }
                token_boundary = false;
                continue;
            }
            if c == '#' && token_boundary && parameter_depth == 0 {
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.advance();
                }
                word.clear();
                word_boundary = true;
                current_word_boundary = true;
                token_boundary = true;
                continue;
            }
            if c == '$' && self.peek() == Some('{') {
                parameter_depth += 1;
            } else if c == '}' && parameter_depth > 0 {
                parameter_depth -= 1;
            }
            let next = self.peek();
            update_command_substitution_case_depth(
                c,
                false,
                false,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
                next,
                &mut case_in_stage,
                &mut case_pattern_region,
                prev_sig,
            );
            if esac_prev_token_char(c) {
                prev_sig = Some(c);
            }
            match c {
                '`' => {
                    self.skip_backtick();
                    token_boundary = false;
                    continue;
                }
                '(' if case_depth == 0 => depth += 1,
                ')' if case_depth == 0 => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '$' if self.peek() == Some('\'') => {
                    self.advance();
                    self.skip_ansi_c_single();
                    token_boundary = false;
                }
                '$' if self.peek() == Some('(') => {
                    self.advance();
                    if self.peek() == Some('(') {
                        self.advance();
                        self.skip_arith_paren();
                    } else {
                        self.skip_cmd_subst();
                    }
                    token_boundary = false;
                }
                '\'' => {
                    self.skip_single();
                    token_boundary = false;
                }
                '"' => {
                    self.skip_double();
                    token_boundary = false;
                }
                '<' if self.peek() == Some('<') && self.peek_after(1) == Some('<') => {
                    self.advance();
                    self.advance();
                }
                '<' if self.peek() == Some('<') => {
                    if self.skip_heredoc_in_command_substitution() {
                        break;
                    }
                    // A heredoc terminator ends on its own line, so the next
                    // character begins a fresh token.
                    token_boundary = true;
                    continue;
                }
                _ => {}
            }
            token_boundary =
                c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        }
    }

    pub(super) fn skip_arith_paren(&mut self) {
        let mut depth = 0usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        while let Some(c) = self.advance() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            match c {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '(' if !single && !double => depth += 1,
                ')' if !single && !double && depth > 0 => depth -= 1,
                ')' if !single && !double && self.peek() == Some(')') => {
                    self.advance();
                    break;
                }
                _ => {}
            }
        }
    }

    pub(super) fn skip_arith_bracket(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.advance() {
            match c {
                '[' => depth += 1,
                ']' if depth > 0 => depth -= 1,
                ']' => break,
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                '\\' => {
                    self.advance();
                }
                _ => {}
            }
        }
    }

    pub(super) fn skip_heredoc_in_command_substitution(&mut self) -> bool {
        self.advance();
        let strip_tabs = if self.peek() == Some('-') {
            self.advance();
            true
        } else {
            false
        };
        while self.peek().is_some_and(|ch| matches!(ch, ' ' | '\t')) {
            self.advance();
        }
        let delimiter_start = self.position;
        // GNU read_token_word: quoting inside the delimiter word makes
        // metacharacters literal — `<< ')'` names `)` as the delimiter, so a
        // quoted `)` (or `;`, `|`, `&`) is delimiter text, not the
        // substitution closer (comsub-posix.tests).
        let mut delimiter_single = false;
        let mut delimiter_double = false;
        while let Some(next) = self.peek() {
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
                // literal `)` delimiter); consume the escape pair as one
                // unit so the quoted `)` is not mistaken for the closer.
                '\\' if !delimiter_single && !delimiter_double && self.peek_after(1).is_some() => {
                    self.advance();
                }
                _ => {}
            }
            self.advance();
        }
        let mut delimiter =
            self.input[delimiter_start..self.position].replace(['\'', '"', '\\'], "");
        if strip_tabs {
            delimiter = delimiter.trim_start_matches('\t').to_string();
        }
        if delimiter.is_empty() {
            return false;
        }
        let mut header_closes_command_substitution = false;
        while self.peek().is_some_and(|ch| ch != '\n') {
            if self.peek() == Some(')') {
                header_closes_command_substitution = true;
            }
            self.advance();
        }
        if self.peek() == Some('\n') {
            self.advance();
        }

        while !self.at_end() {
            let line_start = self.position;
            while self.peek().is_some_and(|ch| ch != '\n') {
                self.advance();
            }
            let line = &self.input[line_start..self.position];
            let comparable = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if comparable
                .strip_suffix(')')
                .is_some_and(|value| value == delimiter)
            {
                let leading_tabs = if strip_tabs {
                    line.chars().take_while(|ch| *ch == '\t').count()
                } else {
                    0
                };
                self.position = line_start + leading_tabs + delimiter.len();
                break;
            }
            if comparable == delimiter {
                if self.peek() == Some('\n') {
                    self.advance();
                }
                break;
            }
            // GNU make_cmd.c:602-611 (PST_EOFTOKEN): a body line that starts
            // with the delimiter and carries `)` later on the line ends the
            // heredoc as if it hit EOF; the rest of the line is pushed back
            // into the parser input, where the `)` then closes the command
            // substitution (`foo=$(cat <<EOF / hi / EOF )`). Resume the span
            // scan at that first `)`.
            if comparable.starts_with(delimiter.as_str()) {
                if let Some(paren) = comparable[delimiter.len()..].find(')') {
                    self.position = line_start + delimiter.len() + paren;
                    break;
                }
            }
            if self.peek() == Some('\n') {
                self.advance();
            }
        }
        header_closes_command_substitution
    }

    pub(super) fn skip_backtick(&mut self) {
        while let Some(c) = self.advance() {
            if c == '`' {
                break;
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    pub(super) fn skip_single(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\'' {
                break;
            }
        }
    }
    pub(super) fn skip_ansi_c_single(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\\' {
                self.advance();
            } else if c == '\'' {
                break;
            }
        }
    }
    pub(super) fn skip_double(&mut self) {
        while let Some(c) = self.advance() {
            if c == '"' {
                break;
            } else if c == '`' {
                self.skip_backtick();
            } else if c == '$' {
                match self.peek() {
                    Some('{') => {
                        self.advance();
                        self.skip_braced(true);
                    }
                    Some('(') => {
                        self.advance();
                        if self.peek() == Some('(') {
                            self.advance();
                            self.skip_arith_paren();
                        } else {
                            self.skip_cmd_subst();
                        }
                    }
                    _ => {}
                }
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    pub(super) fn skip_braced(&mut self, outer_double_quote: bool) {
        let start = self.position.saturating_sub(2);
        // Bash 5.3 funsub: `${ command; }` / `${|command;}` bodies are
        // command lists parsed by the real parser (parse.y:4451 parse_comsub
        // -> yyparse with DOLBRACE): a `}` only ends the body when it is a
        // LONE WORD in command position — parse.y:3465-3468
        // special_case_tokens returns the `}' token only when
        // reserved_word_acceptable(last_read_token) holds (after `;', `&',
        // `|', a newline, or a closed command construct). A `}` inside a
        // word (`\"}\" in `${ echo \"}\"; }`, `a}b`, `x=}`) is word text, and
        // a `{` opens a brace group only in command position (parse.y:3460;
        // `${ echo {; }` runs `echo {`). Same term model as
        // continuation.rs's funsub delimiter.
        if self
            .input
            .get(start + 2..start + 3)
            .is_some_and(|ch| ch == "|" || ch.chars().next().is_some_and(|c| c.is_whitespace()))
        {
            let mut depth = 1usize;
            let mut single = false;
            let mut double = false;
            let mut escaped = false;
            // Command position: true at body start and after a command
            // terminator (`;` `&` `|` newline) or a closed `{ }` group /
            // `( )` subshell.
            let mut term = true;
            let mut paren_depth = 0usize;
            while let Some(c) = self.advance() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' && !single {
                    escaped = true;
                    if !double {
                        term = false;
                    }
                    continue;
                }
                if single {
                    if c == '\'' {
                        single = false;
                    }
                    continue;
                }
                if double {
                    if c == '"' {
                        double = false;
                    }
                    continue;
                }
                match c {
                    '$' => {
                        // parse.y:5494 read_token_word (shellexp branch):
                        // `$' followed by `{', `(' or `'' is consumed as ONE
                        // unit at ANY word position — there is no
                        // command-position gate on it, unlike the bare `{'
                        // group opener below. A nested `${ ... }' inside a
                        // funsub body therefore never opens a bare brace
                        // group and its matching `}' never terminates the
                        // body; only a word BEGINNING with `}' does
                        // (parse.y:5400-5416). Without this arm,
                        // `${ echo X${ echo nested; }Y; }' ended the outer
                        // body at the inner funsub's `}' (341bf41b
                        // follow-up; the same rule now lives in the three
                        // sibling funsub scanners).
                        match self.peek() {
                            Some('{') => {
                                self.advance();
                                self.skip_braced(false);
                            }
                            Some('(') => {
                                self.advance();
                                if self.peek() == Some('(') {
                                    self.advance();
                                    self.skip_arith_paren();
                                } else {
                                    self.skip_cmd_subst();
                                }
                            }
                            Some('\'') => {
                                // $'...' is one quoted unit (shellexp ->
                                // parse_matched_pair): opaque to `;' / `}'.
                                self.advance();
                                while let Some(qc) = self.advance() {
                                    if qc == '\\' {
                                        self.advance();
                                    } else if qc == '\'' {
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
                        // The unit continues the current word
                        // (`X${...}Y', `$(x)y'); a terminator only a
                        // following `;'/`&'/newline can set.
                        term = false;
                    }
                    '\'' => {
                        single = true;
                        term = false;
                    }
                    '"' => {
                        double = true;
                        term = false;
                    }
                    '(' => {
                        paren_depth += 1;
                        term = false;
                    }
                    ')' if paren_depth > 0 => {
                        paren_depth -= 1;
                        // A closed outermost `( ... )` is a complete command.
                        term = paren_depth == 0;
                    }
                    '{' if term && paren_depth == 0 => {
                        depth += 1;
                        // A command follows the opening brace.
                        term = true;
                    }
                    '}' if term && paren_depth == 0 => {
                        // parse.y:5407-5416 read_token_word: a word BEGINNING
                        // with `}` in command position (reserved_word_
                        // acceptable) terminates the substitution even when
                        // more characters follow (`${| REPLY=x; }-tail`).
                        // term==false covers the mid-word case (`a}b`,
                        // `\"}`) — those `}` are word text.
                        if depth > 1 {
                            // A closed `{ }` group is a complete command:
                            // the funsub's own `}` may follow without
                            // another separator (`${ { echo x; } }`).
                            depth -= 1;
                            term = true;
                            continue;
                        }
                        return;
                    }
                    ';' | '&' | '|' | '\n' if paren_depth == 0 => term = true,
                    ' ' | '\t' | '\r' => {}
                    _ => term = false,
                }
            }
            return;
        }
        let context = BraceContext {
            outer_double_quote,
            // parse.y::parse_matched_pair keeps POSIX mode separate from the
            // surrounding quote state. The line tokenizer tracks runtime
            // `set -o posix` switches, mirroring GNU's lazy per-command parse.
            posix: self.posix,
            replacement_context: false,
            initial_state: DolbraceState::Param,
        };
        if let Some(scan) = scan_braced_parameter(&self.input[start..], context) {
            self.position = start + scan.end;
            return;
        }

        let mut depth = 1usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        while let Some(c) = self.advance() {
            if escaped {
                escaped = false;
                continue;
            }

            if c == '\\' {
                escaped = true;
                continue;
            }

            match c {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '$' if !single && self.peek() == Some('{') => {
                    self.advance();
                    depth += 1;
                }
                '$' if !single && self.peek() == Some('(') => {
                    self.advance();
                    if self.peek() == Some('(') {
                        self.advance();
                        self.skip_arith_paren();
                    } else {
                        self.skip_cmd_subst();
                    }
                }
                '$' if !single && self.peek() == Some('[') => {
                    self.advance();
                    self.skip_arith_bracket();
                }
                '}' if !single && !double => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    /// Scans a `{ ... }' brace group starting at the opening brace.
    ///
    /// `closed` reports whether the matching `}' was consumed; an unterminated
    /// group ends at the end of the logical line instead. `comment_start`
    /// reports a word-initial `#' comment seen at the group's top level, so the
    /// caller can tell group text from comment text: a `#' comments out the
    /// rest of its physical line (parse.y read_token -> parse_comment), and
    /// without this the scanner sliced the comment into the brace token
    /// (`f() { # note' became `{ # note'), which the parser then rejected
    /// (niubash #120 follow-up).
    pub(super) fn skip_brace(&mut self) -> BraceScan {
        // rubash#176/#178: the `{` was just consumed, so its byte offset is
        // one before the scan position. GNU reads a script exactly once
        // (parse.y:3557 read_token carries the reader state forward); the
        // batch tokenizer instead re-scans the whole accumulated logical
        // line on every appended physical line, and this scan used to run
        // from each still-open `{` to end-of-input on every pass —
        // O(open_braces · buffer) per pass, near-cubic over a file. The
        // per-logical-line cache stores, per `{`, either the finished
        // result or a continuation of the scan state, valid while the line
        // only grows by appends (the line loop clears the cache on every
        // non-append mutation).
        let brace_offset = self.position - 1;
        let mut cache = self.brace_cache.take();
        let result = self.skip_brace_cached(brace_offset, cache.as_deref_mut());
        self.brace_cache = cache;
        result
    }

    fn skip_brace_cached(
        &mut self,
        brace_offset: usize,
        mut cache: Option<&mut BraceScanCache>,
    ) -> BraceScan {
        if let Some(entry) = cache
            .as_ref()
            .and_then(|cache| cache.lookup_brace(brace_offset))
        {
            match entry {
                BraceScanEntry::Closed { end, comment_start } => {
                    if *end <= self.input.len() {
                        self.position = *end;
                        return BraceScan {
                            closed: true,
                            comment_start: *comment_start,
                        };
                    }
                }
                BraceScanEntry::Unclosed(resume) => {
                    if resume.pos <= self.input.len() {
                        // The resume must restore BOTH halves of the snapshot
                        // (brace_scan_cache.rs soundness rule 3): the scan
                        // state via `SkipBraceScan::from_resume' AND the byte
                        // position via `resume.pos'. Restoring only the state
                        // re-scans the verified prefix from just past the `{'
                        // while KEEPING the snapshot depth — every `{' between
                        // the brace and the snapshot stop is counted once more
                        // per pass, the depth inflates monotonically, and the
                        // group can never reach depth 0 again (rubash#176
                        // follow-up: a depth-4 multi-line group made the
                        // parser reject earlier groups with `unexpected end of
                        // file from `{'' while GNU parses clean; matrix S2/S7
                        // shapes, depth >= 4).
                        let scan = SkipBraceScan::from_resume(resume.clone());
                        self.position = resume.pos;
                        return self.run_skip_brace(brace_offset, scan, cache);
                    }
                }
            }
        }
        self.run_skip_brace(brace_offset, SkipBraceScan::new(), cache)
    }

    fn run_skip_brace(
        &mut self,
        brace_offset: usize,
        mut scan: SkipBraceScan,
        mut cache: Option<&mut BraceScanCache>,
    ) -> BraceScan {
        // Set when the `esac)' case-pattern lookahead ran to end-of-input
        // undecided: its answer was computed against a truncated tail and a
        // longer input could change it, so the scan must not be resumed.
        let mut lookahead_truncated = false;
        loop {
            // Resumed inside an unterminated `#' comment: keep eating
            // comment text through the terminator, exactly as the comment
            // branch below does for in-scan comments.
            if scan.pending_comment {
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.advance();
                }
                if self.peek() == Some('\n') {
                    // The newline itself is processed by the normal path so
                    // the case-depth/word trackers see it, matching a fresh
                    // scan where the comment branch stops before the `\n'.
                    scan.pending_comment = false;
                }
            }
            let Some(c) = self.advance() else { break };
            if scan.escaped {
                scan.escaped = false;
                scan.comment_start = false;
                continue;
            }
            if scan.ansi_single {
                if c == '\\' {
                    scan.escaped = true;
                } else if c == '\'' {
                    scan.ansi_single = false;
                }
                scan.comment_start = false;
                continue;
            }
            // GNU parse.y:3465-3469 close-acceptability bookkeeping: a
            // standalone `}' (or nested `{' opener) is a token only where
            // reserved_word_acceptable (parse.y:5899) holds for the token
            // before it — after `;', `&', `|', `(', `)', a newline, or the
            // reserved words that may be followed by a command; after an
            // ordinary WORD it is word text. This tracker maintains the
            // lexical equivalent for the `{'/`}' arms below (rubash#222).
            if c == '\n' {
                if !scan.word_start {
                    scan.prev_accepts_close =
                        scan.word_plain && brace_close_acceptable_word(&scan.close_word);
                }
                // The newline token itself is in the acceptable set.
                scan.prev_accepts_close = true;
                scan.word_start = true;
                scan.word_plain = true;
                scan.close_word.clear();
            } else if c.is_whitespace() {
                if !scan.word_start {
                    scan.prev_accepts_close =
                        scan.word_plain && brace_close_acceptable_word(&scan.close_word);
                }
                scan.word_start = true;
                scan.word_plain = true;
                scan.close_word.clear();
            } else if !matches!(c, '{' | '}') {
                // Word content and operators. `{'/`}' are classified by
                // the match arms below (they may be tokens or word text).
                if c == '_' || c == ']' || c.is_ascii_alphanumeric() {
                    scan.word_start = false;
                    scan.close_word.push(c);
                } else {
                    match c {
                        ';' | '&' | '|' | '(' | ')' => {
                            scan.prev_accepts_close = true;
                            scan.word_start = true;
                            scan.word_plain = true;
                            scan.close_word.clear();
                        }
                        // Quotes, substitutions, backslash and the rest of
                        // the word's own characters keep the word in
                        // progress; such a word is never a reserved word.
                        _ => {
                            scan.word_plain = false;
                            scan.word_start = false;
                        }
                    }
                }
            }
            let rest = &self.input[self.position..];
            update_brace_group_case_depth(
                c,
                &mut scan.word,
                &mut scan.case_depth,
                &mut scan.word_boundary,
                &mut scan.current_word_boundary,
                rest,
                &mut lookahead_truncated,
            );
            if c == '\n' {
                if scan.depth == 1 {
                    scan.saw_top_level_whitespace = true;
                }
                scan.comment_start = true;
                continue;
            }
            if c.is_whitespace() {
                if scan.depth == 1 {
                    scan.saw_top_level_whitespace = true;
                }
                scan.comment_start = true;
                continue;
            }
            if c == '#' && scan.comment_start {
                if scan.comment_at.is_none() && scan.depth == 1 {
                    // `advance' already consumed the `#', which is one byte.
                    scan.comment_at = Some(self.position - 1);
                }
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.advance();
                }
                if self.peek().is_none() {
                    scan.pending_comment = true;
                }
                continue;
            }
            match c {
                '{' if scan.case_depth == 0 => {
                    scan.comment_start = false;
                    // GNU parse.y:3173 CHECK_FOR_RESERVED_WORD: `{` opens a
                    // group only as the STANDALONE reserved word at a
                    // reserved_word_acceptable position. `{a,b}` / `{xxx`
                    // are ordinary words — `{` is not in shell_break_chars
                    // (syntax.h:30) — and must not raise the depth
                    // (rubash#222).
                    let standalone_opener = scan.word_start
                        && scan.prev_accepts_close
                        && self
                            .peek()
                            .is_none_or(|next| "()<>;&| \t\n\r".contains(next));
                    if standalone_opener {
                        scan.depth += 1;
                        // The `{` token itself is in the acceptable set.
                        scan.prev_accepts_close = true;
                        scan.word_start = true;
                        scan.word_plain = true;
                    } else {
                        scan.word_start = false;
                        scan.word_plain = false;
                    }
                }
                '}' if scan.case_depth == 0 => {
                    scan.comment_start = false;
                    // GNU parse.y:3465-3469: `}` closes the group only as
                    // the standalone reserved word after an
                    // acceptable token; after an ordinary WORD (or glued
                    // into one, `hi}`) it is word text — an argument
                    // (`{ foo } }; }': the first two `}` are foo's
                    // arguments; `{ f() { echo } ; }': the first `}` is
                    // echo's argument) (rubash#222). The reserved word is
                    // the EXACT one-character word: `}` is not in
                    // shell_break_chars (syntax.h:30), so read_token_word
                    // collects `}}'/`}x'/`}{' as ONE word and
                    // CHECK_FOR_RESERVED_WORD's STREQ (parse.y:3174-3175)
                    // never matches it — the mirror of the opener's
                    // standalone rule above. Without the peek test
                    // `f() { :; }}` closed the group at the first `}` of
                    // the glued pair and defined the function, where GNU
                    // reads `}}` as one word, never closes the group, and
                    // reports `syntax error: unexpected end of file from
                    // `{' command on line 1` at end of input (rubash#278).
                    let standalone_closer = scan.word_start
                        && scan.prev_accepts_close
                        && self
                            .peek()
                            .is_none_or(|next| "()<>;&| \t\n\r".contains(next));
                    if !standalone_closer {
                        scan.word_start = false;
                        scan.word_plain = false;
                        continue;
                    }
                    scan.depth -= 1;
                    // The `}' token itself is in the acceptable set
                    // (reserved_word_acceptable, parse.y:5910).
                    scan.prev_accepts_close = true;
                    scan.word_start = true;
                    scan.word_plain = true;
                    if scan.depth == 0 {
                        match self.peek() {
                            // A close decided without consulting past the
                            // current end of input is stable under later
                            // appends and can be cached; a close at
                            // end-of-input is not (the appended bytes could
                            // be a compact-group terminator).
                            Some(_) => {
                                if !scan.saw_top_level_whitespace {
                                    self.record_brace_scan_closed(
                                        brace_offset,
                                        scan.comment_at,
                                        cache.as_deref_mut(),
                                    );
                                    return BraceScan {
                                        closed: true,
                                        comment_start: scan.comment_at,
                                    };
                                }
                            }
                            None => {
                                if !scan.saw_top_level_whitespace {
                                    return BraceScan {
                                        closed: true,
                                        comment_start: scan.comment_at,
                                    };
                                }
                            }
                        }
                        let (ends_group, decided_in_prefix) =
                            self.brace_close_can_end_compact_group();
                        if ends_group {
                            if decided_in_prefix {
                                self.record_brace_scan_closed(
                                    brace_offset,
                                    scan.comment_at,
                                    cache.as_deref_mut(),
                                );
                            }
                            return BraceScan {
                                closed: true,
                                comment_start: scan.comment_at,
                            };
                        }
                        scan.depth = 1;
                    }
                }
                '$' => {
                    scan.comment_start = false;
                    match self.peek() {
                        Some('{') => {
                            self.advance();
                            self.skip_braced(false);
                        }
                        Some('(') => {
                            self.advance();
                            if self.peek() == Some('(') {
                                self.advance();
                                self.skip_arith_paren();
                            } else {
                                self.skip_cmd_subst();
                            }
                        }
                        Some('\'') => {
                            self.advance();
                            scan.ansi_single = true;
                        }
                        _ => {}
                    }
                }
                '`' => {
                    scan.comment_start = false;
                    self.skip_backtick();
                }
                '\'' => {
                    scan.comment_start = false;
                    self.skip_single();
                }
                '"' => {
                    scan.comment_start = false;
                    self.skip_double();
                }
                '\\' => {
                    scan.comment_start = false;
                    self.advance();
                }
                _ => {
                    scan.comment_start = false;
                }
            }
        }
        if !lookahead_truncated {
            if let Some(cache) = cache.as_deref_mut() {
                cache.record_brace(
                    brace_offset,
                    BraceScanEntry::Unclosed(scan.clone().into_resume(self.position)),
                );
            }
        }
        BraceScan {
            closed: false,
            comment_start: scan.comment_at,
        }
    }

    fn record_brace_scan_closed(
        &self,
        brace_offset: usize,
        comment_start: Option<usize>,
        mut cache: Option<&mut BraceScanCache>,
    ) {
        if let Some(cache) = cache.as_deref_mut() {
            cache.record_brace(
                brace_offset,
                BraceScanEntry::Closed {
                    end: self.position,
                    comment_start,
                },
            );
        }
    }

    fn brace_close_can_end_compact_group(&self) -> (bool, bool) {
        let rest = &self.input[self.position..];
        let mut saw_blank = false;
        for (index, ch) in rest.char_indices() {
            match ch {
                ' ' | '\t' | '\r' => {
                    saw_blank = true;
                    continue;
                }
                '\n' => return (true, true),
                // A word-initial `#' after the closing brace comments out the
                // rest of the physical line (parse.y read_token ->
                // parse_comment), so the `}' really does end the group:
                // `{ echo x; } # note' (niubash #120 follow-up). Without this
                // the group keeps scanning for a later `}' and swallows the
                // comment text into the brace token.
                '#' => return (true, true),
                ';' | '|' | '&' | '<' | '>' | ')' => return (true, true),
                _ if !saw_blank => return (true, true),
                _ if ch.is_ascii_digit() => {
                    // Decided in-prefix iff a redirection operator exists in
                    // the remaining text: without one the arm falls through
                    // to the reserved-word check, and a later append could
                    // add the `<'/`>' that flips which arm runs.
                    let redirect_ahead = rest[index..].chars().any(|c| matches!(c, '<' | '>'));
                    if redirect_ahead {
                        return (true, true);
                    }
                    let (ends, decided) = brace_close_followed_by_reserved_word(&rest[index..]);
                    return (ends, decided);
                }
                _ => {
                    let (ends, decided) = brace_close_followed_by_reserved_word(&rest[index..]);
                    return (ends, decided);
                }
            }
        }
        // Only blanks until the end of input: the answer today is `true',
        // but it was decided by end-of-input, not by a real character.
        (true, false)
    }
}

/// Outer-loop state of `skip_brace`, kept in a struct so an interrupted
/// scan can be resumed after more physical lines are appended (rubash#176).
#[derive(Clone)]
struct SkipBraceScan {
    depth: usize,
    case_depth: usize,
    word: String,
    word_boundary: bool,
    current_word_boundary: bool,
    /// True when the scan stopped inside an unterminated `#' comment.
    pending_comment: bool,
    /// The scan starts just past the opening `{', which is itself a word
    /// start, so the character at hand is mid-word: `{#note' is one word in
    /// GNU (a `#' comments only at a word start), and the whitespace
    /// branches of the scan raise this again for `{ # note'.
    comment_start: bool,
    comment_at: Option<usize>,
    saw_top_level_whitespace: bool,
    ansi_single: bool,
    escaped: bool,
    /// GNU parse.y:3465-3469: a `}' closes the group only when the
    /// previous token can end a list — reserved_word_acceptable
    /// (parse.y:5899) at the lexical level means `;', `&', a newline,
    /// `)', `{'/`}', or a completed `fi'/`done'/`esac'. A `}' after an
    /// ordinary word is WORD text (`{ foo } }; }' — the first two `}` are
    /// arguments of `foo'; `f() { echo } ; }' — the first `}` is echo's
    /// argument; rubash#222).
    prev_accepts_close: bool,
    /// The next character begins a new word (previous char was whitespace
    /// or an operator token). A `}' with this false is glued into the
    /// current word (`hi}') and is word text (syntax.h:30: `}' is not in
    /// shell_break_chars).
    word_start: bool,
    /// The current word is pure unquoted identifier text so far; any
    /// quote, substitution, backslash or metacharacter makes the finished
    /// word ineligible as the reserved word that accepts a close.
    word_plain: bool,
    /// Close-acceptability tracker's own word accumulator (`_' letters,
    /// digits and `]' — enough to spell `fi' or `]]'), cleared whenever a
    /// word completes.
    close_word: String,
}

impl SkipBraceScan {
    fn new() -> Self {
        Self {
            depth: 1,
            case_depth: 0,
            word: String::new(),
            word_boundary: true,
            current_word_boundary: true,
            pending_comment: false,
            comment_start: false,
            comment_at: None,
            saw_top_level_whitespace: false,
            ansi_single: false,
            escaped: false,
            prev_accepts_close: true,
            // The scan starts just past the opening `{' token: the next
            // character begins the group's first word.
            word_start: true,
            word_plain: true,
            close_word: String::new(),
        }
    }

    fn from_resume(resume: BraceScanResume) -> Self {
        Self {
            depth: resume.depth,
            case_depth: resume.case_depth,
            word: resume.word,
            word_boundary: resume.word_boundary,
            current_word_boundary: resume.current_word_boundary,
            pending_comment: resume.pending_comment,
            comment_start: resume.comment_start,
            comment_at: resume.comment_at,
            saw_top_level_whitespace: resume.saw_top_level_whitespace,
            ansi_single: resume.ansi_single,
            escaped: resume.escaped,
            prev_accepts_close: resume.prev_accepts_close,
            word_start: resume.word_start,
            word_plain: resume.word_plain,
            close_word: resume.close_word,
        }
    }

    fn into_resume(self, pos: usize) -> BraceScanResume {
        BraceScanResume {
            pos,
            depth: self.depth,
            case_depth: self.case_depth,
            word: self.word,
            word_boundary: self.word_boundary,
            current_word_boundary: self.current_word_boundary,
            pending_comment: self.pending_comment,
            comment_start: self.comment_start,
            comment_at: self.comment_at,
            saw_top_level_whitespace: self.saw_top_level_whitespace,
            ansi_single: self.ansi_single,
            escaped: self.escaped,
            prev_accepts_close: self.prev_accepts_close,
            word_start: self.word_start,
            word_plain: self.word_plain,
            close_word: self.close_word,
        }
    }
}

/// Whether the text after a candidate closing `}' starts a new command (so
/// the `}' ends the group) — the reserved-word check of the compact-group
/// close. Returns `(answer, decided)`: `decided` is false when the answer
/// relied on the input ending (an exact reserved-word match at
/// end-of-input), which a later append could change.
fn brace_close_followed_by_reserved_word(rest: &str) -> (bool, bool) {
    const RESERVED: &[&str] = &["do", "done", "elif", "else", "esac", "fi", "then"];

    let mut answer = false;
    let mut decided = false;
    for word in RESERVED {
        if let Some(tail) = rest.strip_prefix(word) {
            match tail.chars().next() {
                Some(ch) => {
                    if ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>' | ')' | '(') {
                        answer = true;
                        decided = true;
                    }
                }
                None => {
                    // The reserved word ends the input: `true' today, but an
                    // appended separator or word byte decides differently.
                    answer = true;
                }
            }
        }
    }
    (answer, decided)
}

fn update_brace_group_case_depth(
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    rest: &str,
    lookahead_truncated: &mut bool,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if brace_group_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = update_brace_group_reserved_word_depth(
        word,
        *current_word_boundary,
        case_depth,
        ch,
        rest,
        lookahead_truncated,
    );
    word.clear();
    *word_boundary = reserved_word_allows_next || brace_group_separator_allows_reserved_word(ch);
}

fn update_brace_group_reserved_word_depth(
    word: &str,
    word_boundary: bool,
    case_depth: &mut usize,
    delimiter: char,
    rest: &str,
    lookahead_truncated: &mut bool,
) -> bool {
    if !word_boundary {
        return false;
    }

    match word {
        "case" => {
            *case_depth += 1;
            false
        }
        "esac" => {
            let (esac_is_pattern, truncated) = case_pattern_starts_with_esac_rest(delimiter, rest);
            if truncated {
                *lookahead_truncated = true;
            }
            if !esac_is_pattern {
                *case_depth = case_depth.saturating_sub(1);
                true
            } else {
                false
            }
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done" => true,
        _ => false,
    }
}

fn brace_group_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | '{' | '\n')
}

/// Words whose reserved-word token is in GNU's reserved_word_acceptable
/// list (parse.y:5903-5933), so a following standalone `}' (or `{' opener)
/// is a token rather than word text. NOTE: `in', `case', `for', `select'
/// and `function' are NOT in that list.
fn brace_close_acceptable_word(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "elif"
            | "else"
            | "fi"
            | "while"
            | "until"
            | "do"
            | "done"
            | "esac"
            | "time"
            | "coproc"
            | "]]"
    )
}

/// Stage tracker for the `case WORD in' chain, mirroring GNU
/// special_case_tokens (parse.y:3365):
///
/// - 0: idle.
/// - 1: `case' was recognized at a word boundary; the next word that
///   completes is the case SUBJECT (GNU: `reserved_word_acceptable(CASE)` is
///   false, parse.y:5899-5946 — the subject is a plain WORD).
/// - 2: the subject completed. GNU special_case_tokens rule 6
///   (parse.y:3369-3386): a word exactly `in' completing now is the IN token
///   (`last_read_token == WORD && token_before_that == CASE`), even though
///   the subject is not a reserved-word position.
/// - 3: directly after that IN. GNU parse.y:3433-3441: a word exactly `esac'
///   completing now is ESAC unconditionally ("case word in esac, which is a
///   legal construct" — the empty case; `esacs_needed_count` +
///   `last_read_token == IN`). Without this chain, `$(case z in esac)` never
///   balances its `)` because the `)` after `esac` is held back for a case
///   pattern list that the empty case never opens (rubash#284).
///
/// Only the same-line whitespace-separated chain is modeled: `case x\nin`
/// already reaches IN through the newline separator granting a boundary
/// (reserved_word_acceptable('\n'), parse.y:5903).
pub(super) fn update_command_substitution_case_depth(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    next: Option<char>,
    case_in_stage: &mut u8,
    case_pattern_region: &mut bool,
    prev_sig: Option<char>,
) {
    if single || double {
        word.clear();
        *word_boundary = false;
        return;
    }

    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if *case_depth > 0 {
            if ch == ')' && *case_pattern_region {
                // parse.y:3787-3788: `)` closes the pattern list.
                *case_pattern_region = false;
            } else if ch == ';' && matches!(next, Some(';') | Some('&')) {
                // parse.y:3710/3759: `;;`, `;&`, `;;&` start the next
                // pattern list.
                *case_pattern_region = true;
            }
            if *case_pattern_region && *case_in_stage == 3 && matches!(ch, '(' | '|') {
                *case_in_stage = 4;
            }
        }
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    // The word completing while `case' awaits its subject IS the subject
    // (stage 1 -> 2); specific arms below may then rewrite the stage.
    let completing_after_case = *case_in_stage == 1;
    if completing_after_case {
        *case_in_stage = 2;
    }
    let in_pattern_region = *case_pattern_region;
    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary && !in_pattern_region => {
            *case_depth += 1;
            *case_in_stage = 1;
            *case_pattern_region = false;
            false
        }
        "in" if *case_in_stage == 2 => {
            // GNU special_case_tokens rule 6 (parse.y:3369-3386): this `in'
            // follows the case subject, so it is the IN token even off a
            // reserved-word boundary (`in` after `case SUBJECT `).
            *case_in_stage = 3;
            // parse.y:3379/3396: the IN of a case arms the pattern region.
            *case_pattern_region = true;
            true
        }
        "esac" if *case_in_stage == 3 => {
            // GNU parse.y:3433-3441: `esac' directly after IN is ESAC —
            // the empty case `case WORD in esac'. Unconditional there, so
            // no case_pattern_starts_with_esac_rest guard on this arm: the
            // `)` right after `esac` is a stray top-level token, exactly
            // how GNU reports `case x in esac) echo hi;; esac` (syntax
            // error near unexpected token `)', verified vs WSL GNU 5.3.0).
            *case_depth = case_depth.saturating_sub(1);
            *case_in_stage = 0;
            *case_pattern_region = false;
            true
        }
        "esac" if *current_word_boundary => {
            // rubash#380 / parse.y:3177-3186 CHECK_FOR_RESERVED_WORD:
            // `esac' is pattern text ONLY when the previous token is `|'
            // (Posix rule 4) or the pattern-list `(' (phantom rule 4) —
            // witnessed by `prev_sig`, the last significant unquoted char
            // fed to this machine. Every other boundary `esac' (after
            // `;;', at a clause-body start) is the ESAC keyword. This
            // replaces the `)`-then-evidence FORWARD lookahead, whose
            // evidence could come from a construct outside the case
            // (`;; esac)` in a nested comsub).
            if matches!(prev_sig, Some('|') | Some('(')) {
                *case_in_stage = 0;
                false
            } else {
                *case_depth = case_depth.saturating_sub(1);
                *case_in_stage = 0;
                *case_pattern_region = false;
                true
            }
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary && !in_pattern_region =>
        {
            *case_in_stage = 0;
            true
        }
        _ => {
            if !completing_after_case {
                *case_in_stage = 0;
            }
            false
        }
    };
    // PST_CASEPAT port, word-completion half (parse.y:3787-3788): the
    // unquoted `)` terminates the pattern list even when it is GLUED to
    // the last pattern word (`case k in x) body`) — the word-empty branch
    // above never sees this `)` because it completes the word. Without
    // clearing here the pattern region leaks into the clause body, the
    // reserved-word arms (`for`..`done`, `!in_pattern_region`) stop
    // matching, their completing SPACE clears word_boundary, and the real
    // closing `esac` is never recognized as the keyword — the comsub scan
    // then runs past its closer and swallows the rest of the line
    // (rubash#405: `$(case k in x) for f in 1 2; do printf x; done esac)`
    // lost everything after the comsub). Same tail re-check as
    // parser/command_substitution.rs and the embedded_mutations region
    // machine; every continuation.rs twin already has it. The `;;`/`;&`
    // re-arm uses `next` — this tail's `;` is the FIRST char of the pair,
    // so the word-empty branch's second `;` alone would re-arm too late
    // for a `case`-text pattern right after `;;` (`(case|d)` would push a
    // phantom nested case when the region is still clear).
    if *case_depth > 0 {
        if ch == ')' && *case_pattern_region {
            *case_pattern_region = false;
        } else if ch == ';' && matches!(next, Some(';') | Some('&')) {
            *case_pattern_region = true;
        }
    }
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

/// rubash#380: the char counts as the PREVIOUS-TOKEN witness for the
/// `esac` pattern-text rule (parse.y:3181/3183, `last_read_token == '|'
/// or '('`). Word characters belong to the CURRENT token — GNU's
/// last_read_token is the previous completed token — so only operator /
/// separator characters (non-word, non-whitespace) update the witness.
pub(crate) fn esac_prev_token_char(ch: char) -> bool {
    !ch.is_whitespace() && !ch.is_ascii_alphanumeric() && ch != '_'
}

fn case_pattern_starts_with_esac_rest(delimiter: char, rest: &str) -> (bool, bool) {
    if !matches!(delimiter, ')' | '|') {
        return (false, false);
    }

    let chars = std::iter::once(delimiter)
        .chain(rest.chars())
        .collect::<Vec<_>>();
    let mut close = 0usize;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return (false, false),
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        // No `)` before the end of input: the answer was decided by EOF,
        // and more appended text could still supply the closer.
        return (false, true);
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            // `;;` right after `esac)` can be either a case-list separator
            // (esac is a pattern) or an arithmetic-for separator that lives
            // *outside* the command substitution (esac is the keyword and `)`
            // closes the `$(...)`).  GNU arith-for.tests:
            //   for (( $(case x in x) esac);; )); do break; done
            // After `;;`, a `)` (possibly following whitespace/newlines) closes
            // an enclosing `$(...)` or `(( ))`; that cannot be a case-list
            // context, so `esac` is the keyword, not a pattern.
            let mut after = scan + 2;
            while after < chars.len() && chars[after].is_whitespace() {
                after += 1;
            }
            if chars.get(after) == Some(&')') {
                return (false, false);
            }
            // Whitespace running to the end of input leaves the `)` check
            // undecided until more text arrives.
            return (true, after >= chars.len());
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return (true, false);
        }
        if ch == ')' {
            return (false, false);
        }
        if word.is_empty() {
            if command_substitution_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && command_substitution_reserved_word_allows_next(&word);
        word.clear();
        word_boundary =
            reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
        scan += 1;
    }
    // The trailing word ends at end-of-input: a longer input could extend
    // it into `esac' (or past it), so this answer is EOF-based.
    (word == "esac" && word_boundary, true)
}

fn command_substitution_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}

// ---------------------------------------------------------------------------
// Corrected command-substitution balance check
//
// `has_unclosed_command_substitution` (continuation.rs, captain-exclusive) has
// a false positive for `$(case x in x) esac)` when `esac)` is followed by `;;`
// outside the command substitution: the case-depth tracker thinks `esac` is a
// case pattern, not the keyword, so the closing `)` is never found.  The
// actual tokenizer (`skip_cmd_subst` above) uses the corrected
// `case_pattern_starts_with_esac_rest` and tokenizes the input correctly.
//
// These standalone functions provide a secondary balance check using the same
// corrected case-depth tracking, so `has_unclosed_input_syntax` (mod.rs) can
// override the continuation.rs false positive without editing continuation.rs.
// ---------------------------------------------------------------------------

/// Scan a `$((...))` arithmetic substitution starting just after `$((`
/// (i.e. at `start`), returning the index past the closing `))` or `None`.
fn skip_arith_substitution_corrected(chars: &[char], mut index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '\\' => {
                index += 2;
                continue;
            }
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' if chars.get(index + 1) == Some(&')') => {
                return Some(index + 2);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Scan a backtick command substitution starting at the opening backtick,
/// returning the index past the closing backtick or `None`.
fn skip_backtick_corrected(chars: &[char], mut index: usize) -> Option<usize> {
    index += 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '`' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

/// Standalone corrected `skip_parenthesized_unit`: given the char slice and
/// the index of the opening `(` (the one right after `$`), return the index
/// past the matching `)` or `None` if unbalanced.  Uses the corrected
/// `update_command_substitution_case_depth` / `case_pattern_starts_with_esac_rest`.
pub(crate) fn skip_parenthesized_unit_corrected(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    let mut parameter_depth = 0usize;
    // `case WORD in' chain tracker (GNU special_case_tokens,
    // parse.y:3369-3386 + 3433-3441) — see
    // update_command_substitution_case_depth.
    let mut case_in_stage = 0u8;
    let mut case_pattern_region = false;
    // rubash#380: previous significant unquoted char fed to the case
    // word machine (the `esac` previous-token witness,
    // parse.y:3181/3183).
    let mut prev_sig: Option<char> = None;
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word.is_empty()` alone is wrong: `$`, quotes and
    // other non-alphanumeric word characters never reach `word`, so
    // `$(echo $#)` and `$(echo 'a'#b)` would misread `#` as a comment.
    let mut token_boundary = true;
    while index < chars.len() {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        // Skip heredoc body so its `)` chars stay opaque.
        // GNU parse.y read_token: `<<<` is the here-string redirection
        // operator (LESS_LESS_LESS) whose operand is the next ordinary
        // word — there is no body to skip, and the comsub's `)` after the
        // word still closes the substitution (`$(cat <<< hi)`). Check it
        // BEFORE the `<<` heredoc arm: otherwise the second `<` of `<<<`
        // reads as `<<` and the heredoc delimiter scan swallows the
        // here-string word and the closing `)` to EOF, misreporting the
        // substitution as unbalanced (rubash#168).
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) == Some(&'<') {
            index += 3;
            token_boundary = true;
            continue;
        }
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') {
            let (next, _closes) =
                super::heredoc_scan::skip_heredoc_in_chars_with_closure(chars, index);
            index = next;
            // A heredoc terminator ends on its own line, so the next
            // character begins a fresh token.
            token_boundary = true;
            continue;
        }
        // `parameter_depth` keeps `${#x}` text out of the comment rule.
        if ch == '#' && token_boundary && parameter_depth == 0 {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            token_boundary = true;
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'{') {
            parameter_depth += 1;
            token_boundary = false;
            index += 2;
            continue;
        }
        if ch == '}' && parameter_depth > 0 {
            parameter_depth -= 1;
            token_boundary = false;
            index += 1;
            continue;
        }
        // The machine's only lookahead need is the char right after a `;`
        // (is this `;;`/`;&`, the pattern-list re-arm of parse.y:3710/3759)
        // — one indexed get, never a suffix materialization (the per-char
        // collect this replaces was O(span^2), rubash#241; the esac arm
        // has used the backward prev_sig witness since rubash#380).
        let next = chars.get(index + 1).copied();
        update_command_substitution_case_depth(
            ch,
            false,
            false,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            next,
            &mut case_in_stage,
            &mut case_pattern_region,
            prev_sig,
        );
        if esac_prev_token_char(ch) {
            // The opening `(` at `index == open` belongs to the construct,
            // not the body — GNU's last_read_token inside the body never
            // sees it (skip_cmd_subst enters past `$(` and cannot).
            if index > open {
                prev_sig = Some(ch);
            }
        }
        match ch {
            '\'' => single = true,
            '"' => double = true,
            // GNU read_token_word (parse.y:5377-5397): outside quotes a
            // backslash quotes the next character — it can never act as a
            // paren delimiter, so `$(echo \)` does not close the
            // substitution (comsub-posix.tests:42). The quoted character is
            // word text (a placeholder, since `c\ase` is not `case`), so a
            // following `#` stays mid-word (`\;#` in comsub1.sub); a quoted
            // newline is a line continuation, not word content.
            '\\' => {
                if chars.get(index + 1).is_some_and(|next| *next != '\n') {
                    word.push('\u{1}');
                }
                token_boundary = false;
                index += 2;
                continue;
            }
            '`' => {
                if let Some(end) = skip_backtick_corrected(chars, index) {
                    index = end;
                    token_boundary = false;
                    continue;
                }
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                index += 2;
                while index < chars.len() {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '\'' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
                token_boundary = false;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                if chars.get(index + 2) == Some(&'(') {
                    if let Some(end) = skip_arith_substitution_corrected(chars, index + 3) {
                        index = end;
                        token_boundary = false;
                        continue;
                    }
                } else if let Some(end) = skip_parenthesized_unit_corrected(chars, index + 1) {
                    index = end;
                    token_boundary = false;
                    continue;
                }
            }
            '(' if case_depth == 0 => depth += 1,
            ')' if case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        token_boundary =
            ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        index += 1;
    }
    None
}

/// Return `true` if every top-level `$(...)` command substitution in `input`
/// is balanced, using the corrected case-depth tracker.  This is a secondary
/// check used to override false positives from `has_unclosed_command_substitution`
/// (continuation.rs) without editing that captain-exclusive file.
pub(crate) fn command_substitutions_balanced(input: &str) -> bool {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\n' && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace() && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }
        if ch == '\'' && !double && !ansi_single {
            single = !single;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !single && !ansi_single {
            double = !double;
            comment_start = false;
            index += 1;
            continue;
        }
        if single {
            index += 1;
            continue;
        }
        // Skip ${...} parameter expansion so a `$(` inside it is not mistaken
        // for a top-level command substitution.
        if ch == '$' && chars.get(index + 1) == Some(&'{') && !double {
            // rubash#281 (perf4 shape; same fix the captain landed in
            // continuation.rs): the String copy re-collected the ENTIRE
            // remaining input per `${` — O(tail^2) per call. The zero-copy
            // chars API (scan_braced_parameter_body_chars, rubash#185)
            // requires the `${` opener at slice[0], so the slice INCLUDES
            // it; `scan.end` is the CHAR count of the body past the `}`.
            let body = &chars[index..];
            let context = super::dolbrace::BraceContext {
                outer_double_quote: double,
                posix: false,
                replacement_context: false,
                initial_state: super::dolbrace::DolbraceState::Param,
            };
            if let Some(scan) = super::dolbrace::scan_braced_parameter_body_chars(body, context) {
                index += 2 + scan.end;
                comment_start = false;
                continue;
            }
            // Unterminated ${...}: fall through; the input is unclosed.
            return false;
        }
        // Skip backtick command substitution.
        if ch == '`' && !double {
            if let Some(end) = skip_backtick_corrected(&chars, index) {
                index = end;
                comment_start = false;
                continue;
            }
            return false;
        }
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') {
            if let Some(end) = skip_parenthesized_unit_corrected(&chars, index + 1) {
                index = end;
                comment_start = false;
                continue;
            }
            // Check for $((...)) arithmetic.
            if chars.get(index + 2) == Some(&'(') {
                if let Some(end) = skip_arith_substitution_corrected(&chars, index + 3) {
                    index = end;
                    comment_start = false;
                    continue;
                }
            }
            // Genuinely unbalanced command substitution.
            return false;
        }
        // perf19: top-level heredoc — the body is raw text (GNU
        // make_cmd.c:512 make_here_document reads it line by line with no
        // paren state; parse.y:3120 gather_here_documents, parse.y:3557
        // read_token stream past the terminator), so the body's parens
        // never feed this balance scan. Same arm as the parked balanced
        // scan.
        if !single
            && !double
            && !ansi_single
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) != Some(&'<')
        {
            if let Some((next, _terminator_found)) =
                super::heredoc_scan::skip_heredoc_top_level(&chars, index)
            {
                index = next;
                comment_start = true;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one operator, consumed
        // atomically so its second `<` is never a `<<` opener.
        if !single
            && !double
            && !ansi_single
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if !single && !double && !ansi_single {
            comment_start = false;
        }
        index += 1;
    }
    true
}

/// GNU parse.y:5635-5643 read_token_word: an unquoted `[` opens an array
/// subscript — `parse_matched_pair (cd, '[', ']', ..., P_ARRAYSUB)` — when
/// the word so far is a pure identifier at a command position
/// (assignment_acceptable), or at element start inside a compound
/// assignment (`name=(`, PST_COMPASSIGN). parse_matched_pair
/// (parse.y:3906-3912) then reads across lines until the matching `]`;
/// EOF reports `unexpected EOF while looking for matching `]'' at the line
/// the `[` opened (rubash#221: `foo=([)` was silently accepted). GNU exits
/// 1 when the subscript opened inside a compound array assignment
/// (parse_compound_assignment EOF family, like `foo=(]`) and 2 for a
/// command word (`a[b`). `echo a[b` stays literal — `a` is an argument,
/// not a command-position identifier; `x=a[b` has no identifier prefix
/// (`a` after `=` fails token_is_ident) and stays an assignment value.
pub fn unclosed_array_subscript_line(input: &str) -> Option<(usize, bool)> {
    unclosed_array_subscript_walk(input, false)
        .map(|(line, _closer, _construct_line, compassign)| (line, compassign))
}

/// The subscript-EOF diagnostic router (rubash#390 q2): GNU
/// parse_matched_pair recurses on an unescaped `'`/`"` (and `$'`) inside a
/// `P_ARRAYSUB` scan (parse.y:4040-4051, `$'` arm at 4046-4047), and the
/// recursion's own EOF report names the QUOTE as the closer at the line the
/// quote opened (start_lineno, parse.y:3901-3912) — `a["x]=15` reports
/// `"'`, not `]' (WSL GNU 5.3.0 probes: `a[\n"x` → line 2 `"`,
/// `a[$'x]` → line 1 `'`). Exit status follows the same context rule as
/// the `]' shape (parse_compound_assignment EOF family → 1, command word
/// → 2). Returns (report_line, closer, construct_line, inside_compassign):
/// `construct_line` is where the `[` scan itself opened — the COMPLETE
/// commands GNU already ran stop there, so the diagnostic driver's prefix
/// cut must use it, not the (possibly later) report line. Same walk as
/// `unclosed_array_subscript_line` with the quote-open bookkeeping
/// surfaced.
pub fn unclosed_array_subscript_eof(input: &str) -> Option<(usize, char, usize, bool)> {
    unclosed_array_subscript_walk(input, true)
}

fn unclosed_array_subscript_walk(
    input: &str,
    quote_aware: bool,
) -> Option<(usize, char, usize, bool)> {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut in_comment = false;
    // Command-position tracking (parse.y:5899 assignment_acceptable): true
    // at input start, after a separator/operator or `('/`{', and after the
    // reserved words that may be followed by a command.
    let mut command_position = true;
    let mut word = String::new();
    // Compound-assignment `name=( ... )` depth (PST_COMPASSIGN); at element
    // start (right after `(` or whitespace inside the list) a `[` opens a
    // subscript with no identifier prefix needed.
    let mut compassign_depth = 0usize;
    let mut element_start = false;
    let mut line = 1usize;
    // Runtime `extended_glob` mirror (rubash#317). GNU parses scripts
    // incrementally — each command is executed before the next line is
    // read — so a bare top-level `shopt -s extglob` has already run by the
    // time a later `+(a|b[)*` word is parsed (parse.y:5466 gates the group
    // consumption on the live `extended_glob`). Track those commands so
    // extglob pattern groups are consumed as word units, exactly like
    // read_token_word does.
    let mut extglob = false;
    let mut line_start = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                line += 1;
                command_position = true;
                word.clear();
                element_start = false;
                extglob = shopt_line_sets_extglob(&chars[line_start..index], extglob);
                line_start = index + 1;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        // perf19: top-level heredoc — the body is raw text (GNU
        // make_cmd.c:512 make_here_document reads it line by line with no
        // subscript state; parse.y:3120 gather_here_documents,
        // parse.y:3557 read_token stream past the terminator), so the
        // body's `[`/`]`/parens never feed this scan. Same arm as the
        // parked subscript scan; the jumped newlines still advance the
        // diagnostic line counter.
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') {
            if let Some((next, _terminator_found)) =
                super::heredoc_scan::skip_heredoc_top_level(&chars, index)
            {
                let jumped = chars[index..next].iter().filter(|c| **c == '\n').count();
                line += jumped;
                command_position = true;
                word.clear();
                element_start = false;
                index = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one operator, consumed
        // atomically so its second `<` is never a `<<` opener.
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) == Some(&'<') {
            index += 3;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                word.clear();
                index += 1;
                continue;
            }
            '#' if word.is_empty() => {
                in_comment = true;
                index += 1;
                continue;
            }
            '\'' => {
                single = true;
                word.clear();
                index += 1;
                continue;
            }
            '"' => {
                double = true;
                word.clear();
                index += 1;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                ansi_single = true;
                word.clear();
                index += 2;
                continue;
            }
            _ => {}
        }
        if ch.is_whitespace() {
            if ch == '\n' {
                line += 1;
                command_position = true;
                word.clear();
                element_start = false;
                extglob = shopt_line_sets_extglob(&chars[line_start..index], extglob);
                line_start = index + 1;
            } else {
                // `if`, `then`, `while`, ... keep the next word in command
                // position — but only when they themselves stood at command
                // position (`echo if a[b` keeps `a` an argument); a command
                // word like `echo` ends it. Whitespace directly after a
                // delimiter (`; `) keeps the delimiter's decision.
                if !word.is_empty() {
                    command_position = command_position && is_command_position_boundary(&word);
                }
                word.clear();
                if compassign_depth > 0 {
                    element_start = true;
                }
            }
            index += 1;
            continue;
        }
        // GNU parse.y:5464-5490 (read_token_word): while `extended_glob`
        // is live, a PATTERN_CHAR (`@*+?!`, syntax.h:90) immediately
        // followed by `(` hands the whole balanced group to
        // parse_matched_pair and appends it to the token verbatim — the
        // body's `[`, `|`, `(`, `)` never participate in token-level
        // decisions. Mirror that here so `echo +(a|b[)*` (rubash#317) is
        // not misread as a top-level `|` + `b[` subscript hunt. An
        // unbalanced group is a `)`-shaped EOF error owned by the generic
        // close-char scan, not a `]` error — bail out to it.
        if extglob
            && matches!(ch, '@' | '*' | '+' | '?' | '!')
            && chars.get(index + 1) == Some(&'(')
        {
            match skip_extglob_group_chars(&chars, index + 1) {
                Some(end) => {
                    word.push(ch);
                    word.push('(');
                    index = end;
                    continue;
                }
                None => return None,
            }
        }
        // `[` opens a subscript when the word prefix is a pure shell
        // identifier at command position (parse.y:5637), or we are at
        // element start inside a compound assignment (parse.y:5638,
        // token_index == 0 && PST_COMPASSIGN).
        if ch == '['
            && ((command_position && is_pure_identifier(&word))
                || (compassign_depth > 0 && element_start && word.is_empty()))
        {
            // parse_matched_pair ('[', ']'): quote/escape aware, nested
            // `[ ... ]` pairs nest, newlines are consumed by the scan.
            let mut depth = 1usize;
            let mut scan = index + 1;
            let mut scan_line = line;
            let mut q_single = false;
            let mut q_double = false;
            let mut q_ansi = false;
            let mut q_escaped = false;
            let mut quote_line = line;
            while scan < chars.len() {
                let c = chars[scan];
                if c == '\n' {
                    scan_line += 1;
                }
                if q_escaped {
                    q_escaped = false;
                    scan += 1;
                    continue;
                }
                if q_single || q_ansi {
                    // parse.y:3990-3995: inside a `'`-pair a backslash is
                    // literal data; only the closing `'` ends the recursion
                    // (P_ALLOWESC handles `\'` inside `$'...')`.
                    if c == '\'' {
                        q_single = false;
                        q_ansi = false;
                    }
                    scan += 1;
                    continue;
                }
                if q_double {
                    if c == '\\' {
                        q_escaped = true;
                    } else if c == '"' {
                        q_double = false;
                    }
                    scan += 1;
                    continue;
                }
                match c {
                    '\\' => q_escaped = true,
                    '\'' => {
                        q_single = true;
                        quote_line = scan_line;
                    }
                    '"' => {
                        q_double = true;
                        quote_line = scan_line;
                    }
                    '$' if chars.get(scan + 1) == Some(&'\'') => {
                        q_ansi = true;
                        quote_line = scan_line;
                        scan += 1;
                    }
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                scan += 1;
            }
            if depth > 0 {
                // EOF inside the subscript: report at the `[` line
                // (parse.y:3906 start_lineno), with the compound-assignment
                // context deciding the exit status (1 inside `name=(`).
                if quote_aware && (q_single || q_double || q_ansi) {
                    // parse.y:4040-4051: the unterminated quote's own
                    // parse_matched_pair recursion hit EOF first — its
                    // closer (the quote char) and its start_lineno (the
                    // quote's open line) own the report. The prefix cut
                    // still uses the `[` line: no complete command exists
                    // after the construct opened.
                    return Some((
                        quote_line,
                        if q_double { '"' } else { '\'' },
                        line,
                        compassign_depth > 0,
                    ));
                }
                return Some((line, ']', line, compassign_depth > 0));
            }
            index = scan + 1;
            word.clear();
            element_start = false;
            command_position = false;
            continue;
        }
        if ch == '(' {
            // GNU read_token hands a `((` at a command position (empty word)
            // to parse_dparen (parse.y:3726-3733) -> parse_arith_cmd, whose
            // P_ARITH matched-pair scan consumes the balanced body as ONE
            // arithmetic token when the next character is `)` (parse.y:4976)
            // — a `[` inside that body is plain arith data (no P_ARRAYSUB
            // scan ever runs there), so the subscript walk must not look
            // inside (`((a[b))` parses and the EVALUATOR reports
            // `a[b' with status 1, rubash#390 q1 probes k1/k2). A `((`
            // whose group is not followed by `)` is the nested-subshell
            // reinterpretation: fall through to the ordinary `(' handling
            // (its pushed body re-lexes to the same text shape).
            if word.is_empty() && command_position && chars.get(index + 1) == Some(&'(') {
                if let Some(close) = paren_group_close(&chars, index + 2) {
                    if chars.get(close + 1) == Some(&')') {
                        index = close + 2;
                        word.clear();
                        element_start = false;
                        command_position = false;
                        continue;
                    }
                }
            }
            // `name=(` opens a compound-assignment list; any other `(` is a
            // subshell/grouping whose body starts a fresh command position.
            if word.ends_with('=') {
                compassign_depth += 1;
                element_start = true;
            } else {
                command_position = true;
            }
            word.clear();
            index += 1;
            continue;
        }
        if ch == ')' {
            compassign_depth = compassign_depth.saturating_sub(1);
            element_start = false;
            command_position = true;
            word.clear();
            index += 1;
            continue;
        }
        if matches!(ch, ';' | '&' | '|' | '{' | '}') {
            command_position = true;
            word.clear();
            element_start = false;
            index += 1;
            continue;
        }
        if ch == '`' {
            let mut scan = index + 1;
            while scan < chars.len() && chars[scan] != '`' {
                if chars[scan] == '\\' {
                    scan += 1;
                }
                scan += 1;
            }
            index = (scan + 1).min(chars.len());
            word.clear();
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'(') {
            // Command-substitution body: its internals own their scans in
            // the recursive parse; skip the balanced unit.
            if let Some(end) = skip_parenthesized_unit_corrected(&chars, index + 1) {
                index = end;
                word.clear();
                continue;
            }
        }
        word.push(ch);
        index += 1;
    }
    None
}

fn is_pure_identifier(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Shape of an unclosed `$( ...` command substitution at end of the eval
/// string (rubash#318). GNU parses the eval string as fresh parser input
/// (evalstring.c:357 with_input_from_string) continuing the caller's line
/// counter (evalstring.c:345-346 `line_number--`, no SEVAL_RESETLINE for
/// eval), so an EOF-class failure inside a comsub reports under
/// `<script>: eval: line N:` with two distinct shapes:
///
/// * `BareParen` — a subshell `(` that follows body text is immediately
///   followed by a newline or end of input. The comsub-body yyparse
///   (parse.y:4549) sees the offending newline token and
///   report_syntax_error prints `syntax error near unexpected token
///   \`newline' while looking for matching \`)'` (parse.y:6858-6859,
///   shell_eof_token == ')' from parse.y:4519) plus the offending source
///   line (parse.y:6865 print_offending_line), at the `(' line.
///   Non-interactive shells then exit 1 via parse.y:4588-4596
///   (r != 0 -> last_command_exit_value = EXECUTION_FAILURE,
///   jump_to_top_level(FORCE_EOF)). A `(` directly after the `$(` opener
///   (`$( (`) instead ends as a matched-pair EOF (parse.y:4576-4587) —
///   verified probe m8 — and falls to the plain shape.
/// * `NeverClosed` — everything else: `$(` at end of input (empty or
///   whitespace-only body) or a body that consumed input but never saw its
///   `)`. matched_pair_error propagates (parse.y:4586) and the plain
///   `unexpected EOF while looking for matching \`)'` wording reports at
///   one past the last input line of the string (caller + newlines + 1;
///   pinned vs WSL GNU 5.3.0 probes m3/m7/m9/m5/m8/m10), with eval's
///   status 2 and the script continuing.
///
/// The eval-string unclosed-paren classification, GNU Bash 5.3 taxonomy.
/// Every variant's message wording, report line, and abort class was
/// byte-verified against WSL GNU Bash 5.3.0 (probes p1-p28,
/// 2026-09-28 snaprec lane). The second tuple member returned by
/// [`unclosed_comsub_eof_shape`] is the string-relative line at which the
/// parse stopped — complete commands on earlier lines already ran
/// (evalstring.c:359-451 parses and executes command by command).
pub enum UnclosedComsubShape {
    /// `$( ...body... (` — a yacc token error inside a comsub body (the `(`
    /// follows body text; the grammar shifts it as a possible function
    /// definition and then chokes on the NEXT token). Message:
    /// "syntax error near unexpected token `<tok>' while looking for
    /// matching `)'" plus the offending source line, both at the `('s line
    /// (parse.y:6858-6865 with shell_eof_token == ')' from parse.y:4519).
    /// Abort: the comsub's own yyparse fails (r != 0) and parse.y:4588-4596
    /// jumps FORCE_EOF — exit 1, nothing after the eval runs, with NO
    /// `command'-builtin gate (verified: `command eval '$(echo ('` also
    /// dies, rc 1).
    ComsubBodyTokenError { line: usize, token: String },
    /// Unclosed command substitution at EOF — `$( `, `$(`, or `$( (` (a `(`
    /// directly after the opener opens a legal nested subshell, GNU probe
    /// m8). Message: "unexpected EOF while looking for matching `)'" at ONE
    /// PAST the last input line (parse.y:6891). Abort: the EOF_Reached arm
    /// of parse_comsub (parse.y:4576-4587) does NOT jump — the error is
    /// contained to the eval string; only the posix ERREXIT gate
    /// (evalstring.c:588-595) can exit, and `command eval` suppresses even
    /// that (executing_command_builtin, execute_cmd.c:4735).
    ComsubEof { line: usize },
    /// Plain subshell `(` in command position at the eval-string main level
    /// (`( `, `( hi`, `echo a\n( hi`). Message: "syntax error: unexpected
    /// end of file from `(' command on line N" — the bash-5.3 compoundcmd
    /// report (y.tab.c:9258-9260; yyerror's EOF branch fires this shape
    /// because a plain `(` leaves shell_eof_token == 0 and pushes a
    /// compoundcmd_lineno record). Reported at ONE PAST the last input line
    /// with N = the `('s line. Abort: same contained/ERREXIT gate as
    /// ComsubEof (errors8.sub: `command eval '( '` must keep running the
    /// script — GNU prints ok 1..ok 8).
    PlainSubshellEof { line: usize, open_line: usize },
    /// `(` in an invalid position at the eval-string main level (after a
    /// complete word: `echo (`, `echo ((`, `echo (hi`). Message: "syntax
    /// error near unexpected token `<tok>'" plus the offending line, at the
    /// `('s line — yyerror branch 1 (y.tab.c:9208-9215) WITHOUT the
    /// "while looking for matching" suffix, because shell_eof_token is 0
    /// outside a comsub. The named token is the one AFTER the `(` (the
    /// function-definition shift consumes the paren first). Abort: the
    /// failing yyparse is the MAIN one, so parse_and_execute's else arm
    /// (evalstring.c:584-600) applies the posix ERREXIT gate — no
    /// unconditional exit.
    MainLevelTokenError { line: usize, token: String },
    /// A `(` inside a compound-assignment list (`x=((`): parse_compound_
    /// assignment expects WORDs, so any `(` is a token error naming itself
    /// (parse.y:7141-7151, yyerror branch 1 with current_token == '(').
    /// Abort: parse.y:7173-7179 — posix non-interactive shells jump
    /// FORCE_EOF with status 1 (no `command' gate; GNU `command eval
    /// 'x=('` posix dies rc 1), otherwise the DISCARD arm contains it and
    /// eval returns 1.
    CompassignTokenError { line: usize },
}

/// Classify the unclosed-paren shape of an eval string the way GNU's parser
/// does, or return None to defer to the generic close-char scan (the `x=(`
/// compound-assignment EOF family and the `(( `/`$(( ` arithmetic roots keep
/// their own report lines there).
///
/// GNU structure being ported (all line references are the vendored
/// bash.git @b4608166 tree):
/// - a `(` immediately after `$` opens a command substitution; after `=` a
///   compound assignment; anything else is a plain subshell paren;
/// - inside a comsub, the body's own yyparse owns the diagnostic: a token
///   error exits 1 through FORCE_EOF (parse.y:4588-4596), an EOF returns
///   matched_pair_error without jumping (parse.y:4576-4587);
/// - at the eval-string main level the main yyparse fails into
///   evalstring.c:584-600, whose only exit is the posix ERREXIT gate —
///   never a reader abort (push_stream/pop_stream clear EOF_Reached per
///   stream, parse.y:1908/1915, so an eval-string parse error cannot stop
///   the outer script by itself).
pub fn unclosed_comsub_eof_shape(input: &str) -> Option<(UnclosedComsubShape, usize)> {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;
    // (char index of `(`, string-relative open line, opener kind)
    let mut stack: Vec<(usize, usize, OpenerKind)> = Vec::new();
    let mut line = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' => single = true,
            '"' => double = true,
            '\n' => line += 1,
            '`' => {
                // Old-style comsub: skip its balanced body.
                index += 1;
                while index < chars.len() {
                    match chars[index] {
                        '\\' => index += 1,
                        '`' => break,
                        '\n' => line += 1,
                        _ => {}
                    }
                    index += 1;
                }
            }
            '(' => {
                // parse.y:7173-7179: inside a compound-assignment word list
                // every `(` is a token error naming itself — `x=((`.
                if matches!(stack.last().map(|e| e.2), Some(OpenerKind::Compassign)) {
                    return Some((UnclosedComsubShape::CompassignTokenError { line }, line));
                }
                let prev = if index > 0 {
                    Some(chars[index - 1])
                } else {
                    None
                };
                let kind = match prev {
                    Some('$') => OpenerKind::Comsub,
                    Some('=') => OpenerKind::Compassign,
                    _ => OpenerKind::Plain,
                };
                stack.push((index, line, kind));
            }
            ')' => {
                stack.pop();
            }
            _ => {}
        }
        index += 1;
    }
    let &(open_idx, open_line, open_kind) = stack.last()?;
    let newlines = input.matches('\n').count();

    // `x=(` is the compound-assignment EOF family — its own report/exit
    // rules live in the generic close-char scan.
    if open_kind == OpenerKind::Compassign {
        return None;
    }

    // The first `(` that does not open in command position is where the
    // grammar fails: after a complete word it is shifted as a possible
    // function-definition paren and the NEXT token becomes the yacc error
    // (`echo (` -> `newline', `echo ((` -> `(', `echo (hi` -> `hi'). This
    // check precedes the arithmetic exemption: `echo ((` is this class
    // (GNU probe p16), while `(( `/`$(( ` in command position are not.
    let invalid = stack
        .iter()
        .find(|&&(idx, _, kind)| {
            kind == OpenerKind::Plain && !opens_in_command_position(&chars, idx)
        })
        .map(|&(idx, line, _)| (idx, line));
    if let Some((idx, line)) = invalid {
        let inside_comsub = stack
            .iter()
            .any(|&(enc_idx, _, kind)| enc_idx < idx && kind == OpenerKind::Comsub);
        let token = token_after_paren(&chars, idx);
        return if inside_comsub {
            Some((
                UnclosedComsubShape::ComsubBodyTokenError { line, token },
                line,
            ))
        } else {
            Some((
                UnclosedComsubShape::MainLevelTokenError { line, token },
                line,
            ))
        };
    }

    // `(( `/`$(( ` arithmetic roots: matched-pair constructs with their
    // own report/exit rules — leave them to the generic close-char scan
    // (GNU probes p27/p28: both report at the OPEN line, which is the
    // generic path's behavior).
    if open_idx > 0 && chars[open_idx - 1] == '(' {
        return None;
    }

    // EOF shapes. A comsub anywhere in the enclosing chain keeps the
    // comsub EOF wording (probe m8: `$( (` is a matched-pair EOF inside the
    // comsub's own yyparse, shell_eof_token == ')').
    let any_comsub = stack.iter().any(|&(_, _, kind)| kind == OpenerKind::Comsub);
    if any_comsub {
        return Some((
            UnclosedComsubShape::ComsubEof { line: newlines + 1 },
            open_line,
        ));
    }
    // Plain subshell at the main level: the bash-5.3 compoundcmd EOF report.
    Some((
        UnclosedComsubShape::PlainSubshellEof {
            line: newlines + 1,
            open_line,
        },
        open_line,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenerKind {
    /// `$(`
    Comsub,
    /// `name=(`
    Compassign,
    /// any other `(`
    Plain,
}

/// Does the `(` at `idx` open in command position? Scanning backwards over
/// horizontal whitespace, a boundary character (start of input, newline,
/// `;`, `&`, `|`, or an enclosing `(`) means a command may start here;
/// anything else (a word character, `)`, `=`, a redirection operator, a
/// closing quote) means the `(` follows body text — an invalid position.
fn opens_in_command_position(chars: &[char], idx: usize) -> bool {
    let mut i = idx;
    while i > 0 {
        i -= 1;
        match chars[i] {
            ' ' | '\t' | '\r' => continue,
            '\n' | ';' | '&' | '|' | '(' => return true,
            _ => return false,
        }
    }
    true
}

/// The token a yacc error names for a `(` in invalid position: the grammar
/// shifts the paren (function-definition candidate) and fails on the NEXT
/// token — EOF/newline is reported as `newline' (GNU probe p14: `eval
/// 'echo ('` names `newline' though the string has none), another `(` as
/// `(', otherwise the following word's text (p21: `echo (hi' names `hi').
fn token_after_paren(chars: &[char], open_idx: usize) -> String {
    let mut i = open_idx + 1;
    while i < chars.len() && matches!(chars[i], ' ' | '\t' | '\r') {
        i += 1;
    }
    if i >= chars.len() || matches!(chars[i], '\n') {
        return "newline".to_string();
    }
    if chars[i] == '(' {
        return "(".to_string();
    }
    let start = i;
    while i < chars.len()
        && !matches!(
            chars[i],
            ' ' | '\t' | '\r' | '\n' | '(' | ')' | ';' | '&' | '|' | '<' | '>' | '`'
        )
    {
        i += 1;
    }
    chars[start..i].iter().collect()
}

/// Quote/backslash-aware balanced-paren scan for an extglob pattern group
/// body (GNU parse_matched_pair, parse.y:3877: quotes nest, backslash
/// escapes, nested `(`/`)` count). `open` is the index of the `(`; returns
/// the index past its matching `)` or `None` when EOF is reached first.
fn skip_extglob_group_chars(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = open + 1;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let c = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if single {
            if c == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if c == '\\' {
                escaped = true;
            } else if c == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '\'' => single = true,
            '"' => double = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Runtime `extended_glob` mirror for the text-level subscript pre-scan:
/// given one physical line's chars, return the updated extglob state. GNU
/// reads a script incrementally and executes each command before parsing
/// the next (so a `shopt -s extglob` line has already run when later lines
/// are parsed); only bare top-level `shopt` commands on this line are
/// recognized — `;`-separated segments each get a chance, quoted or
/// indirected flips stay out of scope and keep the previous state.
fn shopt_line_sets_extglob(line: &[char], current: bool) -> bool {
    let text: String = line.iter().collect();
    let mut result = current;
    for segment in text.split(';') {
        let mut tokens = segment.split_whitespace();
        if tokens.next() != Some("shopt") {
            continue;
        }
        let mut set: Option<bool> = None;
        for tok in tokens {
            if tok.starts_with('-') && tok != "--" && tok.len() > 1 {
                for flag in tok.chars().skip(1) {
                    match flag {
                        's' => set = Some(true),
                        'u' => set = Some(false),
                        _ => {}
                    }
                }
                continue;
            }
            if tok == "extglob" {
                if let Some(value) = set {
                    result = value;
                }
                // A later `-s`/`-u` + `extglob` on the same line wins.
                set = None;
            }
        }
    }
    result
}

fn is_command_position_boundary(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "elif"
            | "else"
            | "fi"
            | "while"
            | "until"
            | "do"
            | "done"
            | "esac"
            | "!"
            | "time"
            | "coproc"
            | "{"
            | "}"
    )
}

/// Length (in chars, ending just past the closer) of an extglob pattern
/// group whose `(` sits at `open`, consumed the way GNU read_token_word's
/// extglob arm does (parse.y:5466-5475): parse_matched_pair over the
/// `(...)` span with its own quote/backslash state and nesting count.
/// Returns `None` when the group never closes. The PATTERN_CHAR admission
/// (`@ * + ? !`, syntax.h:90-92) and the extglob gate are judged by the
/// callers — this is only the balanced-span walker.
pub(crate) fn extglob_pattern_group_len(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = open + 1;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match ch {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Length (in chars, ending just past the closer) of the `[...]` array
/// subscript span whose `[` sits at `chars[0]`, consumed the way GNU
/// read_token_word's element arm does under PST_COMPASSIGN
/// (parse.y:5635-5651): parse_matched_pair('[', ']', P_ARRAYSUB) with its
/// own quote/backslash state and nested `[` pairs, the span copied into
/// the word by strcpy (parse.y:5645-5647) — spaces inside are word DATA,
/// never element boundaries. Returns `None` when the `]` never comes (the
/// read-time EOF family owns that diagnostic; the splitters only walk
/// already-admitted bodies). The element-leading admission (token_index ==
/// 0 under PST_COMPASSIGN, parse.y:5637) is judged by the callers.
pub(crate) fn arraysub_span_len(chars: &[char]) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match ch {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' if !single && !double => depth += 1,
            ']' if !single && !double => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// rubash#390 q1: GNU read_token hands a `((` at a command position to
/// parse_dparen (parse.y:3726-3733) -> parse_arith_cmd (parse.y:4970), whose
/// parse_matched_pair(`(', `)', P_ARITH) scan counts parens across lines
/// with quote recursions (parse.y:4040-4051) and backslash escapes
/// (parse.y:3997-3998); EOF with the count still positive is the
/// `unexpected EOF while looking for matching `)'' report at the line the
/// group opened (parse.y:3908-3915, start_lineno) with exit 2 (the
/// `error yacc_EOF' production forces EX_BADUSAGE only when the status is
/// still 0 - parse.y:484-490).
///
/// Class A probe set (`((1+2', `((1+(2', `((x=(', `((x=()'): rc 2, `)' at
/// the `((` line. This walker answers one question for the diagnostic
/// router: given the 1-based `open_line` of an unclosed command `(`, does
/// that line start a `((` whose P_ARITH group NEVER closes before end of
/// input?
/// rubash#435: does `input` (the text read so far) end inside an OPEN
/// command-position `((` arithmetic group? GNU parse.y:3726-3733 read_token
/// hands a reserved-word-acceptable `((` to parse_dparen (parse.y:4895),
/// whose parse_matched_pair (P_ARITH) scan (parse.y:4963-4978) reads across
/// physical newlines — read_secondary_line keeps pulling lines until the
/// group closes. The batch tokenizer's logical-line completeness decision
/// therefore needs the same question the EOF diagnostic asks
/// (`dparen_arith_group_never_closes`, rubash#390 q1), asked about the
/// whole accumulated text instead of one line: a `((` whose group has not
/// closed as arithmetic yet means the construct is incomplete and the next
/// physical line belongs to it.
///
/// The scan mirrors parse_dparen's character rules: quote spans and
/// `$(` / `${` / `$[` units are opaque (parse.y:4138-4186
/// parse_dollar_word consumes them; their parens never count), a `#` in
/// command position comments to end of line (parse.y:3922), and a `((`
/// that closes WITHOUT the trailing `)` is the nested-subshell
/// reinterpretation (parse.y:4938-4948) — the reader then continues
/// normally, so only a group that never closes keeps the line open.
pub(crate) fn dparen_arith_group_open_at_end(input: &str) -> bool {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut index = 0usize;
    // GNU reserved_word_acceptable (parse.y:5899): start of input and the
    // positions after `;` `&` `|` `(` and newlines accept the `((` dispatch;
    // blanks keep the current answer.
    let mut command_position = true;
    while index < len {
        match chars[index] {
            '\\' => {
                index += 2;
                command_position = false;
                continue;
            }
            '\'' => {
                index += 1;
                while index < len && chars[index] != '\'' {
                    index += 1;
                }
                command_position = false;
            }
            '"' => {
                index += 1;
                while index < len {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '"' {
                        break;
                    }
                    index += 1;
                }
                command_position = false;
            }
            '`' => {
                index += 1;
                while index < len {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '`' {
                        break;
                    }
                    index += 1;
                }
                command_position = false;
            }
            '$' if matches!(chars.get(index + 1), Some('(' | '{' | '[')) => {
                let (open, close) = match chars[index + 1] {
                    '(' => ('(', ')'),
                    '{' => ('{', '}'),
                    _ => ('[', ']'),
                };
                // parse_dollar_word consumes the unit; its interior never
                // shifts command position. An unterminated unit (a `$((`
                // whose parens are still open — `(( c=$((a+1),` mid-body) is
                // itself an incomplete construct: keep reading until the
                // dedicated substitution gates can settle it.
                match dollar_word_group_len(&chars, index + 1, open, close) {
                    Some(end) => {
                        index = end;
                        command_position = false;
                        continue;
                    }
                    None => return true,
                }
            }
            '#' if command_position => {
                // parse.y:3922: word-initial `#` runs to end of line; the
                // newline after it restores command position.
                while index < len && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            '(' => {
                if command_position && chars.get(index + 1) == Some(&'(') {
                    return match paren_group_close(&chars, index + 2) {
                        // Group never closes in the text read so far: the
                        // construct is incomplete — keep reading.
                        None => true,
                        Some(close) => {
                            // parse.y:4976: a `)` right after the matched
                            // group makes this a closed arithmetic command;
                            // the reader resumes past `))`. Anything else is
                            // the subshell reinterpretation and continues
                            // after the group's own closer.
                            if chars.get(close + 1) == Some(&')') {
                                index = close + 2;
                            } else {
                                index = close + 1;
                            }
                            command_position = false;
                            continue;
                        }
                    };
                }
                command_position = false;
            }
            '\n' | ';' | '&' | '|' => command_position = true,
            c if c.is_whitespace() => {}
            _ => command_position = false,
        }
        index += 1;
    }
    false
}

pub(crate) fn dparen_arith_group_never_closes(input: &str, open_line: usize) -> bool {
    let chars: Vec<char> = input.chars().collect();
    let Some(start) = dparen_open_index(&chars, open_line) else {
        return false;
    };
    paren_group_close(&chars, start + 2).is_none()
}

/// rubash#390 q1 class B: the P_ARITH group CLOSES but the next character
/// is not `)` - GNU reinterprets the `((` as a nested subshell
/// (parse.y:4938-4948) whose pushed body re-lexes as `( span-minus-closer
/// )'. When that reparse leaves a compound assignment `name=(` still open
/// at a CLEAN EOF, parse_compound_assignment's yacc_EOF arm reports
/// `unexpected EOF while looking for matching `)'' at the line the
/// compound opened (parse.y:7151-7152, orig_line_number - the `((` line
/// for a same-line construct) and sets EXECUTION_FAILURE
/// (parse.y:7167-7169) - exit 1, which the `error yacc_EOF' production
/// keeps (parse.y:489-490 only forces 2 when the status is still 0).
/// Probe set (`((X=([))]`, `((X=([))]x`): rc 1, `)' at line 1. Contrast
/// shapes that stay with the existing wording: an unterminated `[` array
/// subscript inside the compound reports `]' (parse.y:3908-3915 with
/// close=`]', e.g. `((x=([y))`), and a fully-closed inner subshell leaves
/// only the outer `(` open (parse.y:6892-6901, e.g. `((x=(y))`).
pub(crate) fn dparen_subshell_reparse_compound_eof(input: &str, open_line: usize) -> bool {
    let chars: Vec<char> = input.chars().collect();
    let Some(start) = dparen_open_index(&chars, open_line) else {
        return false;
    };
    // The P_ARITH span starts just past the second `(` of `((` and includes
    // its balancing `)`.
    let Some(close) = paren_group_close(&chars, start + 2) else {
        return false;
    };
    // parse_arith_cmd:4976 - the character after the matched group decides;
    // `)` means a real arithmetic command, everything else the subshell
    // reinterpretation.
    if chars.get(close + 1) == Some(&')') {
        return false;
    }
    // The pushed body: parse.y:4999-5002 - '(' + span without its last
    // character + ')' (the span's closer becomes the subshell's closer).
    let span = &chars[start + 2..=close];
    let mut body: Vec<char> = Vec::with_capacity(chars.len());
    body.push('(');
    body.extend_from_slice(&span[..span.len() - 1]);
    body.push(')');
    body.extend_from_slice(&chars[close + 1..]);

    // Re-lex the body the way the parser does: the leading `(` opens the
    // inner subshell; `name=(` opens a compound assignment
    // (parse.y:5653-5658); a `[` at element start consumes the matched
    // `[...]` as word data (parse.y:5635-5651) - across the pushed-string
    // boundary into the real input tail; a `)` at element position closes
    // the compound (parse.y:7140's loop exit).
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut paren_depth = 1usize; // the pushed body's own leading `(`
    let mut compassign_depth = 0usize;
    let mut word = String::new();
    let mut element_start = true;
    let mut index = 0usize;
    while index < body.len() {
        let ch = body[index];
        if escaped {
            escaped = false;
            word.push(ch);
            index += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                word.clear();
            }
            '\'' => {
                single = true;
                word.clear();
            }
            '"' => {
                double = true;
                word.clear();
            }
            c if c.is_whitespace() => {
                word.clear();
                if compassign_depth > 0 {
                    element_start = true;
                }
            }
            '(' => {
                // `name=(` opens a compound assignment only when the word so
                // far is an identifier ending at `=` (parse.y:5653).
                if word.ends_with('=')
                    && !word[..word.len() - 1].is_empty()
                    && word[..word.len() - 1]
                        .chars()
                        .all(|c| c == '_' || c.is_ascii_alphanumeric())
                {
                    compassign_depth += 1;
                    element_start = true;
                } else {
                    paren_depth += 1;
                }
                word.clear();
            }
            ')' => {
                if compassign_depth > 0 {
                    compassign_depth -= 1;
                } else if paren_depth > 0 {
                    paren_depth -= 1;
                }
                word.clear();
                element_start = false;
            }
            '[' if compassign_depth > 0 && element_start && word.is_empty() => {
                // parse.y:5637: element-leading `[` under PST_COMPASSIGN -
                // P_ARRAYSUB scan over `[...]` (quotes, escapes and nested
                // `[` pairs, parse.y:3877+). An unterminated scan is the
                // `]' report shape, not this class.
                let mut sub_depth = 1usize;
                let mut scan = index + 1;
                let mut q_single = false;
                let mut q_double = false;
                let mut q_escaped = false;
                while scan < body.len() {
                    let c = body[scan];
                    if q_escaped {
                        q_escaped = false;
                        scan += 1;
                        continue;
                    }
                    if q_single {
                        if c == '\'' {
                            q_single = false;
                        }
                        scan += 1;
                        continue;
                    }
                    if q_double {
                        if c == '\\' {
                            q_escaped = true;
                        } else if c == '"' {
                            q_double = false;
                        }
                        scan += 1;
                        continue;
                    }
                    match c {
                        '\\' => q_escaped = true,
                        '\'' => q_single = true,
                        '"' => q_double = true,
                        '[' => sub_depth += 1,
                        ']' => {
                            sub_depth -= 1;
                            if sub_depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    scan += 1;
                }
                if sub_depth > 0 {
                    // Unterminated subscript inside the reparse: the `]'
                    // family owns the report (probe `((x=([y))`).
                    return false;
                }
                index = scan;
                word.clear();
                element_start = false;
            }
            c => {
                word.push(c);
                if c != '=' && !(c == '_' || c.is_ascii_alphanumeric()) {
                    word.clear();
                }
                element_start = false;
            }
        }
        index += 1;
    }
    // Clean EOF with the compound still open: parse.y:7140-7152.
    compassign_depth > 0
}

/// Index of the first `((` on the 1-based `open_line` of `chars`, if any.
fn dparen_open_index(chars: &[char], open_line: usize) -> Option<usize> {
    let mut line = 1usize;
    let mut index = 0usize;
    while index < chars.len() && line < open_line {
        if chars[index] == '\n' {
            line += 1;
        }
        index += 1;
    }
    loop {
        if index >= chars.len() || chars[index] == '\n' {
            return None;
        }
        if chars[index] == '(' && chars.get(index + 1) == Some(&'(') {
            return Some(index);
        }
        index += 1;
    }
}

/// parse_matched_pair(`(', `)', P_ARITH) from `from`: quote recursions,
/// backslash escapes, `$(`/`${`/`$[` groups (parse.y:4131-4145). Returns
/// the index of the balancing `)`, or None when the group never closes.
fn paren_group_close(chars: &[char], from: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut scan = from;
    let mut single = false;
    let mut double = false;
    let mut backtick = false;
    let mut escaped = false;
    while scan < chars.len() {
        let ch = chars[scan];
        if escaped {
            escaped = false;
            scan += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            scan += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                double = false;
            }
            scan += 1;
            continue;
        }
        if backtick {
            if ch == '\\' {
                escaped = true;
            } else if ch == '`' {
                backtick = false;
            }
            scan += 1;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' => single = true,
            '"' => double = true,
            '`' => backtick = true,
            '$' if matches!(chars.get(scan + 1), Some('(' | '{' | '[')) => {
                let (open, close) = match chars[scan + 1] {
                    '(' => ('(', ')'),
                    '{' => ('{', '}'),
                    _ => ('[', ']'),
                };
                match dollar_word_group_len(chars, scan + 1, open, close) {
                    Some(end) => scan = end,
                    None => return None,
                }
                continue;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(scan);
                }
            }
            _ => {}
        }
        scan += 1;
    }
    None
}

/// Balanced span of a `$(`/`${`/`$[` group whose opener sits at `open`
/// (parse.y parse_dollar_word → parse_matched_pair with the pair's own
/// open/close). Returns the index just past the closer, or None when it
/// never closes. Quote/escape aware like the P_ARITH walker.
pub(crate) fn dollar_word_group_len(
    chars: &[char],
    open: usize,
    open_ch: char,
    close_ch: char,
) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = open + 1;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' => single = true,
            '"' => double = true,
            c if c == open_ch => depth += 1,
            c if c == close_ch => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}
