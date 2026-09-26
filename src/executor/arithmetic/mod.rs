//! Arithmetic expression parsing and evaluation.
//!
//! Provides parsing and evaluation of shell arithmetic expressions including
//! variables, arrays, assignments, and ternary conditionals.

mod parser;

use parser::ConditionalArithParser;
pub(crate) use parser::{ArithEvalDiag, ArithEvalError};
use std::collections::HashMap;

use super::Executor;
use crate::executor::execution_misc::RandomGen;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::executor::{is_marked_var, SubstitutionQuoteContext, ASSOC_VARS};

thread_local! {
    /// Variable writes performed by the arithmetic evaluator between the
    /// start and end of one top-level evaluation: (name, value before the
    /// first write). Replaces the former whole-env snapshot + O(n) diff,
    /// which cloned the entire variable table on every `$(( ))` evaluation.
    static ARITH_WRITES: std::cell::RefCell<Vec<(String, Option<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };

    /// The evalerror record left by the most recent evaluation — GNU's
    /// `expression`/`lasttp` globals at the moment evalerror ran
    /// (expr.c:1524-1535). The record belongs to the frame that failed,
    /// which may be a nested subscript or variable-value evaluation.
    static ARITH_EVAL_ERROR: std::cell::RefCell<Option<ArithEvalError>> =
        const { std::cell::RefCell::new(None) };

    /// Non-fatal diagnostics produced during the most recent evaluation
    /// (`` `a[]': not a valid identifier ``, `a[]: bad array subscript`) —
    /// GNU prints these from the bind/subscript helpers while evaluation
    /// continues, so they are emitted even when the expression succeeds.
    static ARITH_EVAL_DIAGS: std::cell::RefCell<Vec<ArithEvalDiag>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Take (and clear) the evalerror record of the last evaluation.
pub(in crate::executor) fn take_arith_eval_error() -> Option<ArithEvalError> {
    ARITH_EVAL_ERROR.with(|slot| slot.borrow_mut().take())
}

/// Inspect (without clearing) the evalerror record of the last evaluation.
pub(in crate::executor) fn peek_arith_eval_error() -> Option<ArithEvalError> {
    ARITH_EVAL_ERROR.with(|slot| slot.borrow().clone())
}

/// Take (and clear) the non-fatal diagnostics of the last evaluation.
pub(in crate::executor) fn take_arith_eval_diags() -> Vec<ArithEvalDiag> {
    ARITH_EVAL_DIAGS.with(|slot| std::mem::take(&mut *slot.borrow_mut()))
}

pub(super) fn record_arith_write(name: &str, old_value: Option<String>) {
    ARITH_WRITES.with(|log| {
        log.borrow_mut().push((name.to_string(), old_value));
    });
}

fn take_arith_writes() -> Vec<(String, Option<String>)> {
    ARITH_WRITES.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

/// Sync shell_state with the variables the arithmetic evaluator wrote.
/// Mirrors the former whole-map diff semantics: a name whose env_vars value
/// equals its pre-evaluation value is left alone; a removed name is ignored
/// (the diff loop only visited surviving keys).
fn sync_arith_writes_to_shell_state(executor: &mut Executor) {
    for (name, old_value) in take_arith_writes() {
        let new_value = executor.shell_state.env_vars.get(&name);
        let changed = match (&old_value, new_value) {
            (Some(old), Some(new)) => old != new,
            (None, Some(_)) => true,
            (Some(_), None) | (None, None) => false,
        };
        if !changed {
            continue;
        }
        let new_value = new_value.expect("changed implies present").clone();
        if let Some(variable) = executor.shell_state.variables.get_mut(&name) {
            variable.value = crate::shell::ShellValue::Scalar(new_value);
        } else if !name.starts_with("__RUBASH_") {
            let _ = executor.shell_state.variables.set_scalar(&name, &new_value);
        }
    }
}

/// Categories surfaced by GNU Bash's arithmetic evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArithmeticErrorCategory {
    EmptyArraySubscript,
    DivisionByZero,
    InvalidLiteral,
    NonVariableAssignment,
    EvaluatorFailure,
    /// GNU expr.c:484-485: `curtok != 0` after `EXP_LOWEST()` — trailing
    /// input after a successful parse, e.g. `(( x=9 y=41 ))`.
    TrailingInput,
}

/// GNU expr.c diagnostic class for the unparsed remainder of an arithmetic
/// expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::executor) enum TrailingInputKind {
    /// The full parse completed and left trailing tokens (expr.c:485):
    /// `x=9 y=41` -> `arithmetic syntax error in expression`.
    InExpression,
    /// The parse died at an operand position -- the token at the stop point
    /// cannot begin an operand (expr.c:1120 `exp0`'s operand-expected):
    /// `x = 2 ,, 3` stops at the second `,`, `5 + * 3` at `*`.
    OperandExpected,
    /// The parse completed but the leftover begins with a character that is
    /// no valid arithmetic token at all -- readtok's junk branch
    /// (expr.c:1507-1509) with the previous token an operand:
    /// `2 @ 3` -> `invalid arithmetic operator`.
    InvalidOperator,
}

impl Executor {
    /// Snapshot the arithmetic error flags so a subshell boundary (command
    /// substitution, pipeline element) can restore them afterwards; errors
    /// raised inside the subshell must not leak into the enclosing command's
    /// word-expansion check.
    pub(crate) fn snapshot_arithmetic_error_flags(
        &self,
    ) -> (bool, bool, bool, bool, Option<ArithmeticErrorCategory>) {
        (
            self.shell_state.arithmetic_expansion_error.get(),
            self.shell_state.arithmetic_nonfatal_error.get(),
            self.shell_state.arithmetic_fatal_error.get(),
            self.shell_state.arithmetic_nounset_error.get(),
            self.shell_state.arithmetic_last_error_category.get(),
        )
    }

    /// Restore flags saved by [`Self::snapshot_arithmetic_error_flags`].
    /// Returns true when a `set -u` unbound-variable error was raised inside
    /// the bounded region (it was clear on entry and is set now).
    pub(crate) fn restore_arithmetic_error_flags(
        &self,
        saved: &(bool, bool, bool, bool, Option<ArithmeticErrorCategory>),
    ) -> bool {
        let nounset_hit = self.shell_state.arithmetic_nounset_error.get() && !saved.3;
        self.shell_state.arithmetic_expansion_error.set(saved.0);
        self.shell_state.arithmetic_nonfatal_error.set(saved.1);
        self.shell_state.arithmetic_fatal_error.set(saved.2);
        self.shell_state.arithmetic_nounset_error.set(saved.3);
        self.shell_state.arithmetic_last_error_category.set(saved.4);
        nounset_hit
    }

    /// GNU eval.c expands the `$(( ))` expression text like a double-quoted
    /// word before expr.c evaluates it, so current-shell `${ ...; }`
    /// substitutions run during that expansion (comsub21.sub:
    /// `$(( ${ number; } ))` is 123, not 0). The payload escapes the embedded
    /// walker leaves behind are decoded before evaluation.
    pub(crate) fn expand_arithmetic_expression_mut(&mut self, expression: &str) -> String {
        let routed = self.route_current_shell_substitutions(expression);
        let expanded = self.expand_arithmetic_special_parameters(&routed);
        crate::executor::execution_misc::restore_command_substitution_output(
            &crate::executor::execution_misc::decode_command_substitution_payload(&expanded),
        )
    }

    /// GNU arrayfunc.c:1368-1378 array_expand_index re-expands the resolved
    /// indexed subscript through
    /// `expand_arith_string(exp, Q_DOUBLE_QUOTES|Q_ARITH|Q_ARRAYSUB)`.
    /// Parameter/command/arithmetic expansion runs, but under
    /// Q_DOUBLE_QUOTES `string_quote_removal` (subst.c:12199) keeps `'`,
    /// `"` and non-CBSDQUOTE backslashes as literal data — a quote produced
    /// by an escape (`a[\" \"]=v` resolves to subscript `" "`) reaches
    /// evalexp as junk and fails "operand expected" instead of delimiting a
    /// quoted span. This is expand_arithmetic_expression_mut with the
    /// walker's quote toggling disabled and the data carriers restored to
    /// the literal characters evalexp would see.
    pub(crate) fn expand_arithmetic_subscript_mut(&mut self, expression: &str) -> String {
        let routed = self.route_current_shell_substitutions(expression);
        // The same $#/$- special-parameter substitutions
        // expand_arithmetic_special_parameters performs.
        let routed = routed
            .replace("$#", &self.shell_state.positional_params.len().to_string())
            .replace("$-", "0");
        let protected = routed
            .replace("\\\"", crate::executor::markers::DATA_DQUOTE_STR)
            .replace('"', crate::executor::markers::DATA_DQUOTE_STR)
            .replace('\'', crate::executor::markers::DATA_SQUOTE_STR)
            .replace("\\$", DATA_DOLLAR_STR);
        let expanded = self.expand_embedded_parameters(&protected);
        expanded
            .replace(DATA_DOLLAR, "$")
            .replace(crate::executor::markers::DATA_BACKTICK, "`")
            .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
            .replace(crate::executor::markers::DATA_SQUOTE, "'")
            .replace(crate::executor::markers::DATA_DQUOTE, "\"")
    }

    /// Splice current-shell `${ ...; }` / `${| ...; }` substitution values
    /// into an arithmetic expression before evaluation (subst.c: the
    /// expression undergoes normal expansion first).
    fn route_current_shell_substitutions(&mut self, expression: &str) -> String {
        if !expression.contains("${") {
            return expression.to_string();
        }
        let bytes = expression.as_bytes();
        let mut output = String::new();
        let mut index = 0usize;
        while index < bytes.len() {
            let ch = expression[index..].chars().next().unwrap();
            if ch == '$' && bytes.get(index + 1) == Some(&b'{') {
                let after = &expression[index + 2..];
                let is_funsub =
                    after.starts_with('|') || after.starts_with(|c: char| c.is_whitespace());
                if is_funsub {
                    let mut inner = after.chars().peekable();
                    if let Some(value) = self.expand_current_shell_braced_substitution(&mut inner) {
                        output.push_str(&value);
                        let remainder: String = inner.collect();
                        index = expression.len() - remainder.len();
                        continue;
                    }
                }
                output.push_str("${");
                index += 2;
                continue;
            }
            output.push(ch);
            index += ch.len_utf8();
        }
        output
    }

    pub(crate) fn eval_arithmetic_command_value(&mut self, expression: &str) -> Option<i128> {
        self.eval_arithmetic_command_value_with_flags(expression, true, "((")
    }

    /// `let`'s operand (GNU let.def: `evalexp(arg, EXP_EXPANDED)`): the
    /// argument was already word-expanded once, so top-level `$name`/`$(...)`
    /// text is data for readtok's junk branch — never re-expanded. Indexed
    /// array subscripts still expand inside array_expand_index unless
    /// `shopt -s array_expand_once` (expr.c:1171, arrayfunc.c:1368-1378).
    pub(crate) fn eval_arithmetic_command_value_no_expand(
        &mut self,
        expression: &str,
    ) -> Option<i128> {
        self.eval_arithmetic_command_value_with_flags(expression, false, "let")
    }

