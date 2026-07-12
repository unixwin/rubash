use super::{CommandNode, QuoteKind, WordQuote};

pub(super) fn record_word_quotes_for_word(command: &mut CommandNode, word_index: usize, raw: &str) {
    let quotes = word_quotes_in_raw(raw).into_iter().map(|mut quote| {
        quote.word_index = Some(word_index);
        quote
    });
    command.word_quotes.extend(quotes);
}

pub(super) fn record_word_quotes_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    raw_value: &str,
    word_index: Option<usize>,
) {
    let quotes = word_quotes_in_raw(raw_value).into_iter().map(|mut quote| {
        quote.assignment_name = Some(assignment_name.to_string());
        quote.word_index = word_index;
        quote
    });
    command.word_quotes.extend(quotes);
}

fn word_quotes_in_raw(raw: &str) -> Vec<WordQuote> {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut quotes = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') {
            if let Some((quote, next_index)) =
                quoted_segment(&chars, index, 2, '\'', QuoteKind::AnsiC)
            {
                quotes.push(quote);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'"') {
            if let Some((quote, next_index)) =
                quoted_segment(&chars, index, 2, '"', QuoteKind::Locale)
            {
                quotes.push(quote);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '\'' {
            if let Some((quote, next_index)) =
                quoted_segment(&chars, index, 1, '\'', QuoteKind::Single)
            {
                quotes.push(quote);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '"' {
            if let Some((quote, next_index)) =
                quoted_segment(&chars, index, 1, '"', QuoteKind::Double)
            {
                quotes.push(quote);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '\\' {
            index += 1;
        }
        index += 1;
    }
    quotes
}

fn quoted_segment(
    chars: &[char],
    start: usize,
    opener_len: usize,
    terminator: char,
    kind: QuoteKind,
) -> Option<(WordQuote, usize)> {
    let mut index = start + opener_len;
    while index < chars.len() {
        if chars[index] == '\\' && terminator != '\'' {
            index += 2;
            continue;
        }
        if chars[index] == terminator {
            return Some((
                WordQuote {
                    text: chars[start..=index].iter().collect(),
                    open_delimiter: chars[start..start + opener_len].iter().collect(),
                    body: chars[start + opener_len..index].iter().collect(),
                    kind,
                    close_delimiter: terminator.to_string(),
                    word_index: None,
                    assignment_name: None,
                },
                index + 1,
            ));
        }
        index += 1;
    }
    None
}
