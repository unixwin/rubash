use super::{parse_array_tokens, split_storage_words, unquote_storage_value};

pub(in crate::builtins) fn parse_assoc_words(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };
    let tokens = merge_assoc_subscript_tokens(split_storage_words(inner).collect());
    // GNU ASSOC_KVPAIR_ASSIGNMENT (arrayfunc.c kvpair_assignment_p +
    // assign_assoc_from_kvlist): when the first compound word carries no
    // [key]= assignment form, the words alternate key, value (assoc11:
    // declare -A inside=(a 1 b 2 c 3)).
    match tokens.first() {
        Some(first) if assoc_assignment_token(first).is_none() && !first.starts_with('[') => {
            return tokens
                .chunks(2)
                .filter_map(|pair| {
                    let key = pair.first()?;
                    Some((
                        unquote_storage_value(key),
                        pair.get(1)
                            .map(|value| unquote_storage_value(value))
                            .unwrap_or_default(),
                    ))
                })
                .collect();
        }
        _ => {}
    }

    tokens
        .into_iter()
        .filter_map(|part| {
            // Storage pairs are `[key]=value`; split at the quote-aware
            // subscript close so a quoted `=` inside a stored key stays in
            // the key (assoc.tests assoc4: ["a]=test1;#a"]="123").
            if let Some((key, value, _)) = assoc_assignment_token(&part) {
                let key = crate::executor::arithmetic::decode_arithmetic_assoc_key(key)
                    .unwrap_or_else(|| unquote_storage_value(key));
                return Some((key, unquote_storage_value(value)));
            }
            let (key, value) = part.split_once('=')?;
            Some((
                unquote_storage_value(key.trim_start_matches('[').trim_end_matches(']')),
                unquote_storage_value(value),
            ))
        })
        .collect()
}
pub(in crate::builtins) fn append_assoc_value(
    current: &str,
    value: &str,
    integer: bool,
    variables: &std::collections::HashMap<String, String>,
) -> String {
    // GNU arrayfunc.c assign_compound_array_list / bind_assoc_variable: when
    // the array carries the integer attribute, every element value is
    // evaluated as an arithmetic expression before it is stored
    // (assoc.tests: declare -Ai chaff=([one]=3+7) stores 10). The evaluation
    // resolves shell variables, so it uses the real evaluator.
    let eval_element = |raw: &str| -> String {
        if integer {
            crate::executor::arithmetic::eval_conditional_arith_value(raw, variables)
                .unwrap_or(0)
                .to_string()
        } else {
            raw.to_string()
        }
    };
    let mut entries = parse_assoc_words(current);
    let tokens = merge_assoc_subscript_tokens(parse_array_tokens(value));
    // DECLARE-path mode decision (GNU declare.def routes the compound
    // operand with W_ASSIGNMENT words, unlike the plain-assignment path in
    // executor/assignment_helpers.rs): a first word that is a complete
    // [key]=value token selects strict mode; any other first word ([x] --
    // no '=' -- or a=b) puts the list into alternating literal key/value
    // pairs (assoc-kv3 probe D1: declare -A a=([x] one [y] two) stores
    // keys "[x]"/"[y]"; D2: a=(a=b c=d) stores [a=b]="c=d").
    let explicit_subscripts = tokens
        .first()
        .map(|token| {
            // The quote-aware [key]=value scanner (the same parse the loop
            // below runs) decides the mode: a position-based `]`-before-`=`
            // heuristic misread an element whose VALUE contains `]` —
            // `declare -A a=([k]=$X'Q')` with X='\[\e[33m\]' arrives here
            // as `[k]="\[\e[33m\]Q"` and fell into kvpair mode, storing the
            // whole word as the key with an empty value (rubash#295).
            // `[x]` (no `=` tail) and `a=b` still parse as None.
            assoc_assignment_token(token).is_some()
        })
        .unwrap_or(false);

    if !explicit_subscripts {
        for pair in tokens.chunks(2) {
            let Some(key) = pair.first() else {
                continue;
            };
            let key = unquote_storage_value(key);
            let value = pair
                .get(1)
                .map(|value| unquote_storage_value(value))
                .unwrap_or_default();
            entries.push((key, eval_element(&value)));
        }
        return format_assoc_storage(entries);
    }

    for token in tokens {
        if let Some((key, rhs, append)) = assoc_assignment_token(&token) {
            // \x1e-hex encoded keys from rewrite_compound_element_subscripts
            // decode back to their literal text before storage.
            let key = crate::executor::arithmetic::decode_arithmetic_assoc_key(key)
                .unwrap_or_else(|| unquote_storage_value(key));
            let rhs = unquote_storage_value(rhs);
            if append {
                if let Some((_, entry_value)) = entries
                    .iter_mut()
                    .rev()
                    .find(|(entry_key, _)| entry_key == &key)
                {
                    if integer {
                        *entry_value = (crate::executor::arithmetic::eval_conditional_arith_value(
                            entry_value,
                            variables,
                        )
                        .unwrap_or(0)
                            + crate::executor::arithmetic::eval_conditional_arith_value(
                                &rhs, variables,
                            )
                            .unwrap_or(0))
                        .to_string();
                    } else {
                        entry_value.push_str(&rhs);
                        *entry_value = eval_element(entry_value);
                    }
                } else {
                    entries.push((key, eval_element(&rhs)));
                }
                continue;
            }
            // GNU bind_assoc_variable -> assoc_insert: a repeated key keeps
            // its first-insert slot and the new value replaces the old one
            // (assoc13: declare a[@]=at2 overwrites [@" at").
            if let Some((_, entry_value)) = entries
                .iter_mut()
                .rev()
                .find(|(entry_key, _)| entry_key == &key)
            {
                *entry_value = eval_element(&rhs);
            } else {
                entries.push((key, eval_element(&rhs)));
            }
            continue;
        }
        entries.push((
            "0".to_string(),
            eval_element(&unquote_storage_value(&token)),
        ));
    }

    format_assoc_storage(entries)
}

