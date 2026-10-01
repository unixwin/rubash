#[path = "cursor.rs"]
mod cursor;
#[path = "expression.rs"]
mod expression;
#[path = "factor.rs"]
mod factor;
#[path = "lvalue.rs"]
mod lvalue;
#[path = "value.rs"]
mod value;

use crate::executor::execution_misc::RandomGen;

/// GNU expr.c `evalerror` record (expr.c:1524-1535): the diagnostic carries
/// the *current frame's* `expression` text (a variable's value when the
/// failure happened inside `expr_streval`'s nested `subexpr`, expr.c:1241),
/// the message verbatim, and the `lasttp` remainder inside it.
/// `display_end` mirrors the in-place NUL readtok writes while scanning a
/// NUM/STR token (expr.c:1412-1415) — a failing literal truncates both the
/// displayed expression and the error token at the token's end.
///
/// `evalerror` itself only `sh_longjmp`s to the innermost `evalexp`
/// (expr.c:1535 -> 429-445); whether the failure aborts the command list is
/// each evalexp caller's decision (expok==0 -> `((`/`let`/`[[` return status
/// 1 and continue; array_expand_index/make_variable_value jump DISCARD).
#[derive(Clone, Debug)]
pub(crate) struct ArithEvalError {
    /// The failing frame's expression text (leading whitespace skipped at
    /// display time, GNU expr.c:1530).
    pub expr: String,
    /// The evalerror message verbatim, e.g. "arithmetic syntax error:
    /// operand expected".
    pub msg: String,
    /// Byte offset into `expr` where the error token (`lasttp`) starts.
    pub tok_start: usize,
    /// Display cut: `expr[..display_end]` is echoed and the error token is
    /// `expr[tok_start..display_end]` — both may be truncated mid-expression
    /// when readtok's in-place NUL is still active (number literals).
    pub display_end: usize,
}

/// A non-fatal arithmetic diagnostic — printed but the evaluation continues
/// (GNU prints these from the bind/subscript helpers, not via evalerror's
/// longjmp). The label (`((`, `let`, `[[`, or none) is the caller's
/// `this_command_name`.
#[derive(Clone, Debug)]
pub(crate) enum ArithEvalDiag {
    /// `` `name[]': not a valid identifier `` — bind of an empty-subscript
    /// element (expr_bind_variable -> sh_invalidid); the bind is skipped but
    /// the expression value is unaffected.
    InvalidIdentifier(String),
    /// `name[]: bad array subscript` — reading an element whose subscript
    /// resolved to empty (arrayfunc.c bash_badsub_errmsg).
    BadSubscript(String),
}

impl ArithEvalError {
    /// The displayed expression text: GNU evalerror skips leading blanks
    /// only (expr.c:1530) and may see a truncated buffer (readtok's NUL).
    pub(crate) fn display(&self) -> &str {
        self.expr[..self.display_end.min(self.expr.len())].trim_start()
    }

    /// The `(error token is "...")` payload: `expr[lasttp..display_end]`.
    /// GNU prints the clause only when the token is non-empty
    /// (expr.c:1534 `(lasttp && *lasttp) ? lasttp : ""` — the clause is
    /// always printed, but with an empty token it shows `""`).
    pub(crate) fn token(&self) -> &str {
        let end = self.display_end.min(self.expr.len());
        let start = self.tok_start.min(end);
        &self.expr[start..end]
    }

    /// GNU expr.c:1526-1535 evalerror format: `%s: %s (error token is
    /// "%s")`. The message is verbatim from the expr.c evalerror site —
    /// every 5.3.0 site carries the `arithmetic` prefix unconditionally
    /// (expr.c:485/529/1120/1507/1509); only the `name:` prefix
    /// (`this_command_name`: `let`, `((`) varies by caller, and that is
    /// added by the diagnostic sites, not here. (5.2.21 dropped the
    /// prefix in the word-expansion context; probe 2026-09-27:
    /// `/usr/local/bin/bash tmp.sh` with `echo $((j=))` prints
    /// `j=: arithmetic syntax error: operand expected (error token is "=")`.)
    pub(crate) fn render(&self) -> String {
        format!(
            "{}: {} (error token is \"{}\")",
            self.display(),
            self.msg,
            self.token()
        )
    }
}

