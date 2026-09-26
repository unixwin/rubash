use super::*;
use crate::executor::markers::DATA_DOLLAR;
use crate::executor::parameter_core::word_contains_current_shell_command_substitution;

// The quoted-null carrier (GNU subst.c CTLNUL): an empty quoted span
// ('', "", an unset "$e", a no-output "$( : )") in a ${var+word}
// alternate leaves a marker in the expansion so the field splitter can
// keep it as an empty argument (subst.c expand_word_internal adds CTLNUL
// to the word text at 11940-11944 / 11841-11847, and list_string:3181-3186
// keeps a QUOTED_NULL field as an empty argv entry). U+E000/U+E001 are
// taken by the raw-byte and assignment data-quote sentinels; U+E002 free.
pub(crate) const QUOTED_NULL_MARKER: char = crate::executor::markers::QUOTED_NULL_MARKER;

// Whitespace that an expansion produced inside a quoted region of an
// alternate word must survive field splitting (GNU carries CTLESC on
// quoted expansion results); the \x1c prefix marks it for the splitter.
pub(in crate::executor) fn mark_alternate_whitespace(value: &str) -> String {
    let mut marked = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, ' ' | '\t' | '\n') {
            marked.push(crate::executor::markers::IFS_GLUE);
        }
        marked.push(ch);
    }
    marked
}

// GNU arrayfunc.c: an associative compound word is expanded with
// expand_assignment_string_to_string / expand_subscript_string (line 652,
// 817, 865), which never field-splits, while indexed elements go through
// expand_words_no_vars (line 610), which does. In preserve_quotes mode the
// walker therefore tags whitespace produced by an expansion in an unquoted
// region: the associative storage pass keeps it glued inside the element
// and the indexed pass re-splits on it. Alternate rhs words keep their
// existing quoted-region marking.
fn expansion_ws_marked(alternate: bool, preserve_quotes: bool, in_double: bool) -> bool {
    (alternate && in_double) || (preserve_quotes && !in_double)
}

// The compound tag must NOT be \x1c: that byte is the IFS-protection
// sentinel (command_prepare.rs mark_literal_ifs_chars /
// strip_ifs_protection_markers), and the assignment-builtin boundary
// strips it before the value reaches storage. U+E309 is disjoint from the
// DATA_* quote sentinels (E301-E308) and survives to split_storage_words,
// where it glues its whitespace into the element word.
pub(crate) const COMPOUND_EXPANSION_WS_TAG: char =
    crate::executor::markers::COMPOUND_EXPANSION_WS_TAG;

pub(in crate::executor) fn mark_expansion_whitespace(value: &str, preserve_quotes: bool) -> String {
    if !preserve_quotes {
        return mark_alternate_whitespace(value);
    }
    let mut marked = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, ' ' | '\t' | '\n') {
            marked.push(COMPOUND_EXPANSION_WS_TAG);
        }
        marked.push(ch);
    }
    marked
}

impl Executor {
    pub(in crate::executor) fn expand_embedded_parameters_mut(&mut self, word: &str) -> String {
        self.expand_embedded_parameters_mut_with_context(word, SubstitutionQuoteContext::Unquoted)
    }

    pub(in crate::executor) fn expand_embedded_parameters_mut_with_context(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        let heredoc = matches!(context, SubstitutionQuoteContext::HereDocument);
        self.expand_embedded_parameters_mut_inner(word, context, heredoc, false, false)
    }

    // Compound array assignment RHS (`a=( ... )`): GNU defers expansion to
    // the per-word pass (arrayfunc.c:557 expand_compound_array_assignment
    // tokenizes the raw text; each element is then expanded individually),
    // so quote characters produced by an element's quote removal are DATA
    // and are never re-scanned as syntax. Rubash expands the whole `( ... )`
    // body in one pass and re-splits it, which would read a dequoted `"`
    // back as a delimiter. This variant keeps the element quote syntax in
    // the output so the storage tokenizer sees the same quoting GNU's raw
    // tokenization kept (assoc11.sub: ('"' dquote "'" squote) must store
    // the keys " and ').
    pub(in crate::executor) fn expand_compound_assignment_parameters_mut(
        &mut self,
        word: &str,
    ) -> String {
        self.expand_embedded_parameters_mut_inner(
            word,
            SubstitutionQuoteContext::Unquoted,
            false,
            false,
            true,
        )
    }

    // Alternate-operator rhs (`${var-word}` word half) expansion: the same
    // walker with Unquoted quote removal plus two additions GNU applies to
    // an unquoted word's expansion (subst.c expand_string_for_rhs with
    // quoted == 0): whitespace that was quoted or escaped is marked with
    // the \x1c sentinel so the field splitter keeps it glued to its field
    // (posixexp2 37, more-exp ${B:-"$A"}), and backslash escapes resolve
    // per unquoted-word rules (\$name is a protected literal $ that must
    // NOT expand -- rhs-exp t33/t34 -- while \p drops the backslash,
    // t47). The quote structure stays in the word during the walk, so
    // expansions that happen inside a quoted region see in_double and
    // their own whitespace gets the sentinel too.
    pub(in crate::executor) fn expand_embedded_parameters_alternate_mut(
        &mut self,
        word: &str,
    ) -> String {
        self.expand_embedded_parameters_mut_inner(
            word,
            SubstitutionQuoteContext::Unquoted,
            false,
            true,
            false,
        )
    }

    // GNU redir.c:373 (r_reading_string) runs the redirectee word through
    // expand_assignment_string_to_string, and inside that pipeline every
    // single-quoted span is literal data (subst.c:11882-11886 — the '...'
    // body goes to add_quoted_string without consulting the comsub
    // scanner). The parser stores the raw herestring word (span boundaries
    // intact, redirections.rs assign_here_string_redirect_raw), so walk it
    // segment by segment: '...' spans verbatim, "..." spans and unquoted
    // segments through the expansion walker, then a final
    // dequote_string-equivalent carrier decode. Handing the
    // quote-stripped value straight to the comsub walker executed
    // `<<<'a'\''`b`'` and leaked \x17 (rubash#153 n20/n21/n25).
    pub(in crate::executor) fn expand_here_string_mut(&mut self, word: &str) -> String {
        if let Some(pre) = preexpanded_stdin_body(word) {
            return crate::executor::execution_misc::decode_stdin_body_enq(pre);
        }
        let expanded = self.expand_here_string_segments(word);
        crate::executor::execution_misc::decode_stdin_body_enq(
            &crate::executor::markers::decode_word_position_carriers(&expanded),
        )
    }

