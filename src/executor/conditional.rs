//! Conditional/test expression evaluation for the executor.
//!
//! Handles `[[ ... ]]` compound commands and the `test` / `[` builtins,
//! including pattern matching, regex matching, file tests, and numeric
//! comparisons.

use std::collections::BTreeMap;

use super::{
    is_marked_var, is_shell_name_char, is_shell_name_start, mark_env_name,
    parse_helpers::decode_ansi_c_escapes, ExecuteError, Executor, ARRAY_VARS, NAMEREF_VARS,
};
use crate::executor::arithmetic::{
    assoc_skip_substitution, assoc_subscript_end, eval_mutable_arith_value_with_random_flags,
};
use crate::executor::arrays::{format_indexed_array_storage, sync_typed_indexed_array};
use crate::parser::QuoteKind;

mod args;
mod extglob;
pub(crate) mod pattern;

use args::{
    conditional_effective_len, conditional_logical_index, conditional_outer_parentheses,
    conditional_pattern_or_string_matches, conditional_regex_operands, is_conditional_file_binary,
    is_conditional_file_unary, is_conditional_numeric_op, needs_extglob_reassembly,
    reassemble_extglob_args, restore_numeric_decimal_regex_escapes,
};

pub(super) use args::simple_grep_pattern_matches;
pub(crate) use extglob::extglob_case_pattern_matches;
#[allow(unused_imports)]
pub(in crate::executor) use extglob::extglob_case_pattern_matches_nocase;
pub(in crate::executor) use extglob::extglob_case_pattern_matches_nocase_with_word_chars;
pub(crate) use extglob::extglob_case_pattern_matches_with_word_chars;
pub(in crate::executor) use pattern::{
    case_bracket_expression_matches_with_case, case_pattern_matches, case_pattern_matches_nocase,
    case_pattern_matches_nocase_with_word_chars, case_pattern_matches_with_word_chars,
};

pub(crate) fn shell_pattern_matches(pattern: &str, word: &str) -> bool {
    case_pattern_matches(pattern, word)
}

impl Executor {
    pub(super) fn execute_conditional_command(
        &mut self,
        command: &crate::parser::ConditionalCommand,
    ) -> i32 {
        // rubash#1915: publish the procsub spans the parser recorded
        // for this conditional's operands so the nounset scan points
        // below mask exactly those spans before judging a word (see
        // Executor::conditional_procsub_span_targets). Save/restore:
        // a conditional evaluated inside an operand's command
        // substitution on this same executor (eval/source frames)
        // nests inside the outer conditional's table.
        let span_targets: Vec<(String, Vec<String>)> = command
            .args
            .iter()
            .zip(&command.arg_metadata)
            .filter_map(|(arg, metadata)| {
                if metadata.process_substitutions.is_empty() {
                    None
                } else {
                    Some((
                        arg.clone(),
                        metadata
                            .process_substitutions
                            .iter()
                            .map(|substitution| substitution.target.clone())
                            .collect(),
                    ))
                }
            })
            .collect();
        let saved_span_targets =
            std::mem::replace(&mut self.conditional_procsub_span_targets, span_targets);
        let status = if let Some(status) =
            self.conditional_status_with_metadata(&command.args, &command.arg_metadata)
        {
            status
        } else {
            self.execute_conditional(&command.args)
        };
        self.conditional_procsub_span_targets = saved_span_targets;
        status
    }

