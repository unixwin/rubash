//! Arithmetic expression parsing and evaluation.
//!
//! Provides parsing and evaluation of shell arithmetic expressions including
//! variables, arrays, assignments, and ternary conditionals.

mod parser;

use parser::ConditionalArithParser;
pub(crate) use parser::{ArithEvalDiag, ArithEvalError};

use super::Executor;
use crate::executor::execution_misc::RandomGen;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::executor::{
    is_marked_var, is_shell_name, sync_shell_assignment_process_env, SubstitutionQuoteContext,
    ARRAY_VARS, ASSOC_128_VARS, ASSOC_VARS, NAMEREF_VARS, READONLY_VARS, UNSET_DYNAMIC_VARS,
};
use std::collections::HashMap;

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

    /// First-knife arithmetic parse cache (rubash#156 follow-up): one
    /// thread-local entry holding at most one CmpLT (`i<N`), one PostInc
    /// (`i++`) and one AddAssign (`x+=i` / `x+=1`) specialization side by
    /// side (a single static, bounded — never a growing map). GNU expr.c
    /// has no such cache, so this is pure acceleration: any semantic doubt
    /// falls back to the slow parser and the entry is kept (never evicted
    /// on a hit-fallback).
    static ARITH_PARSE_CACHE: std::cell::RefCell<ArithParseCache> =
        const { std::cell::RefCell::new(ArithParseCache::EMPTY) };

    /// rubash#1202: when armed, `report_indexed_subscript_error` collects
    /// its rendered diagnostic lines here instead of writing them to the
    /// raw process stderr, so a builtin whose output is redirectable
    /// (`test -v name[sub] 2>file`) can route them through the buffered
    /// output path like every other builtin diagnostic. Disarmed (None)
    /// is the historical raw-stderr behavior for expansion-time errors,
    /// which GNU also leaves unredirected.
    static OPERAND_SUBSCRIPT_DIAG_CAPTURE: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Arm the operand-subscript diagnostic capture (see the thread-local).
pub(in crate::executor) fn arm_operand_subscript_diag_capture() {
    OPERAND_SUBSCRIPT_DIAG_CAPTURE.with(|slot| *slot.borrow_mut() = Some(Vec::new()));
}