    fn eval_arithmetic_command_value_with_flags(
        &mut self,
        expression: &str,
        expand: bool,
        label: &'static str,
    ) -> Option<i128> {
        self.shell_state.arithmetic_last_error_category.set(None);
        let _ = take_arith_eval_error();
        let _ = take_arith_eval_diags();
        // Stale nested-subscript failure marker (lvalue.rs records the
        // subscript text so evalerror reports it, GNU array_expand_index
        // style); each new evaluation starts clean.
        self.shell_state
            .env_vars
            .remove("__RUBASH_ARITH_SUBSCRIPT_EXPR");
        // Associative subscripts are expanded first, in their own pass, and
        // replaced by an opaque literal (see expand_arithmetic_assoc_subscripts)
        // so the ordinary expansion below cannot expand them a second time and
        // the parser stores the key verbatim.

        // GNU expr.c:1171 expr_streval: `tflag = (array_expand_once &&
        // already_expanded) ? AV_NOEXPAND : 0` — `let`/`[[` operands arrive
        // EXP_EXPANDED, `(( ))` does not, so only the former switches the
        // assoc subscript scan to flag-1 (verbatim key) semantics.
        let assoc_noexpand = !expand
            && crate::builtins::shopt::option_enabled(
                &self.shell_state.env_vars,
                "array_expand_once",
            );
        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(expression, assoc_noexpand);

        let expression = if expand {
            normalize_arithmetic_quotes(&self.expand_arithmetic_expression_mut(&with_assoc_keys))
        } else {
            normalize_arithmetic_quotes(&self.expand_arith_eval_subscripts(&with_assoc_keys))
        };
        *self.arithmetic_last_eval_input.borrow_mut() = expression.clone();
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.shell_state.env_vars)
            {
                self.shell_state.arithmetic_nounset_error.set(true);
                if !self.shell_state.arithmetic_expansion_error.replace(true) {
                    eprintln!("{}{}: unbound variable", self.diagnostic_prefix(), name);
                    use std::io::Write;
                    let _ = std::io::stderr().flush();
                }
                return None;
            }
        }
        // In arithmetic command context Bash removes double quotes, but a
        // single-quoted operand is not a numeric literal. Preserve it as an
        // evaluation error so `(( '1' ))` is not silently accepted as 1. A
        // single quote inside an associative-array subscript is a different
        // thing: there it delimits a string key, which GNU accepts
        // (`(( A['a b']++ ))`), so only bare quotes are rejected.
        if has_bare_single_quote(&expression, &self.shell_state.env_vars) {
            return None;
        }
        if empty_quoted_operand_has_operator(&expression) {
            return None;
        }
        // GNU expr.c: by the time evalexp parses, the expression text is
        // already expanded (expand_arith_string at execute_cmd.c:3936 /
        // subst.c for $((...))), so a surviving `$name`/`$(...)` is data
        // that fails readtok's is_arithop check -> "operand expected"
        // (expr.c:1503-1511), never a second command-substitution run.
        // The parser must therefore run with no_expand in BOTH branches.
        // GNU expr.c:1171 expr_streval: `tflag = (array_expand_once &&
        // already_expanded) ? AV_NOEXPAND : 0` — an operand GNU's caller
        // already expanded (`let` passes EXP_EXPANDED; `(( ))` and `[[ ]]`
        // do not) skips the subscript's expand_arith_string pass entirely
        // under the option, so `let 'a[""]=26'` feeds `""` to evalexp
        // verbatim -> "operand expected" (verified GNU 5.3). The marker
        // mirrors EXP_EXPANDED for the parser's subscript evaluation.
        let exp_expanded = !expand
            && crate::builtins::shopt::option_enabled(
                &self.shell_state.env_vars,
                "array_expand_once",
            );
        if exp_expanded {
            self.shell_state
                .env_vars
                .insert("__RUBASH_ARITH_EXP_EXPANDED".to_string(), "1".to_string());
        }
        // Save a snapshot of variable values before evaluation to detect changes.
        ARITH_WRITES.with(|log| log.borrow_mut().clear());
        let (value, category) = eval_mutable_arith_value_with_random_flags(
            &expression,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            true,
        );
        self.shell_state
            .env_vars
            .remove("__RUBASH_ARITH_EXP_EXPANDED");
        self.shell_state
            .arithmetic_last_error_category
            .set(category);
        self.report_arithmetic_readonly_error();
        // GNU prints bind/subscript diagnostics (`a[]: bad array
        // subscript`, `` `a[]': not a valid identifier ``) while the
        // evaluation continues — they surface even when the expression
        // succeeds (`(( a[]=24 ))` -> status 0).
        self.flush_arith_diags(Some(label));

        // Sync any variable changes from env_vars to shell_state.variables
        // so that subsequent variable expansions see the updated values.
        sync_arith_writes_to_shell_state(self);

        value
    }

    /// Print the non-fatal diagnostics recorded during the last
    /// evaluation. `label` is this_command_name (`((`, `let`, `[[`) or
    /// None — GNU's subscript/bind helpers run with this_command_name
    /// NULL inside array_expand_index (arrayfunc.c:1374-1378), so
    /// expansion-context diagnostics carry no command label.
    pub(in crate::executor) fn flush_arith_diags(&self, label: Option<&str>) {
        let mut printed = false;
        for diag in take_arith_eval_diags() {
            match (&diag, label) {
                (ArithEvalDiag::InvalidIdentifier(display), Some(label)) => eprintln!(
                    "{}{}: `{}': not a valid identifier",
                    self.diagnostic_prefix(),
                    label,
                    display
                ),
                (ArithEvalDiag::InvalidIdentifier(display), None) => eprintln!(
                    "{}`{}': not a valid identifier",
                    self.diagnostic_prefix(),
                    display
                ),
                (ArithEvalDiag::BadSubscript(display), _) => eprintln!(
                    "{}{}: bad array subscript",
                    self.diagnostic_prefix(),
                    display
                ),
            }
            printed = true;
        }
        if printed {
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
    }

    /// GNU arrayfunc.c:1353-1391 `array_expand_index` -> `evalexp`: evaluate
    /// subscript text that has ALREADY been resolved by the caller — expanded
    /// exactly once under `array_expand_once` (the AV_NOEXPAND branch passes
    /// the text to `evalexp` unchanged) or expanded a second time through
    /// `expand_arith_string` when the option is unset. The text reaches the
    /// parser with no further shell expansion: a surviving `$(...)`, backquote
    /// or `$name` is not a valid arithmetic token, so expr.c `readtok` /
    /// `expr_streval` fails it as "operand expected" instead of executing it
    /// (verified: GNU 5.3 `a[$x]=v` with `x='$(echo INJ; echo 0)'` under
    /// `shopt -s array_expand_once` prints
    /// `$(echo INJ; echo 0): arithmetic syntax error: operand expected` and
    /// never runs the substitution). Variable names, operators and assignment
    /// side effects (`a[i++]=v`) still evaluate normally and their writes
    /// sync back to shell_state.
    pub(in crate::executor) fn eval_indexed_subscript_expression(
        &mut self,
        resolved: &str,
    ) -> Option<i128> {
        // Same resolved text at the same `${}` site already produced its
        // index (and its side effects) in an earlier pass; GNU's
        // array_expand_index evaluates it once.
        let memo_key = crate::executor::expand_braced_indices::sub_site_key(resolved);
        if let Some(hit) = memo_key
            .as_ref()
            .and_then(crate::executor::expand_braced_indices::sub_idx_lookup)
        {
            return match hit {
                crate::executor::subscript_expansion::IndexedSubscript::Index(index) => Some(index),
                _ => None,
            };
        }
        self.shell_state.arithmetic_last_error_category.set(None);
        let _ = take_arith_eval_error();
        let _ = take_arith_eval_diags();
        self.shell_state
            .env_vars
            .remove("__RUBASH_ARITH_SUBSCRIPT_EXPR");
        // GNU array_expand_index (arrayfunc.c:1368-1378): the resolved
        // subscript text is expanded AGAIN by expand_arith_string unless
        // array_expand_once is set — `a[$x]` in an already-expanded word
        // still expands `$x` under the default. The Q_DOUBLE_QUOTES variant
        // keeps quote characters as data (`a[\" \"]=v` -> evalexp(`" "`)
        // fails "operand expected", verified GNU 5.3).
        let reexpanded = if crate::builtins::shopt::option_enabled(
            &self.shell_state.env_vars,
            "array_expand_once",
        ) {
            resolved
                .replace(DATA_DOLLAR, "$")
                .replace(crate::executor::markers::DATA_BACKTICK, "`")
                .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
                .replace(crate::executor::markers::DATA_SQUOTE, "'")
                .replace(crate::executor::markers::DATA_DQUOTE, "\"")
        } else {
            self.expand_arithmetic_subscript_mut(resolved)
        };

        // GNU arrayfunc.c:1376 evalexp(t, eflag): eflag=0 for compat>51 —
        // the nested assoc subscript scan stays flag-0 even under
        // array_expand_once (AV_NOEXPAND applied to the OUTER subscript's
        // expand_arith_string, not this inner evaluation).
        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(&reexpanded, false);

        let expression = normalize_arithmetic_quotes(&with_assoc_keys);
        *self.arithmetic_last_eval_input.borrow_mut() = expression.clone();
        ARITH_WRITES.with(|log| log.borrow_mut().clear());
        let (value, category) = eval_mutable_arith_value_with_random_flags(
            &expression,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            true,
        );
        self.shell_state
            .arithmetic_last_error_category
            .set(category);
        self.report_arithmetic_readonly_error();
        self.flush_arith_diags(None);
        sync_arith_writes_to_shell_state(self);
        if let Some(key) = memo_key {
            crate::executor::expand_braced_indices::sub_idx_store(
                key,
                match value {
                    Some(index) => {
                        crate::executor::subscript_expansion::IndexedSubscript::Index(index)
                    }
                    None => crate::executor::subscript_expansion::IndexedSubscript::Error,
                },
            );
        }
        value
    }

    /// GNU expr.c `evalerror` -> `jump_to_top_level (DISCARD)`: an
    /// arithmetic evaluation failure discards the rest of the command list
    /// that contained the failing command — `a[$x]=v; echo after` never
    /// prints `after` (verified GNU 5.3). The reader-level loop in
    /// `execute_ast_inner` skips the commands sharing the failing command's
    /// source line; nested lists unwind silently.
    pub(in crate::executor) fn raise_evalerror_abort(&self) {
        self.evalerror_pending.set(true);
    }

    /// GNU expr.c `evalerror` diagnostic for a subscript evaluated through
    /// `array_expand_index`'s `evalexp` path (arrayfunc.c:1353-1391): the
    /// parse stops at the first unparseable token and `lasttp` supplies the
    /// error token verbatim, e.g.
    /// `$x: arithmetic syntax error: operand expected (error token is "$x")`.
    /// The token is located by re-parsing the resolved text under the same
    /// no-expand rules with an empty env, so a surviving `$name`/`$(...)`
    /// fails at the same byte position the real evaluation did.
    pub(in crate::executor) fn report_indexed_subscript_error(&self, resolved: &str) {
        // GNU evalerror prints once and longjmps DISCARD, so a word that is
        // expanded more than once by our pipeline (eval re-parse, braced
        // expansion retries, embedded-parameter passes) still reports only
        // the first failure — later invocations are already dead code in
        // GNU's model.
        if self.evalerror_pending.get() {
            self.raise_evalerror_abort();
            return;
        }
        self.raise_evalerror_abort();
        // The subscript's nested evalexp recorded the real evalerror state —
        // its frame expression IS the subscript text, with lasttp inside it.
        if let Some(record) = take_arith_eval_error() {
            eprintln!("{}{}", self.diagnostic_prefix(), record.render(true));
        } else {
            let token = indexed_noexpand_error_token(resolved);
            eprintln!(
                "{}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                self.diagnostic_prefix(),
                resolved,
                token
            );
        }
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    /// Evaluate a `$(( ... ))` expansion embedded in a word. This is the
    /// expansion context: Bash strips double quotes from the expression
    /// before evaluation (`$(( "1" + 1 ))` is `2`), while the command
    /// context (`for (( ... ))` headers) keeps them and rejects them.
    pub(crate) fn eval_arithmetic_expansion_value(&mut self, expression: &str) -> Option<i128> {
        self.shell_state.arithmetic_last_error_category.set(None);
        let _ = take_arith_eval_error();
        let _ = take_arith_eval_diags();
        self.shell_state
            .env_vars
            .remove("__RUBASH_ARITH_SUBSCRIPT_EXPR");

        // $(( )) runs its own expansion pass inside evalexp — the operand is
        // not EXP_EXPANDED, so the assoc subscript scan stays flag-0
        // (expr.c:1171).

        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(expression, false);
        let expression =
            normalize_arithmetic_quotes(&self.expand_arithmetic_expression_mut(&with_assoc_keys));
        *self.arithmetic_last_eval_input.borrow_mut() = expression.clone();
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.shell_state.env_vars)
            {
                self.shell_state.arithmetic_nounset_error.set(true);
                if !self.shell_state.arithmetic_expansion_error.replace(true) {
                    eprintln!("{}{}: unbound variable", self.diagnostic_prefix(), name);
                    use std::io::Write;
                    let _ = std::io::stderr().flush();
                }
                return None;
            }
        }
        if empty_quoted_operand_has_operator(&expression) {
            return None;
        }
        // Save a snapshot to detect variable changes from arithmetic side effects
        ARITH_WRITES.with(|log| log.borrow_mut().clear());
        // GNU expr.c: the expansion pass above already ran, so the parser
        // evaluates under evalexp's already-expanded rules — a surviving
        // `$name`/`$(...)` is "operand expected" data, not a re-expansion.
        let (value, category) = eval_mutable_arith_value_with_random_flags(
            &expression,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            true,
        );
        self.shell_state
            .arithmetic_last_error_category
            .set(category);
        self.report_arithmetic_readonly_error();
        // Expansion context: this_command_name is NULL inside evalexp for
        // $(( )), so bind/subscript diagnostics carry no command label.
        self.flush_arith_diags(None);
        // GNU $((r=0)) with an empty nameref cell still yields the assigned
        // value while reporting the failed bind without a command label.
        self.report_arithmetic_nameref_error(None);

        // Sync any variable changes from env_vars to shell_state.variables
        // so that subsequent parameter expansions see arithmetic side effects
        // (e.g., ++i in array subscripts like a[++i]=value).
        sync_arith_writes_to_shell_state(self);

        value
    }

    fn report_arithmetic_readonly_error(&mut self) {
        let Some(name) = self
            .shell_state
            .env_vars
            .remove("__RUBASH_ARITH_READONLY_ERROR")
        else {
            return;
        };
        if !self.shell_state.arithmetic_expansion_error.replace(true) {
            eprintln!("{}{}: readonly variable", self.diagnostic_prefix(), name);
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
    }

    /// A fatal arithmetic evaluation error (expr.c evalerror) raised while
    /// expanding parts of a compound command — case patterns, loop headers —
    /// abandons the whole compound command with status 1 (127 under `set -u`
    /// unbound). Compound executors call this after expansion steps because
    /// the simple-command flag check in command_execute.rs does not apply to
    /// them.
    pub(crate) fn abandon_on_arithmetic_expansion_error(
        &mut self,
    ) -> Result<(), crate::executor::ExecuteError> {
        if !self.shell_state.arithmetic_fatal_error.get()
            && !self.shell_state.arithmetic_nounset_error.get()
        {
            return Ok(());
        }
        let nounset = self.shell_state.arithmetic_nounset_error.replace(false);
        self.shell_state.arithmetic_fatal_error.set(false);
        self.shell_state.arithmetic_expansion_error.set(false);
        if nounset {
            // `set -u` unbound in a compound-expansion context (case words,
            // loop headers) terminates the noninteractive shell exactly like
            // the simple-command path (GNU probe a5: `case $((b)) in ...`
            // exits the script).
            self.exit_code = 127;
            return Err(crate::executor::ExecuteError::ExitCode(127));
        }
        self.exit_code = 1;
        Err(crate::executor::ExecuteError::ExpansionFailure(1))
    }

    /// GNU subst.c expand_subscript_string: an associative-array subscript is
    /// word-expanded exactly once — parameter, command, arithmetic and tilde
    /// expansion plus quote removal, with no field splitting, no pathname
    /// expansion and no process substitution — and the result is used
    /// verbatim as the key, never re-expanded.
    ///
    /// expr.c reaches it through `array_variable_part` -> `array_value_internal`
    /// (`arrayfunc.c:1596`, `akey = expand_subscript_string (t, 0)`) for every
    /// form that names the element: `(( A[sub] ))`, `(( A[sub] = v ))`,
    /// `(( A[sub]++ ))`, `$(( A[sub] ))` and `(( x = A[sub] ))`.
    ///
    /// Rubash's arithmetic parser never sees an `Executor` (it works on
    /// `env_vars` alone), so the expansion happens here, before evaluation:
    /// each subscript is replaced by [`ARITH_ASSOC_KEY_MARKER`] followed by a
    /// hex encoding of the expanded key. The parser reads it back untouched
    /// (`lvalue::parse_assoc_subscript`), which is what keeps
    /// `A['$v']` → `$v` and `k='$w'; A[$k]` → `$w` from expanding twice.

    /// `noexpand` models GNU's VA_NOEXPAND (expr.c:1171 `tflag` /
    /// expr.c:361-378 expr_skipsubscript): when the caller's operand is
    /// already word-expanded (`let`/`[[` — EXP_EXPANDED) and
    /// `array_expand_once` is set, the subscript scan takes the FIRST `]`
    /// (quotes are plain data) and the text between brackets is the
    /// associative key verbatim — `let "++a[$b]"` keys on `80's` and
    /// `let '++a[$b]'` keys on `$b`.
    pub(in crate::executor) fn expand_arithmetic_assoc_subscripts(
        &mut self,
        expression: &str,
        noexpand: bool,
    ) -> String {
        let bytes = expression.as_bytes();
        let mut output = String::with_capacity(expression.len());
        let mut index = 0usize;
        while index < bytes.len() {
            let ch = bytes[index];
            if !(ch.is_ascii_alphabetic() || ch == b'_') {
                // A substitution yields a value, not an arithmetic lvalue, so
                // anything inside it (`${A[sub]}` — where the braced-parameter
                // expander already runs expand_subscript_string once — or
                // `$(...)`) is left to the ordinary expansion below. Without
                // this the key of `${A[sub]}` was replaced by its own marker
                // and the lookup missed (assoc16.sub `$(( ${A[$(echo
                // Darwin)]} ))`).
                let is_substitution = ch == b'`'
                    || (ch == b'$' && matches!(bytes.get(index + 1), Some(&b'(') | Some(&b'{')));
                if is_substitution {
                    let end = assoc_skip_substitution(bytes, index);
                    output.push_str(&expression[index..end]);
                    index = end;
                    continue;
                }
                let next = expression[index..].chars().next().unwrap_or_default();
                output.push(next);
                index += next.len_utf8();
                continue;
            }
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &expression[start..index];
            if index < bytes.len()
                && bytes[index] == b'['
                && is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
            {
                // GNU skipsubscript flag-1 (VA_NOEXPAND): the first `]`
                // closes the subscript — quotes, escapes and nested
                // brackets are plain data (subst.c:2186 flags&1).
                let end = if noexpand {
                    bytes[index..]
                        .iter()
                        .position(|&byte| byte == b']')
                        .map(|offset| index + offset + 1)
                        .unwrap_or(bytes.len())
                } else {
                    assoc_subscript_end(bytes, index)
                };
                if end > index + 1 && bytes.get(end - 1) == Some(&b']') {
                    let raw = &expression[index + 1..end - 1];
                    if raw.starts_with(ARITH_ASSOC_KEY_MARKER) {
                        // Caller already ran this pass (e.g. the substring
                        // offset path pre-encodes before delegating to
                        // eval_arithmetic_expansion_value, which encodes
                        // again): re-encoding the marker text would make the
                        // decoded key the encoded string itself, so the
                        // lookup misses. The pass must be idempotent.
                        output.push_str(name);
                        output.push('[');
                        output.push_str(raw);
                        output.push(']');
                        index = end;
                        continue;
                    }
                    // GNU expr.c:1171 expr_streval: under array_expand_once
                    // an EXP_EXPANDED operand (`let`/`[[` args, already
                    // word-expanded) takes AV_NOEXPAND — the subscript text
                    // is used verbatim, so `a[" "]` keys on `" "` with the
                    // quote characters kept as data (array25.sub 6/8).
                    let key = if noexpand {
                        // The already-expanded text is the key verbatim;
                        // only the lexer's data-quote/dollar markers come
                        // back to their characters (eval_indexed_subscript_
                        // expression's ExpandedOnce branch does the same).
                        raw.replace('', "$")
                            .replace('', "`")
                            .replace('', "\\")
                            .replace('', "'")
                            .replace('', "\"")
                    } else {
                        self.expand_assoc_subscript_once(raw)
                    };
                    output.push_str(name);
                    output.push('[');
                    output.push_str(&encode_arithmetic_assoc_key(&key));
                    output.push(']');
                    index = end;
                    continue;
                }
            }
            output.push_str(name);
        }
        output
    }

    /// GNU arrayfunc.c:1368-1378 array_expand_index: when
    /// `array_expand_once` is unset (the default), a subscript inside an
    /// EXP_EXPANDED evaluation — `let` operands, `[[` arithcomp operands —
    /// is still run through expand_arith_string. Only the subscript text is
    /// expanded (`let 'jv += $iv'` keeps `$iv` as operand-expected data),
    /// so this pass must not expand anything outside `name[...]`. The
    /// expanded text is marker-encoded like associative keys so an empty
    /// result (`a[$i]` with i unset -> index 0) stays distinguishable from
    /// a literal `a[]` bad subscript.
    pub(in crate::executor) fn expand_arith_indexed_subscripts(
        &mut self,
        expression: &str,
    ) -> String {
        let bytes = expression.as_bytes();
        let mut output = String::with_capacity(expression.len());
        let mut index = 0usize;
        while index < bytes.len() {
            let ch = bytes[index];
            if !(ch.is_ascii_alphabetic() || ch == b'_') {
                let is_substitution = ch == b'`'
                    || (ch == b'$' && matches!(bytes.get(index + 1), Some(&b'(') | Some(&b'{')));
                if is_substitution {
                    let end = assoc_skip_substitution(bytes, index);
                    output.push_str(&expression[index..end]);
                    index = end;
                    continue;
                }
                let next = expression[index..].chars().next().unwrap_or_default();
                output.push(next);
                index += next.len_utf8();
                continue;
            }
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &expression[start..index];
            if index < bytes.len()
                && bytes[index] == b'['
                && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
            {
                let end = assoc_subscript_end(bytes, index);
                if end > index + 1 && bytes.get(end - 1) == Some(&b']') {
                    let raw = &expression[index + 1..end - 1];
                    // A literally-empty `a[]` is a bad subscript, not an
                    // expansion — leave it for the parser. An already
                    // marker-encoded subscript is the caller's finished
                    // product — pass it through so a second pass stays
                    // idempotent (same rule as the assoc scanner above).
                    if !raw.is_empty() && !raw.starts_with(ARITH_ASSOC_KEY_MARKER) {
                        let expanded = self.expand_arithmetic_expression_mut(raw);
                        output.push_str(name);
                        output.push('[');
                        output.push_str(&encode_arithmetic_assoc_key(&expanded));
                        output.push(']');
                        index = end;
                        continue;
                    }
                }
            }
            output.push_str(name);
        }
        output
    }

    /// Subscript re-expansion for eval contexts whose eflag lets
    /// array_expand_index expand (`let`'s EXP_EXPANDED operands, `[[`
    /// arithcomp operands) — a no-op under `shopt -s array_expand_once`
    /// (expr.c:1171, test.c:652).
    pub(in crate::executor) fn expand_arith_eval_subscripts(&mut self, expression: &str) -> String {
        if crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "array_expand_once") {
            return expression.to_string();
        }
        self.expand_arith_indexed_subscripts(expression)
    }

    /// One `expand_subscript_string` pass over a raw subscript. A wholly
    /// single-quoted subscript is literal data — nothing expands inside a
    /// single-quoted span, which is why `A['$v']` keys on `$v` — and anything
    /// else goes through the ordinary word expansion, which stops at one round.
    ///
    /// NOTE: the arithmetic evaluator is handed the `(( ))` body AFTER the
    /// lexer's own escape pass, so its subscript text is not the source
    /// spelling; re-running the assignment path's quote-removal pass here
    /// would drop one backslash too many (`A[a\\b]` read `a\b` instead of
    /// `ab`). The assignment path owns the faithful
    /// `expand_subscript_string`; this path keeps its own one-round word
    /// expansion until the `(( ))` body reaches the evaluator unescaped.
    fn expand_assoc_subscript_once(&mut self, raw: &str) -> String {
        if let Some(literal) =
            crate::executor::subscript_expansion::wholly_single_quoted_literal(raw)
        {
            return literal;
        }
        // GNU expand_word_internal never re-lexes `'` — sq is lex-time only,
        // so a `'` reaching expand_subscript_string here is always data
        // produced by the earlier expansion (`let "++a[$b]"` with
        // b=`80's`). Mark it with the \x17 carrier so the word expansion
        // emits it verbatim instead of consuming it as an sq opener —
        // without this the key stored `80s` (assoc9.sub `let "++a[$b]"`).
        let raw = crate::executor::subscript_expansion::mark_expanded_once_data_squotes(raw);
        self.expand_word_mut_with_context(&raw, SubstitutionQuoteContext::Unquoted)
    }

    pub(super) fn expand_arithmetic_special_parameters(&self, expression: &str) -> String {
        // In arithmetic contexts, special parameters expand to numeric values:
        // $- -> 0 (shell flags not meaningful in arithmetic), $# -> param count
        // GNU Bash treats $- as 0 in $(( $- )). See array.tests line 60.
        let expression = expression
            .replace("$#", &self.shell_state.positional_params.len().to_string())
            .replace("$-", "0");
        // GNU subst.c: the text between (( and )) is treated as if in double
        // quotes — single quotes are literal data for expr.c (`(( '1' ))` is
        // an operand error, not the number 1), so they must survive the
        // embedded-parameter walker's quote removal. \x17 is the walker's
        // literal-single-quote marker. Backslash-escaped double quotes
        // (`\"`) must also survive as literal `"` — the walker strips bare
        // `"` via toggle mode, so `\"` → `\` + removed quote. \x18 is the
        // walker's literal-double-quote marker.
        // Quotes INSIDE a ${...}/$(...)/`...` span are not arith-text data —
        // they belong to the nested substitution's own expansion, where
        // expand_subscript_string strips them (assoc16.sub:
        // $(( ${A['lit']} )) keys on `lit`, not `'lit'`).
        let bytes = expression.as_bytes();
        let mut protected = String::with_capacity(expression.len());
        let mut index = 0usize;
        while index < bytes.len() {
            let ch = bytes[index];
            if ch == b'`' || (ch == b'$' && matches!(bytes.get(index + 1), Some(b'(') | Some(b'{')))
            {
                let end = assoc_skip_substitution(bytes, index);
                protected.push_str(&expression[index..end]);
                index = end;
                continue;
            }
            match ch {
                b'\\' if bytes.get(index + 1) == Some(&b'"') => {
                    protected.push(crate::executor::markers::DATA_DQUOTE);
                    index += 2;
                }
                b'\\' if bytes.get(index + 1) == Some(&b'$') => {
                    protected.push(DATA_DOLLAR);
                    index += 2;
                }
                b'\'' => {
                    protected.push(crate::executor::markers::DATA_SQUOTE);
                    index += 1;
                }
                _ => {
                    let next = expression[index..].chars().next().unwrap_or_default();
                    protected.push(next);
                    index += next.len_utf8();
                }
            }
        }
        self.expand_embedded_parameters(&protected)
            .replace(DATA_DOLLAR_STR, "$")
    }
}