    /// Segment walker for the here-string word: a top-level `'...'` span is
    /// literal (GNU subst.c:11882-11886); `$'...'` gets ANSI-C decoding;
    /// `"..."` spans and unquoted segments go through the heredoc-mode
    /// expansion walker, where a `'` inside double quotes is data (parse.y
    /// read_token_word's double-quote scanner never opens single-quote
    /// state). Substitution units (`$(...)`, `${...}`, `` `...` ``) own
    /// their internal quoting (parse.y parse_matched_pair), so the bare-
    /// segment scanner skips them whole instead of reading their quotes as
    /// span delimiters.
    fn expand_here_string_segments(&mut self, raw: &str) -> String {
        let chars: Vec<char> = raw.chars().collect();
        let mut output = String::new();
        let mut index = 0usize;

        let mut expand_segment = |executor: &mut Self, segment: &str, sink: &mut String| {
            sink.push_str(&executor.expand_embedded_parameters_mut_inner(
                segment,
                SubstitutionQuoteContext::Unquoted,
                true,
                false,
                false,
            ));
        };

        while index < chars.len() {
            if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
                let quote = chars[index + 1];
                if let Some(end) = crate::executor::compound_exec::quoted_case_pattern_end(
                    &chars,
                    index + 2,
                    quote,
                ) {
                    let body: String = chars[index + 2..end].iter().collect();
                    if quote == '\'' {
                        output.push_str(&crate::executor::parse_helpers::decode_ansi_c_escapes(
                            &body,
                        ));
                    } else {
                        expand_segment(self, &body, &mut output);
                    }
                    index = end + 1;
                    continue;
                }
            }

            if chars[index] == '\'' {
                if let Some(end) =
                    crate::executor::compound_exec::quoted_case_pattern_end(&chars, index + 1, '\'')
                {
                    output.push_str(&chars[index + 1..end].iter().collect::<String>());
                    index = end + 1;
                    continue;
                }
            }

            if chars[index] == '"' {
                if let Some(end) =
                    crate::executor::compound_exec::quoted_case_pattern_end(&chars, index + 1, '"')
                {
                    let body: String = chars[index + 1..end].iter().collect();
                    expand_segment(self, &body, &mut output);
                    index = end + 1;
                    continue;
                }
            }

            let mut pending_start = index;
            while index < chars.len()
                && chars[index] != '\''
                && chars[index] != '"'
                && !(chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')))
            {
                if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
                    if let Some(close) =
                        crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                    {
                        index = close.min(chars.len());
                        continue;
                    }
                }
                if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
                    if let Some(close) =
                        crate::executor::compound_exec::skip_braced_case_pattern_unit(
                            &chars,
                            index + 1,
                        )
                    {
                        index = close + 1;
                        continue;
                    }
                }
                if chars[index] == '`' {
                    index += 1;
                    while index < chars.len() && chars[index] != '`' {
                        if chars[index] == '\\' && index + 1 < chars.len() {
                            index += 1;
                        }
                        index += 1;
                    }
                    if index < chars.len() {
                        index += 1;
                    }
                    continue;
                }
                if chars[index] == '\\' && index + 1 < chars.len() {
                    // GNU parse.y:5366-5398 read_token_word: an unquoted
                    // backslash quotes exactly the next character, which
                    // survives as a quoted literal (got_escaped_character,
                    // parse.y:5694-5706). Flush the pending expandable run,
                    // then emit the escaped character as data.
                    if pending_start < index {
                        let segment: String = chars[pending_start..index].iter().collect();
                        expand_segment(self, &segment, &mut output);
                    }
                    output.push(chars[index + 1]);
                    index += 2;
                    pending_start = index;
                    continue;
                }
                index += 1;
            }
            if pending_start < index {
                let segment: String = chars[pending_start..index].iter().collect();
                expand_segment(self, &segment, &mut output);
            }
        }

        output
    }

    /// Expands a here-string from a typed carrier. Returns the preexpanded
    /// text if present; otherwise performs expansion.
    pub(in crate::executor) fn expand_here_string_mut_from_carrier(
        &mut self,
        carrier: &Option<crate::parser::StdinBody>,
    ) -> String {
        match carrier {
            Some(crate::parser::StdinBody::Preexpanded(text)) => {
                crate::executor::execution_misc::decode_stdin_body_enq(text)
            }
            Some(crate::parser::StdinBody::NeedsExpansion(word)) => {
                self.expand_here_string_mut(word)
            }
            None => String::new(),
        }
    }

    fn expand_embedded_parameters_mut_inner(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
        heredoc: bool,
        alternate: bool,
        preserve_quotes: bool,
    ) -> String {
        self.apply_parameter_assignment_expansions_in_word(word);
        let saved_parameter_state =
            word_contains_current_shell_command_substitution(word).then(|| {
                (
                    self.shell_state.env_vars.clone(),
                    self.shell_state.pipestatus.clone(),
                )
            });
        let expanded = self.expand_embedded_parameters_ordered_mut(
            word,
            saved_parameter_state.as_ref(),
            context,
            heredoc,
            alternate,
            preserve_quotes,
        );
        let expanded = if word.contains("$(") || word.contains('`') {
            if preserve_quotes || matches!(context, SubstitutionQuoteContext::HereDocument) {
                // Compound RHS keeps escape syntax for the storage
                // tokenizer; stripping it here would turn escaped data
                // quotes back into syntax (assoc compound elements).
                expanded
            } else {
                unescape_remaining_shell_escapes(&expanded)
                    .replace("\\\\'", "'")
                    .replace("\\'", "'")
            }
        } else {
            expanded
        };
        let restored = restore_protected_replacement_quotes(&expanded)
            .replace(DATA_DOLLAR, "$")
            .replace(crate::executor::markers::DATA_BACKTICK, "`")
            .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
            .replace(crate::lexer::PARAM_NAME_END_MARKER, "");
        // Quoted-null markers only matter to the alternate field splitter
        // (unquoted_outer_braced_alternate_values); every other consumer
        // (assignment rhs, quoted alternates) drops them like GNU's
        // dequote_list.
        if alternate {
            restored
        } else {
            restored.replace(QUOTED_NULL_MARKER, "")
        }
    }

    fn expand_embedded_parameters_ordered_mut(
        &mut self,
        word: &str,
        saved_parameter_state: Option<&(std::collections::HashMap<String, String>, Vec<i32>)>,
        context: SubstitutionQuoteContext,
        heredoc: bool,
        alternate: bool,
        preserve_quotes: bool,
    ) -> String {
        let mut output = String::new();
        let mut chars = word.chars().peekable();
        let mut in_double = false;
        // Top-level `${` ordinal for the cross-pass subscript-eval memo —
        // matches the pre-scan counter (SUB_RES_XPASS).
        let mut frag_index = 0usize;
        // Output length when the current double-quoted span opened, for the
        // quoted-null carrier below (alternate mode only).
        let mut dquote_open_len: Option<usize> = None;

        while let Some(ch) = chars.next() {
            // Alternate rhs: whitespace inside a quoted region is quote
            // data for the field splitter even after quote removal.
            if alternate && in_double && matches!(ch, ' ' | '\t' | '\n') {
                output.push(crate::executor::markers::IFS_GLUE);
                output.push(ch);
                continue;
            }
            if ch == crate::executor::markers::DATA_BACKTICK {
                output.push('`');
                continue;
            }

            if ch == crate::lexer::PARAM_NAME_END_MARKER {
                // Lexer quote removal marks where a quote boundary ends an
                // unbraced $name; the name already stopped (the marker is a
                // non-name character) and must not reach the output.
                continue;
            }

            if ch == DATA_DOLLAR {
                output.push('$');
                continue;
            }

            if ch == crate::executor::markers::DATA_SQUOTE {
                // Only command-substitution words cross a later quote-removal
                // pass; ordinary words retain their existing decoding path.
                if word.contains("$(") || word.contains('`') {
                    output.push(crate::executor::markers::DATA_SQUOTE);
                } else {
                    output.push('\'');
                }
                continue;
            }

            if ch == crate::executor::markers::DATA_DQUOTE {
                // Quoted double marks are lexer sentinels; preserve the
                // quote and its expansion context for nested single quotes.
                if matches!(context, SubstitutionQuoteContext::Unquoted) {
                    in_double = !in_double;
                }
                // Compound RHS: emit the data-double-quote carrier the
                // storage tokenizer recognizes instead of a bare quote
                // that would reopen a quoted span during the re-split.
                output.push(if preserve_quotes { '\u{E302}' } else { '"' });
                continue;
            }

            if ch == '\u{E302}' {
                // DATA_DOUBLE_QUOTE (assignment_expansion.rs): the compound
                // hoist encodes the lexer's \x18 quote sentinels as E302 so
                // they pass through untouched. They still delimit a "..."
                // region for quote-state tracking (GNU parse.y:5305
                // read_token_word keeps the dquote bit on the word), which
                // is what makes `$'` literal inside it (issue #109).
                if matches!(context, SubstitutionQuoteContext::Unquoted) {
                    in_double = !in_double;
                }
                output.push(ch);
                continue;
            }

            // Quotes that survive to expansion belong to parameter-expansion
            // bodies (the lexer keeps `${...}` verbatim). GNU removes them
            // only in unquoted expansions; a double-quoted expansion keeps
            // them in its result (`"${IFS+'}'z}"` -> `'}'z`). Heredoc text
            // treats quotes as data.
            if !heredoc && matches!(context, SubstitutionQuoteContext::Unquoted) && ch == '"' {
                let closing = in_double;
                in_double = !in_double;
                // Compound RHS: keep the element's quote syntax so the
                // storage tokenizer splits on GNU's raw tokens.
                if preserve_quotes {
                    output.push('"');
                    continue;
                }
                // A double-quoted span whose expansion produced nothing is a
                // quoted null: "" and $xxx"" become CTLNUL (subst.c
                // 11841-11847 "What we have is \"\""). Alternate mode only:
                // the marker is consumed by the alternate field splitter.
                if alternate {
                    if closing {
                        if dquote_open_len == Some(output.len()) {
                            output.push(QUOTED_NULL_MARKER);
                        }
                        dquote_open_len = None;
                    } else {
                        dquote_open_len = Some(output.len());
                    }
                }
                continue;
            }

            if !heredoc
                && matches!(context, SubstitutionQuoteContext::Unquoted)
                && ch == '\''
                && !in_double
            {
                if preserve_quotes {
                    // Compound RHS: a single-quoted element is literal text
                    // (GNU tokenizes it before any expansion); keep the
                    // quote characters so the storage tokenizer sees one
                    // word exactly like GNU's raw token stream.
                    output.push('\'');
                    for quoted_ch in chars.by_ref() {
                        output.push(quoted_ch);
                        if quoted_ch == '\'' {
                            break;
                        }
                    }
                    continue;
                }
                let span_start = output.len();
                let mut closed = false;
                for quoted_ch in chars.by_ref() {
                    if quoted_ch == '\'' {
                        closed = true;
                        break;
                    }
                    if alternate && matches!(quoted_ch, ' ' | '\t' | '\n') {
                        output.push(crate::executor::markers::IFS_GLUE);
                    }
                    output.push(quoted_ch);
                }
                // An empty single-quoted span is a quoted null (subst.c
                // 11940-11944: c = CTLNUL; goto add_character).
                if alternate && closed && output.len() == span_start {
                    output.push(QUOTED_NULL_MARKER);
                }
                continue;
            }

            if ch == '\'' && in_double {
                // GNU parse.y:5305 read_token_word only treats ' as quote
                // syntax at unquoted level; inside a "..." region it is
                // ordinary data. Emit the ANSI-C data-quote marker (the
                // CTLESC-analog used for decoded $'...' quotes, quotes.rs
                // ANSI_C_QUOTE_MARKER) so downstream quote removal can
                // never re-read it as a quote delimiter.
                output.push(crate::lexer::ANSI_C_QUOTE_MARKER);
                continue;
            }

            if ch == '\\' && alternate {
                // Unquoted-word escape rules for the alternate rhs: $, `,
                // \\, " keep their protection; an escaped blank stays out
                // of field splitting; a backslash before any other
                // character is removed (GNU quote removal).
                match chars.peek().copied() {
                    Some('`') => {
                        chars.next();
                        output.push(crate::executor::markers::DATA_BACKTICK);
                        continue;
                    }
                    Some('$') | Some('"') => {
                        let next = chars.next().unwrap();
                        if preserve_quotes {
                            // Keep the escape pair verbatim so the storage
                            // tokenizer sees escaped data rather than a
                            // quote/dollar that reopens syntax.
                            output.push('\\');
                        }
                        output.push(next);
                        continue;
                    }
                    Some('\\') => {
                        chars.next();
                        output.push('\\');
                        continue;
                    }
                    Some('\'') => {
                        chars.next();
                        if preserve_quotes && !in_double {
                            // Keep \' verbatim: the data quote must not
                            // reopen a single-quoted span in the re-split.
                            output.push('\\');
                            output.push('\'');
                            continue;
                        }
                        if in_double {
                            // GNU dquote rule: a backslash before an
                            // ordinary character is literal data.
                            output.push('\\');
                        }
                        output.push('\'');
                        continue;
                    }
                    Some('\n') | Some('\r') => {
                        chars.next();
                        continue;
                    }
                    Some(ws @ (' ' | '\t')) => {
                        chars.next();
                        output.push(crate::executor::markers::IFS_GLUE);
                        output.push(ws);
                        continue;
                    }
                    Some(other) => {
                        chars.next();
                        if in_double {
                            // GNU dquote rule: a backslash before an
                            // ordinary character is literal data.
                            output.push('\\');
                        }
                        output.push(other);
                        continue;
                    }
                    None => {}
                }
            }

            if ch == '\\' && preserve_quotes {
                // Compound RHS: GNU keeps every escape pair in the token
                // text until quote removal (parse.y:5368-5397
                // read_token_word; dequote happens in subst.c:4807
                // dequote_word). The storage tokenizer + unquote pass are
                // the same two phases, so copying `\` + next verbatim is
                // the exact port: \\ stays an escaped backslash instead of
                // collapsing to a \ that would glue the next word (assoc11
                // `\\ 5`), and \" / \' / \$ stay data instead of reopening
                // quote or expansion syntax. Newline joins are the only
                // pair GNU removes at read time.
                match chars.peek().copied() {
                    Some('\n') | Some('\r') => {
                        chars.next();
                    }
                    Some(next) => {
                        chars.next();
                        output.push('\\');
                        output.push(next);
                    }
                    None => output.push('\\'),
                }
                continue;
            }

            if ch == '\\' {
                if let Some(&next) = chars.peek() {
                    match next {
                        '`' => {
                            chars.next();
                            output.push(crate::executor::markers::DATA_BACKTICK);
                            continue;
                        }
                        // GNU subst.c: backslash before backslash is
                        // consumed, leaving one literal \.  This must
                        // happen before the `\$` check below so that
                        // `\\$var` yields `\<expanded>` rather than
                        // `\$var` (rhs-exp.tests t5/t6).
                        '\\' => {
                            chars.next();
                            output.push('\\');
                            continue;
                        }
                        // A backslash inside a parameter body protects the
                        // next special character (GNU subst.c: the backslash
                        // keeps its escaping meaning before $, `, ", \.
                        // Emit $ and " literally here: by the time the later
                        // unescape pass runs, an unprotected $ has already
                        // opened a parameter expansion (esc6/esc7 probes:
                        // "${v-\$x}" must yield a$x, not drop the $x).
                        '$' => {
                            chars.next();
                            output.push(next);
                            continue;
                        }
                        '"' if !matches!(context, SubstitutionQuoteContext::HereDocument) => {
                            chars.next();
                            output.push(next);
                            continue;
                        }
                        // GNU dquote escape set (subst.c CBSDQUOTE): inside a
                        // "..." region \ only escapes $ ` " \ newline, so \'
                        // is literal backslash + quote data. Emit the
                        // backslash self-escaped (\\) so the later
                        // unescape pass keeps it, and the ' as the ANSI-C
                        // data-quote marker (CTLESC-analog).
                        '\'' if in_double => {
                            chars.next();
                            output.push_str("\\\\");
                            output.push(crate::lexer::ANSI_C_QUOTE_MARKER);
                            continue;
                        }
                        // GNU parse.y:5368-5397 read_token_word: outside
                        // quotes ' is a quoted literal ' - the backslash
                        // is consumed and the quote is DATA (o=${c='q'}
                        // stores 'q'). Emit the ANSI-C data-quote carrier
                        // so downstream quote removal never re-reads it as
                        // a delimiter.
                        '\'' if !matches!(context, SubstitutionQuoteContext::HereDocument) => {
                            chars.next();
                            output.push(crate::lexer::ANSI_C_QUOTE_MARKER);
                            continue;
                        }
                        // Here-document bodies expand with Q_HERE_DOCUMENT,
                        // where the escape set is CBSHDOC and not CBSDQUOTE
                        // (subst.c:11628; syntax.h slashify_in_here_document
                        // = backslash, backtick, dollar). The double quote is
                        // not special inside an unquoted heredoc body, so a
                        // backslash before one is literal data (heredoc.tests
                        // "echo \""). Fall through to the literal copy.
                        _ => {}
                    }
                }
            }

            if ch == '`' {
                let mut source = String::new();
                let mut escaped = false;
                let mut closed = false;
                for source_ch in chars.by_ref() {
                    if escaped {
                        if !matches!(source_ch, '$' | '`' | '\\' | '\n' | '\r') {
                            source.push('\\');
                        }
                        source.push(source_ch);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if source_ch == '`' {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                if closed {
                    let expanded = substitution_result_visible_text(
                        &self
                            .expand_command_substitution_mut_typed_with_context(&source, context)
                            .text_lossy(),
                    );
                    let protected = protect_command_substitution_output(&expanded);
                    if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                        let value = mark_expansion_whitespace(&protected, preserve_quotes);
                        if matches!(context, SubstitutionQuoteContext::HereDocument) {
                            output.push_str(&value.replace(
                                crate::executor::markers::PROTECTED_BACKSLASH,
                                crate::executor::markers::DATA_BACKSLASH_STR,
                            ));
                        } else {
                            output.push_str(&value);
                        }
                    } else if matches!(context, SubstitutionQuoteContext::HereDocument) {
                        output.push_str(&protected.replace(
                            crate::executor::markers::PROTECTED_BACKSLASH,
                            crate::executor::markers::DATA_BACKSLASH_STR,
                        ));
                    } else {
                        output.push_str(&protected);
                    }
                } else {
                    output.push('`');
                    output.push_str(&source);
                }
                continue;
            }

            if ch != '$' {
                output.push(ch);
                continue;
            }

            match chars.peek().copied() {
                Some('?') => {
                    chars.next();
                    output.push_str(&self.exit_code.to_string());
                }
                Some('$') => {
                    chars.next();
                    output.push_str(&self.shell_pid_value().to_string());
                }
                Some('!') => {
                    chars.next();
                    output.push_str(&self.last_background_pid_value());
                }
                Some('@') => {
                    chars.next();
                    let value = self.shell_state.positional_params.join(" ");
                    if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
                }
                Some('*') => {
                    chars.next();
                    // Bash joins `$*` with the first IFS character (not a space).
                    let value = self.positional_params_star_joined();
                    if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
                }
                Some('#') => {
                    chars.next();
                    output.push_str(&self.shell_state.positional_params.len().to_string());
                }
                Some('-') => {
                    chars.next();
                    output.push_str(&self.shell_option_flags());
                }
                Some('{') => {
                    chars.next();
                    // GNU param_expand resolves one `${}` expansion once:
                    // memoize array-element fetches for this fragment so a
                    // subscript's side effects run once (AEPV_MEMO).
                    let _memo_frame = crate::executor::expand_braced_indices::AepvMemoFrame::new();
                    // Record this fragment's site (word pointer + `$`
                    // offset) so layered re-checks of the same `${}`
                    // dedup subscript side effects (SUB_RES_XPASS). When
                    // the walked word IS the `${}` fragment being
                    // evaluated (a `${name}` body re-walked inside the
                    // enclosing fragment's site), it inherits that site
                    // instead of re-keying on the synthetic string.
                    let whole_braced =
                        crate::executor::parameter_ops::braced_parameter_spans_whole_word(word)
                            && crate::executor::expand_braced_indices::sub_site_active();
                    let this_frag = frag_index;
                    frag_index += 1;
                    let _site_guard = (!whole_braced).then(|| {
                        crate::executor::expand_braced_indices::SubSiteGuard::new(this_frag)
                    });
                    if let Some(value) = self.expand_current_shell_braced_substitution(&mut chars) {
                        if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                            output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                        } else {
                            output.push_str(&value);
                        }
                    } else if matches!(context, SubstitutionQuoteContext::DoubleQuoted)
                        && self.posix_mode_enabled()
                    {
                        let remainder: String = chars.clone().collect();
                        if let Some(close) =
                            matching_parameter_brace_in_context(&remainder, true, true)
                        {
                            let consumed = word.len() - remainder.len();
                            let name = remainder[..close].to_string();
                            chars = word[consumed + close + 1..].chars().peekable();
                            let value =
                                self.expand_with_parameter_env(saved_parameter_state, |executor| {
                                    executor.expand_word_mut_with_context(
                                        &format!("${{{name}}}"),
                                        context,
                                    )
                                });
                            if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                                output
                                    .push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                            } else {
                                output.push_str(&value);
                            }
                        } else {
                            let name = collect_braced_parameter_name(&mut chars);
                            let value =
                                self.expand_with_parameter_env(saved_parameter_state, |executor| {
                                    executor.expand_word_mut_with_context(
                                        &format!("${{{name}}}"),
                                        context,
                                    )
                                });
                            if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                                output
                                    .push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                            } else {
                                output.push_str(&value);
                            }
                        }
                    } else {
                        let name = collect_braced_parameter_name(&mut chars);
                        let value =
                            self.expand_with_parameter_env(saved_parameter_state, |executor| {
                                // Propagate the outer quote context so a
                                // double-quoted "${v:-~}" keeps its quoted
                                // default-word semantics (no tilde expansion).
                                executor
                                    .expand_word_mut_with_context(&format!("${{{name}}}"), context)
                            });
                        if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                            output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                        } else {
                            output.push_str(&value);
                        }
                    }
                }
                Some('(') => {
                    chars.next();
                    if chars.peek().copied() == Some('(') {
                        chars.next();
                        let (expression, matched) =
                            collect_dollar_paren_arithmetic_expansion(&mut chars);
                        if matched {
                            // GNU subst.c:10842-10862: `$((` content is
                            // arithmetic only when the text inside the outer
                            // parens ends in `)` and survives chk_arithsub's
                            // balance check; otherwise the whole construct is
                            // a nested command substitution (`$(( echo ab
                            // cde ) )` runs `( echo ab cde )`).
                            let temp2 = expression.strip_suffix(')').unwrap_or(expression.as_str());
                            let (expression, force_comsub) =
                                if let Some(inner) = temp2.strip_suffix(')') {
                                    if arith_sub_parens_balanced(inner) {
                                        (inner.to_string(), false)
                                    } else {
                                        (format!("({temp2}"), true)
                                    }
                                } else {
                                    (format!("({temp2}"), true)
                                };
                            if force_comsub {
                                let value = protect_command_substitution_output(
                                    &self.expand_command_substitution_mut_with_context(
                                        &expression,
                                        context,
                                    ),
                                );
                                if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                                    output.push_str(&mark_expansion_whitespace(
                                        &value,
                                        preserve_quotes,
                                    ));
                                } else {
                                    output.push_str(&value);
                                }
                            } else if let Some(value) =
                                self.eval_arithmetic_expansion_value(&expression)
                            {
                                let value = value.to_string();
                                if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                                    output.push_str(&mark_expansion_whitespace(
                                        &value,
                                        preserve_quotes,
                                    ));
                                } else {
                                    output.push_str(&value);
                                }
                            } else {
                                let actual_fatal = self
                                    .shell_state
                                    .arithmetic_last_error_category
                                    .take()
                                    .is_some();
                                if (actual_fatal
                                    || crate::executor::arithmetic::arithmetic_expansion_is_fatal(
                                        &expression,
                                    ))
                                    && !embedded_command_substitution_expression(&expression)
                                {
                                    self.shell_state.arithmetic_fatal_error.set(true);
                                    if !self.shell_state.arithmetic_expansion_error.replace(true) {
                                        // GNU evalexp reports against the
                                        // post-expansion string
                                        // (expand_arith_string ran first);
                                        // the captured eval input echoes
                                        // `$var` values, not literal text.
                                        let eval_input =
                                            self.arithmetic_last_eval_input.borrow().clone();
                                        let display = if eval_input.is_empty() {
                                            expression.as_str()
                                        } else {
                                            eval_input.as_str()
                                        };
                                        if let Some(message) =
                                            crate::executor::arithmetic::arithmetic_error_message(
                                                display,
                                                true,
                                                &self.shell_state.env_vars,
                                            )
                                        {
                                            eprintln!("{}{}", self.diagnostic_prefix(), message);
                                        }
                                    }
                                } else {
                                    let value = protect_command_substitution_output(
                                        &self.expand_command_substitution_mut_with_context(
                                            &expression,
                                            context,
                                        ),
                                    );
                                    if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                                        output.push_str(&mark_expansion_whitespace(
                                            &value,
                                            preserve_quotes,
                                        ));
                                    } else {
                                        output.push_str(&value);
                                    }
                                }
                            }
                        } else {
                            output.push_str("$((");
                            output.push_str(&expression);
                        }
                        continue;
                    }

                    let (source, closed) = collect_command_substitution_source_ex(
                        &mut chars,
                        &self.shell_state.aliases,
                    );
                    if !closed {
                        // GNU parse.y parse_comsub: an unclosed `$(` reports
                        // `unexpected EOF` and the expansion fails, aborting
                        // the command while the script continues (braces.tests
                        // "${a+'$('\'}"). parser_error (error.c:300) uses
                        // yy_input_name()=="command substitution" and the
                        // inherited line_number (evalstring.c push_stream(0))
                        // — the current command line plus the newlines the
                        // comsub text consumed.
                        let eof_line = self
                            .shell_state
                            .env_vars
                            .get("__RUBASH_CURRENT_LINE")
                            .and_then(|line| line.parse::<usize>().ok())
                            .unwrap_or(1)
                            + source.lines().count().saturating_sub(1);
                        eprintln!(
                            "{}unexpected EOF while looking for matching `)'",
                            self.comsub_eof_diagnostic(eof_line)
                        );
                        self.shell_state.arithmetic_fatal_error.set(true);
                        self.shell_state.arithmetic_expansion_error.set(true);
                        continue;
                    }
                    let value =
                        protect_command_substitution_output(&substitution_result_visible_text(
                            &self.expand_command_substitution_mut_with_context(&source, context),
                        ));
                    if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
                }
                Some('[') => {
                    chars.next();
                    let (expression, matched) =
                        collect_dollar_bracket_arithmetic_expansion(&mut chars);
                    if matched {
                        if let Some(value) = self.eval_arithmetic_expansion_value(&expression) {
                            let value = value.to_string();
                            if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                                output
                                    .push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                            } else {
                                output.push_str(&value);
                            }
                        } else {
                            // GNU subst.c: `$[` is unambiguous arithmetic
                            // (no `$(` comsub fallback like `$((`). An eval
                            // error is a word-expansion failure — expr.c
                            // evalerror DISCARDs the command, so the echo
                            // never runs (errors.tests line 286).
                            if self
                                .shell_state
                                .arithmetic_last_error_category
                                .take()
                                .is_some()
                            {
                                self.shell_state.arithmetic_fatal_error.set(true);
                            }
                            if !self.shell_state.arithmetic_expansion_error.replace(true) {
                                // GNU evalexp reports against the
                                // post-expansion string (expand_arith_string
                                // ran first).
                                let eval_input = self.arithmetic_last_eval_input.borrow().clone();
                                let display = if eval_input.is_empty() {
                                    expression.as_str()
                                } else {
                                    eval_input.as_str()
                                };
                                if let Some(message) =
                                    crate::executor::arithmetic::arithmetic_error_message(
                                        display,
                                        true,
                                        &self.shell_state.env_vars,
                                    )
                                {
                                    eprintln!("{}{}", self.diagnostic_prefix(), message);
                                }
                            }
                        }
                    } else {
                        output.push_str("$[");
                        output.push_str(&expression);
                    }
                }
                Some(first) if first.is_ascii_digit() => {
                    chars.next();
                    let index = first.to_digit(10).unwrap_or(0) as usize;
                    if index == 0 {
                        let value = self.script_name_value();
                        if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                            output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                        } else {
                            output.push_str(&value);
                        }
                    } else {
                        let value = self
                            .shell_state
                            .positional_params
                            .get(index - 1)
                            .map(String::as_str)
                            .unwrap_or("");
                        if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                            output.push_str(&mark_expansion_whitespace(value, preserve_quotes));
                        } else {
                            output.push_str(value);
                        }
                    }
                }
                Some(first) if is_shell_name_start(first) => {
                    let mut name = String::new();
                    while let Some(name_ch) = chars.peek().copied() {
                        if !is_shell_name_char(name_ch) {
                            break;
                        }
                        chars.next();
                        name.push(name_ch);
                    }
                    if let Some(value) =
                        self.expand_with_parameter_env(saved_parameter_state, |executor| {
                            executor.dynamic_parameter_value(&name).or_else(|| {
                                executor
                                    .shell_variable_value(&name)
                                    .or_else(|| std::env::var(&name).ok())
                            })
                        })
                    {
                        let value = shell_safe_value(&value);
                        if expansion_ws_marked(alternate, preserve_quotes, in_double) {
                            output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                        } else {
                            output.push_str(&value);
                        }
                    }
                }
                // GNU subst.c: `$'...'` is a self-contained ANSI-C quoted
                // string. Consuming only the `$` and the opening quote would
                // leave the closing quote for the single-quoted span pass
                // below, which then swallows the rest of the word (unicode1.sub
                // array elements: `A=([k]=$'\001')` stored `$'\001`).
                // Decode the span here, the way the lexer does for whole words.
                //
                // Heredoc exception: GNU does NOT expand `$'...'` in
                // here-documents (subst.c heredoc_expand calls
                // expand_string_to_string with Q_HERE_DOCUMENT, and
                // param_expand has no case for `'` — `$'` falls through to
                // the default variable-name lookup, which rejects `'` and
                // outputs `$` literally, then `'` is a literal character).
                // nquote5.sub: `od -c <<EOF` with body `a$'\01'b` must keep
                // the literal text `a$'\01'b`, not decode to `a\001b`.
                Some('\'') if heredoc => {
                    chars.next();
                    output.push('$');
                    output.push('\'');
                    // Copy the ANSI-C quoted body verbatim (no decode).
                    let mut escaped = false;
                    for quoted_ch in chars.by_ref() {
                        if escaped {
                            output.push('\\');
                            output.push(quoted_ch);
                            escaped = false;
                            continue;
                        }
                        if quoted_ch == '\\' {
                            escaped = true;
                            continue;
                        }
                        if quoted_ch == '\'' {
                            output.push('\'');
                            break;
                        }
                        output.push(quoted_ch);
                    }
                    if escaped {
                        output.push('\\');
                    }
                }
                Some('\'') => {
                    chars.next();
                    if in_double {
                        // GNU parse.y:5546-5566 read_token_word dispatches
                        // $'...' to ansiexpand only at unquoted token level;
                        // inside a "..." region $ is not an expansion
                        // introducer before ' and the ' is data. Emit the $
                        // self-escaped (\$ = literal $ for the later
                        // unescape pass) and the ' as the ANSI-C data-quote
                        // marker (CTLESC-analog), then keep processing the
                        // region normally so `"$'a$y'"` still expands $y
                        // (issue #109 class).
                        output.push_str("\\$");
                        output.push(crate::lexer::ANSI_C_QUOTE_MARKER);
                    } else {
                        let mut quoted = String::new();
                        let mut escaped = false;
                        let mut closed = false;
                        for quoted_ch in chars.by_ref() {
                            if escaped {
                                quoted.push('\\');
                                quoted.push(quoted_ch);
                                escaped = false;
                                continue;
                            }
                            if quoted_ch == '\\' {
                                escaped = true;
                                continue;
                            }
                            if quoted_ch == '\'' {
                                closed = true;
                                break;
                            }
                            quoted.push(quoted_ch);
                        }
                        if escaped {
                            quoted.push('\\');
                        }
                        if closed {
                            let decoded = crate::lexer::decode_ansi_c_quoted(&quoted);
                            if decoded.is_empty() {
                                // GNU parse.y:5566 wraps the ansiexpand result in
                                // sh_single_quote, so $'' stays a QUOTED empty
                                // word: `x=($'')` stores an empty element and
                                // `echo a$''b` still yields `ab` after quote
                                // removal. Emit a quoted-empty token so the word
                                // is not dropped from the word list (issue #109
                                // class: decoded $'...' array elements).
                                output.push_str("\"\"");
                            } else if alternate {
                                for ch in decoded.chars() {
                                    if matches!(ch, ' ' | '\t' | '\n') {
                                        output.push(crate::executor::markers::IFS_GLUE);
                                    }
                                    output.push(ch);
                                }
                            } else if decoded
                                .chars()
                                // GNU subst.c keeps a quoted span one word even when
                                // it decodes to whitespace, so re-quote the decoded
                                // value when it would otherwise be field-split
                                // (A=( $'n\nl' ) is one element, not `n` and `l`).
                                // ASCII whitespace here, not char::is_whitespace:
                                // StorageWordIter splits on is_ascii_whitespace,
                                // which includes form feed (unicode1.sub
                                // [0x000c]=$'\f' stored an empty element), while
                                // char::is_whitespace would additionally hide
                                // non-ASCII space separators we must not quote.
                                .any(|ch| ch.is_ascii_whitespace() || ch == '\x0b')
                            {
                                // When the expansion context is already
                                // double-quoted (e.g. `"${var:-$'\t'}"`), the
                                // outer quotes already protect the decoded value
                                // from field splitting. Use the \x1c whitespace
                                // sentinel instead of wrapping in synthetic
                                // double quotes, which would leak literal `"`
                                // into the output (nquote.tests: `"${mytab:-$'\t'}"`
                                // must yield a bare tab, not `"^I"`).
                                if matches!(context, SubstitutionQuoteContext::DoubleQuoted) {
                                    for ch in decoded.chars() {
                                        if matches!(ch, ' ' | '\t' | '\n') {
                                            output.push(crate::executor::markers::IFS_GLUE);
                                        }
                                        output.push(ch);
                                    }
                                } else {
                                    output.push('"');
                                    for ch in decoded.chars() {
                                        match ch {
                                            '\\' => output.push_str("\\\\"),
                                            '"' => output.push_str("\\\""),
                                            '$' => output.push_str("\\$"),
                                            '`' => output.push_str("\\`"),
                                            _ => output.push(ch),
                                        }
                                    }
                                    output.push('"');
                                }
                            } else {
                                // Tag decoded quotes with E010/E011 markers so
                                // downstream quote removal (remove_shell_quotes in
                                // append_array_value) treats them as data, not
                                // syntax operators. Without this, `$'a"b'` decodes
                                // to `a"b` and the bare `"` is stripped when stored
                                // in an array (issue #109).
                                output.push_str(&crate::lexer::escape_decoded_ansi_c_quotes(
                                    &decoded,
                                ));
                            }
                        } else {
                            output.push('$');
                            output.push('\'');
                            output.push_str(&quoted);
                        }
                    }
                }
                Some(other) => {
                    chars.next();
                    output.push('$');
                    output.push(other);
                }
                None => output.push('$'),
            }
            // GNU arrayfunc.c:1353 array_expand_index -> evalexp evaluates
            // subscript arithmetic against the live environment, so a write
            // (`${a[$((i++))]}`) is visible to the very next fragment of the
            // same word. The `&self` subscript evaluators queue theirs
            // through PENDING_SUBSCRIPT_WRITES; flush them here so the
            // left-to-right order holds inside this mutable walk.
            self.apply_pending_subscript_writes();
        }

        output
    }

    fn expand_with_parameter_env<T>(
        &mut self,
        saved_parameter_state: Option<&(std::collections::HashMap<String, String>, Vec<i32>)>,
        expand: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let Some((saved_parameter_env, saved_parameter_pipestatus)) = saved_parameter_state else {
            return expand(self);
        };

        let current_env =
            std::mem::replace(&mut self.shell_state.env_vars, saved_parameter_env.clone());
        let current_pipestatus = std::mem::replace(
            &mut self.shell_state.pipestatus,
            saved_parameter_pipestatus.clone(),
        );
        let expanded = expand(self);
        self.shell_state.env_vars = current_env;
        self.shell_state.pipestatus = current_pipestatus;
        expanded
    }

    pub(in crate::executor) fn expand_current_shell_braced_substitution(
        &mut self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> Option<String> {
        let pipe_output = chars.peek().copied() == Some('|');
        if pipe_output {
            chars.next();
        } else if !chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            return None;
        }

        let mut depth = 1usize;
        let mut source = String::new();
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        let mut closed = false;
        for source_ch in chars.by_ref() {
            if escaped {
                source.push(source_ch);
                escaped = false;
                continue;
            }
            if source_ch == '\\' && !single {
                source.push(source_ch);
                escaped = true;
                continue;
            }
            match source_ch {
                '\'' if !double => {
                    single = !single;
                    source.push(source_ch);
                }
                '"' if !single => {
                    double = !double;
                    source.push(source_ch);
                }
                '{' if !single && !double => {
                    depth += 1;
                    source.push(source_ch);
                }
                '}' if !single && !double => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                _ => source.push(source_ch),
            }
        }

        if closed {
            Some(protect_command_substitution_output(
                &self.expand_current_shell_command_substitution(&source, pipe_output),
            ))
        } else {
            let mut literal = if pipe_output {
                "${|".to_string()
            } else {
                "${".to_string()
            };
            literal.push_str(&source);
            Some(literal)
        }
    }

    fn expand_current_shell_command_substitution(
        &mut self,
        source: &str,
        pipe_output: bool,
    ) -> String {
        // The body text was cut out of the current word, so its tokens would
        // restart at line 1. GNU parse.y keeps the in-place line counter for
        // command substitutions — diagnostics inside the body report the
        // original script line of the substitution (comsub2.tests: line 68).
        let body_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let source = &self.comsub_body_alias_splice_extracted(source);
        let tokens = crate::lexer::tokenize_comsub_body(
            source,
            self.posix_mode_enabled(),
            body_start_line,
            true,
        );
        let ast = crate::parser::parse(&tokens);

        // GNU subst.c function_substitute: a funsub `${ ...; }` redirects the
        // body's stdout to the anonymous capture file (its expansion value);
        // a valsub `${| ...; }` does NOT redirect stdout — the body writes to
        // the caller's real stdout — and instead makes REPLY a fresh local of
        // the body frame (subst.c:7054 make_local_variable +
        // uw_unbind_localvar), whose final value is the expansion value.
        // GNU subst.c:7020-7030 function_substitute: unless inherit_errexit
        // is set (POSIX mode enables it), the nofork comsub clears the -e
        // flag itself for the body — `set -e; ${ false; echo x; }` still
        // prints x (comsub22.sub), and an explicit `set -e` inside the body
        // re-enables it. The flag lives in env_vars, so save/restore around
        // the body like uw_restore_errexit does.
        let inherit_errexit = self.posix_mode_enabled()
            || crate::builtins::shopt::option_enabled(
                &self.shell_state.env_vars,
                "inherit_errexit",
            );
        let saved_errexit_flag = self.shell_state.env_vars.get("__RUBASH_ERREXIT").cloned();
        let saved_errexit_opt =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "errexit");
        if !inherit_errexit {
            self.shell_state.env_vars.remove("__RUBASH_ERREXIT");
            crate::builtins::set::set_shell_option(
                &mut self.shell_state.env_vars,
                "errexit",
                false,
            );
        }

        // The body's alias expansion already ran at stream level in
        // comsub_body_alias_splice (parse.y alias_expand_token on the fresh
        // input); executor-level expansion must not fire a second time.
        let saved_alias_streamed = self.mark_alias_streamed();
        let (captured, body_reply, result);
        if pipe_output {
            self.shell_state.local_var_scopes.push(HashMap::new());
            self.shell_state.local_attr_scopes.push(HashMap::new());
            self.shell_state.local_typed_scopes.push(HashMap::new());
            if let Some(scope) = self.shell_state.local_var_scopes.last_mut() {
                scope.insert(
                    "REPLY".to_string(),
                    self.shell_state.env_vars.get("REPLY").cloned(),
                );
            }
            if let Some(typed) = self.shell_state.local_typed_scopes.last_mut() {
                typed.insert(
                    "REPLY".to_string(),
                    self.shell_state.variables.get("REPLY").cloned(),
                );
            }
            // Fresh local: the body sees REPLY unset; restore_function_locals
            // brings the caller's value (or unset) back afterwards.
            self.shell_state.env_vars.remove("REPLY");
            self.shell_state.variables.remove("REPLY");
            self.shell_state.function_depth += 1;
            let r = self.execute_ast(&ast);
            self.shell_state.function_depth -= 1;
            body_reply = self.shell_state.env_vars.get("REPLY").cloned();
            self.restore_function_locals();
            captured = Vec::new();
            result = r;
        } else {
            let saved_capture = self.stdout_capture.take();
            self.stdout_capture = Some(Vec::new());
            // Direct-stdout builtins inside the body consult the thread-local
            // capture, which belongs to an enclosing pipeline stage when this
            // substitution runs inside one; give the body its own capture.
            let (thread_captured, r) = crate::executor::shell_options::capture_stdout(|| {
                self.execute_current_shell_body(&ast)
            });
            let mut cap = self.stdout_capture.take().unwrap_or_default();
            cap.extend_from_slice(&thread_captured);
            self.stdout_capture = saved_capture;
            captured = cap;
            body_reply = None;
            result = r;
        }
        self.resume_alias_streamed(saved_alias_streamed);

        if !inherit_errexit {
            match saved_errexit_flag {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_ERREXIT".to_string(), value);
                }
                None => {
                    self.shell_state.env_vars.remove("__RUBASH_ERREXIT");
                }
            }
            crate::builtins::set::set_shell_option(
                &mut self.shell_state.env_vars,
                "errexit",
                saved_errexit_opt,
            );
        }

        // `exit N` inside the body aborts the enclosing (sub)shell with N
        // (comsub26.sub line 32: the subshell never prints and $? = 42).
        if let Err(crate::executor::ExecuteError::ExitCode(code)) = &result {
            self.current_shell_substitution_exit.set(Some(*code));
        }
        let status = command_substitution_status(result, self.exit_code);

        // The body's own exit status is $? for expansions later on the same
        // command line (comsub26.sub line 35: `echo ${ ...; return 42; } $?`
        // prints `var=inside 42`); the finished command then overwrites $?
        // with its own status as usual.
        self.exit_code = status;
        self.last_command_substitution_status.set(Some(status));

        if pipe_output {
            body_reply.unwrap_or_default()
        } else {
            bytes_to_shell_text(&captured)
                .trim_capture_terminator()
                .to_string()
        }
    }

    /// Bash 5.3 nofork command substitution (subst.c): the `${ command; }` /
    /// `${| command; }` body runs in the current shell but with a
    /// function-like variable frame — `local` scopes to the body and `return`
    /// ends only the body, while plain assignments still mutate the current
    /// environment (comsub2.tests: `outside: 42` vs `outside:` empty).
    fn execute_current_shell_body(&mut self, ast: &crate::parser::Ast) -> Result<(), ExecuteError> {
        self.shell_state.local_var_scopes.push(HashMap::new());
        self.shell_state.local_attr_scopes.push(HashMap::new());
        self.shell_state.local_typed_scopes.push(HashMap::new());
        self.shell_state.function_depth += 1;
        let result = self.execute_ast(ast);
        self.shell_state.function_depth -= 1;
        self.restore_function_locals();
        result
    }

    pub(in crate::executor) fn expand_command_substitution_mut_typed_with_context(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> SubstitutionOutput {
        // GNU command_substitute (subst.c:7143) runs the body in a subshell:
        // arithmetic subscript writes queued inside never reach the parent
        // environment. Scope the deferred-write queue to this substitution.
        let _subscript_writes_guard =
            crate::executor::expand_braced_indices::PendingSubscriptWritesGuard::new();
        // GNU make_cmd.c:602-611: a heredoc inside a command substitution
        // where the `)` closes on the delimiter line (e.g. `EOF)`) is
        // "delimited by end-of-file" and gets a warning. The heredoc path
        // needs the untrimmed source to detect this (trailing newline
        // distinguishes `EOF)` from `EOF\n)`). Only trim for the non-heredoc
        // path. Also count leading newlines to adjust the comsub start line
        // for heredoc warning line numbers (GNU reports the line of the
        // `cat` command, not the line of the outer `$(`).
        let has_heredoc = source.contains("<<");
        let leading_newlines = source.chars().take_while(|ch| *ch == '\n').count();
        let source = if has_heredoc {
            source.trim()
        } else {
            source.trim()
        };
        // GNU applies alias expansion while reading the substitution body
        // (parse.y alias_expand_token + push_string): expand once at stream
        // level for this body's own word scan; downstream real-parser paths
        // receive the raw source and splice for themselves at their own
        // parse boundary.
        let words = split_shell_words(&self.comsub_body_alias_splice_extracted(source));
        // Store leading newlines for the heredoc path to adjust warning
        // line numbers: when `$(` is at end of line, the comsub body starts
        // on the next line, and the `cat` command line is
        // current_line + leading_newlines.
        self.comsub_leading_newlines.set(leading_newlines);
        if let Some(output) = self.command_substitution_heredoc_output_mut_typed(source, context) {
            return output;
        }
        let saved_positional_params = self.shell_state.positional_params.clone();
        // GNU subst.c:7143 command_substitute feeds the body to
        // parse_and_execute, so the real parser owns each word's quoting:
        // `"$x"` stays one field, `""` stays one empty argument, and `"*"`
        // never reaches pathname expansion. The function-call shortcut
        // re-splits the body with split_shell_words, which erases that
        // quote state — the stored word `$x` then looks unquoted and
        // field-splits (issue #116). Keep the shortcut only for bodies the
        // text split provably cannot misread: whitespace-separated plain
        // words with no quoting, expansion, escape, redirection, or
        // command-syntax characters. Everything else falls through to the
        // real parser/executor paths below.
        if command_substitution_function_call_is_trivial(source, &words) {
            if let Some(output) = self.run_function_command_substitution(&words) {
                self.set_positional_params(saved_positional_params);
                let status = self.last_command_substitution_status.get().unwrap_or(0);
                // Shell text -> raw capture bytes: decode marker pairs once
                // so readback/assignment_text encode exactly once.
                return SubstitutionOutput::readback(
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
                    status,
                    context,
                );
            }
        }
        self.set_positional_params(saved_positional_params);
        if command_substitution_words_contain_here_string(&words) {
            // The words were already stream-expanded above; pass the raw
            // source so run_ast's own splice performs the single expansion
            // (re-expanding joined words would expand the alias a second
            // time — parse.y never re-reads pushed text twice).
            if let Some(output) = self.run_ast_command_substitution_with_context(source, context) {
                return output;
            }
        }
        if command_substitution_uses_specialized_path(self, source, &words) {
            let output = self.expand_command_substitution_with_context(source, context);
            let status = self.last_command_substitution_status.get().unwrap_or(0);
            return SubstitutionOutput::readback(
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
                status,
                context,
            );
        }
        // A command list (`echo a; echo b`, `a && b`) must run as an AST, not
        // through the single-command specialized dispatch below: routing
        // `echo mn; echo op` to the echo shortcut treats `;` as an argument
        // and yields `mn; echo op` instead of `mn\nop` (comsub.tests
        // `ab$(echo mn; echo op)yz`). A quoted `a;b` argument is harmless to
        // route here too: the AST still prints it correctly. Newlines in the
        // raw source are command separators the same way (old-style
        // backticks span lines: `echo ab\ncd` runs two commands).
        if words
            .iter()
            .any(|word| word.contains(';') || matches!(word.as_str(), "&&" | "||"))
            || source.contains('\n')
        {
            if let Some(output) = self.run_ast_command_substitution_with_context(source, context) {
                return output;
            }
        }
        // Simple builtins that the non-mut special-case dispatch handles with
        // proper quote stripping (echo/printf/cat/...). Prefer that path so
        // nested `"$(...)"` arguments do not leak quote characters through
        // the full-AST execution path.
        if is_specialized_command_substitution_word(&words) {
            let output = self.expand_command_substitution_with_context(source, context);
            let status = self.last_command_substitution_status.get().unwrap_or(0);
            return SubstitutionOutput::readback(
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
                status,
                context,
            );
        }
        if let Some(output) = self.run_ast_command_substitution_with_context(source, context) {
            return output;
        }
        let output = self.expand_command_substitution_with_context(source, context);
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
            status,
            context,
        )
    }

    pub(in crate::executor) fn expand_command_substitution_mut_with_context(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        self.expand_command_substitution_mut_typed_with_context(source, context)
            .text_lossy()
    }

    pub(in crate::executor) fn run_ast_command_substitution_with_context(
        &mut self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<SubstitutionOutput> {
        if command_substitution_contains_heredoc(source) {
            return None;
        }

        // Keep GNU's in-place line counter: the body was extracted from the
        // current word, so body diagnostics must report the original script
        // line instead of restarting at 1.
        let body_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let source = &self.comsub_body_alias_splice_extracted(source);
        let tokens = crate::lexer::tokenize_comsub_body(
            source,
            self.posix_mode_enabled(),
            body_start_line,
            true,
        );
        let ast = crate::parser::parse(&tokens);
        if !command_substitution_needs_ast_execution(&ast) {
            return None;
        }

        // subst.c:7143 command_substitute / execute_cmd.c:1576
        // execute_in_subshell: the body runs in a forked child, so NO shell
        // state — variables, functions, aliases, positional params, the job
        // registry (`$( sleep 5 & )` jobs stay invisible to the parent),
        // coproc names, scopes, RNG, or expansion-error latches — may leak
        // back. The typed ShellState clone is the whole boundary; process
        // resources (cwd, OS env, exit code, captures) are handled
        // separately below.
        let saved_state = self.shell_state.clone_for_child_save();
        let saved_exit_code = self.exit_code;
        let saved_dir = env::current_dir().ok();
        // The body is fresh parser input whose alias expansion GNU applies
        // at its read (subst.c:7143 parse_and_execute); it already ran at
        // stream level in comsub_body_alias_splice above, so mark the inner
        // execution streamed — saved_state rolls the marker back below.
        self.mark_alias_streamed();
        self.shell_state
            .subshell_depth
            .set(saved_state.subshell_depth.get() + 1);
        self.shell_state.in_command_substitution.set(true);

        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        // Bash runs command substitution in a subshell where errexit is
        // suppressed: `$(false; echo ok)` prints ok because the inner `false`
        // does not abort the substitution (set-e.tests "command subst should
        // not inherit -e"); only the substitution's final status (echo's 0)
        // propagates to the outer assignment, which then checks -e.
        // POSIX mode is the exception: `set -o posix; z=$(false;echo posix)`
        // exits (set-e1.sub), so keep errexit active there.
        let posix_mode = self
            .shell_state
            .env_vars
            .get("__RUBASH_POSIX_MODE")
            .map(String::as_str)
            == Some("1");
        let inherit_errexit =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "inherit_errexit");
        // Direct-stdout builtins inside the body consult the thread-local
        // capture, which belongs to an enclosing pipeline stage when this
        // substitution runs inside one; give the body its own capture.
        let (thread_captured, result) = crate::executor::shell_options::capture_stdout(|| {
            if posix_mode || inherit_errexit {
                self.execute_ast(&ast)
            } else {
                self.with_errexit_suppressed(|executor| executor.execute_ast(&ast))
            }
        });
        let mut output = self.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        self.stdout_capture = saved_capture;

        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => {
                status
            }
            Err(_) => 1,
        };

        // GNU parse.y: a syntax error inside the substitution body is a
        // read-time failure of the ENCLOSING command — after this command
        // finishes, the reader stops (`$( esac ; ...)` in a case pattern:
        // the `*)` arm still prints `ok 2`, `echo we should not see this`
        // never runs). The body ran in place, so its parse_error latch is
        // already ours — propagate it to the abort flag the ast loop checks.
        if self.parse_error_occurred {
            self.last_command_substitution_parse_error.set(true);
        }

        self.restore_flat_subshell(saved_state, saved_dir);
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(SubstitutionOutput::readback(output, status, context))
    }

    pub(in crate::executor) fn run_function_command_substitution(
        &mut self,
        words: &[String],
    ) -> Option<String> {
        let name = words.first()?;
        if !self.shell_state.functions.contains_key(name) {
            return None;
        }
        // A function call is only a shortcut when the substitution body is a
        // single simple command. GNU subst.c parses the body into a command
        // list first: `$(f a b | wc -l)` must pipe f's output through wc,
        // not run f with `| wc -l` in its positional params (issue #70).
        if command_substitution_words_have_operators(words) {
            return None;
        }

        let args = words[1..]
            .iter()
            .flat_map(|word| self.expand_command_substitution_arg_values(word))
            .collect::<Vec<_>>();
        let mut call = CommandNode::new();
        call.words = words.to_vec();

        // `$(f)` runs the function inside the substitution's subshell — a
        // forked child in GNU — so variable/scope/job mutations of the call
        // die with the substitution. Whole-state clone, not a field list.
        let saved_state = self.shell_state.clone_for_child_save();
        let saved_dir = env::current_dir().ok();
        let saved_exit_code = self.exit_code;
        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        // GNU subst.c:7306-7313 command_substitute: the substitution child's
        // stdout is the capture pipe, never the caller's fd 1 — dup2
        // (fildes[1], 1) replaces whatever binding the parent carried. This
        // shortcut runs the function on the caller's own executor, so the
        // same replacement must be bracketed around the call (identical to
        // the fd-1 rebind in command_list_substitution_output_typed,
        // niubash shell-quirks Q16). Without it, an enclosing `exec > f` /
        // `source f > f` fd-1 file binding routes the function's stdout to
        // the outer file while the capture reads an empty pipe (rubash#161:
        // bash-it alias reload empty, nvm `nvm ls` missing default aliases).
        // fd 2 stays inherited — `$()` does not capture stderr (GNU
        // subst.c:7149).
        let saved_fd1 = self.fd_table.entries.insert(
            1,
            crate::executor::fd_table::FdEntry {
                read: None,
                write: Some(FdWriteEndpoint::Stdout),
                closed: false,
                dynamic: false,
            },
        );
        // Direct-stdout builtins inside the function consult the thread-local
        // capture, which belongs to an enclosing pipeline stage when this
        // substitution runs inside one; give the call its own capture.
        let (thread_captured, result) = crate::executor::shell_options::capture_stdout(|| {
            self.execute_function(name, &args, &call)
        });
        match saved_fd1 {
            Some(entry) => {
                self.fd_table.entries.insert(1, entry);
            }
            None => {
                self.fd_table.entries.remove(&1);
            }
        }
        let mut output = self.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        self.stdout_capture = saved_capture;
        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => {
                status
            }
            Err(_) => 1,
        };
        self.restore_flat_subshell(saved_state, saved_dir);
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(
            bytes_to_shell_text(&output)
                .trim_capture_terminator()
                .to_string(),
        )
    }
}

