//! Arithmetic expression parsing and evaluation.
//!
//! Provides parsing and evaluation of shell arithmetic expressions including
//! variables, arrays, assignments, and ternary conditionals.

mod parser;

use parser::ConditionalArithParser;
use std::cell::Cell;
use std::collections::HashMap;

use super::Executor;
use crate::executor::{is_marked_var, SubstitutionQuoteContext, ASSOC_VARS};

/// Categories surfaced by GNU Bash's arithmetic evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArithmeticErrorCategory {
    EmptyArraySubscript,
    DivisionByZero,
    InvalidLiteral,
    NonVariableAssignment,
    EvaluatorFailure,
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
            self.arithmetic_expansion_error.get(),
            self.arithmetic_nonfatal_error.get(),
            self.arithmetic_fatal_error.get(),
            self.arithmetic_nounset_error.get(),
            self.arithmetic_last_error_category.get(),
        )
    }

    /// Restore flags saved by [`Self::snapshot_arithmetic_error_flags`].
    /// Returns true when a `set -u` unbound-variable error was raised inside
    /// the bounded region (it was clear on entry and is set now).
    pub(crate) fn restore_arithmetic_error_flags(&self, saved: &(bool, bool, bool, bool, Option<ArithmeticErrorCategory>)) -> bool {
        let nounset_hit = self.arithmetic_nounset_error.get() && !saved.3;
        self.arithmetic_expansion_error.set(saved.0);
        self.arithmetic_nonfatal_error.set(saved.1);
        self.arithmetic_fatal_error.set(saved.2);
        self.arithmetic_nounset_error.set(saved.3);
        self.arithmetic_last_error_category.set(saved.4);
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
            &crate::executor::execution_misc::decode_command_substitution_payload(
                &expanded,
            ),
        )
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
                let is_funsub = after.starts_with('|')
                    || after.starts_with(|c: char| c.is_whitespace());
                if is_funsub {
                    let mut inner = after.chars().peekable();
                    if let Some(value) =
                        self.expand_current_shell_braced_substitution(&mut inner)
                    {
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
        self.arithmetic_last_error_category.set(None);
        // Associative subscripts are expanded first, in their own pass, and
        // replaced by an opaque literal (see expand_arithmetic_assoc_subscripts)
        // so the ordinary expansion below cannot expand them a second time and
        // the parser stores the key verbatim.
        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(expression);
        let expression = normalize_arithmetic_quotes(
            &self.expand_arithmetic_expression_mut(&with_assoc_keys),
        );
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "nounset") {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.env_vars) {
                self.arithmetic_nounset_error.set(true);
                if !self.arithmetic_expansion_error.replace(true) {
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
        if has_bare_single_quote(&expression, &self.env_vars) {
            return None;
        }
        if empty_quoted_operand_has_operator(&expression) {
            return None;
        }
        // Save a snapshot of variable values before evaluation to detect changes.
        let pre_eval_vars: HashMap<String, String> = self.env_vars.clone();
        let (value, category) = eval_mutable_arith_value_with_random(
            &expression,
            &mut self.env_vars,
            Some(&self.random_state),
        );
        self.arithmetic_last_error_category.set(category);
        self.report_arithmetic_readonly_error();

        // Sync any variable changes from env_vars to shell_state.variables
        // so that subsequent variable expansions see the updated values.
        for (name, new_value) in &self.env_vars {
            if pre_eval_vars.get(name) != Some(new_value) {
                // Variable was modified during arithmetic evaluation
                if let Some(variable) = self.shell_state.variables.get_mut(name) {
                    variable.value = crate::shell::ShellValue::Scalar(new_value.clone());
                } else if !name.starts_with("__RUBASH_") {
                    // Create a new entry in shell_state for non-internal variables
                    let _ = self.shell_state.variables.set_scalar(name, new_value);
                }
            }
        }

        value
    }

    /// Evaluate a `$(( ... ))` expansion embedded in a word. This is the
    /// expansion context: Bash strips double quotes from the expression
    /// before evaluation (`$(( "1" + 1 ))` is `2`), while the command
    /// context (`for (( ... ))` headers) keeps them and rejects them.
    pub(crate) fn eval_arithmetic_expansion_value(&mut self, expression: &str) -> Option<i128> {
        self.arithmetic_last_error_category.set(None);
        let with_assoc_keys = self.expand_arithmetic_assoc_subscripts(expression);
        let expression = normalize_arithmetic_quotes(
            &self.expand_arithmetic_expression_mut(&with_assoc_keys),
        );
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "nounset") {
            if let Some(name) = arithmetic_unbound_variable(&expression, &self.env_vars) {
                self.arithmetic_nounset_error.set(true);
                if !self.arithmetic_expansion_error.replace(true) {
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
        let pre_eval_vars: HashMap<String, String> = self.env_vars.clone();
        let (value, category) = eval_mutable_arith_value_with_random(
            &expression,
            &mut self.env_vars,
            Some(&self.random_state),
        );
        self.arithmetic_last_error_category.set(category);
        self.report_arithmetic_readonly_error();

        // Sync any variable changes from env_vars to shell_state.variables
        // so that subsequent parameter expansions see arithmetic side effects
        // (e.g., ++i in array subscripts like a[++i]=value).
        for (name, new_value) in &self.env_vars {
            if pre_eval_vars.get(name) != Some(new_value) {
                if let Some(variable) = self.shell_state.variables.get_mut(name) {
                    variable.value = crate::shell::ShellValue::Scalar(new_value.clone());
                } else if !name.starts_with("__RUBASH_") {
                    let _ = self.shell_state.variables.set_scalar(name, new_value);
                }
            }
        }

        value
    }

    fn report_arithmetic_readonly_error(&mut self) {
        let Some(name) = self.env_vars.remove("__RUBASH_ARITH_READONLY_ERROR") else {
            return;
        };
        if !self.arithmetic_expansion_error.replace(true) {
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
        if !self.arithmetic_fatal_error.get() && !self.arithmetic_nounset_error.get() {
            return Ok(());
        }
        let nounset = self.arithmetic_nounset_error.replace(false);
        self.arithmetic_fatal_error.set(false);
        self.arithmetic_expansion_error.set(false);
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
    fn expand_arithmetic_assoc_subscripts(&mut self, expression: &str) -> String {
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
            while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &expression[start..index];
            if index < bytes.len()
                && bytes[index] == b'['
                && is_marked_var(&self.env_vars, ASSOC_VARS, name)
            {
                let end = assoc_subscript_end(bytes, index);
                if end > index + 1 && bytes.get(end - 1) == Some(&b']') {
                    let raw = &expression[index + 1..end - 1];
                    let key = self.expand_assoc_subscript_once(raw);
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
        self.expand_word_mut_with_context(raw, SubstitutionQuoteContext::Unquoted)
    }

    pub(super) fn expand_arithmetic_special_parameters(&self, expression: &str) -> String {
        // In arithmetic contexts, special parameters expand to numeric values:
        // $- -> 0 (shell flags not meaningful in arithmetic), $# -> param count
        // GNU Bash treats $- as 0 in $(( $- )). See array.tests line 60.
        let expression = expression
            .replace("$#", &self.positional_params.len().to_string())
            .replace("$-", "0");
        // GNU subst.c: the text between (( and )) is treated as if in double
        // quotes — single quotes are literal data for expr.c (`(( '1' ))` is
        // an operand error, not the number 1), so they must survive the
        // embedded-parameter walker's quote removal. \x17 is the walker's
        // literal-single-quote marker. Backslash-escaped double quotes
        // (`\"`) must also survive as literal `"` — the walker strips bare
        // `"` via toggle mode, so `\"` → `\` + removed quote. \x18 is the
        // walker's literal-double-quote marker.
        let protected = expression
            .replace("\\\"", "\x18")
            .replace('\'', "\x17");
        self.expand_embedded_parameters(&protected)
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
    let (_, category) = eval_mutable_arith_result(expression, &mut env_vars, None);
    category
}

pub(crate) fn eval_conditional_arith_value(
    value: &str,
    env_vars: &HashMap<String, String>,
) -> Option<i128> {
    let mut env_vars = env_vars.clone();
    eval_mutable_arith_value(value, &mut env_vars)
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
    eval_mutable_arith_result(value, &mut env_vars, None)
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
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch != b'"' {
            output.push(ch as char);
            index += 1;
            continue;
        }
        index += 1;
        while index < bytes.len() {
            let next = bytes[index];
            if next == b'"' {
                index += 1;
                break;
            }
            if next == b'\\' {
                index += 1;
                if index < bytes.len() {
                    output.push(bytes[index] as char);
                    index += 1;
                } else {
                    output.push('\\');
                }
            } else {
                output.push(next as char);
                index += 1;
            }
        }
    }
    output
}

/// Byte index just past the `]` that closes the subscript opened at `open`
/// (`bytes[open] == b'['`), honoring single/double quotes and `\` escapes so a
/// `]` inside a quoted key does not terminate the subscript.
fn assoc_subscript_end(bytes: &[u8], open: usize) -> usize {
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
    index
}

/// Skip the shell substitution starting at `start` (`bytes[start] == b'$'` for
/// `$(`, `$((`, `${`, or a backtick) and return the index just past it. An
/// unterminated substitution runs to the end of the input so a missing close
/// can never make a stray `]` look like the subscript delimiter.
fn assoc_skip_substitution(bytes: &[u8], start: usize) -> usize {
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
pub(super) const ARITH_ASSOC_KEY_MARKER: char = '\u{1e}';

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
pub(super) fn decode_arithmetic_assoc_key(text: &str) -> Option<String> {
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
                index = assoc_subscript_end(bytes, index);
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
pub(in crate::executor) fn arithmetic_error_message(
    expression: &str,
    trailing_space: bool,
) -> Option<String> {
    // GNU expr.c evalerror skips the expression's leading whitespace at
    // display time (expr.c:1528: `for (t = expression; whitespace (*t); t++)`)
    // while keeping everything from there on verbatim, trailing blanks
    // included. Every message below therefore echoes the leading-trimmed
    // text, never the raw expansion.
    let expression = expression.trim_start();
    let token_space = if trailing_space { " " } else { "" };
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
        return Some(format!(
            "{display}: {error} (error token is \"{token}\")"
        ));
    }

    // GNU Bash rejects a bare assignment target behind && / || even when
    // short-circuit skips it: `$((0 && B=42))` fails with
    // "attempted assignment to non-variable" (error token is "=42").
    if let Some(token) = logical_rhs_assignment_token(expression) {
        return Some(format!(
            "{expression}: attempted assignment to non-variable (error token is \"{token}\")"
        ));
    }

    // An empty ternary branch is a parse failure in Bash:
    // `$((4 ? 20 : ))` reports "expression expected" (error token is ": ").
    if let Some(token) = empty_ternary_branch_token(expression) {
        return Some(format!(
            "{expression}: expression expected (error token is \"{token}\")"
        ));
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
            "{display_expression}: syntax error: operand expected (error token is \"{token}\")"
        ));
    }
    let assignment_lvalue_is_digit = trimmed
        .split_once('=')
        .is_some_and(|(left, _)| {
            let left = left.trim();
            !left.ends_with(['=', '<', '>', '!'])
                && left.chars().all(|ch| ch.is_ascii_digit())
        })
        || trimmed
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
            "syntax error: operand expected"
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
            "{expression}: syntax error: operand expected (error token is \"\"\")"
        ));
    }

    if trimmed.ends_with(['+', '-', '*', '/', '%', '&', '|', '^', '<', '>']) {
        let token = trimmed.chars().last().unwrap_or_default();
        return Some(format!(
            "{expression}: syntax error: operand expected (error token is \"{token}\")"
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
            let token = &expression[index + 1..end];
            return Some(format!(
                "{expression}: syntax error: invalid arithmetic operator (error token is \"{token}{token_space}\")"
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
            return Some(format!(
                "{expression}: exponent less than 0 (error token is \"{digits}{token_space}\")"
            ));
        }
    }

    // Quoted operand like `$(( '1' ))`: Bash treats `'1'` as a variable
    // reference (which does not exist) and reports `operand expected`.
    // Double quotes are fine (`$(( "1" ))` is 1), so only single quotes count.
    if let Some(start) = expression.find('\'') {
        let rest = &expression[start + 1..];
        let end = rest
            .find('\'')
            .map(|index| start + 1 + index)
            .unwrap_or(expression.len());
        let token = &expression[start..end];
        return Some(format!(
            "{expression}: syntax error: operand expected (error token is \"{token}{token_space}\")"
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

    None
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
    random_state: Option<&Cell<u32>>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
    // GNU Bash's subexpr() treats an empty arithmetic expression as zero.
    // This matters for expansion and variable contexts, where an empty
    // quoted operand is valid rather than a parser failure. Lexer quote
    // markers must be normalized before lvalue parsing as well as expansion.
    let normalized = normalize_arithmetic_quotes(value);
    if normalized.trim().is_empty() {
        return (Some(0), None);
    }
    eval_mutable_arith_result(value, env_vars, random_state)
}

fn eval_mutable_arith_result(
    value: &str,
    env_vars: &mut HashMap<String, String>,
    random_state: Option<&Cell<u32>>,
) -> (Option<i128>, Option<ArithmeticErrorCategory>) {
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
    };
    let value = parser.parse_comma();
    parser.skip_ws();
    let value = value.filter(|_| parser.pos == parser.input.len());
    let category = parser.error_category.or_else(|| {
        if value.is_none() && numeric_assignment_expression(&normalized) {
            Some(ArithmeticErrorCategory::NonVariableAssignment)
        } else if value.is_none() {
            Some(ArithmeticErrorCategory::EvaluatorFailure)
        } else {
            None
        }
    });
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

/// Detects `$((cond ? true :))` shapes where the false branch holds only
/// whitespace; returns the `": "` error token GNU prints.
fn empty_ternary_branch_token(expression: &str) -> Option<String> {
    let question = expression.find('?')?;
    let colon = expression[question..].find(':')? + question;
    if expression[colon + 1..].trim().is_empty() {
        Some(": ".to_string())
    } else {
        None
    }
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
fn trailing_operator_error(expression: &str, _trailing_space: bool) -> Option<String> {
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
                "**" | "==" | "!=" | "<=" | ">=" | "<<" | ">>" | "&&" | "||" | "++" | "--" | "+="
                    | "-=" | "*=" | "/=" | "%=" | "&=" | "^=" | "|="
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
