use super::*;

pub(in crate::executor) fn normalize_single_element_array_assignment(
    value: &str,
) -> Option<String> {
    let inner = value.strip_prefix('(')?.strip_suffix(')')?;
    Some(format!("({})", strip_matching_quotes(inner.trim())))
}

pub(in crate::executor) fn strip_matching_quotes(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

pub(in crate::executor) fn strip_wrapping_subshell_group(source: &str) -> Option<&str> {
    let inner = source.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && index + ch.len_utf8() != source.len() {
                    return None;
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(inner.trim())
}

#[derive(Clone, Copy)]
pub(in crate::executor) enum MatchLength {
    Shortest,
    Longest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::executor) enum PatternRemoval {
    ShortestPrefix,
    LongestPrefix,
    ShortestSuffix,
    LongestSuffix,
}

pub(in crate::executor) fn parse_indirect_pattern_removal(
    name: &str,
) -> Option<(&str, &str, PatternRemoval)> {
    for (operator, operation) in [
        ("##", PatternRemoval::LongestPrefix),
        ("%%", PatternRemoval::LongestSuffix),
        ("#", PatternRemoval::ShortestPrefix),
        ("%", PatternRemoval::ShortestSuffix),
    ] {
        if let Some((left, pattern)) = name.split_once(operator) {
            // A `/` in the left operand means this is actually a
            // `${var/pat/repl}` pattern substitution whose pattern begins
            // with the `#`/`%` anchor (GNU subst.c pat_subst reads `#`/`%`
            // after `/` as the match-anchor, not a removal operator):
            // `${a[@]/#/x}` prepends `x`, it does not remove a prefix.
            if !left.is_empty() && !left.contains('/') {
                return Some((left, pattern, operation));
            }
        }
    }
    None
}

pub(in crate::executor) fn remove_matching_prefix(
    value: &str,
    pattern: &str,
    length: MatchLength,
    extglob: bool,
) -> String {
    if pattern_has_raw_byte_markers(pattern) {
        return remove_matching_prefix_bytes(value, pattern, length, extglob);
    }
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let iter: Box<dyn Iterator<Item = usize>> = match length {
        MatchLength::Shortest => Box::new(indices.into_iter()),
        MatchLength::Longest => Box::new(indices.into_iter().rev()),
    };

    for end in iter {
        if removal_pattern_matches(pattern, &value[..end], extglob) {
            return value[end..].to_string();
        }
    }

    value.to_string()
}

pub(in crate::executor) fn remove_matching_suffix(
    value: &str,
    pattern: &str,
    length: MatchLength,
    extglob: bool,
) -> String {
    if pattern_has_raw_byte_markers(pattern) {
        return remove_matching_suffix_bytes(value, pattern, length, extglob);
    }
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let iter: Box<dyn Iterator<Item = usize>> = match length {
        MatchLength::Shortest => Box::new(indices.into_iter().rev()),
        MatchLength::Longest => Box::new(indices.into_iter()),
    };

    for start in iter {
        if removal_pattern_matches(pattern, &value[start..], extglob) {
            return value[..start].to_string();
        }
    }

    value.to_string()
}

/// Check if the pattern contains raw-byte marker sentinels (U+E000).
fn pattern_has_raw_byte_markers(pattern: &str) -> bool {
    pattern.contains(
        char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
            .expect("raw-byte sentinel is a valid char"),
    )
}

/// Byte-level prefix removal for patterns containing raw-byte markers.
/// GNU subst.c operates on bytes, so we iterate over byte positions, not
/// char boundaries (intl3.sub: ${euro##*$o202} removes bytes E2 82 from
/// the 3-byte UTF-8 Euro sign, leaving byte AC).
fn remove_matching_prefix_bytes(
    value: &str,
    pattern: &str,
    length: MatchLength,
    extglob: bool,
) -> String {
    let value_bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(value);
    let iter: Box<dyn Iterator<Item = usize>> = match length {
        MatchLength::Shortest => Box::new(0..=value_bytes.len()),
        MatchLength::Longest => Box::new((0..=value_bytes.len()).rev()),
    };
    for end in iter {
        let prefix =
            crate::executor::substitution_metadata::bytes_to_shell_text(&value_bytes[..end]);
        if removal_pattern_matches(pattern, &prefix, extglob) {
            return crate::executor::substitution_metadata::bytes_to_shell_text(
                &value_bytes[end..],
            );
        }
    }
    value.to_string()
}

/// Byte-level suffix removal for patterns containing raw-byte markers.
fn remove_matching_suffix_bytes(
    value: &str,
    pattern: &str,
    length: MatchLength,
    extglob: bool,
) -> String {
    let value_bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(value);
    let iter: Box<dyn Iterator<Item = usize>> = match length {
        MatchLength::Shortest => Box::new((0..=value_bytes.len()).rev()),
        MatchLength::Longest => Box::new(0..=value_bytes.len()),
    };
    for start in iter {
        let suffix =
            crate::executor::substitution_metadata::bytes_to_shell_text(&value_bytes[start..]);
        if removal_pattern_matches(pattern, &suffix, extglob) {
            return crate::executor::substitution_metadata::bytes_to_shell_text(
                &value_bytes[..start],
            );
        }
    }
    value.to_string()
}

pub(in crate::executor) fn remove_parameter_pattern(
    value: &str,
    pattern: &str,
    operation: PatternRemoval,
    extglob: bool,
) -> String {
    match operation {
        PatternRemoval::ShortestPrefix => {
            remove_matching_prefix(value, pattern, MatchLength::Shortest, extglob)
        }
        PatternRemoval::LongestPrefix => {
            remove_matching_prefix(value, pattern, MatchLength::Longest, extglob)
        }
        PatternRemoval::ShortestSuffix => {
            remove_matching_suffix(value, pattern, MatchLength::Shortest, extglob)
        }
        PatternRemoval::LongestSuffix => {
            remove_matching_suffix(value, pattern, MatchLength::Longest, extglob)
        }
    }
}

/// True when the pattern contains an extglob operator `+(`/`*(`/`?(`/`@(`/
/// `!(` outside a backslash escape.
pub(in crate::executor) fn pattern_uses_extglob_syntax(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => {
                index += 2;
                continue;
            }
            '+' | '*' | '?' | '@' | '!' => {
                if chars.get(index + 1) == Some(&'(') {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

/// GNU subst.c matches removal patterns with FNM_EXTMATCH when extglob is
/// enabled (match_upattern); otherwise the pattern is an ordinary glob.
fn removal_pattern_matches(pattern: &str, word: &str, extglob: bool) -> bool {
    if extglob && pattern_uses_extglob_syntax(pattern) {
        super::conditional::extglob_case_pattern_matches(pattern, word)
    } else {
        case_pattern_matches(pattern, word)
    }
}

pub(in crate::executor) fn strip_surrounding_quotes(s: &str) -> String {
    if s.len() >= 2 {
        if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

pub(in crate::executor) fn decode_parameter_pattern_quotes(pattern: &str) -> String {
    let mut output = String::new();
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') {
            index += 2;
            let mut quoted = String::new();
            let mut escaped = false;
            while index < chars.len() {
                let ch = chars[index];
                index += 1;
                if escaped {
                    quoted.push('\\');
                    quoted.push(ch);
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '\'' {
                    break;
                }
                quoted.push(ch);
            }
            if escaped {
                quoted.push('\\');
            }
            push_literal_pattern_str(&mut output, &decode_ansi_c_escapes(&quoted));
            continue;
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'"') {
            index += 2;
            while index < chars.len() {
                let ch = chars[index];
                index += 1;
                if ch == '"' {
                    break;
                }
                if ch == '\\' {
                    if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                        chars.get(index).copied()
                    {
                        index += 1;
                        if escaped != '\n' {
                            push_literal_pattern_char(&mut output, escaped);
                        }
                        continue;
                    }
                }
                push_quoted_pattern_char(&mut output, ch);
            }
            continue;
        }

        match chars[index] {
            crate::executor::markers::DATA_SQUOTE => {
                output.push('\'');
                index += 1;
            }
            '\'' => {
                if let Some(close_offset) = chars[index + 1..].iter().position(|ch| *ch == '\'') {
                    let close = index + 1 + close_offset;
                    for ch in &chars[index + 1..close] {
                        push_literal_pattern_char(&mut output, *ch);
                    }
                    index = close + 1;
                } else {
                    output.push('\'');
                    index += 1;
                }
            }
            '"' => {
                if !chars[index + 1..].iter().any(|ch| *ch == '"') {
                    output.push('"');
                    index += 1;
                    continue;
                }
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    index += 1;
                    if ch == '"' {
                        break;
                    }
                    if ch == '\\' {
                        if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                            chars.get(index).copied()
                        {
                            index += 1;
                            if escaped != '\n' {
                                push_literal_pattern_char(&mut output, escaped);
                            }
                            continue;
                        }
                    }
                    push_quoted_pattern_char(&mut output, ch);
                }
                // Mark the end of the double-quoted section so the embedded
                // parameter expander terminates any `$var` name at the
                // closing quote (e.g. `"$v"a` expands `$v` then appends `a`,
                // rather than treating `$va` as a single variable name).
                output.push(crate::lexer::PARAM_NAME_END_MARKER);
            }
            '\\' => {
                if let Some(ch) = chars.get(index + 1) {
                    if *ch == '\\' {
                        output.push(crate::executor::markers::DATA_DQUOTE);
                    } else {
                        push_literal_pattern_char(&mut output, *ch);
                    }
                    index += 2;
                } else {
                    // Trailing backslash is a literal backslash in a pattern
                    // (`${P#\}` matches a single `\`), not a dropped char.
                    output.push(crate::executor::markers::DATA_DQUOTE);
                    index += 1;
                }
            }
            ch => {
                output.push(ch);
                index += 1;
            }
        }
    }
    output
}

/// Literal-context variant of push_quoted_pattern_char: single-quoted,
/// ANSI-C and backslash-escaped pattern text is inert for expansion (GNU
/// pat_expand honors the quoting), so `$`, `` ` `` and `"` are emitted in
/// escaped form -- the embedded expander turns `\$` / \` / `\"` into data
/// characters instead of opening a substitution or a quote span.
fn push_literal_pattern_str(output: &mut String, value: &str) {
    for ch in value.chars() {
        push_literal_pattern_char(output, ch);
    }
}

fn push_literal_pattern_char(output: &mut String, ch: char) {
    match ch {
        '\'' | '"' | '$' | '`' => {
            output.push('\\');
            output.push(ch);
        }
        _ => push_quoted_pattern_char(output, ch),
    }
}

fn push_quoted_pattern_str(output: &mut String, value: &str) {
    for ch in value.chars() {
        push_quoted_pattern_char(output, ch);
    }
}

/// Mask `"..."` spans of a pattern word into `slots` before the shared
/// quote decoder runs. GNU subst.c expands substitutions inside a quoted
/// pattern span with quoting live, so the OUTPUT chars are quoted (CTLESC)
/// — `"$p"` with p='*' is a literal `*` pattern while unquoted `$p` stays a
/// wildcard. The shared embedded expander strips that context, so the span
/// content is expanded here and every output char is literal-marked; the
/// caller's slot-restore then splices the marked text back after decoding.
pub(in crate::executor) fn mask_quoted_pattern_spans(
    pattern: &str,
    exec: &Executor,
    slots: &mut Vec<String>,
) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0usize;
    let mut single = false;
    let mut masked = String::with_capacity(pattern.len());
    while i < chars.len() {
        let ch = chars[i];
        if single {
            masked.push(ch);
            if ch == '\'' {
                single = false;
            }
            i += 1;
            continue;
        }
        if ch == '\'' {
            single = true;
            masked.push(ch);
            i += 1;
            continue;
        }
        if ch == '\\' && i + 1 < chars.len() {
            masked.push(ch);
            masked.push(chars[i + 1]);
            i += 2;
            continue;
        }
        // A `$(...)` / `` `...` `` / `${...}` unit is expanded later by the
        // shared expander; a `"` inside it belongs to the substitution body,
        // not to the pattern quoting layer (parse.y parse_comsub nests the
        // context). Skip the unit so `$(printf "%s" \\)` keeps its inner
        // quotes out of this mask.
        if ch == '$' && chars.get(i + 1) == Some(&'(') {
            if let Some(end) = crate::lexer::skip_parenthesized_unit_corrected(&chars, i + 1) {
                masked.extend(chars[i..end].iter());
                i = end;
                continue;
            }
        }
        if ch == '$' && chars.get(i + 1) == Some(&'{') {
            let rest: String = chars[i + 2..].iter().collect();
            if let Some(end) = crate::executor::parameter_ops::matching_parameter_brace(&rest) {
                masked.extend(chars[i..i + 2 + end + 1].iter());
                i += 2 + end + 1;
                continue;
            }
        }
        if ch == '`' {
            masked.push(ch);
            i += 1;
            while i < chars.len() {
                let bt = chars[i];
                masked.push(bt);
                i += 1;
                if bt == '\\' && i < chars.len() {
                    masked.push(chars[i]);
                    i += 1;
                    continue;
                }
                if bt == '`' {
                    break;
                }
            }
            continue;
        }
        // `$"..."` locale strings expand exactly like `"..."` in the C
        // locale (subst.c: the dollar is part of the quote syntax); keep the
        // `$` inside the masked span instead of leaking it into the pattern.
        let dollar_locale = ch == '$' && chars.get(i + 1) == Some(&'"');
        if ch == '"' || dollar_locale {
            let mut j = i + if dollar_locale { 2 } else { 1 };
            let mut content = String::new();
            let mut closed = false;
            while j < chars.len() {
                let inner = chars[j];
                if inner == '\\' && j + 1 < chars.len() {
                    let next = chars[j + 1];
                    // Inside `"..."`, `\` quotes only `$`, `` ` ``, `"`,
                    // `\`, and newline (GNU subst.c
                    // string_extract_double_quoted); every other `\X` is
                    // literal backslash + X. The embedded expander PRESERVES
                    // backslash sequences verbatim (it does not collapse
                    // `\\`), so pushing the doubled form doubled the final
                    // marker output and the pattern failed to match
                    // (`${v#"C:\Users"}` — rubash#127): emit exactly one
                    // backslash per source backslash; the post-expansion
                    // marking turns it into the proven single-literal form.
                    // A `\\` pair is likewise ONE literal backslash.
                    match next {
                        '$' | '`' | '"' => {
                            content.push(inner);
                            content.push(next);
                        }
                        '\'' => {
                            content.push('\\');
                            content.push('\\');
                            content.push('\\');
                            content.push(next);
                        }
                        '\\' => {
                            content.push('\\');
                        }
                        _ => {
                            content.push('\\');
                            content.push(next);
                        }
                    }
                    j += 2;
                    continue;
                }
                // Inside `"..."`, a `$(...)` / `` `...` `` / `${...}` is a
                // nested substitution with its own quoting rules — a `"`
                // inside it does NOT close the outer quote (parse.y
                // parse_matched_pair + parse_comsub nest the contexts).
                // Skip each as a unit so `"$(printf "%s" \\)"` keeps the
                // inner `"%s"` quotes inside the comsub.
                if inner == '$' && chars.get(j + 1) == Some(&'(') {
                    if let Some(end) =
                        crate::lexer::skip_parenthesized_unit_corrected(&chars, j + 1)
                    {
                        content.extend(chars[j..end].iter());
                        j = end;
                        continue;
                    }
                }
                if inner == '$' && chars.get(j + 1) == Some(&'{') {
                    let rest: String = chars[j + 2..].iter().collect();
                    if let Some(end) =
                        crate::executor::parameter_ops::matching_parameter_brace(&rest)
                    {
                        content.extend(chars[j..j + 2 + end + 1].iter());
                        j += 2 + end + 1;
                        continue;
                    }
                }
                if inner == '`' {
                    content.push(inner);
                    j += 1;
                    while j < chars.len() {
                        let bt = chars[j];
                        content.push(bt);
                        j += 1;
                        if bt == '\\' && j < chars.len() {
                            content.push(chars[j]);
                            j += 1;
                            continue;
                        }
                        if bt == '`' {
                            break;
                        }
                    }
                    continue;
                }
                if inner == '"' {
                    closed = true;
                    j += 1;
                    break;
                }
                if inner == '\'' {
                    // Inside `"..."` a `'` is literal data (GNU
                    // string_extract_double_quoted keeps it verbatim); a
                    // bare `'` handed to the embedded expander would be
                    // eaten as an unclosed quote opener, so feed it the
                    // escaped form (`${t//"'"/X}` in quote1.sub).
                    content.push('\\');
                    content.push(inner);
                    j += 1;
                    continue;
                }
                content.push(inner);
                j += 1;
            }
            if !closed {
                masked.push('"');
                i += 1;
                continue;
            }
            let expanded =
                exec.expand_embedded_parameters_preserving_escaped_single_quotes(&content);
            let mut marked = String::new();
            push_literal_pattern_str(&mut marked, &expanded);
            slots.push(marked);
            masked.push(crate::executor::markers::IFS_GLUE);
            masked.push_str(&(slots.len() - 1).to_string());
            i = j;
            continue;
        }
        masked.push(ch);
        i += 1;
    }
    masked
}

fn push_quoted_pattern_char(output: &mut String, ch: char) {
    if ch == '\'' {
        // A decoded literal quote must survive the embedded-parameter
        // expander, which drops a bare quote as an unclosed span. Emit it
        // escaped; the expander turns \\' into data.
        output.push('\\');
        output.push('\'');
        return;
    }
    if matches!(ch, '*' | '?' | '[' | '\\') {
        output.push(crate::executor::markers::CTLESC);
    }
    output.push(ch);
}
