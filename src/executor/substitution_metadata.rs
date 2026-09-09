//! Typed command-substitution capture metadata.
//!
//! This is the first owner-scoped boundary corresponding to GNU Bash
//! subst.c::read_comsub. Payload bytes remain data; lexical context is kept
//! separately instead of being encoded in global C0 sentinels.
//!
//! Raw bytes travel as U+E000 sentinel + payload-char marker pairs; a literal
//! U+E000 in payload text is escaped by doubling the sentinel, so private-use
//! text (prompt glyphs such as U+E0A0) can never collide with the marker
//! space and survives every encode/decode round trip byte-exact.

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::executor) enum SubstitutionQuoteContext {
    Unquoted,
    DoubleQuoted,
    HereDocument,
}

/// Sentinel introducing a raw-byte marker pair.
pub(crate) const RAW_BYTE_MARKER_ESCAPE: u32 = 0xe000;
/// First marker payload char, encoding raw byte 0x00.
pub(crate) const RAW_BYTE_MARKER_FIRST: u32 = 0xe001;
/// Last marker payload char, encoding raw byte 0xff.
pub(crate) const RAW_BYTE_MARKER_LAST: u32 = 0xe100;

/// Build the two-char raw-byte marker (sentinel + payload char) for a byte.
pub(crate) fn encode_raw_byte_marker(byte: u8) -> String {
    let mut output = String::new();
    push_raw_byte_marker(&mut output, byte);
    output
}

/// GNU u32cconv (lib/sh/unicode.c:239-262): on a 4-byte-wchar_t platform the
/// conversion is wctomb for every value <= 0x7fffffff, so UTF-8 output uses
/// the mathematical UTF-8 form even for surrogate code points (wctomb
/// encodes U+D800 as ED A0 80) and for the 5/6-byte forms above 0x10ffff
/// (printf '\U00200000' -> F8 88 80 80 80, '\U7000002c' -> FD B0 80 80 80
/// AC under GNU bash 5.3.0). Values above 0x7fffffff fail the conversion
/// and produce no output. Bytes that cannot be carried as UTF-8 scalars
/// (surrogate encodings, the 5/6-byte forms) travel as raw-byte marker
/// pairs; printable scalar results stay as plain chars.
pub(crate) fn u32cconv_utf8_text(value: u32) -> String {
    // A representable Unicode scalar (everything except the surrogate
    // range) travels as its own char: glob2.sub uses IFS=$'\u3b1' and the
    // IFS splitter must see exactly one delimiter char, not a marker-pair
    // byte sequence. Only values with no char form need the raw-byte
    // marker carrier.
    if let Some(ch) = char::from_u32(value) {
        return String::from(ch);
    }
    if value > 0x7fff_ffff {
        return String::new();
    }
    let length = if value < 0x800 {
        2
    } else if value < 0x1_0000 {
        3
    } else if value < 0x20_0000 {
        4
    } else if value < 0x400_0000 {
        5
    } else {
        6
    };
    let first_mask: u32 = match length {
        2 => 0xc0,
        3 => 0xe0,
        4 => 0xf0,
        5 => 0xf8,
        _ => 0xfc,
    };
    let first_shift = 6 * (length - 1);
    let mut output = String::new();
    push_u32cconv_byte(&mut output, first_mask | (value >> first_shift));
    let mut shift = first_shift;
    while shift >= 6 {
        shift -= 6;
        push_u32cconv_byte(&mut output, 0x80 | ((value >> shift) & 0x3f));
    }
    output
}

fn push_u32cconv_byte(output: &mut String, byte: u32) {
    output.push_str(&encode_raw_byte_marker(byte as u8));
}

fn push_raw_byte_marker(output: &mut String, byte: u8) {
    output.push(char::from_u32(RAW_BYTE_MARKER_ESCAPE).expect("sentinel is valid"));
    output.push(char::from_u32(RAW_BYTE_MARKER_FIRST + byte as u32).expect("marker char is valid"));
}

/// Push shell text, escaping literal U+E000 sentinels by doubling them so
/// decoders can tell payload sentinels apart from marker introducers.
fn push_escaped_text(output: &mut String, text: &str) {
    for ch in text.chars() {
        if ch as u32 == RAW_BYTE_MARKER_ESCAPE {
            output.push(ch);
        }
        output.push(ch);
    }
}

