use super::{
    arithmetic_operators, arithmetic_variables, is_arithmetic_assignment_operator,
    is_arithmetic_comparison_operator, ArithmeticExpansion, CommandNode,
};

pub(super) fn record_arithmetic_expansions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    let expansions = arithmetic_expansions_in_word(word)
        .into_iter()
        .map(|mut expansion| {
            expansion.word_index = Some(word_index);
            expansion
        });
    command.arithmetic_expansions.extend(expansions);
}

pub(super) fn record_arithmetic_expansions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let expansions = arithmetic_expansions_in_word(value)
        .into_iter()
        .map(|mut expansion| {
            expansion.assignment_name = Some(assignment_name.to_string());
            expansion.word_index = word_index;
            expansion
        });
    command.arithmetic_expansions.extend(expansions);
}

fn arithmetic_expansions_in_word(word: &str) -> Vec<ArithmeticExpansion> {
    let chars = word.chars().collect::<Vec<_>>();
    let mut expansions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$'
            && chars.get(index + 1) == Some(&'(')
            && chars.get(index + 2) == Some(&'(')
        {
            if let Some((expansion, next_index)) = arithmetic_expansion(&chars, index) {
                expansions.push(expansion);
                index = next_index;
                continue;
            }
        }
        index += 1;
    }
    expansions
}

fn arithmetic_expansion(chars: &[char], start: usize) -> Option<(ArithmeticExpansion, usize)> {
    let mut index = start + 3;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index + 1 < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double && depth > 0 => depth -= 1,
            ')' if !single && !double && chars.get(index + 1) == Some(&')') => {
                let expression = chars[start + 3..index].iter().collect::<String>();
                return Some((
                    arithmetic_expansion_node(chars, start, index, expression),
                    index + 2,
                ));
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn arithmetic_expansion_node(
    chars: &[char],
    start: usize,
    end: usize,
    expression: String,
) -> ArithmeticExpansion {
    let operators = arithmetic_operators(&expression);
    ArithmeticExpansion {
        text: chars[start..=end + 1].iter().collect(),
        variables: arithmetic_variables(&expression),
        has_assignment: operators
            .iter()
            .any(|operator| is_arithmetic_assignment_operator(&operator.text)),
        has_comparison: operators
            .iter()
            .any(|operator| is_arithmetic_comparison_operator(&operator.text)),
        has_logical: operators
            .iter()
            .any(|operator| matches!(operator.text.as_str(), "&&" | "||" | "!")),
        has_update: operators
            .iter()
            .any(|operator| matches!(operator.text.as_str(), "++" | "--")),
        operators,
        expression,
        word_index: None,
        assignment_name: None,
    }
}