fn command_substitution_needs_ast_execution(ast: &Ast) -> bool {
    ast.commands.iter().any(command_has_ast_substitution_shape)
        || ast
            .commands
            .iter()
            .any(command_contains_current_shell_substitution)
        || (ast.commands.len() > 1 && ast.commands.iter().all(command_is_ast_list_substitution))
}

// GNU subst.c treats a syntactically command-like $((...)) body as a
// command substitution fallback, even though arithmetic parsing rejects it.
// Keep ordinary arithmetic diagnostics (notably 1/0 and invalid octal 08).
fn embedded_command_substitution_expression(expression: &str) -> bool {
    expression.contains(';') && expression.contains('(')
}

fn collect_dollar_paren_arithmetic_expansion(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> (String, bool) {
    // GNU parse.y parse_comsub -> parse_matched_pair(P_ARITH): after `$((`
    // the scan is plain paren nesting, not an adjacent-`))` search — the
    // second `(` of `$((` is one open, so depth starts at 2 (the `$(` plus
    // that paren) and the word ends when it returns to 0. Quotes and
    // backslash escapes keep their `)`s out of the count.
    let mut expression = String::new();
    let mut paren_depth: usize = 2;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            expression.push(ch);
            continue;
        }
        if single {
            expression.push(ch);
            if ch == '\'' {
                single = false;
            }
            continue;
        }
        if double {
            expression.push(ch);
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                double = false;
            }
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                expression.push(ch);
            }
            '\'' => {
                single = true;
                expression.push(ch);
            }
            '"' => {
                double = true;
                expression.push(ch);
            }
            '(' => {
                paren_depth += 1;
                expression.push(ch);
            }
            ')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    expression.push(ch);
                    return (expression, true);
                }
                expression.push(ch);
            }
            _ => expression.push(ch),
        }
    }

    (expression, false)
}