pub(super) struct ConditionalArithParser<'a> {
    pub(super) input: &'a [u8],
    pub(super) pos: usize,
    pub(super) env_vars: &'a mut crate::shell::var_table::VarTable,
    pub(super) resolving: Vec<String>,
    pub(super) random_state: Option<&'a RandomGen>,
    /// Lazy dynamic-parameter context injected by the Executor (see
    /// `Executor::arith_dynamic_context`). GNU expr.c:1150 expr_streval
    /// resolves operand names through find_variable, which materializes
    /// dynamic variables (BASHPID, BASH_SUBSHELL, ...) exactly like
    /// `$name` expansion; the evaluator only carries `env_vars`, so the
    /// Executor supplies a read-time-resolvable context here. GNU builds
    /// no per-evaluation snapshot (variables.c:1844 INIT_DYNAMIC_VAR
    /// getters resolve at find_variable time), and neither does this
    /// port: a name resolves only when an operand actually spells it.
    pub(super) dynamic_values: Option<&'a super::super::dynamic_arrays::ArithDynamicContext>,
    pub(super) error_category: Option<super::ArithmeticErrorCategory>,
    pub(super) no_expand: bool,
    /// The first recorded evalerror — GNU longjmps on the first failure, so
    /// later diagnostics never overwrite it.
    pub(super) error: Option<ArithEvalError>,
    /// Non-fatal diagnostics emitted during evaluation (bad subscript reads,
    /// invalid-identifier binds).
    pub(super) diags: Vec<ArithEvalDiag>,
    /// Start offset of the most recently consumed token — GNU `lasttp`
    /// (expr.c:1342). End-of-input diagnostics use the last token's start
    /// (`jv += ` reports `+= `).
    pub(super) last_tok_start: usize,
    /// Whether the last consumed token was an operand (NUM/STR). readtok's
    /// junk branch (expr.c:1506-1509) splits "operand expected" (previous
    /// token an operator or none) from "invalid arithmetic operator"
    /// (previous token an operand) on it.
    pub(super) last_tok_operand: bool,
    /// GNU expr.c global `noeval` — the depth of short-circuited regions
    /// (`explor`/`expland` RHS, `expcond` untaken branches increment it,
    /// expr.c:690/719/655/666). While set, variable reads short-circuit
    /// to 0 (`expr_streval` returns immediately, expr.c:1164-1167), binds
    /// are skipped (expr.c:607-615, 1097-1105) and division-by-zero /
    /// negative-exponent evalerrors are suppressed (expr.c:909-920,
    /// 977-991) — but SYNTAX errors still fire (exp0:1120 and readtok's
    /// junk branch are unguarded). Nested `evalexp` frames reset it
    /// (expr.c:426), which rubash models by constructing fresh parsers.
    pub(super) noeval: usize,
}

