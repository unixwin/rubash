use super::*;
use crate::executor::markers::DATA_DOLLAR;

pub(in crate::executor) fn split_assignment_word(word: &str) -> Option<(&str, &str)> {
    let (name, value) = word.split_once('=')?;
    let (base_name, _) = assignment_name_and_append(name);
    if is_shell_name(base_name) {
        Some((name, value))
    } else {
        None
    }
}

pub(in crate::executor) fn assignment_name_and_append(name: &str) -> (&str, bool) {
    name.strip_suffix('+')
        .map(|base| (base, true))
        .unwrap_or((name, false))
}

pub(in crate::executor) fn arithmetic_expression_arg(expression: &str) -> String {
    expression.replace(COMPOUND_ASSIGNMENT_MARKER, "")
}

pub(in crate::executor) fn arithmetic_assignment_suffix(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|ch| matches!(ch, b'+' | b'-' | b'*' | b'/' | b'%'))
}

pub(in crate::executor) fn single_unquoted_parameter_name(value: &str) -> Option<&str> {
    if let Some(name) = value
        .strip_prefix("${")
        .and_then(|name| name.strip_suffix('}'))
    {
        return is_shell_name(name).then_some(name);
    }
    let name = value.strip_prefix('$')?;
    is_shell_name(name).then_some(name)
}

pub(in crate::executor) fn append_assoc_value(
    current: &str,
    value: &str,
    integer: bool,
    env_vars: &HashMap<String, String>,
) -> String {
    // GNU arrayfunc.c assign_compound_array_list / bind_assoc_variable: when
    // the array carries the integer attribute, every element value is
    // evaluated as an arithmetic expression before it is stored
    // (assoc.tests: declare -i chaff; chaff=( [zero]=1+4 [one]=3+7 )). The
    // evaluation resolves shell variables (flix=9; wheat=([foo bar]=flix)
    // stores 9), so it uses the real evaluator, not the storage-shape one.
    let eval_element = |raw: &str| -> String {
        if integer {
            eval_conditional_arith_value(raw, env_vars)
                .unwrap_or(0)
                .to_string()
        } else {
            raw.to_string()
        }
    };
    let mut entries = assoc_entries(current);
    let tokens = merge_assoc_subscript_tokens(array_assignment_tokens(value));
    // GNU arrayfunc.c kvpair_assignment_p: the FIRST compound word decides
    // the mode — kvpair (alternating pairs) requires the first word to NOT
    // start with `[` (assoc-kv2 probe M2: a=(a=b c=d) stores [a=b]="c=d").
    let explicit_subscripts = tokens
        .first()
        .map(|token| token.starts_with('['))
        .unwrap_or(false);

    if !explicit_subscripts {
        for pair in tokens.chunks(2) {
            let Some(key) = pair.first() else {
                continue;
            };
            let key = unquote_storage_value(key);
            // GNU assign_assoc_from_kvlist (arrayfunc.c:644-650): an empty
            // expanded key reports `<word>: bad array subscript` and skips
            // only that pair (continue, not break) — no any_failed, so the
            // assignment itself still succeeds.
            if key.is_empty() {
                continue;
            }
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
            // A subscript resolved by rewrite_compound_element_subscripts
            // arrives hex-encoded behind the \x1e carrier so `]`/`=` inside
            // the key survives the token re-parse; decode it back to the
            // literal key before storage.
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
                        // GNU bind_assoc_variable: an integer append adds the
                        // two expressions arithmetically (wheat[foo bar]+=7
                        // with wheat[foo bar]=eval(flix)=9 stores 16).
                        *entry_value = (eval_conditional_arith_value(entry_value, env_vars)
                            .unwrap_or(0)
                            + eval_conditional_arith_value(&rhs, env_vars).unwrap_or(0))
                        .to_string();
                    } else {
                        *entry_value = append_scalar_value(entry_value, &rhs);
                    }
                } else {
                    entries.push((key, eval_element(&rhs)));
                }
                continue;
            }
            // GNU bind_assoc_variable -> assoc_insert: a repeated key keeps
            // its first-insert slot and the new value replaces the old one
            // (assoc13: declare a[*]=star2 overwrites [*]" star").
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
        // Bare element (no `[key]=` form): GNU rejects it with
        // "<name>: <word>: must use subscript when assigning associative
        // array" and skips the element. The error is emitted by the caller
        // (apply_shell_assignment) which owns the diagnostic context.
    }

    format_assoc_storage(entries)
}

