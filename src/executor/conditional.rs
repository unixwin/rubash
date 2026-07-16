//! Conditional/test expression evaluation for the executor.
//!
//! Handles `[[ ... ]]` compound commands and the `test` / `[` builtins,
//! including pattern matching, regex matching, file tests, and numeric
//! comparisons.

use std::collections::BTreeMap;

use super::{
    is_marked_var, mark_env_name, unescape_remaining_shell_escapes, Executor, ARRAY_VARS,
    NAMEREF_VARS,
};
use crate::executor::arithmetic::eval_mutable_arith_value_with_random;
use crate::executor::arrays::format_indexed_array_storage;

mod args;
mod extglob;
mod pattern;

use args::{
    conditional_effective_len, conditional_logical_index, conditional_outer_parentheses,
    conditional_pattern_or_string_matches, conditional_regex_operands, is_conditional_file_binary,
    is_conditional_file_unary, reassemble_extglob_args, restore_numeric_decimal_regex_escapes,
};

pub(super) use args::simple_grep_pattern_matches;
pub(in crate::executor) use extglob::{
    extglob_case_pattern_matches, extglob_case_pattern_matches_nocase,
};
pub(super) use pattern::{case_pattern_matches, case_pattern_matches_nocase};

impl Executor {
    pub(super) fn execute_conditional_command(
        &mut self,
        command: &crate::parser::ConditionalCommand,
    ) -> i32 {
        if let Some(status) =
            self.conditional_status_with_metadata(&command.args, &command.arg_metadata)
        {
            return status;
        }
        self.execute_conditional(&command.args)
    }