/// GNU subst.c:9727 chk_arithsub — paren-balance check on the inside of
/// `$(( ... ))`: a stray `)` means the construct is really a nested command
/// substitution (`$(( echo ab cde ) )`), not arithmetic. Quotes and
/// backslash escapes are skipped exactly like the C version.
fn arith_sub_parens_balanced(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut count = 0i32;
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\\' => {
                index += 2;
                continue;
            }
            '\'' | '"' => {
                let quote = chars[index];
                index += 1;
                while index < chars.len() {
                    if quote == '"' && chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == quote {
                        break;
                    }
                    index += 1;
                }
                index += 1;
                continue;
            }
            '(' => count += 1,
            ')' => {
                count -= 1;
                if count < 0 {
                    return false;
                }
            }
            _ => {}
        }
        index += 1;
    }
    count == 0
}

fn collect_dollar_bracket_arithmetic_expansion(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> (String, bool) {
    let mut expression = String::new();
    let mut bracket_depth: usize = 0;

    for ch in chars.by_ref() {
        match ch {
            '[' => {
                bracket_depth += 1;
                expression.push(ch);
            }
            ']' if bracket_depth == 0 => return (expression, true),
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                expression.push(ch);
            }
            _ => expression.push(ch),
        }
    }

    (expression, false)
}

