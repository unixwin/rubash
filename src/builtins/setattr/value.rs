use super::COMPOUND_ASSIGNMENT_MARKER;
use super::{ARRAY_VARS, ASSOC_VARS, INTEGER_VARS};
use crate::builtins::declare::storage::{append_array_value, append_assoc_value};
use crate::builtins::declare::storage::{
    append_baseline_mirror, fold_indexed_storage_dupes, format_assoc_storage,
    format_indexed_array_storage, indexed_array_entries, parse_assoc_words,
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
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
    oob_word: &mut Option<String>,
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
            // GNU arrayfunc.c:710-743: an append continues the live ARRAY
            // object. A declared-but-unset array is empty; an existing scalar
            // (even "") keeps element 0. See append_baseline_mirror.
            append_baseline_mirror(
                env_vars.get(name).map(String::as_str),
                env_vars.is_marked(crate::builtins::set::DECLARED_UNSET_VARS, name),
            )
        } else {
            "()".to_string()
        };
        let integer = env_vars.is_marked(INTEGER_VARS, name);
        let assoc_target = assoc
            || env_vars.is_marked(ASSOC_VARS, name)
            || env_vars.is_marked(crate::executor::types::ASSOC_128_VARS, name);
        return (
            if assoc_target {
                append_assoc_value(&current, compound, integer, env_vars)
            } else {
                append_array_value(&current, compound, integer, env_vars, oob_word)
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
    let integer = env_vars.is_marked(INTEGER_VARS, name);
    let literal = if integer {
        eval_integer_operand(compound, env_vars).to_string()
    } else {
        compound.to_string()
    };
    let assoc_target = assoc
        || env_vars.is_marked(ASSOC_VARS, name)
        || env_vars.is_marked(crate::executor::types::ASSOC_128_VARS, name);
    let array_target = array
        || env_vars.is_marked(ARRAY_VARS, name)
        || env_vars.get(name).is_some_and(|current| {
            current.starts_with(STORAGE_WORD_PREFIX)
                || (current.starts_with('(') && current.ends_with(')'))
        });
    if assoc_target {
        let current = env_vars.get(name).cloned().unwrap_or_default();
        let mut entries = parse_assoc_words(&current);
        match entries.iter_mut().find(|(key, _)| key == "0") {
            Some(entry) if append && integer => {
                entry.1 = integer_appended_value(&entry.1, compound, env_vars)
            }
            Some(entry) if append => entry.1.push_str(&literal),
            Some(entry) => entry.1 = literal,
            None => entries.push(("0".to_string(), literal)),
        }
        return (format_assoc_storage(entries), true);
    }
    if array_target {
        let current = env_vars.get(name).cloned().unwrap_or_default();
        let mut entries = indexed_array_entries(&current);
        let stored = if append && integer {
            let prior = entries.get(&0).cloned().unwrap_or_default();
            integer_appended_value(&prior, compound, env_vars)
        } else if append {
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
            integer_appended_value(&current, compound, env_vars)
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
        // rubash#616: fold transient duplicates from the append-updated
        // write path (identity on canonical mirrors).
        return fold_indexed_storage_dupes(value, rendered);
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

pub(crate) fn split_assignment(arg: &str) -> (&str, bool, Option<&str>) {
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

pub(crate) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// GNU variables.c:2905 make_variable_value, integer branch: binding an
/// integer variable stores the *evaluated* expression, and an ASS_APPEND
/// bind stores `evalexp(old value) + evalexp(new value)` — arithmetic
/// addition through the real evaluator, never text concatenation
/// (rubash#1710). This replaces the residual split('+') evaluator this
/// file used to carry: it could not see `2*3`, parentheses, variable
/// references, or octal literals, so an integer append onto an array's
/// element 0 concatenated the operand text (`10` + `5` stored `105`).
fn eval_integer_operand(text: &str, env_vars: &crate::shell::var_table::VarTable) -> i128 {
    crate::executor::arithmetic::eval_conditional_arith_value(text, env_vars).unwrap_or(0)
}

/// The element-0 / scalar text stored by an integer `name+=operand`
/// bind: the prior text ("" for an unset element, evaluating to 0) and
/// the operand are both evaluated, then added — make_variable_value's
/// ASS_APPEND integer rule above.
fn integer_appended_value(
    prior: &str,
    operand: &str,
    env_vars: &crate::shell::var_table::VarTable,
) -> String {
    (eval_integer_operand(prior, env_vars) + eval_integer_operand(operand, env_vars)).to_string()
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

/// Display quoting for the no-arg `export -p` / `readonly -p` listings:
/// GNU setattr.def show_var_attributes applies the same rule as
/// `declare -p` (setattr.def:528-531) — strtrans.c ansic_quote ($'...')
/// when the value holds non-printing bytes, sh_double_quote otherwise.
/// The listings used to wrap quote_export_value in literal double quotes
/// unconditionally, embedding raw control bytes (a value with a newline
/// printed across two lines) where GNU prints $'x\ny' (rubash#1184
/// finding 2). Mirrors builtins/declare/storage.rs quote_declare_value,
/// including the raw-byte-marker byte view.
pub(super) fn quote_listing_value(value: &str) -> String {
    if crate::executor::substitution_metadata::has_raw_byte_marker_pair(value) {
        return crate::executor::ansic_quote(value);
    }
    let visible = crate::locale::decode_to_visible_text(value);
    if crate::executor::ansic_shouldquote(&visible) {
        return crate::executor::ansic_quote(&visible);
    }
    format!("\"{}\"", quote_export_value(&visible))
}