/// Return the FIRST bare (non `[key]=value`) element of an associative
/// array compound assignment so the caller can emit the GNU "must use
/// subscript" error. GNU assign_compound_array_list (arrayfunc.c) breaks
/// the strict loop at the first offending word, so exactly one diagnostic
/// is printed. Returns an empty vec for the alternating `key value`
/// form (no explicit subscripts) — that form has no bare elements.
pub(in crate::executor) fn assoc_bare_elements(value: &str) -> Vec<String> {
    let tokens = merge_assoc_subscript_tokens(array_assignment_tokens(value));
    // GNU arrayfunc.c kvpair_assignment_p: the FIRST compound word decides
    // the mode — kvpair (alternating literal key/value pairs) requires the
    // first word to NOT start with `[` (strict [key]=value words always
    // start with the bracket). A first word like `a=b` is kvpair data: the
    // `=` inside a compound assignment list has no assignment semantics
    // (assoc-kv2 probe M2: a=(a=b c=d) stores [a=b]="c=d").
    let strict_mode = tokens
        .first()
        .map(|token| token.starts_with('['))
        .unwrap_or(false);
    if !strict_mode {
        return Vec::new();
    }
    let first_bare = tokens
        .iter()
        .find(|token| assoc_assignment_token(token).is_none())
        .map(|token| unquote_storage_value(token));
    first_bare.into_iter().collect()
}

/// Key words of a kvpair-mode assoc compound assignment whose expanded
/// key is empty — GNU assign_assoc_from_kvlist (arrayfunc.c:644-650)
/// reports each as `<word>: bad array subscript` and skips just that pair.
/// The word text is the re-quoted form from the rebuilt word list (`""`
/// prints `''`). Returns empty for the strict `[key]=value` form, whose
/// empty key is handled by rewrite_compound_element_subscripts'
/// err_badarraysub instead.
pub(crate) fn assoc_empty_key_words(value: &str) -> Vec<String> {
    let tokens = merge_assoc_subscript_tokens(array_assignment_tokens(value));
    let strict_mode = tokens
        .first()
        .map(|token| token.starts_with('['))
        .unwrap_or(false);
    if strict_mode {
        return Vec::new();
    }
    tokens
        .chunks(2)
        .filter_map(|pair| {
            let key = pair.first()?;
            unquote_storage_value(key)
                .is_empty()
                .then(|| key.to_string())
        })
        .collect()
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

/// Quote-aware unclosed `[` / quote state of a storage token: whitespace
/// between an unclosed `[` and its `]`, or inside an unclosed quote, is part
/// of the assoc key/value, not a word separator (assoc.tests:
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

pub(in crate::executor) fn append_assoc_scalar_value(current: &str, value: &str) -> String {
    let mut entries = assoc_entries(current);
    let value = unquote_storage_value(value);
    if let Some((_, entry_value)) = entries.iter_mut().rev().find(|(key, _)| key == "0") {
        *entry_value = value;
    } else {
        entries.push(("0".to_string(), value));
    }
    format_assoc_storage(entries)
}

pub(in crate::executor) fn format_assoc_storage(entries: Vec<(String, String)>) -> String {
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

pub(in crate::executor) fn quote_assoc_key(key: &str) -> String {
    // The storage form is re-parsed by split_storage_words on every read:
    // a bare ``` or `$` in the key opens a substitution span in that
    // tokenizer and glues the following pairs into this pair's value
    // (assoc9.sub dict['`']=2 then dict["'"]=3), so they must
    // force quoting alongside whitespace, quotes, backslash and `]`.
    if !key.is_empty()
        && !key
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '\'' | '"' | '\\' | ']' | '`' | '$'))
    {
        return key.to_string();
    }

    quote_assoc_storage_value_forced(key)
}

pub(in crate::executor) fn quote_assoc_storage_value(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '\'' | '"' | '\\' | '`' | '$'))
    {
        return value.to_string();
    }

    quote_assoc_storage_value_forced(value)
}

