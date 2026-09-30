use super::brace_scan_cache::{BraceScanCache, HeredocOpEntry, HeredocOpScanResume};
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
    /// GNU PST_CONDCMD (parser.h:33): set while the conditional command's
    /// tokens are being read (parse.y:3654 on `[[', cleared by `]]').
    in_cond_command: bool,
    /// GNU PST_REGEXP (parser.h:38): armed after the `=~' operator word of
    /// a conditional (parse.y:5169) and cleared once the RHS word has been
    /// read (parse.y:5212). Rubash's tokenizer splits that one GNU word
    /// into several tokens (`(', fragments, `)'); these three fields model
    /// the still-open RHS word: whether its first fragment was consumed,
    /// the open `('/`)' depth inside it, and where the previous token ended
    /// (a fragment directly abutting the previous token at depth 0 still
    /// belongs to the word — `[[ a =~ (b)#c ]]' keeps `#c' data).
    cond_rhs_regexp: bool,
    cond_rhs_started: bool,
    cond_rhs_paren_depth: i32,
    cond_rhs_last_token_end: usize,
}

/// Snapshot of every cross-token Lexer field at a byte offset where a pass
/// over an accumulating logical line ended BETWEEN tokens (rubash#281
/// complete-command-boundary checkpoint). GNU reads its input once, token
/// by token (parse.y:3557 read_token): the reader state after the tokens
/// of a prefix is exactly the state a longer input continues from. The
/// batch tokenizer's per-join full re-lex is the deviation; resuming a
/// Lexer from this snapshot reproduces the tail tokens the full pass
/// would emit, provided the prefix ended at a clean token boundary.
///
/// Boundary cleanliness is decided by the producer (tokenize loop in
/// `mod.rs`): quotes, command substitutions, compound assignments and
/// parameter expansions all closed, no open `(`/`((` group, no pending
/// extglob split (every one of those states means the final token was
/// flushed mid-scan at end-of-input and would tokenize differently in the
/// longer text), and the logical line has only grown by appends since.
#[derive(Clone)]
pub(super) struct LexerBoundaryState {
    parse_state: LexerParseState,
    last_token_end: Option<usize>,
    last_token_was_open_paren: bool,
}

impl<'a> Lexer<'a> {
    /// Snapshot the between-token state, or `None` when the lexer stopped
    /// inside an open construct (see the struct docs).
    pub(super) fn boundary_state(&self) -> Option<LexerBoundaryState> {
        if !self.open_parens.is_empty() || self.extglob_split_pending {
            return None;
        }
        Some(LexerBoundaryState {
            parse_state: self.parse_state.clone(),
            last_token_end: self.last_token_end,
            last_token_was_open_paren: self.last_token_was_open_paren,
        })
    }

    /// Resume scanning `input` at byte `position` with a previously
    /// captured between-token state (same discipline as
    /// `restore_parse_state`, plus the intra-line `(`-adjacency fields).
    pub(super) fn new_resumed_at(
        input: &'a str,
        posix: bool,
        brace_cache: &'a mut BraceScanCache,
        position: usize,
        state: &LexerBoundaryState,
    ) -> Self {
        let mut lexer = Self::new_with_cache(input, posix, brace_cache);
        lexer.position = position.min(input.len());
        lexer.parse_state = state.parse_state.clone();
        lexer.last_token_end = state.last_token_end;
        lexer.last_token_was_open_paren = state.last_token_was_open_paren;
        lexer
    }
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
    /// Parse-time extglob gate (rubash#131): GNU read_token_word consumes
    /// `?(`/`*(`/`+(`/`@(`/`!(` pattern groups only while the `extglob`
    /// shopt is on (parse.y:5466 `extended_glob && PATTERN_CHAR`); without
    /// it the `(` is an ordinary token (a case pattern then fails with
    /// "syntax error near unexpected token `('").
    pub(super) extended_glob: bool,
    parse_state: LexerParseState,
    /// Resumable brace-group scan continuations shared by every re-tokenize
    /// pass over one accumulating logical line (rubash#176/#178). `None`
    /// for one-shot Lexer uses that get no reuse anyway.
    pub(super) brace_cache: Option<&'a mut BraceScanCache>,
    /// Open `(` groups of the logical line being scanned, innermost last.
    /// An entry is an arithmetic-command candidate when it is the first
    /// `(` of an adjacent `((` pair; `arith_close_verified` memoizes the
    /// parse_dparen-style verification that the group ends in `))`.
    open_parens: Vec<OpenParenGroup>,
    /// End offset of the last emitted token (for `((` adjacency) and
    /// whether that token was an unquoted `(`.
    last_token_end: Option<usize>,
    last_token_was_open_paren: bool,
    /// Set by `skip_word_inner` when the word scan broke at a `(' directly
    /// after an extglob operator with the parse-time gate closed; consumed
    /// by `finish_word_token` onto the produced token (rubash#131).
    pub(super) extglob_split_pending: bool,
}