impl ConditionalArithParser<'_> {
    /// Record an evalerror and fail the parse. The first failure wins —
    /// GNU's sh_longjmp leaves the evaluator on the spot.
    pub(super) fn fail(&mut self, msg: &str, tok_start: usize) -> Option<i128> {
        self.fail_display(msg, tok_start, self.input.len())
    }

    /// Like [`Self::fail`] but truncates the displayed expression at
    /// `display_end` — readtok's in-place NUL while scanning a NUM/STR token
    /// (expr.c:1412) makes evalerror echo only up to the offending literal.
    pub(super) fn fail_display(
        &mut self,
        msg: &str,
        tok_start: usize,
        display_end: usize,
    ) -> Option<i128> {
        if self.error.is_none() {
            self.error = Some(ArithEvalError {
                expr: String::from_utf8_lossy(self.input).into_owned(),
                msg: msg.to_string(),
                tok_start: tok_start.min(self.input.len()),
                display_end: display_end.min(self.input.len()),
            });
        }
        None
    }

    /// GNU expr.c:1120 exp0 / 1506-1509 readtok: nothing usable at operand
    /// position. The error token is the remainder at the stop position, or
    /// the last consumed token when the expression simply ran out
    /// (`jv += ` -> `+= `).
    pub(super) fn fail_operand_expected(&mut self) -> Option<i128> {
        let tok = if self.pos < self.input.len() {
            self.pos
        } else {
            self.last_tok_start
        };
        self.fail("arithmetic syntax error: operand expected", tok)
    }

    /// GNU expr.c:1508-1510 readtok junk branch: a character that cannot
    /// begin a token after an operand position — "invalid arithmetic
    /// operator".
    pub(super) fn fail_invalid_operator(&mut self) -> Option<i128> {
        self.fail(
            "arithmetic syntax error: invalid arithmetic operator",
            self.pos,
        )
    }

    /// Mark a just-consumed operator token for `last_tok_start` tracking.
    pub(super) fn note_op(&mut self, start: usize) {
        self.last_tok_start = start;
        self.last_tok_operand = false;
    }

    /// Mark a just-consumed operand token (NUM/STR) for `last_tok_start`.
    pub(super) fn note_operand(&mut self, start: usize) {
        self.last_tok_start = start;
        self.last_tok_operand = true;
    }

    /// Adopt a nested evaluator's failure — GNU's nested subexpr frame
    /// reports its own expression/lasttp through the shared globals, so the
    /// inner record is what evalerror prints.
    pub(super) fn adopt_error(&mut self, other: Option<ArithEvalError>) {
        if self.error.is_none() {
            self.error = other;
        }
    }

    /// Merge non-fatal diagnostics produced by a nested evaluation.
    pub(super) fn adopt_diags(&mut self, other: Vec<ArithEvalDiag>) {
        self.diags.extend(other);
    }

    /// GNU expr.c:484-485 subexpr vs 528-529 expassign vs 1506-1509
    /// readtok: call after `skip_ws` at the end of a parse — when tokens
    /// remain (`curtok != 0`), record the error GNU would raise at this
    /// frame. An assignment operator trailing a non-STR token is
    /// "attempted assignment to non-variable" (`x++ = 7`, `x = 9 = 8`), a
    /// character that cannot begin a token is readtok's "invalid
    /// arithmetic operator" (`x ] `, since curtok was an operand), and
    /// anything else is "arithmetic syntax error in expression"
    /// (`x=9 y=41` -> token `y=41 `).
    pub(super) fn record_trailing(&mut self) {
        if self.error.is_some() || self.pos >= self.input.len() {
            return;
        }
        let rest = &self.input[self.pos..];
        let msg = if super::assignment_operator_at(rest, 0).is_some() {
            "attempted assignment to non-variable"
        } else if rest.first().is_some_and(|ch| !super::token_can_start(ch)) {
            "arithmetic syntax error: invalid arithmetic operator"
        } else {
            "arithmetic syntax error in expression"
        };
        self.fail(msg, self.pos);
    }
}

#[derive(Clone)]
pub(super) enum ArithLValue {
    Scalar(String),
    Indexed {
        name: String,
        index: i128,
    },
    /// Array element with a raw subscript expression that must be evaluated
    /// lazily *after* the RHS of an assignment, matching GNU expr.c:1395-1401
    /// where `expr_streval` is skipped when the next token is `=`. The subscript
    /// is re-evaluated at bind time, so side effects in the RHS (e.g.
    /// `a[n]=++n`) are visible to the subscript.
    IndexedRaw {
        name: String,
        subscript: String,
    },
    Assoc {
        name: String,
        key: String,
    },
    /// `name[]` with an empty subscript — GNU tokenizes it as STR `name[]`
    /// but every use fails non-fatally: reads report `bad array subscript`
    /// and binds report `` `name[]': not a valid identifier `` while the
    /// expression keeps evaluating (verified GNU 5.3: `(( a[]=24 ))`
    /// reports the diagnostic, assigns nothing, and yields status 0).
    InvalidElement {
        display: String,
    },
}