pub(in crate::executor) struct SubstitutionOutput {
    pub(in crate::executor) bytes: Vec<u8>,
    pub(in crate::executor) status: i32,
    pub(in crate::executor) context: SubstitutionQuoteContext,
}

impl SubstitutionOutput {
    pub(in crate::executor) fn readback(
        mut bytes: Vec<u8>,
        status: i32,
        context: SubstitutionQuoteContext,
    ) -> Self {
        // GNU read_comsub discards NUL bytes while reading the child pipe.
        bytes.retain(|byte| *byte != 0);
        while bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        Self {
            bytes,
            status,
            context,
        }
    }

    pub(in crate::executor) fn text_lossy(&self) -> String {
        bytes_to_shell_text(&self.bytes)
    }

    /// Materialize assignment values without colliding with legacy C0 metadata.
    ///
    /// `\x14..\x1f` are still used by parser/assignment adapters, so a raw
    /// command-substitution byte in that range must use the owner-tagged byte
    /// marker before entering the legacy scalar map.
    pub(in crate::executor) fn assignment_text(&self) -> String {
        bytes_to_assignment_shell_text(&self.bytes)
    }

    /// Convert capture metadata into an expansion fragment without decoding payload bytes.
    pub(in crate::executor) fn into_fragment(self) -> ExpandedFragment {
        let quoted = !matches!(self.context, SubstitutionQuoteContext::Unquoted);
        ExpandedFragment {
            bytes: self.bytes,
            quoted,
            splittable: !quoted,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::executor) enum SubstitutionSplitPolicy {
    Split,
    NoSplit,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::executor) struct ExpandedFragment {
    pub(in crate::executor) bytes: Vec<u8>,
    pub(in crate::executor) quoted: bool,
    pub(in crate::executor) splittable: bool,
}

impl ExpandedFragment {
    #[allow(dead_code)]
    pub(in crate::executor) fn literal(text: &str, quoted: bool) -> Self {
        Self {
            bytes: text.as_bytes().to_vec(),
            quoted,
            splittable: false,
        }
    }

    #[allow(dead_code)]
    pub(in crate::executor) fn expanded(text: &str, quoted: bool) -> Self {
        Self {
            bytes: text.as_bytes().to_vec(),
            quoted,
            splittable: true,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::executor) struct ExpandedWord {
    pub(in crate::executor) fragments: Vec<ExpandedFragment>,
    pub(in crate::executor) status: Option<i32>,
}

impl ExpandedWord {
    #[allow(dead_code)]
    pub(in crate::executor) fn append_literal(&mut self, text: &str, quoted: bool) {
        self.fragments.push(ExpandedFragment::literal(text, quoted));
    }

    #[allow(dead_code)]
    pub(in crate::executor) fn append_substitution(&mut self, output: SubstitutionOutput) {
        self.status = Some(output.status);
        self.fragments.push(output.into_fragment());
    }

    #[allow(dead_code)]
    pub(in crate::executor) fn split(
        &self,
        ifs: Option<&str>,
        policy: SubstitutionSplitPolicy,
    ) -> Vec<String> {
        split_expanded_fragments(&self.fragments, ifs, policy)
    }

    #[allow(dead_code)]
    pub(in crate::executor) fn materialize_lossy_at_boundary(&self) -> String {
        let bytes = self
            .fragments
            .iter()
            .flat_map(|fragment| fragment.bytes.iter().copied())
            .collect::<Vec<_>>();
        bytes_to_shell_text(&bytes)
    }
}

/// Read a file-backed input redirection target as shell text.
///
/// GNU redir.c opens input redirection targets and feeds raw bytes to the
/// reading owner (read builtin and descriptors duplicated from the entry).
/// Rubash stores virtual input records as String, so strict UTF-8 decoding
/// here either dropped the record entirely or aborted the whole redirect
/// when the file contained invalid sequences. Encode invalid bytes with the
/// owner-tagged RAW_BYTE_MARKER contract so downstream owners keep byte
/// provenance until their own output boundary decodes exactly once.
pub(crate) fn read_shell_input_file(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(bytes_to_shell_text(&bytes))
}
/// Decode owner-tagged raw-byte markers back into real bytes.
///
/// This is the exact-once consumer boundary for byte sinks such as
/// child-process stdin writers and final pipeline output materialization.
/// Only sentinel-introduced marker pairs decode to bytes; every other code
/// point, including private-use glyphs, keeps its UTF-8 encoding.
pub(crate) fn shell_text_to_raw_bytes(text: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch as u32 != RAW_BYTE_MARKER_ESCAPE {
            let mut encoded = [0; 4];
            output.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            continue;
        }
        match chars.peek().copied() {
            Some(next) if next as u32 == RAW_BYTE_MARKER_ESCAPE => {
                chars.next();
                let mut encoded = [0; 4];
                output.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            }
            Some(next)
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(next as u32)) =>
            {
                chars.next();
                output.push((next as u32 - RAW_BYTE_MARKER_FIRST) as u8);
            }
            _ => {
                let mut encoded = [0; 4];
                output.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    output
}

/// Decode marker characters embedded in an otherwise byte-oriented output.
///
/// Echo can receive shell text from a read record, but it also emits raw
/// bytes for -e octal and hexadecimal escapes. Decode the reserved UTF-8
/// encoding in-place so those already-raw bytes remain untouched.
pub(crate) fn decode_raw_byte_markers(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let Some((ch, char_len)) = next_char(bytes, index) else {
            output.push(bytes[index]);
            index += 1;
            continue;
        };
        if ch as u32 != RAW_BYTE_MARKER_ESCAPE {
            output.extend_from_slice(&bytes[index..index + char_len]);
            index += char_len;
            continue;
        }
        match next_char(bytes, index + char_len) {
            Some((next_ch, next_len)) if next_ch as u32 == RAW_BYTE_MARKER_ESCAPE => {
                output.extend_from_slice(&bytes[index..index + char_len]);
                index += char_len + next_len;
            }
            Some((next_ch, next_len))
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(next_ch as u32)) =>
            {
                output.push((next_ch as u32 - RAW_BYTE_MARKER_FIRST) as u8);
                index += char_len + next_len;
            }
            _ => {
                output.extend_from_slice(&bytes[index..index + char_len]);
                index += char_len;
            }
        }
    }
    output
}

fn next_char(bytes: &[u8], index: usize) -> Option<(char, usize)> {
    if index >= bytes.len() {
        return None;
    }
    let len = utf8_sequence_len(bytes[index])?;
    if index + len > bytes.len() {
        return None;
    }
    let chunk = std::str::from_utf8(&bytes[index..index + len]).ok()?;
    Some((chunk.chars().next()?, len))
}

fn utf8_sequence_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}
pub(crate) fn bytes_to_assignment_shell_text(bytes: &[u8]) -> String {
    let text = bytes_to_shell_text(bytes);
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        let codepoint = ch as u32;
        if (0x14..=0x1f).contains(&codepoint) {
            push_raw_byte_marker(&mut output, codepoint as u8);
        } else {
            output.push(ch);
        }
    }
    output
}

pub(crate) fn bytes_to_shell_text(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                push_escaped_text(&mut output, text);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                push_escaped_text(
                    &mut output,
                    std::str::from_utf8(&remaining[..valid]).unwrap_or_default(),
                );
                push_raw_byte_marker(&mut output, remaining[valid]);
                remaining = &remaining[valid + 1..];
            }
        }
    }
    output
}