pub(super) fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

fn empty_quoted_operand_has_operator(expression: &str) -> bool {
    let chars = expression.chars().collect::<Vec<_>>();
    let mut outside = String::new();
    let mut index = 0;
    let mut found_empty = false;
    while index < chars.len() {
        if chars[index] == '"' {
            let start = index + 1;
            index = start;
            while index < chars.len() && chars[index] != '"' {
                index += 1;
            }
            if index == chars.len() {
                return false;
            }
            if chars[start..index].iter().all(|ch| ch.is_whitespace()) {
                found_empty = true;
            } else {
                outside.extend(chars[start..index].iter().copied());
            }
            index += 1;
        } else {
            outside.push(chars[index]);
            index += 1;
        }
    }
    found_empty
        && outside.chars().any(|ch| {
            matches!(
                ch,
                '+' | '-' | '*' | '/' | '%' | '<' | '>' | '&' | '|' | '^' | '?' | ':'
            )
        })
}

// Word-expansion arithmetic failures abort the enclosing command list:
// top-level lists end the noninteractive run while function bodies stop
// before their remaining commands. GNU Bash 5.2 evidence (2026-08-24):
// $((08)), $((2#2)), $((1/0)), $((1=2)), $((1++)) and $((4 ? 20 : )) all
// print one diagnostic then skip every later command with status 1. A
// `( )` frame ends only the subshell; the caller continues. `let` and
// `(( ))` never reach this predicate: their evaluation failures stay
// nonfatal status-1 continuations (probes d2/d3).
pub(crate) fn arithmetic_expansion_is_fatal(expression: &str) -> bool {
    arithmetic_error_category(expression).is_some()
}