fn quote_assoc_storage_value_forced(value: &str) -> String {
    // Inside "..." the storage tokenizer still honors `$` and ``` as
    // substitution openers (they are not gated on in_double), so they
    // are backslash-escaped like `\"` and `\\\\`; unquote_storage_value's
    // double-quote branch decodes all four.
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '"' | '\\' | '`' | '$') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

pub(in crate::executor) fn assoc_entries(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    merge_assoc_subscript_tokens(split_storage_words(inner).collect())
        .into_iter()
        .filter_map(|part| {
            // Storage pairs are `[key]=value`; split at the quote-aware
            // subscript close so a quoted `=` inside a stored key stays in
            // the key (assoc.tests assoc4: ["a]=test1;#a"]="123").
            if let Some((key, value, _)) = assoc_assignment_token(&part) {
                return Some((unquote_storage_value(key), unquote_storage_value(value)));
            }
            let (key, value) = part.split_once('=')?;
            Some((
                unquote_storage_value(key.trim_start_matches('[').trim_end_matches(']')),
                unquote_storage_value(value),
            ))
        })
        .collect()
}

pub(in crate::executor) fn assoc_value_at(value: &str, key: &str) -> Option<String> {
    assoc_entries(value)
        .into_iter()
        .rev()
        .find_map(|(entry_key, entry_value)| (entry_key == key).then_some(entry_value))
}

pub(in crate::executor) fn assoc_keys(value: &str, nbuckets: usize) -> Vec<String> {
    // bash_assoc_order items are (entry_index, (key, value)); collect keys.
    bash_assoc_order(&assoc_entries(value), nbuckets)
        .into_iter()
        .map(|(_, (key, _))| key)
        .collect()
}

/// GNU assoc iteration depends on the table's bucket count, which is fixed
/// at table creation: make_new_assoc_variable uses ASSOC_HASH_BUCKETS=1024
/// (variables.c:2857, assoc.h:28), convert_var_to_assoc uses
/// assoc_create(0)==DEFAULT_HASH_BUCKETS=128 (arrayfunc.c:114-117,
/// hashlib.h:72), and the dynamic vars inherit their source table's size —
/// BASH_CMDS from hashed_filenames (hashcmd.h:24 FILENAME_HASH_BUCKETS=256),
/// BASH_ALIASES from aliases (alias.c:49 ALIAS_HASH_BUCKETS=64)
/// (variables.c:1692,1762). assoc_copy preserves the source count
/// (hashlib.c:174 hash_copy -> hash_create(table->nbuckets)).
pub(crate) fn assoc_nbuckets(env_vars: &HashMap<String, String>, name: &str) -> usize {
    match name {
        "BASH_CMDS" => 256,
        "BASH_ALIASES" => 64,
        _ if is_marked_var(env_vars, ASSOC_128_VARS, name) => 128,
        _ => 1024,
    }
}

/// FNV-1 (multiply first, then xor) over `char` bytes, 32 bit — hashlib.c
/// hash_string. On x86 a plain `char` is signed, so bytes >= 0x80 sign-
/// extend before the xor.
fn bash_hash_string(key: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in key.bytes() {
        hash = hash.wrapping_mul(16777619);
        hash ^= (byte as i8) as i32 as u32;
    }
    hash
}

/// Bash assoc.c / hashlib.c table iteration order: FNV-1 hashed keys into a
/// power-of-two bucket array, head-insertion chains, grow x4 when nentries >=
/// nbuckets * 2 (rehash walks old buckets 0..n and re-inserts each item at its
/// new chain head). Iteration visits bucket 0..n, each chain head to tail. A
/// repeated key keeps its first-insert slot and the last value wins
/// (hash_search replaces data in place). `nbuckets` is the table's creation
/// size — see assoc_nbuckets for the per-source values.
pub(crate) fn bash_assoc_order(
    entries: &[(String, String)],
    nbuckets: usize,
) -> Vec<(usize, (String, String))> {
    // First occurrence fixes the slot; last occurrence supplies the value.
    let mut first_index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut unique: Vec<usize> = Vec::new();
    for (index, (key, _)) in entries.iter().enumerate() {
        if first_index.contains_key(key.as_str()) {
            continue;
        }
        first_index.insert(key.as_str(), index);
        unique.push(index);
    }

    let mut nbuckets: usize = nbuckets.max(1);
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); nbuckets];
    let mut count = 0usize;
    for &entry_index in &unique {
        if count >= nbuckets * 2 {
            let mut grown = vec![Vec::new(); nbuckets * 4];
            for old_bucket in &buckets {
                for &item in old_bucket {
                    let bucket = bash_hash_string(&entries[item].0) as usize & (nbuckets * 4 - 1);
                    grown[bucket].insert(0, item);
                }
            }
            buckets = grown;
            nbuckets *= 4;
        }
        let bucket = bash_hash_string(&entries[entry_index].0) as usize & (nbuckets - 1);
        buckets[bucket].insert(0, entry_index);
        count += 1;
    }

    buckets
        .into_iter()
        .flatten()
        .map(|index| (index, entries[index].clone()))
        .collect()
}