/// Split an assoc assignment token (`[key]=value` / `[key]+=value`) at the
/// subscript-closing `]`, honoring quotes and escapes: GNU parses the raw
/// compound assignment text so quoted `]`/`=` inside a key stay literal
/// (assoc.tests assoc4: ["a]=test1;#a"]="123").
fn assoc_assignment_token(token: &str) -> Option<(&str, &str, bool)> {
    let rest = token.strip_prefix('[')?;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ']' if !in_single && !in_double => {
                let key = &rest[..index];
                let after = &rest[index + 1..];
                if let Some(value) = after.strip_prefix("+=") {
                    return Some((key, value, true));
                }
                let value = after.strip_prefix('=')?;
                return Some((key, value, false));
            }
            _ => {}
        }
    }
    None
}

/// Quote-aware unclosed `[` / quote state of a storage token: whitespace between an
/// unclosed `[` and its `]`, or inside an unclosed quote, is part of the
/// assoc key/value, not a word separator (assoc.tests:
/// wheat=([six]=6 [foo bar]="qux qix" )). Returns (bracket_depth, in_single,
/// in_double).
fn assoc_token_scan_state(token: &str) -> (usize, bool, bool) {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    // [key]=value structure: the first unquoted ']' closes the subscript
    // and every later bracket belongs to the value text. A stored key like
    // '[' produces the token '[[]=lbracket', which must read as closed:
    // counting nested brackets in the key/value text made the merger glue
    // unrelated pairs together.
    let mut subscript_open = token.starts_with('[');
    let mut after_subscript = false;
    for (i, ch) in token.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ']' if !in_single && !in_double => {
                if subscript_open || i > 0 {
                    subscript_open = false;
                    after_subscript = true;
                }
            }
            // A `[` only opens a subscript at word start
            // (arrayfunc.c assign_compound_array_list): a bare `[` inside a
            // k/v-pair word is data, not an unclosed subscript — `foo[bar`
            // must not glue the following words into one key (assoc11.sub).
            _ => {}
        }
    }
    (if subscript_open { 1 } else { 0 }, in_single, in_double)
}

/// Re-join tokens that were split on whitespace inside an unclosed `[...]`
/// subscript or an unclosed quote so assoc pair parsing sees GNU's raw
/// subscript/value text.
fn merge_assoc_subscript_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in tokens {
        // Strip the indexed-storage word-expansion tag (\x10): the assoc
        // path neither field-splits nor globs (GNU assign_assoc_from_
        // kvlist), and the tag pushed control-byte elements past
        // unquote_storage_value's `$'...'` arm into the bare-value
        // escape-drop loop (assoc15.sub; see the executor-side twin in
        // assignment_helpers.rs merge_assoc_subscript_tokens).
        let token = token
            .strip_prefix(crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER)
            .map(str::to_string)
            .unwrap_or(token);
        match out.last_mut() {
            Some(last)
                if {
                    let (depth, in_single, in_double) = assoc_token_scan_state(last);
                    depth > 0 || in_single || in_double
                } =>
            {
                last.push(' ');
                last.push_str(&token);
            }
            _ => out.push(token),
        }
    }
    out
}

pub(in crate::builtins) fn format_assoc_storage(entries: Vec<(String, String)>) -> String {
    format!(
        "({})",
        entries
            .into_iter()
            .map(|(key, value)| {
                format!(
                    "[{}]={}",
                    quote_assoc_key(&key),
                    quote_assoc_storage_value(&value)
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    )
}

pub(in crate::builtins::declare) fn quote_assoc_key(key: &str) -> String {
    // The storage form is re-parsed by split_storage_words on every
    // read: a bare ``` or `$` opens a substitution span in that
    // tokenizer and glues the following pairs into this pair's value
    // (assoc9.sub), so they must force quoting alongside whitespace,
    // quotes, backslash and `]`.
    if !key.is_empty()
        && !key
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '\'' | '"' | '\\' | ']' | '`' | '$'))
    {
        return key.to_string();
    }

    quote_assoc_storage_value_forced(key)
}

pub(in crate::builtins::declare) fn quote_assoc_storage_value(value: &str) -> String {
    if value.contains(['\n', '\r', '\'']) {
        return super::quote_declare_value(value);
    }

    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\' | '`' | '$'))
    {
        return value.to_string();
    }

    let mut quoted = String::from("\"");
    for ch in value.chars() {
        // `$` and ``` are still substitution openers inside "..."
        // for the storage tokenizer; unquote_storage_value decodes all
        // four escapes.
        if matches!(ch, '"' | '\\' | '`' | '$') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

fn quote_assoc_storage_value_forced(value: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        // `$` and ``` are still substitution openers inside "..."
        // for the storage tokenizer; unquote_storage_value decodes all
        // four escapes.
        if matches!(ch, '"' | '\\' | '`' | '$') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternating_bracket_words_are_literal_keys() {
        let variables = std::collections::HashMap::new();
        assert_eq!(
            append_assoc_value("()", "([x] one [y] two)", false, &variables),
            "([\"[x]\"]=one [\"[y]\"]=two)"
        );
    }
}