    /// rubash#1915: mask the process-substitution spans the parser
    /// recorded for THIS conditional word out of the text a `[[ ]]`
    /// nounset scan is about to judge. The lookup is by the word's
    /// exact operand text, and the spans come from that word's own
    /// parse record: a `<(...)`-shaped string inside a quoted sibling
    /// operand was never recorded for it, so its `$` remains a live
    /// parent reference and the scan still reports it. Words with no
    /// recorded span (including derived strings the evaluator builds)
    /// scan exactly as before.
    fn mask_conditional_procsub_spans<'a>(&self, word: &'a str) -> std::borrow::Cow<'a, str> {
        let Some((_, targets)) = self
            .conditional_procsub_span_targets
            .iter()
            .find(|(arg, _)| arg == word)
        else {
            return std::borrow::Cow::Borrowed(word);
        };
        Self::mask_recorded_span_targets(word, targets.iter().map(String::as_str))
    }

    pub(super) fn execute_conditional(&mut self, args: &[String]) -> i32 {
        // TODO(parse.y/execute_cmd.c/test.c): Bash `[[` is a compound command
        // with its own parser, operators, pattern matching, and short-circuit
        // logic. Keep extending this bridge with test.c-compatible primitives.
        // Reassembly only fires when a word ending in an extglob operator is
        // followed by a bare `(` token; when no such pair exists the whole-Vec
        // clone is a pure copy, so borrow the original slice instead.
        let reassembled;
        let args = if needs_extglob_reassembly(args) {
            reassembled = reassemble_extglob_args(args);
            reassembled.as_slice()
        } else {
            args
        };
        if let Some(inner) = conditional_outer_parentheses(args) {
            return self.execute_conditional(inner);
        }

        if let Some(index) = conditional_logical_index(args, "||") {
            let left = self.execute_conditional(&args[..index]);
            return if left == 0 {
                0
            } else {
                self.execute_conditional(&args[index + 1..])
            };
        }
        if let Some(index) = conditional_logical_index(args, "&&") {
            let left = self.execute_conditional(&args[..index]);
            return if left == 0 {
                self.execute_conditional(&args[index + 1..])
            } else {
                1
            };
        }

        if let Some((left, right)) = conditional_regex_operands(args) {
            return self.conditional_regex_match_status(left, &right);
        }

        match args {
            // GNU parse.y:5123-5124 cond_term: `!` XORs CMD_INVERT_RETURN
            // onto the following term — `[[ ! -n "" ]]` traces `! -n ''`.
            // A `!` in front of `( ... )` inverts the group node instead,
            // so the inner leaf terms trace without `!`.
            [not, rest @ ..] if not == "!" => {
                if conditional_outer_parentheses(rest).is_none() {
                    let saved = self
                        .conditional_invert_pending
                        .replace(!self.conditional_invert_pending.get());
                    let status = self.execute_conditional(rest);
                    self.conditional_invert_pending.set(saved);
                    i32::from(status == 0)
                } else {
                    i32::from(self.execute_conditional(rest) == 0)
                }
            }
            // GNU cond.c `[[ -v name[sub] ]]` -> test.c's -v machinery: the
            // subscript of the already-expanded operand is evaluated under
            // no-expand rules; a surviving $name/$(...) is "operand
            // expected" and the evalerror discards the rest of the command
            // list (rewrite_conditional_v_operand raises it).
            [op, operand, end] if op == "-v" && end == "]]" => {
                // GNU cond_expand_word(op, 3) keeps the subscript text —
                // the operand as written is the right trace rendering.
                self.xtrace_conditional_term("-v", operand, None);
                match self.conditional_dash_v(operand, None) {
                    Ok(set) => i32::from(!set),
                    Err(()) => 1,
                }
            }
            [op, operand] if op == "-v" => {
                self.xtrace_conditional_term("-v", operand, None);
                match self.conditional_dash_v(operand, None) {
                    Ok(set) => i32::from(!set),
                    Err(()) => 1,
                }
            }
            [op, operand, end] if op == "-R" && end == "]]" => {
                let name = self.expand_conditional_word(operand);
                self.xtrace_conditional_term("-R", &name, None);
                i32::from(!is_marked_var(
                    &self.shell_state.env_vars,
                    NAMEREF_VARS,
                    &name,
                ))
            }
            [op, operand] if op == "-R" => {
                let name = self.expand_conditional_word(operand);
                self.xtrace_conditional_term("-R", &name, None);
                i32::from(!is_marked_var(
                    &self.shell_state.env_vars,
                    NAMEREF_VARS,
                    &name,
                ))
            }
            [op, operand, end] if op == "-o" && end == "]]" => {
                i32::from(!self.conditional_shell_option_unary(operand))
            }
            [op, operand] if op == "-o" => i32::from(!self.conditional_shell_option_unary(operand)),
            [op, operand, end] if matches!(op.as_str(), "-n" | "-z") && end == "]]" => {
                i32::from(!self.conditional_string_unary(op, operand))
            }
            [op, operand] if matches!(op.as_str(), "-n" | "-z") => {
                i32::from(!self.conditional_string_unary(op, operand))
            }
            // `[[ x ]]` is `[[ -n x ]]` (parse.y:5174-5178 synthesizes the
            // -n node), so the trace prints `-n <arg>`.
            [operand, end] if end == "]]" => {
                let value = self.expand_conditional_word(operand);
                self.xtrace_conditional_term("-n", &value, None);
                i32::from(value.is_empty())
            }
            [operand] => {
                let value = self.expand_conditional_word(operand);
                self.xtrace_conditional_term("-n", &value, None);
                i32::from(value.is_empty())
            }
            // GNU xtrace runs between cond_expand_word and cond_test
            // (execute_cmd.c:4024): `+ [[ -t X ]]` precedes the
            // `X: integer expected` diagnostic. Expand once, trace, then
            // validate and dispatch on the already-expanded operand.
            [op, operand, end] if op == "-t" && end == "]]" => {
                let w = self.expand_conditional_word(operand);
                self.xtrace_conditional_term(op, &w, None);
                if crate::builtins::test::valid_number(&w).is_none() {
                    self.report_conditional_error(&format!("{}: integer expected", w));
                    return 2;
                }
                let args = vec![op.to_string(), w];
                self.sync_fd_terminal_marks(None);
                i32::from(
                    !(crate::builtins::test::execute(&args, false, &self.shell_state.env_vars)
                        .unwrap_or(1)
                        == 0),
                )
            }
            [op, operand] if op == "-t" => {
                let w = self.expand_conditional_word(operand);
                self.xtrace_conditional_term(op, &w, None);
                if crate::builtins::test::valid_number(&w).is_none() {
                    self.report_conditional_error(&format!("{}: integer expected", w));
                    return 2;
                }
                let args = vec![op.to_string(), w];
                self.sync_fd_terminal_marks(None);
                i32::from(
                    !(crate::builtins::test::execute(&args, false, &self.shell_state.env_vars)
                        .unwrap_or(1)
                        == 0),
                )
            }
            [op, operand, end] if is_conditional_file_unary(op) && end == "]]" => {
                i32::from(!self.conditional_file_unary(op, operand))
            }
            [op, operand] if is_conditional_file_unary(op) => {
                i32::from(!self.conditional_file_unary(op, operand))
            }
            [left, op, right, end]
                if matches!(op.as_str(), "=" | "==" | "!=" | "=~" | "<" | ">") && end == "]]" =>
            {
                if op == "=~" {
                    return self.conditional_regex_match_status(left, right);
                }
                i32::from(!self.conditional_string_binary(left, op, right))
            }
            [left, op, right] if matches!(op.as_str(), "=" | "==" | "!=" | "=~" | "<" | ">") => {
                if op == "=~" {
                    return self.conditional_regex_match_status(left, right);
                }
                i32::from(!self.conditional_string_binary(left, op, right))
            }
            [left, op, right, end] if is_conditional_numeric_op(op) && end == "]]" => {
                self.conditional_numeric_binary_status(left, op, right)
            }
            [left, op, right] if is_conditional_numeric_op(op) => {
                self.conditional_numeric_binary_status(left, op, right)
            }
            [left, op, right, end] if is_conditional_file_binary(op) && end == "]]" => {
                i32::from(!self.conditional_file_binary(left, op, right))
            }
            [left, op, right] if is_conditional_file_binary(op) => {
                i32::from(!self.conditional_file_binary(left, op, right))
            }
            _ => 1,
        }
    }

    /// GNU cond.c cond_expand_word -> expand_word: the operand reaches the
    /// test with quote removal complete — a quoted-empty alternate
    /// expansion (`[[ -n ${a[@]+"${a[2]+x}"} ]]`, rubash#315) is the empty
    /// string, not the argv splitter's QUOTED_NULL_MARKER carrier. Strip
    /// the in-band carrier the way remove_quoted_nulls drops CTLNUL at
    /// dequote (subst.c:4795+); no [[ ]] context consumes the field-
    /// splitting semantics it encodes.
    fn expand_conditional_word(&mut self, word: &str) -> String {
        // rubash#1422: an earlier operand already hit the DISCARD-class
        // error below; GNU abandons the conditional at the first bad
        // word, so later operands are never expanded at all.
        if self.conditional_expansion_error_status.get().is_some() {
            return String::new();
        }
        // GNU cond.c cond_expand_word -> expand_word: a nounset unbound
        // variable (or another FATAL expansion error) in a [[ ]]
        // operand raises FORCE_EOF, exactly as in a simple command's
        // words (rubash#1130). Expansion happens per term, so
        // short-circuited branches never reach this scan. The evaluator
        // returns only i32, so record the fatal for
        // execute_conditional_command_with_redirects to unwind.
        if !self.conditional_expansion_fatal.get() {
            // rubash#1915: the scan judges the parent word, never a
            // recorded procsub body — mask this word's recorded spans
            // (the expansion below still consumes the original word).
            let masked = self.mask_conditional_procsub_spans(word);
            if let Some((name, message, status)) = self.parameter_expansion_error_in_word(&masked) {
                if status == 1 {
                    // GNU subst.c: a non-fatal word-expansion error
                    // (substring expression < 0, bad substitution) is
                    // an expand_wdesc_error -> DISCARD inside [[ ]]
                    // too: print the diagnostic, fail the conditional,
                    // and abandon the rest of the current command
                    // list, exactly as the simple-command path does
                    // (rubash#1422). Record the status; the unwind
                    // happens in
                    // execute_conditional_command_with_redirects.
                    let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
                    let _ = self.write_diagnostic_fd2(line.as_bytes());
                    self.exit_code = status;
                    // rubash#439: expansion error, not a failed command.
                    self.command_expansion_failed.set(true);
                    self.conditional_expansion_error_status.set(Some(status));
                    return String::new();
                }
                if status == Self::FATAL_PARAMETER_EXPANSION_STATUS {
                    let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
                    let _ = self.write_diagnostic_fd2(line.as_bytes());
                    self.exit_code = if self.expansion_error_is_interactive_discard() {
                        1
                    } else {
                        self.expansion_fatal_status()
                    };
                    // rubash#1201: park the fatal status — the conditional
                    // wrapper overwrites exit_code with the predicate result
                    // (0 for `-z` of the failed expansion) before it reads
                    // the fatal flag, which lost the status entirely.
                    self.conditional_fatal_status.set(Some(self.exit_code));
                    // rubash#439: expansion error, not a failed command.
                    self.command_expansion_failed.set(true);
                    self.conditional_expansion_fatal.set(true);
                    return String::new();
                }
            }
        }
        // A conditional word is a comparison VALUE: drop the
        // ANSI_C_IFS_GUARD pairs (decoded `$'...'` blanks) so both
        // operands compare as their real text (rubash#379).
        // E400-aware: user U+E401 data (E400 E401) must survive.
        crate::executor::markers::strip_ansi_c_ifs_guard(
            // E002 is registry-zone: a typed `$'\uE002'` arrives
            // E400-escaped and is user data — strip only the BARE
            // carrier (rubash#840).
            &crate::executor::markers::restore_bare_sentinels(
                &self.expand_word_mut(word),
                &[(crate::executor::embedded_mutations::QUOTED_NULL_MARKER, "")],
            ),
        )
    }

    fn conditional_status_with_metadata(
        &mut self,
        args: &[String],
        metadata: &[crate::parser::WordMetadata],
    ) -> Option<i32> {
        if args.len() != metadata.len() {
            return None;
        }

        // Bash rejects an empty conditional (`[[ ]]`) and a conditional that
        // starts with `)` (`[[ ) ]]`) as syntax errors. Rubash has no parser
        // error channel yet, so evaluate them as false instead of letting
        // the fallback treat them as a truthy word.
        if args.is_empty() || args[0] == ")" || (args.len() == 1 && args[0] == "]]") {
            return Some(1);
        }

        if conditional_outer_parentheses(args).is_some() {
            let end = conditional_effective_len(args);
            return self.conditional_status_with_metadata(&args[1..end - 1], &metadata[1..end - 1]);
        }

        if let Some(index) = conditional_logical_index(args, "||") {
            let left = self
                .conditional_status_with_metadata(&args[..index], &metadata[..index])
                .unwrap_or_else(|| self.execute_conditional(&args[..index]));
            return Some(if left == 0 {
                0
            } else {
                self.conditional_status_with_metadata(&args[index + 1..], &metadata[index + 1..])
                    .unwrap_or_else(|| self.execute_conditional(&args[index + 1..]))
            });
        }

        if let Some(index) = conditional_logical_index(args, "&&") {
            let left = self
                .conditional_status_with_metadata(&args[..index], &metadata[..index])
                .unwrap_or_else(|| self.execute_conditional(&args[..index]));
            return Some(if left == 0 {
                self.conditional_status_with_metadata(&args[index + 1..], &metadata[index + 1..])
                    .unwrap_or_else(|| self.execute_conditional(&args[index + 1..]))
            } else {
                1
            });
        }

        if let [not, rest @ ..] = args {
            // GNU parse.y:3168-3181 CHECK_FOR_RESERVED_WORD recognizes
            // the negation word from the RAW token text: a quoted `'!'`
            // (raw != "!") is an ordinary WORD operand, never BANG
            // (rubash#1706). Only the bare spelling negates.
            if not == "!" && metadata.first().is_some_and(|entry| entry.raw == "!") {
                // Same invert-pending bookkeeping as execute_conditional:
                // `!` marks the next leaf term unless it negates a
                // parenthesized group (GNU parse.y CMD_INVERT_RETURN).
                let saved = if conditional_outer_parentheses(rest).is_none() {
                    Some(
                        self.conditional_invert_pending
                            .replace(!self.conditional_invert_pending.get()),
                    )
                } else {
                    None
                };
                let status = self
                    .conditional_status_with_metadata(rest, &metadata[1..])
                    .unwrap_or_else(|| self.execute_conditional(rest));
                if let Some(saved) = saved {
                    self.conditional_invert_pending.set(saved);
                }
                return Some(i32::from(status == 0));
            }
        }

        // `[[ -v name[sub] ]]`: TEST_ARRAYEXP needs the raw operand token —
        // the cooked args collapse `'name[$k]'` and `name[\$k]` to the same
        // carrier text, but GNU's flag-1 valid_array_reference sees the real
        // quotes (execute_cmd.c:4015-4027).
        if let [op, operand, ..] = args {
            if op == "-v" && (args.len() == 2 || args.get(2).is_some_and(|end| end == "]]")) {
                let raw = metadata.get(1).map(|entry| entry.raw.as_str());
                self.xtrace_conditional_term("-v", operand, None);
                let status = match self.conditional_dash_v(operand, raw) {
                    Ok(set) => i32::from(!set),
                    Err(()) => 1,
                };
                return Some(status);
            }
        }

        // Arithmetic-comparison leaf (`[[ a -op b ]]`): the operands are
        // arithmetic values, not patterns, so the quoted-pattern pass below
        // can never fire for these operators. Handle it here instead of
        // falling through to `execute_conditional`, which would re-run
        // `reassemble_extglob_args` plus the paren/logical scans a second
        // time. The token shape must already match: a `(` token would change
        // it and falls through to the full path (where the extglob merge
        // happens), so this is admission, not a semantic shortcut. rubash#117:
        // the value work stays in `conditional_numeric_binary_status`.
        if let [left, op, right, end] = args {
            if end == "]]" && is_conditional_numeric_op(op) {
                return Some(self.conditional_numeric_binary_status(left, op, right));
            }
        }
        if let [left, op, right] = args {
            if is_conditional_numeric_op(op) {
                return Some(self.conditional_numeric_binary_status(left, op, right));
            }
        }

        // rubash#1706: a quoted `'!'` in the left-operand slot is data
        // (see the negation gate above), but the cooked-args evaluator
        // this function falls back to cannot tell it apart from the
        // negation operator and would invert a mangled primary, so the
        // binary primaries it heads are evaluated here, where the raw
        // spelling is still available. ==/!=/=~ with a quoted RHS stay
        // with quoted_conditional_pattern_status below (its literal /
        // quoted-regex handling owns those spellings).
        if args.first().map(String::as_str) == Some("!")
            && metadata.first().is_some_and(|entry| entry.raw != "!")
        {
            let operands = match args {
                [left, op, right, end] if end == "]]" => Some((left, op, right)),
                [left, op, right] => Some((left, op, right)),
                _ => None,
            };
            if let Some((left, op, right)) = operands {
                let rhs_quoted = metadata.get(2).is_some_and(|entry| {
                    Self::conditional_rhs_word_is_quoted(&entry.raw) || entry.raw.contains('\\')
                });
                let status = match op.as_str() {
                    "=" | "==" | "!=" if !rhs_quoted => {
                        Some(i32::from(!self.conditional_string_binary(left, op, right)))
                    }
                    "<" | ">" => Some(i32::from(!self.conditional_string_binary(left, op, right))),
                    "=~" if !rhs_quoted => Some(self.conditional_regex_match_status(left, right)),
                    _ if is_conditional_file_binary(op) => {
                        Some(i32::from(!self.conditional_file_binary(left, op, right)))
                    }
                    _ => None,
                };
                if status.is_some() {
                    return status;
                }
            }
            // The single-word shape (`[[ '!' ]]`) is a plain -n test of
            // the word (parse.y:5174-5178); left to the cooked fallback
            // it would be read as a negation with an empty term.
            let single = match args {
                [word] => Some(word),
                [word, end] if end == "]]" => Some(word),
                _ => None,
            };
            if let Some(word) = single {
                let value = self.expand_conditional_word(word);
                self.xtrace_conditional_term("-n", &value, None);
                return Some(i32::from(value.is_empty()));
            }
        }

        self.quoted_conditional_pattern_status(args, metadata)
    }

    /// Whether a conditional RHS operand word carries quote spans (GNU
    /// parse.y W_QUOTED on the `=~`/`==` RHS decides regex-vs-literal and
    /// pattern-vs-literal semantics). wt44/parse4: the parse-time word-quote
    /// scan is gone (GNU runs no per-word analyses at parse; subst.c/cond
    /// consult quoting at use), so the check re-derives the spans from the
    /// raw spelling on demand — the same pure function of the same input.
    fn conditional_rhs_word_is_quoted(raw: &str) -> bool {
        !crate::parser::word_quotes_in_raw_public(raw).is_empty()
    }

    fn quoted_conditional_pattern_status(
        &mut self,
        args: &[String],
        metadata: &[crate::parser::WordMetadata],
    ) -> Option<i32> {
        match args {
            [left, op, right, end]
                if end == "]]"
                    && matches!(op.as_str(), "=" | "==" | "!=")
                    && metadata.get(2).is_some_and(|m| {
                        Self::conditional_rhs_word_is_quoted(&m.raw) || m.raw.contains('\\')
                    }) =>
            {
                let left = self.expand_conditional_word(left);
                let right = self.expand_conditional_word(right);
                self.xtrace_conditional_term(op, &left, Some(&right));
                let right_pattern = self.quote_aware_glob_rhs(&right, &metadata[2]);
                let matched =
                    crate::executor::conditional::case_pattern_matches(&right_pattern, &left);
                Some(match op.as_str() {
                    "!=" => i32::from(matched),
                    _ => i32::from(!matched),
                })
            }
            [left, op, right]
                if matches!(op.as_str(), "=" | "==" | "!=")
                    && metadata.get(2).is_some_and(|m| {
                        Self::conditional_rhs_word_is_quoted(&m.raw) || m.raw.contains('\\')
                    }) =>
            {
                let left = self.expand_conditional_word(left);
                let right = self.expand_conditional_word(right);
                self.xtrace_conditional_term(op, &left, Some(&right));
                let right_pattern = self.quote_aware_glob_rhs(&right, &metadata[2]);
                let matched =
                    crate::executor::conditional::case_pattern_matches(&right_pattern, &left);
                Some(match op.as_str() {
                    "!=" => i32::from(matched),
                    _ => i32::from(!matched),
                })
            }
            [left, op, right, end]
                if end == "]]"
                    && op == "=~"
                    && metadata.get(2).is_some_and(|m| {
                        Self::conditional_rhs_word_is_quoted(&m.raw) || m.raw.contains('\\')
                    }) =>
            {
                Some(self.conditional_quoted_regex_match_status(left, right, &metadata[2]))
            }
            [left, op, right]
                if op == "=~"
                    && metadata.get(2).is_some_and(|m| {
                        Self::conditional_rhs_word_is_quoted(&m.raw) || m.raw.contains('\\')
                    }) =>
            {
                Some(self.conditional_quoted_regex_match_status(left, right, &metadata[2]))
            }
            _ => None,
        }
    }

    pub(super) fn conditional_string_binary(&mut self, left: &str, op: &str, right: &str) -> bool {
        let left = self.expand_conditional_word(left);
        // For the pattern operators the RHS backslashes are glob escape
        // syntax, not quote-removal subjects (GNU cond.c: the pattern
        // reaches patmatch with its backslashes intact — `a\\b` matches
        // the literal `a\b`). The lexer's DATA_BACKSLASH carrier stands
        // for ONE escaped backslash in the source; the pattern matcher
        // needs it doubled so its own escape arm reads it as a literal
        // backslash instead of an escape of the following character
        // (rubash#1092). The metadata path (quote_aware_glob_rhs) handles
        // the raw spelling directly; this is the no-metadata fallback.
        let right = if matches!(op, "=" | "==" | "!=") {
            let doubled = right.replace(
                crate::executor::markers::DATA_BACKSLASH,
                &crate::executor::markers::DATA_BACKSLASH
                    .to_string()
                    .repeat(2),
            );
            self.expand_conditional_word(&doubled)
        } else {
            self.expand_conditional_word(right)
        };
        self.xtrace_conditional_term(op, &left, Some(&right));
        let right_pattern = right.clone();
        let extglob = crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extglob")
            || contains_extglob_pattern(&right);
        let nocasematch =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "nocasematch");
        match op {
            "=" | "==" if extglob && nocasematch => {
                extglob_case_pattern_matches_nocase(&right_pattern, &left)
            }
            "=" | "==" if extglob => extglob_case_pattern_matches(&right_pattern, &left),
            "=" | "==" => conditional_pattern_or_string_matches(&left, &right_pattern, nocasematch),
            "!=" if extglob && nocasematch => {
                !extglob_case_pattern_matches_nocase(&right_pattern, &left)
            }
            "!=" if extglob => !extglob_case_pattern_matches(&right_pattern, &left),
            "!=" => !conditional_pattern_or_string_matches(&left, &right_pattern, nocasematch),
            "=~" => self.conditional_regex_match(&left, &right),
            // GNU test.c:404-425 binary_test, reached from
            // execute_cmd.c:4110 which always passes TEST_LOCALE for
            // `[[ ]]`: string `<`/`>` compare with strcoll(3) when
            // posixly_correct or the shell compatibility level is above
            // 40, and with strcmp otherwise. (The `test`/`[` builtin
            // reaches binary_test WITHOUT TEST_LOCALE — test.c:472 — and
            // is implemented separately in builtins/test.rs, so it keeps
            // byte order; do not route it through here.)
            "<" => self.conditional_string_less(&left, &right),
            ">" => self.conditional_string_less(&right, &left),
            _ => false,
        }
    }

    /// The `<` half of the `[[ ]]` string-order comparison (see the
    /// binary_test citation at the call site): strcoll under the active
    /// LC_COLLATE when the GNU gate is open, plain byte order otherwise.
    /// `crate::locale::strcoll` itself falls back to byte order under the
    /// C/POSIX collation family and for unresolvable locales, so the C
    /// locale stays byte-identical to strcmp.
    fn conditional_string_less(&self, a: &str, b: &str) -> bool {
        let locale_compare =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "posix")
                || crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) > 40;
        if locale_compare {
            crate::locale::strcoll(a, b) == std::cmp::Ordering::Less
        } else {
            a < b
        }
    }

    /// Build the `[[ ... == ... ]]` glob while retaining quote boundaries.
    /// Unquoted segments keep glob operators; quoted segments contribute
    /// literal text (including a quoted parameter's expanded value).
    fn quote_aware_glob_rhs(
        &mut self,
        expanded_right: &str,
        metadata: &crate::parser::WordMetadata,
    ) -> String {
        if metadata.raw.is_empty() {
            return expanded_right.to_string();
        }

        let chars: Vec<char> = metadata.raw.chars().collect();
        let mut output = String::new();
        let mut index = 0;
        let mut unquoted_start = 0;

        while index < chars.len() {
            // Nested expansion bodies have their own quote state (GNU
            // parse.y xparse_dolparen / parse_matched_pair): a `\` inside
            // `$(...)`/`${...}`/`` `...` `` quotes a character of the nested
            // body, not the outer word. Keep the span inside the unquoted
            // run so expand_word_mut handles its inner escapes.
            if let Some(next) =
                crate::parser::pathname_pattern::skip_nested_expansion(&chars, index)
            {
                index = next;
                continue;
            }
            if chars[index] == '\\' {
                if unquoted_start < index {
                    output.push_str(&self.expand_conditional_word(
                        &chars[unquoted_start..index].iter().collect::<String>(),
                    ));
                }
                if let Some(next) = chars.get(index + 1) {
                    output.push('\\');
                    output.push(*next);
                    index += 2;
                } else {
                    index += 1;
                }
                unquoted_start = index;
                continue;
            }

            let Some((kind, opener_len, end)) = raw_quote_at(&chars, index) else {
                index += 1;
                continue;
            };
            if unquoted_start < index {
                output.push_str(&self.expand_conditional_word(
                    &chars[unquoted_start..index].iter().collect::<String>(),
                ));
            }
            let body = chars[index + opener_len..end].iter().collect::<String>();
            let value = match kind {
                QuoteKind::Single | QuoteKind::Backslash => body,
                QuoteKind::AnsiC => decode_ansi_c_escapes(&body),
                QuoteKind::Double | QuoteKind::Locale => {
                    // parse.y read_token_word's double-quote scanner never
                    // opens single-quote state, so a `'` inside the body is
                    // literal data (GNU cond_expand_word keeps it). Feeding
                    // a bare `'` to expand_word_mut makes quote removal
                    // treat it as an unclosed single-quote span and drop
                    // it, collapsing `[[ "''" == "''" ]]`'s pattern to the
                    // empty string. Tag it with the lexer's decoded-quote
                    // marker (the same protection escape_decoded_ansi_c_
                    // quotes applies) so it survives as data.
                    let body = body.replace('\'', crate::lexer::ANSI_C_QUOTE_MARKER_STR);
                    self.expand_conditional_word(&body)
                }
            };
            append_literal_glob_text(&mut output, &value);
            index = end + 1;
            unquoted_start = index;
        }

        if unquoted_start < chars.len() {
            output.push_str(
                &self.expand_conditional_word(&chars[unquoted_start..].iter().collect::<String>()),
            );
        }
        output
    }

    /// GNU `[[ -v name[sub] ]]` (execute_cmd.c:4008-4031): `varflag` runs
    /// `valid_array_reference(raw_word, VA_NOEXPAND)` — flag-1 — on the RAW
    /// operand token, so `'name[$k]'` is NOT TEST_ARRAYEXP (the `[` sits
    /// inside quotes and `'name` fails the name check) while `name[$k]` is.
    /// `raw` is the parser's raw operand token; nested `execute_conditional`
    /// fallback callers pass None and the check runs on the quote-carrier
    /// operand text instead. Err(()) means the diagnostic was already
    /// printed (evalerror abort).
    fn conditional_dash_v(&mut self, operand: &str, raw: Option<&str>) -> Result<bool, ()> {
        let arrayref = crate::executor::subscript_expansion::valid_array_reference_env(
            raw.unwrap_or(operand),
            true,
            false,
            &self.shell_state.env_vars,
        );
        let cooked = self.expand_conditional_word(operand);
        let rewritten = self.rewrite_conditional_v_operand(&cooked, arrayref, raw)?;

        Ok(crate::builtins::test::variable_is_set(
            &rewritten,
            &self.shell_state.env_vars,
        ))
    }

    pub(super) fn conditional_string_unary(&mut self, op: &str, operand: &str) -> bool {
        let value = self.expand_conditional_word(operand);
        self.xtrace_conditional_term(op, &value, None);
        match op {
            "-n" => !value.is_empty(),
            "-z" => value.is_empty(),
            _ => false,
        }
    }

    /// GNU print_cmd.c:956 xtrace_print_cond_term — one `+ [[ ... ]]`
    /// line per evaluated leaf term, with the cond_expand_word'd operands
    /// (empty prints `''`, a pending `!` inversion prints `! `). Emitted
    /// on fd 2 after the command's redirections are bound
    /// (execute_cmd.c:4024/4075 run after do_redirections).
    fn xtrace_conditional_term(&mut self, op: &str, arg1: &str, arg2: Option<&str>) {
        let invert = self.conditional_invert_pending.replace(false);
        if !self.xtrace_enabled() {
            return;
        }
        let prefix = self.xtrace_prefix();
        let invert_mark = if invert { "! " } else { "" };
        fn quote(value: &str) -> &str {
            if value.is_empty() {
                "''"
            } else {
                value
            }
        }
        let line = match arg2 {
            None => format!("{prefix}[[ {invert_mark}{op} {} ]]\n", quote(arg1)),
            Some(right) => format!(
                "{prefix}[[ {invert_mark}{} {op} {} ]]\n",
                quote(arg1),
                quote(right)
            ),
        };
        self.xtrace_write(line.as_bytes());
    }

    /// Report a `[[ ]]` syntax error to stderr using the GNU-style prefix
    /// (`<script>: line N: [[: <message>`), matching `test_syntax_error`
    /// in test.c as called from `cond_test`.
    fn report_conditional_error(&mut self, message: &str) {
        let prefix = if let (Some(script), Some(line)) = (
            self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME"),
            self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE"),
        ) {
            format!("{script}: line {line}: [[: ")
        } else {
            "rubash: [[: ".to_string()
        };
        // Diagnostics go through the shell's fd-2 channel so `[[ ]] 2>f`
        // captures them (GNU prints via internal_error -> stderr, which is
        // the redirected stream inside execute_cond_command).
        let _ = self.write_default_stderr(format!("{prefix}{message}\n").as_bytes());
    }

    pub(super) fn conditional_shell_option_unary(&mut self, operand: &str) -> bool {
        let name = self.expand_conditional_word(operand);
        self.xtrace_conditional_term("-o", &name, None);
        crate::builtins::set::is_shell_option(&name)
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, &name)
    }

    pub(super) fn conditional_file_unary(&mut self, op: &str, operand: &str) -> bool {
        if let Some(result) = self.conditional_process_substitution_unary(op, operand) {
            return result;
        }
        let value = self.expand_conditional_word(operand);
        self.xtrace_conditional_term(op, &value, None);
        let args = vec![op.to_string(), value];
        crate::builtins::test::execute(&args, false, &self.shell_state.env_vars).unwrap_or(1) == 0
    }

    /// rubash#545 (cell 3): GNU realizes a `[[ ]]` process-substitution
    /// operand exactly like an argument — subst.c runs the body, the word
    /// becomes `/dev/fd/N`, and xtrace_print_cond_term prints the REALIZED
    /// path (`+ [[ -r /dev/fd/63 ]]`, the body's own `++ ` trace arriving
    /// from the substitution process). The old static matrix answered
    /// without running the body at all, so no trace line and no `++`
    /// firing ever appeared. The verdict matrix itself is unchanged (GNU's
    /// answers for a LIVE substitution: an input `/dev/fd/N` is `-r`/`-e`/
    /// `-a`/`-p`, an output one is `-w`/`-e`/`-a`/`-p`).
    fn conditional_process_substitution_unary(&mut self, op: &str, operand: &str) -> Option<bool> {
        let input = operand
            .strip_prefix("<(")
            .and_then(|operand| operand.strip_suffix(')'))
            .map(|source| (false, source));
        let output = operand
            .strip_prefix(">(")
            .and_then(|operand| operand.strip_suffix(')'))
            .map(|source| (true, source));
        let (is_output, source) = output.or(input)?;
        let _ = is_output;
        let word = if is_output {
            let path = self.empty_process_substitution_temp().ok()?;
            let word = self.process_substitution_output_fd_word(&path);
            // rubash#1184 f4: the body runs at finish in a spawned child;
            // capture the constructing `[[` command's line now (the
            // CURRENT_LINE discipline the in-process body parse seeds
            // from, rubash#1024) so the child's diagnostics attribute to
            // it instead of line 1.
            let line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0);
            self.conditional_output_procsubs
                .push((path, source.to_string(), line));
            word
        } else {
            let output_bytes = self.process_substitution_output_bytes(source)?;
            self.process_substitution_fd_word(&output_bytes)
        };
        self.xtrace_conditional_term(op, &word, None);
        // WSL GNU 5.3.0(1) matrix (script-file probes, 2026-10-12 — the
        // ad-hoc nested-quoting channel gives polluted answers there, the
        // file channel is authoritative): a LIVE /dev/fd/N operand answers
        // TRUE for -e -p -r -w -L -h -O -G on BOTH substitution sides (the
        // fd is a readable-writable end of a pipe reached through a magic
        // symlink owned by this user), and FALSE for -f -d -x -s -b -c -S
        // -u -g -k -N (probe3/probe5: `-f <(:)`=1, `-s <(printf hello)`=1,
        // `-L <(:)`=0, `-r >(:)`=0).
        Some(match op {
            "-a" | "-e" | "-p" | "-r" | "-w" | "-L" | "-h" | "-O" | "-G" => true,
            "-f" | "-d" | "-x" | "-s" | "-S" | "-b" | "-c" | "-u" | "-g" | "-k" | "-N" => false,
            _ => return None,
        })
    }

    pub(super) fn conditional_file_binary(&mut self, left: &str, op: &str, right: &str) -> bool {
        let left_exp = self.expand_conditional_word(left);
        let right_exp = self.expand_conditional_word(right);
        self.xtrace_conditional_term(op, &left_exp, Some(&right_exp));
        let args = vec![left_exp, op.to_string(), right_exp];
        crate::builtins::test::execute(&args, false, &self.shell_state.env_vars).unwrap_or(1) == 0
    }

    pub(super) fn conditional_regex_match(&mut self, left: &str, right: &str) -> bool {
        // Both operands are byte strings in GNU (sh_regmatch regexec's the
        // expanded text); decode the comsub payload spelling on the subject
        // side too, symmetric with the pattern decode inside
        // compile_conditional_regex.
        let left = crate::executor::execution_misc::decode_command_substitution_payload(left);
        let right = restore_numeric_decimal_regex_escapes(right);
        let Ok(regex) = self.compile_conditional_regex(&right) else {
            return false;
        };
        let Some(captures) = regex.captures(&left) else {
            self.clear_bash_rematch();
            return false;
        };

        self.store_bash_rematch(captures);
        true
    }

    pub(super) fn store_bash_rematch(&mut self, captures: Vec<Option<String>>) {
        // GNU execute_cmd.c:4144-4150 (sh_regmatch -> store: every group in
        // 0..nmatch gets an element): a group that did not participate is
        // bound to the EMPTY string, not left unset — `${#BASH_REMATCH[@]}`
        // counts it and `${BASH_REMATCH[i]+x}` reports it set (rubash#1059,
        // optional groups and the untaken side of an alternation alike).
        //
        // rubash#1365: if BASH_REMATCH is readonly, GNU refuses the store
        // (variables.c: bind_variable), emits "BASH_REMATCH: readonly
        // variable" on stderr, and keeps the stale value. The match itself
        // still succeeds.
        if self
            .shell_state
            .env_vars
            .is_marked(crate::executor::READONLY_VARS, "BASH_REMATCH")
        {
            eprintln!("{}BASH_REMATCH: readonly variable", self.error_prefix());
            return;
        }
        let entries: BTreeMap<usize, String> = captures
            .into_iter()
            .enumerate()
            .map(|(index, capture)| (index, capture.unwrap_or_default()))
            .collect();
        sync_typed_indexed_array(&mut self.shell_state.variables, "BASH_REMATCH", &entries);
        self.shell_state.env_vars.insert(
            "BASH_REMATCH".to_string(),
            format_indexed_array_storage(entries),
        );
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, "BASH_REMATCH");
    }

    pub(super) fn clear_bash_rematch(&mut self) {
        sync_typed_indexed_array(
            &mut self.shell_state.variables,
            "BASH_REMATCH",
            &BTreeMap::new(),
        );
        self.shell_state.env_vars.insert(
            "BASH_REMATCH".to_string(),
            format_indexed_array_storage(BTreeMap::new()),
        );
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, "BASH_REMATCH");
    }

    /// GNU cond.c cond_expand_word -> subst.c expand_word: the `=~`
    /// operands are expanded like any other `[[ ]]` word, so an unbound
    /// parameter under `set -u` raises err_unboundvar (subst.c:11027)
    /// BEFORE the match is attempted. The regex paths expand through
    /// expand_conditional_word, whose plain-reference arm carries no
    /// nounset verdict, so `[[ x =~ $u ]]` silently matched (rubash#1177).
    /// Scan both raw operands with the shared nounset scan, subject
    /// first (GNU expands left-to-right); on a hit, run the comsubs that
    /// precede it in the word (GNU forks each as the walk reaches it),
    /// print the diagnostic, and latch the fatal for the dispatch sites
    /// to convert via take_conditional_nounset_fatal.
    fn regex_operands_nounset_fatal(&mut self, left: &str, right: &str) -> bool {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            return false;
        }
        let mut prefix_comsubs: Vec<String> = Vec::new();
        for word in [left, right] {
            // rubash#1915: same recorded-span masking as the operand
            // expansion scan — a procsub body's `$` is the child's,
            // never a parent reference for the `=~` nounset verdict.
            let masked = self.mask_conditional_procsub_spans(word);
            let (unbound, bodies) = self.nounset_unbound_parameter_with_comsubs(&masked);
            prefix_comsubs.extend(bodies);
            if let Some(name) = unbound {
                for body in &prefix_comsubs {
                    self.expand_command_substitution(body);
                }
                let line = format!("{}{}: unbound variable\n", self.diagnostic_prefix(), name);
                let _ = self.write_default_stderr(line.as_bytes());
                self.shell_state.conditional_nounset_error.set(true);
                return true;
            }
        }
        false
    }

    /// Convert a latched `[[ =~ ]]` nounset fatal (rubash#1177) into the
    /// exit the simple-command pre-scan produces for the same hit
    /// (command_prepare.rs parameter_expansion_error FATAL arm):
    /// interactive shells DISCARD the command with status 1; elsewhere
    /// the FORCE_EOF status is context-owned (expansion_fatal_status).
    pub(in crate::executor) fn take_conditional_nounset_fatal(
        &mut self,
    ) -> Option<Result<(), ExecuteError>> {
        if !self.shell_state.conditional_nounset_error.replace(false) {
            return None;
        }
        if self.expansion_error_is_interactive_discard() {
            self.exit_code = 1;
            self.command_expansion_failed.set(true);
            return Some(Err(ExecuteError::ExpansionFailure(1)));
        }
        let code = self.expansion_fatal_status();
        self.exit_code = code;
        Some(Err(ExecuteError::ExitCode(code)))
    }

    pub(super) fn conditional_regex_match_status(&mut self, left: &str, right: &str) -> i32 {
        if self.regex_operands_nounset_fatal(left, right) {
            return 1;
        }
        let left_exp = self.expand_conditional_word(left);
        let right_exp = self.expand_conditional_word(right);
        let right_f = restore_numeric_decimal_regex_escapes(&right_exp);
        let left = crate::executor::execution_misc::decode_command_substitution_payload(&left_exp);
        let right = right_f;
        // GNU xtrace prints arg2 as cond_expand_word returned it —
        // expanded but not yet glob-quoted (execute_cmd.c:4074).
        self.xtrace_conditional_term("=~", &left, Some(&right));
        let Ok(regex) = self.compile_conditional_regex(&right) else {
            self.report_invalid_regex(&right);
            return 2;
        };
        let Some(captures) = regex.captures(&left) else {
            self.clear_bash_rematch();
            return 1;
        };

        self.store_bash_rematch(captures);
        0
    }

    fn conditional_quoted_regex_match_status(
        &mut self,
        left: &str,
        right: &str,
        metadata: &crate::parser::WordMetadata,
    ) -> i32 {
        // GNU execute_cmd.c:4048-4057 (execute_cond_node): `rmatch &&
        // shell_compatibility_level > 31` selects mode 2 (cond_expand_word
        // QGLOB_REGEXP: quoted characters literalize regex metacharacters).
        // The level is the derived compat level (shopt compatNN flags win,
        // else BASH_COMPAT): under `shopt -s compat31` OR `BASH_COMPAT=31`
        // the mode stays 0 — plain dequote_list — and sh_regmatch still
        // compiles the result as a REGEX (rubash#346).
        // The unquoted-RHS path (conditional_regex_match_status) already has
        // exactly those mode-0 semantics.
        if crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) <= 31 {
            return self.conditional_regex_match_status(left, right);
        }
        if self.regex_operands_nounset_fatal(left, right) {
            return 1;
        }
        let left = crate::executor::execution_misc::decode_command_substitution_payload(
            &self.expand_conditional_word(left),
        );
        let right = self.quote_aware_regex_rhs(right, metadata);
        let right = restore_numeric_decimal_regex_escapes(&right);
        self.xtrace_conditional_term("=~", &left, Some(&right));
        let Ok(regex) = self.compile_conditional_regex(&right) else {
            self.report_invalid_regex(&right);
            return 2;
        };
        let Some(captures) = regex.captures(&left) else {
            self.clear_bash_rematch();
            return 1;
        };

        self.store_bash_rematch(captures);
        0
    }

    /// GNU execute_cmd.c:4137-4148 / lib/sh/shmatch.c:58 sh_regmatch: a
    /// failed regcomp is status 2 plus `invalid regular expression
    /// `<pat>': <regerror>` on stderr (pat is the post-
    /// quote_string_for_globbing pattern). glibc regerror texts differ
    /// from the regex crate's, so map the recognizable POSIX ERE shape
    /// failures onto the glibc wording.
    fn report_invalid_regex(&mut self, pattern: &str) {
        let reason = regexp_error_reason(pattern);
        self.report_conditional_error(&format!("invalid regular expression `{pattern}': {reason}"));
    }

    fn compile_conditional_regex(
        &self,
        pattern: &str,
    ) -> Result<CompiledConditionalRegex, regex::Error> {
        // perf (rubash#1119): compilation is a pure function of the
        // expanded pattern text, the nocasematch flag and the locale
        // granularity (rubash#1061 byte mode) — the decode, POSIX
        // bracket translation, class-name validation and the regex
        // build are all deterministic in those inputs — but a full
        // regex-crate build costs tens of microseconds where glibc
        // regcomp costs a few, so a `[[ =~ ]]` in a loop paid it every
        // iteration (233ms vs GNU 42ms for 2000 matches). Cache the
        // last few compiled programs per thread (the executor is
        // single-threaded per shell) and hand out clones. Failures
        // cache as None: the error payload is never inspected (the
        // diagnostic path re-derives its reason from the pattern
        // text), so a stand-in error is returned for a known-invalid
        // pattern.
        let case_insensitive =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "nocasematch");
        let byte_mode = conditional_regex_byte_mode();
        if let Some(cached) = conditional_regex_cache_lookup(pattern, case_insensitive, byte_mode) {
            return match cached {
                Some(regex) => Ok((*regex).clone()),
                None => Err(regex::Regex::new("(").unwrap_err()),
            };
        }
        match self.compile_conditional_regex_uncached(pattern, case_insensitive, byte_mode) {
            Ok(regex) => {
                let regex = std::rc::Rc::new(regex);
                conditional_regex_cache_store(
                    pattern,
                    case_insensitive,
                    byte_mode,
                    Some(regex.clone()),
                );
                Ok((*regex).clone())
            }
            Err(error) => {
                conditional_regex_cache_store(pattern, case_insensitive, byte_mode, None);
                Err(error)
            }
        }
    }

    fn compile_conditional_regex_uncached(
        &self,
        pattern: &str,
        case_insensitive: bool,
        byte_mode: bool,
    ) -> Result<CompiledConditionalRegex, regex::Error> {
        // The comsub payload spelling is transport text, not regex syntax:
        // decode it to the carried byte so `[[ $v =~ a$(printf '\001')c ]]`
        // compiles the pattern GNU's cond_expand_word produced — the raw
        // byte 0x01 (execute_cmd.c:4137 sh_regmatch regcomps the expanded
        // operand). rubash#729 widened the protected byte set, so this
        // decode boundary is what keeps the sentinel out of regcomp.
        let pattern = crate::executor::execution_misc::decode_command_substitution_payload(pattern);
        // In a UTF-8 locale the bracket classes expand to Unicode
        // tables (glibc regcomp resolves them through wctype); in byte
        // mode they pass through to the regex crate's ASCII classes,
        // which are exactly the single-byte tables (rubash#1621).
        let pattern = translate_posix_bracket_classes(&pattern, !byte_mode);
        // GNU regcomp rejects an unknown `[:name:]` with REG_ECTYPE
        // ("Invalid character class name"); the regex crate parses the
        // span as a nested class instead, so validate names before
        // handing the pattern over (cond-regexp2.sub `[[:invalid:]`).
        if !posix_bracket_class_names_valid(&pattern)
            || posix_ere_has_extended_group(&pattern)
            || posix_ere_has_dangling_quantifier(&pattern)
            || posix_ere_has_empty_min_interval(&pattern)
        {
            return Err(regex::Regex::new("(").unwrap_err());
        }
        // POSIX ERE quantifier spellings the regex crate reads
        // differently (nested `?`, empty-minimum intervals): rewrite
        // to the nested form before compiling (rubash#1058).
        let pattern = translate_posix_ere_quantifiers(&pattern);
        // GNU regcomp is called WITHOUT REG_NEWLINE (lib/sh/shmatch.c
        // sh_regmatch), so `.` matches a newline (rubash#1058).
        // Granularity follows the locale model (rubash#1061): glibc
        // regexec matches one multibyte CHARACTER in a UTF-8 locale and
        // one BYTE otherwise; rubash's locale model (locale.rs is_utf8)
        // names the active case — an unset locale resolves to C there
        // (rubash#1622), so only an actually active UTF-8 locale keeps
        // the char engine.
        if byte_mode {
            regex::bytes::RegexBuilder::new(&pattern)
                .case_insensitive(case_insensitive)
                .dot_matches_new_line(true)
                .unicode(false)
                .build()
                .map(CompiledConditionalRegex::Bytes)
        } else {
            regex::RegexBuilder::new(&pattern)
                .case_insensitive(case_insensitive)
                .dot_matches_new_line(true)
                .build()
                .map(CompiledConditionalRegex::Text)
        }
    }

    /// GNU execute_cmd.c:4077 cond_expand_word(w, 2) -> subst.c:11229
    /// expand_word_internal keeps the quoting marks, then
    /// pathexp.c:212 quote_string_for_globbing(QGLOB_REGEXP|QGLOB_CTLESC)
    /// renders them as ERE quoting: a quoted ere_char gets a backslash,
    /// any other quoted char drops its mark, an unquoted `[` opens a
    /// bracket expression scanned with its own member rules, and a bare
    /// backslash in expanded text passes through untouched.
    fn quote_aware_regex_rhs(
        &mut self,
        right: &str,
        metadata: &crate::parser::WordMetadata,
    ) -> String {
        let marked = self.regex_rhs_marked_chars(right, metadata);
        quote_marked_chars_for_regexp(&marked)
    }

    /// Phase 1 of cond_expand_word(w, 2): expand the RHS word while
    /// retaining, per character, whether it was quoted (GNU's CTLESC).
    /// The parser's raw token (`metadata.raw`) supplies the quoting
    /// boundaries that the cooked arg already lost.
    fn regex_rhs_marked_chars(
        &mut self,
        right: &str,
        metadata: &crate::parser::WordMetadata,
    ) -> Vec<(char, bool)> {
        if metadata.raw.is_empty() {
            return self
                .expand_word_mut(right)
                .chars()
                .map(|c| (c, false))
                .collect();
        }

        let chars: Vec<char> = metadata.raw.chars().collect();
        let mut marked = Vec::new();
        let mut index = 0usize;
        while index < chars.len() {
            if chars[index] == '\\' {
                if let Some(&c) = chars.get(index + 1) {
                    marked.push((c, true));
                    index += 2;
                } else {
                    index += 1;
                }
                continue;
            }

            if let Some((kind, opener_len, end)) = raw_quote_at(&chars, index) {
                let body = chars[index + opener_len..end].iter().collect::<String>();
                let quoted = match kind {
                    QuoteKind::Single | QuoteKind::Backslash => body,
                    QuoteKind::AnsiC => decode_ansi_c_escapes(&body),
                    QuoteKind::Double | QuoteKind::Locale => self.expand_conditional_word(&body),
                };
                let quoted =
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(&quoted);
                // The bytes are the UTF-8 encoding of the expanded text (the
                // carrier markers decoded); widening each byte to a char
                // Latin-1-encodes multi-byte payload, and the reassembled
                // regex RHS arrived mojibaked (中文 -> ä¸­). Decode back to
                // characters so the marked stream carries the same chars the
                // no-raw-token path above does. Byte products that are not
                // valid UTF-8 degrade to U+FFFD instead of distinct Latin-1
                // chars — they could never match a char-mode subject either
                // way.
                let quoted = String::from_utf8_lossy(&quoted);
                marked.extend(quoted.chars().map(|c| (c, true)));
                index = end + 1;
                continue;
            }

            let start = index;
            while index < chars.len() {
                // Nested `$(...)`/`${...}`/`` `...` `` bodies keep their own
                // quote state (GNU xparse_dolparen): `\` and quotes inside
                // them do not terminate the outer unquoted run.
                if let Some(next) =
                    crate::parser::pathname_pattern::skip_nested_expansion(&chars, index)
                {
                    index = next;
                    continue;
                }
                if chars[index] == '\\' || raw_quote_at(&chars, index).is_some() {
                    break;
                }
                index += 1;
            }
            let segment = chars[start..index].iter().collect::<String>();
            let expanded = self.expand_conditional_word(&segment);
            let expanded =
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&expanded);
            let expanded = String::from_utf8_lossy(&expanded);
            marked.extend(expanded.chars().map(|c| (c, false)));
        }

        marked
    }

    /// GNU cond_expand_word(op, 3) -> expand_word_internal under Q_ARITH:
    /// a `name[sub]` operand region runs expand_array_subscript
    /// (subst.c:11107) — the subscript is expanded once
    /// (expand_subscript_string) and every product byte that could
    /// restart an expansion or delimit a subscript is backslash-quoted
    /// (abstab: `[` `]` `$` `` ` `` `~` `\` `'` `"`), so the later
    /// evalexp/array_expand_index passes see expansion products as data.
    /// `assoc[$key]` with key=`x],b[$(echo uname >&2)` cooks to
    /// `assoc[x\],b\[\$(echo uname >&2)]` — the `\]` stays inside the
    /// subscript and `\$(` never executes.
    fn expand_cond_arith_operand(&mut self, raw: &str) -> String {
        let bytes = raw.as_bytes();
        if !raw.contains('[') {
            return self.expand_conditional_word(raw);
        }
        let mut output = String::new();
        let mut literal_start = 0usize;
        let mut index = 0usize;
        let mut single = false;
        let mut double = false;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => index += 1,
                b'\'' if !double => single = !single,
                b'"' if !single => double = !double,
                // `$(`/`${`/backquote bodies own their `[`: `${a[0]}` is a
                // parameter expansion for expand_word_mut, not an operand-level
                // array reference. GNU's arithcomp (test.c:357) sees it after
                // cond word expansion as the resolved value.
                b'`' => index = assoc_skip_substitution(bytes, index),
                b'$' if matches!(bytes.get(index + 1), Some(&b'(') | Some(&b'{')) => {
                    index = assoc_skip_substitution(bytes, index);
                }
                b'[' if !single && !double => {
                    let mut name_start = index;
                    while name_start > literal_start
                        && is_shell_name_char(bytes[name_start - 1] as char)
                    {
                        name_start -= 1;
                    }
                    let closed = name_start < index
                        && is_shell_name_start(bytes[name_start] as char)
                        && assoc_subscript_end(bytes, index) > index + 1
                        && bytes.get(assoc_subscript_end(bytes, index) - 1) == Some(&b']');
                    if !closed {
                        index += 1;
                        continue;
                    }
                    let end = assoc_subscript_end(bytes, index);
                    output.push_str(&self.expand_conditional_word(&raw[literal_start..name_start]));
                    output.push_str(&raw[name_start..index]);
                    output.push('[');
                    let expanded = self.expand_subscript_string(&raw[index + 1..end - 1]);
                    for ch in expanded.chars() {
                        if matches!(ch, '[' | ']' | '$' | '`' | '~' | '\\' | '\'' | '"') {
                            output.push('\\');
                        }
                        output.push(ch);
                    }
                    output.push(']');
                    literal_start = end;
                    index = end;
                    continue;
                }
                _ => {}
            }
            index += 1;
        }
        if literal_start < raw.len() {
            output.push_str(&self.expand_conditional_word(&raw[literal_start..]));
        }
        output
    }

    pub(super) fn conditional_numeric_binary(&mut self, left: &str, op: &str, right: &str) -> bool {
        self.conditional_numeric_binary_status(left, op, right) == 0
    }

    pub(super) fn conditional_numeric_binary_status(
        &mut self,
        left: &str,
        op: &str,
        right: &str,
    ) -> i32 {
        // Whitelist fast path (rubash#117 admission, not a blacklist): both
        // operands are already plain decimal literals, so GNU's
        // cond_expand_word (execute_cmd.c:4024) and test.c:357 arithcomp ->
        // evalexp are the identity. Skip the whole expand + subscript +
        // dynamic-context + eval + flush + sync chain for both operands.
        // A literal cannot write, diagnose, or read RANDOM/arrays, so the
        // only observable side effects are the xtrace line and the status.
        // Doubt (octal leading zero, sign, overflow, `$`/`[`/quote carriers)
        // -> None -> the full path below.
        if let (Some(left_val), Some(right_val)) = (
            cond_arith_decimal_literal(left),
            cond_arith_decimal_literal(right),
        ) {
            // Mirror the per-eval boundary: a successful eval clears the
            // stale evalerror record and leaves no diagnostics.
            let _ = crate::executor::arithmetic::take_arith_eval_error();
            let _ = crate::executor::arithmetic::take_arith_eval_diags();
            self.xtrace_conditional_term(op, left, Some(right));
            // A literal can never queue an arithmetic write, but keep the
            // boundary identical to the full path (the take-walk is empty).
            super::arithmetic::sync_arith_writes(self);
            return i32::from(!conditional_numeric_matches(left_val, op, right_val));
        }
        let left_expanded = self.expand_cond_arith_operand(left);
        let right_expanded = self.expand_cond_arith_operand(right);
        self.xtrace_conditional_term(op, &left_expanded, Some(&right_expanded));
        // GNU test.c:357 arithcomp -> evalexp: expr.c:1190-1216
        // expr_streval raises err_unboundvar for a bare unset name when
        // unbound_vars_is_error. The raw evaluator below reads unset
        // names as 0, so the nounset prologue the (( ))/let paths run
        // has to run here too, per operand, in evaluation order
        // (rubash#1709 — the #1130/#1201 family covered the $-expansion
        // path; a bare name never expands, it reaches evalexp as text).
        if let Some(status) = self.conditional_nounset_arith_trap(&left_expanded) {
            return status;
        }

        // GNU execute_cmd.c:4049-4068 -> test.c:357-372 arithcomp -> evalexp:
        // the operands were word-expanded by cond_expand_word (mode 3,
        // Q_ARITH). Under compat>51 arithcomp passes eflag=0, so
        // already_expanded is false and array_expand_index expands indexed
        // subscripts unconditionally (expr.c:1171 — the array_expand_once
        // gate needs already_expanded set). A surviving top-level
        // `$name`/`$(...)` is still readtok junk -> "operand expected"
        // (expr.c:1502-1510), hence no_expand inside the parser.
        let left_eval = self.expand_arith_indexed_subscripts(&left_expanded);
        let left_dynamic = self.arith_dynamic_context();
        let (Some(left_val), _) = eval_mutable_arith_value_with_random_flags(
            &left_eval,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            Some(&left_dynamic),
            true,
            Some(&self.shell_state.variables),
        ) else {
            self.flush_arith_diags(Some("[["));
            self.report_conditional_arithmetic_error(&left_eval);
            return 1;
        };
        self.flush_arith_diags(Some("[["));
        // rubash#404: GNU test.c:357-372 arithcomp -> evalexp binds the
        // operand's assignment side effects into the one variable store, so
        // the next parameter expansion sees them (`n=1; x='n+=2';
        // [[ x -eq 3 ]]; echo $n` prints 3). The evaluator wrote env_vars
        // only; sync the typed variables map at this boundary like the
        // `(( ))` / `let` / subscript evaluators do.
        super::arithmetic::sync_arith_writes(self);
        if let Some(status) = self.conditional_nounset_arith_trap(&right_expanded) {
            return status;
        }
        let right_eval = self.expand_arith_indexed_subscripts(&right_expanded);
        let right_dynamic = self.arith_dynamic_context();
        let (Some(right_val), _) = eval_mutable_arith_value_with_random_flags(
            &right_eval,
            &mut self.shell_state.env_vars,
            Some(&self.shell_state.random_state),
            Some(&right_dynamic),
            true,
            Some(&self.shell_state.variables),
        ) else {
            self.flush_arith_diags(Some("[["));
            self.report_conditional_arithmetic_error(&right_eval);
            return 1;
        };
        self.flush_arith_diags(Some("[["));
        super::arithmetic::sync_arith_writes(self);
        i32::from(!conditional_numeric_matches(left_val, op, right_val))
    }

    /// The `set -u` side of GNU's arithcomp evaluation (expr.c:1190-1216
    /// expr_streval -> err_unboundvar): scan one already-expanded
    /// numeric operand for a bare unset name. On a trap, mirror
    /// expand_conditional_word's FATAL branch — print the diagnostic
    /// through the conditional's fd 2 and park the fatal status for
    /// execute_conditional_command_with_redirects to unwind with the
    /// FORCE_EOF / interactive-DISCARD classes (rubash#1709).
    fn conditional_nounset_arith_trap(&mut self, operand: &str) -> Option<i32> {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset") {
            return None;
        }
        let name =
            super::arithmetic::arithmetic_unbound_variable(operand, &self.shell_state.env_vars)?;
        let line = format!(
            "{}{}: {}\n",
            self.diagnostic_prefix(),
            name,
            "unbound variable"
        );
        let _ = self.write_diagnostic_fd2(line.as_bytes());
        self.exit_code = if self.expansion_error_is_interactive_discard() {
            1
        } else {
            self.expansion_fatal_status()
        };
        self.conditional_fatal_status.set(Some(self.exit_code));
        self.command_expansion_failed.set(true);
        self.conditional_expansion_fatal.set(true);
        Some(self.exit_code)
    }
}

