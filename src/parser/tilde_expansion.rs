use super::{CommandNode, TildeExpansion};

pub(super) fn record_tilde_expansions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    let expansions = tilde_expansions_in_word(word)
        .into_iter()
        .map(|mut expansion| {
            expansion.word_index = Some(word_index);
            expansion
        });
    command.tilde_expansions.extend(expansions);
}

pub(super) fn record_tilde_expansions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let expansions =
        tilde_expansions_in_assignment_value(value)
            .into_iter()
            .map(|mut expansion| {
                expansion.assignment_name = Some(assignment_name.to_string());
                expansion.word_index = word_index;
                expansion
            });
    command.tilde_expansions.extend(expansions);
}

pub(super) fn tilde_expansions_in_word(word: &str) -> Vec<TildeExpansion> {
    if word.starts_with('\x1b') {
        return Vec::new();
    }

    tilde_expansion_at(word, false).into_iter().collect()
}

fn tilde_expansions_in_assignment_value(value: &str) -> Vec<TildeExpansion> {
    if value.starts_with(crate::expand::tilde::tilde::QUOTED_ASSIGNMENT_VALUE) {
        return Vec::new();
    }

    let mut expansions = Vec::new();
    let mut start = 0usize;
    let mut after_colon = false;
    for (index, ch) in value.char_indices() {
        if index == 0 || ch != ':' {
            continue;
        }
        if let Some(expansion) = tilde_expansion_at(&value[start..index], after_colon) {
            expansions.push(expansion);
        }
        start = index + ch.len_utf8();
        after_colon = true;
    }
    if let Some(expansion) = tilde_expansion_at(&value[start..], after_colon) {
        expansions.push(expansion);
    }
    expansions
}

fn tilde_expansion_at(segment: &str, after_colon: bool) -> Option<TildeExpansion> {
    let rest = segment.strip_prefix('~')?;
    let prefix_len = rest.find('/').map_or(segment.len(), |slash| slash + 1);
    let prefix = &segment[..prefix_len];
    if prefix == "~+"
        || prefix == "~-"
        || prefix == "~"
        || valid_tilde_dirstack(prefix)
        || valid_tilde_login(prefix)
    {
        return Some(TildeExpansion {
            text: segment.to_string(),
            open_delimiter: "~".to_string(),
            prefix: prefix.to_string(),
            close_delimiter: String::new(),
            suffix: segment[prefix_len..].to_string(),
            after_colon,
            word_index: None,
            assignment_name: None,
        });
    }
    None
}

fn valid_tilde_dirstack(prefix: &str) -> bool {
    prefix
        .strip_prefix("~+")
        .or_else(|| prefix.strip_prefix("~-"))
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
}

fn valid_tilde_login(prefix: &str) -> bool {
    prefix.len() > 1
        && prefix[1..]
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}