pub(crate) fn arithmetic_error_category(expression: &str) -> Option<ArithmeticErrorCategory> {
    let mut env_vars = HashMap::new();
    let (_, category) = eval_mutable_arith_result(expression, &mut env_vars, None, false);
    category
}

/// GNU expr.c:484-485: when `curtok != 0` after `EXP_LOWEST()`, the parser
/// reports "arithmetic syntax error in expression" with `lasttp` as the
/// error token. `lasttp` points to the start of the last-read token, which
/// for a trailing-input case like `x=9 y=41 ` is the beginning of the
/// unparsed remainder (`y=41 `, trailing blank included). This function
/// re-parses the expression and returns that trailing remainder so the
/// diagnostic can format it exactly as GNU does.
pub(in crate::executor) fn trailing_input_token(
    expression: &str,
) -> Option<(String, TrailingInputKind)> {
    let normalized = normalize_arithmetic_quotes(expression);
    if normalized.trim().is_empty() {
        return None;
    }
    // Numeric assignment expressions like `7 = 43` are handled by the
    // non-variable-assignment path, not the trailing-input path.
    if numeric_assignment_expression(&normalized) {
        return None;
    }
    let mut env_vars = HashMap::new();
    let mut parser = ConditionalArithParser {
        input: normalized.as_bytes(),
        pos: 0,
        env_vars: &mut env_vars,
        resolving: Vec::new(),
        random_state: None,
        error_category: None,
        no_expand: false,
        error: None,
        diags: Vec::new(),
        last_tok_start: 0,
        last_tok_operand: false,
    };
    let parsed_ok = parser.parse_comma().is_some();
    parser.skip_ws();
    if parser.pos < parser.input.len() {
        // GNU lasttp points to the token start; the remainder from there to
        // the end of the expression is the error token (trailing blank kept).
        let token = &normalized[parser.pos..];
        if !token.is_empty() {
            // GNU expr.c: a mid-parse failure means the stop position was an
            // operand slot (exp0 -> operand expected). A completed parse with
            // leftover input splits on whether the leftover can begin a
            // token at all: junk characters hit readtok's
            // invalid-arithmetic-operator branch (expr.c:1507-1509, curtok
            // is an operand), valid token starts are plain trailing input
            // (expr.c:485).
            let kind = if !parsed_ok {
                TrailingInputKind::OperandExpected
            } else if token_starts_with_non_arith_char(token) {
                TrailingInputKind::InvalidOperator
            } else {
                TrailingInputKind::InExpression
            };
            return Some((token.to_string(), kind));
        }
    }
    None
}

/// GNU expr.c `evalerror` "operand expected" token for a subscript
/// evaluated through arrayfunc.c:1353-1391 `array_expand_index` ->
/// `evalexp`: the parse runs under the AV_NOEXPAND rules (a surviving
/// `$(...)`, backquote or `$name` is never expanded or executed), stops at
/// the first unparseable token, and `lasttp` supplies the remainder of the
/// expression verbatim as the error token. An empty env is used so a bare
/// variable name also stops the parse at the same position it would during
/// the real subscript evaluation.
fn indexed_noexpand_error_token(resolved: &str) -> String {
    let normalized = normalize_arithmetic_quotes(resolved);
    let mut env_vars = HashMap::new();
    let mut parser = ConditionalArithParser {
        input: normalized.as_bytes(),
        pos: 0,
        env_vars: &mut env_vars,
        resolving: Vec::new(),
        random_state: None,
        error_category: None,
        no_expand: true,
        error: None,
        diags: Vec::new(),
        last_tok_start: 0,
        last_tok_operand: false,
    };
    let _ = parser.parse_comma();
    parser.skip_ws();
    normalized[parser.pos.min(normalized.len())..].to_string()
}