/// GNU `[[ a -op b ]]` comparison verdict (test.c:357 arithcomp compares the
/// two `intmax_t` results). Shared by the literal whitelist and the full
/// expand+eval path so the two spellings cannot drift.
fn conditional_numeric_matches(left_val: i128, op: &str, right_val: i128) -> bool {
    match op {
        "-eq" => left_val == right_val,
        "-ne" => left_val != right_val,
        "-lt" => left_val < right_val,
        "-le" => left_val <= right_val,
        "-gt" => left_val > right_val,
        "-ge" => left_val >= right_val,
        _ => false,
    }
}

/// A `[[ ]]` arithmetic operand that is a plain, unambiguous decimal literal.
/// GNU test.c:357 arithcomp -> evalexp (expr.c) reads such a token as its own
/// value: no expansion, no assignment, no octal/sign/base reinterpretation.
/// Only that exact class is whitelisted — a multi-digit leading zero takes
/// GNU strlong's octal arm (`[[ 010 -eq 8 ]]` is true), a sign goes through
/// the unary operators, an overflow takes strlong's wrapping/error arm, and
/// any `$`/`[`/quote carrier spelling is not a literal at all. Every rejected
/// shape returns None and keeps the full expand+eval path.
fn cond_arith_decimal_literal(raw: &str) -> Option<i128> {
    let bytes = raw.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if bytes.len() > 1 && bytes[0] == b'0' {
        return None;
    }
    raw.parse::<i64>().ok().map(|value| value as i128)
}

