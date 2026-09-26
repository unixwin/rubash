use super::classification::{is_brace_expansion, is_word_delimiter};
use super::quotes::normalize_backtick_command_substitution;
use super::token::{Token, TokenKind};

/// GNU reader state that survives across logical lines: parse.y keeps a
/// single parser_state for the whole input, so last_read_token and
/// PST_CASEPAT naturally span physical/logical line boundaries. Rubash
/// re-lexes each logical line, so the state is carried explicitly.
#[derive(Clone, Default)]
pub(super) struct LexerParseState {
    /// GNU parse.y last_read_token / token_before_that, kept as
    /// (kind, value) so `reserved_word_position` can reproduce
    /// reserved_word_acceptable (parse.y:5899) for the `{` fold decision.
    last_token: Option<(TokenKind, String)>,
    token_before_that: Option<(TokenKind, String)>,
    /// GNU PST_CASEPAT (parser.h:29): inside a case pattern list reserved
    /// words are ordinary word text (parse.y:3177 suppresses every reserved
    /// word except a pattern-position esac), so `{` is a pattern character,
    /// not a group opener.
    case_pattern: bool,
    /// Depth of open `case` statements so `;;`/`;&`/`;;&` re-enter pattern
    /// state for the next clause (parse.y:3710/3759 set PST_CASEPAT).
    case_stmt_depth: usize,
    /// `case` just read: the grammar expects the operand word next
    /// (parse.y expecting_in_command == CASE).
    case_expect_word: bool,
    /// `case WORD` read: an `in` (or newline then `in`) enters pattern state
    /// (parse.y:3370-3396).
    case_expect_in: bool,
    /// GNU lexes `-p' directly after `time' as TIMEOPT and `--' after
    /// `time'/`time -p' as TIMEIGN (parse.y:3470-3479); both are in
    /// reserved_word_acceptable's list (parse.y:5929-5930), so
    /// `time -p { echo; }' keeps `{' a group opener. True when last_token
    /// was such an option word.
    last_was_time_option: bool,
}

impl LexerParseState {
    /// GNU read_token reads the newline that ends a logical line as a real
    /// '\n' token, so the first token of the next line is lexed with
    /// last_read_token == '\n' — which reserved_word_acceptable
    /// (parse.y:5902) accepts: `{ echo; }` after a complete command on the
    /// previous line is a group. The per-line tokenizer's synthetic `;`
    /// separator is emitted downstream of the Lexer, so the line break must
    /// be folded into the carried state explicitly; without it the state
    /// kept the previous line's last word and `{` degraded to word text.
    pub(super) fn note_line_break(&mut self) {
        let previous = self.last_token.take();
        self.token_before_that = previous;
        self.last_token = Some((TokenKind::Semicolon, ";".to_string()));
    }
}