#[allow(dead_code)]
pub(in crate::executor) fn split_expanded_fragments(
    fragments: &[ExpandedFragment],
    ifs: Option<&str>,
    policy: SubstitutionSplitPolicy,
) -> Vec<String> {
    if policy == SubstitutionSplitPolicy::NoSplit {
        let bytes = fragments
            .iter()
            .flat_map(|fragment| fragment.bytes.iter().copied())
            .collect::<Vec<_>>();
        return vec![bytes_to_shell_text(&bytes)];
    }
    let ifs = ifs.unwrap_or(" \t\n");
    let whitespace: Vec<u8> = ifs
        .bytes()
        .filter(|byte| byte.is_ascii_whitespace())
        .collect();
    let non_whitespace: Vec<u8> = ifs
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut saw_unquoted = false;
    let mut pending_non_whitespace = false;
    for fragment in fragments {
        for byte in &fragment.bytes {
            let is_ifs = ifs.as_bytes().contains(byte);
            if fragment.splittable && !fragment.quoted && is_ifs {
                saw_unquoted = true;
                if non_whitespace.contains(byte) {
                    fields.push(std::mem::take(&mut current));
                    pending_non_whitespace = true;
                } else if !current.is_empty() {
                    fields.push(std::mem::take(&mut current));
                    pending_non_whitespace = false;
                }
                continue;
            }
            if !fragment.quoted && whitespace.contains(byte) {
                continue;
            }
            if pending_non_whitespace && current.is_empty() {
                pending_non_whitespace = false;
            }
            current.push(*byte as char);
        }
    }
    if !current.is_empty() || !saw_unquoted {
        fields.push(current);
    }
    fields
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::executor) struct SubstitutionSpan {
    pub(in crate::executor) start: usize,
    pub(in crate::executor) end: usize,
    pub(in crate::executor) context: SubstitutionQuoteContext,
}