/// One open `(` group tracked while scanning a logical line.
#[derive(Clone)]
struct OpenParenGroup {
    /// True once an immediately adjacent second `(` made this group the
    /// opener of a possible `(( ... ))` arithmetic command.
    arithmetic_candidate: bool,
    /// Offset of the first `(` of the (possible) pair.
    open_pos: usize,
    /// Memoized result of `closes_as_arithmetic_group` for this group.
    arith_close_verified: Option<bool>,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(input: &'a str, posix: bool) -> Self {
        Self {
            input,
            position: 0,
            posix,
            extended_glob: false,
            parse_state: LexerParseState::default(),
            brace_cache: None,
            open_parens: Vec::new(),
            last_token_end: None,
            last_token_was_open_paren: false,
            extglob_split_pending: false,
        }
    }

    /// Tokenize with the shared resumable brace-scan cache of an
    /// accumulating logical line (see `BraceScanCache`).
    pub(super) fn new_with_cache(
        input: &'a str,
        posix: bool,
        brace_cache: &'a mut BraceScanCache,
    ) -> Self {
        Self {
            brace_cache: Some(brace_cache),
            ..Self::new(input, posix)
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

        // GNU PST_CONDCMD/PST_REGEXP port. `[[' opens the conditional
        // (parse.y:3654), `]]' closes it; the `=~' operator arms the
        // regexp RHS word (parse.y:5169) which stays ONE GNU word —
        // read_token_word absorbs `('/`)'/`|' and everything between via
        // parse_matched_pair (parse.y:5443-5461) — until whitespace at
        // paren depth zero ends it (parse.y:5212 clears the bits after the
        // RHS token returns). Rubash splits that word into several tokens;
        // these fields track whether the word is still open so the `#'
        // branch below can keep a token-initial `#' as DATA the way GNU
        // does (read_token's `#' comment branch, parse.y:3607, never sees
        // the inside of the RHS word).
        let cond_paren_delta = |token: &Token| -> i32 {
            if token.kind == TokenKind::Keyword {
                match token.value.as_str() {
                    "(" => 1,
                    ")" => -1,
                    _ => 0,
                }
            } else {
                0
            }
        };
        let token_end = token.position + token.raw.len();
        if self.reserved_word_position() && (keyword_is("[[") || word_is("[[")) {
            self.parse_state.in_cond_command = true;
            self.parse_state.cond_rhs_regexp = false;
            self.parse_state.cond_rhs_started = false;
            self.parse_state.cond_rhs_paren_depth = 0;
        } else if self.parse_state.in_cond_command && (keyword_is("]]") || word_is("]]")) {
            self.parse_state.in_cond_command = false;
            self.parse_state.cond_rhs_regexp = false;
            self.parse_state.cond_rhs_started = false;
            self.parse_state.cond_rhs_paren_depth = 0;
        } else if self.parse_state.in_cond_command && word_is("=~") {
            self.parse_state.cond_rhs_regexp = true;
            self.parse_state.cond_rhs_started = false;
            self.parse_state.cond_rhs_paren_depth = 0;
        } else if self.parse_state.cond_rhs_regexp {
            if !self.parse_state.cond_rhs_started {
                // The first fragment after `=~' begins the RHS word
                // regardless of leading whitespace (GNU's read_token skips
                // it before read_token_word starts).
                self.parse_state.cond_rhs_started = true;
                self.parse_state.cond_rhs_paren_depth += cond_paren_delta(token);
            } else if !token.leading_ws.is_empty() && self.parse_state.cond_rhs_paren_depth <= 0 {
                // Whitespace at depth zero: the GNU RHS word ended before
                // this token (parse.y:5212).
                self.parse_state.cond_rhs_regexp = false;
                self.parse_state.cond_rhs_started = false;
                self.parse_state.cond_rhs_paren_depth = 0;
            } else {
                self.parse_state.cond_rhs_paren_depth += cond_paren_delta(token);
            }
        }
        self.parse_state.cond_rhs_last_token_end = token_end;

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
        let token = self.scan_token()?;
        self.note_open_paren_group(&token);
        Some(token)
    }

    /// Track `(` group opens/closes so the `<<` decision can tell an
    /// arithmetic-command body from command context (GNU parse.y:2626 hands
    /// `((` to parse_dparen, which consumes the body through
    /// parse_matched_pair before read_token's REDIR_LESSLESS branch at
    /// parse.y:2630+ could ever see the `<<`).
    fn note_open_paren_group(&mut self, token: &Token) {
        if token.kind == TokenKind::Keyword {
            match token.value.as_str() {
                "(" => {
                    let adjacent_pair = self.last_token_was_open_paren
                        && self.last_token_end == Some(token.position);
                    if adjacent_pair {
                        if let Some(first) = self.open_parens.last_mut() {
                            first.arithmetic_candidate = true;
                        }
                    }
                    self.open_parens.push(OpenParenGroup {
                        arithmetic_candidate: false,
                        open_pos: token.position,
                        arith_close_verified: None,
                    });
                }
                ")" => {
                    self.open_parens.pop();
                }
                _ => {}
            }
        }
        self.last_token_was_open_paren = token.kind == TokenKind::Keyword && token.value == "(";
        self.last_token_end = Some(token.position + token.raw.len());
    }

    /// Whether the scan position currently sits inside an open `((` group
    /// that closes as an arithmetic command (the `))` sits directly after
    /// the balanced inner group — the parse_dparen verdict). Used to keep `<<`
    /// the shift/shift-assign operator instead of a here-document opener
    /// (rubash#181: `((x<<=2))` swallowed the rest of the script).
    pub(super) fn inside_arithmetic_command(&mut self) -> bool {
        for index in 0..self.open_parens.len() {
            if !self.open_parens[index].arithmetic_candidate {
                continue;
            }
            let verified = match self.open_parens[index].arith_close_verified {
                Some(verified) => verified,
                None => {
                    let verified =
                        closes_as_arithmetic_group(self.input, self.open_parens[index].open_pos);
                    self.open_parens[index].arith_close_verified = Some(verified);
                    verified
                }
            };
            if verified {
                return true;
            }
        }
        false
    }

    fn scan_token(&mut self) -> Option<Token> {
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
            // GNU parse.y:3728-3734 read_token hands a word-initial `((' to
            // parse_dparen (parse.y:4895) BEFORE any word-level scanning:
            // when reserved_word_acceptable holds (parse.y:4926) and the
            // parenthesis pair is followed by `)' (parse_arith_cmd,
            // parse.y:4965-4978), the ENTIRE body becomes one ARITH_CMD
            // token read by parse_matched_pair (P_ARITH) as raw text —
            // read_token's `#' comment branch (parse.y:3607) never sees the
            // body, so `#', `;'-looking text and newlines inside stay data
            // (rubash#222: `((# 1 + 2))' must parse). `for ((' keeps the
            // separate-token form: the arith-for parser owns it (GNU's
            // parse_dparen FOR branch runs before the reserved-word gate,
            // and `for' is not in rubash's reserved_word_position list).
            '(' if self.peek() == Some('(')
                && self.reserved_word_position()
                && closes_as_arithmetic_group(self.input, start) =>
            {
                self.advance();
                self.skip_arith_paren();
                Some(Token::new(TokenKind::Keyword, self.slice(start), start))
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
                    if self.inside_arithmetic_command() {
                        // GNU parse.y:2626 read_token routes `((` to
                        // parse_dparen (parse.y:3517+), which consumes the
                        // entire arithmetic body through parse_matched_pair
                        // as raw text — read_token's redirection branch
                        // (parse.y:2630+) never sees the `<<`, so inside an
                        // arithmetic command `<<` is the shift operator and
                        // `<<=` the shift-assign, never REDIR_LESSLESS
                        // (rubash#181: `((x<<=2))` opened a here-document
                        // and swallowed the rest of the script). The
                        // arithmetic parser re-combines `<<` with the
                        // following `=…` token (arithmetic_combined_operator).
                        return Some(Token::new(TokenKind::Word, "<<", start));
                    }
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
            // GNU parse.y:5490-5524 read_token_word's shellexp arm
            // (syntax.h:84: `<`/`>` are shellexp characters): a `<`/`>`
            // immediately followed by `(` NEVER terminates the word — the
            // whole `<(...)`/`>(...)` process substitution is consumed into
            // the token, so `cat 2>(echo x)` / `cat 3<(echo y)` are single
            // WORDS (rubash#339) whose digits are literal text glued to the
            // substitution result. The digit-prefixed redirection operators
            // engage only when the `(` does not follow (the NUMBER decision
            // at parse.y:5729 requires the word to have ENDED at the `<`/
            // `>`, which `(` prevents).
            '0'..='9'
                if matches!(self.peek(), Some('<' | '>')) && self.peek_after(1) == Some('(') =>
            {
                self.skip_word_at(start);
                Some(Token::new(TokenKind::Word, self.slice(start), start))
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
                // GNU parse.y:3630-3643 read_token(): a word-initial `#`
                // comments out the rest of the physical line, and the lexer
                // then RETURNS A NEWLINE token (`character = '\n';` falls
                // into the newline branch) — the newline that terminates the
                // comment still separates commands (compound_list
                // `list1 '\n' newline_list`, parse.y:1262-1279). Emit the
                // same line-break `;' token a bare newline produces, so a
                // command followed by a comment line is a COMPLETED command
                // (rubash#219: `{ case...esac; cmd # comment` + newline +
                // `}' died as "unexpected end of file from `{'" because the
                // `}' followed a bare word with no terminator). At EOF with
                // no trailing newline GNU returns yacc_EOF without the
                // newline token — keep the old fall-through there.
                //
                // EXCEPT inside an open `=~' RHS word (PST_REGEXP,
                // parse.y:5169): GNU's read_token_word consumed the whole
                // regex as ONE word — `(' groups verbatim through
                // parse_matched_pair (parse.y:5443-5461) — so its `#'
                // characters are word DATA and read_token's comment branch
                // (parse.y:3607) never sees them. Rubash splits that word
                // into several tokens; when the RHS word is still open
                // (first fragment consumed, and either inside a `(' group or
                // directly abutting the previous fragment), a token-initial
                // `#' must scan as an ordinary word instead of a comment
                // (rubash#322: `[[ "#FFFFFF" =~ ^(#?([a-fA-F0-9]{6}|...))$ ]]`
                // died as "unexpected end of file from `('").
                if self.parse_state.cond_rhs_regexp
                    && self.parse_state.cond_rhs_started
                    && (self.parse_state.cond_rhs_paren_depth > 0
                        || self.parse_state.cond_rhs_last_token_end == start)
                {
                    self.skip_word_at(start);
                    return Some(self.finish_word_token(start, false));
                }
                let mut terminated_by_newline = false;
                loop {
                    match self.advance() {
                        Some('\n') => {
                            terminated_by_newline = true;
                            break;
                        }
                        Some(_) => continue,
                        None => break,
                    }
                }
                if terminated_by_newline {
                    let mut token = Token::new(TokenKind::Semicolon, ";", start);
                    token.line_break = true;
                    Some(token)
                } else {
                    self.next_token()
                }
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
                    // `{`/`}` are not word delimiters (syntax.h:29-30), so
                    // is_word_delimiter alone answers the continuation.
                    if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
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
                Some(Token::new_with_raw_owned(
                    TokenKind::CommandSubst,
                    value,
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
                // GNU parse.y read_token: `{` is a reserved word only as a
                // standalone token — read_token_word treats a brace glued
                // to a word character (`{xxx) as an ordinary word that
                // ends at the first shell break char (`{` is not in
                // shell_break_chars, syntax.h:30). Such a word never opens
                // a group and must NOT be scanned across whitespace for a
                // matching `}`: `if {[catch {` tokenizes as `if' `{[catch'
                // `{' and EOF inside the `if' then reports "unexpected end
                // of file from `if' command on line 1" (rubash#135; the
                // cross-whitespace swallow used to fold the rest of the
                // file into one Word token and cascade into a spurious
                // `;' token error).
                if self.input[start + 1..]
                    .chars()
                    .next()
                    .is_some_and(|ch| !"()<>;&| \t\n\r".contains(ch))
                {
                    let mut token = self.finish_word_token(start, false);
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
                // `{`/`}` are not word delimiters (syntax.h:29-30): any brace
                // glued to the closed group keeps the word alive, matching
                // read_token_word's metacharacter set.
                if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
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
                let mut token = Token::new(kind, v, start);
                // rubash#131: stamp this pass's extglob gate for the body
                // re-parse (see Token::extglob_gate).
                token.extglob_gate = self.extended_glob;
                Some(token)
            }
            '}' => {
                // GNU syntax.h:29-30: `}' is neither a shell metacharacter
                // (shell_meta_chars "()<>;&|") nor a word-break char
                // (shell_break_chars "()<>;&| \t\n"), so read_token_word
                // collects `}}'/`}x'/`}{' as ONE word; CHECK_FOR_RESERVED_WORD
                // (parse.y:3168, exact STREQ at parse.y:3174-3175) yields the
                // reserved `}' token only when the whole word is exactly `}'
                // in a reserved-word-acceptable position (parse.y:5899)
                // outside a case pattern list (parse.y:3177). `{{ echo hi; }}'
                // therefore lexes `{{' and `}}' as literal words (GNU runs
                // both as commands: "{{: command not found" / "}}: command
                // not found"), and `echo } }x }}' passes three arguments.
                if self.parse_state.case_pattern
                    || !self.reserved_word_position()
                    || self.input[start + 1..]
                        .chars()
                        .next()
                        .is_some_and(|ch| !"()<>;&| \t\n\r".contains(ch))
                {
                    return Some(self.finish_word_token(start, false));
                }
                Some(Token::new(TokenKind::Keyword, "}", start))
            }
            _ => Some(self.finish_word_token(start, true)),
        }
    }

    fn brace_group_contains_heredoc_operator(&mut self) -> bool {
        // rubash#176/#178: this query used to rebuild and walk the whole
        // rest of the buffer per `{` per re-tokenize pass. Like `skip_brace`
        // it is a left-to-right DFA over the accumulating logical line, so
        // its result or continuation is cached per opening-brace offset
        // (GNU reads the input exactly once — parse.y:3557 read_token — and
        // never re-scans text for a nested construct).
        let brace_offset = self.position - 1;
        let mut cache = self.brace_cache.take();
        let found = match cache
            .as_deref_mut()
            .and_then(|cache| cache.lookup_operator(brace_offset).cloned())
        {
            Some(HeredocOpEntry::Found) => true,
            Some(HeredocOpEntry::Absent) => false,
            Some(HeredocOpEntry::Resume(resume)) => {
                let (found, state) = self.scan_brace_group_for_heredoc_operator(Some(resume));
                if let Some(cache) = cache.as_deref_mut() {
                    cache.record_operator(brace_offset, state);
                }
                found
            }
            None => {
                let (found, state) = self.scan_brace_group_for_heredoc_operator(None);
                if let Some(cache) = cache.as_deref_mut() {
                    cache.record_operator(brace_offset, state);
                }
                found
            }
        };
        self.brace_cache = cache;
        found
    }

    /// The operator scan itself: does the brace group opened at the current
    /// position contain an unquoted `<<` (heredoc) before its close? Pure
    /// query — `self.position` is not moved. Returns the answer plus the
    /// cache entry describing how the scan ended (found / closed cleanly /
    /// resumable).
    fn scan_brace_group_for_heredoc_operator(
        &self,
        resume: Option<HeredocOpScanResume>,
    ) -> (bool, HeredocOpEntry) {
        let scan_start = self.position;
        let mut index = resume.as_ref().map_or(0usize, |state| state.index);
        let mut depth = resume.as_ref().map_or(1usize, |state| state.depth);
        let mut single = resume.as_ref().is_some_and(|state| state.single);
        let mut double = resume.as_ref().is_some_and(|state| state.double);
        let mut escaped = resume.as_ref().is_some_and(|state| state.escaped);

        let input = self.input;
        let char_at =
            |pos: usize| -> Option<char> { input.get(pos..).and_then(|rest| rest.chars().next()) };
        while let Some(ch) = char_at(scan_start + index) {
            if escaped {
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '\\' && !single {
                escaped = true;
                index += ch.len_utf8();
                continue;
            }
            if ch == '\'' && !double {
                single = !single;
                index += ch.len_utf8();
                continue;
            }
            if ch == '"' && !single {
                double = !double;
                index += ch.len_utf8();
                continue;
            }
            if single || double {
                index += ch.len_utf8();
                continue;
            }

            match ch {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return (false, HeredocOpEntry::Absent);
                    }
                }
                '<' if char_at(scan_start + index + 1) == Some('<')
                    && char_at(scan_start + index + 2) != Some('<') =>
                {
                    return (true, HeredocOpEntry::Found);
                }
                _ => {}
            }
            index += ch.len_utf8();
        }

        (
            false,
            HeredocOpEntry::Resume(HeredocOpScanResume {
                index,
                depth,
                single,
                double,
                escaped,
            }),
        )
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

/// GNU parse_dparen verdict in text space: does the `((` pair at `open_pos`
/// close as an arithmetic command — the balanced inner group followed
/// directly by `))` (parse.y:3517+ reads the matched pair and accepts the
/// arithmetic reading only when the next character is `)`)? A `((` that
/// closes as a subshell (`((echo hi); cat <<X)`) keeps its here-document
/// semantics. Quote- and backslash-aware like parse_matched_pair; iterating
/// raw bytes is safe because every delimiter checked is ASCII and UTF-8
/// continuation bytes never collide with them.
fn closes_as_arithmetic_group(input: &str, open_pos: usize) -> bool {
    let bytes = input.as_bytes();
    let mut index = open_pos + 2;
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 2;
                continue;
            }
            b'\'' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'\'' {
                    index += 1;
                }
            }
            b'"' => {
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index += 2;
                        continue;
                    }
                    if bytes[index] == b'"' {
                        break;
                    }
                    index += 1;
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return bytes.get(index + 1) == Some(&b')');
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}