thread_local! {
    /// Compiled-program cache for `[[ =~ ]]` (rubash#1119): the last few
    /// (pattern, nocasematch, byte-mode) -> program mappings, most
    /// recent last. `None` records a pattern that failed to compile.
    /// Bounded so a script generating unbounded distinct patterns
    /// cannot grow it.
    static CONDITIONAL_REGEX_CACHE: std::cell::RefCell<
        Vec<(String, bool, bool, Option<std::rc::Rc<CompiledConditionalRegex>>)>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

const CONDITIONAL_REGEX_CACHE_CAP: usize = 16;

fn conditional_regex_cache_lookup(
    pattern: &str,
    case_insensitive: bool,
    byte_mode: bool,
) -> Option<Option<std::rc::Rc<CompiledConditionalRegex>>> {
    CONDITIONAL_REGEX_CACHE.with(|cache| {
        let cache = cache.borrow();
        cache
            .iter()
            .rev()
            .find(|(cached_pattern, cached_ci, cached_bm, _)| {
                cached_ci == &case_insensitive
                    && cached_bm == &byte_mode
                    && cached_pattern == pattern
            })
            .map(|(_, _, _, compiled)| compiled.clone())
    })
}

fn conditional_regex_cache_store(
    pattern: &str,
    case_insensitive: bool,
    byte_mode: bool,
    compiled: Option<std::rc::Rc<CompiledConditionalRegex>>,
) {
    CONDITIONAL_REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(cached_pattern, cached_ci, cached_bm, _)| {
            !(cached_ci == &case_insensitive
                && cached_bm == &byte_mode
                && cached_pattern == pattern)
        });
        cache.push((pattern.to_string(), case_insensitive, byte_mode, compiled));
        if cache.len() > CONDITIONAL_REGEX_CACHE_CAP {
            cache.remove(0);
        }
    });
}