pub(super) struct Lexer<'a> {
    pub(super) input: &'a str,
    pub(super) position: usize,
    /// POSIX parse mode (Austin Group Interp 221): single quotes inside a
    /// double-quoted `${...}` are literal, so `}` closes the expansion.
    pub(super) posix: bool,
    parse_state: LexerParseState,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(input: &'a str, posix: bool) -> Self {
        Self {
            input,
            position: 0,
            posix,
            parse_state: LexerParseState::default(),
        }
    }

    /// Carry the GNU reader state in from a previous logical line
    /// (parse.y keeps one parser_state for the whole input).
    pub(super) fn restore_parse_state(&mut self, state: LexerParseState) {
        self.parse_state = state;
    }

    /// Export the GNU reader state after this logical line so the next
    /// logical line resumes where the parser would have (PST_CASEPAT,
    /// last_read_token, expecting_in_command).
    pub(super) fn take_parse_state(&mut self) -> LexerParseState {
        self.parse_state.clone()
    }

    /// GNU parse.y:5899 reserved_word_acceptable: whether a reserved word
    /// (here `{`) may appear after the previously emitted token.
    fn reserved_word_position(&self) -> bool {
        let Some((kind, value)) = &self.parse_state.last_token else {
            return true;
        };
        match kind {
            TokenKind::Semicolon
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Background
            // DOLPAREN / DOLBRACE in the GNU list.
            | TokenKind::CommandSubst
            | TokenKind::Variable => true,
            TokenKind::Keyword => matches!(
                value.as_str(),
                "(" | ")"
                    | "{"
                    | "}"
                    | "!"
                    | "do"
                    | "done"
                    | "elif"
                    | "else"
                    | "esac"
                    | "fi"
                    | "if"
                    | "then"
                    | "time"
                    | "while"
                    | "until"
                    | "coproc"
            ),
            // `;;`, `;&`, `;;&` tokenize as Word (SEMI_SEMI family are in the
            // GNU list); a word after `function`/`coproc` also qualifies;
            // TIMEOPT/TIMEIGN (`time -p' / `time --') are in the GNU list
            // too (parse.y:5929-5930).
            TokenKind::Word => {
                matches!(value.as_str(), ";;" | ";&" | ";;&")
                    || self.parse_state.last_was_time_option
                    || matches!(&self.parse_state.token_before_that,
                        Some((TokenKind::Keyword, v)) if v == "function" || v == "coproc")
            }
            _ => false,
        }
    }

    /// Track the GNU reader state that affects `{` handling:
    /// reserved-word acceptability (last two tokens) and PST_CASEPAT.
    fn record_token(&mut self, token: &Token) {
        let keyword_is = |v: &str| token.kind == TokenKind::Keyword && token.value == v;
        let word_is = |v: &str| token.kind == TokenKind::Word && token.value == v;
        let is_case_operand = matches!(
            token.kind,
            TokenKind::Word
                | TokenKind::Variable
                | TokenKind::CommandSubst
                | TokenKind::Assignment
                | TokenKind::BraceExpand
        ) && !word_is(";;")
            && !word_is(";&")
            && !word_is(";;&");

        // `case` is a reserved word only in command position; counting it
        // unconditionally would turn `echo case x in {)` into pattern state.
        if self.reserved_word_position() && keyword_is("case") {
            self.parse_state.case_stmt_depth += 1;
            self.parse_state.case_expect_word = true;
        } else if self.parse_state.case_expect_word {
            // GNU expecting_in_command == CASE: the operand must be a word
            // token (parse.y:3370 token_before_that == CASE && "in").
            self.parse_state.case_expect_word = false;
            if is_case_operand {
                self.parse_state.case_expect_in = true;
            }
        } else if self.parse_state.case_expect_in {
            // parse.y:3390: `in` is also recognized after a newline
            // following the case word.
            if keyword_is("in") || word_is("in") {
                self.parse_state.case_pattern = true;
                self.parse_state.case_expect_in = false;
            } else if token.kind != TokenKind::Semicolon {
                self.parse_state.case_expect_in = false;
            }
        }
        if self.parse_state.case_pattern && keyword_is(")") {
            // parse.y:3787-3788: the `)` ending a pattern list leaves
            // PST_CASEPAT; `;;`/family below re-enter it for the next clause.
            self.parse_state.case_pattern = false;
        }
        if self.parse_state.case_stmt_depth > 0
            && (word_is(";;") || word_is(";&") || word_is(";;&"))
        {
            self.parse_state.case_pattern = true;
        }
        if self.parse_state.case_stmt_depth > 0 && (keyword_is("esac") || word_is("esac")) {
            self.parse_state.case_stmt_depth -= 1;
            self.parse_state.case_pattern = false;
            self.parse_state.case_expect_in = false;
        }

        // TIMEOPT/TIMEIGN recognition (parse.y:3470-3479): `-p' after the
        // `time' keyword and `--' after `time'/`time -p' are dedicated
        // tokens, not plain words — computed against the PREVIOUS token
        // before last_token shifts.
        self.parse_state.last_was_time_option = token.kind == TokenKind::Word
            && matches!(
                (token.value.as_str(), &self.parse_state.last_token),
                ("-p", Some((TokenKind::Keyword, kw))) if kw == "time"
            )
            || token.kind == TokenKind::Word
                && token.value == "--"
                && matches!(
                    &self.parse_state.last_token,
                    Some((TokenKind::Keyword, kw)) if kw == "time"
                )
            || token.kind == TokenKind::Word
                && token.value == "--"
                && matches!(
                    &self.parse_state.last_token,
                    Some((TokenKind::Word, prev)) if prev == "-p"
                );

        self.parse_state.token_before_that = self.parse_state.last_token.take();
        self.parse_state.last_token = Some((token.kind.clone(), token.value.clone()));
    }

    #[inline]
    pub(super) fn at_end(&self) -> bool {
        self.position >= self.input.len()
    }

    #[inline]
    pub(super) fn peek(&self) -> Option<char> {
        if self.at_end() {
            None
        } else {
            self.input[self.position..].chars().next()
        }
    }

    #[inline]
    pub(super) fn peek_after(&self, offset: usize) -> Option<char> {
        self.input[self.position..].chars().nth(offset)
    }

    #[inline]
    pub(super) fn advance(&mut self) -> Option<char> {
        if self.at_end() {
            None
        } else {
            let c = self.input[self.position..].chars().next()?;
            self.position += c.len_utf8();
            Some(c)
        }
    }

    pub(super) fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.advance();
            } else {
                break;
            }
        }
    }

    pub(super) fn slice(&self, start: usize) -> &str {
        let end = self.position.min(self.input.len());
        &self.input[start..end]
    }

    pub(super) fn next_token(&mut self) -> Option<Token> {
        self.skip_ws();
        if self.at_end() {
            return Some(Token::new(TokenKind::Eof, "", self.position));
        }

        let start = self.position;
        let c = self.advance()?;

        match c {
            '\r' => {
                if self.peek() == Some('\n') {
                    self.advance();
                }
                let mut token = Token::new(TokenKind::Semicolon, ";", start);
                token.line_break = true;
                Some(token)
            }
            '\n' => {
                let mut token = Token::new(TokenKind::Semicolon, ";", start);
                token.line_break = true;
                Some(token)
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::Or, "||", start))
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::PipeErr, "|&", start))
                } else {
                    Some(Token::new(TokenKind::Pipe, "|", start))
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::And, "&&", start))
                } else if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        Some(Token::new(TokenKind::Append, "&>>", start))
                    } else {
                        Some(Token::new(TokenKind::RedirectOut, "&>", start))
                    }
                } else if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else {
                    Some(Token::new(TokenKind::Background, "&", start))
                }
            }
            '(' | ')' => Some(Token::new(TokenKind::Keyword, self.slice(start), start)),
            '!' => {
                if self.peek() == Some('=') {
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else if self.peek() == Some('(') {
                    Some(self.finish_word_token(start, false))
                } else if self
                    .peek()
                    .is_some_and(|ch| !is_word_delimiter(ch) && !ch.is_whitespace())
                {
                    // `!!`, `!2`, `!$`, history word designators and `!foo` are
                    // words when not isolated as the `!` pipeline negation
                    // keyword. Splitting `!!` into two `!` keywords makes
                    // `eval echo '!!'` print `! !` (histexp).
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else {
                    Some(Token::new(TokenKind::Keyword, "!", start))
                }
            }
            ';' => {
                if self.peek() == Some(';') {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        Some(Token::new(TokenKind::Word, ";;&", start))
                    } else {
                        Some(Token::new(TokenKind::Word, ";;", start))
                    }
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::Word, ";&", start))
                } else {
                    Some(Token::new(TokenKind::Semicolon, ";", start))
                }
            }
            '<' => match self.peek() {
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('<') {
                        self.advance();
                        Some(Token::new(TokenKind::HereString, "<<<", start))
                    } else if self.peek() == Some('-') {
                        self.advance();
                        Some(Token::new(TokenKind::HereDoc, "<<-", start))
                    } else {
                        Some(Token::new(TokenKind::HereDoc, "<<", start))
                    }
                }
                Some('>') => {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, "<>", start))
                }
                Some('&') => {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectIn, "<&", start))
                }
                _ => Some(Token::new(TokenKind::RedirectIn, "<", start)),
            },
            '>' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Some(Token::new(TokenKind::Append, ">>", start))
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, ">&", start))
                } else if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, ">|", start))
                } else {
                    Some(Token::new(TokenKind::RedirectOut, ">", start))
                }
            }
            '0'..='9' if self.peek().is_some_and(|ch| ch.is_ascii_digit()) => {
                Some(self.finish_number_token(start))
            }
            '0'..='9' if c != '2' && self.peek() == Some('>') => {
                self.advance();
                if self.peek() == Some('>') {
                    self.advance();
                    Some(Token::new(TokenKind::Append, self.slice(start), start))
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, self.slice(start), start))
                } else if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, self.slice(start), start))
                } else {
                    Some(Token::new(TokenKind::RedirectOut, self.slice(start), start))
                }
            }
            '0'..='9' if c != '2' && self.peek() == Some('<') => {
                Some(self.finish_prefixed_input_redirect(start))
            }
            '2' => {
                if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErrAppend, "2>>", start))
                    } else if self.peek() == Some('&') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErr, "2>&", start))
                    } else if self.peek() == Some('|') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErr, "2>|", start))
                    } else {
                        Some(Token::new(TokenKind::RedirectErr, "2>", start))
                    }
                } else if self.peek() == Some('<') {
                    Some(self.finish_prefixed_input_redirect(start))
                } else {
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                }
            }
            '#' => {
                while self.advance().is_some_and(|ch| ch != '\n') {}
                self.next_token()
            }
            '$' => match self.peek() {
                Some('\'') => {
                    self.advance();
                    self.skip_ansi_c_single();
                    Some(self.finish_word_token(start, false))
                }
                Some('"') => {
                    self.advance();
                    self.skip_double();
                    Some(self.finish_word_token(start, false))
                }
                Some('(') => {
                    self.advance();
                    if self.peek() == Some('(') {
                        self.advance();
                        self.skip_arith_paren();
                    } else {
                        self.skip_cmd_subst();
                    }
                    if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(
                        TokenKind::CommandSubst,
                        self.slice(start),
                        start,
                    ))
                }
                Some('{') => {
                    self.advance();
                    self.skip_braced(false);
                    // A brace group adjacent to a closed parameter expansion
                    // continues the same word (parse.y read_token_word keeps
                    // scanning until a real metacharacter): "${a}{x,y}" is
                    // one word whose brace group then expands. A `}` right
                    // after the close is literal word text too (GNU probe:
                    // `echo ${x-d{}}` is the single word d{}; splitting it
                    // off made the trailing brace a standalone word).
                    if self
                        .peek()
                        .is_some_and(|ch| !is_word_delimiter(ch) || ch == '{' || ch == '}')
                    {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(TokenKind::Variable, self.slice(start), start))
                }
                Some('[') => {
                    self.advance();
                    self.skip_arith_bracket();
                    if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                }
                _ => {
                    let pos = self.position;
                    self.skip_word_at(start);
                    if !is_simple_parameter_tail(self.slice(pos)) {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(
                        TokenKind::Variable,
                        &format!("${}", self.slice(pos)),
                        start,
                    ))
                }
            },
            '`' => {
                self.skip_backtick();
                if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                    return Some(self.finish_word_token(start, false));
                }
                let raw = self.slice(start);
                let value = normalize_backtick_command_substitution(raw);
                Some(Token::new_with_raw(
                    TokenKind::CommandSubst,
                    &value,
                    raw,
                    start,
                ))
            }
            '\'' => {
                self.skip_single();
                Some(self.finish_word_token(start, false))
            }
            '"' => {
                self.skip_double();
                Some(self.finish_word_token(start, false))
            }
            '\\' => {
                self.advance();
                Some(self.finish_word_token(start, false))
            }
            '{' => {
                if self.brace_group_contains_heredoc_operator() {
                    return Some(Token::new(TokenKind::Keyword, "{", start));
                }
                // GNU parse.y: `{` is the group reserved word only where
                // reserved_word_acceptable (parse.y:5899) and outside a
                // case pattern list (PST_CASEPAT, parse.y:3177). Everywhere
                // else `{` is an ordinary word character (read_token_word):
                // `case x in {)` keeps `{` as the pattern instead of
                // folding `{...}` across the enclosing function's `}` and
                // silently dropping every definition in between (the
                // git-completion.bash `__git_aliased_command` clause).
                if self.parse_state.case_pattern || !self.reserved_word_position() {
                    let mut token = self.finish_word_token(start, false);
                    // `{a,b}`-family words still brace-expand anywhere
                    // (expansion is orthogonal to reserved-word status).
                    if token.kind == TokenKind::Word && is_brace_expansion(&token.raw) {
                        token.kind = TokenKind::BraceExpand;
                    }
                    return Some(token);
                }
                let scan = self.skip_brace();
                if !scan.closed {
                    // GNU parse.y read_token: `{` is an ordinary word
                    // character (it is not in shell_break_chars,
                    // syntax.h:30); it becomes the reserved word '{' only
                    // when the whole token is `{' and the grammar is in
                    // command position (reserved_word_acceptable,
                    // parse.y:5899). A group whose `}' is not on this
                    // logical line must not be folded into one token: emit
                    // just the opener and rescan the rest of the line as
                    // ordinary tokens, so the parser pairs the `{` with a
                    // `}' from a later logical line
                    // (matching_brace_group_end) -- the path multiline
                    // `f() {' bodies already take. Without this,
                    // `f() { echo x;' + `}' and `f() { echo x; # note' + `}'
                    // died as "unexpected end of file from `{' command"
                    // even though GNU reads on to the close brace
                    // (niubash#130). Only a standalone `{' qualifies: the
                    // byte after the brace must end the word the way GNU's
                    // break set ends a token, because `{a,b'/`{#note' are
                    // single words there.
                    if self.input[start + 1..]
                        .chars()
                        .next()
                        .map_or(true, |ch| "()<>;&| \t\n\r".contains(ch))
                    {
                        self.position = start + 1;
                        return Some(Token::new(TokenKind::Keyword, "{", start));
                    }
                    if let Some(comment_start) = scan.comment_start {
                        // A word-initial `#' at the group's top level is a
                        // comment, not group text (parse.y read_token ->
                        // parse_comment). When the logical line ends inside the
                        // group -- `f() { # note' is the whole first logical
                        // line -- slicing to end of input used to fuse the
                        // comment into the token (`{ # note'), which then
                        // failed the parser's `value == "{"' body gate and
                        // degraded into `syntax error near unexpected token
                        // `('' (niubash #120 follow-up). Emit only the group
                        // text seen before the comment and rescan from the
                        // comment, which the dispatcher then drops like any
                        // other comment.
                        let value = self.input[start..comment_start].trim_end().to_string();
                        let kind = if is_brace_expansion(&value) {
                            TokenKind::BraceExpand
                        } else {
                            TokenKind::Keyword
                        };
                        self.position = comment_start;
                        return Some(Token::new(kind, &value, start));
                    }
                }
                if self
                    .peek()
                    .is_some_and(|ch| !is_word_delimiter(ch) || ch == '{')
                {
                    return Some(self.finish_word_token(start, false));
                }
                let v = self.slice(start);
                let kind = if is_brace_expansion(v) {
                    TokenKind::BraceExpand
                } else if self.input[start + 1..]
                    .chars()
                    .next()
                    .is_some_and(|ch| !"()<>;&| \t\n\r".contains(ch))
                {
                    // GNU parse.y: `{' is a reserved word only as a
                    // standalone token — `{xxx}` (brace followed by a word
                    // character) is an ordinary word, never a group
                    // opener. The unclosed scan above applies the same
                    // break-set test to decide whether `{' qualifies.
                    TokenKind::Word
                } else {
                    TokenKind::Keyword
                };
                Some(Token::new(kind, v, start))
            }
            '}' => Some(Token::new(TokenKind::Keyword, "}", start)),
            _ => Some(self.finish_word_token(start, true)),
        }
    }

    fn brace_group_contains_heredoc_operator(&self) -> bool {
        let chars = self.input[self.position..].chars().collect::<Vec<_>>();
        let mut index = 0usize;
        let mut depth = 1usize;
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
            if single || double {
                index += 1;
                continue;
            }

            match ch {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return false;
                    }
                }
                '<' if chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') => {
                    return true;
                }
                _ => {}
            }
            index += 1;
        }

        false
    }

    fn finish_prefixed_input_redirect(&mut self, start: usize) -> Token {
        if self.peek_after(1) == Some('<') {
            return self.finish_number_token(start);
        }

        self.advance();
        if matches!(self.peek(), Some('>' | '&')) {
            self.advance();
        }
        let kind = if self.slice(start).ends_with("<>") {
            TokenKind::RedirectOut
        } else {
            TokenKind::RedirectIn
        };
        Token::new(kind, self.slice(start), start)
    }
}

fn is_simple_parameter_tail(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if matches!(first, '?' | '$' | '!' | '@' | '*' | '#' | '-') {
        return chars.next().is_none();
    }

    if first.is_ascii_digit() {
        return chars.next().is_none();
    }

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    fn next(&mut self) -> Option<Self::Item> {
        let token = self.next_token();
        if let Some(token) = &token {
            // Eof is not a real GNU token: last_read_token stays the last
            // real token, which the next logical line's reserved-word
            // decisions must see (the `{` fold checks it).
            if token.kind != TokenKind::Eof {
                self.record_token(token);
            }
        }
        token
    }
}