pub(in crate::executor) fn collect_command_substitution_source(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    aliases: &std::collections::HashMap<String, crate::builtins::alias::Alias>,
) -> String {
    collect_command_substitution_source_ex(chars, aliases).0
}

/// Returns the collected source plus whether the closing `)` was found.
/// GNU parse.y parse_comsub reports `unexpected EOF` when the substitution
/// runs past the input; callers that only need the span keep the plain
/// variant.
pub(in crate::executor) fn collect_command_substitution_source_ex(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    aliases: &std::collections::HashMap<String, crate::builtins::alias::Alias>,
) -> (String, bool) {
    let mut depth = 1usize;
    let mut closed = false;
    let mut source = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    let mut parameter_depth = 0usize;
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start of the body. `word.is_empty()` alone is wrong: `$`, quotes,
    // escapes and other non-alphanumeric word characters never reach `word`,
    // so `$(echo $#)` and `$(echo 'a'#b)` would misread `#` as a comment.
    let mut token_boundary = true;

    while let Some(source_ch) = chars.next() {
        if escaped {
            source.push(source_ch);
            escaped = false;
            token_boundary = false;
            // A backslash-quoted character is word text (placeholder:
            // `c\ase` is not `case`), so a following `#` stays mid-word
            // (`\;#` in comsub1.sub).
            word.push('\u{1}');
            continue;
        }
        if source_ch == '\\' && !single {
            source.push(source_ch);
            escaped = true;
            token_boundary = false;
            continue;
        }
        // `${` opens parameter text: inside it `#` is a parameter operator
        // (e.g. `${#x}`), never a comment introducer.
        if source_ch == '$' && !single && chars.peek().copied() == Some('{') {
            source.push(source_ch);
            source.push(chars.next().expect("parameter brace"));
            parameter_depth += 1;
            token_boundary = false;
            continue;
        }
        if source_ch == '}' && parameter_depth > 0 {
            source.push(source_ch);
            parameter_depth -= 1;
            token_boundary = false;
            continue;
        }
        if source_ch == '#' && !single && !double && token_boundary && parameter_depth == 0 {
            source.push(source_ch);
            while let Some(comment_ch) = chars.peek().copied() {
                if comment_ch == '\n' {
                    break;
                }
                source.push(comment_ch);
                chars.next();
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            token_boundary = true;
            continue;
        }
        if source_ch == '`' && !single {
            source.push(source_ch);
            token_boundary = false;
            let mut backtick_escaped = false;
            for backtick_ch in chars.by_ref() {
                source.push(backtick_ch);
                if backtick_escaped {
                    backtick_escaped = false;
                    continue;
                }
                if backtick_ch == '\\' {
                    backtick_escaped = true;
                    continue;
                }
                if backtick_ch == '`' {
                    break;
                }
            }
            continue;
        }
        if source_ch == '<' && !single && !double && chars.peek().copied() == Some('<') {
            let mut lookahead = chars.clone();
            lookahead.next();
            if lookahead.peek().copied() != Some('<') {
                source.push(source_ch);
                source.push(chars.next().expect("heredoc second less-than"));
                let mut header = String::new();
                // Delimiter text with quoting resolved the way the parser
                // reports it (make_cmd.c heredoc delimiter): quote bytes are
                // removed and a backslash contributes its escaped character.
                // `<<\)` names delimiter `)`; leaving the backslash in made
                // the body comparison impossible (comsub-eof5.sub `cat <<\)`).
                let mut delimiter = String::new();
                let mut header_single = false;
                let mut header_double = false;
                while let Some(&header_ch) = chars.peek() {
                    match header_ch {
                        '\'' if !header_double => header_single = !header_single,
                        '"' if !header_single => header_double = !header_double,
                        // An unquoted `)` ends the `<<word` header -- GNU's
                        // parser tokenizes it as the substitution closer
                        // (PST_EOFTOKEN), not as delimiter text.
                        ')' if !header_single && !header_double => break,
                        '\\' if !header_single => {
                            chars.next();
                            source.push(header_ch);
                            header.push(header_ch);
                            if let Some(&escaped) = chars.peek() {
                                chars.next();
                                source.push(escaped);
                                header.push(escaped);
                                if escaped != '\n' {
                                    delimiter.push(escaped);
                                }
                            }
                            continue;
                        }
                        _ => delimiter.push(header_ch),
                    }
                    chars.next();
                    source.push(header_ch);
                    if header_ch == '\n' {
                        break;
                    }
                    header.push(header_ch);
                }
                let strip_tabs = header.trim_start().starts_with('-');
                let delimiter = delimiter.trim().trim_start_matches('-').trim().to_string();
                if !delimiter.is_empty() {
                    let mut body_line = String::new();
                    while let Some(body_ch) = chars.next() {
                        // GNU make_cmd.c:605-611 (PST_EOFTOKEN): a heredoc
                        // body line that starts with the delimiter and
                        // carries the eof token `)` later on ends the
                        // document as if it hit EOF; the `)` is pushed back
                        // to the parser input, where it closes the command
                        // substitution. A `)` inside an ordinary body line
                        // is data (`x=$(cat <<EOF` + `this paren ) ...`),
                        // while `EOF)` both ends the document and closes
                        // the substitution. Text between the delimiter and
                        // the `)` stays in the collected source the way
                        // GNU's ungets feeds it back to the comsub parser.
                        if body_ch == ')'
                            && (if strip_tabs {
                                body_line.trim_start_matches('\t')
                            } else {
                                body_line.as_str()
                            })
                            .starts_with(delimiter.as_str())
                        {
                            closed = true;
                            break;
                        }
                        source.push(body_ch);
                        if body_ch == '\n' {
                            // `<<-` strips leading tabs on the terminator
                            // line the same way the `)` check above does.
                            let terminator = if strip_tabs {
                                body_line.trim_start_matches('\t')
                            } else {
                                body_line.as_str()
                            };
                            if terminator.trim_end() == delimiter {
                                break;
                            }
                            body_line.clear();
                        } else {
                            body_line.push(body_ch);
                        }
                    }
                    if closed {
                        break;
                    }
                }
                // A heredoc terminator ends on its own line, so the next
                // character begins a fresh token.
                token_boundary = true;
                continue;
            }
        }

        let rest = chars.clone().collect::<String>();
        // GNU parse.y alias_expand_token: inside a command substitution
        // body, alias expansion happens while parsing, so an alias for
        // `case` (e.g. `alias switch=case`) is recognized as the `case`
        // reserved word.  collect_command_substitution_source must track
        // case_depth through such aliases so a `)` in a case pattern does
        // not prematurely close the substitution (comsub5.sub: `echo $(
        // switch foo in foo) echo ok 2;; esac )`).  Check the word before
        // update_command_substitution_case_depth clears it.
        let word_for_alias = word.clone();
        let boundary_for_alias = current_word_boundary;
        let alias_case_delta =
            if !single && !double && !word_for_alias.is_empty() && boundary_for_alias {
                if let Some(alias) = aliases.get(&word_for_alias) {
                    match alias.value.split_whitespace().next() {
                        Some("case") => Some(1i32),
                        Some("esac") => Some(-1i32),
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };
        update_command_substitution_case_depth(
            source_ch,
            single,
            double,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            &rest,
        );
        if let Some(delta) = alias_case_delta {
            if delta > 0 {
                case_depth += 1;
            } else {
                case_depth = case_depth.saturating_sub(1);
            }
        }
        match source_ch {
            '\'' if !double => {
                single = !single;
                token_boundary = false;
                source.push(source_ch);
            }
            '"' if !single => {
                double = !double;
                token_boundary = false;
                source.push(source_ch);
            }
            '(' if !single && !double && case_depth == 0 => {
                depth += 1;
                source.push(source_ch);
            }
            ')' if !single && !double && case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    closed = true;
                    break;
                }
                source.push(source_ch);
            }
            _ => source.push(source_ch),
        }
        // Quoted characters are word text handled by the quote arms; the
        // boundary only tracks characters the live tokenizer sees.
        if !single && !double {
            token_boundary = source_ch.is_whitespace()
                || matches!(source_ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        }
    }

    // GNU parse.y parse_comsub feeds the collected body to the parser
    // verbatim — a `\"` inside is an escaped-quote token the INNER lexer
    // dequotes at execution time (PST_NOEXPAND keeps the backslash in the
    // token, parse.y:5366-5375). Post-processing the body with
    // unescape_storage_command_substitution_source stripped `\"` to `"`,
    // so the inner parse saw a syntactic quote and dropped it
    // (`$(echo $((x)) | echo "\"q\"")` printed `q` instead of `"q"`).
    (source, closed)
}

fn command_substitution_status(result: Result<(), ExecuteError>, exit_code: i32) -> i32 {
    match result {
        Ok(()) => exit_code,
        Err(ExecuteError::Return(status)) => status,
        Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => status,
        Err(_) => 1,
    }
}

fn command_has_ast_substitution_shape(command: &CommandNode) -> bool {
    command.and_or_list.is_some()
        || command.inverted_command.is_some()
        || command.background_command.is_some()
        || command_has_here_string_substitution(command)
        || command_has_compound_substitution(command)
}

fn command_has_compound_substitution(command: &CommandNode) -> bool {
    command.pipeline_command.as_ref().is_some_and(|pipeline| {
        pipeline
            .stages
            .iter()
            .any(command_has_compound_substitution)
    }) || command
        .and_or_list
        .as_ref()
        .is_some_and(|list| list.commands.iter().any(command_has_compound_substitution))
        || command
            .inverted_command
            .as_ref()
            .is_some_and(|inverted| command_has_compound_substitution(&inverted.command))
        || command
            .time_command
            .as_ref()
            .is_some_and(|time| command_has_compound_substitution(&time.command))
        || command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.select_command.is_some()
        || command.case_command.is_some()
        || command.coproc_command.is_some()
        || command.subshell_command.is_some()
        || command.brace_group.is_some()
        || command.arithmetic_command.is_some()
        || command.conditional_command.is_some()
}

fn command_contains_current_shell_substitution(command: &CommandNode) -> bool {
    command
        .words
        .iter()
        .any(|word| word_contains_current_shell_command_substitution(word))
}

fn command_has_here_string_substitution(command: &CommandNode) -> bool {
    command.here_string.is_some()
        || command
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.here_string)
        || command.pipeline_command.as_ref().is_some_and(|pipeline| {
            pipeline
                .stages
                .iter()
                .any(command_has_here_string_substitution)
        })
        || command.and_or_list.as_ref().is_some_and(|list| {
            list.commands
                .iter()
                .any(command_has_here_string_substitution)
        })
        || command
            .inverted_command
            .as_ref()
            .is_some_and(|inverted| command_has_here_string_substitution(&inverted.command))
        || command
            .time_command
            .as_ref()
            .is_some_and(|time| command_has_here_string_substitution(&time.command))
}

fn command_is_ast_list_substitution(command: &CommandNode) -> bool {
    if !command_has_simple_substitution_shape(command) {
        return false;
    }
    if !command.assignments.is_empty() {
        return true;
    }
    matches!(
        command.words.first().map(String::as_str),
        Some("echo" | "printf" | "true" | "false" | ":" | "pwd")
    )
}

fn command_has_simple_substitution_shape(command: &CommandNode) -> bool {
    command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.inverted_command.is_none()
        && command.background_command.is_none()
        && command.time_command.is_none()
        && command.for_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.select_command.is_none()
        && command.case_command.is_none()
        && command.coproc_command.is_none()
        && command.subshell_command.is_none()
        && command.brace_group.is_none()
        && command.arithmetic_command.is_none()
        && command.conditional_command.is_none()
}

fn command_substitution_uses_specialized_path(
    executor: &Executor,
    source: &str,
    words: &[String],
) -> bool {
    command_substitution_contains_heredoc(source)
        || (words.iter().any(|word| word == "|")
            && !command_substitution_contains_here_string(source))
        || words.first().map(String::as_str) == Some("time")
        || executor
            .command_substitution_cd_pwd_output(source)
            .is_some()
}

fn command_substitution_words_contain_here_string(words: &[String]) -> bool {
    words
        .iter()
        .any(|word| word == "<<<" || word.ends_with("<<<"))
}

/// Whitelist admission for the word-level function-call substitution
/// shortcut. GNU subst.c:7143 command_substitute hands the body to
/// parse_and_execute unconditionally; the shortcut is equivalent only when
/// `split_shell_words` reproduces the real token stream exactly — every
/// character is plain word text or inline whitespace, so no quote is
/// stripped (a stripped quote also erases field-split/glob suppression:
/// `$(f "$x")` split `$x` into three args, issue #116), no escape produces
/// a carrier, no `$`/backtick expands, no `*`/`?`/`~`/`[...]` pattern or
/// redirection/operator character needs the lexer, and no `""` empty word
/// is dropped. `source` is the raw body; `words` are the post-alias split
/// words, checked too because alias expansion can inject characters the
/// source never contained.
fn command_substitution_function_call_is_trivial(source: &str, words: &[String]) -> bool {
    fn plain_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '_' | '-' | '.' | ',' | '/' | ':' | '=' | '+' | '%' | '@'
            )
    }
    source
        .chars()
        .all(|ch| ch == ' ' || ch == '\t' || plain_char(ch))
        && words
            .iter()
            .all(|word| !word.is_empty() && word.chars().all(plain_char))
}