fn contains_extglob_pattern(pattern: &str) -> bool {
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '@' | '*' | '+' | '?' | '!') && chars.peek() == Some(&'(') {
            return true;
        }
    }
    false
}

fn raw_quote_at(chars: &[char], start: usize) -> Option<(QuoteKind, usize, usize)> {
    let (kind, opener_len, terminator) = match chars.get(start)? {
        '$' if chars.get(start + 1) == Some(&'\'') => (QuoteKind::AnsiC, 2, '\''),
        '$' if chars.get(start + 1) == Some(&'"') => (QuoteKind::Locale, 2, '"'),
        '\'' => (QuoteKind::Single, 1, '\''),
        '"' => (QuoteKind::Double, 1, '"'),
        _ => return None,
    };

    let mut index = start + opener_len;
    while index < chars.len() {
        if chars[index] == '\\' && terminator != '\'' {
            index += 2;
            continue;
        }
        if chars[index] == terminator {
            return Some((kind, opener_len, index));
        }
        index += 1;
    }

    None
}

/// pathexp.c:135 ere_char — POSIX ERE characters that must be
/// backslash-quoted to match themselves.
fn ere_char(c: char) -> bool {
    matches!(
        c,
        '.' | '[' | '\\' | '(' | ')' | '*' | '+' | '?' | '{' | '|' | '^' | '$'
    )
}