    pub(super) fn execute_conditional(&mut self, args: &[String]) -> i32 {
        // TODO(parse.y/execute_cmd.c/test.c): Bash `[[` is a compound command
        // with its own parser, operators, pattern matching, and short-circuit
        // logic. Keep extending this bridge with test.c-compatible primitives.
        let args = reassemble_extglob_args(args);
        let args = args.as_slice();
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
            [not, rest @ ..] if not == "!" => i32::from(self.execute_conditional(rest) == 0),
            [op, operand, end] if op == "-v" && end == "]]" => i32::from(
                !crate::builtins::test::variable_is_set(&self.expand_word(operand), &self.env_vars),
            ),
            [op, operand] if op == "-v" => i32::from(!crate::builtins::test::variable_is_set(
                &self.expand_word(operand),
                &self.env_vars,
            )),
            [op, operand, end] if op == "-R" && end == "]]" => {
                i32::from(!is_marked_var(&self.env_vars, NAMEREF_VARS, operand))
            }
            [op, operand] if op == "-R" => {
                i32::from(!is_marked_var(&self.env_vars, NAMEREF_VARS, operand))
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
            [operand, end] if end == "]]" => i32::from(self.expand_word(operand).is_empty()),
            [operand] => i32::from(self.expand_word(operand).is_empty()),
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
            [left, op, right, end]
                if matches!(op.as_str(), "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge")
                    && end == "]]" =>
            {
                i32::from(!self.conditional_numeric_binary(left, op, right))
            }
            [left, op, right]
                if matches!(op.as_str(), "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge") =>
            {
                i32::from(!self.conditional_numeric_binary(left, op, right))
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

    fn conditional_status_with_metadata(
        &mut self,
        args: &[String],
        metadata: &[crate::parser::WordMetadata],
    ) -> Option<i32> {
        if args.len() != metadata.len() {
            return None;
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
            if not == "!" {
                let status = self
                    .conditional_status_with_metadata(rest, &metadata[1..])
                    .unwrap_or_else(|| self.execute_conditional(rest));
                return Some(i32::from(status == 0));
            }
        }

        self.quoted_conditional_pattern_status(args, metadata)
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
                    && metadata
                        .get(2)
                        .is_some_and(|metadata| !metadata.word_quotes.is_empty()) =>
            {
                let left = self.expand_word(left);
                let right = self.expand_word(right);
                let matched = left == right;
                Some(match op.as_str() {
                    "!=" => i32::from(matched),
                    _ => i32::from(!matched),
                })
            }
            [left, op, right]
                if matches!(op.as_str(), "=" | "==" | "!=")
                    && metadata
                        .get(2)
                        .is_some_and(|metadata| !metadata.word_quotes.is_empty()) =>
            {
                let left = self.expand_word(left);
                let right = self.expand_word(right);
                let matched = left == right;
                Some(match op.as_str() {
                    "!=" => i32::from(matched),
                    _ => i32::from(!matched),
                })
            }
            _ => None,
        }
    }

    pub(super) fn conditional_string_binary(&mut self, left: &str, op: &str, right: &str) -> bool {
        let left = self.expand_word(left);
        let right = self.expand_word(right);
        let extglob = crate::builtins::shopt::option_enabled(&self.env_vars, "extglob")
            || contains_extglob_pattern(&right);
        let nocasematch = crate::builtins::shopt::option_enabled(&self.env_vars, "nocasematch");
        match op {
            "=" | "==" if extglob && nocasematch => {
                extglob_case_pattern_matches_nocase(&right, &left)
            }
            "=" | "==" if extglob => extglob_case_pattern_matches(&right, &left),
            "=" | "==" => conditional_pattern_or_string_matches(&left, &right, nocasematch),
            "!=" if extglob && nocasematch => !extglob_case_pattern_matches_nocase(&right, &left),
            "!=" if extglob => !extglob_case_pattern_matches(&right, &left),
            "!=" => !conditional_pattern_or_string_matches(&left, &right, nocasematch),
            "=~" => self.conditional_regex_match(&left, &right),
            "<" => left < right,
            ">" => left > right,
            _ => false,
        }
    }

    pub(super) fn conditional_string_unary(&self, op: &str, operand: &str) -> bool {
        let value = self.expand_word(operand);
        match op {
            "-n" => !value.is_empty(),
            "-z" => value.is_empty(),
            _ => false,
        }
    }

    pub(super) fn conditional_shell_option_unary(&self, operand: &str) -> bool {
        let name = self.expand_word(operand);
        crate::builtins::set::is_shell_option(&name)
            && crate::builtins::set::shell_option_enabled(&self.env_vars, &name)
    }

    pub(super) fn conditional_file_unary(&self, op: &str, operand: &str) -> bool {
        let args = vec![op.to_string(), self.expand_word(operand)];
        crate::builtins::test::execute(&args, false, &self.env_vars).unwrap_or(1) == 0
    }

    pub(super) fn conditional_file_binary(&self, left: &str, op: &str, right: &str) -> bool {
        let args = vec![
            self.expand_word(left),
            op.to_string(),
            self.expand_word(right),
        ];
        crate::builtins::test::execute(&args, false, &self.env_vars).unwrap_or(1) == 0
    }

    pub(super) fn conditional_regex_match(&mut self, left: &str, right: &str) -> bool {
        let right = unescape_remaining_shell_escapes(right);
        let right = restore_numeric_decimal_regex_escapes(&right);
        let Ok(regex) = regex::Regex::new(&right) else {
            return false;
        };
        let Some(captures) = regex.captures(left) else {
            self.clear_bash_rematch();
            return false;
        };

        self.store_bash_rematch(captures);
        true
    }

    pub(super) fn store_bash_rematch(&mut self, captures: regex::Captures<'_>) {
        let entries: BTreeMap<usize, String> = captures
            .iter()
            .enumerate()
            .filter_map(|(index, capture)| {
                capture.map(|matched| (index, matched.as_str().to_string()))
            })
            .collect();
        self.env_vars.insert(
            "BASH_REMATCH".to_string(),
            format_indexed_array_storage(entries),
        );
        mark_env_name(&mut self.env_vars, ARRAY_VARS, "BASH_REMATCH");
    }

    pub(super) fn clear_bash_rematch(&mut self) {
        self.env_vars.insert(
            "BASH_REMATCH".to_string(),
            format_indexed_array_storage(BTreeMap::new()),
        );
        mark_env_name(&mut self.env_vars, ARRAY_VARS, "BASH_REMATCH");
    }

    pub(super) fn conditional_regex_match_status(&mut self, left: &str, right: &str) -> i32 {
        let left = self.expand_word(left);
        let right = unescape_remaining_shell_escapes(&self.expand_word(right));
        let right = restore_numeric_decimal_regex_escapes(&right);
        let Ok(regex) = regex::Regex::new(&right) else {
            return 2;
        };
        let Some(captures) = regex.captures(&left) else {
            self.clear_bash_rematch();
            return 1;
        };

        self.store_bash_rematch(captures);
        0
    }

    pub(super) fn conditional_numeric_binary(&mut self, left: &str, op: &str, right: &str) -> bool {
        let left = self.expand_word(left);
        let right = self.expand_word(right);
        let Some(left) = eval_mutable_arith_value_with_random(
            &left,
            &mut self.env_vars,
            Some(&self.random_state),
        ) else {
            return false;
        };
        let Some(right) = eval_mutable_arith_value_with_random(
            &right,
            &mut self.env_vars,
            Some(&self.random_state),
        ) else {
            return false;
        };
        match op {
            "-eq" => left == right,
            "-ne" => left != right,
            "-lt" => left < right,
            "-le" => left <= right,
            "-gt" => left > right,
            "-ge" => left >= right,
            _ => false,
        }
    }
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