/// Builtin commands whose command-substitution output is produced by the
/// non-mut special-case dispatch in `expand_command_substitution` (echo,
/// printf, cat, basename, ...). Routing these through that path keeps nested
/// `"$(...)"` argument quote handling consistent with Bash.
fn is_specialized_command_substitution_word(words: &[String]) -> bool {
    matches!(
        words.first().map(String::as_str),
        Some(
            "echo"
                | "printf"
                | "cat"
                | "basename"
                | "umask"
                | "ulimit"
                | "pwd"
                | "type"
                | "kill"
                | "trap"
                | "mktemp"
                | "set"
                | "export"
                | "true"
                | ":"
        )
    )
}

fn command_substitution_contains_here_string(source: &str) -> bool {
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
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
            '<' if !single && !double && chars.peek().copied() == Some('<') => {
                chars.next();
                if chars.peek().copied() == Some('<') {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn command_substitution_contains_heredoc(source: &str) -> bool {
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
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
            '<' if !single && !double && chars.peek().copied() == Some('<') => {
                chars.next();
                if chars.peek().copied() == Some('<') {
                    chars.next();
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }

    false
}

pub(in crate::executor) fn update_command_substitution_case_depth(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    rest: &str,
) {
    if single || double {
        word.clear();
        *word_boundary = false;
        return;
    }

    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary => {
            *case_depth += 1;
            false
        }
        "esac" if *current_word_boundary && !case_pattern_starts_with_esac_rest(ch, rest) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary =>
        {
            true
        }
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

fn case_pattern_starts_with_esac_rest(delimiter: char, rest: &str) -> bool {
    if !matches!(delimiter, ')' | '|') {
        return false;
    }

    let chars = std::iter::once(delimiter)
        .chain(rest.chars())
        .collect::<Vec<_>>();
    let mut close = 0usize;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return false,
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        return false;
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return true;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        if ch == ')' {
            return false;
        }
        if word.is_empty() {
            if command_substitution_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && command_substitution_reserved_word_allows_next(&word);
        word.clear();
        word_boundary =
            reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn command_substitution_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}
