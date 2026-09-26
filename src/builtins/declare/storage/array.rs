use std::collections::{BTreeMap, HashMap};

use super::{
    eval_arith_value, parse_array_tokens, parse_array_words, split_indexed_tagged_token,
    split_storage_words, unquote_storage_value,
};
use crate::executor::glob::{pathname_expand_word, PathnameExpansion};
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::markers::STORAGE_WORD_PREFIX_STR;

/// GNU arrayfunc.c quote_array_assignment_chars (arrayfunc.c:1107+) marks
/// `[subscript]=value` / `[subscript]+=value` compound words W_NOGLOB: they
/// are assignment words, never pathname-expanded (niubash #121:
/// `declare -a a=([0]=nope-*)` keeps the literal element under nullglob).
fn token_is_subscript_assignment(unquoted_token: &str) -> bool {
    if let Some((left, _)) = unquoted_token.split_once('=') {
        if array_assignment_has_subscript(left) {
            return true;
        }
    }
    if let Some((left, _)) = unquoted_token.split_once("+=") {
        if array_assignment_has_subscript(left) {
            return true;
        }
    }
    false
}

/// GNU assign_compound_array_list (arrayfunc.c:700+). `Err(pattern)`
/// reports a failglob pathname-expansion failure on one element word —
/// expand_compound_array_assignment (arrayfunc.c:557) aborts the operand
/// before bind, so `declare -a g=(zzz-*)` leaves g unset.
pub(in crate::builtins) fn append_array_value(
    current: &str,
    value: &str,
    integer: bool,
    env_vars: &HashMap<String, String>,
) -> Result<String, String> {
    let mut entries = indexed_array_entries(current);
    let mut next_index = entries
        .keys()
        .next_back()
        .map(|index| index + 1)
        .unwrap_or(0);
    let scalar_append = !value.starts_with('(');

    for token in parse_array_tokens(value)
        .into_iter()
        .flat_map(|token| split_indexed_tagged_token(&token))
    {
        // GNU arrayfunc.c:753 assign_compound_array_list: only words with
        // the W_ASSIGNMENT flag (set during parsing) are checked for
        // [subscript]=value form. Words produced by field-splitting an
        // expanded parameter (tagged with ARRAY_FIELD_SPLIT_MARKER \x10)
        // are stored as bare elements even if they look like [2]=2]
        // (array19.sub: declare -a var=($value) with value containing
        // "[2]=2]" stores [3]="[2]=2]", not [2]="2]").
        let from_field_split =
            token.starts_with(crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER);
        let token = token
            .strip_prefix(crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER)
            .unwrap_or(&token);
        // GNU arrayfunc.c assign_compound_array_list: the raw compound word
        // keeps its quote characters, but [subscript]=value detection must
        // see through outer quotes (array19.sub: "0)]=1" is a bare element,
        // not a [0)]= assignment; "[2]=2]" IS a [2]= assignment).
        let unquoted_token = unquote_storage_value(&token);
        // GNU arrayfunc.c:557 expand_compound_array_assignment runs the real
        // pathname expansion on every element word that is not an
        // assignment word (W_NOGLOB): nullglob removes unmatched words,
        // failglob aborts the assignment, slash-bearing patterns match
        // path components (niubash #121). The element text was re-parsed
        // (arrayfunc.c:574), so raw parse tokens get their quote operators
        // re-encoded into the glob engine's escape model before globbing
        // (`declare -a x=("dir"/*.txt)` stores the matches); field-split
        // products carry quotes as DATA and skip the re-encoding.
        if from_field_split || !token_is_subscript_assignment(&unquoted_token) {
            let glob_word = if from_field_split {
                token.to_string()
            } else {
                crate::executor::glob::compound_element_glob_pattern(token)
            };
            match pathname_expand_word(&glob_word, env_vars) {
                PathnameExpansion::Matches(matches) => {
                    for value in matches {
                        entries.insert(next_index, value);
                        next_index += 1;
                    }
                    continue;
                }
                PathnameExpansion::NoMatch => {}
                PathnameExpansion::Fail(pattern) => return Err(pattern),
            }
        }
        if !from_field_split {
            if let Some((left, rhs)) = unquoted_token.split_once("+=") {
                if let Some(index) = array_assignment_index(left, &entries) {
                    let current = entries.get(&index).cloned().unwrap_or_default();
                    let rhs = unquote_storage_value(rhs);
                    let value = if integer {
                        (eval_arith_value(&current) + eval_arith_value(&rhs)).to_string()
                    } else {
                        format!("{current}{rhs}")
                    };
                    entries.insert(index, value);
                    next_index = index + 1;
                    continue;
                }
                if array_assignment_has_subscript(left) {
                    continue;
                }
            }

            if let Some((left, rhs)) = unquoted_token.split_once('=') {
                if let Some(index) = array_assignment_index(left, &entries) {
                    // GNU dequote_string (subst.c:4807) strips the CTLESC
                    // sentinels before quoted glob metacharacters after
                    // globbing passes: `declare -a x=([0]="*y")` stores `*y`.
                    let stored =
                        crate::executor::markers::dequote_ctlesc_pairs(&unquote_storage_value(rhs));
                    entries.insert(index, stored);
                    next_index = index + 1;
                    continue;
                }
                if array_assignment_has_subscript(left) {
                    continue;
                }
            }
        }

        // GNU keeps whitespace inside a quoted compound word as ONE element
        // for both quote families ('a b' and "a b" each store a single
        // element; only unquoted whitespace splits).
        let quoted_token = (token.starts_with('"') && token.ends_with('"'))
            || (token.starts_with('\'') && token.ends_with('\''));
        let token = unquoted_token;
        let unquoted_command_substitution = token.starts_with(STORAGE_WORD_PREFIX);
        let token = token.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(&token);
        // Same dequote_string strip for plain elements: quoted glob
        // metacharacters keep their data, never the \x11 sentinel.
        let token = crate::executor::markers::dequote_ctlesc_pairs(token);
        if token.contains(char::is_whitespace) && (!quoted_token || unquoted_command_substitution) {
            for value in token.split_whitespace() {
                entries.insert(next_index, value.to_string());
                next_index += 1;
            }
            continue;
        }

        if scalar_append && !entries.is_empty() {
            let current = entries.get(&0).cloned().unwrap_or_default();
            let appended = if integer {
                (eval_arith_value(&current) + eval_arith_value(&token)).to_string()
            } else {
                format!("{current}{token}")
            };
            entries.insert(0, appended);
        } else {
            entries.insert(next_index, token.to_string());
            next_index += 1;
        }
    }

    if integer {
        for element in entries.values_mut() {
            *element = eval_arith_value(element).to_string();
        }
    }

    Ok(format_indexed_array_storage(entries))
}