/// GNU expr.c token-starter test for a byte: a digit, a
/// `legal_variable_starter` (alpha/underscore), or an `is_arithop`
/// character (expr.c:1285-1303) can begin a token; anything else hits
/// readtok's junk branch (expr.c:1502-1510).
fn token_can_start(ch: &u8) -> bool {
    ch.is_ascii_alphanumeric()
        || *ch == b'_'
        || matches!(
            *ch,
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

/// True when `token` starts with a character GNU's arithmetic tokenizer
/// cannot begin a token with -- not a digit, not a `legal_variable_starter`
/// (alpha/underscore), and not an `is_arithop` character
/// (expr.c:1285-1303). `@`, `#`, `[`, `;` land here; `~` is BNOT (a real
/// operator token) and stays plain trailing input.
fn token_starts_with_non_arith_char(token: &str) -> bool {
    token
        .as_bytes()
        .first()
        .is_some_and(|ch| !token_can_start(ch))
}

pub(crate) fn eval_conditional_arith_value(
    value: &str,
    env_vars: &HashMap<String, String>,
) -> Option<i128> {
    let mut env_vars = env_vars.clone();
    eval_mutable_arith_value(value, &mut env_vars)
}

/// Like eval_conditional_arith_value, but also returns the variables that
/// were modified by side effects (e.g. count++ in ${arr[$((count++))]}).
/// The caller is responsible for applying these writes to the real env_vars.
pub(crate) fn eval_conditional_arith_value_with_writes(
    value: &str,
    env_vars: &HashMap<String, String>,
) -> (Option<i128>, Vec<(String, String)>) {
    let mut cloned = env_vars.clone();
    let result = eval_mutable_arith_value(value, &mut cloned);
    let writes = cloned
        .iter()
        .filter(|(name, _)| name.as_str() != "__RUBASH_ARITH_SUBSCRIPT_EXPR")
        .filter(|(name, new_value)| env_vars.get(name.as_str()) != Some(new_value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    (result, writes)
}

/// Like `eval_conditional_arith_value`, but also reports the error category
/// from the actual evaluation, so callers can classify fatality without
/// re-evaluating the expression in a fresh environment (GNU expr.c raises
/// evalerror from the real evaluation; state-dependent errors such as
/// `x+=2` with `x` declared only disappear under a fresh environment).
pub(crate) fn eval_conditional_arith_value_categorized(
    value: &str,
    env_vars: &HashMap<String, String>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    let mut env_vars = env_vars.clone();
    eval_mutable_arith_result(value, &mut env_vars, None, false)
}

/// `eval_conditional_arith_value_categorized` plus the write-capture of
/// `eval_conditional_arith_value_with_writes`: `&self` arithmetic
/// expansions (`$((i++))` inside a `${}` body or array subscript) still
/// have GNU-visible side effects, so the deltas are queued for the
/// mutable caller to apply.
pub(crate) fn eval_conditional_arith_value_categorized_with_writes(
    value: &str,
    env_vars: &HashMap<String, String>,
) -> (
    Option<i128>,
    Vec<(String, String)>,
    Option<ArithmeticErrorCategory>,
) {
    let mut cloned = env_vars.clone();
    let (result, category) = eval_mutable_arith_result(value, &mut cloned, None, false);
    let writes = cloned
        .iter()
        .filter(|(name, _)| name.as_str() != "__RUBASH_ARITH_SUBSCRIPT_EXPR")
        .filter(|(name, new_value)| env_vars.get(name.as_str()) != Some(new_value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    (result, writes, category)
}

pub(super) fn arithmetic_unbound_variable(
    expression: &str,
    env_vars: &HashMap<String, String>,
) -> Option<String> {
    let mut chars = expression.chars().peekable();
    let mut previous = None;
    while let Some(ch) = chars.next() {
        if !(ch == '_' || ch.is_ascii_alphabetic()) {
            previous = Some(ch);
            continue;
        }
        // Do not mistake digits in hexadecimal or `base#digits` literals for
        // variable names while nounset validation scans the expression.
        if previous.is_some_and(|prev| prev.is_ascii_digit() || prev == '#') {
            while chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_')
            {
                previous = chars.next();
            }
            continue;
        }
        let mut name = String::from(ch);
        while chars
            .peek()
            .is_some_and(|next| *next == '_' || next.is_ascii_alphanumeric())
        {
            name.push(chars.next().expect("peeked arithmetic identifier"));
        }
        // An associative-array subscript is a string key, not an arithmetic
        // operand: GNU expr.c never routes it through expr_streval, so any
        // name inside it is data and must not be reported as unbound
        // (`set -u; declare -A A; (( A[k] ))` is fine, and a key like `x1`
        // is not a variable). An *indexed* subscript is evaluated
        // arithmetically, so its identifier is a real read and keeps the
        // check (`set -u; echo $(( I[j] ))` reports `j`).
        if chars.peek() == Some(&'[') && is_marked_var(env_vars, ASSOC_VARS, &name) {
            chars.next();
            skip_subscript_chars(&mut chars);
            previous = Some(']');
            continue;
        }
        // An identifier that is the left-hand side of an assignment
        // (`i=0`, `i+=1`, `x = 2`) is a write target, not a value read.
        // GNU expr.c only routes *reads* through expr_streval, so `set -u;
        // for ((i=0; i<n; i++))` must not report `i` as unbound (issue #67).
        // A following `==` is comparison and keeps the read check.
        if arithmetic_identifier_is_assignment_lhs(&mut chars) {
            previous = name.chars().last();
            continue;
        }
        if !env_vars.contains_key(&name)
            && !matches!(
                name.as_str(),
                // Dynamic parameters resolved by the evaluator without an
                // env_vars entry (RANDOM/SRANDOM advance the RNG state).
                "RANDOM" | "SRANDOM" | "SECONDS" | "EPOCHSECONDS" | "LINENO"
            )
        {
            return Some(name);
        }
        previous = name.chars().last();
    }
    // Every identifier in the expression is bound: leave validation to the
    // real evaluator. GNU expr.c only raises the nounset error for a
    // reference to an unset variable (expr_streval); a malformed expression
    // (`a b`, `x=9 y=41`) is reported by the parser with its own
    // "syntax error in expression" diagnostic regardless of `set -u`.
    // Returning a synthesized error here used to make `set -u; a=0;
    // echo $((a))` fail with rc=127 (issue #67).
    None
}

/// Consume the rest of an array subscript whose opening `[` was just read,
/// tracking nested brackets, quotes and escapes so a `]` inside a quoted or
/// nested key does not end it early.
fn skip_subscript_chars(chars: &mut std::iter::Peekable<std::str::Chars>) {
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for next in chars.by_ref() {
        if escaped {
            escaped = false;
            continue;
        }
        if next == '\\' && !single {
            escaped = true;
            continue;
        }
        match next {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' if !single && !double => depth += 1,
            ']' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Returns true when the identifier just scanned is immediately followed by
/// an assignment operator (`=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `^=`,
/// `|=`, `<<=`, `>>=`, `**=`), optionally after blanks. A `==` at the same
/// position is an equality test, so the identifier is still a read.
fn arithmetic_identifier_is_assignment_lhs(
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> bool {
    while chars.peek().is_some_and(|next| matches!(next, ' ' | '\t')) {
        chars.next();
    }
    let rest_starts_with = |pattern: &str| -> bool {
        // Peekable has no multi-char lookahead; clone the remaining iterator.
        let mut clone = chars.clone();
        pattern
            .chars()
            .all(|expected| clone.next().is_some_and(|actual| actual == expected))
    };
    if rest_starts_with("==") || !rest_starts_with("=") {
        return false;
    }
    // Consume the operator so the outer scan resumes after it.
    for _ in 0..rest_assignment_operator_len(chars) {
        chars.next();
    }
    true
}

fn rest_assignment_operator_len(chars: &std::iter::Peekable<std::str::Chars>) -> usize {
    let rest: String = chars.clone().collect();
    for op in [
        "<<=", ">>=", "**=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
    ] {
        if rest.starts_with(op) {
            return op.len();
        }
    }
    1
}

/// Strip double quotes from an arithmetic expression before evaluation.
///
/// Bash's expansion context (`$(( ... ))`, `(( ... ))` command, array
/// subscripts, ...) removes double quotes from the expression before the
/// arithmetic evaluator runs: `$(( "1" + 1 ))` is `2` and `$(( "i < 3" ))`
/// evaluates `i < 3`. Single quotes are preserved so the error path can
/// report `operand expected` for `$(( '1' ))` exactly like Bash.
pub(super) fn strip_arith_double_quotes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    // Walk `chars` rather than `as_bytes`: `byte as char` would Latin-1-encode
    // multibyte operand text (e.g. `$((中))` diagnostics print `ä¸­`).
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            output.push(ch);
            continue;
        }
        while let Some(next) = chars.next() {
            if next == '"' {
                break;
            }
            if next == '\\' {
                if let Some(inner) = chars.next() {
                    output.push(inner);
                } else {
                    output.push('\\');
                }
            } else {
                output.push(next);
            }
        }
    }
    output
}

/// GNU expand_arith_string output form for an indexed-array subscript that
/// carried no expansion of its own: single quotes stay literal (evalexp
/// reports `'x'` as "operand expected"), while double quotes and backslash
/// escapes are removed (`a[" "]` resolves to 0, `a[' ']` errors). Used when
/// the cooked index equals the dequoted raw — i.e. the subscript expanded
/// to itself — so rebuilding the arith-context text from the raw spelling
/// cannot re-run substitutions (`a[$(echo INJ)]=v` still executes once).
pub(super) fn arith_subscript_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                out.push('\'');
                for inner in chars.by_ref() {
                    out.push(inner);
                    if inner == '\'' {
                        break;
                    }
                }
            }
            '"' => {
                while let Some(inner) = chars.next() {
                    match inner {
                        '"' => break,
                        '\\' => {
                            if let Some(next) = chars.next() {
                                out.push(next);
                            } else {
                                out.push('\\');
                            }
                        }
                        _ => out.push(inner),
                    }
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                } else {
                    out.push('\\');
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Byte index just past the `]` that closes the subscript opened at `open`
/// (`bytes[open] == b'['`), honoring single/double quotes and `\` escapes so a
/// `]` inside a quoted key does not terminate the subscript.
pub(crate) fn assoc_subscript_end(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut index = open;
    while index < bytes.len() {
        let ch = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == b'\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if !single && !double {
            // A `]` inside a substitution is data, not the subscript close:
            // GNU's parser skips `$(...)`, `$((...))`, `${...}` and backticks
            // as units when it looks for the matching bracket, so
            // `A[$(echo a]b)]` keys on `a]b`.
            if ch == b'$' && matches!(bytes.get(index + 1), Some(&b'(') | Some(&b'{')) {
                index = assoc_skip_substitution(bytes, index);
                continue;
            }
            if ch == b'`' {
                index = assoc_skip_substitution(bytes, index);
                continue;
            }
        }
        index += 1;
        match ch {
            b'\'' if !double => single = !single,
            b'"' if !single => double = !double,
            b'[' if !single && !double => depth += 1,
            b']' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    // GNU skipsubscript flag-0 (subst.c:2186 -> skip_matched_pair): a `]`
    // inside an open quote does not close the subscript, so `a[80's]` is an
    // UNTERMINATED reference — the whole token is invalid (`bad array
    // subscript`), not a key with the quote stripped. Report `open` (no
    // match) so callers leave the text for the parser to diagnose.
    if single || double {
        return open;
    }
    index
}

/// Skip the shell substitution starting at `start` (`bytes[start] == b'$'` for
/// `$(`, `$((`, `${`, or a backtick) and return the index just past it. An
/// unterminated substitution runs to the end of the input so a missing close
/// can never make a stray `]` look like the subscript delimiter.
pub(in crate::executor) fn assoc_skip_substitution(bytes: &[u8], start: usize) -> usize {
    let opener = bytes[start];
    let (open, close) = if opener == b'`' {
        (b'`', b'`')
    } else {
        match bytes.get(start + 1) {
            Some(b'(') => (b'(', b')'),
            Some(b'{') => (b'{', b'}'),
            _ => return start + 1,
        }
    };
    let mut index = if opener == b'`' { start } else { start + 1 };
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < bytes.len() {
        let ch = bytes[index];
        index += 1;
        if escaped {
            escaped = false;
            continue;
        }
        if ch == b'\\' && !single {
            escaped = true;
            continue;
        }
        match ch {
            b'\'' if !double => single = !single,
            b'"' if !single => double = !double,
            _ if single || double => {}
            _ if ch == open => depth += 1,
            _ if ch == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    index
}

/// Marker that introduces a pre-expanded associative-array subscript key in an
/// arithmetic expression (see `Executor::expand_arithmetic_assoc_subscripts`).
/// Control byte 0x1e: the parameter-expansion walker and the arithmetic parser
/// both pass it through unchanged, and it cannot appear in ordinary shell
/// source, so a subscript that starts with it is unambiguously a pre-expanded
/// key rather than user text.
pub(super) const ARITH_ASSOC_KEY_MARKER: char = crate::executor::markers::SUBSCRIPT_CARRIER;

/// Encode an expanded associative-subscript key so the arithmetic parser can
/// read it back verbatim. Hex digits keep the payload free of `$`, quotes,
/// backslashes and `]`, which would otherwise be re-interpreted.
pub(super) fn encode_arithmetic_assoc_key(key: &str) -> String {
    let mut encoded = String::with_capacity(1 + key.len() * 2);
    encoded.push(ARITH_ASSOC_KEY_MARKER);
    for byte in key.as_bytes() {
        encoded.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        encoded.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    encoded
}

/// Decode a subscript produced by [`encode_arithmetic_assoc_key`]; `None` when
/// `text` is ordinary user-written subscript text.
pub(crate) fn decode_arithmetic_assoc_key(text: &str) -> Option<String> {
    let hex = text.strip_prefix(ARITH_ASSOC_KEY_MARKER)?;
    let digits = hex.as_bytes();
    if digits.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        bytes.push((hi * 16 + lo) as u8);
    }
    String::from_utf8(bytes).ok()
}

/// A single quote in `expression` that is not part of an associative-array
/// subscript. GNU rejects `(( '1' ))` — a quoted operand is not a number — but
/// accepts a quoted assoc subscript (`(( A['a b']++ ))`), which is a string
/// key, so those quotes must not trigger the operand error.
fn has_bare_single_quote(expression: &str, env_vars: &HashMap<String, String>) -> bool {
    let bytes = expression.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return true;
        }
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if index < bytes.len()
                && bytes[index] == b'['
                && is_marked_var(env_vars, ASSOC_VARS, &expression[start..index])
            {
                let end = assoc_subscript_end(bytes, index);
                // An unterminated subscript (missing `]` or unclosed quote)
                // consumes the rest of the operand — its quotes are part of
                // the subscript text, so they are never "bare" here. The
                // malformed subscript itself reports `bad array subscript`
                // downstream (GNU expr.c expr_skipsubscript/subst.c
                // skipsubscript flag-0 semantics).
                index = if end == index { bytes.len() } else { end };
                continue;
            }
            continue;
        }
        index += 1;
    }
    false
}

fn normalize_arithmetic_quotes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && matches!(chars.peek(), Some('"')) {
            chars.next();
            output.push('"');
        } else {
            output.push(ch);
        }
    }
    output
}

/// Produces a Bash-style error message for an arithmetic expansion that
/// failed to evaluate (`$(( 1.5 ))`, `$(( 2 ** -1 ))`, division by zero, ...).
/// Rubash used to silently drop these; Bash reports them on stderr with rc=1.
///
/// GNU bash 5.3.0(1) has an invocation-mode split for `$(( ))` expansion
/// diagnostics that is observable only in the expansion context, not in
/// command contexts:
///   - `bash script.sh`  → `arithmetic syntax error: ...`
///   - `bash -c '...'`   → `syntax error: ...`
/// Command contexts (`(( ))`, `let`, `for ((;;))`, `[[ ]]`) always carry the
/// `arithmetic` prefix regardless of mode. Rubash tags `-c` invocations with
/// `__RUBASH_IS_C=1` (main.rs:253), so an expansion-context diagnostic
/// mirrors GNU only when that flag is absent. See
/// [`arithmetic_command_error_message`] for the always-`arithmetic` variant.
pub(in crate::executor) fn arithmetic_error_message(
    expression: &str,
    trailing_space: bool,
    env_vars: &HashMap<String, String>,
) -> Option<String> {
    let command_context = env_vars.get("__RUBASH_IS_C").map(String::as_str) != Some("1");
    arithmetic_error_message_ctx(expression, trailing_space, command_context)
}

/// Command-context variant: `(( ))` / `let` / `[[ ]]` diagnostics carry an
/// `arithmetic` prefix (verified bash 5.3.0: `(( 7++ ))` → `((: 7++ :
/// arithmetic syntax error: operand expected (error token is "+ ")`).
pub(in crate::executor) fn arithmetic_command_error_message(
    expression: &str,
    trailing_space: bool,
) -> Option<String> {
    arithmetic_error_message_ctx(expression, trailing_space, true)
}

fn arithmetic_error_message_ctx(
    expression: &str,
    trailing_space: bool,
    command_context: bool,
) -> Option<String> {
    // The real evaluation recorded the GNU evalerror state
    // (expr.c:1524-1535): the failing frame's expression text and lasttp.
    // Prefer it over the expression-text heuristics below — GNU's
    // diagnostic never re-derives the token from the whole string.
    if let Some(record) = take_arith_eval_error() {
        return Some(record.render(command_context));
    }
    // GNU expr.c evalerror skips the expression's leading whitespace at
    // display time (expr.c:1528: `for (t = expression; whitespace (*t); t++)`)
    // while keeping everything from there on verbatim, trailing blanks
    // included. Every message below therefore echoes the leading-trimmed
    // text, never the raw expansion.
    let expression = expression.trim_start();
    let token_space = if trailing_space { " " } else { "" };
    let operand_expected = if command_context {
        "arithmetic syntax error: operand expected"
    } else {
        "syntax error: operand expected"
    };
    let invalid_operator = if command_context {
        "arithmetic syntax error: invalid arithmetic operator"
    } else {
        "syntax error: invalid arithmetic operator"
    };
    if let Some(token) = arithmetic_division_by_zero_token(expression) {
        return Some(format!(
            "{expression}: division by 0 (error token is \"{token}\")"
        ));
    }

    if let Some((token, error)) = invalid_based_literal(expression) {
        // readtok NUL-terminates the number token in place before calling
        // strlong (expr.c:1404-1408), so both the display and the error token
        // appear truncated at the token itself: `$(( 3425#56 ))` reports
        // `3425#56:`, with no trailing blank.
        let display = expression.trim_end();
        return Some(format!("{display}: {error} (error token is \"{token}\")"));
    }

    // GNU Bash rejects a bare assignment target behind && / || even when
    // short-circuit skips it: `$((0 && B=42))` fails with
    // "attempted assignment to non-variable" (error token is "=42").
    if let Some(token) = logical_rhs_assignment_token(expression) {
        return Some(format!(
            "{expression}: attempted assignment to non-variable (error token is \"{token}\")"
        ));
    }

    // GNU expr.c:529: `--x=7` / `++x=7` — pre-increment returns a value,
    // not an lvalue, so the `=` is "attempted assignment to non-variable".
    if let Some(token) = pre_increment_assignment_token(expression) {
        return Some(format!(
            "{expression}: attempted assignment to non-variable (error token is \"{token}\")"
        ));
    }

    // GNU expr.c:529: `x++=7` / `x--=7` — post-increment returns a value,
    // not an lvalue, so the `=` is "attempted assignment to non-variable".
    if let Some(token) = post_increment_assignment_token(expression) {
        return Some(format!(
            "{expression}: attempted assignment to non-variable (error token is \"{token}\")"
        ));
    }

    // GNU expr.c:1465-1471: `--x++` / `++x--` — pre-increment returns a
    // value, not an lvalue, so the post-increment fails with
    // "++: assignment requires lvalue" / "--: assignment requires lvalue".
    if let Some((token, msg)) = pre_post_increment_lvalue_token(expression) {
        return Some(format!("{expression}: {msg} (error token is \"{token}\")"));
    }

    // An empty ternary branch is a parse failure in Bash:
    // `$((4 ? 20 : ))` reports "expression expected" (error token is ": ").
    if let Some(token) = empty_ternary_branch_token(expression) {
        return Some(format!(
            "{expression}: expression expected (error token is \"{token}\")"
        ));
    }

    // GNU expr.c:649-654: a ternary `?` without a matching `:` reports
    // "`:' expected for conditional expression" with the token after the
    // false-branch expression as the error token.
    // e.g. `1 ? 20` -> "`:' expected for conditional expression" (error token is "20 ")
    if let Some(token) = missing_ternary_colon_token(expression) {
        return Some(format!(
            "{expression}: `:' expected for conditional expression (error token is \"{token}\")"
        ));
    }

    // GNU expr.c:647: a ternary `?` with an empty true-branch reports
    // "expression expected" with the token after `?` as the error token.
    // e.g. `4 ? : $A` -> "expression expected" (error token is ": 3 + 5 ")
    // (the false-branch is parsed first, then the error points at the `:`)
    if let Some(token) = empty_ternary_true_branch_token(expression) {
        return Some(format!(
            "{expression}: expression expected (error token is \"{token}\")"
        ));
    }

    // GNU expr.c:484-485: `curtok != 0` after `EXP_LOWEST()` — trailing
    // input after a successful sub-expression parse, e.g. `(( x=9 y=41 ))`
    // reports "arithmetic syntax error in expression" with the unparsed
    // remainder as the error token.
    if let Some((token, kind)) = trailing_input_token(expression) {
        let msg = match kind {
            TrailingInputKind::OperandExpected => operand_expected,
            TrailingInputKind::InvalidOperator => invalid_operator,
            TrailingInputKind::InExpression => {
                if command_context {
                    "arithmetic syntax error in expression"
                } else {
                    "syntax error in expression"
                }
            }
        };
        return Some(format!("{expression}: {msg} (error token is \"{token}\")"));
    }

    // An operator missing its right-hand operand (`j=`, `7++`, `3**`,
    // `j+=`, `7<=`, ...).  GNU expr.c reports these from the recursive
    // descent with the error token taken from lasttp.
    if let Some(message) = trailing_operator_error(expression, trailing_space, command_context) {
        return Some(message);
    }

    // Bash rejects numeric constants as assignment or increment lvalues.
    // The evaluator reports this as a failed expression; preserve the useful
    // diagnostic instead of silently returning status 1.
    let trimmed = expression.trim();
    if trimmed == "++" || trimmed == "--" {
        let operator = if trimmed == "++" { "+" } else { "-" };
        // A raw-captured `((` expression keeps the real trailing blank
        // before `))`; GNU echoes it verbatim and the lasttp remainder
        // supplies the token (e.g. `(( -- ))` -> "-- : ... \"- \"").
        let raw_spaced = expression.ends_with([' ', '\t']);
        let (display_expression, token) = if raw_spaced {
            let token_start = expression
                .rfind(operator)
                .unwrap_or(expression.len().saturating_sub(1));
            (
                expression.to_string(),
                expression[token_start..].to_string(),
            )
        } else {
            (
                format!("{trimmed}{token_space}"),
                format!("{operator}{token_space}"),
            )
        };
        return Some(format!(
            "{display_expression}: {operand_expected} (error token is \"{token}\")"
        ));
    }
    let assignment_lvalue_is_digit = trimmed.split_once('=').is_some_and(|(left, _)| {
        let left = left.trim();
        !left.ends_with(['=', '<', '>', '!']) && left.chars().all(|ch| ch.is_ascii_digit())
    }) || trimmed
        .strip_suffix("++")
        .or_else(|| trimmed.strip_suffix("--"))
        .is_some_and(|value| value.trim().chars().all(|ch| ch.is_ascii_digit()));
    if assignment_lvalue_is_digit {
        // GNU raises these from expassign/expvalue with lasttp pointing at
        // the operator token that has no valid left side (`=` for `7=4`, the
        // second `+` of `7++` because curtok==STR is required for POSTINC).
        // The reported token is the raw remainder from that operator to the
        // end of the expression, trailing blanks included; the display is the
        // leading-trimmed expression verbatim.
        let message = if trimmed.contains('=') {
            "attempted assignment to non-variable"
        } else {
            operand_expected
        };
        let token_start = if trimmed.contains('=') {
            expression.find('=').unwrap_or(0)
        } else {
            // `7++`: the error token starts at the LAST +/- in the run —
            // the operator whose right operand is missing.
            expression.rfind(['+', '-']).unwrap_or(0)
        };
        return Some(format!(
            "{expression}: {message} (error token is \"{}\")",
            &expression[token_start..]
        ));
    }

    if empty_quoted_operand_has_operator(expression) {
        return Some(format!(
            "{expression}: {operand_expected} (error token is \"\"\")"
        ));
    }

    if trimmed.ends_with(['+', '-', '*', '/', '%', '&', '|', '^', '<', '>']) {
        let token = trimmed.chars().last().unwrap_or_default();
        return Some(format!(
            "{expression}: {operand_expected} (error token is \"{token}\")"
        ));
    }

    let bytes = expression.as_bytes();
    for index in 0..bytes.len() {
        // Floating point like `1.5`: digit followed by `.digit`.
        if bytes[index].is_ascii_digit()
            && bytes.get(index + 1) == Some(&b'.')
            && bytes
                .get(index + 2)
                .is_some_and(|byte| byte.is_ascii_digit())
        {
            let mut end = index + 1;
            while bytes
                .get(end)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
            {
                end += 1;
            }
            // GNU lasttp points at `.` (readtok's junk branch); the token
            // is the raw remainder to the end of the expression.
            let token = &expression[index + 1..];
            return Some(format!(
                "{expression}: {invalid_operator} (error token is \"{token}\")"
            ));
        }
    }

    if let Some(index) = expression.find("**") {
        let after = expression[index + 2..].trim_start();
        if let Some(digits) = after
            .strip_prefix('-')
            .map(|rest| {
                rest.chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
            })
            .filter(|digits| !digits.is_empty())
        {
            // GNU lasttp points at the first digit of the exponent; the
            // token is the raw remainder to the end of the expression.
            let digit_start = expression.len() - after.len() + 1;
            let token = &expression[digit_start..];
            return Some(format!(
                "{expression}: exponent less than 0 (error token is \"{token}\")"
            ));
        }
    }

    // Quoted operand like `$(( '1' ))`: Bash treats `'1'` as a variable
    // reference (which does not exist) and reports `operand expected`.
    // Double quotes are fine (`$(( "1" ))` is 1), so only single quotes count.
    if let Some(start) = expression.find('\'') {
        // remainder to the end of the expression, trailing blanks included.
        let token = &expression[start..];
        return Some(format!(
            "{expression}: {operand_expected} (error token is \"{token}\")"
        ));
    }

    if expression.contains('?') && expression.contains(':') && expression.contains('=') {
        let branch = expression
            .split_once(':')
            .map(|(_, branch)| branch)
            .unwrap_or("");
        let equals = branch.find('=').unwrap_or(0);
        let token_start = equals
            .checked_sub(1)
            .filter(|index| "+-*/%&^|<>".contains(branch.as_bytes()[*index] as char))
            .unwrap_or(equals);
        let token = branch[token_start..].trim();
        return Some(format!(
            "{expression}: attempted assignment to non-variable (error token is \"{token}\")"
        ));
    }

    // GNU expr.c: unbalanced ( reports missing ) with the last token
    // as the error token (e.g. 7 + (43 * 6 -> token 6).
    if let Some(token) = missing_paren_token(expression) {
        return Some(format!(
            "{expression}: missing `)' (error token is \"{token}\")"
        ));
    }

    None
}

/// GNU expr.c: a sub-expression with an unbalanced `(` reports "missing `)'"
/// with the last token as the error token (e.g. `7 + (43 * 6` -> token "6").
fn missing_paren_token(expression: &str) -> Option<String> {
    let mut depth: i32 = 0;
    for ch in expression.chars() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
        }
    }
    if depth <= 0 {
        return None;
    }
    // GNU expr.c: lasttp points to the start of the last token read.
    // For an unbalanced (, the parser has consumed the last token and
    // expects ), so the error token is the last token in the expression.
    let trimmed = expression.trim_end();
    if let Some(space_pos) = trimmed.rfind(char::is_whitespace) {
        Some(trimmed[space_pos..].trim().to_string())
    } else {
        Some(trimmed.to_string())
    }
}

fn invalid_octal_literal(expression: &str) -> Option<String> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit()
            || (index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_'))
        {
            index += 1;
            continue;
        }

        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        let token = &expression[start..index];
        if token.len() > 1 && token.starts_with('0') && token.bytes().any(|byte| byte >= b'8') {
            return Some(token.to_string());
        }
    }
    None
}

#[derive(Clone, Copy)]
enum ArithmeticLiteralError {
    InvalidBase,
    InvalidIntegerConstant,
    ValueTooGreatForBase,
    InvalidNumber,
}

impl ArithmeticLiteralError {
    fn message(self) -> &'static str {
        match self {
            Self::InvalidBase => "invalid arithmetic base",
            Self::InvalidIntegerConstant => "invalid integer constant",
            Self::ValueTooGreatForBase => "value too great for base",
            Self::InvalidNumber => "invalid number",
        }
    }
}

fn invalid_based_literal(expression: &str) -> Option<(String, &'static str)> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit()
            || (index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_'))
        {
            index += 1;
            continue;
        }

        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'#') {
            continue;
        }
        index += 1;
        let digits_start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_'))
        {
            index += 1;
        }
        let mut token_end = index;
        while bytes.get(token_end) == Some(&b'#') {
            token_end += 1;
            while bytes
                .get(token_end)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_'))
            {
                token_end += 1;
            }
        }
        let token = &expression[start..token_end];
        let base = expression[start..digits_start - 1].parse::<u32>().ok();
        let digits = &expression[digits_start..index];
        let error = if token_end != index {
            ArithmeticLiteralError::InvalidNumber
        } else if base == Some(0) {
            ArithmeticLiteralError::InvalidNumber
        } else if base.is_none() || !base.is_some_and(|base| (2..=64).contains(&base)) {
            ArithmeticLiteralError::InvalidBase
        } else if digits.is_empty() {
            ArithmeticLiteralError::InvalidIntegerConstant
        } else if !digits.chars().all(|digit| {
            arithmetic_digit_value(digit, base.unwrap()).is_some_and(|value| value < base.unwrap())
        }) {
            ArithmeticLiteralError::ValueTooGreatForBase
        } else {
            continue;
        };
        return Some((token.to_string(), error.message()));
    }
    invalid_octal_literal(expression).map(|token| {
        (
            token,
            ArithmeticLiteralError::ValueTooGreatForBase.message(),
        )
    })
}

pub(super) fn arithmetic_division_by_zero_token(expression: &str) -> Option<String> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        // Handle compound assignment operators `/=` and `%=`: the division
        // by 0 check applies to the operand after the `=`, not the `=` itself.
        // e.g. `b /= 0` -> division by 0 (error token is "0 ")
        if matches!(bytes.get(index), Some(b'/') | Some(b'%'))
            && bytes.get(index + 1) == Some(&b'=')
        {
            index += 2;
            while bytes
                .get(index)
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                index += 1;
            }
            let operand_start = index;
            if matches!(bytes.get(index), Some(b'+' | b'-')) {
                index += 1;
            }
            let start = index;
            while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
                index += 1;
            }
            if start != index
                && expression[start..index]
                    .parse::<i128>()
                    .is_ok_and(|value| value == 0)
            {
                return Some(expression[operand_start..].to_string());
            }
            continue;
        }
        if !matches!(bytes[index], b'/' | b'%') {
            index += 1;
            continue;
        }
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            index += 1;
        }
        // GNU expr.c::expmuldiv points lasttp at the first non-whitespace
        // position after the operator before raising "division by 0", and
        // evalerror prints the raw remainder from lasttp to the end of the
        // expression string (unary sign included, trailing blanks kept):
        // "44 / 0 " -> "0 ", "i < 4/0" -> "0".
        let operand_start = index;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if start != index
            && expression[start..index]
                .parse::<i128>()
                .is_ok_and(|value| value == 0)
        {
            return Some(expression[operand_start..].to_string());
        }
    }
    None
}

