pub(super) fn skip_heredoc_in_chars(chars: &[char], start: usize) -> usize {
    let mut index = start + 2;
    let strip_tabs = if chars.get(index) == Some(&'-') {
        index += 1;
        true
    } else {
        false
    };
    while chars.get(index).is_some_and(|ch| matches!(ch, ' ' | '\t')) {
        index += 1;
    }
    let delimiter_start = index;
    while chars
        .get(index)
        .is_some_and(|ch| !ch.is_whitespace() && !matches!(ch, ';' | '|' | '&' | ')'))
    {
        index += 1;
    }
    let mut delimiter = chars[delimiter_start..index]
        .iter()
        .collect::<String>()
        .replace(['\'', '"', '\\'], "");
    if strip_tabs {
        delimiter = delimiter.trim_start_matches('\t').to_string();
    }
    if delimiter.is_empty() {
        return index;
    }
    while chars.get(index).is_some_and(|ch| *ch != '\n') {
        index += 1;
    }
    if chars.get(index) == Some(&'\n') {
        index += 1;
    }

    while index < chars.len() {
        let line_start = index;
        while chars.get(index).is_some_and(|ch| *ch != '\n') {
            index += 1;
        }
        let line = chars[line_start..index].iter().collect::<String>();
        let comparable = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if comparable
            .strip_suffix([')', '`'])
            .is_some_and(|value| value == delimiter)
        {
            let leading_tabs = if strip_tabs {
                line.chars().take_while(|ch| *ch == '\t').count()
            } else {
                0
            };
            index = line_start + leading_tabs + delimiter.chars().count();
            break;
        }
        if comparable == delimiter {
            if chars.get(index) == Some(&'\n') {
                index += 1;
            }
            break;
        }
        if chars.get(index) == Some(&'\n') {
            index += 1;
        }
    }

    index
}