/// pathexp.c:212 quote_string_for_globbing under QGLOB_REGEXP|QGLOB_CTLESC.
/// `marked` pairs each expanded character with a quoted flag standing in
/// for a CTLESC prefix:
/// - quoted ere_char -> `\c`; any other quoted char -> `c` bare
///   (pathexp.c:243-259).
/// - unquoted `[` opens a bracket expression whose members are scanned
///   with the bracket rules: a quoted member emits bare, `[:.:]`,
///   `[=.=]`, `[...]` spans are recognized, and an unterminated bracket
///   rescans the `[` as an ordinary character (pathexp.c:275-360).
/// - an unquoted backslash (e.g. from variable expansion) passes through
///   untouched (pathexp.c:390-393).
fn quote_marked_chars_for_regexp(marked: &[(char, bool)]) -> String {
    fn unquoted(marked: &[(char, bool)], j: usize, c: char) -> bool {
        marked.get(j).is_some_and(|&(mc, q)| !q && mc == c)
    }
    let mut out = String::new();
    let mut i = 0usize;
    'outer: while i < marked.len() {
        let (c, quoted) = marked[i];
        if quoted {
            if ere_char(c) {
                out.push('\\');
            }
            out.push(c);
            i += 1;
            continue;
        }
        if c == '[' {
            out.push('[');
            let save_out = out.len();
            let mut j = i + 1;
            let save_i = j;
            let mut cclass = false;
            let mut equiv = false;
            let mut collsym = false;
            let mut c = marked.get(j).copied();
            j += 1;
            if c == Some(('^', false)) {
                out.push('^');
                c = marked.get(j).copied();
                j += 1;
            }
            if c == Some((']', false)) {
                out.push(']');
                c = marked.get(j).copied();
                j += 1;
            }
            let mut terminated = false;
            let mut abort = false;
            loop {
                let Some((cc, q)) = c else {
                    // c == 0 as the first member: GNU goto endpat — the
                    // pattern simply ends (leaving the stray `[`).
                    abort = true;
                    break;
                };
                if q {
                    // CTLESC <member>: emit the member bare.
                    out.push(cc);
                } else if cc == '[' && unquoted(marked, j, ':') {
                    out.push('[');
                    out.push(':');
                    j += 1;
                    cclass = true;
                } else if cclass && cc == ':' && unquoted(marked, j, ']') {
                    out.push(':');
                    out.push(']');
                    j += 1;
                    cclass = false;
                } else if cc == '[' && unquoted(marked, j, '=') {
                    out.push('[');
                    out.push('=');
                    j += 1;
                    if unquoted(marked, j, ']') {
                        out.push(']');
                        j += 1;
                    }
                    equiv = true;
                } else if equiv && cc == '=' && unquoted(marked, j, ']') {
                    out.push('=');
                    out.push(']');
                    j += 1;
                    equiv = false;
                } else if cc == '[' && unquoted(marked, j, '.') {
                    out.push('[');
                    out.push('.');
                    j += 1;
                    if unquoted(marked, j, ']') {
                        out.push(']');
                        j += 1;
                    }
                    collsym = true;
                } else if collsym && cc == '.' && unquoted(marked, j, ']') {
                    out.push('.');
                    out.push(']');
                    j += 1;
                    collsym = false;
                } else {
                    out.push(cc);
                }
                c = marked.get(j).copied();
                j += 1;
                match c {
                    Some((']', false)) => {
                        terminated = true;
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
            if abort {
                break 'outer;
            }
            if !terminated {
                // Rescan without the bracket interpretation: the `[`
                // stays emitted and its members are reprocessed through
                // the ordinary quoting rules.
                out.truncate(save_out);
                i = save_i;
                continue 'outer;
            }
            out.push(']');
            i = j;
            continue 'outer;
        }
        // Ordinary character (including an unquoted `\`, which is data
        // in ERE quoting mode).
        out.push(c);
        i += 1;
    }
    out
}

/// A compiled `=~` pattern in the granularity the active locale model
/// dictates (rubash#1061): the char engine for a UTF-8 locale, the byte
/// engine otherwise. Captures are reported as storage strings in both
/// modes; in byte mode a capture that is not valid UTF-8 (e.g. `.`
/// splitting a multibyte character, exactly what glibc regexec yields)
/// carries its stray bytes as U+E000 raw-byte marker pairs, which the
/// normal output boundaries decode back to the original bytes.
#[derive(Clone)]
enum CompiledConditionalRegex {
    Text(regex::Regex),
    Bytes(regex::bytes::Regex),
}

impl CompiledConditionalRegex {
    fn captures(&self, left: &str) -> Option<Vec<Option<String>>> {
        match self {
            CompiledConditionalRegex::Text(regex) => regex.captures(left).map(|captures| {
                captures
                    .iter()
                    .map(|capture| capture.map(|matched| matched.as_str().to_string()))
                    .collect()
            }),
            CompiledConditionalRegex::Bytes(regex) => {
                regex.captures(left.as_bytes()).map(|captures| {
                    captures
                        .iter()
                        .map(|capture| {
                            capture
                                .map(|matched| regex_capture_bytes_to_storage(matched.as_bytes()))
                        })
                        .collect()
                })
            }
        }
    }
}

/// Whether `=~` matches byte-wise. GNU regexec is byte-oriented in a
/// single-byte locale and character-oriented in a UTF-8 locale. The
/// locale decision itself lives in [`crate::locale::is_utf8`]: since
/// rubash#1622 an unset locale resolves to the C locale there (GNU's
/// setlocale(LC_ALL, "") startup default), so this is the plain
/// negation — the empty-name clause this function used to carry was a
/// one-surface compensation for the old unset-means-UTF-8 default
/// (rubash#1061) and is gone with it.
fn conditional_regex_byte_mode() -> bool {
    !crate::locale::is_utf8()
}

/// Convert one byte-mode capture into rubash's storage spelling: valid
/// UTF-8 runs stay text; each invalid byte becomes a U+E000 raw-byte
/// marker pair (executor::markers::push_raw_byte_marker), the same
/// encoding `$'\xff'` data uses.
fn regex_capture_bytes_to_storage(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                out.push_str(text);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..valid]).unwrap_or_default());
                let invalid_len = error.error_len().unwrap_or(rest.len() - valid);
                for &byte in &rest[valid..valid + invalid_len] {
                    crate::executor::markers::push_raw_byte_marker(&mut out, byte);
                }
                rest = &rest[valid + invalid_len..];
                if rest.is_empty() {
                    break;
                }
            }
        }
    }
    out
}