/// Disarm the capture and return the collected diagnostic lines.
pub(in crate::executor) fn take_operand_subscript_diag_capture() -> Vec<String> {
    OPERAND_SUBSCRIPT_DIAG_CAPTURE.with(|slot| slot.borrow_mut().take().unwrap_or_default())
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

/// Executor-visible wrapper for evaluation boundaries outside this module.
/// GNU expr.c binds assignments through expr_bind_variable -> bind_variable
/// into the ONE variable store, so every caller of the evaluator — `(( ))`,
/// `let`, array subscripts AND `[[ -eq ... ]]` (test.c:357 arithcomp ->
/// evalexp) — sees the write in the next parameter expansion. Rubash's
/// evaluator writes env_vars while parameter lookup prefers the typed
/// `shell_state.variables` map, so each boundary must sync; the `[[ ]]`
/// boundary missed it (rubash#404: `n=1; x='n=0'; [[ x -eq 0 ]]; echo $n`
/// left the stale n).
pub(in crate::executor) fn sync_arith_writes(executor: &mut Executor) {
    sync_arith_writes_to_shell_state(executor)
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
        // rubash#616 (plan Phase 1a): an arithmetic element write stores the
        // rendered array storage into the mirror (value.rs set_array_element
        // -> format_indexed_array_storage); the structured cell must follow
        // with the real entries. The previous blind Scalar downgrade left a
        // Scalar(storage) cell, so a later element-level typed write (printf
        // -v name[i]) extended an EMPTY structured base and shadowed the
        // mirror count — `${#name[@]}` then read the typed cell (the fast
        // path) instead of the mirror. `sync_typed_indexed_storage` syncs an
        // indexed render and drops a stale structured cell for other shapes;
        // the scalar branches keep the pre-#616 behavior.
        if new_value.starts_with(crate::executor::markers::STORAGE_WORD_PREFIX) {
            crate::executor::arrays::sync_typed_indexed_storage(
                &mut executor.shell_state.variables,
                &name,
                &new_value,
            );
        } else if let Some(variable) = executor.shell_state.variables.get_mut(&name) {
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

/// A subscript that is nothing but ASCII decimal digits with a non-zero
/// lead (or a lone `0`): arithmetic-evaluates to itself, so the whole
/// pre-evaluation pipeline is skippable (GNU arrayfunc.c:1368 evalexp on
/// `a[N]`).
pub(in crate::executor) fn literal_decimal_subscript(resolved: &str) -> Option<i128> {
    let text = resolved.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None; // 0NNN is octal in shell arithmetic — full evaluator
    }
    text.parse::<i128>().ok()
}

/// Pure-variable indexed-subscript fast path: the resolved subscript is
/// exactly one shell name whose current value is decimal-literal text (an
/// empty or unset name reads as 0) — GNU arrayfunc.c:1353-1391
/// array_expand_index -> evalexp evaluates the bare name with no nested
/// expansion, so the whole pre-evaluation pipeline (re-expansion, the
/// assoc-subscript scan, quote normalization, the nounset scan, the dynamic
/// context snapshot, the full parse) is skippable. The loaded value reuses
/// the existing decimal lane above, so octal-looking (`010`), signed
/// (`-5`), expression or recursive values stay on the full pipeline with
/// byte-identical GNU semantics. Whitelist admission, never a symptom
/// guard — any doubt returns None and the caller runs the slow path:
/// - non-name text (operators, `$`/quotes/brackets) is not a bare read;
/// - `set -u` stays slow so the unbound diagnostic keeps its slow path;
/// - dynamic spellings (RANDOM/SECONDS/.../BASHPID/..., `__RUBASH_*`)
///   read live state and never constant-fold;
/// - array/assoc/nameref marks bind through their own helpers, not the
///   scalar read.
pub(in crate::executor) fn plain_variable_subscript_index(
    resolved: &str,
    env_vars: &crate::shell::var_table::VarTable,
) -> Option<i128> {
    if !is_shell_name(resolved) || arith_parse_cache_is_dynamic(resolved) {
        return None;
    }
    if is_marked_var(env_vars, ARRAY_VARS, resolved)
        || is_marked_var(env_vars, ASSOC_VARS, resolved)
        || is_marked_var(env_vars, ASSOC_128_VARS, resolved)
        || is_marked_var(env_vars, NAMEREF_VARS, resolved)
    {
        return None;
    }
    if crate::builtins::set::shell_option_enabled(env_vars, "nounset") {
        return None;
    }
    let value = env_vars
        .get(resolved)
        .cloned()
        .or_else(|| std::env::var(resolved).ok())
        .unwrap_or_default();
    // GNU expr.c:271 subexpr on the variable's value: empty reads as 0
    // (variable_value -> evaluate_variable_text's trim-empty arm).
    if value.trim().is_empty() {
        return Some(0);
    }
    // The existing decimal lane admits only non-zero-led digit text, so
    // every other shape (octal, signs, nested expressions, self-recursion)
    // keeps the slow evaluator's exact semantics.
    literal_decimal_subscript(&value)
}

#[cfg(test)]
mod variable_subscript_tests {
    use super::plain_variable_subscript_index;
    use crate::shell::var_table::VarTable;

    fn table_with(pairs: &[(&str, &str)]) -> VarTable {
        let mut table = VarTable::default();
        for (name, value) in pairs {
            table.insert((*name).to_string(), (*value).to_string());
        }
        table
    }

    #[test]
    fn plain_decimal_values_read_directly() {
        let table = table_with(&[
            ("i", "42"),
            ("zero", "0"),
            ("empty", ""),
            ("blank", "   "),
            ("ws", "  7  "),
        ]);
        assert_eq!(plain_variable_subscript_index("i", &table), Some(42));
        assert_eq!(plain_variable_subscript_index("zero", &table), Some(0));
        // GNU expr.c:271 subexpr on the value: empty reads as 0.
        assert_eq!(plain_variable_subscript_index("empty", &table), Some(0));
        assert_eq!(plain_variable_subscript_index("blank", &table), Some(0));
        assert_eq!(plain_variable_subscript_index("ws", &table), Some(7));
        // Unset names read as 0 (nounset off).
        assert_eq!(
            plain_variable_subscript_index("never_set_xyz", &table),
            Some(0)
        );
    }

    #[test]
    fn doubt_falls_through_to_slow() {
        let table = table_with(&[
            ("oct", "010"),
            ("neg", "-1"),
            ("plus", "+2"),
            ("expr", "1+1"),
            ("name", "j"),
            ("hex", "0x10"),
        ]);
        // Non-decimal values keep the full evaluator (octal, signs,
        // nested expressions, recursion).
        for subscript in ["oct", "neg", "plus", "expr", "name", "hex"] {
            assert_eq!(
                plain_variable_subscript_index(subscript, &table),
                None,
                "{subscript} must stay slow"
            );
        }
        // Dynamic spellings read live state, never constant-fold.
        for subscript in ["RANDOM", "SECONDS", "LINENO", "BASHPID", "__RUBASH_X"] {
            assert_eq!(
                plain_variable_subscript_index(subscript, &table),
                None,
                "{subscript} must stay slow"
            );
        }
        // Non-name text is not a bare read.
        for subscript in ["", "i+1", "$i", "a[0]", "1", "  "] {
            assert_eq!(
                plain_variable_subscript_index(subscript, &table),
                None,
                "{subscript:?} must stay slow"
            );
        }
    }
}

#[cfg(test)]
mod literal_subscript_tests {
    use super::literal_decimal_subscript;

    #[test]
    fn decimal_literals_are_their_own_value() {
        assert_eq!(literal_decimal_subscript("0"), Some(0));
        assert_eq!(literal_decimal_subscript("1"), Some(1));
        assert_eq!(literal_decimal_subscript(" 42 "), Some(42));
        assert_eq!(
            literal_decimal_subscript("170141183460469231731687303715884105727"),
            Some(i128::MAX)
        );
    }

    #[test]
    fn everything_else_takes_the_full_path() {
        // Octal spellings, signs, names, expansions, quotes, empty.
        assert_eq!(literal_decimal_subscript("010"), None);
        assert_eq!(literal_decimal_subscript("00"), None);
        assert_eq!(literal_decimal_subscript("-1"), None);
        assert_eq!(literal_decimal_subscript("+1"), None);
        assert_eq!(literal_decimal_subscript("i"), None);
        assert_eq!(literal_decimal_subscript("$i"), None);
        assert_eq!(literal_decimal_subscript("1+1"), None);
        assert_eq!(literal_decimal_subscript(""), None);
        assert_eq!(literal_decimal_subscript("  "), None);
        assert_eq!(literal_decimal_subscript("0x10"), None);
    }
}

/// Whether the arithmetic expression still carries a `${` whose brace never
/// closes (quote-aware): GNU's param_expand pass over the arithmetic string
/// answers `bad substitution` for it (subst.c bad_substitution label,
/// rubash#533).
pub(crate) fn expression_has_unterminated_parameter_brace(expression: &str) -> bool {
    let mut rest = expression;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match crate::executor::parameter_ops::matching_parameter_brace(after) {
            Some(end) => rest = &after[end + 1..],
            None => return true,
        }
    }
    false
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
        // rubash#1588: a `${name:=word}` fragment inside arithmetic text
        // binds in GNU (the text is expanded like a double-quoted word
        // before evaluation), but the embedded expansion below runs on
        // &self paths that drop the bind. The word-level pre-scan skips
        // `$((...))` spans as command-substitution frames, so this entry
        // — the one &mut pass arithmetic text goes through — applies
        // the binds first. Re-application on a later evaluation of the
        // same text is inert: the apply arms re-test the operator and
        // find the variable already set.
        if expression.contains("${") {
            self.apply_parameter_assignment_expansions_in_word(expression);
        }
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
            .replace(
                "$#",
                &self.shell_state.positional_window().len().to_string(),
            )
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

    /// State half of the #607 fast-ticket gate: the current bindings must
    /// make every skipped ticket item unobservable. The write log must be
    /// empty on entry (it is drained by the sync below after every
    /// evaluation, so a non-empty log means an unevaluated push is pending
    /// and only the slow ticket's pre-clear drops it correctly). Every
    /// identifier the expression can read must be unset (reads 0 with no
    /// recursion) or hold a plain decimal value (parsed directly with no
    /// nested evaluation) — any other shape (another expression, a name, a
    /// subscript, an expansion) would recurse into the nested evaluator,
    /// where the skipped dynamic context, EXP_EXPANDED marker or write log
    /// are observable. Whitelist admission only: doubt returns false and
    /// the caller runs the full ticket.
    fn fast_arith_ticket_state_ok(&self, expression: &str) -> bool {
        if !ARITH_WRITES.with(|log| log.borrow().is_empty()) {
            return false;
        }
        let bytes = expression.as_bytes();
        let mut index = 0usize;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte != b'_' && !byte.is_ascii_alphabetic() {
                index += 1;
                continue;
            }
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index] == b'_' || bytes[index].is_ascii_alphanumeric())
            {
                index += 1;
            }
            if let Some(value) = self.shell_state.env_vars.get(&expression[start..index]) {
                if !arith_value_is_plain_decimal(value) {
                    return false;
                }
            }
        }
        true
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

        // Fast admission (rubash#156): an expression drawn entirely from
        // the pure arithmetic alphabet contains none of the characters the
        // pre-evaluation pipeline reacts to (no `$`/backquote for the
        // expansion passes, no `'`/`"`/`\` for the quote checks and
        // normalization, no `[` for the assoc/indexed subscript passes), so
        // that pipeline is provably the identity for it and the expression
        // can go straight to the evaluator. This mirrors GNU's
        // expand_arith_string, which leaves an expansion-character-free
        // string untouched. A false admission is impossible (the whitelist
        // is the character class itself, not a symptom blacklist); a
        // non-matching expression falls through to the full pipeline.
        let preexpansion_is_identity = arithmetic_preexpansion_is_identity(expression.as_bytes());
        // #607 residual per-evaluation ticket: even identity-admitted
        // expressions still paid the post-expansion ticket on every
        // `(( ))` evaluation — the `array_expand_once` shopt lookup, the
        // EXP_EXPANDED remove, the ARITH_WRITES clear, the dynamic-context
        // snapshot and the write sync. GNU evaluates a `(( ))` command via
        // execute_arith_for_command (execute_cmd.c:3219), which — unlike
        // eval_arith_for_expr (execute_cmd.c:3163, the `let` path passing
        // EXP_EXPANDED per expr.c:340,368) — never marks its operand
        // already-expanded, so for the `expand` caller every EXP_EXPANDED
        // item below is provably a no-op; expand_arith_string (subst.c:3981)
        // likewise leaves this alphabet untouched. When the shape gate
        // (pure syntax) and the state gate (current bindings) both admit,
        // the ticket items are skipped; any doubt falls through to the
        // full ticket. DEBUG-trap/xtrace/LINENO firing lives in the command
        // loop around this call, not in the ticket, so it still fires;
        // nounset, the EXP_EXPANDED consumers, readonly and subscript
        // diagnostics keep their exact slow-path handling (see each gate).
        let ticket_shape = if expand && preexpansion_is_identity {
            fast_arith_ticket_shape(expression)
        } else {
            None
        };
        let fast_ticket = ticket_shape.is_some() && self.fast_arith_ticket_state_ok(expression);
        // GNU expr.c:1171 expr_streval: `tflag = (array_expand_once &&
        // already_expanded) ? AV_NOEXPAND : 0` — `let`/`[[` operands arrive
        // EXP_EXPANDED, `(( ))` does not, so only the former switches the
        // assoc subscript scan to flag-1 (verbatim key) semantics.
        // Fast ticket: an identity-alphabet expression holds no `[`, so the
        // assoc scan this flag parametrizes can never fire — hard-coding it
        // skips the shopt lookup.
        let assoc_noexpand = if fast_ticket {
            false
        } else {
            !expand
                && crate::builtins::shopt::option_enabled(
                    &self.shell_state.env_vars,
                    "array_expand_once",
                )
        };
        let expression = if preexpansion_is_identity {
            std::borrow::Cow::Borrowed(expression)
        } else {
            let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(
                expression,
                assoc_noexpand,
                // GNU expr.c:340,368: the `let` operand arrives EXP_EXPANDED
                // (already word-expanded), so `$'...'` is data there, while
                // the `(( ))` body reaches expand_subscript_string raw and
                // its ANSI-C arm decodes (issue rubash#828).
                if expand {
                    crate::executor::subscript_expansion::ArithAnsiSpanMode::Decode
                } else {
                    crate::executor::subscript_expansion::ArithAnsiSpanMode::LetOperand
                },
            );
            let expanded = if expand {
                self.expand_arithmetic_expression_mut(&with_assoc_keys)
            } else {
                self.expand_arith_eval_subscripts(&with_assoc_keys)
            };
            std::borrow::Cow::Owned(normalize_arithmetic_quotes(&expanded).into_owned())
        };
        // GNU expr.c: a fatal error inside a NESTED expansion of the
        // expression (`((i<$((1/0))))`, an arith-for header) raises
        // evalerror while expand_arith_string runs — the evaluation is
        // abandoned before the mangled remainder (`i<`) is ever parsed.
        // The nested expansion already printed its diagnostic; parsing
        // on would synthesize a second, bogus one (`((: i<: arithmetic
        // syntax error: operand expected`) from the hollowed text.
        if self.shell_state.arithmetic_fatal_error.get() {
            return None;
        }
        // GNU expr.c's expr_string/lasttp only matter once evalerror
        // records a diagnostic — success paths never consult them, so the
        // diagnostic input slot is written lazily: the unbound-variable
        // early return and the failed-evaluation tail below are the only
        // paths whose callers read arithmetic_last_eval_input.
        // Fast ticket: identifier-free text (digits and operators only)
        // cannot spell an unbound name, so both the option lookup and the
        // scan are provably no-ops and are skipped. Anything spelling an
        // identifier keeps the exact slow-path check.
        let skip_nounset = fast_ticket
            && ticket_shape
                .as_ref()
                .is_some_and(|shape| !shape.has_identifier);
        if !skip_nounset
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset")
        {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.shell_state.env_vars)
            {
                *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
                self.shell_state.arithmetic_nounset_error.set(true);
                if !self.shell_state.arithmetic_expansion_error.replace(true) {
                    eprintln!("{}{}: unbound variable", self.error_prefix(), name);
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
        // Both quote checks are provably false for a whitelisted
        // (identity-preexpansion) expression: the alphabet admits no
        // quote characters at all.
        if !preexpansion_is_identity {
            if has_bare_single_quote(&expression, &self.shell_state.env_vars) {
                *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
                return None;
            }
            if empty_quoted_operand_has_operator(&expression) {
                *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
                return None;
            }
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
        // GNU computes tflag at expr_streval time (post-expansion): on the
        // full pre-evaluation pipeline a command substitution may have
        // flipped the option, so the lookup re-runs there; the
        // identity-admitted path ran nothing between the two points, so
        // the assoc_noexpand value is provably still current. The fast
        // ticket forces assoc_noexpand false with expand set, so
        // exp_expanded is false and the marker insert below is skipped.
        let exp_expanded = if preexpansion_is_identity {
            assoc_noexpand
        } else {
            !expand
                && crate::builtins::shopt::option_enabled(
                    &self.shell_state.env_vars,
                    "array_expand_once",
                )
        };
        if exp_expanded {
            self.shell_state
                .env_vars
                .insert("__RUBASH_ARITH_EXP_EXPANDED".to_string(), "1".to_string());
        }
        // The write log is drained by the sync below after every
        // evaluation; the state gate proved it empty on entry, so the
        // pre-evaluation clear is a no-op on the fast ticket and skipped.
        if !fast_ticket {
            ARITH_WRITES.with(|log| log.borrow_mut().clear());
        }
        // Fast ticket: the state gate proved no operand can observe the
        // dynamic context (no dynamic-context name is spelled, and every
        // identifier reads as unset/0 or a plain decimal with no nested
        // evaluation), so the snapshot — and its per-evaluation clones —
        // are skipped and the parser runs context-free. RANDOM/SRANDOM
        // travel on the RNG state, LINENO on `__RUBASH_CURRENT_LINE` and
        // SECONDS/EPOCH* on the env-derived helper, none of which need it.
        let dynamic_values = if fast_ticket {
            None
        } else {
            Some(self.arith_dynamic_context())
        };
        let (value, category) = eval_mutable_arith_value_with_random_flags(
            &expression,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            dynamic_values.as_ref(),
            true,
            Some(&self.shell_state.variables),
        );
        // Failure path: every consumer of arithmetic_last_eval_input
        // (arithmetic_aliases, embedded_mutations, parameter_core error
        // arms) reads it only after a None result — see the
        // expr_string/lasttp note above.
        if value.is_none() {
            *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
        }
        // Fast ticket: the marker above is never inserted (exp_expanded is
        // false), and its lifecycle is a straight-line insert-to-remove
        // with no early exits between, so no stale entry can be present
        // and the removal is skipped.
        if !fast_ticket {
            self.shell_state
                .env_vars
                .remove("__RUBASH_ARITH_EXP_EXPANDED");
        }
        self.shell_state
            .arithmetic_last_error_category
            .set(category);
        self.report_arithmetic_readonly_error();
        self.report_arithmetic_noassign_error();
        // GNU prints bind/subscript diagnostics (`a[]: bad array
        // subscript`, `` `a[]': not a valid identifier ``) while the
        // evaluation continues — they surface even when the expression
        // succeeds (`(( a[]=24 ))` -> status 0).
        self.flush_arith_diags(Some(label));

        // Sync any variable changes from env_vars to shell_state.variables
        // so that subsequent variable expansions see the updated values.
        // Fast ticket: a read-only shape provably leaves the write log
        // empty (empty on entry per the state gate, no top-level writer per
        // the shape, no nested writer per the plain-decimal bindings), so
        // the take-and-walk is skipped. Write-shaped fast expressions
        // (`i++`, `i=0`) keep the exact slow sync.
        let needs_sync = !fast_ticket || ticket_shape.is_some_and(|shape| shape.write_possible);
        if needs_sync {
            sync_arith_writes_to_shell_state(self);
        }
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
            // Diag payloads are built from parser-input text, and on the
            // EXP_EXPANDED entries (`let`, `[[`) that input carries the
            // subscript carrier encoding (`a[<0x1e><hex>]`) from the
            // indexed-subscript rewrite. GNU echoes the lvalue token as
            // written (expr.c:1526-1535), so decode the carrier spans at
            // this single rendering point — every producer (plain and
            // compound binds, read and bind diagnostics) is covered
            // (rubash#2341's `let` bind line leaked the raw encoding).
            match (&diag, label) {
                (ArithEvalDiag::InvalidIdentifier(display), Some(label)) => eprintln!(
                    "{}{}: `{}': not a valid identifier",
                    self.diagnostic_prefix(),
                    label,
                    parser::de_carrier_arith_text(display)
                ),
                (ArithEvalDiag::InvalidIdentifier(display), None) => eprintln!(
                    "{}`{}': not a valid identifier",
                    self.diagnostic_prefix(),
                    parser::de_carrier_arith_text(display)
                ),
                (ArithEvalDiag::BadSubscript(display), _) => eprintln!(
                    "{}{}: bad array subscript",
                    self.error_prefix(),
                    parser::de_carrier_arith_text(display)
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
        // Pure decimal literal fast path: GNU arrayfunc.c:1368 runs
        // evalexp on the resolved subscript; for text made only of ASCII
        // decimal digits with a non-zero lead every pre-evaluation pass
        // (re-expansion, assoc-subscript scan, quote normalization, the
        // nounset name scan) is provably the identity — the value IS the
        // literal. Leading-zero subscripts stay on the full path (shell
        // arithmetic reads 0NN as octal). Whitelist admission, not a
        // symptom guard: a non-matching subscript falls through to the
        // real pipeline unchanged.
        if let Some(index) = literal_decimal_subscript(resolved) {
            return Some(index);
        }
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
        // Pure-variable fast path (`a[i]` with a plain-decimal `i`): one
        // scalar load replaces the re-expansion, the assoc-subscript scan,
        // the dynamic snapshot and the full parse. The helper reuses the
        // decimal lane on the loaded value; doubt falls through unchanged.
        // The hit is memoized like the slow path's result so the layered
        // passes evaluate the subscript once per word.
        if let Some(index) = plain_variable_subscript_index(resolved, &self.shell_state.env_vars) {
            if let Some(key) = memo_key {
                crate::executor::expand_braced_indices::sub_idx_store(
                    key,
                    crate::executor::subscript_expansion::IndexedSubscript::Index(index),
                );
            }
            return Some(index);
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
        // expand_arith_string, not this inner evaluation). The nested scan
        // is a reader-side expand_subscript_string pass, so its `$'...'`
        // spans decode (issue rubash#828).
        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(
            &reexpanded,
            false,
            crate::executor::subscript_expansion::ArithAnsiSpanMode::Decode,
        );

        let expression = normalize_arithmetic_quotes(&with_assoc_keys);
        // GNU expr.c:1190-1216 expr_streval: an unset (or invisible)
        // variable reference with unbound_vars_is_error set aborts with
        // err_unboundvar — the same prologue the arithmetic command and
        // expansion evaluators run. This is the write path's only
        // arithmetic-subscript entry (rubash#639); without it
        // `set -u; a[k]=v` silently stored.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.shell_state.env_vars)
            {
                *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
                self.shell_state.arithmetic_nounset_error.set(true);
                if !self.shell_state.arithmetic_expansion_error.replace(true) {
                    eprintln!("{}{}: unbound variable", self.error_prefix(), name);
                    use std::io::Write;
                    let _ = std::io::stderr().flush();
                }
                return None;
            }
        }
        *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
        ARITH_WRITES.with(|log| log.borrow_mut().clear());
        let dynamic_values = self.arith_dynamic_context();
        let (value, category) = eval_mutable_arith_value_with_random_flags(
            &expression,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            Some(&dynamic_values),
            true,
            Some(&self.shell_state.variables),
        );
        self.shell_state
            .arithmetic_last_error_category
            .set(category);
        self.report_arithmetic_readonly_error();
        self.report_arithmetic_noassign_error();
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

    /// rubash#1318: arm the reader-level DISCARD for a standalone
    /// assignment failure (GNU subst.c bash_variable_assignment_error ->
    /// exp_jump_to_top_level(DISCARD): the rest of the current list is
    /// abandoned, the script resumes at the next line). The
    /// Err(ExpansionFailure) the caller returns is absorbed into a plain
    /// status by the brace-group and function-frame boundaries it
    /// crosses, which used to let the enclosing list continue on the
    /// SAME line (`{ z=1; }; echo after` printed `after`); the armed
    /// pending makes every enclosing list loop skip the rest of the
    /// failing logical line through the standard abandon check. The
    /// boundary line is pinned here rather than left to the lazy
    /// reader_command_line capture so the abandon check does not
    /// rewrite the status the failure already chose (1 for a bind
    /// failure, EX_BADUSAGE 2 for the ${:=} readonly class, rubash#402).
    /// The pinned line is the READER's current list line: a failing
    /// assignment inside a function body carries the body's own
    /// re-based line number, but GNU abandons the caller's list — the
    /// line the depth-1 loop is executing (the call site). The node
    /// line is only the fallback for contexts with no reader line yet.
    pub(in crate::executor) fn raise_assignment_discard(&self, line: Option<usize>) {
        // Sourced files and eval strings carry their own top_level catch
        // (evalstring.c): the Err propagation alone already reproduces
        // GNU there (the sourced reader resumes at its next line and the
        // caller keeps the status), and this executor's reader line
        // still points at the CALLER's list, so arming would corrupt
        // both readers' boundaries.
        if self.in_source_context() || self.eval_list_depth.get().is_some() {
            return;
        }
        self.evalerror_pending.set(true);
        let boundary = self.reader_command_line.get().or(line);
        if let Some(line) = boundary.filter(|line| *line != 0) {
            self.evalerror_line.set(Some(line));
        }
    }

    /// rubash#753: absorb a pending failglob-class discard at a containment
    /// boundary (the `eval` and `source` builtins — GNU evalstring.c:365,
    /// 422-442 installs a local top_level catch for the text they run, so
    /// the discard ends only that text and the CALLER's remaining commands
    /// run). Drops the armed abandon and the class marker; the exit code is
    /// left to the caller (the failglob site already set 1). Returns true
    /// when a discard was absorbed, so a caller can tell "contained" from
    /// "nothing pending".
    pub(crate) fn absorb_failglob_discard(&mut self) -> bool {
        if self.evalerror_failglob_pending.replace(false) {
            self.evalerror_pending.set(false);
            self.evalerror_line.set(None);
            true
        } else {
            false
        }
    }

    /// Report a failed subscript evaluation — unless the failure was already
    /// reported as nounset (rubash#639: the unbound-variable message prints
    /// once at the trap site inside `eval_indexed_subscript_expression`, so
    /// every downstream consumer stays silent or the user sees two
    /// diagnostics for one trap).
    pub(in crate::executor) fn report_indexed_subscript_error_unless_nounset(
        &self,
        resolved: &str,
    ) {
        if self.shell_state.arithmetic_nounset_error.get() {
            return;
        }
        self.report_indexed_subscript_error(resolved);
    }

    /// After a subscript evaluation failed under `set -u` (rubash#639):
    /// abandon with the context-owned fatal status (127 at the `-c` top
    /// level, 1 in script mode) — the same rails the read path's nounset
    /// handling uses. Returns true when it fired, so callers skip their own
    /// status/message and fail now. Consumes the flag: a caught abort
    /// (eval/function/subshell boundary) must never poison later commands
    /// with a stale trap.
    pub(in crate::executor) fn raise_nounset_subscript_abort(&mut self) -> bool {
        if self.shell_state.arithmetic_nounset_error.replace(false) {
            self.shell_state.arithmetic_expansion_error.set(false);
            self.exit_code = self.expansion_fatal_status();
            self.evalerror_pending.set(true);
            // Distinct FORCE_EOF class for the abandon point (rubash#639):
            // consumed there, cleared alongside every pending-clear.
            self.nounset_force_eof.set(true);
            true
        } else {
            false
        }
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
        let line = if let Some(record) = take_arith_eval_error() {
            format!("{}{}", self.error_prefix(), record.render())
        } else {
            let token = indexed_noexpand_error_token(resolved);
            format!(
                "{}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                self.error_prefix(),
                resolved,
                token
            )
        };
        // rubash#1202: an armed capture (test -v's subscript rewrite)
        // collects the line for the builtin's buffered, redirectable
        // stderr; otherwise it goes to the raw process stderr as before.
        let captured = OPERAND_SUBSCRIPT_DIAG_CAPTURE.with(|slot| {
            if let Some(lines) = slot.borrow_mut().as_mut() {
                lines.push(line.clone());
                true
            } else {
                false
            }
        });
        if !captured {
            eprintln!("{line}");
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
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

        // GNU expand_string_dollar_arith runs the arithmetic string through
        // expand_arith_string (param_expand) before evalexp: an unterminated
        // `${` there is the bad_substitution verdict — `<string>: bad
        // substitution`, status 1, the expansion discarded
        // (expand_wdesc_error) while the script continues (rubash#533).
        if expression_has_unterminated_parameter_brace(expression) {
            self.shell_state.arithmetic_nonfatal_error.set(true);
            if !self.shell_state.arithmetic_expansion_error.replace(true) {
                // GNU subst.c report_error (error_prolog family): honors
                // gnu_errfmt (rubash#1038).
                eprintln!("{}{expression}: bad substitution", self.error_prefix());
                use std::io::Write;
                let _ = std::io::stderr().flush();
            }
            return None;
        }
        // `$(( ))` is a reader entry: GNU's expand_subscript_string decodes
        // the raw `$'...'` spelling (issue rubash#828).
        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(
            expression,
            false,
            crate::executor::subscript_expansion::ArithAnsiSpanMode::Decode,
        );
        let expanded = self.expand_arithmetic_expression_mut(&with_assoc_keys);
        let expanded = arith_string_quote_removal(&expanded);
        let expression = normalize_arithmetic_quotes(&expanded);
        *self.arithmetic_last_eval_input.borrow_mut() = expression.to_string();
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.shell_state.env_vars)
            {
                self.shell_state.arithmetic_nounset_error.set(true);
                if !self.shell_state.arithmetic_expansion_error.replace(true) {
                    eprintln!("{}{}: unbound variable", self.error_prefix(), name);
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
        let dynamic_values = self.arith_dynamic_context();
        let (value, category) = eval_mutable_arith_value_with_random_flags(
            &expression,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            Some(&dynamic_values),
            true,
            Some(&self.shell_state.variables),
        );
        self.shell_state
            .arithmetic_last_error_category
            .set(category);
        self.report_arithmetic_readonly_error();
        self.report_arithmetic_noassign_error();
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

    /// GNU expr.c:350: an att_noassign bind inside arithmetic fails
    /// the evaluation (the (( ))/let status becomes 1) with NO
    /// diagnostic — bind_variable_internal prints only for readonly.
    /// Drain the marker the bind paths left and latch the same
    /// "bind failure accounted" pair the readonly reporter uses, so
    /// the command reporters fail the command without synthesizing
    /// an evalerror after it (rubash#2369).
    fn report_arithmetic_noassign_error(&mut self) {
        if self
            .shell_state
            .env_vars
            .remove("__RUBASH_ARITH_NOASSIGN_ERROR")
            .is_none()
        {
            return;
        }
        self.shell_state.arithmetic_readonly_error.set(true);
        let _ = self.shell_state.arithmetic_expansion_error.replace(true);
    }

    fn report_arithmetic_readonly_error(&mut self) {
        let Some(name) = self
            .shell_state
            .env_vars
            .remove("__RUBASH_ARITH_READONLY_ERROR")
        else {
            return;
        };
        // rubash#1238: remember the diagnostic was printed so the
        // (( ))/let reporters don't add a synthesized syntax error after
        // it (GNU's bind failure produces exactly one diagnostic line).
        self.shell_state.arithmetic_readonly_error.set(true);
        if !self.shell_state.arithmetic_expansion_error.replace(true) {
            eprintln!("{}{}: readonly variable", self.error_prefix(), name);
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
        // rubash#1518: compound-header arithmetic evalerror (case words,
        // loop headers) is the same pure-DISCARD class — the reader's
        // errexit promotion skips it.
        self.evalerror_arith_pending.set(true);
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
        ansi_mode: crate::executor::subscript_expansion::ArithAnsiSpanMode,
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
                        raw.replace(crate::executor::markers::DATA_DOLLAR, "$")
                            .replace(crate::executor::markers::DATA_BACKTICK, "`")
                            .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
                            .replace(crate::executor::markers::DATA_SQUOTE, "'")
                            .replace(crate::executor::markers::DATA_DQUOTE, "\"")
                    } else {
                        let key = self.expand_assoc_subscript_once(raw, ansi_mode);
                        if key.is_empty() {
                            // rubash#860 empty-span family: the resolution is
                            // empty, and GNU's law branches on the TOKEN text
                            // (the bracket content after the body-level
                            // expansion kept single quotes and stripped double
                            // quotes). Route the parser into the right class
                            // instead of encoding an empty key: an empty
                            // token text takes the `name[]` law (bind:
                            // `` `name[]': not a valid identifier ``; read:
                            // `name[]` x2), a nonempty one (`['']`,
                            // `[$'']`) takes the `*akey == 0` law (bind:
                            // `name['']: bad array subscript`, no store;
                            // read: bare `name` once).
                            if self.arith_assoc_subscript_token_text(raw).is_empty() {
                                output.push_str(name);
                                output.push_str("[]");
                            } else {
                                // The parse-cooked token text (`$''` -> `''`,
                                // `''''''` preserved verbatim — the sq rides
                                // the arith token as syntax) feeds the
                                // parser's empty-resolution law so the
                                // diagnostics print GNU's word.
                                output.push_str(name);
                                output.push('[');
                                output.push_str(&requote_arith_span_token_text(raw));
                                output.push(']');
                            }
                            index = end;
                            continue;
                        }
                        key
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
    /// `ansi_mode` owns the literal `$'...'` spans (issue rubash#828): the
    /// `((...))`-family entries decode them ANSI-C style with backslash
    /// protection, while `let` — whose operand GNU considers already
    /// word-expanded — drops the quotes verbatim without decoding. Bare
    /// (non-`$'`) spellings are NOT re-processed here: the arithmetic
    /// evaluator is handed the `(( ))` body AFTER the lexer's own escape
    /// pass, so its subscript text is not the source spelling; re-running
    /// the assignment path's quote-removal pass here would drop one
    /// backslash too many (`A[a\\b]` read `a\b` instead of `ab`). The
    /// assignment path owns the faithful `expand_subscript_string`; this
    /// path keeps its own one-round word expansion until the `(( ))` body
    /// reaches the evaluator unescaped.
    fn expand_assoc_subscript_once(
        &mut self,
        raw: &str,
        ansi_mode: crate::executor::subscript_expansion::ArithAnsiSpanMode,
    ) -> String {
        let raw_owned;
        let raw = if raw.contains("$'") {
            raw_owned =
                crate::executor::subscript_expansion::transform_arith_ansi_c_spans(raw, ansi_mode);
            raw_owned.as_str()
        } else {
            raw
        };
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
        let expanded = self.expand_word_mut_with_context(&raw, SubstitutionQuoteContext::Unquoted);
        // The expansion owns the final key text, so the data carriers the
        // pre-pass marked (`\x17` for the expanded-once `'` above, the
        // span/dollar carriers of a kept `$'...'` spelling) dequote here —
        // markers.rs:380 decode_word_position_carriers, GNU subst.c:4807
        // dequote_string. Without it a dquoted span kept raw by the
        // rubash#847 dual-state law stored a literal \x17 mid-key
        // (`((G["pre$'x'post"]=1))` keyed `pre$\x17x'post` and the
        // variable-spelling read missed the cell).
        crate::executor::markers::decode_word_position_carriers(&expanded)
    }

    /// The bracket text GNU's arith token carries for this subscript — the
    /// body-level expansion with single quotes PRESERVED as syntax and
    /// double quotes stripped (verified GNU 5.3.0, rubash#860: `((C['']=1))`
    /// tokens `C['']` while `((C3[""]=1))` tokens `C3[]`). `$'...'` spans
    /// are decoded and re-quoted at that phase (`$''` tokens `['']`), and
    /// `$name`/arithmetic expansions run once (`[$u]` with u empty tokens
    /// `[]`). Only the empty/nonempty VERDICT feeds the empty-key class
    /// split, so the protected-sq rendering never reaches output.
    fn arith_assoc_subscript_token_text(&self, raw: &str) -> String {
        let requoted = requote_arith_span_token_text(raw);
        if !requoted.contains('$') && !requoted.contains('"') {
            // No expansion sites: the quotes are the token text verbatim
            // (`''` stays `''` — nonempty — while the walker would strip it).
            return requoted;
        }
        // expand_arithmetic_special_parameters protects `'` as data for the
        // walker (its own doc: single quotes are literal data for expr.c)
        // and the walker strips paired `"` — exactly the body-level law.
        self.expand_arithmetic_special_parameters(&requoted)
    }

    pub(super) fn expand_arithmetic_special_parameters(&self, expression: &str) -> String {
        // In arithmetic contexts, special parameters expand to numeric values:
        // $- -> 0 (shell flags not meaningful in arithmetic), $# -> param count
        // GNU Bash treats $- as 0 in $(( $- )). See array.tests line 60.
        let expression = expression
            .replace(
                "$#",
                &self.shell_state.positional_window().len().to_string(),
            )
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

/// The parse-cooked form of an arith subscript's `$'...'` spans: each closed
/// span is decoded and sh_single_quote'd (`$''` -> `''`), everything else —
/// including the span-less majority — rides verbatim. Mirrors the scan of
/// [`crate::executor::subscript_expansion::transform_arith_ansi_c_spans`]
/// (dual quote state, backslash pairs literal, unterminated spans verbatim);
/// this is the rendering GNU's read_token_word/ansiexpand gives the word at
/// parse time, which is what its diagnostics print (rubash#860 row
/// `D[$'']=1` -> `D['']: bad array subscript`).
pub(in crate::executor) fn requote_arith_span_token_text(text: &str) -> String {
    if !text.contains("$'") {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' && index + 1 < chars.len() {
            output.push(ch);
            output.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            output.push(ch);
            index += 1;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            output.push(ch);
            index += 1;
            continue;
        }
        if !in_single && !in_double && ch == '$' && chars.get(index + 1) == Some(&'\'') {
            let mut inner_end = index + 2;
            let mut inner = String::new();
            let mut closed = false;
            while inner_end < chars.len() {
                let c = chars[inner_end];
                if c == '\\' && inner_end + 1 < chars.len() {
                    inner.push(c);
                    inner.push(chars[inner_end + 1]);
                    inner_end += 2;
                    continue;
                }
                if c == '\'' {
                    closed = true;
                    break;
                }
                inner.push(c);
                inner_end += 1;
            }
            if closed {
                let decoded = crate::lexer::decode_ansi_c_quoted(&inner);
                output.push_str(
                    &crate::executor::subscript_expansion::diagnostic_single_quote(&decoded),
                );
                index = inner_end + 1;
                continue;
            }
        }
        output.push(ch);
        index += 1;
    }
    output
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
    let mut env_vars = crate::shell::var_table::VarTable::default();
    let (_, category) =
        eval_mutable_arith_result(expression, &mut env_vars, None, None, false, None);
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
    let mut env_vars = crate::shell::var_table::VarTable::default();
    let mut parser = ConditionalArithParser {
        input: normalized.as_bytes(),
        pos: 0,
        env_vars: &mut env_vars,
        resolving: Vec::new(),
        random_state: None,
        dynamic_values: None,
        cells: None,
        error_category: None,
        no_expand: false,
        error: None,
        diags: Vec::new(),
        last_tok_start: 0,
        last_tok_operand: false,
        noeval: 0,
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
    let mut env_vars = crate::shell::var_table::VarTable::default();
    let mut parser = ConditionalArithParser {
        input: normalized.as_bytes(),
        pos: 0,
        env_vars: &mut env_vars,
        resolving: Vec::new(),
        random_state: None,
        dynamic_values: None,
        cells: None,
        error_category: None,
        no_expand: true,
        error: None,
        diags: Vec::new(),
        last_tok_start: 0,
        last_tok_operand: false,
        noeval: 0,
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
    env_vars: &crate::shell::var_table::VarTable,
) -> Option<i128> {
    let mut env_vars = env_vars.clone();
    eval_mutable_arith_value(value, &mut env_vars)
}

/// Like eval_conditional_arith_value, but also returns the variables that
/// were modified by side effects (e.g. count++ in ${arr[$((count++))]}).
/// The caller is responsible for applying these writes to the real env_vars.
/// All rubash callers pass the Executor's dynamic state (RANDOM advances
/// the shell RNG, expr.c:1183 expr_streval -> find_variable); pass None for
/// both state arguments when no Executor is reachable.
pub(crate) fn eval_conditional_arith_value_with_writes(
    value: &str,
    env_vars: &crate::shell::var_table::VarTable,
    random_state: Option<&RandomGen>,
    dynamic_values: Option<&super::dynamic_arrays::ArithDynamicContext>,
    cells: Option<&crate::shell::VariableStore>,
) -> (Option<i128>, Vec<(String, String)>) {
    let (result, writes, _category) =
        eval_conditional_arith_value_categorized_with_writes_and_state(
            value,
            env_vars,
            random_state,
            dynamic_values,
            cells,
        );
    (result, writes)
}

/// `eval_conditional_arith_value_with_writes` that also reports the error
/// category from the actual evaluation (with the caller's dynamic state),
/// so a failure site can classify fatality without re-evaluating in a
/// fresh environment (rubash#536: `${a[$((1/0))]}`).
pub(crate) fn eval_conditional_arith_value_categorized_with_writes_and_state(
    value: &str,
    env_vars: &crate::shell::var_table::VarTable,
    random_state: Option<&RandomGen>,
    dynamic_values: Option<&super::dynamic_arrays::ArithDynamicContext>,
    cells: Option<&crate::shell::VariableStore>,
) -> (
    Option<i128>,
    Vec<(String, String)>,
    Option<ArithmeticErrorCategory>,
) {
    let mut cloned = env_vars.clone();
    let (result, category) = eval_mutable_arith_value_with_random(
        value,
        &mut cloned,
        random_state,
        dynamic_values,
        cells,
    );
    let writes = cloned
        .iter()
        .filter(|(name, _)| name.as_str() != "__RUBASH_ARITH_SUBSCRIPT_EXPR")
        .filter(|(name, new_value)| env_vars.get(name.as_str()) != Some(new_value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    (result, writes, category)
}

/// Like `eval_conditional_arith_value`, but also reports the error category
/// from the actual evaluation, so callers can classify fatality without
/// re-evaluating the expression in a fresh environment (GNU expr.c raises
/// evalerror from the real evaluation; state-dependent errors such as
/// `x+=2` with `x` declared only disappear under a fresh environment).
pub(crate) fn eval_conditional_arith_value_categorized(
    value: &str,
    env_vars: &crate::shell::var_table::VarTable,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    let mut env_vars = env_vars.clone();
    eval_mutable_arith_result(value, &mut env_vars, None, None, false, None)
}

/// `eval_conditional_arith_value_categorized` plus the write-capture of
/// `eval_conditional_arith_value_with_writes`: `&self` arithmetic
/// expansions (`$((i++))` inside a `${}` body or array subscript) still
/// have GNU-visible side effects, so the deltas are queued for the
/// mutable caller to apply.
pub(crate) fn eval_conditional_arith_value_categorized_with_writes(
    value: &str,
    env_vars: &crate::shell::var_table::VarTable,
) -> (
    Option<i128>,
    Vec<(String, String)>,
    Option<ArithmeticErrorCategory>,
) {
    let mut cloned = env_vars.clone();
    let (result, category) = eval_mutable_arith_result(value, &mut cloned, None, None, false, None);
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
    env_vars: &crate::shell::var_table::VarTable,
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
            // rubash#1239: GNU treats an associative array with no
            // elements as UNSET for nounset purposes (variables.c: the
            // variable has no value until an element exists), so a
            // subscripted READ (`h[k]`, `h[k]++`, `h[k]+=1`) traps
            // `h: unbound variable`, while a pure `h[k]=value`
            // assignment is a write and stays legal.
            if !arithmetic_subscript_is_pure_assignment(&chars)
                && env_vars
                    .get(&name)
                    .map(|value| {
                        crate::executor::assignment_helpers::assoc_entries(value).is_empty()
                    })
                    .unwrap_or(true)
            {
                return Some(name);
            }
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
            && !(matches!(
                name.as_str(),
                // Dynamic parameters resolved by the evaluator without an
                // env_vars entry (RANDOM/SRANDOM advance the RNG state;
                // BASHPID/BASH_SUBSHELL/... come from the Executor's
                // dynamic_values snapshot). GNU expr_streval sees all of
                // them through find_variable, so under `set -u` they are
                // bound — unless `unset -v` unbound the dynamic name
                // (variables.c:3839 unbind_variable).
                "RANDOM"
                    | "SRANDOM"
                    | "SECONDS"
                    | "EPOCHSECONDS"
                    | "EPOCHREALTIME"
                    | "BASH_MONOSECONDS"
                    | "LINENO"
                    | "BASHPID"
                    | "BASH_SUBSHELL"
                    | "BASH_ARGV0"
                    | "FUNCNAME"
                    | "GROUPS"
                    | "BASH_COMMAND"
                    | "SHELLOPTS"
                    | "BASHOPTS"
                    | "PIPESTATUS"
            ) && !is_marked_var(env_vars, UNSET_DYNAMIC_VARS, &name))
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

/// True when the text following a consumed array subscript is a pure
/// `= value` assignment (blanks allowed). `==` is a comparison, and the
/// op-assignments (`+=`, ...) read the old element, so neither counts —
/// only the plain `=` form is write-only (rubash#1239).
fn arithmetic_subscript_is_pure_assignment(chars: &std::iter::Peekable<std::str::Chars>) -> bool {
    let mut clone = chars.clone();
    while clone.peek().is_some_and(|next| matches!(next, ' ' | '\t')) {
        clone.next();
    }
    clone.next() == Some('=') && clone.peek() != Some(&'=')
}

fn rest_assignment_operator_len(chars: &std::iter::Peekable<std::str::Chars>) -> usize {
    let rest: String = chars.clone().collect();
    for op in [
        "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
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

/// A single quote in `expression` that is not part of an array subscript.
/// GNU rejects `(( '1' ))` — a quoted operand is not a number — but a quote
/// inside ANY `name[...]` bracket span belongs to the subscript's own
/// evalexp frame, never to this level: for an associative array it delimits
/// a string key (`(( A['a b']++ ))`, accepted), and for an indexed array
/// the subscript text is evaluated by array_expand_index in its own frame
/// (arrayfunc.c:1356-1391), whose evalerror names the subscript text
/// (`(( a['k'] ))` -> `'k': arithmetic syntax error: operand expected
/// (error token is \"'k'\")`, rubash#1603). Treating an indexed subscript's
/// quotes as bare used to short-circuit the whole evaluation here, and the
/// caller's text heuristics then framed the diagnostic on the OUTER
/// expression (`((: a['k'] : ... (error token is \"'k'] \")`).
fn has_bare_single_quote(expression: &str, _env_vars: &crate::shell::var_table::VarTable) -> bool {
    let bytes = expression.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return true;
        }
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'[' {
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

/// GNU subst.c:3981 expand_arith_string's quote-removal branch, applied to
/// an arithmetic string (`$(( ))` body via expand_string_dollar_arith,
/// `$[...]` body via param_expand case '[' at subst.c:10912-10926): when
/// the string carries no expansion character (`$`, backquote, CTLESC —
/// ARITH_EXP_CHAR, subst.c:3802) the word-expansion branch is skipped, and
/// a body containing `'`, `\` or `"` instead goes through
/// string_quote_removal (subst.c:12199) with Q_DOUBLE_QUOTES|Q_ARITH.
/// Under those flags a backslash is dropped exactly before the
/// Q_ARITH-special brackets and tilde (`[`, `]`, `~`) and before the
/// CBSDQUOTE set (`$`, backquote, `"`, `\`, newline); before any other
/// character it is kept, an unescaped `"` is dropped, and `'` is literal.
/// This is what makes `$[1\]]` evaluate the string `1]` (error token `]`,
/// not `\`) while `$[1\+1]` keeps `\+` (rubash#2374). Borrowed when the
/// branch does not apply.
pub(in crate::executor) fn arith_string_quote_removal(input: &str) -> std::borrow::Cow<'_, str> {
    if input.contains('$')
        || input.contains('`')
        || input.contains('\u{11}')
        || !input
            .bytes()
            .any(|byte| matches!(byte, b'\'' | b'"' | b'\\'))
    {
        return std::borrow::Cow::Borrowed(input);
    }
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(next) if matches!(next, '[' | ']' | '~' | '$' | '`' | '"' | '\\' | '\n') => {
                    output.push(next);
                }
                Some(next) => {
                    output.push('\\');
                    output.push(next);
                }
                None => output.push('\\'),
            },
            '"' => {}
            _ => output.push(ch),
        }
    }
    std::borrow::Cow::Owned(output)
}

/// Reduce `\"` to `"` (the arith-text quote normalization). Borrowed when no
/// backslash is present — the overwhelmingly common case for arithmetic
/// expressions — so the three normalize sites on the evaluation path no
/// longer allocate per evaluation (rubash#156).
fn normalize_arithmetic_quotes(input: &str) -> std::borrow::Cow<'_, str> {
    if !input.as_bytes().contains(&b'\\') {
        return std::borrow::Cow::Borrowed(input);
    }
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
    std::borrow::Cow::Owned(output)
}

/// Produces a Bash-style error message for an arithmetic expansion that
/// failed to evaluate (`$(( 1.5 ))`, `$(( 2 ** -1 ))`, division by zero, ...).
/// Rubash used to silently drop these; Bash reports them on stderr with rc=1.
///
/// GNU bash 5.3.0 keeps the `arithmetic` prefix on every evalerror message
/// in every context — `$(( ))` word expansion (script file AND `-c`),
/// `(( ))`, `let`, `[[ ]]`. The expr.c messages are unconditional
/// compile-time strings (expr.c:485 `arithmetic syntax error in
/// expression`, 529 `attempted assignment to non-variable`, 1120/1507
/// `arithmetic syntax error: operand expected`, 1509 `arithmetic syntax
/// error: invalid arithmetic operator`); only `this_command_name`
/// (`let:`, `((:`) varies, and the caller prefixes that. 5.2.21 dropped
/// the prefix in the expansion context — rubash's former split (and the
/// `__RUBASH_IS_C` probe) reproduced 5.2, not 5.3. Probes 2026-09-27,
/// WSL `/usr/local/bin/bash` 5.3.0(1): script `echo $((j=))` →
/// `j=: arithmetic syntax error: operand expected (error token is "=")`;
/// matrix `-c "echo $((7<=))"` → `7<=: arithmetic syntax error: operand
/// expected (error token is "<=")`.
pub(in crate::executor) fn arithmetic_error_message(
    expression: &str,
    trailing_space: bool,
    _env_vars: &HashMap<String, String>,
) -> Option<String> {
    arithmetic_error_message_ctx(expression, trailing_space)
}

/// Command-context variant (`(( ))` / `let` / `[[ ]]`): identical message
/// text — the context shows up only in the caller-supplied `let:`/`((:`
/// prefix (verified bash 5.3.0: `(( 7++ ))` → `((: 7++ : arithmetic
/// syntax error: operand expected (error token is "+ ")`).
pub(in crate::executor) fn arithmetic_command_error_message(
    expression: &str,
    trailing_space: bool,
) -> Option<String> {
    arithmetic_error_message_ctx(expression, trailing_space)
}

fn arithmetic_error_message_ctx(expression: &str, trailing_space: bool) -> Option<String> {
    // The real evaluation recorded the GNU evalerror state
    // (expr.c:1524-1535): the failing frame's expression text and lasttp.
    // Prefer it over the expression-text heuristics below — GNU's
    // diagnostic never re-derives the token from the whole string.
    if let Some(record) = take_arith_eval_error() {
        return Some(record.render());
    }
    // GNU expr.c evalerror skips the expression's leading whitespace at
    // display time (expr.c:1528: `for (t = expression; whitespace (*t); t++)`)
    // while keeping everything from there on verbatim, trailing blanks
    // included. Every message below therefore echoes the leading-trimmed
    // text, never the raw expansion.
    let expression = expression.trim_start();
    let token_space = if trailing_space { " " } else { "" };
    // expr.c:1120/1507 and 1509 — context-free message strings (see the
    // arithmetic_error_message doc comment for the 5.2-vs-5.3 history).
    let operand_expected = "arithmetic syntax error: operand expected";
    let invalid_operator = "arithmetic syntax error: invalid arithmetic operator";
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
            TrailingInputKind::InExpression => "arithmetic syntax error in expression",
        };
        return Some(format!("{expression}: {msg} (error token is \"{token}\")"));
    }

    // An operator missing its right-hand operand (`j=`, `7++`, `3**`,
    // `j+=`, `7<=`, ...).  GNU expr.c reports these from the recursive
    // descent with the error token taken from lasttp.
    if let Some(message) = trailing_operator_error(expression, trailing_space) {
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

fn eval_mutable_arith_value(
    value: &str,
    env_vars: &mut crate::shell::var_table::VarTable,
) -> Option<i128> {
    eval_mutable_arith_value_with_random(value, env_vars, None, None, None).0
}

pub(super) fn eval_mutable_arith_value_with_random(
    value: &str,
    env_vars: &mut crate::shell::var_table::VarTable,
    random_state: Option<&RandomGen>,
    dynamic_values: Option<&super::dynamic_arrays::ArithDynamicContext>,
    cells: Option<&crate::shell::VariableStore>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    eval_mutable_arith_value_with_random_flags(
        value,
        env_vars,
        random_state,
        dynamic_values,
        false,
        cells,
    )
}

pub(super) fn eval_mutable_arith_value_with_random_flags(
    value: &str,
    env_vars: &mut crate::shell::var_table::VarTable,
    random_state: Option<&RandomGen>,
    dynamic_values: Option<&super::dynamic_arrays::ArithDynamicContext>,
    no_expand: bool,
    cells: Option<&crate::shell::VariableStore>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    // GNU Bash's subexpr() treats an empty arithmetic expression as zero.
    // This matters for expansion and variable contexts, where an empty
    // quoted operand is valid rather than a parser failure. Lexer quote
    // markers must be normalized before lvalue parsing as well as expansion.
    // The normalization and empty check are not repeated here:
    // eval_mutable_arith_result performs both, so this wrapper only passes
    // the flags through (the former duplicate normalize was one of the
    // ~10 full-string scans per `(( ))` evaluation; rubash#156).
    eval_mutable_arith_result(
        value,
        env_vars,
        random_state,
        dynamic_values,
        no_expand,
        cells,
    )
}

fn eval_mutable_arith_result(
    value: &str,
    env_vars: &mut crate::shell::var_table::VarTable,
    random_state: Option<&RandomGen>,
    dynamic_values: Option<&super::dynamic_arrays::ArithDynamicContext>,
    no_expand: bool,
    cells: Option<&crate::shell::VariableStore>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    // Fresh evaluation: a stale record/diagnostic from an earlier
    // expression must not label this one (early-return paths below never
    // run the parser).
    let _ = take_arith_eval_error();
    let _ = take_arith_eval_diags();
    // Parse-cache fast path (first knife): the key is the raw expression
    // text (no normalization) plus the no_expand flag. A hit executes the
    // specialized closure with live reads; a load that finds a
    // non-plain-decimal (or otherwise doubtful) binding falls back to the
    // slow parser WITHOUT evicting the entry. A miss tries admission for
    // the next evaluation and then runs slow this time.
    if let Some(hit) = arith_parse_cache_lookup(value, no_expand) {
        match hit {
            ArithParseCacheKind::CmpLt { var, bound } => {
                if let Some(result) = arith_parse_cache_exec_cmp_lt(&var, bound, env_vars) {
                    return (Some(result), None);
                }
            }
            ArithParseCacheKind::PostInc { var } => {
                if let Some(result) = arith_parse_cache_exec_post_inc(&var, env_vars) {
                    return (Some(result), None);
                }
            }
            ArithParseCacheKind::AddAssign { dst, src } => {
                if let Some(result) = arith_parse_cache_exec_add_assign(&dst, &src, env_vars) {
                    return (Some(result), None);
                }
            }
        }
    } else if let Some((kind, slot)) = arith_parse_cache_try_admit(value) {
        arith_parse_cache_store(slot, value.to_string(), no_expand, kind);
    }
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
        dynamic_values,
        cells,
        error_category: None,
        no_expand,
        error: None,
        diags: Vec::new(),
        last_tok_start: 0,
        last_tok_operand: false,
        noeval: 0,
    };
    // Debug knob resolved once (RUBASH_EXEC_PROFILE's own ensure_init
    // precedent): GNU's expr.c debugging is compiled out entirely
    // (EXPRDEBUG); a per-evaluation environment-variable scan cost more
    // than the parse itself on Windows (~3 us of the ~6.6 us a
    // `for ((...))` test/update pair spent in this function).
    static DEBUG_ARITH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DEBUG_ARITH.get_or_init(|| std::env::var("RUBASH_DEBUG_ARITH").is_ok()) {
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
                expr: normalized.clone().into_owned(),
                msg: "attempted assignment to non-variable".to_string(),
                tok_start: tok,
                display_end: normalized.len(),
            });
        } else if value.is_none()
            && parser
                .env_vars
                .get("__RUBASH_ARITH_READONLY_ERROR")
                .is_some()
        {
            // rubash#1238: the parse failed on a readonly bind; the
            // Executor layer prints the `readonly variable` diagnostic
            // from the marker. GNU raises no evalerror here, so record
            // none — synthesizing "operand expected" would double-report.
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
        "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
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
    } else {
        // expr.c:1120 — context-free message (see arithmetic_error_message).
        "arithmetic syntax error: operand expected"
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

/// Whitelist for the identity-preexpansion fast path: every byte belongs to
/// the pure arithmetic alphabet (identifiers, digits, operators, parens,
/// comma, colon, question mark, whitespace). A string over this alphabet
/// contains none of the characters the pre-evaluation pipeline reacts to —
/// no `$`/backquote (expansion), no `'`/`"`/`\` (quote handling), no `[`
/// (array subscripts), no `#` (base# literals) — so every pre-evaluation
/// pass (route_current_shell_substitutions, expand_arithmetic_special_
/// parameters, assoc/indexed subscript expansion, normalize_arithmetic_
/// quotes, the bare-quote checks) is the identity on it, matching GNU's
/// expand_arith_string behavior for expansion-character-free input.
/// rubash#156.
fn arithmetic_preexpansion_is_identity(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| {
        matches!(b,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'_'
                | b'+'
                | b'-'
                | b'*'
                | b'/'
                | b'%'
                | b'<'
                | b'>'
                | b'='
                | b'!'
                | b'&'
                | b'|'
                | b'^'
                | b'~'
                | b'?'
                | b':'
                | b','
                | b'('
                | b')'
                | b' '
                | b'\t')
    })
}

/// Executor-injected dynamic names behind `ArithDynamicContext`
/// (`Executor::arith_dynamic_context`, the port of GNU variables.c
/// INIT_DYNAMIC_VAR getters resolved through find_variable at expr.c:1150
/// expr_streval): an operand spelling one of these reads the
/// per-evaluation context, so the fast ticket — which skips building it —
/// admits no expression mentioning them. Every other dynamic spelling
/// (RANDOM/SRANDOM via the RNG state, LINENO via `__RUBASH_CURRENT_LINE`,
/// SECONDS/EPOCHSECONDS/EPOCHREALTIME via the env-derived helper) resolves
/// without the context and stays eligible.
const ARITH_DYNAMIC_CONTEXT_NAMES: [&str; 9] = [
    "BASHPID",
    "BASH_SUBSHELL",
    "BASH_ARGV0",
    "FUNCNAME",
    "GROUPS",
    "BASH_COMMAND",
    "SHELLOPTS",
    "BASHOPTS",
    "PIPESTATUS",
];

/// Syntax half of the #607 fast-ticket gate, over an expression the
/// identity whitelist already admitted (so only the pure arithmetic
/// alphabet can appear — no `$`, quotes, backslashes or `[`).
struct ArithTicketShape {
    /// Whether the text spells any identifier. Identifier-free text needs
    /// no nounset scan; anything with identifiers keeps the slow check.
    has_identifier: bool,
    /// Whether the text can write: contains `=` (every assignment operator
    /// ends in `=`) or `++`/`--`. A read-only shape provably leaves the
    /// write log empty, so the post-evaluation sync is skipped.
    write_possible: bool,
}

/// Whitelist shape gate for the #607 fast ticket: `Some` admits, `None` is
/// doubt and falls through to the slow ticket. Rejects any identifier the
/// skipped ticket items could observe — the nine dynamic-context names
/// (which read the skipped snapshot) and `__RUBASH_` internals (a write
/// shaped like `(( __RUBASH_ARITH_EXP_EXPANDED=1 ))` would leak past the
/// skipped marker removal).
fn fast_arith_ticket_shape(expression: &str) -> Option<ArithTicketShape> {
    let bytes = expression.as_bytes();
    let mut index = 0usize;
    let mut shape = ArithTicketShape {
        has_identifier: false,
        write_possible: false,
    };
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'=' {
            shape.write_possible = true;
            index += 1;
        } else if (byte == b'+' || byte == b'-') && bytes.get(index + 1) == Some(&byte) {
            shape.write_possible = true;
            index += 2;
        } else if byte == b'_' || byte.is_ascii_alphabetic() {
            shape.has_identifier = true;
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index] == b'_' || bytes[index].is_ascii_alphanumeric())
            {
                index += 1;
            }
            let name = &expression[start..index];
            if name.starts_with("__RUBASH_")
                || ARITH_DYNAMIC_CONTEXT_NAMES
                    .iter()
                    .any(|dynamic| *dynamic == name)
            {
                return None;
            }
        } else {
            index += 1;
        }
    }
    Some(shape)
}

/// Whether a variable value needs no nested evaluation: empty (reads 0) or
/// an optional sign plus ASCII digits (parsed directly, with the leading-
/// zero-octal form resolved inline). Any such read terminates without
/// touching the dynamic context, the EXP_EXPANDED marker or the write log,
/// which is what lets the fast ticket skip them.
fn arith_value_is_plain_decimal(value: &str) -> bool {
    let text = value.trim();
    // An empty (or whitespace-only) binding reads as 0 with no recursion.
    if text.is_empty() {
        return true;
    }
    let text = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Canonical counter triple for the arith-for fast driver
/// (`compound_exec.rs`): `for ((v=C0; v<C1; v++))` over one plain variable
/// with pure-decimal bounds. GNU evaluates the same shapes through
/// execute_arith_for_command (execute_cmd.c:3219) + expr.c descent on every
/// iteration; the fast driver performs the identical counter directly and
/// the identical write pipeline, so admission is a strict whitelist and any
/// semantic doubt falls through to the slow loop. The test/update halves
/// reuse the parse-cache admission (identity alphabet, ticket shape,
/// forbidden-op rejection, strict `v<N` / `v++` parse, non-dynamic var);
/// the init half parses its `v=C0` shape here under the same strictness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArithCounterLoop {
    /// Loop variable (`i` in `for ((i=0; i<N; i++))`).
    pub var: String,
    /// Canonical init value (`C0`).
    pub init: i64,
    /// Canonical test bound (`C1`).
    pub bound: i64,
}

/// Strict canonical decimal: ASCII digits only, no sign, no leading-zero
/// multi-digit form (GNU strlong reads that as octal — doubt, not fast),
/// fitting `i64` exactly as rendered (bash_arith wraps every literal, so an
/// overflowing bound takes a different slow value and stays slow).
fn arith_counter_canonical_decimal(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if trimmed.len() > 1 && trimmed.starts_with('0') {
        return None;
    }
    let value: i64 = trimmed.parse().ok()?;
    if value.to_string() != trimmed {
        return None;
    }
    Some(value)
}

/// Whitelist admission for the counter fast driver over the RAW header
/// strings. `Some` admits, `None` is doubt and runs the slow loop.
/// Requires non-empty init/test/update spelling exactly `v=C0`, `v<C1`,
/// `v++` over ONE variable; everything else (`<=`, `==`, `++v`, `v+=1`,
/// `v=i+1`, signed/octal/huge bounds, empty sections, name mismatches)
/// stays slow.
pub(crate) fn arith_counter_loop_shapes(
    init: &str,
    test: &str,
    update: &str,
) -> Option<ArithCounterLoop> {
    if init.trim().is_empty() || test.trim().is_empty() || update.trim().is_empty() {
        return None;
    }
    let (test_var, cached_bound) = match arith_parse_cache_try_admit(test)? {
        (ArithParseCacheKind::CmpLt { var, bound }, _) => (var, bound),
        _ => return None,
    };
    let update_var = match arith_parse_cache_try_admit(update)? {
        (ArithParseCacheKind::PostInc { var }, _) => var,
        _ => return None,
    };
    if test_var != update_var {
        return None;
    }
    // The cached bound is `i64`-wrapped (bash_arith); re-derive it from the
    // raw text and require the exact value so an overflowing literal keeps
    // its slow (differently-wrapped) semantics.
    let angle = test.find('<')?;
    let bound = arith_counter_canonical_decimal(&test[angle + 1..])?;
    if i128::from(bound) != cached_bound {
        return None;
    }
    if test[..angle].trim() != test_var {
        return None;
    }
    // Init is the one shape the parse cache never admits (it rejects `=`);
    // parse it under the same strictness: exactly `v=C0`.
    let (name, value) = init.split_once('=')?;
    if name.trim() != test_var {
        return None;
    }
    Some(ArithCounterLoop {
        var: test_var,
        init: arith_counter_canonical_decimal(value)?,
        bound,
    })
}

/// Binding half of the counter gate, mirroring the write-path guards of
/// `arith_parse_cache_exec_post_inc` (readonly must diagnose, arrays and
/// assocs bind through their own helpers, namerefs retarget) plus the
/// dynamic-name rejection: any doubt runs the slow loop, which keeps the
/// exact slow handling.
pub(crate) fn arith_counter_var_doubtful(
    env_vars: &crate::shell::var_table::VarTable,
    var: &str,
) -> bool {
    arith_parse_cache_is_dynamic(var)
        || is_marked_var(env_vars, ARRAY_VARS, var)
        || is_marked_var(env_vars, ASSOC_VARS, var)
        || is_marked_var(env_vars, ASSOC_128_VARS, var)
        || is_marked_var(env_vars, NAMEREF_VARS, var)
        || is_marked_var(env_vars, READONLY_VARS, var)
}

/// Whether the arithmetic write log is empty. The counter fast driver
/// performs its update writes through the same record/sync pipeline as the
/// slow path (which drains the log per evaluation), so a non-empty log on
/// entry means an unevaluated push is pending and only the slow loop's
/// handling is correct.
pub(crate) fn arith_write_log_is_empty() -> bool {
    ARITH_WRITES.with(|log| log.borrow().is_empty())
}

/// First-knife arithmetic parse cache: `CmpLT(i<N)`, `PostInc(i++)` and
/// `AddAssign(x+=i|x+=1)` specializations. GNU expr.c (5.3) has no parse
/// cache — readtok/re-descent runs per evaluation — so this is pure
/// acceleration; any semantic doubt falls back to the slow parser.
#[derive(Clone, Debug)]
enum ArithParseCacheKind {
    CmpLt { var: String, bound: i128 },
    PostInc { var: String },
    AddAssign { dst: String, src: ArithAddAssignSrc },
}

/// AddAssign right-hand side: a live variable read or an immediate literal.
/// `((x+=i))` evaluates to the NEW value (unlike PostInc's old value).
#[derive(Clone, Debug)]
enum ArithAddAssignSrc {
    Var(String),
    Imm(i128),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArithParseCacheSlot {
    CmpLt,
    PostInc,
    AddAssign,
}

#[derive(Clone, Debug, Default)]
struct ArithParseCache {
    cmp_lt: Option<(String, bool, String, i128)>,
    post_inc: Option<(String, bool, String)>,
    add_assign: Option<(String, bool, String, ArithAddAssignSrc)>,
}

impl ArithParseCache {
    const EMPTY: Self = Self {
        cmp_lt: None,
        post_inc: None,
        add_assign: None,
    };
}

/// Dynamic spellings that must always read live (GNU variables.c dynamic
/// getters / expr.c:1150 expr_streval via find_variable): never constant.
/// The cache closures read live, but admitting these would risk folding a
/// volatile read into a cached shape, so admission rejects them outright
/// and execution double-checks. Covers the fast-ticket context names plus
/// the RNG/derived-clock spellings and the noassign bash arrays.
const ARITH_PARSE_CACHE_DYNAMIC_NAMES: [&str; 20] = [
    "RANDOM",
    "SRANDOM",
    "SECONDS",
    "EPOCHSECONDS",
    "EPOCHREALTIME",
    "BASH_MONOSECONDS",
    "LINENO",
    "BASHPID",
    "BASH_SUBSHELL",
    "BASH_ARGV0",
    "BASH_ARGC",
    "BASH_ARGV",
    "BASH_LINENO",
    "BASH_SOURCE",
    "FUNCNAME",
    "GROUPS",
    "BASH_COMMAND",
    "SHELLOPTS",
    "BASHOPTS",
    "PIPESTATUS",
];

fn arith_parse_cache_is_dynamic(var: &str) -> bool {
    ARITH_PARSE_CACHE_DYNAMIC_NAMES.contains(&var) || var.starts_with("__RUBASH_")
}

/// Third admission layer (new shape rejection): the two cached shapes never
/// contain these operators, so any occurrence is doubt -> slow path.
/// Reused alongside `arithmetic_preexpansion_is_identity` and
/// `fast_arith_ticket_shape` (three-layer AND).
fn arith_parse_cache_has_forbidden_op(expression: &str) -> bool {
    expression.contains('?')
        || expression.contains(':')
        || expression.contains(',')
        || expression.contains('/')
        || expression.contains('%')
        || expression.contains("**")
        || expression.contains("&&")
        || expression.contains("||")
}

/// Strict AddAssign shape: exactly one contiguous `+=` splitting a plain
/// shell-name `dst` and a `src` that is either a plain shell name (Var) or an
/// ASCII-decimal immediate (Imm). Rejects `==`/`<=`/`!=` (equality/relational
/// spellings), any other operator symbol, leading-zero multi-digit immediates
/// (GNU strlong octal arm: `08`/`010` stay slow), overflowing literals
/// (`parse` failure stays slow) and dynamic names. Whitespace around the
/// operands is allowed; anything else is doubt -> None.
fn arith_parse_cache_parse_add_assign(
    expression: &str,
) -> Option<(ArithParseCacheKind, ArithParseCacheSlot)> {
    // Only one `+=` join; `==`/`<=`/`!=`/shifts never contain `+=`.
    if expression.matches("+=").count() != 1 {
        return None;
    }
    // Defensive symbol rejection: the AddAssign alphabet is names, decimal
    // digits, blanks and the single `+=` join only.
    if expression.contains("==") || expression.contains("<=") || expression.contains("!=") {
        return None;
    }
    if expression.contains('<')
        || expression.contains('>')
        || expression.contains('!')
        || expression.contains('?')
        || expression.contains(':')
        || expression.contains(',')
        || expression.contains('/')
        || expression.contains('%')
        || expression.contains('*')
        || expression.contains('&')
        || expression.contains('|')
        || expression.contains('^')
        || expression.contains('~')
        || expression.contains('(')
        || expression.contains(')')
    {
        return None;
    }
    let (left_raw, right_raw) = expression.split_once("+=")?;
    let dst = left_raw.trim();
    let src = right_raw.trim();
    if dst.is_empty() || src.is_empty() {
        return None;
    }
    if !is_shell_name(dst) || arith_parse_cache_is_dynamic(dst) {
        return None;
    }
    // Immediate path: unsigned ASCII digits only (a leading `-`/`+` stays
    // slow via the doubt below, keeping the union pre-filter minimal).
    if src.bytes().all(|b| b.is_ascii_digit()) {
        if src.len() > 1 && src.starts_with('0') {
            return None;
        }
        let imm: i128 = src.parse().ok()?;
        let imm = imm as i64 as i128;
        return Some((
            ArithParseCacheKind::AddAssign {
                dst: dst.to_string(),
                src: ArithAddAssignSrc::Imm(imm),
            },
            ArithParseCacheSlot::AddAssign,
        ));
    }
    // Variable path: a single plain shell name, never dynamic.
    if !is_shell_name(src) || arith_parse_cache_is_dynamic(src) {
        return None;
    }
    Some((
        ArithParseCacheKind::AddAssign {
            dst: dst.to_string(),
            src: ArithAddAssignSrc::Var(src.to_string()),
        },
        ArithParseCacheSlot::AddAssign,
    ))
}

/// Strict shape parse over the RAW expression text (no normalization):
/// exactly `name < decimal` (single `<`, no `<=`/`<<`/`=`/`>`/`!`),
/// exactly `name++` (suffix only, whitespace allowed), or exactly
/// `dst += src` where `src` is a variable name or a decimal immediate
/// (single `+=`, no `==`/`<=`/`!=`, no other operator symbols, no
/// leading-zero multi-digit immediates, no overflowing literals, no dynamic
/// names). Anything else is doubt -> None (slow path).
fn arith_parse_cache_parse(expression: &str) -> Option<(ArithParseCacheKind, ArithParseCacheSlot)> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return None;
    }
    // PostInc first: `name++` with optional blanks before `++`.
    if let Some(stripped) = trimmed.strip_suffix("++") {
        let base = stripped.trim_end();
        if !base.is_empty()
            && is_shell_name(base)
            && !arith_parse_cache_is_dynamic(base)
            && stripped.as_bytes().iter().all(|b| {
                matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b' ' | b'\t' | b'+')
            })
            && !base.contains('+')
            && !trimmed.contains("--")
        {
            return Some((
                ArithParseCacheKind::PostInc {
                    var: base.to_string(),
                },
                ArithParseCacheSlot::PostInc,
            ));
        }
        return None;
    }
    // AddAssign: exactly `dst+=src` / `dst += src` with a plain-name dst
    // and a Var|Imm src. Single `+=`, no `==`/`<=`/`!=`/other symbols, no
    // leading-zero immediates, no overflowing literals, no dynamic names.
    if let Some(parsed) = arith_parse_cache_parse_add_assign(expression) {
        return Some(parsed);
    }
    // CmpLT: exactly one `<`, otherwise only name/digits/blanks.
    if expression.as_bytes().iter().filter(|b| **b == b'<').count() != 1 {
        return None;
    }
    if expression.contains('>') || expression.contains('=') || expression.contains('!') {
        return None;
    }
    let angle = expression.find('<')?;
    let (left_raw, right_raw) = expression.split_at(angle);
    let right_raw = &right_raw[1..];
    // Reject `<=` / `<<` (the byte after `<` must start the bound).
    if right_raw.starts_with(['=', '<']) {
        return None;
    }
    let left = left_raw.trim();
    let right = right_raw.trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    if !is_shell_name(left) || arith_parse_cache_is_dynamic(left) {
        return None;
    }
    if !right.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Leading-zero multi-digit bounds take GNU's octal strlong path —
    // doubt -> slow (keeps `010` == 8 semantics).
    if right.len() > 1 && right.starts_with('0') {
        return None;
    }
    let bound: i128 = right.parse().ok()?;
    // Bound itself goes through bash_arith wrapping like any literal.
    let bound = bound as i64 as i128;
    Some((
        ArithParseCacheKind::CmpLt {
            var: left.to_string(),
            bound,
        },
        ArithParseCacheSlot::CmpLt,
    ))
}

/// Admission: three-layer AND reusing the existing whitelist gates plus the
/// new shape rejection. Returns the parsed specialization and its slot.
///
/// The union-alphabet pre-filter runs first: every cached shape only contains
/// bytes in `[A-Za-z0-9_ \t<+=]`, so a single short-circuiting scan rejects
/// anything outside it before the identity + ticket + forbidden scans run.
/// Any expression the pre-filter rejects would also fail the strict parse
/// below, so the short-circuit is semantics-preserving.
fn arith_parse_cache_try_admit(
    expression: &str,
) -> Option<(ArithParseCacheKind, ArithParseCacheSlot)> {
    if !expression.as_bytes().iter().all(|b| {
        matches!(
            b,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b' ' | b'\t' | b'<' | b'+' | b'='
        )
    }) {
        return None;
    }
    if !arithmetic_preexpansion_is_identity(expression.as_bytes()) {
        return None;
    }
    if fast_arith_ticket_shape(expression).is_none() {
        return None;
    }
    if arith_parse_cache_has_forbidden_op(expression) {
        return None;
    }
    arith_parse_cache_parse(expression)
}

fn arith_parse_cache_lookup(expression: &str, no_expand: bool) -> Option<ArithParseCacheKind> {
    ARITH_PARSE_CACHE.with(|cache| {
        let cache = cache.borrow();
        if let Some((expr, flag, var, bound)) = cache.cmp_lt.as_ref() {
            if expr == expression && *flag == no_expand {
                return Some(ArithParseCacheKind::CmpLt {
                    var: var.clone(),
                    bound: *bound,
                });
            }
        }
        if let Some((expr, flag, var)) = cache.post_inc.as_ref() {
            if expr == expression && *flag == no_expand {
                return Some(ArithParseCacheKind::PostInc { var: var.clone() });
            }
        }
        if let Some((expr, flag, dst, src)) = cache.add_assign.as_ref() {
            if expr == expression && *flag == no_expand {
                return Some(ArithParseCacheKind::AddAssign {
                    dst: dst.clone(),
                    src: src.clone(),
                });
            }
        }
        None
    })
}

fn arith_parse_cache_store(
    slot: ArithParseCacheSlot,
    expression: String,
    no_expand: bool,
    kind: ArithParseCacheKind,
) {
    ARITH_PARSE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        match (slot, kind) {
            (ArithParseCacheSlot::CmpLt, ArithParseCacheKind::CmpLt { var, bound }) => {
                cache.cmp_lt = Some((expression, no_expand, var, bound));
            }
            (ArithParseCacheSlot::PostInc, ArithParseCacheKind::PostInc { var }) => {
                cache.post_inc = Some((expression, no_expand, var));
            }
            (ArithParseCacheSlot::AddAssign, ArithParseCacheKind::AddAssign { dst, src }) => {
                cache.add_assign = Some((expression, no_expand, dst, src));
            }
            _ => {}
        }
    });
}

/// Plain-decimal load with octal doubt: mirrors `variable_value` /
/// `evaluate_variable_text` for the values the fast ticket already admits
/// (empty -> 0, sign+digits -> bash_arith). A leading-zero multi-digit
/// load takes GNU strlong's octal arm -> None (slow path, entry kept).
fn arith_parse_cache_plain_load(raw: &str) -> Option<i128> {
    if !arith_value_is_plain_decimal(raw) {
        return None;
    }
    let text = raw.trim();
    if text.is_empty() {
        return Some(0);
    }
    let digits = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    text.parse::<i128>().ok().map(|v| v as i64 as i128)
}

fn arith_parse_cache_var_doubtful(env_vars: &crate::shell::var_table::VarTable, var: &str) -> bool {
    if arith_parse_cache_is_dynamic(var) {
        return true;
    }
    is_marked_var(env_vars, ARRAY_VARS, var)
        || is_marked_var(env_vars, ASSOC_VARS, var)
        || is_marked_var(env_vars, ASSOC_128_VARS, var)
        || is_marked_var(env_vars, NAMEREF_VARS, var)
}

fn arith_parse_cache_exec_cmp_lt(
    var: &str,
    bound: i128,
    env_vars: &mut crate::shell::var_table::VarTable,
) -> Option<i128> {
    // Read-only shape: readonly cells read fine, but arrays/assocs/namerefs
    // and dynamic spellings re-enter the nested evaluator -> slow.
    if arith_parse_cache_var_doubtful(env_vars, var) {
        return None;
    }
    let raw = env_vars
        .get(var)
        .cloned()
        .or_else(|| std::env::var(var).ok())
        .unwrap_or_default();
    let current = arith_parse_cache_plain_load(&raw)?;
    Some(i128::from(current < bound))
}

fn arith_parse_cache_exec_post_inc(
    var: &str,
    env_vars: &mut crate::shell::var_table::VarTable,
) -> Option<i128> {
    // Writes need the exact slow-path guards: readonly must diagnose,
    // arrays/assocs/namerefs bind through their own helpers -> slow.
    if arith_parse_cache_var_doubtful(env_vars, var) || is_marked_var(env_vars, READONLY_VARS, var)
    {
        return None;
    }
    let raw = env_vars
        .get(var)
        .cloned()
        .or_else(|| std::env::var(var).ok())
        .unwrap_or_default();
    let current = arith_parse_cache_plain_load(&raw)?;
    let updated = current.wrapping_add(1) as i64 as i128;
    let old_value = env_vars.get(var).cloned();
    let new_value = updated.to_string();
    env_vars.insert(var.to_string(), new_value.clone());
    record_arith_write(var, old_value);
    sync_shell_assignment_process_env(env_vars, var, &new_value);
    // GNU variables.c:5819/5919 sv_path -> phash_flush (rubash#1551).
    if var == "PATH" {
        crate::builtins::hash::flush_command_hash_table(env_vars);
    }
    Some(current)
}

/// Whether a cached-shape variable is currently unbound (no shell entry and
/// no process-environment entry). An unbound read takes the slow path so the
/// nounset trap and the exact slow binding creation stay intact; the cache
/// entry is kept (never evicted on this hit-fallback).
fn arith_parse_cache_is_unbound(env_vars: &crate::shell::var_table::VarTable, var: &str) -> bool {
    env_vars.get(var).is_none() && std::env::var(var).is_err()
}

fn arith_parse_cache_exec_add_assign(
    dst: &str,
    src: &ArithAddAssignSrc,
    env_vars: &mut crate::shell::var_table::VarTable,
) -> Option<i128> {
    // Write guards mirror the slow path: readonly must diagnose,
    // arrays/assocs/namerefs bind through their own helpers, dynamic names
    // read live -> all slow. Unbound or non-plain-decimal loads are plain
    // doubt -> slow; the entry is kept in every fallback.
    if arith_parse_cache_var_doubtful(env_vars, dst) || is_marked_var(env_vars, READONLY_VARS, dst)
    {
        return None;
    }
    if arith_parse_cache_is_unbound(env_vars, dst) {
        return None;
    }
    let addend = match src {
        ArithAddAssignSrc::Imm(imm) => *imm,
        ArithAddAssignSrc::Var(var) => {
            if arith_parse_cache_var_doubtful(env_vars, var) {
                return None;
            }
            if arith_parse_cache_is_unbound(env_vars, var) {
                return None;
            }
            let raw = env_vars
                .get(var)
                .cloned()
                .or_else(|| std::env::var(var).ok())
                .unwrap_or_default();
            arith_parse_cache_plain_load(&raw)?
        }
    };
    let raw_dst = env_vars
        .get(dst)
        .cloned()
        .or_else(|| std::env::var(dst).ok())
        .unwrap_or_default();
    let current = arith_parse_cache_plain_load(&raw_dst)?;
    let updated = current.wrapping_add(addend) as i64 as i128;
    let old_value = env_vars.get(dst).cloned();
    let new_value = updated.to_string();
    env_vars.insert(dst.to_string(), new_value.clone());
    record_arith_write(dst, old_value);
    sync_shell_assignment_process_env(env_vars, dst, &new_value);
    // GNU variables.c:5819/5919 sv_path -> phash_flush (rubash#1551).
    if dst == "PATH" {
        crate::builtins::hash::flush_command_hash_table(env_vars);
    }
    // `((x+=i))` evaluates to the NEW value, unlike PostInc's old value.
    Some(updated)
}

#[cfg(test)]
mod arith_parse_cache_tests {
    use super::{arith_parse_cache_parse, arith_parse_cache_try_admit, ArithParseCacheKind};
    use crate::executor::Executor;

    #[test]
    fn only_cached_shapes_admit() {
        assert!(arith_parse_cache_try_admit("i<200000").is_some());
        assert!(arith_parse_cache_try_admit("i++").is_some());
        assert!(arith_parse_cache_try_admit("i ++").is_some());
        assert!(arith_parse_cache_try_admit("x+=i").is_some());
        assert!(arith_parse_cache_try_admit("x+=1").is_some());
        assert!(arith_parse_cache_try_admit("x += i").is_some());
        // Everything else is doubt -> slow path, no store.
        for expression in [
            "i<=n",
            "i<<2",
            "i==n",
            "i=0",
            "x==i",
            "x<=1",
            "x!=i",
            "x+=RANDOM",
            "RANDOM+=1",
            "x+=08",
            "x+=010",
            "x+=1+2",
            "x+=i*2",
            "x+=--1",
            "x+=-1",
            "i--",
            "++i",
            "a?b:c",
            "a,b",
            "a/b",
            "a%b",
            "a**2",
            "a&&b",
            "a||b",
            "RANDOM<10",
            "RANDOM++",
            "x+=170141183460469231731687303715884105728",
            "i<N ",
        ] {
            if expression == "i<N " {
                // Trailing blank still parses as CmpLT over a name bound —
                // the state gate (not admission) decides; admission holds.
                continue;
            }
            assert!(
                arith_parse_cache_try_admit(expression).is_none(),
                "{expression} must not admit"
            );
        }
    }

    #[test]
    fn cached_shapes_evaluate_like_slow() {
        let mut executor = Executor::new();
        executor.set_env("i", "5");
        // First evaluation misses (stores), the rest hit the closures.
        for _ in 0..3 {
            assert_eq!(executor.eval_arithmetic_command_value("i<10"), Some(1));
            assert_eq!(executor.eval_arithmetic_command_value("i<3"), Some(0));
            assert_eq!(executor.eval_arithmetic_command_value("i++"), Some(5));
            executor.set_env("i", "5");
        }
        assert_eq!(executor.get_env("i"), Some("5"));
        // Octal-valued loads and expression-valued loads fall back to slow
        // without changing the outcome.
        executor.set_env("i", "010");
        assert_eq!(executor.eval_arithmetic_command_value("i<20"), Some(1));
        executor.set_env("i", "1+2");
        assert_eq!(executor.eval_arithmetic_command_value("i<10"), Some(1));
        assert_eq!(executor.eval_arithmetic_command_value("i++"), Some(3));
        assert_eq!(executor.get_env("i"), Some("4"));
    }

    #[test]
    fn add_assign_evaluates_to_new_value() {
        let mut executor = Executor::new();
        executor.set_env("x", "5");
        executor.set_env("i", "3");
        // First evaluation misses (stores), the rest hit the closure.
        // Unlike PostInc (old value), AddAssign returns the NEW value.
        for _ in 0..3 {
            executor.set_env("x", "5");
            assert_eq!(executor.eval_arithmetic_command_value("x+=i"), Some(8));
            assert_eq!(executor.get_env("x"), Some("8"));
            executor.set_env("x", "5");
            assert_eq!(executor.eval_arithmetic_command_value("x+=1"), Some(6));
            assert_eq!(executor.get_env("x"), Some("6"));
        }
        // Doubtful loads fall back to slow without changing the outcome.
        executor.set_env("x", "010");
        assert_eq!(executor.eval_arithmetic_command_value("x+=i"), Some(11));
        executor.set_env("x", "1+2");
        executor.set_env("i", "3");
        assert_eq!(executor.eval_arithmetic_command_value("x+=i"), Some(6));
    }

    #[test]
    fn cache_keys_keep_raw_text_and_flag() {
        // `i<10` vs `i < 10` are distinct raw keys sharing one slot: each
        // miss re-parses slowly and correctly, hits stay exact.
        let mut executor = Executor::new();
        executor.set_env("i", "5");
        assert_eq!(executor.eval_arithmetic_command_value("i<10"), Some(1));
        assert_eq!(executor.eval_arithmetic_command_value("i < 10"), Some(1));
        assert_eq!(executor.eval_arithmetic_command_value("i<10"), Some(1));
        let _ = arith_parse_cache_parse("i<10");
        assert!(matches!(
            arith_parse_cache_parse("i<10"),
            Some((ArithParseCacheKind::CmpLt { .. }, _))
        ));
    }
}

#[cfg(test)]
mod fast_arith_ticket_tests {
    use super::arithmetic_preexpansion_is_identity;
    use super::{
        arith_value_is_plain_decimal, fast_arith_ticket_shape, ARITH_DYNAMIC_CONTEXT_NAMES,
    };
    use crate::executor::Executor;

    #[test]
    fn hot_loop_shapes_are_fast() {
        for expression in [
            "1", "i<200000", "i++", "++i", "i--", "i=0", "i += 1", "a+b*c", "(x)", "i<=n",
        ] {
            assert!(
                arithmetic_preexpansion_is_identity(expression.as_bytes()),
                "{expression} must pass the identity whitelist"
            );
            assert!(
                fast_arith_ticket_shape(expression).is_some(),
                "{expression} must stay fast-eligible"
            );
        }
        assert!(!fast_arith_ticket_shape("i<200000").unwrap().write_possible);
        assert!(!fast_arith_ticket_shape("1").unwrap().write_possible);
        assert!(fast_arith_ticket_shape("i++").unwrap().write_possible);
        assert!(fast_arith_ticket_shape("i=0").unwrap().write_possible);
        assert!(!fast_arith_ticket_shape("1").unwrap().has_identifier);
        assert!(fast_arith_ticket_shape("i++").unwrap().has_identifier);
    }

    #[test]
    fn doubt_falls_through_to_slow() {
        // Dynamic-context names read the skipped snapshot, so the shape
        // gate must reject them even though they pass the identity
        // whitelist.
        for name in ARITH_DYNAMIC_CONTEXT_NAMES {
            let expression = format!("{name}+1");
            assert!(
                arithmetic_preexpansion_is_identity(expression.as_bytes()),
                "{expression} passes identity so the shape gate must reject it"
            );
            assert!(
                fast_arith_ticket_shape(&expression).is_none(),
                "{expression} must take the slow ticket"
            );
        }
        // Internal marker traffic is never fast.
        assert!(fast_arith_ticket_shape("__RUBASH_ARITH_EXP_EXPANDED").is_none());
        // Subscripts and expansions never reach the shape gate: the identity
        // whitelist rejects them first.
        for expression in ["a[1]", "$i"] {
            assert!(
                !arithmetic_preexpansion_is_identity(expression.as_bytes()),
                "{expression} must not pass the identity whitelist"
            );
        }
    }

    #[test]
    fn decimal_values_keep_state_eligible() {
        assert!(arith_value_is_plain_decimal("199999"));
        assert!(arith_value_is_plain_decimal(""));
        assert!(arith_value_is_plain_decimal("   "));
        assert!(arith_value_is_plain_decimal("  +42 "));
        assert!(arith_value_is_plain_decimal("-7"));
        assert!(arith_value_is_plain_decimal("007"));
        assert!(!arith_value_is_plain_decimal("1+2"));
        assert!(!arith_value_is_plain_decimal("BASHPID"));
        assert!(!arith_value_is_plain_decimal("a[1]"));
        assert!(!arith_value_is_plain_decimal("+"));
    }

    /// Eligibility probe for the contract test below: the production gate
    /// also requires the `(( ))` caller (expand), which this test's
    /// evaluator is.
    fn fast_eligible(executor: &Executor, expression: &str) -> bool {
        arithmetic_preexpansion_is_identity(expression.as_bytes())
            && fast_arith_ticket_shape(expression).is_some()
            && executor.fast_arith_ticket_state_ok(expression)
    }

    /// Contract: fast-eligible expressions evaluate (value and side-effect
    /// mirror) exactly as the slow ticket does. Eligible inputs run the
    /// fast ticket inside the evaluator, so any fast/slow divergence in
    /// evaluation, nounset, readonly or sync surfaces as a wrong value or
    /// a wrong binding here.
    #[test]
    fn fast_eligible_exprs_evaluate_exactly() {
        let mut executor = Executor::new();
        executor.set_env("i", "5");
        // (expression, expected value, expected `i` binding afterwards)
        let probes: &[(&str, Option<i128>, &str)] = &[
            ("1", Some(1), "5"),
            ("i", Some(5), "5"),
            ("i<200000", Some(1), "5"),
            ("i+1", Some(6), "5"),
            ("i++", Some(5), "6"),
            ("++i", Some(7), "7"),
            ("i=200000", Some(200000), "200000"),
        ];
        for (expression, expected, i_after) in probes {
            assert!(
                fast_eligible(&executor, expression),
                "{expression} must be fast-eligible under decimal bindings"
            );
            assert_eq!(
                executor.eval_arithmetic_command_value(expression),
                *expected,
                "value of {expression}"
            );
            assert_eq!(
                executor.get_env("i"),
                Some(*i_after),
                "binding of i after {expression}"
            );
        }
        // The `let` caller stays on the slow ticket (EXP_EXPANDED) and
        // must agree with the fast `(( ))` result.
        executor.set_env("i", "5");
        assert_eq!(
            executor.eval_arithmetic_command_value_no_expand("i++"),
            Some(5)
        );
        assert_eq!(executor.get_env("i"), Some("6"));
    }

    #[test]
    fn state_gate_falls_back_without_changing_semantics() {
        let mut executor = Executor::new();
        // A non-decimal binding fails the state gate (slow ticket) but
        // evaluates identically: the nested read still resolves.
        executor.set_env("i", "1+2");
        assert!(!fast_eligible(&executor, "i"));
        assert_eq!(executor.eval_arithmetic_command_value("i"), Some(3));
        // Dynamic-context spellings stay slow and stay correct.
        assert!(!fast_eligible(&executor, "BASHPID+1"));
        assert!(executor.eval_arithmetic_command_value("BASHPID+1").unwrap() > 0);
    }

    #[test]
    fn fast_ticket_preserves_nounset() {
        let mut executor = Executor::new();
        executor.set_shell_option("nounset", true);
        // Identifier-free fast expressions skip the nounset scan and pass.
        assert_eq!(executor.eval_arithmetic_command_value("1"), Some(1));
        // Bound identifiers pass with the exact slow check.
        executor.set_env("i", "5");
        assert_eq!(executor.eval_arithmetic_command_value("i"), Some(5));
        // Unbound identifiers trap exactly as on the slow ticket.
        assert_eq!(
            executor.eval_arithmetic_command_value("missing_name_xyz"),
            None
        );
        assert!(executor.shell_state.arithmetic_nounset_error.get());
    }
}
