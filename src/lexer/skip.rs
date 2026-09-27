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
            let rest = &self.input[self.position..];
            update_command_substitution_case_depth(
                c,
                false,
                false,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
                rest,
            );
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
                    // echo's argument) (rubash#222).
                    let standalone_closer = scan.word_start && scan.prev_accepts_close;
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
                            Some('}') => {
                                scan.depth = 1;
                                continue;
                            }
                            // A close decided without consulting past the
                            // current end of input is stable under later
                            // appends and can be cached; a close at
                            // end-of-input is not (the appended bytes could
                            // be the `}' of a `}}' cascade or a compact-group
                            // terminator).
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

fn update_command_substitution_case_depth(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    rest: &str,
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
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary => {
            *case_depth += 1;
            false
        }
        "esac" if *current_word_boundary && !case_pattern_starts_with_esac_rest(ch, rest).0 => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary =>
        {
            true
        }
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
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
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word.is_empty()` alone is wrong: `$`, quotes and other
    // non-alphanumeric word characters never reach `word`, so `$(echo $#)`
    // and `$(echo 'a'#b)` would misread `#` as a comment.
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
        let rest: String = chars[index..].iter().collect();
        update_command_substitution_case_depth(
            ch,
            false,
            false,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            // `rest` begins at `ch`: advance by its UTF-8 width, not a fixed
            // byte — multibyte chars here panicked on the byte slice
            // (niubash#139 `"${v}$(echo 中)"`).
            &rest[ch.len_utf8()..],
        );
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
            let body: String = chars[index + 2..].iter().collect();
            let context = super::dolbrace::BraceContext {
                outer_double_quote: double,
                posix: false,
                replacement_context: false,
                initial_state: super::dolbrace::DolbraceState::Param,
            };
            if let Some(scan) = super::dolbrace::scan_braced_parameter_body(&body, context) {
                index += 2 + body[..scan.end].chars().count();
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

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                line += 1;
                command_position = true;
                word.clear();
                element_start = false;
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
            let mut q_single = false;
            let mut q_double = false;
            let mut q_escaped = false;
            while scan < chars.len() {
                let c = chars[scan];
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
                return Some((line, compassign_depth > 0));
            }
            index = scan + 1;
            word.clear();
            element_start = false;
            command_position = false;
            continue;
        }
        if ch == '(' {
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