fn eval_mutable_arith_value(value: &str, env_vars: &mut HashMap<String, String>) -> Option<i128> {
    eval_mutable_arith_value_with_random(value, env_vars, None).0
}

pub(super) fn eval_mutable_arith_value_with_random(
    value: &str,
    env_vars: &mut HashMap<String, String>,
    random_state: Option<&RandomGen>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    eval_mutable_arith_value_with_random_flags(value, env_vars, random_state, false)
}

pub(super) fn eval_mutable_arith_value_with_random_flags(
    value: &str,
    env_vars: &mut HashMap<String, String>,
    random_state: Option<&RandomGen>,
    no_expand: bool,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    // GNU Bash's subexpr() treats an empty arithmetic expression as zero.
    // This matters for expansion and variable contexts, where an empty
    // quoted operand is valid rather than a parser failure. Lexer quote
    // markers must be normalized before lvalue parsing as well as expansion.
    let normalized = normalize_arithmetic_quotes(value);
    if normalized.trim().is_empty() {
        return (Some(0), None);
    }
    eval_mutable_arith_result(value, env_vars, random_state, no_expand)
}

fn eval_mutable_arith_result(
    value: &str,
    env_vars: &mut HashMap<String, String>,
    random_state: Option<&RandomGen>,
    no_expand: bool,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    // Fresh evaluation: a stale record/diagnostic from an earlier
    // expression must not label this one (early-return paths below never
    // run the parser).
    let _ = take_arith_eval_error();
    let _ = take_arith_eval_diags();
    let normalized = normalize_arithmetic_quotes(value);
    if normalized.trim().is_empty() {
        return (Some(0), None);
    }
    let mut parser = ConditionalArithParser {
        input: normalized.as_bytes(),
        pos: 0,
        env_vars,
        resolving: Vec::new(),
        random_state,
        error_category: None,
        no_expand,
        error: None,
        diags: Vec::new(),
        last_tok_start: 0,
        last_tok_operand: false,
    };
    if std::env::var("RUBASH_DEBUG_ARITH").is_ok() {
        eprintln!("ARITH-INPUT: {normalized:?}");
    }
    let value = parser.parse_comma();
    parser.skip_ws();
    let trailing = parser.pos != parser.input.len();
    let value = value.filter(|_| !trailing);
    if parser.error.is_none() {
        if trailing {
            // GNU expr.c:484-485: `curtok != 0` after EXP_LOWEST — the
            // diagnostic splits on the trailing token kind.
            parser.record_trailing();
        } else if value.is_none() && numeric_assignment_expression(&normalized) {
            // `7 = 43`: the `=` trails a NUM operand — expassign
            // (expr.c:528-529) reports "attempted assignment to
            // non-variable" with lasttp at the `=`.
            let tok = normalized.find('=').unwrap_or(0);
            parser.error = Some(ArithEvalError {
                expr: normalized.clone(),
                msg: "attempted assignment to non-variable".to_string(),
                tok_start: tok,
                display_end: normalized.len(),
            });
        } else if value.is_none() {
            // The parser returned no value without recording a cause —
            // GNU exp0's operand-expected (expr.c:1120) with lasttp at the
            // last consumed token is the conservative record.
            let _ = parser.fail_operand_expected();
        }
    }
    let category = parser.error_category.or_else(|| {
        if value.is_none() && numeric_assignment_expression(&normalized) {
            Some(ArithmeticErrorCategory::NonVariableAssignment)
        } else if trailing {
            Some(ArithmeticErrorCategory::TrailingInput)
        } else if value.is_none() {
            Some(ArithmeticErrorCategory::EvaluatorFailure)
        } else {
            None
        }
    });
    ARITH_EVAL_ERROR.with(|slot| *slot.borrow_mut() = parser.error);
    ARITH_EVAL_DIAGS.with(|slot| *slot.borrow_mut() = parser.diags);
    (value, category)
}

