//! functions module.
//!
//! GNU Bash source ownership:
// - arrayfunc.c
// - arrayfunc.h

pub fn append_indexed_value(current: &str, value: &str, integer: bool) -> String {
    let mut elements = crate::shell::arrays::indexed::values(current);
    let scalar_append = integer && !value.starts_with('(');
    for token in crate::shell::arrays::indexed::assignment_tokens(value) {
        if let Some((left, rhs)) = token.split_once("+=") {
            if let Some(index) = crate::shell::arrays::indexed::assignment_index(left) {
                while elements.len() <= index {
                    elements.push(String::new());
                }
                elements[index] =
                    (eval_arith_value(&elements[index]) + eval_arith_value(rhs)).to_string();
                continue;
            }
        }

        if let Some((left, rhs)) = token.split_once('=') {
            if let Some(index) = crate::shell::arrays::indexed::assignment_index(left) {
                while elements.len() <= index {
                    elements.push(String::new());
                }
                elements[index] = rhs.to_string();
                continue;
            }
        }

        if scalar_append && !elements.is_empty() {
            elements[0] = (eval_arith_value(&elements[0]) + eval_arith_value(&token)).to_string();
        } else {
            elements.push(token);
        }
    }

    if integer {
        for element in &mut elements {
            *element = eval_arith_value(element).to_string();
        }
    }

    format!("({})", elements.join(" "))
}

fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}
