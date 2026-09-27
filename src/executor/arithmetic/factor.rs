use super::ConditionalArithParser;
use crate::executor::arithmetic::bash_arith;
use crate::executor::is_shell_name_start;

/// GNU expr.c `is_arithop` characters (expr.c:1285-1303): `=`, `>`, `<`,
/// `+`, `-`, `*`, `/`, `%`, `!`, `(`, `)`, `&`, `|`, `^`, `~`, `?`, `:`,
/// `,`. Anything else at token-start position hits readtok's junk branch
/// (expr.c:1502-1510), which splits on the previous token: an operand
/// (NUM/STR) before it yields "invalid arithmetic operator"; an operator or
/// no previous token yields "operand expected".
fn is_arithop_char(ch: u8) -> bool {
    matches!(
        ch,
        b'=' | b'>'
            | b'<'
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'!'
            | b'('
            | b')'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'?'
            | b':'
            | b','
    )
}

impl ConditionalArithParser<'_> {
    /// GNU expr.c:1502-1510 readtok junk branch vs expr.c:1120 exp0
    /// operand-expected: `ch` is a character that cannot start an operand.
    /// When it cannot begin a token at all and the previous token was an
    /// operand, GNU reports "invalid arithmetic operator" (`2 @ 3`); a
    /// recognized operator char, a non-operand predecessor, or END OF INPUT
    /// reports "operand expected" (`5 + * 3`, `x ] ` is `]` junk after STR
    /// -> invalid operator; `7<=` at EOF -> exp0's operand expected,
    /// expr.c:1331-1337 returns curtok 0 before the junk branch).
    fn fail_operand_position(&mut self) -> Option<i128> {
        if self
            .peek()
            .is_some_and(|ch| !is_arithop_char(ch) && self.last_tok_operand)
        {
            self.fail_invalid_operator()
        } else {
            self.fail_operand_expected()
        }
    }

    pub(super) fn parse_factor(&mut self) -> Option<i128> {
        self.skip_ws();
        // GNU expr.c:1034-1059 exp0 PREINC/PREDEC: readtok produces
        // PREINC/PREDEC only when `++`/`--` is followed by (optional
        // whitespace and) an identifier start (expr.c:1472-1481) — otherwise
        // the second `+`/`-` is ungot and the text parses as two unary
        // operators (`++4` is `+(+4)` = 4).
        if self.input[self.pos..].starts_with(b"++") {
            let op_start = self.pos;
            self.pos += 2;
            if let Some(lvalue) = self.parse_lvalue() {
                self.note_op(op_start);
                let result = self.update_lvalue(&lvalue, 1, true);
                if result.is_some() {
                    self.post_incdec_after_preincdec();
                }
                return result;
            }
            self.pos = op_start;
        }
        if self.input[self.pos..].starts_with(b"--") {
            let op_start = self.pos;
            self.pos += 2;
            if let Some(lvalue) = self.parse_lvalue() {
                self.note_op(op_start);
                let result = self.update_lvalue(&lvalue, -1, true);
                if result.is_some() {
                    self.post_incdec_after_preincdec();
                }
                return result;
            }
            self.pos = op_start;
        }
        match self.peek() {
            // GNU expr.c readtok tokenizes greedily: `+=`, `-=`, `!=` are
            // single OP_ASSIGN/NEQ tokens, so in operand position they are
            // "operand expected" with lasttp at the operator's first char
            // (`+=2` reports `+=2`, `++=2` reports `+=2` after the first
            // unary `+`). The unary `+`/`-`/`!` below must not consume the
            // first byte of a compound token.
            Some(b'+') | Some(b'-') | Some(b'!') if self.input.get(self.pos + 1) == Some(&b'=') => {
                self.fail_operand_expected()
            }
            Some(b'+') => {
                self.note_op(self.pos);
                self.pos += 1;
                self.parse_factor()
            }
            Some(b'-') => {
                self.note_op(self.pos);
                self.pos += 1;
                self.parse_factor().map(|value| bash_arith(-value))
            }
            Some(b'!') => {
                self.note_op(self.pos);
                self.pos += 1;
                self.parse_factor().map(|value| i128::from(value == 0))
            }
            Some(b'~') => {
                self.note_op(self.pos);
                self.pos += 1;
                self.parse_factor().map(|value| bash_arith(!value))
            }
            Some(b'(') => {
                self.note_op(self.pos);
                self.pos += 1;
                let value = self.parse_comma()?;
                self.skip_ws();
                // GNU expr.c:1066-1067: `curtok != RPAR` -> "missing `)'";
                // lasttp is the current token start (the last consumed token
                // when input ran out).
                if self.peek() != Some(b')') {
                    let tok = if self.pos < self.input.len() {
                        self.pos
                    } else {
                        self.last_tok_start
                    };
                    return self.fail("missing `)'", tok);
                }
                self.note_op(self.pos);
                self.pos += 1;
                Some(value)
            }
            Some(b'$') => self.parse_dollar_variable(),
            Some(ch) if ch.is_ascii_digit() => self.parse_number(),
            Some(ch) if is_shell_name_start(ch as char) => self.parse_variable(),
            _ => self.fail_operand_position(),
        }
    }

    /// GNU expr.c:1461-1470: after a PREINC/PREDEC result (curtok==NUM with
    /// lasttok still PREINC/PREDEC) a following `++`/`--` evalerrors
    /// "++/--: assignment requires lvalue" (`--x++`, `++x--`).
    fn post_incdec_after_preincdec(&mut self) {
        self.skip_ws();
        if self.input[self.pos..].starts_with(b"++") {
            self.fail("++: assignment requires lvalue", self.pos);
        } else if self.input[self.pos..].starts_with(b"--") {
            self.fail("--: assignment requires lvalue", self.pos);
        }
    }

    /// GNU expr.c:1406-1418 readtok NUM + 1551-1636 strlong: the token scans
    /// ISALNUM plus `#`, `@`, `_`, is NUL-terminated in place, and handed to
    /// strlong — so a malformed literal (`0#4`, `08`, `123abc`, `1_0`)
    /// truncates the displayed expression AND the error token at the
    /// token's end (`123abc + 1` displays as `123abc`, token `123abc`).
    pub(super) fn parse_number(&mut self) -> Option<i128> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'#' | b'@' | b'_'))
        {
            self.pos += 1;
        }
        let end = self.pos;
        self.note_operand(start);
        let token = &self.input[start..end];
        match strlong_value(token) {
            Ok(value) => Some(value),
            Err(msg) => self.fail_display(msg, start, end),
        }
    }

    /// GNU expr.c: `$` is neither a legal_variable_starter nor an arithop —
    /// a `$name`/`$(...)` that survived into evalexp hits readtok's junk
    /// branch (expr.c:1502-1510): "operand expected" after an operator or at
    /// the start, "invalid arithmetic operator" after an operand. The error
    /// token is the `$...` remainder (`let 'jv += $iv'` reports `$iv`).
    pub(super) fn parse_dollar_variable(&mut self) -> Option<i128> {
        self.fail_operand_position()
    }

    /// GNU expr.c:1344-1404 readtok STR + 1077-1118 exp0: the
    /// `name`/`name[subscript]` token is read with its lvalue context (the
    /// evaluated subscript index is kept in `curlval`), then the following
    /// token is peeked — whitespace before a trailing `++`/`--` is allowed
    /// because readtok's own whitespace skip runs first (expr.c:1329), so
    /// `a[0] ++` is a valid post-increment.
    pub(super) fn parse_variable(&mut self) -> Option<i128> {
        let tok_start = self.pos;
        let lvalue = self.parse_lvalue()?;
        // GNU expr.c:1458-1459: `++`/`--` directly after the STR token
        // (whitespace already skipped by readtok) is POSTINC/POSTDEC; exp0
        // then binds through the saved lvalue (expr.c:1097-1105).
        self.skip_ws();
        if self.input[self.pos..].starts_with(b"++") {
            self.pos += 2;
            self.note_operand(tok_start);
            return self.update_lvalue(&lvalue, 1, false);
        }
        if self.input[self.pos..].starts_with(b"--") {
            self.pos += 2;
            self.note_operand(tok_start);
            return self.update_lvalue(&lvalue, -1, false);
        }
        self.note_operand(tok_start);
        let value = self.lvalue_value(&lvalue);
        if value.is_none() && self.error.is_none() {
            // GNU expr.c:271 pushexp already recorded the recursion error in
            // variable_value; anything else that returned no value without
            // a record is exp0's operand-expected (expr.c:1120).
            self.fail_operand_expected();
        }
        value
    }
}