pub(in crate::executor) fn assoc_hash_ordered_entries(
    value: &str,
    nbuckets: usize,
) -> Vec<(String, String)> {
    bash_assoc_order(&assoc_entries(value), nbuckets)
        .into_iter()
        .map(|(_, entry)| entry)
        .collect()
}

pub(in crate::executor) fn assoc_hash_ordered_values(value: &str, nbuckets: usize) -> Vec<String> {
    bash_assoc_order(&assoc_entries(value), nbuckets)
        .into_iter()
        .map(|(_, (_, entry_value))| entry_value)
        .collect()
}

pub(in crate::executor) fn split_storage_words(value: &str) -> impl Iterator<Item = String> + '_ {
    StorageWordIter {
        input: value,
        offset: 0,
    }
}

/// GNU arrayfunc.c:610 expand_words_no_vars field-splits every indexed
/// compound element's expansion, so the \x1c-tagged expansion whitespace
/// (embedded_mutations expansion_ws_marked) is a split boundary for
/// indexed arrays even though the same bytes stay glued for associative
/// words (arrayfunc.c:652/865 expand_assignment_string_to_string never
/// field-splits). Empty fields drop like GNU's field splitting.
pub(in crate::executor) fn split_indexed_tagged_token(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = token.chars().peekable();
    while let Some(ch) = chars.next() {
        if (ch == crate::executor::markers::IFS_GLUE
            || ch == crate::executor::COMPOUND_EXPANSION_WS_TAG)
            && matches!(chars.peek(), Some(' ' | '\t' | '\n'))
        {
            chars.next();
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

struct StorageWordIter<'a> {
    input: &'a str,
    offset: usize,
}

impl Iterator for StorageWordIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ch) = self.input.get(self.offset..)?.chars().next() {
            if !ch.is_ascii_whitespace() {
                break;
            }
            self.offset += ch.len_utf8();
        }

        let mut word = String::new();
        let mut in_double = false;
        let mut in_single = false;
        let mut escaped = false;
        let mut chars = self.input[self.offset..].char_indices().peekable();
        while let Some((relative, ch)) = chars.next() {
            if escaped {
                word.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' && in_double {
                word.push(ch);
                escaped = true;
                continue;
            }
            // GNU parse.y:5368-5397 read_token_word: a backslash outside
            // any quote removes itself and keeps the next char literal. We
            // keep the backslash in the token so pathname_expand_array_token
            // can see it and skip glob expansion (array.tests:245
            // `\*` must stay literal). unquote_storage_value removes it.
            if ch == '\\' && !in_double && !in_single {
                word.push(ch);
                escaped = true;
                continue;
            }
            if ch == '$' && !in_single && matches!(chars.peek(), Some((_, '{'))) {
                // A `${...}` body is one lexical unit: GNU parse_matched_pair
                // scans it with its own nested-pair quote state, so body
                // whitespace never splits a compound-assignment word
                // (array6.sub: ("${a[@]/#/-iname \'}")).
                word.push(ch);
                word.push('{');
                chars.next();
                let rest = &self.input[self.offset + relative + 2..];
                if let Some(scan) = crate::lexer::dolbrace::scan_braced_parameter_body(
                    rest,
                    crate::lexer::dolbrace::BraceContext {
                        outer_double_quote: in_double,
                        posix: false,
                        replacement_context: false,
                        initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                    },
                ) {
                    word.push_str(&rest[..scan.end]);
                    for _ in 0..rest[..scan.end].chars().count() {
                        chars.next();
                    }
                }
                continue;
            }
            // Expansion-produced whitespace the compound walker tagged
            // (embedded_mutations expansion_ws_marked, GNU arrayfunc.c:652
            // expand_assignment_string_to_string never field-splits): glue
            // the marker and its whitespace into the word so assoc kv-pairs
            // keep them; the indexed callers re-split on the marker. The
            // \x1c IFS-protection sentinel takes the same glued form here.
            if ch == crate::executor::markers::IFS_GLUE
                || ch == crate::executor::COMPOUND_EXPANSION_WS_TAG
            {
                word.push(ch);
                if let Some((_, next)) = chars.next() {
                    word.push(next);
                }
                continue;
            }
            // GNU parse_string_to_word_list re-parses the compound value, so
            // `$(...)`/backtick substitutions are single lexical words —
            // whitespace inside them never splits (array19.sub:
            // declare -a e=$y with y='($(echo Darwin))').
            if ch == '$' && !in_single && matches!(chars.peek(), Some((_, '('))) {
                word.push(ch);
                word.push('(');
                chars.next();
                let rest_offset = self.offset + relative + 2;
                let rest = &self.input[rest_offset..];
                let mut depth = 1usize;
                let mut inner_single = false;
                let mut inner_double = false;
                let mut inner_escaped = false;
                let mut consumed = 0usize;
                for (off, c) in rest.char_indices() {
                    if inner_escaped {
                        inner_escaped = false;
                        continue;
                    }
                    match c {
                        '\\' if !inner_single => inner_escaped = true,
                        '\'' if !inner_double => inner_single = !inner_single,
                        '"' if !inner_single => inner_double = !inner_double,
                        '(' if !inner_single && !inner_double => depth += 1,
                        ')' if !inner_single && !inner_double => {
                            depth -= 1;
                            if depth == 0 {
                                consumed = off + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if consumed == 0 {
                    consumed = rest.len();
                }
                word.push_str(&rest[..consumed]);
                for _ in 0..rest[..consumed].chars().count() {
                    chars.next();
                }
                continue;
            }
            if ch == '`' && !in_single {
                word.push(ch);
                loop {
                    match chars.next() {
                        Some((_, '`')) => {
                            word.push('`');
                            break;
                        }
                        Some((_, '\\')) => {
                            word.push('\\');
                            if let Some((_, nc)) = chars.next() {
                                word.push(nc);
                            }
                        }
                        Some((_, c)) => word.push(c),
                        None => break,
                    }
                }
                continue;
            }
            // Mirror the declare storage splitter (declare/storage/words.rs):
            // whitespace inside EITHER quote family does not split a
            // compound-assignment word ('a b' stores one element, assoc12
            // "1 2" stays one kvpair key).
            if ch == '\'' && !in_double {
                in_single = !in_single;
                word.push(ch);
                continue;
            }
            if ch == '"' && !in_single {
                in_double = !in_double;
                word.push(ch);
                continue;
            }
            if ch.is_ascii_whitespace() && !in_double && !in_single {
                self.offset += relative + ch.len_utf8();
                return Some(word);
            }
            word.push(ch);
        }
        self.offset = self.input.len();
        (!word.is_empty()).then_some(word)
    }
}

pub(in crate::executor) fn unquote_storage_value(value: &str) -> String {
    // GNU arrayfunc.c assign_compound_array_list runs each raw compound word
    // through the same quote removal as an ordinary word, so a dollar-single-quote
    // element value is ANSI-C decoded here. unicode1.sub's C_UTF_8 table is 1318
    // octal-escape elements, and without this branch the whole table collapsed
    // into one literal element (intl: Failed 1 of 1).
    // The lexer decoder owns the >=0x80 owner-marker contract, so it is not
    // re-implemented with char::from_u32 here.
    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        // Tag decoded quotes with E010/E011 markers so subsequent quote
        // removal (unquote_storage_value on the stored token) can tell
        // data quotes from syntax quotes. Without this, `$'a"b'` decodes
        // to `a"b` and the bare `"` is stripped as a quote operator
        // (issue #109: x=($'a"b') stored "ab" instead of "a\"b").
        let decoded = crate::lexer::decode_ansi_c_quoted(inner);
        return crate::lexer::escape_decoded_ansi_c_quotes(&decoded);
    }

    fn restore_quote_markers(value: &str) -> String {
        value
            .replace(DATA_DOLLAR, "$")
            .replace(crate::executor::markers::DATA_BACKTICK, "`")
            .replace(crate::executor::markers::DATA_SQUOTE, "'")
            .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
            // The \x1c expansion-whitespace tag (embedded_mutations
            // expansion_ws_marked) marks the whitespace itself as data;
            // strip the tag and keep the character it protected.
            .replace(crate::executor::markers::IFS_GLUE, "")
            .replace(crate::executor::COMPOUND_EXPANSION_WS_TAG, "")
            .replace(crate::lexer::ANSI_C_QUOTE_MARKER_STR, "'")
            .replace(crate::lexer::ANSI_C_DQUOTE_MARKER_STR, "\"")
    }

    if value == "\\\"\\" {
        return "\"\"".to_string();
    }

    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        // quote_array_value -> ansic_quote (arrays/storage.rs) emits the
        // dollar-single-quoted form for an element value holding a
        // non-printing character, using the same ANSI-C escape grammar as a
        // shell dollar-single-quoted word. decode_ansi_c_quoted is exactly
        // its inverse. No marker restore is needed: ansic_quote
        // octal-escapes every byte below 0x20 instead of substituting one
        // of the marker characters, so a marker byte can never appear
        // inside the quoted span.
        return crate::lexer::ansi::decode_ansi_c_quoted(inner);
    }
    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return restore_quote_markers(inner);
    }

    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        // A bare storage value still carries the ANSI-C quote markers: a
        // value that the lexer decoded from $'...' reaches this fallthrough
        // without quote delimiters of its own, so the PUA markers must come
        // back as ' and ". The C0 markers (U+0014/17/1A/1F) are
        // deliberately NOT restored here -- on this path they are
        // indistinguishable from genuine data bytes that ANSI-C decoding
        // produced ($'\027' is a real U+0017), and restoring them corrupted
        // those values. The quoted paths above keep the full
        // restore_quote_markers because there the C0 bytes are
        // unambiguously walker markers.
        //
        // GNU parse.y:5368-5397 read_token_word: a backslash outside any
        // quote removes itself and keeps the next char literal. Raw-byte
        // marker pairs (U+E000 + U+E0xx) must be preserved through storage
        // and retrieval so that carrier bytes survive the expansion path
        // (unicode1.sub nameref + assoc: EChar len=0, %q=''). The markers
        // are decoded at the final output stage (echo/printf) by
        // shift_echo_builtins.rs.
        let sentinel = crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE;
        let marker_first = crate::executor::substitution_metadata::RAW_BYTE_MARKER_FIRST;
        let marker_last = crate::executor::substitution_metadata::RAW_BYTE_MARKER_LAST;
        let mut decoded = String::new();
        let mut chars = value.chars().peekable();
        let mut escaped = false;
        while let Some(ch) = chars.next() {
            if escaped {
                decoded.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch as u32 == sentinel {
                if let Some(&next) = chars.peek() {
                    if (marker_first..=marker_last).contains(&(next as u32)) {
                        // Preserve raw-byte marker pair for the expansion
                        // path to decode at output time.
                        decoded.push(ch);
                        decoded.push(next);
                        chars.next();
                        continue;
                    }
                }
            }
            decoded.push(ch);
        }
        // Do NOT restore E010/E011 markers here: they tag data quotes
        // from ANSI-C decoding that must survive remove_shell_quotes.
        // The markers are restored to actual quotes at output time.
        // \x1c is the expansion-whitespace tag (expansion_ws_marked): the
        // whitespace it precedes is data, the tag itself is not.
        return decoded
            .replace(crate::executor::markers::IFS_GLUE, "")
            .replace(crate::executor::COMPOUND_EXPANSION_WS_TAG, "");
    };

    let mut unquoted = String::new();
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            if !matches!(ch, '$' | '`' | '"' | '\\' | '\n') {
                unquoted.push('\\');
            }
            unquoted.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            unquoted.push(ch);
        }
    }
    if escaped {
        unquoted.push('\\');
    }
    if unquoted == "\\\"\\" {
        return "\"\"".to_string();
    }
    restore_quote_markers(&unquoted)
}

pub(in crate::executor) fn quote_compound_field_value(value: &str) -> String {
    quote_array_value(value)
}