/// POSIX ERE has no `(?...)` construct: glibc regcomp rejects every
/// unescaped `(?` outside a bracket expression with REG_BADRPT
/// ("Invalid preceding regular expression") — `(?:...)`, `(?i)`,
/// `(?=...)` alike (rubash#1058). The regex crate accepts several of
/// these extensions, so scan for the shape before compiling.
fn posix_ere_has_extended_group(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0usize;
    let mut in_class = false;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => {
                if chars.get(index + 1) == Some(&'?') {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

/// Parse a POSIX ERE interval token at chars[start] == '{': `{n}`,
/// `{n,}`, `{n,m}`, and the glibc empty-minimum forms `{,m}` / `{,}`
/// (an omitted minimum is 0). Returns (end index, minimum, maximum)
/// with maximum None for an open upper bound, or None when the brace
/// does not form an interval and is literal text.
fn ere_interval_at(chars: &[char], start: usize) -> Option<(usize, u64, Option<u64>)> {
    let mut i = start + 1;
    let min_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    let min_text: String = chars[min_start..i].iter().collect();
    let (max_text, end) = if i < chars.len() && chars[i] == ',' {
        i += 1;
        let max_start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '}' {
            return None;
        }
        (Some(chars[max_start..i].iter().collect::<String>()), i + 1)
    } else if i < chars.len() && chars[i] == '}' && !min_text.is_empty() {
        (None, i + 1)
    } else {
        return None;
    };
    let minimum: u64 = if min_text.is_empty() {
        0
    } else {
        min_text.parse().unwrap_or(u64::MAX)
    };
    let maximum: Option<u64> = match &max_text {
        None => Some(minimum),
        Some(text) if text.is_empty() => None,
        Some(text) => Some(text.parse().unwrap_or(u64::MAX)),
    };
    Some((end, minimum, maximum))
}

/// Index just past the bracket expression at chars[start] == '[', or
/// None when it never closes (the `[` is then literal text).
fn bracket_expression_end(chars: &[char], start: usize) -> Option<usize> {
    translate_bracket_expression(chars, start, false).map(|(next, _)| next)
}

/// glibc regcomp (REG_BADRPT): a repetition operator with no
/// quantifiable operand — at the start of the pattern or a
/// subexpression, right after `|` or `(`, or right after an anchor
/// (an anchor is not an operand: `a^*`, `$*a`, and `a$*` all fail
/// with REG_BADRPT in glibc regcomp; integration re-verified against
/// regcomp directly and GNU bash 5.3.20, correcting an earlier probe
/// note that claimed `$*a` compiles). Stacked operators are fine
/// (`a**`, `a*{2}` nest); only the first operator of a run can
/// dangle.
fn posix_ere_has_dangling_quantifier(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut have_atom = false;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                have_atom = true;
                i += 2;
            }
            '[' => {
                have_atom = true;
                i = bracket_expression_end(&chars, i).unwrap_or(i + 1);
            }
            '(' => {
                have_atom = false;
                i += 1;
            }
            ')' => {
                have_atom = true;
                i += 1;
            }
            '|' => {
                have_atom = false;
                i += 1;
            }
            '^' | '$' => {
                have_atom = false;
                i += 1;
            }
            '*' | '+' | '?' => {
                if !have_atom {
                    return true;
                }
                i += 1;
            }
            '{' => {
                if let Some((next, _, _)) = ere_interval_at(&chars, i) {
                    if !have_atom {
                        return true;
                    }
                    i = next;
                } else {
                    have_atom = true;
                    i += 1;
                }
            }
            _ => {
                have_atom = true;
                i += 1;
            }
        }
    }
    false
}

