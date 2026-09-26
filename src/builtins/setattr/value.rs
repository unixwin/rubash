use std::collections::HashMap;
use std::env;

use super::marks::marked_vars;
use super::COMPOUND_ASSIGNMENT_MARKER;
use super::{ARRAY_VARS, ASSOC_VARS, INTEGER_VARS};
use crate::builtins::declare::storage::{append_array_value, append_assoc_value};
use crate::builtins::declare::storage::{
    format_assoc_storage, format_indexed_array_storage, indexed_array_entries, parse_assoc_words,
};
use crate::executor::markers::STORAGE_WORD_PREFIX;

pub(super) fn is_array_value(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')')
}

pub(super) fn array_attribute_assignment_value(
    value: &str,
    array: bool,
    assoc: bool,
    append: bool,
    env_vars: &HashMap<String, String>,
    name: &str,
) -> (String, bool) {
    // The bool reports whether the bind produced array storage (a compound
    // assignment or a scalar bound to an existing array's element 0) so the
    // caller marks att_array only for real array binds — a quoted literal
    // `(...)` scalar must stay scalar (setattr.def:269-274).
    // GNU setattr.def:240-258 (export -a/-A) and :292 (readonly -a/-A)
    // rewrite the builtin as `declare -gx{a,A}` / `declare -gr{a,A}`, so a
    // compound `(...)` operand is tokenized by
    // expand_compound_array_assignment (arrayfunc.c:557) instead of being
    // stored verbatim. The operand arrives tagged with
    // COMPOUND_ASSIGNMENT_MARKER plus ARRAY_FIELD_SPLIT_MARKER element
    // tags — declare's append_*_value helpers consume both; storing the
    // tagged text raw leaked \x10 and quote syntax into the value.
    let compound = value
        .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
        .unwrap_or(value);
    let tagged = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
    let compound_shape = is_array_value(compound);
    if compound_shape && (tagged || array || assoc) {
        let current = if append {
            env_vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| "()".to_string())
        } else {
            "()".to_string()
        };
        let integer = marked_vars(env_vars, INTEGER_VARS).contains(name);
        let assoc_target = assoc
            || marked_vars(env_vars, ASSOC_VARS).contains(name)
            || marked_vars(env_vars, crate::executor::types::ASSOC_128_VARS).contains(name);
        return (
            if assoc_target {
                append_assoc_value(&current, compound, integer, env_vars)
            } else {
                append_array_value(&current, compound, integer, env_vars)
                    .unwrap_or_else(|_| compound.to_string())
            },
            true,
        );
    }
    // Scalar bind: the flagless operand went through GNU's
    // do_assignment_no_expand (setattr.def:269-274), a plain scalar
    // assignment. On an array target bind_variable (variables.c:3441)
    // routes it to element 0 and keeps the remaining elements
    // (`readonly 'a=(x)'` on a=(p q) stores "(x)" at [0]); on a plain or
    // missing target it is a scalar literal.
    let integer = marked_vars(env_vars, INTEGER_VARS).contains(name);
    let literal = if integer {
        eval_arith_value(compound).to_string()
    } else {
        compound.to_string()
    };
    let assoc_target = assoc
        || marked_vars(env_vars, ASSOC_VARS).contains(name)
        || marked_vars(env_vars, crate::executor::types::ASSOC_128_VARS).contains(name);
    let array_target = array
        || marked_vars(env_vars, ARRAY_VARS).contains(name)
        || env_vars.get(name).is_some_and(|current| {
            current.starts_with(STORAGE_WORD_PREFIX)
                || (current.starts_with('(') && current.ends_with(')'))
        });
    if assoc_target {
        let current = env_vars.get(name).cloned().unwrap_or_default();
        let mut entries = parse_assoc_words(&current);
        match entries.iter_mut().find(|(key, _)| key == "0") {
            Some(entry) if append => entry.1.push_str(&literal),
            Some(entry) => entry.1 = literal,
            None => entries.push(("0".to_string(), literal)),
        }
        return (format_assoc_storage(entries), true);
    }
    if array_target {
        let current = env_vars.get(name).cloned().unwrap_or_default();
        let mut entries = indexed_array_entries(&current);
        let stored = if append {
            let prior = entries.get(&0).cloned().unwrap_or_default();
            format!("{prior}{literal}")
        } else {
            literal
        };
        entries.insert(0, stored);
        return (format_indexed_array_storage(entries), true);
    }
    if !append {
        return (literal, false);
    }
    let mut current = env_vars.get(name).cloned().unwrap_or_default();
    (
        if integer {
            (eval_arith_value(&current) + eval_arith_value(&literal)).to_string()
        } else {
            current.push_str(&literal);
            current
        },
        false,
    )
}

pub(super) fn readonly_error_subject(
    value: &str,
    explicit_array: bool,
    context_name: Option<&str>,
) -> Option<String> {
    // GNU Bash (variables.c/execute_cmd.c): a readonly array reassignment reports
    // `<name>: readonly variable` using the variable name, never the enclosing
    // function name. Drop the subject so the caller prints `{name}: readonly variable`.
    if explicit_array && value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
        // Bash 5.3 (subst.c expand_declaration_argument → make_internal_declare):
        // an *unquoted* compound assignment argument of an assignment builtin is
        // bound during word expansion, before the builtin runs, so the
        // declare-internal sh_readonly (builtins/common.c builtin_error_prolog)
        // prefixes it with `this_command_name` — the name of the enclosing
        // function invocation when the builtin is invoked from a function body
        // (attr.tests:17 `f2: a: readonly variable`, attr1.sub:40 `f: r: ...`).
        // Outside a function this_command_name is NULL and the subject is
        // dropped (plain `{name}: readonly variable`).
        return context_name.map(str::to_string);
    }
    if explicit_array {
        return Some("readonly".to_string());
    }
    None
}

pub(super) fn format_array_value(value: &str) -> String {
    if let Some(rendered) = value.strip_prefix(STORAGE_WORD_PREFIX) {
        return rendered.to_string();
    }
    if value == "()" {
        return "()".to_string();
    }
    let inner = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);
    format!("([0]=\"{}\")", quote_export_value(inner))
}

pub(super) fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        env::var("__RUBASH_SCRIPT_NAME"),
        env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

pub(super) fn split_assignment(arg: &str) -> (&str, bool, Option<&str>) {
    match arg.find('=') {
        Some(index) => {
            let name = &arg[..index];
            let Some(base_name) = name.strip_suffix('+') else {
                return (name, false, Some(&arg[index + 1..]));
            };
            (base_name, true, Some(&arg[index + 1..]))
        }
        None => (arg, false, None),
    }
}

pub(super) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(super) fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

pub(super) fn quote_export_value(value: &str) -> String {
    let mut quoted = String::new();
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted
}