pub(in crate::builtins) fn indexed_array_entries(value: &str) -> BTreeMap<usize, String> {
    if let Some(rendered) = value.strip_prefix(STORAGE_WORD_PREFIX) {
        return rendered_array_entries(rendered);
    }

    parse_array_words(value).into_iter().enumerate().collect()
}

fn rendered_array_entries(rendered: &str) -> BTreeMap<usize, String> {
    let inner = rendered
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(rendered);
    let mut default_index = 0;
    split_storage_words(inner)
        .filter_map(|part| {
            if let Some((left, right)) = part.split_once('=') {
                if let Some(index) = left
                    .strip_prefix('[')
                    .and_then(|value| value.strip_suffix(']'))
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    default_index = index + 1;
                    return Some((index, super::decode_ansic_storage_value(right)));
                }
            }
            let index = default_index;
            default_index += 1;
            Some((index, super::decode_ansic_storage_value(&part)))
        })
        .collect()
}

pub(in crate::builtins) fn format_indexed_array_storage(
    entries: BTreeMap<usize, String>,
) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]={}", super::quote_array_element_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{STORAGE_WORD_PREFIX_STR}({rendered})")
}

fn array_assignment_index(left: &str, entries: &BTreeMap<usize, String>) -> Option<usize> {
    let expression = left.strip_prefix('[')?.strip_suffix(']')?;
    if expression.trim().is_empty() {
        return entries
            .keys()
            .next_back()
            .map(|index| index + 1)
            .or(Some(0));
    }
    let index = eval_arith_value(expression);
    if index >= 0 {
        return usize::try_from(index).ok();
    }
    let max_index = entries.keys().next_back().copied()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(index)?;
    usize::try_from(resolved).ok()
}

fn array_assignment_has_subscript(left: &str) -> bool {
    // GNU arrayfunc.c:753 assign_compound_array_list: a word is a
    // [subscript]=value assignment only when it starts with `[`. A bare
    // value like `0)]=1` (array19.sub) contains `]` but is not a subscript
    // assignment.
    left.starts_with('[')
}