#[allow(dead_code)]
pub(in crate::executor) fn scan_substitution_spans(raw: &str) -> Vec<SubstitutionSpan> {
    let chars: Vec<(usize, char)> = raw.char_indices().collect();
    let mut spans = Vec::new();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        let (offset, ch) = chars[index];
        if ch == '\\' && !single {
            index += 2;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        let dollar_paren = ch == '$'
            && chars.get(index + 1).is_some_and(|(_, next)| *next == '(')
            && chars.get(index + 2).is_none_or(|(_, next)| *next != '(');
        let backtick = ch == char::from(96);
        if !single && (dollar_paren || backtick) {
            let start = offset;
            let context = if double {
                SubstitutionQuoteContext::DoubleQuoted
            } else {
                SubstitutionQuoteContext::Unquoted
            };
            let mut depth = if dollar_paren { 1usize } else { 0usize };
            let mut cursor = index + if dollar_paren { 2 } else { 1 };
            let mut inner_single = false;
            let mut inner_double = false;
            // Track `case ... esac` pattern lists inside the span: a pattern
            // clause's `)` (`case 1 in 1) echo;; esac`) must not close the
            // substitution. Same word-boundary rules as the lexer's comsub
            // scanner (lexer/skip.rs) and the embedded walker.
            let mut inner_case_depth = 0usize;
            let mut inner_word = String::new();
            let mut inner_word_boundary = true;
            let mut inner_current_word_boundary = true;
            while cursor < chars.len() {
                let (_, inner) = chars[cursor];
                if inner == '\\' && !inner_single {
                    cursor += 2;
                    continue;
                }
                if backtick && inner == char::from(96) && !inner_single && !inner_double {
                    spans.push(SubstitutionSpan {
                        start,
                        end: chars[cursor].0 + 1,
                        context,
                    });
                    index = cursor + 1;
                    break;
                }
                if dollar_paren {
                    crate::executor::embedded_mutations::update_command_substitution_case_depth(
                        inner,
                        inner_single,
                        inner_double,
                        &mut inner_word,
                        &mut inner_case_depth,
                        &mut inner_word_boundary,
                        &mut inner_current_word_boundary,
                        &raw[cursor + 1..],
                    );
                    if inner == '\'' && !inner_double {
                        inner_single = !inner_single;
                    }
                    if inner == '"' && !inner_single {
                        inner_double = !inner_double;
                    }
                    if !inner_single
                        && !inner_double
                        && inner == '$'
                        && chars.get(cursor + 1).is_some_and(|(_, next)| *next == '(')
                    {
                        depth += 1;
                        cursor += 2;
                        continue;
                    }
                    if !inner_single && !inner_double && inner == ')' && inner_case_depth == 0 {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            spans.push(SubstitutionSpan {
                                start,
                                end: chars[cursor].0 + 1,
                                context,
                            });
                            index = cursor + 1;
                            break;
                        }
                    }
                }
                cursor += 1;
            }
            if index <= cursor {
                index = cursor;
            }
            continue;
        }
        index += 1;
    }
    spans
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::executor) struct RawWordFragment {
    pub(in crate::executor) text: String,
    pub(in crate::executor) substitution: bool,
    pub(in crate::executor) context: Option<SubstitutionQuoteContext>,
}