/// GNU expr.c:1551-1636 strlong: base-prefixed integer literal parsing.
/// `VALID_NUMCHAR(c)` is ISALNUM plus `_` and `@` (expr.c:1549). Returns
/// the value or the evalerror message.
fn strlong_value(num: &[u8]) -> Result<i128, &'static str> {
    let mut index = 0usize;
    let mut base = 10u32;
    let mut foundbase = false;
    if num.first() == Some(&b'0') {
        index += 1;
        if index == num.len() {
            return Ok(0);
        }
        if num[index] == b'x' || num[index] == b'X' {
            base = 16;
            index += 1;
            // STRICT_ARITH_PARSING is not defined in the GNU build
            // (verified 5.3: `$((0x))` is 0), so a bare `0x` yields 0.
        } else {
            base = 8;
        }
        foundbase = true;
    }
    let mut value = 0i128;
    while index < num.len() {
        let c = num[index];
        index += 1;
        if c == b'#' {
            if foundbase {
                return Err("invalid number");
            }
            if !(2..=64).contains(&value) {
                return Err("invalid arithmetic base");
            }
            base = value as u32;
            value = 0;
            foundbase = true;
            if index >= num.len() || !valid_numchar(num[index]) {
                return Err("invalid integer constant");
            }
            continue;
        }
        if !valid_numchar(c) {
            break;
        }
        let digit = if c.is_ascii_digit() {
            u32::from(c - b'0')
        } else if c.is_ascii_lowercase() {
            u32::from(c) - u32::from(b'a') + 10
        } else if c.is_ascii_uppercase() {
            u32::from(c) - u32::from(b'A') + if base <= 36 { 10 } else { 36 }
        } else if c == b'@' {
            62
        } else {
            // `_`
            63
        };
        if digit >= base {
            return Err("value too great for base");
        }
        value = bash_arith(value * i128::from(base) + i128::from(digit));
    }
    Ok(value)
}

/// expr.c:1549 `VALID_NUMCHAR` — ISALNUM plus `_` and `@`.
fn valid_numchar(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'@'
}
