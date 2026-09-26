pub(in crate::builtins::declare) fn split_storage_words(
    value: &str,
) -> impl Iterator<Item = String> + '_ {
    StorageWordIter {
        input: value,
        offset: 0,
    }
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
            // keep the backslash in the token so pathname expansion can see
            // it and skip globbing; unquote_storage_value removes it later.
            if ch == '\\' && !in_double && !in_single {
                word.push(ch);
                escaped = true;
                continue;
            }
            // Expansion-produced whitespace tagged by the compound walker
            // (embedded_mutations expansion_ws_marked): glue the marker and
            // its whitespace into the word so assoc kv-pairs keep them;
            // indexed callers re-split on the marker. The \x1c
            // IFS-protection sentinel takes the same glued form here.
            if ch == crate::executor::markers::IFS_GLUE
                || ch == crate::executor::COMPOUND_EXPANSION_WS_TAG
            {
                word.push(ch);
                escaped = true;
                continue;
            }
            if ch == '$' && !in_single && matches!(chars.peek(), Some((_, '{'))) {
                // GNU parse_matched_pair scans a `${...}` body with its own
                // nested-pair quote state: body whitespace never splits the
                // compound-assignment word.
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
            // GNU expand_compound_array_assignment (arrayfunc.c:580)
            // re-parses the compound body with parse_string_to_word_list, so
            // `$(...)` and backtick substitutions are single lexical words —
            // whitespace inside them never splits (array.tests:
            // declare -a e=$y with y='($(echo Darwin))').
            if ch == '$' && !in_single && matches!(chars.peek(), Some((_, '('))) {
                word.push(ch);
                word.push('(');
                chars.next();
                let rest = &self.input[self.offset + relative + 2..];
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

pub(in crate::builtins::declare) fn unquote_storage_value(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        return unquote_ansi_c_storage(inner);
    }

    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        // Bare value (not wrapped whole-token in quotes): GNU dequote_string
        // (subst.c:4807) removes quoting positionally — `'...'`, `"..."`,
        // `$'...'` spans and `\x` escapes all decode mid-word, so a
        // `[0]=$'\001'` element keeps its octal escapes inside the
        // dollar-single-quote span instead of losing the backslash to a
        // flat strip (array29.sub stored "001" for byte 0x01).
        let mut decoded = String::new();
        let chars: Vec<char> = value.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '\\' {
                if let Some(next) = chars.get(i + 1) {
                    decoded.push(*next);
                    i += 2;
                } else {
                    decoded.push('\\');
                    i += 1;
                }
                continue;
            }
            let (quote, ansi) = if ch == '$' && chars.get(i + 1) == Some(&'\'') {
                i += 1;
                ('\'', true)
            } else if ch == '\'' || ch == '"' {
                (ch, false)
            } else {
                decoded.push(ch);
                i += 1;
                continue;
            };
            i += 1;
            let mut span = String::new();
            while i < chars.len() {
                let c = chars[i];
                if ansi && c == '\\' {
                    span.push(c);
                    if let Some(next) = chars.get(i + 1) {
                        span.push(*next);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    continue;
                }
                if !ansi
                    && quote == '"'
                    && c == '\\'
                    && matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`'))
                {
                    span.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if c == quote {
                    i += 1;
                    break;
                }
                span.push(c);
                i += 1;
            }
            if ansi {
                decoded.push_str(&crate::lexer::decode_ansi_c_quoted(&span));
            } else {
                decoded.push_str(&span);
            }
        }
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
    unquoted
        .replace(crate::executor::markers::IFS_GLUE, "")
        .replace(crate::executor::COMPOUND_EXPANSION_WS_TAG, "")
}

fn unquote_ansi_c_storage(value: &str) -> String {
    // GNU assign_compound_array_list runs each raw compound word through
    // the same quote removal as an ordinary word, so a stored $'...' form
    // (ansic_quote at arrays/storage.rs emits \ooo octal, \E, \a, \xHH,
    // \uXXXX escapes for non-printing bytes) decodes through the real
    // ANSI-C decoder, not a partial escape table — the hand-rolled map
    // here dropped the backslash and left "ab001cd" for $'ab\001cd'
    // (assoc15.sub). Plain decode is right for this consumer: the key and
    // value text goes straight to comparisons and declare -p output, so a
    // decoded `'` must arrive as the real character, not a data marker.
    crate::lexer::decode_ansi_c_quoted(value)
}

/// GNU arrayfunc.c:610 expand_words_no_vars field-splits every indexed
/// compound element's expansion, so the \x1c-tagged expansion whitespace
/// (embedded_mutations expansion_ws_marked) is a split boundary for
/// indexed arrays even though the same bytes stay glued for associative
/// words. Empty fields drop like GNU's field splitting.
pub(in crate::builtins::declare) fn split_indexed_tagged_token(token: &str) -> Vec<String> {
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

pub(in crate::builtins::declare) fn parse_array_tokens(value: &str) -> Vec<String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_string()]
        };
    };
    split_storage_words(inner).collect()
}
