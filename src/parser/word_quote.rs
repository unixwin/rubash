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

pub(super) fn word_quotes_in_raw(raw: &str) -> Vec<WordQuote> {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut quotes = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        // Nested expansion bodies have their own quote state (GNU parse.y
        // xparse_dolparen / parse_matched_pair): quotes and escapes inside
        // `$(...)`/`${...}`/`` `...` `` do not quote the enclosing word.
        if let Some(next_index) = super::pathname_pattern::skip_nested_expansion(&chars, index)
        {
            index = next_index;
            continue;
        }

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
            // GNU CTLESC (parse.y:5694-5706 got_escaped_character): a
            // top-level `\x` escape quotes its character — record it like
            // a quote segment so consumers can tell `\b` from a bare `b`.
            if let Some(&escaped) = chars.get(index + 1) {
                let text: String = chars[index..index + 2].iter().collect();
                quotes.push(WordQuote {
                    open_delimiter_metadata: delimiter_metadata("\\"),
                    open_delimiter: "\\".to_string(),
                    body: escaped.to_string(),
                    kind: QuoteKind::Backslash,
                    close_delimiter_metadata: delimiter_metadata(""),
                    close_delimiter: String::new(),
                    text,
                    word_index: None,
                    assignment_name: None,
                });
                index += 2;
                continue;
            }
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
            let open_delimiter = chars[start..start + opener_len].iter().collect::<String>();
            let close_delimiter = terminator.to_string();
            return Some((
                WordQuote {
                    text: chars[start..=index].iter().collect(),
                    open_delimiter_metadata: delimiter_metadata(&open_delimiter),
                    open_delimiter,
                    body: chars[start + opener_len..index].iter().collect(),
                    kind,
                    close_delimiter_metadata: delimiter_metadata(&close_delimiter),
                    close_delimiter,
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

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::literal(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}