/// glibc regcomp (REG_BADBR): an interval whose minimum exceeds its
/// maximum, `a{2,1}` — "Invalid content of \{\}". Bracket spans are
/// skipped (a `{` inside one is a member character).
fn posix_ere_has_bad_interval(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '[' => i = bracket_expression_end(&chars, i).unwrap_or(i + 1),
            '{' => {
                if let Some((next, minimum, Some(maximum))) = ere_interval_at(&chars, i) {
                    if minimum > maximum {
                        return true;
                    }
                    i = next;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Whether the pattern contains an interval with an omitted minimum
/// (`{,5}` / `{,}`) outside brackets and escapes. POSIX ERE requires
/// the leading number; glibc regcomp accepts the omission as {0,5},
/// but the strict regcomp reading — the GNU oracle rubash#1658 was
/// filed against — rejects it with "invalid repetition count(s)", so
/// the compile gate does too.
fn posix_ere_has_empty_min_interval(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '[' => i = bracket_expression_end(&chars, i).unwrap_or(i + 1),
            '{' => {
                if chars.get(i + 1) == Some(&',') && ere_interval_at(&chars, i).is_some() {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// Rewrite the two POSIX ERE quantifier spellings the regex crate
/// reads differently, leaving everything else byte-identical:
///
/// - A `?` directly after another quantifier is a NESTED quantifier
///   in ERE (glibc: `a+?` is `(a+)?`, leftmost-longest, so it matches
///   greedily) but the regex crate's LAZY marker. Wrap the whole
///   repetition in a non-capturing group so the `?` quantifies it:
///   `a+?` -> `(?:a+)?`, `a{2,}?` -> `(?:a{2,})?`, `(aa)+?` ->
///   `(?:(aa)+)?`. Other stacked operators (`**`, `*{2}`, `?+`) nest
///   identically in both engines and pass through.
///
/// Empty-minimum intervals (`{,m}` / `{,}`) never reach this rewrite:
/// the compile gate rejects them first (rubash#1658).
fn translate_posix_ere_quantifiers(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len() + 8);
    // (start, end) span in `out` of the most recent atom, and whether
    // a quantifier already applies to it (the repetition tail a
    // following `?` would nest over).
    let mut last_atom: Option<(usize, usize)> = None;
    let mut quantified = false;
    let mut group_starts: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                let start = out.len();
                out.push('\\');
                if i + 1 < chars.len() {
                    out.push(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
                last_atom = Some((start, out.len()));
                quantified = false;
            }
            '[' => {
                let start = out.len();
                match bracket_expression_end(&chars, i) {
                    Some(next) => {
                        out.extend_from_slice(&chars[i..next]);
                        i = next;
                    }
                    None => {
                        out.push('[');
                        i += 1;
                    }
                }
                last_atom = Some((start, out.len()));
                quantified = false;
            }
            '(' => {
                group_starts.push(out.len());
                out.push('(');
                last_atom = None;
                quantified = false;
                i += 1;
            }
            ')' => {
                out.push(')');
                let start = group_starts.pop().unwrap_or_else(|| out.len() - 1);
                last_atom = Some((start, out.len()));
                quantified = false;
                i += 1;
            }
            '|' => {
                out.push('|');
                last_atom = None;
                quantified = false;
                i += 1;
            }
            '*' | '+' => {
                out.push(chars[i]);
                if last_atom.is_some() {
                    quantified = true;
                }
                i += 1;
            }
            '?' => {
                if quantified {
                    let (atom_start, _) = last_atom.unwrap();
                    let mut wrapped: Vec<char> = Vec::with_capacity(out.len() + 4);
                    wrapped.extend_from_slice(&out[..atom_start]);
                    wrapped.extend(['(', '?', ':']);
                    wrapped.extend_from_slice(&out[atom_start..]);
                    wrapped.push(')');
                    let group_end = wrapped.len();
                    wrapped.push('?');
                    out = wrapped;
                    last_atom = Some((atom_start, group_end));
                    // `quantified` stays true: the `?` just pushed is
                    // the wrapped group's own quantifier, so a further
                    // `?` nests over the whole group again (`a+??`).
                } else {
                    out.push('?');
                    if last_atom.is_some() {
                        quantified = true;
                    }
                }
                i += 1;
            }
            '{' => match ere_interval_at(&chars, i) {
                Some((next, _, _)) => {
                    out.extend_from_slice(&chars[i..next]);
                    if last_atom.is_some() {
                        quantified = true;
                    }
                    i = next;
                }
                None => {
                    let start = out.len();
                    out.push('{');
                    last_atom = Some((start, out.len()));
                    quantified = false;
                    i += 1;
                }
            },
            _ => {
                let start = out.len();
                out.push(chars[i]);
                last_atom = Some((start, out.len()));
                quantified = false;
                i += 1;
            }
        }
    }
    out.into_iter().collect()
}

/// glibc regcomp repetition/interval errors, scanned left to right
/// with bracket expressions and backslash escapes skipped (their own
/// errors belong to the bracket scan in [`regexp_error_reason`]).
/// Verified against the system regcomp (rubash#1491 family):
/// - a repetition operator (`*`/`+`/`?`/valid `{m,n}`) with no
///   preceding atom (pattern start, after `(` or `|`) fails with
///   REG_BADRPT, "Invalid preceding regular expression" — `*a`, `(*a)`,
///   `a|*b`, `{2}a`; stacked operators after an atom (`a**`) are fine;
/// - `{...}` after an atom: unclosed is REG_EBRACE "Unmatched \{"
///   (`a{2`), closed with invalid content is REG_BADBR "Invalid
///   content of \{\}" (`a{}`, `a{abc}`, `a{2,1}`); `{5,}` is a valid
///   interval, while `{,5}` (omitted minimum) is rejected with
///   "invalid repetition count(s)" (rubash#1658).
fn regexp_repetition_error(pattern: &str) -> Option<&'static str> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut prev_atom = false;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                prev_atom = true;
                i += 2;
                continue;
            }
            '[' => {
                // Skip the bracket expression; an unterminated one is
                // the bracket scan's error to report, not this scan's.
                let mut j = i + 1;
                if chars.get(j) == Some(&'^') {
                    j += 1;
                }
                if chars.get(j) == Some(&']') {
                    j += 1;
                }
                let mut closed = false;
                while j < chars.len() {
                    if chars[j] == ']' {
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                if !closed {
                    return None;
                }
                prev_atom = true;
                i = j + 1;
                continue;
            }
            '(' | '|' => prev_atom = false,
            ')' => prev_atom = true,
            '*' | '+' | '?' => {
                if !prev_atom {
                    return Some("Invalid preceding regular expression");
                }
            }
            '{' => {
                let mut j = i + 1;
                let mut content = String::new();
                let mut closed = false;
                while j < chars.len() {
                    if chars[j] == '}' {
                        closed = true;
                        break;
                    }
                    content.push(chars[j]);
                    j += 1;
                }
                if !closed {
                    if prev_atom {
                        return Some("Unmatched \\{");
                    }
                    return None;
                }
                // An omitted minimum (`{,5}` / `{,}`) after an atom is
                // an invalid repetition count (rubash#1658); without a
                // preceding atom the interval is a dangling operator
                // and the interval_like path below reports it as such.
                if prev_atom && content.starts_with(',') && ere_interval_at(&chars, i).is_some() {
                    return Some("invalid repetition count(s)");
                }
                // Interval syntax: digits with an optional comma and
                // optional upper bound (`{2}`, `{2,}`, `{,5}`, `{2,3}`).
                let interval_like = {
                    let mut parts = content.splitn(2, ',');
                    let lo = parts.next().unwrap_or("");
                    let hi = parts.next();
                    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
                    !content.is_empty()
                        && digits(lo)
                        && hi.map(digits).unwrap_or(true)
                        && (!lo.is_empty() || hi.is_some_and(|h| !h.is_empty()))
                };
                if interval_like {
                    if !prev_atom {
                        return Some("Invalid preceding regular expression");
                    }
                    let mut parts = content.splitn(2, ',');
                    let lo: Option<u64> = parts.next().and_then(|s| s.parse().ok());
                    let hi: Option<u64> = parts.next().and_then(|s| s.parse().ok());
                    if let (Some(lo), Some(hi)) = (lo, hi) {
                        if lo > hi {
                            return Some("Invalid content of \\{\\}");
                        }
                    }
                } else if prev_atom {
                    return Some("Invalid content of \\{\\}");
                } else {
                    return None;
                }
                i = j + 1;
                continue;
            }
            _ => prev_atom = true,
        }
        i += 1;
    }
    None
}

/// Best-effort glibc regerror wording for patterns the regex crate
/// rejected, so `[[ x =~ bad ]]` diagnostics match GNU's
/// `invalid regular expression `<pat>': <reason>` shape.
fn regexp_error_reason(pattern: &str) -> &'static str {
    if posix_ere_has_extended_group(pattern) {
        return "Invalid preceding regular expression";
    }
    if let Some(reason) = regexp_repetition_error(pattern) {
        return reason;
    }
    let chars: Vec<char> = pattern.chars().collect();
    // Trailing backslash (odd-length run of backslashes at the end).
    let trailing_bs = chars.iter().rev().take_while(|&&c| c == '\\').count();
    if trailing_bs % 2 == 1 {
        return "Trailing backslash";
    }
    // glibc REG_BADRPT, checked before REG_BADBR: `{2,1}` at the start
    // of a pattern reports the dangling operator, not the bad range.
    if posix_ere_has_dangling_quantifier(pattern) {
        return "Invalid preceding regular expression";
    }
    // Bracket expression scan.
    let mut depth_paren = 0i32;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => {
                let mut j = i + 1;
                if chars.get(j) == Some(&'^') {
                    j += 1;
                }
                if chars.get(j) == Some(&']') {
                    j += 1;
                }
                let mut closed = false;
                while j < chars.len() {
                    if chars[j] == '\\' {
                        j += 1;
                    } else if chars[j] == ']' {
                        closed = true;
                        break;
                    } else if chars[j] == '[' && chars.get(j + 1) == Some(&':') {
                        let name_end = chars[j + 2..]
                            .iter()
                            .position(|&c| c == ':')
                            .filter(|&pos| chars.get(j + 2 + pos + 1) == Some(&']'))
                            .map(|pos| j + 2 + pos);
                        if let Some(end) = name_end {
                            let name: String = chars[j + 2..end].iter().collect();
                            const CLASSES: [&str; 12] = [
                                "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower",
                                "print", "punct", "space", "upper", "xdigit",
                            ];
                            if !CLASSES.contains(&name.as_str()) {
                                return "Invalid character class name";
                            }
                            j = end + 1;
                        }
                    } else if chars[j] == '-'
                        && j > i + 1
                        && j + 1 < chars.len()
                        && chars[j - 1] != '['
                        && chars[j + 1] != ']'
                        && chars[j - 1] > chars[j + 1]
                    {
                        return "Invalid range end";
                    }
                    j += 1;
                }
                if !closed {
                    // glibc regcomp distinguishes a bracket that is the
                    // very END of the pattern (nothing after `[`, or
                    // nothing after `[^`): that shape fails with
                    // REG_BADPAT, whose regerror text is "Invalid regular
                    // expression" — verified against the system regcomp
                    // (`[` and `a[` -> err 2) versus `[a` / `[]` ->
                    // REG_EBRACK "Unmatched [, ..." (err 7). GNU prints
                    // the regerror text verbatim (lib/sh/shmatch.c:58
                    // sh_regmatch -> execute_cmd.c invalid-regex report),
                    // so `[[ x =~ [ ]]` reads "Invalid regular
                    // expression", not the EBRACK wording (rubash#1491).
                    let mut content_start = i + 1;
                    if chars.get(content_start) == Some(&'^') {
                        content_start += 1;
                    }
                    if content_start >= chars.len() {
                        return "Invalid regular expression";
                    }
                    return "Unmatched [, [^, [:, [., or [=";
                }
                i = j;
            }
            '(' => depth_paren += 1,
            ')' => {
                depth_paren -= 1;
                if depth_paren < 0 {
                    return "Unmatched ) or \\)";
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth_paren > 0 {
        return "Unmatched ( or \\(";
    }
    if posix_ere_has_bad_interval(pattern) {
        return "Invalid content of \\{\\}";
    }
    "Invalid regular expression"
}

fn append_literal_glob_text(output: &mut String, text: &str) {
    for ch in text.chars() {
        if matches!(ch, '*' | '?' | '[' | '\\') {
            output.push('\\');
        }
        output.push(ch);
    }
}

/// Translates POSIX bracket-expression collation syntax that the regex
/// crate does not understand into equivalent bracket members before
/// compiling a [[ ... =~ ... ]] right-hand side. POSIX recognizes three
/// bracket prefixes: [=c=] equivalence classes, [.s.] collating symbols,
/// and [:class:] character classes. The regex crate supports [:class:]
/// natively, so only the first two are rewritten; in the C locale an
/// equivalence class is the single character itself. Escapes and
/// bracket-span rules follow POSIX ERE bracket expressions (a ] as the
/// first member is literal, a backslash escapes the next character, an
/// unterminated bracket is literal text).
/// GNU regcomp's bracket-expression parser only accepts the POSIX class
/// names in `[:name:]`; anything else is REG_ECTYPE. The regex crate has
/// no such validation (an unknown name degrades to a nested class), so
/// scan the post-translation pattern the same way regcomp does.
fn posix_bracket_class_names_valid(pattern: &str) -> bool {
    // The twelve wctype property names (glibc and bionic alike);
    // `word`/`ascii` are regex-crate Perl classes, not regcomp
    // classes, and GNU rejects them with REG_ECTYPE (rubash#1717).
    // The glob matcher has its own 14-name table on purpose
    // (lib/glob/smatch.c cclass_name[]) — that side is not this gate.
    const CLASSES: [&str; 12] = [
        "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
        "upper", "xdigit",
    ];
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => {
                let mut j = i + 1;
                if chars.get(j) == Some(&'^') {
                    j += 1;
                }
                if chars.get(j) == Some(&']') {
                    j += 1;
                }
                while j < chars.len() {
                    if chars[j] == '\\' {
                        j += 1;
                    } else if chars[j] == ']' {
                        break;
                    } else if chars[j] == '[' && chars.get(j + 1) == Some(&':') {
                        let name_end = chars[j + 2..]
                            .iter()
                            .position(|&c| c == ':')
                            .filter(|&pos| chars.get(j + 2 + pos + 1) == Some(&']'))
                            .map(|pos| j + 2 + pos);
                        if let Some(end) = name_end {
                            let name: String = chars[j + 2..end].iter().collect();
                            if !CLASSES.contains(&name.as_str()) {
                                return false;
                            }
                            j = end + 1;
                        }
                    }
                    j += 1;
                }
                i = j;
            }
            _ => {}
        }
        i += 1;
    }
    true
}

fn translate_posix_bracket_classes(pattern: &str, unicode_classes: bool) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len());
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '\u{5c}' && index + 1 < chars.len() {
            out.push(chars[index]);
            out.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if chars[index] == '[' {
            if let Some((next, translated)) =
                translate_bracket_expression(&chars, index, unicode_classes)
            {
                out.push_str(&translated);
                index = next;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Parses one bracket expression starting at start (a [), returning the
/// index just past its closing ] and the regex-crate text with [=c=] and
/// [.s.] spans rewritten. Returns None when the bracket is unterminated,
/// in which case the caller emits the [ literally.
fn translate_bracket_expression(
    chars: &[char],
    start: usize,
    unicode_classes: bool,
) -> Option<(usize, String)> {
    let mut out = String::from("[");
    let mut index = start + 1;
    if index < chars.len() && chars[index] == '^' {
        out.push('^');
        index += 1;
    }
    let mut first = true;
    while index < chars.len() {
        let c = chars[index];
        if c == ']' && !first {
            out.push(']');
            return Some((index + 1, out));
        }
        if c == ']' && first {
            // POSIX: a `]` as the first member is literal data.
            out.push('\u{5c}');
            out.push(']');
            first = false;
            index += 1;
            continue;
        }
        first = false;
        if c == '\u{5c}' {
            // A bare backslash inside a POSIX bracket expression is a
            // literal member (pathexp.c bracket scan has no backslash
            // case) — the regex crate needs it escaped.
            out.push('\u{5c}');
            out.push('\u{5c}');
            index += 1;
            continue;
        }
        if c == '[' {
            if let Some((next, text)) = translate_bracket_prefix(chars, index, unicode_classes) {
                out.push_str(&text);
                index = next;
                continue;
            }
            // A plain literal `[` member (POSIX: `[` has no special meaning
            // inside a bracket expression) must be escaped for the regex
            // crate — `[\][a-z]` is an "unclosed character class" parse
            // error there because the class only closes at the final `]`
            // while the crate re-scans the inner `[`. GNU's own ERE
            // pipeline produces exactly this shape for a class with a
            // quoted leading `]` member (pathexp.c:257 quote_string_for_
            // globbing drops the backslash of a quoted `]` since `]` is
            // not an ere_char, so `[[ =~ [\]\[a-z]+ ]]` reaches regcomp as
            // `[][a-z]+`); glibc regcomp accepts the POSIX form, the regex
            // crate does not (bash-completion tar.bash line 142,
            // rubash#153).
            out.push('\\');
            out.push('[');
            index += 1;
            continue;
        }
        out.push(c);
        index += 1;
    }
    None
}

/// Handles the three [-led bracket prefixes at index. Returns None for a
/// plain literal [ member. [=c=] becomes the equivalent character, [.s.]
/// the collating symbol, and [:class:] passes through verbatim (the regex
/// crate understands POSIX classes) — except in a UTF-8 locale, where
/// [:class:] expands to the locale's Unicode tables instead
/// (rubash#1621, [`posix_class_unicode_expansion`]).
fn translate_bracket_prefix(
    chars: &[char],
    index: usize,
    unicode_classes: bool,
) -> Option<(usize, String)> {
    let close = |open: char, from: usize| -> Option<usize> {
        let mut scan = from;
        while scan + 1 < chars.len() {
            if chars[scan] == open && chars[scan + 1] == ']' {
                return Some(scan + 2);
            }
            scan += 1;
        }
        None
    };
    match chars.get(index + 1) {
        Some('=') => {
            let end = close('=', index + 2)?;
            let body: String = chars[index + 2..end - 2].iter().collect();
            let mut text = String::new();
            for c in body.chars() {
                for e in escape_bracket_char(c) {
                    text.push(e);
                }
            }
            Some((end, text))
        }
        Some('.') => {
            let end = close('.', index + 2)?;
            let body: String = chars[index + 2..end - 2].iter().collect();
            let mut text = String::new();
            for c in body.chars() {
                for e in escape_bracket_char(c) {
                    text.push(e);
                }
            }
            Some((end, text))
        }
        Some(':') => {
            let end = close(':', index + 2)?;
            if unicode_classes {
                let name: String = chars[index + 2..end - 2].iter().collect();
                if let Some(expanded) = posix_class_unicode_expansion(&name) {
                    return Some((end, expanded));
                }
            }
            // The body spans from the leading [ (already part of the text) so
            // it is passed through verbatim without prepending another one.
            let body: String = chars[index..end].iter().collect();
            Some((end, body))
        }
        _ => None,
    }
}

/// The regex-crate text for one POSIX class under a UTF-8 locale,
/// mirroring [`crate::locale::utf8_posix_class_matches`] — the pattern
/// matcher's predicate form of the same glibc tables (rubash#1621).
/// `print`/`graph`/`punct` come out as single negated classes by De
/// Morgan: graph = print minus blank, punct = graph minus alnum.
/// `digit` and `xdigit` return None: the crate's own [:name:]
/// classes already are glibc's ASCII-only tables there. `word` and
/// `ascii` have no expansion either — they are not regcomp classes
/// at all, so the verbatim span reaches the class-name gate in
/// [`posix_bracket_class_names_valid`] and fails with REG_ECTYPE
/// (rubash#1717).
fn posix_class_unicode_expansion(class: &str) -> Option<String> {
    const BLANK_ITEMS: &str = "\\t \\u{1680}\\u{2000}-\\u{200A}\\u{205F}\\u{3000}";
    const CNTRL_ITEMS: &str = "\\p{Cc}\\u{2028}\\u{2029}";
    Some(match class {
        "upper" => "\\p{Uppercase}".to_string(),
        "lower" => "\\p{Lowercase}".to_string(),
        "alpha" => format!(
            "\\p{{Alphabetic}}{}",
            crate::locale::non_ascii_nd_class_items()
        ),
        "alnum" => "\\p{Alphabetic}\\p{Nd}".to_string(),
        "cntrl" => CNTRL_ITEMS.to_string(),
        "print" => format!("[^{}]", CNTRL_ITEMS),
        "graph" => format!("[^{}{}]", CNTRL_ITEMS, BLANK_ITEMS),
        "punct" => format!(
            "[^{}{}\\p{{Alphabetic}}\\p{{Nd}}]",
            CNTRL_ITEMS, BLANK_ITEMS
        ),
        "space" => {
            "\\t\\n\\x0B\\f\\r \\u{1680}\\u{2000}-\\u{200A}\\u{2028}\\u{2029}\\u{205F}\\u{3000}"
                .to_string()
        }
        "blank" => BLANK_ITEMS.to_string(),
        _ => return None,
    })
}

fn escape_bracket_char(c: char) -> Vec<char> {
    if matches!(c, ']' | '\u{5c}' | '[' | '-' | '^') {
        vec!['\u{5c}', c]
    } else {
        vec![c]
    }
}

#[cfg(test)]
mod conditional_pattern_tests {
    //! rubash#1092: the unquoted `[[ == ]]` RHS keeps `\\` as a literal
    //! backslash for the matcher (GNU execute_cmd.c COND_BINARY
    //! patmatch arm + subst.c:4077 cond_expand_word special=1: no quote
    //! removal; sm_loop.c:121 `\\` arm), while expansion-produced `\`
    //! keeps its escape meaning. Cooked RHS spellings below mimic the
    //! lexer: unquoted `a\\b` cooks to `a\x14b` (DATA_BACKSLASH),
    //! `'a\b'` cooks to `a\b` (span bytes verbatim).
    use crate::executor::Executor;

    #[test]
    fn cond_rhs_double_backslash_is_literal_backslash() {
        let mut executor = Executor::new();
        // `[[ ab == a\\b ]]`: no match (GNU rc=1).
        assert!(!executor.conditional_string_binary("ab", "==", "a\u{14}b"));
        // `[[ 'a\b' == a\\b ]]`: match (GNU rc=0).
        assert!(executor.conditional_string_binary("a\\b", "==", "a\u{14}b"));
        // `[[ ab == a\b ]]` (cooked `ab`, like GNU's
        // pass_next_character consuming the quote): match.
        assert!(executor.conditional_string_binary("ab", "==", "ab"));
        // `!=` is the negation over the same pattern reading.
        assert!(executor.conditional_string_binary("ab", "!=", "a\u{14}b"));
        assert!(!executor.conditional_string_binary("a\\b", "!=", "a\u{14}b"));
    }

    #[test]
    fn cond_rhs_expansion_backslash_keeps_escape_meaning() {
        let mut executor = Executor::new();
        executor.set_env("x", "a\\b");
        // x='a\b'; `[[ ab == $x ]]` matches (GNU rc=0): the value's `\`
        // is an escape quoting `b`, unlike lex-level `\\`.
        assert!(executor.conditional_string_binary("ab", "==", "$x"));
        // ...and it does not match a literal-backslash word.
        assert!(!executor.conditional_string_binary("a\\b", "==", "$x"));
    }
}