fn numeric_assignment_expression(expression: &str) -> bool {
    let Some((left, _)) = expression.split_once('=') else {
        return false;
    };
    !left.trim().is_empty() && left.trim().chars().all(|ch| ch.is_ascii_digit())
}

fn bash_arith(value: i128) -> i128 {
    value as i64 as i128
}

fn checked_arithmetic_pow(base: i128, exponent: i128) -> Option<i128> {
    let exponent = u32::try_from(exponent).ok()?;
    let mut value = 1i128;
    for _ in 0..exponent {
        value = bash_arith(value * base);
    }
    Some(value)
}

fn parse_arithmetic_digits(digits: &[u8], base: u32) -> Option<i128> {
    let mut value = 0i128;
    for digit in std::str::from_utf8(digits).ok()?.chars() {
        let digit = arithmetic_digit_value(digit, base)?;
        if digit >= base {
            return None;
        }
        value = bash_arith(value * i128::from(base) + i128::from(digit));
    }
    Some(value)
}

fn arithmetic_digit_value(ch: char, base: u32) -> Option<u32> {
    match ch {
        '0'..='9' => Some(ch as u32 - '0' as u32),
        'a'..='z' => Some(10 + ch as u32 - 'a' as u32),
        'A'..='Z' if base <= 36 => Some(10 + ch as u32 - 'A' as u32),
        'A'..='Z' => Some(36 + ch as u32 - 'A' as u32),
        '@' => Some(62),
        '_' => Some(63),
        _ => None,
    }
}

fn skip_arith_ws(input: &[u8], pos: &mut usize) {
    while input.get(*pos).is_some_and(|ch| ch.is_ascii_whitespace()) {
        *pos += 1;
    }
}

fn assignment_operator_at(input: &[u8], pos: usize) -> Option<&'static str> {
    for op in [
        "<<=", ">>=", "**=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
    ] {
        if op == "="
            && (input.get(pos + 1) == Some(&b'=')
                || (pos > 0 && matches!(input.get(pos - 1), Some(b'!') | Some(b'<') | Some(b'>'))))
        {
            continue;
        }
        if input
            .get(pos..)
            .is_some_and(|rest| rest.starts_with(op.as_bytes()))
        {
            return Some(op);
        }
    }
    None
}

#[cfg(test)]
mod fatality_tests {
    use super::arithmetic_expansion_is_fatal;

    #[test]
    fn invalid_literals_are_fatal_arithmetic_expansion_errors() {
        assert!(arithmetic_expansion_is_fatal("08"));
        assert!(arithmetic_expansion_is_fatal("2#2"));
    }

    /// Word-expansion probe evidence (GNU Bash 5.2.37, 2026-08-24):
    /// `$((1/0)); echo after` never reaches `after`, status 1. Only
    /// command-context evaluation (`let`, `(( ))`) keeps running.
    #[test]
    fn division_by_zero_in_word_expansion_is_fatal() {
        assert!(arithmetic_expansion_is_fatal("1/0"));
    }
}

