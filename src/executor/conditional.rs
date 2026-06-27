//! Conditional/test expression evaluation for the executor.
//!
//! Handles `[[ ... ]]` compound commands and the `test` / `[` builtins,
//! including pattern matching, regex matching, file tests, and numeric
//! comparisons.

use std::collections::BTreeMap;

use super::{
    mark_env_name, pattern_contains_glob, unescape_remaining_shell_escapes, ARRAY_VARS,
    Executor,
};
use crate::executor::arithmetic::eval_mutable_arith_value_with_random;
use crate::executor::arrays::format_indexed_array_storage;

impl Executor {
    pub(super) fn execute_conditional(&mut self, args: &[String]) -> i32 {
        // TODO(parse.y/execute_cmd.c/test.c): Bash `[[` is a compound command
        // with its own parser, operators, pattern matching, and short-circuit
        // logic. Keep extending this bridge with test.c-compatible primitives.
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
                !crate::builtins::test::variable_is_set(operand, &self.env_vars),
            ),
            [op, operand] if op == "-v" => i32::from(!crate::builtins::test::variable_is_set(
                operand,
                &self.env_vars,
            )),
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

    pub(super) fn conditional_string_binary(&mut self, left: &str, op: &str, right: &str) -> bool {
        let left = self.expand_word(left);
        let right = self.expand_word(right);
        match op {
            "=" | "==" => conditional_pattern_or_string_matches(&left, &right),
            "!=" => !conditional_pattern_or_string_matches(&left, &right),
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

    pub(super) fn conditional_numeric_binary(
        &mut self,
        left: &str,
        op: &str,
        right: &str,
    ) -> bool {
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

pub(super) fn case_pattern_matches(pattern: &str, word: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    case_pattern_matches_at(&pattern, 0, &word, 0)
}

pub(super) fn is_conditional_file_unary(op: &str) -> bool {
    matches!(
        op,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-h"
            | "-L"
            | "-k"
            | "-p"
            | "-r"
            | "-s"
            | "-S"
            | "-t"
            | "-u"
            | "-w"
            | "-x"
            | "-O"
            | "-G"
            | "-N"
    )
}

pub(super) fn is_conditional_file_binary(op: &str) -> bool {
    matches!(op, "-nt" | "-ot" | "-ef")
}

pub(super) fn conditional_logical_index(args: &[String], op: &str) -> Option<usize> {
    let end = conditional_effective_len(args);
    let mut depth = 0usize;
    for index in (0..end).rev() {
        match args[index].as_str() {
            ")" => depth += 1,
            "(" => depth = depth.saturating_sub(1),
            value if value == op && depth == 0 && index > 0 && index + 1 < end => {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

pub(super) fn conditional_outer_parentheses(args: &[String]) -> Option<&[String]> {
    let end = conditional_effective_len(args);
    if end < 2 || args.first().map(String::as_str) != Some("(") {
        return None;
    }

    let mut depth = 0usize;
    for (index, arg) in args[..end].iter().enumerate() {
        match arg.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && index != end - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }

    (depth == 0 && args[end - 1] == ")").then_some(&args[1..end - 1])
}

pub(super) fn conditional_regex_operands(args: &[String]) -> Option<(&str, String)> {
    let end = conditional_effective_len(args);
    let op = args[..end].iter().position(|word| word == "=~")?;
    if op != 1 || op + 1 >= end {
        return None;
    }

    Some((args[0].as_str(), args[op + 1..end].join("")))
}

pub(super) fn conditional_effective_len(args: &[String]) -> usize {
    args.len() - usize::from(args.last().map(String::as_str) == Some("]]"))
}

pub(super) fn conditional_pattern_or_string_matches(left: &str, right: &str) -> bool {
    if pattern_contains_glob(right) {
        case_pattern_matches(right, left)
    } else {
        left == right
    }
}

pub(super) fn simple_grep_pattern_matches(line: &str, pattern: &str) -> bool {
    if let Some(pattern) = pattern.strip_prefix('^') {
        line.starts_with(pattern)
    } else {
        line.contains(pattern)
    }
}

pub(super) fn restore_numeric_decimal_regex_escapes(pattern: &str) -> String {
    pattern
        .replace("([0-9]*).([0-9]+)", "([0-9]*)\\.([0-9]+)")
        .replace("([0-9]+)(.([0-9]+))?", "([0-9]+)(\\.([0-9]+))?")
        .replace("(.*).(.*)", "(.*)\\.(.*)")
}

fn case_pattern_matches_at(
    pattern: &[char],
    p_index: usize,
    word: &[char],
    w_index: usize,
) -> bool {
    if p_index == pattern.len() {
        return w_index == word.len();
    }

    match pattern[p_index] {
        '\x18' => {
            w_index < word.len()
                && word[w_index] == '\\'
                && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
        '*' => {
            case_pattern_matches_at(pattern, p_index + 1, word, w_index)
                || (w_index < word.len()
                    && case_pattern_matches_at(pattern, p_index, word, w_index + 1))
        }
        '?' => {
            w_index < word.len() && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
        '[' => {
            let Some((matches_class, next_index)) =
                case_bracket_expression_matches(pattern, p_index, word.get(w_index).copied())
            else {
                return w_index < word.len()
                    && pattern[p_index] == word[w_index]
                    && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1);
            };

            matches_class && case_pattern_matches_at(pattern, next_index, word, w_index + 1)
        }
        '\\' if p_index + 1 < pattern.len() => {
            w_index < word.len()
                && pattern[p_index + 1] == word[w_index]
                && case_pattern_matches_at(pattern, p_index + 2, word, w_index + 1)
        }
        literal => {
            w_index < word.len()
                && literal == word[w_index]
                && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
    }
}

fn case_bracket_expression_matches(
    pattern: &[char],
    start: usize,
    candidate: Option<char>,
) -> Option<(bool, usize)> {
    let mut index = start + 1;
    if index >= pattern.len() {
        return None;
    }

    let negated = matches!(pattern[index], '!' | '^');
    if negated {
        index += 1;
    }

    let mut matched = false;
    let mut saw_member = false;
    let candidate = candidate?;
    while index < pattern.len() {
        if pattern[index] == ']' && saw_member {
            return Some((if negated { !matched } else { matched }, index + 1));
        }

        let current = pattern[index];
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            let end = pattern[index + 2];
            if current <= candidate && candidate <= end {
                matched = true;
            }
            saw_member = true;
            index += 3;
        } else {
            if current == candidate {
                matched = true;
            }
            saw_member = true;
            index += 1;
        }
    }

    None
}