#[allow(dead_code)]
pub(in crate::executor) fn split_raw_word_fragments(raw: &str) -> Vec<RawWordFragment> {
    let spans = scan_substitution_spans(raw);
    if spans.is_empty() {
        return vec![RawWordFragment {
            text: raw.to_string(),
            substitution: false,
            context: None,
        }];
    }
    let mut fragments = Vec::new();
    let mut cursor = 0usize;
    for span in spans {
        if span.start > cursor {
            fragments.push(RawWordFragment {
                text: raw[cursor..span.start].to_string(),
                substitution: false,
                context: None,
            });
        }
        fragments.push(RawWordFragment {
            text: raw[span.start..span.end].to_string(),
            substitution: true,
            context: Some(span.context),
        });
        cursor = span.end;
    }
    if cursor < raw.len() {
        fragments.push(RawWordFragment {
            text: raw[cursor..].to_string(),
            substitution: false,
            context: None,
        });
    }
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::command_substitution_value_needs_payload_protection;

    #[test]
    fn payload_protection_boundary_rejects_lexical_backticks_and_existing_tokens() {
        assert!(command_substitution_value_needs_payload_protection(
            "$x", "a\x1a"
        ));
        assert!(!command_substitution_value_needs_payload_protection(
            "`$x`", "a\x1a"
        ));
        assert!(!command_substitution_value_needs_payload_protection(
            "$x",
            "__RUBASH_CSB1_1a;",
        ));
        assert!(!command_substitution_value_needs_payload_protection(
            "literal", "a\x1a"
        ));
    }

    #[test]
    fn span_scanner_tracks_mixed_quote_contexts() {
        let spans = scan_substitution_spans(r#"pre$(u)"$(q)"post"#);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].context, SubstitutionQuoteContext::Unquoted);
        assert_eq!(spans[1].context, SubstitutionQuoteContext::DoubleQuoted);
    }

    #[test]
    fn span_scanner_ignores_arithmetic_expansions() {
        assert!(scan_substitution_spans("A:$(( )); B:$(printf x)").len() == 1);
        assert_eq!(scan_substitution_spans("A:$(( ))"), Vec::new());
    }

    #[test]
    fn span_scanner_ignores_nested_syntax_inside_single_quotes() {
        let spans = scan_substitution_spans(r#"'$(literal)' $(outer $(inner))"#);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].context, SubstitutionQuoteContext::Unquoted);
    }

    #[test]
    fn raw_fragments_preserve_adjacent_literal_and_substitution_spans() {
        let fragments = split_raw_word_fragments(r#"pre$(u)"$(q)"post"#);
        assert_eq!(fragments.len(), 5);
        assert_eq!(fragments[0].text, "pre");
        assert!(!fragments[0].substitution);
        assert_eq!(fragments[1].text, "$(u)");
        assert_eq!(
            fragments[1].context,
            Some(SubstitutionQuoteContext::Unquoted)
        );
        assert_eq!(fragments[2].text, "\"");
        assert_eq!(fragments[3].text, "$(q)");
        assert_eq!(
            fragments[3].context,
            Some(SubstitutionQuoteContext::DoubleQuoted)
        );
        assert_eq!(fragments[4].text, "\"post");
    }

    #[test]
    fn readback_removes_nuls_and_only_trailing_newlines() {
        let output = SubstitutionOutput::readback(
            b"a\0b\n\n".to_vec(),
            17,
            SubstitutionQuoteContext::HereDocument,
        );
        assert_eq!(output.bytes, b"ab");
        assert_eq!(output.status, 17);
        assert_eq!(output.context, SubstitutionQuoteContext::HereDocument);
        assert_eq!(output.text_lossy(), "ab");
    }

    #[test]
    fn readback_preserves_sentinels_and_invalid_utf8_until_text_conversion() {
        let output = SubstitutionOutput::readback(
            vec![0x1d, 0x1f, 0x1a, 0x15, 0xff, b'\n', b'\n'],
            23,
            SubstitutionQuoteContext::DoubleQuoted,
        );
        assert_eq!(output.bytes, vec![0x1d, 0x1f, 0x1a, 0x15, 0xff]);
        assert_eq!(output.status, 23);
        assert_eq!(output.context, SubstitutionQuoteContext::DoubleQuoted);
        assert_eq!(
            output
                .text_lossy()
                .chars()
                .map(|ch| ch as u32)
                .collect::<Vec<_>>(),
            vec![
                0x1d,
                0x1f,
                0x1a,
                0x15,
                RAW_BYTE_MARKER_ESCAPE,
                RAW_BYTE_MARKER_FIRST + 0xff,
            ],
        );
    }

    #[test]
    fn substitution_output_into_fragment_preserves_bytes_and_quote_policy() {
        let unquoted = SubstitutionOutput::readback(
            vec![0x1d, 0x1f, b'a'],
            0,
            SubstitutionQuoteContext::Unquoted,
        )
        .into_fragment();
        assert_eq!(unquoted.bytes, vec![0x1d, 0x1f, b'a']);
        assert!(!unquoted.quoted);
        assert!(unquoted.splittable);

        let quoted = SubstitutionOutput::readback(
            vec![0x1a, 0x15, b'b'],
            0,
            SubstitutionQuoteContext::DoubleQuoted,
        )
        .into_fragment();
        assert_eq!(quoted.bytes, vec![0x1a, 0x15, b'b']);
        assert!(quoted.quoted);
        assert!(!quoted.splittable);
    }

    #[test]
    fn expanded_word_tracks_status_and_materializes_only_at_boundary() {
        let mut word = ExpandedWord::default();
        word.append_literal("pre", true);
        word.append_substitution(SubstitutionOutput::readback(
            vec![0x1d, b'a', b' ', b'b'],
            41,
            SubstitutionQuoteContext::Unquoted,
        ));
        assert_eq!(word.status, Some(41));
        assert_eq!(
            word.materialize_lossy_at_boundary().as_bytes(),
            &[b'p', b'r', b'e', 0x1d, b'a', b' ', b'b']
        );
        assert_eq!(
            word.split(Some(" "), SubstitutionSplitPolicy::Split),
            vec![format!("pre{}a", char::from(0x1d)), "b".to_string()]
        );
    }

    #[test]
    fn unquoted_ifs_splits_but_quoted_fragment_does_not() {
        let fragments = [
            ExpandedFragment::expanded("a  b", false),
            ExpandedFragment::literal(" c d", true),
        ];
        assert_eq!(
            split_expanded_fragments(&fragments, Some(" \t\n"), SubstitutionSplitPolicy::Split),
            vec!["a", "b c d"]
        );
    }

    #[test]
    fn non_whitespace_ifs_preserve_interior_empty_fields() {
        let fragments = [ExpandedFragment::expanded("a::b:", false)];
        assert_eq!(
            split_expanded_fragments(&fragments, Some(":"), SubstitutionSplitPolicy::Split),
            vec!["a", "", "b"]
        );
    }

    #[test]
    fn literal_ifs_bytes_are_not_field_split() {
        let fragments = [ExpandedFragment::literal("a::b:", false)];
        assert_eq!(
            split_expanded_fragments(&fragments, Some(":"), SubstitutionSplitPolicy::Split),
            vec!["a::b:"],
        );
    }

    #[test]
    fn no_split_keeps_empty_and_adjacent_fragments_as_one_word() {
        let fragments = [
            ExpandedFragment::literal("pre", true),
            ExpandedFragment::literal("", true),
            ExpandedFragment::literal("post", true),
        ];
        assert_eq!(
            split_expanded_fragments(&fragments, Some(":"), SubstitutionSplitPolicy::NoSplit),
            vec!["prepost"]
        );
    }

    #[test]
    fn no_split_materializes_invalid_bytes_as_raw_markers() {
        let fragments = [ExpandedFragment {
            bytes: vec![b'a', 0xff, b'b'],
            quoted: true,
            splittable: false,
        }];
        let fields = split_expanded_fragments(&fragments, None, SubstitutionSplitPolicy::NoSplit);
        assert_eq!(
            fields[0].chars().map(|ch| ch as u32).collect::<Vec<_>>(),
            vec![
                b'a' as u32,
                RAW_BYTE_MARKER_ESCAPE,
                RAW_BYTE_MARKER_FIRST + 0xff,
                b'b' as u32,
            ],
        );
    }

    #[test]
    fn readback_keeps_child_quote_bytes_as_data() {
        let output = SubstitutionOutput::readback(
            b"\"x y\"\n".to_vec(),
            0,
            SubstitutionQuoteContext::DoubleQuoted,
        );
        assert_eq!(output.text_lossy(), "\"x y\"");
    }

    #[test]
    fn read_shell_input_file_preserves_invalid_utf8_as_raw_markers() {
        let dir =
            std::env::temp_dir().join(format!("rubash-read-input-bytes-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("input.bin");
        std::fs::write(&path, [b'a', b'b', 0xff, b'\n']).unwrap();

        let text = read_shell_input_file(&path).unwrap();
        assert_eq!(
            text.chars().map(|ch| ch as u32).collect::<Vec<_>>(),
            vec![
                b'a' as u32,
                b'b' as u32,
                RAW_BYTE_MARKER_ESCAPE,
                RAW_BYTE_MARKER_FIRST + 0xff,
                b'\n' as u32,
            ]
        );

        // Valid UTF-8 segments keep their exact bytes, including multibyte
        // characters adjacent to an invalid sequence.
        std::fs::write(&path, vec![0xe4, 0xbd, 0xa0, 0xff, b'o', b'k']).unwrap();
        let mixed = read_shell_input_file(&path).unwrap();
        assert_eq!(
            mixed.chars().map(|ch| ch as u32).collect::<Vec<_>>(),
            vec![
                0x4f60,
                RAW_BYTE_MARKER_ESCAPE,
                RAW_BYTE_MARKER_FIRST + 0xff,
                b'o' as u32,
                b'k' as u32
            ],
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_text_roundtrip_preserves_raw_bytes_and_valid_utf8() {
        let bytes = vec![b'a', 0xff, b'\n', 0xe4, 0xbd, 0xa0];
        let encoded = bytes_to_shell_text(&bytes);
        assert_eq!(shell_text_to_raw_bytes(&encoded), bytes);
        // Valid UTF-8 passes through without marker encoding.
        let text = "plain \u{4f60} text";
        assert_eq!(
            String::from_utf8(shell_text_to_raw_bytes(text)).unwrap(),
            text.to_string()
        );
    }

    #[test]
    fn private_use_glyphs_survive_marker_decode() {
        // Prompt themes use private-use glyphs (U+E0A0 git branch,
        // U+E0B0 powerline separators). They are payload text, not markers.
        let text = "\u{e0a0}\u{e0b0} main";
        assert_eq!(
            String::from_utf8(decode_raw_byte_markers(text.as_bytes())).unwrap(),
            text
        );
        assert_eq!(shell_text_to_raw_bytes(text), text.as_bytes().to_vec());
    }

    #[test]
    fn prompt_assignment_markers_restore_c0_but_not_pua_glyphs() {
        // Starship-style PS1: \[ ESC [1;35m \] U+E0A0 " main". The ESC byte
        // is marker-encoded for scalar storage and restored at the prompt
        // boundary; the private-use glyph passes through byte-exact.
        let mut ps1 = b"\\[\x1b[1;35m\\]".to_vec();
        ps1.extend_from_slice("\u{e0a0}".as_bytes());
        ps1.extend_from_slice(b" main");
        let stored = bytes_to_assignment_shell_text(&ps1);
        assert_eq!(
            decode_raw_byte_markers(stored.as_bytes()),
            ps1,
            "prompt boundary must restore ESC exactly and keep the glyph"
        );
    }

    #[test]
    fn literal_sentinel_roundtrips_through_marker_encoding() {
        let text = "a\u{e000}b\u{e0a0}";
        let encoded = bytes_to_shell_text(text.as_bytes());
        assert_ne!(encoded, text, "literal sentinels must be escaped");
        assert_eq!(
            String::from_utf8(shell_text_to_raw_bytes(&encoded)).unwrap(),
            text
        );
        let invalid = vec![b'a', 0xe0, b'b'];
        assert_eq!(
            shell_text_to_raw_bytes(&bytes_to_shell_text(&invalid)),
            invalid
        );
    }
}