/// Detects `&& B=...` / `|| B=...` shapes whose right-hand side starts with
/// an identifier assignment. Returns the operator-prefixed error token
/// (for example `"=42"` for `0 && B=42`), mirroring GNU expr.c diagnostics.
fn logical_rhs_assignment_token(expression: &str) -> Option<String> {
    for op in ["&&", "||"] {
        let mut from = 0;
        while let Some(at) = expression[from..].find(op) {
            let rest = expression[from + at + op.len()..].trim_start();
            let first = match rest.chars().next() {
                Some(ch) => ch,
                None => return None,
            };
            if !(first.is_ascii_alphabetic() || first == '_') {
                from += at + op.len();
                continue;
            }
            let mut len = first.len_utf8();
            while rest[len..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                len += rest[len..].chars().next().unwrap().len_utf8();
            }
            let after_trimmed = rest[len..].trim_start();
            if !after_trimmed.starts_with('=') {
                return None;
            }
            // `B==42` is an equality test, not an assignment.
            if after_trimmed[1..].starts_with('=') {
                return None;
            }
            // GNU lasttp points at the `=` operator; the token is the raw
            // remainder to the end of the expression, trailing blanks
            // included (`0 && B=42 ` -> "=42 ").
            let from_abs = from + at + op.len();
            let rest_abs = from_abs + (expression[from_abs..].len() - rest.len());
            let skipped_ws = rest[len..].len() - after_trimmed.len();
            let eq_abs = rest_abs + len + skipped_ws;
            return Some(expression[eq_abs..].to_string());
        }
    }
    None
}

/// GNU expr.c:529: `x++=7` / `x--=7` — post-increment returns a value,
/// not an lvalue, so the following `=` is "attempted assignment to non-variable".
/// The error token is the `=` and everything after.
/// e.g. `x++=7` -> error token is "=7 "
fn post_increment_assignment_token(expression: &str) -> Option<String> {
    let trimmed = expression.trim_start();
    let first = match trimmed.chars().next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => ch,
        _ => return None,
    };
    let mut len = first.len_utf8();
    while trimmed[len..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        len += trimmed[len..].chars().next().unwrap().len_utf8();
    }
    for post_op in ["++", "--"] {
        if !trimmed[len..].starts_with(post_op) {
            continue;
        }
        let after_op = trimmed[len + post_op.len()..].trim_start();
        if !after_op.starts_with('=') {
            continue;
        }
        // `x===7` is not a valid assignment.
        if after_op[1..].starts_with('=') {
            continue;
        }
        let var_end = len;
        let ws_len = trimmed[len + post_op.len()..].len() - after_op.len();
        let eq_abs = var_end + post_op.len() + ws_len;
        let prefix_len = expression.len() - trimmed.len();
        return Some(expression[prefix_len + eq_abs..].to_string());
    }
    None
}

/// GNU expr.c:1465-1471: `--x++` / `++x--` — pre-increment returns a value,
/// not an lvalue, so the following post-increment/decrement fails with
/// "++: assignment requires lvalue" or "--: assignment requires lvalue".
/// The error token is the post-op and everything after.
/// e.g. `--x++` -> error token is "++ "
fn pre_post_increment_lvalue_token(expression: &str) -> Option<(String, &'static str)> {
    let trimmed = expression.trim_start();
    for pre_op in ["--", "++"] {
        if !trimmed.starts_with(pre_op) {
            continue;
        }
        let rest = trimmed[pre_op.len()..].trim_start();
        let first = rest.chars().next()?;
        if !(first.is_ascii_alphabetic() || first == '_') {
            continue;
        }
        let mut len = first.len_utf8();
        while rest[len..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            len += rest[len..].chars().next().unwrap().len_utf8();
        }
        let after_var = rest[len..].trim_start();
        for post_op in ["++", "--"] {
            if after_var.starts_with(post_op) {
                let var_end = pre_op.len() + (trimmed[pre_op.len()..].len() - rest.len()) + len;
                let ws_len = rest[len..].len() - after_var.len();
                let post_abs = var_end + ws_len;
                let msg = if post_op == "--" {
                    "--: assignment requires lvalue"
                } else {
                    "++: assignment requires lvalue"
                };
                return Some((expression[post_abs..].to_string(), msg));
            }
        }
    }
    None
}

/// GNU expr.c:529: `--x=7` / `++x=7` — pre-increment/decrement returns a
/// value, not an lvalue, so the following `=` is "attempted assignment to
/// non-variable". The error token is the `=` and everything after.
/// e.g. `--x=7` -> error token is "=7 "
fn pre_increment_assignment_token(expression: &str) -> Option<String> {
    let trimmed = expression.trim_start();
    for op in ["--", "++"] {
        if !trimmed.starts_with(op) {
            continue;
        }
        let rest = trimmed[op.len()..].trim_start();
        let first = rest.chars().next()?;
        if !(first.is_ascii_alphabetic() || first == '_') {
            continue;
        }
        let mut len = first.len_utf8();
        while rest[len..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            len += rest[len..].chars().next().unwrap().len_utf8();
        }
        let after_var = rest[len..].trim_start();
        if !after_var.starts_with('=') {
            continue;
        }
        // `x==7` is equality, not assignment.
        if after_var[1..].starts_with('=') {
            continue;
        }
        let var_end = op.len() + (trimmed[op.len()..].len() - rest.len()) + len;
        let ws_len = rest[len..].len() - after_var.len();
        let eq_abs = var_end + ws_len;
        return Some(expression[eq_abs..].to_string());
    }
    None
}

/// Detects `$((cond ? true :))` shapes where the false branch holds only
/// whitespace; returns the `": "` error token GNU prints.
fn empty_ternary_branch_token(expression: &str) -> Option<String> {
    let question = expression.find('?')?;
    let colon = expression[question..].find(':')? + question;
    if expression[colon + 1..].trim().is_empty() {
        // GNU lasttp points at the `:`; the token is the raw remainder to
        // the end of the expression, trailing blanks included.
        Some(expression[colon..].to_string())
    } else {
        None
    }
}

/// GNU expr.c:649-654: a ternary `?` without a matching `:` reports
/// "`:' expected for conditional expression". The error token is the
/// false-branch expression (everything after `?`).
/// e.g. `1 ? 20` -> error token is "20 "
fn missing_ternary_colon_token(expression: &str) -> Option<String> {
    let question = expression.find('?')?;
    // No `:` found after `?` -- the entire false-branch is the error token.
    if expression[question + 1..].find(':').is_none() {
        let false_branch = expression[question + 1..].trim_start();
        if !false_branch.is_empty() {
            let mut token_start = question + 1;
            while expression.as_bytes().get(token_start) == Some(&b' ') {
                token_start += 1;
            }
            return Some(expression[token_start..].to_string());
        }
    }
    None
}

/// GNU expr.c:647: a ternary `?` with an empty true-branch reports
/// "expression expected". The error token is the `:` and everything after.
/// e.g. `4 ? : $A` -> error token is ": 3 + 5 "
fn empty_ternary_true_branch_token(expression: &str) -> Option<String> {
    let question = expression.find('?')?;
    let after_q = expression[question + 1..].trim_start();
    if after_q.starts_with(':') {
        let colon_pos = expression[question + 1..].find(':')? + question + 1;
        return Some(expression[colon_pos..].to_string());
    }
    None
}

/// An operator at the end of the expression whose right-hand operand is
/// missing (`j=`, `7+=`, `7++`, `3**`, `7<=`, `j==`, `7&&`, `7,` ...).
///
/// GNU expr.c reaches `exp0` with no token left and reports
/// `arithmetic syntax error: operand expected`; `evalerror` prints the
/// suffix of the expression from the start of that operator token
/// (lasttp).  Assignment operators (`=`, `op=`) are reported by
/// `expassign` instead: a non-variable left-hand side
/// (`if (lasttok != STR)`) is `attempted assignment to non-variable`,
/// while a variable left-hand side passes the lvalue check and then fails
/// with `operand expected` once the missing operand is read.
pub(in crate::executor) fn trailing_operator_error(
    expression: &str,
    _trailing_space: bool,
    command_context: bool,
) -> Option<String> {
    // Bare `++` / `--` keep their dedicated diagnostic.
    let trimmed = expression.trim();
    if trimmed == "++" || trimmed == "--" {
        return None;
    }

    #[derive(Clone, Copy, PartialEq)]
    enum ArithTokenKind {
        None,
        Num,
        Str,
        IncDec,
        Op,
        Inert,
    }

    let bytes = expression.as_bytes();
    let mut index = 0usize;
    let mut prev_kind = ArithTokenKind::None;
    let mut last_kind = ArithTokenKind::None;
    let mut last_start = 0usize;
    let mut last_end = 0usize;
    let mut last_op = String::new();

    while index < bytes.len() {
        let c = bytes[index];
        if c.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        let kind;
        let mut op = String::new();
        if c.is_ascii_digit() {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric()
                    || matches!(bytes[index], b'#' | b'@' | b'_'))
            {
                index += 1;
            }
            kind = ArithTokenKind::Num;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'[' {
                // Array subscripts defer to the evaluator diagnostics.
                return None;
            }
            kind = ArithTokenKind::Str;
        } else if matches!(c, b'\'' | b'"' | b'$' | b'.' | b'`') {
            // Quoted, expanded, or float text has its own diagnostics.
            return None;
        } else {
            let rest = &expression[index..];
            let three = rest.get(..3).unwrap_or("");
            let two = rest.get(..2).unwrap_or("");
            if matches!(three, "<<=" | ">>=") {
                op = three.to_string();
                kind = ArithTokenKind::Op;
                index += 3;
            } else if matches!(
                two,
                "**" | "=="
                    | "!="
                    | "<="
                    | ">="
                    | "<<"
                    | ">>"
                    | "&&"
                    | "||"
                    | "++"
                    | "--"
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "&="
                    | "^="
                    | "|="
            ) {
                if two == "++" || two == "--" {
                    // expr.c readtok: `id++` / `id--` (post) only follows a
                    // variable token; after a number the pair splits into
                    // two single operators (expr.c ungets the second `+`);
                    // otherwise pre-increment only when an identifier
                    // follows.
                    if prev_kind == ArithTokenKind::Str {
                        kind = ArithTokenKind::IncDec;
                        index += 2;
                    } else if prev_kind == ArithTokenKind::Num {
                        op = if two == "++" { "+" } else { "-" }.to_string();
                        kind = ArithTokenKind::Op;
                        index += 1;
                    } else {
                        let after = rest[2..].trim_start();
                        if after
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                        {
                            kind = ArithTokenKind::IncDec;
                            index += 2;
                        } else {
                            op = if two == "++" { "+" } else { "-" }.to_string();
                            kind = ArithTokenKind::Op;
                            index += 1;
                        }
                    }
                } else {
                    op = two.to_string();
                    kind = ArithTokenKind::Op;
                    index += 2;
                }
            } else if matches!(
                c,
                b'=' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b','
            ) {
                op = (c as char).to_string();
                kind = ArithTokenKind::Op;
                index += 1;
            } else {
                // `!`, `~`, parens, `?`, `:`, ... are not right-hand-operand
                // consumers; their diagnostics live elsewhere.
                kind = ArithTokenKind::Inert;
                index += 1;
            }
        }
        prev_kind = last_kind;
        last_kind = kind;
        last_start = start;
        last_end = index;
        last_op = op;
    }

    if last_kind != ArithTokenKind::Op || last_op.is_empty() {
        return None;
    }
    if !expression[last_end..].trim().is_empty() {
        return None;
    }
    let assignment = matches!(
        last_op.as_str(),
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "^=" | "|=" | "<<=" | ">>="
    );
    let message = if assignment && prev_kind != ArithTokenKind::Str {
        "attempted assignment to non-variable"
    } else if command_context {
        "arithmetic syntax error: operand expected"
    } else {
        "syntax error: operand expected"
    };
    let token = &expression[last_start..];
    // The token is the raw remainder from lasttp to the end of the
    // expression: the operator plus whatever whitespace follows it. GNU
    // never appends a synthetic separator (expr.c evalerror echoes lasttp
    // verbatim), so the captured trailing blank is the only one.
    Some(format!(
        "{expression}: {message} (error token is \"{token}\")"
    ))
}
