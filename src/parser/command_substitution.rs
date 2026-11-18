use super::{parse, CommandSubstitutionNode};

/// rubash#2789: the read-time screen's extraction entry — the same
/// scan as `command_substitutions_in_word`, but the opener search
/// replays the word's lexical state (parse.y:5305 read_token_word):
/// an opener inside single quotes or behind a backslash is literal
/// text, never a substitution, while openers inside double quotes
/// stay live. The plain scan above cannot tell (it fires on any
/// `$`+`(` byte pair), which would let the read-time screen kill
/// legal units like `echo '$(a |)'` or `echo \$(a |)`. Openers that
/// ARE live are handed to the same span extractors, whose own quote
/// tracking consumes the body, so quoted spans inside a live body
/// are never re-judged by this outer state machine.
pub(super) fn command_substitutions_in_word_live(word: &str) -> Vec<CommandSubstitutionNode> {
    if !word.contains('$') && !word.contains('`') {
        return Vec::new();
    }

    let chars = word.chars().collect::<Vec<_>>();
    let mut substitutions = Vec::new();
    let mut index = 0;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match ch {
            '\\' if !single => {
                escaped = true;
                index += 1;
            }
            '\'' if !double => {
                single = !single;
                index += 1;
            }
            '"' if !single => {
                double = !double;
                index += 1;
            }
            '`' if !single => {
                if let Some((substitution, next_index)) =
                    backtick_command_substitution(&chars, index)
                {
                    substitutions.push(substitution);
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            '$' if !single
                && chars.get(index + 1) == Some(&'(')
                && chars.get(index + 2) != Some(&'(') =>
            {
                if let Some((substitution, next_index)) = dollar_command_substitution(&chars, index)
                {
                    substitutions.push(substitution);
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            '$' if !single && chars.get(index + 1) == Some(&'{') => {
                if let Some((substitution, next_index)) = braced_command_substitution(&chars, index)
                {
                    substitutions.push(substitution);
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            _ => {
                index += 1;
            }
        }
    }
    substitutions
}

pub(super) fn command_substitutions_in_word(word: &str) -> Vec<CommandSubstitutionNode> {
    // Provably-empty admission (rubash#117 whitelist discipline): every
    // production this scan can find requires the trigger byte below, so its
    // absence proves the empty answer and skips the char collect + walk.
    // GNU anchor: parse.y:5305 read_token_word reads a word once; GNU runs
    // NO per-word expansion scans at parse time at all (subst.c analyzes at
    // execution) — this port's scans are the executor-facing metadata
    // source, and the gate only skips scans that cannot match.
    if !word.contains('$') && !word.contains('`') {
        return Vec::new();
    }

    let chars = word.chars().collect::<Vec<_>>();
    let mut substitutions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$'
            && chars.get(index + 1) == Some(&'(')
            && chars.get(index + 2) != Some(&'(')
        {
            if let Some((substitution, next_index)) = dollar_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some((substitution, next_index)) = braced_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '`' {
            if let Some((substitution, next_index)) = backtick_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    substitutions
}

fn dollar_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let mut index = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    // `case WORD in' chain + PST_CASEPAT pattern-region state
    // (rubash#380): see update_command_substitution_case_depth.
    let mut case_in_stage = 0u8;
    let mut case_pattern_region = false;
    // rubash#2789: pattern-content bookkeeping for the in-field
    // unclosed-case reclassification at the `)` arm below. While the
    // pattern region is armed, a `)` that arrives with NO pattern
    // content yet seen cannot be a pattern closer (there is no
    // pattern to close), so it closes the substitution itself
    // (parse.y:4519-4521: `)` is the substitution's shell_eof_token)
    // and the truncated body (`case x in`) reaches the body parser,
    // whose grammar verdict the read-time screen reports — instead of
    // the extractor scanning off the end of the word and classifying
    // the shape as an unclosed EOF.
    let mut case_pattern_content = false;
    let mut case_pattern_paren_depth = 0usize;
    let mut case_pattern_opener_available = true;
    let mut case_pattern_region_seen = false;
    // rubash#2789: completion latch for the SPACED empty preamble
    // (`$(case x in )`). When `in` completes at a blank, the
    // reserved-word machine's stage bookkeeping collapses before
    // the pattern region can arm (its word-boundary preempt zeroes
    // the stage), so none of the close arm's three conditions can
    // see the shape. The latch remembers "an `in` word just
    // completed and nothing has been consumed since": it is set at
    // the completion char, cleared by any pattern-content char and
    // when the region disarms, and it lets the `)` that follows
    // close the substitution like its glued twin `in)`.
    let mut case_in_completed_empty = false;
    // rubash#3605: paren depth of a `$(` nested inside the case context
    // (`$(case $(w) in e) ...`). The `(`/`)` match arms below do not
    // count parens while `case_depth > 0` (pattern parens belong to the
    // case machinery), so without this tracker the `)` ending the inner
    // substitution reaches the truncated-case close arm and the outer
    // substitution ends early at `$(w` — GNU parse_comsub instead lets
    // the nested substitution's own reader consume its `)`.
    let mut nested_case_subst = 0usize;
    // rubash#3306: paren depth opened INSIDE a case context while the
    // pattern region is disarmed (clause body, or the `case … in`
    // preamble). GNU's `)` is the substitution's shell_eof_token only
    // at body paren depth zero (parse.y:4519-4521); `(b)`/`((…))`
    // groups in the body own their own `)` first, exactly like the
    // pattern-paren tracker above owns pattern parens.
    let mut case_body_paren_depth = 0usize;
    // rubash#3306: a `;;`/`;&`/`;;&` seen while the pattern region is
    // still armed is fatal in GNU's grammar — a clause terminator is
    // only legal after a clause BODY (region disarmed). Once seen the
    // case can never complete, so the next top-level `)` closes the
    // substitution for the body parser's verdict no matter what the
    // pattern bookkeeping makes of it (`$(case x in a;; echo hi)`).
    let mut case_clause_terminator_in_pattern = false;
    // rubash#3834: previous significant unquoted char fed to the case
    // word machine — the `esac` pattern-text witness (parse.y:3181/3183,
    // `last_read_token == '|' or '('`), arm-for-arm with the skip.rs /
    // embedded_mutations machines (rubash#380).
    let mut prev_sig: Option<char> = None;
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word_boundary`/`word.is_empty()` are wrong: `$`, quotes
    // and other non-alphanumeric word characters never reach `word`, so
    // `$(echo $#)` and `$(echo a # )` both misjudge `#`.
    let mut token_boundary = true;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            token_boundary = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            // An escaped character inside an armed pattern region is
            // pattern content (rubash#2789 bookkeeping).
            if case_depth > 0 && case_pattern_region && !double {
                case_pattern_content = true;
            }
            token_boundary = false;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            token_boundary = false;
            index += 2;
            continue;
        }
        if ch == '#' && !single && !double && token_boundary {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            token_boundary = true;
            index += 1;
            continue;
        }
        if ch == '`' && !single {
            // A backquote unit inside an armed pattern region is
            // pattern content (rubash#2789 bookkeeping).
            if case_depth > 0 && case_pattern_region && !double {
                case_pattern_content = true;
            }
            if let Some((_, next_index)) = backtick_command_substitution(chars, index) {
                index = next_index;
                token_boundary = false;
                continue;
            }
        }
        // Inside double quotes, `$(...)` is a nested command substitution
        // (GNU parse.y `xparse_dolparen` / subst.c `extract_command_substitution`):
        // a `"` inside the nested `$(...)` does NOT close the outer double
        // quote.  Recursively skip the nested `$(...)` as a unit so the
        // outer `double` state is preserved.  Without this,
        // `$(echo "foo$(echo ")")")` misparses the inner `"` as the close of
        // the outer quote, truncating the substitution body.
        //
        // The same unit skip applies UNQUOTED (parse.y:4173: `$(` hands the
        // whole nested text to parse_comsub, whose `)` owns the inner
        // substitution, never the enclosing case machinery). Without it the
        // inner `)` reaches the case-subject bookkeeping as if it were
        // glued to the subject word — `$(case $(ls) in *) ... ;; esac)` cut
        // the body at `case $(ls`, and the trailing ` in *) ... ` `)` then
        // surfaced as the read-time `unexpected token `)'`. The `\u{1}`
        // placeholder marks the skipped span as one word unit, exactly how
        // the `\\` arm keeps `c\ase` out of the reserved-word machine, so a
        // `$(...)` case subject still completes the `case WORD` stage
        // (stage 1 -> 2) and lets `in` arm the pattern region. A `$(` span
        // inside an armed pattern region is pattern content — same
        // bookkeeping the backquote arm above applies.
        // `$((` is included: arithmetic or `$(` + subshell, the recursive
        // span ends at the same `))`/` ) ` boundary, and its `)` belongs to
        // the nested unit either way (`$(case $((1)) in *) ... ;; esac)`
        // cut at `case $((1)` for the same reason).
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') {
            if let Some((_, next_index)) = dollar_command_substitution(chars, index) {
                index = next_index;
                if !double {
                    word.push('\u{1}');
                    if case_depth > 0 && case_pattern_region {
                        case_pattern_content = true;
                        case_in_completed_empty = false;
                    }
                }
                token_boundary = false;
                continue;
            }
        }
        // Heredoc bodies are opaque to command-substitution delimiter matching.
        if ch == '<' && !single && !double && chars.get(index + 1) == Some(&'<') {
            if let Some((next_index, header_close)) =
                skip_command_substitution_heredoc(chars, index)
            {
                if let Some((paren, header_end)) = header_close {
                    // GNU parse.y parse_comsub:4564 + print_comsub
                    // (parse.y:4632): the `)` on the heredoc header line
                    // closes the substitution while the pending body is
                    // gathered from the following input lines and reprinted
                    // inside the word (`echo $(cat << EOF)` + `foo`/`bar`/
                    // `EOF` lines yields `foo bar`).  Rebuild the inner
                    // source the same way: header text up to the `)`, then
                    // the gathered body lines.
                    let mut source: String = chars[start + 2..paren].iter().collect();
                    source.extend(chars[header_end..next_index].iter());
                    let mut text = String::from("$(");
                    text.push_str(&source);
                    text.push(')');
                    return Some((
                        command_substitution_node(text, source, false, false, false),
                        next_index,
                    ));
                }
                index = next_index;
                // A heredoc terminator ends on its own line, so the next
                // character begins a fresh token.
                token_boundary = true;
                continue;
            }
        }
        // rubash#2789 bookkeeping (see the state declarations above):
        // track whether the armed pattern region has seen pattern
        // content yet. The captures are the pre-update values the
        // close arm below judges by.
        let region_before = case_pattern_region;
        let content_before = case_pattern_content;
        let stage_before = case_in_stage;
        let word_before = word.clone();
        let sig_before = prev_sig;
        // rubash#2789: note the `in` completion (see the declaration
        // above); the latch itself is applied after the region
        // bookkeeping at the bottom of the loop, so the disarm-clear
        // there cannot erase it in its own iteration. The completing
        // char itself is not `)` — the glued form is condition (b)
        // at the close arm, not the latch.
        let in_completed_here =
            word_before == "in" && stage_before == 2 && ch != ')' && !single && !double;
        if case_depth > 0 && region_before && !single && !double {
            match ch {
                '(' => {
                    // The pattern-list opener is available only as the
                    // first content of a pattern; any other `(` nests.
                    if case_pattern_paren_depth == 0
                        && !case_pattern_content
                        && case_pattern_opener_available
                    {
                        case_pattern_opener_available = false;
                    } else {
                        case_pattern_paren_depth += 1;
                        case_pattern_content = true;
                        case_in_completed_empty = false;
                    }
                }
                '|' if case_pattern_paren_depth == 0 => {
                    // Pattern separator: the next pattern starts empty.
                    case_pattern_content = false;
                    case_pattern_opener_available = true;
                }
                ')' if case_pattern_paren_depth > 0 => {
                    case_pattern_paren_depth -= 1;
                }
                '&' => {
                    // `;&` is a clause terminator, not pattern content.
                    if chars.get(index.wrapping_sub(1)) != Some(&';') {
                        case_pattern_content = true;
                        case_in_completed_empty = false;
                    }
                }
                ';' => {
                    // rubash#3306: `;;`/`;&`/`;;&` while the pattern
                    // region is armed is a clause terminator in pattern
                    // position — fatal in GNU's grammar (the terminator
                    // is only legal after a clause body). Latch it: the
                    // case can never complete, so the next top-level `)`
                    // closes the substitution for the body verdict.
                    if case_pattern_paren_depth == 0
                        && matches!(chars.get(index + 1), Some(';') | Some('&'))
                    {
                        case_clause_terminator_in_pattern = true;
                    }
                }
                c if c.is_whitespace() => {}
                _ => {
                    case_pattern_content = true;
                    case_in_completed_empty = false;
                }
            }
        }
        update_command_substitution_case_depth(
            &chars,
            index,
            ch,
            single,
            double,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            &mut case_in_stage,
            &mut case_pattern_region,
            prev_sig,
        );
        // rubash#380 witness feed (skip.rs:2176 / quotes.rs:1325
        // convention): only unquoted punctuation updates the previous-
        // token witness; word chars and whitespace never do. This loop
        // has no early single/double arm — quoted bytes reach the call
        // above — so the feed is gated explicitly.
        if !single && !double && crate::lexer::esac_prev_token_char(ch) {
            prev_sig = Some(ch);
        }
        // `$(` nested inside the case context: its `)` belongs to the
        // inner substitution, never to the case-close arm below. The
        // counter is a paren DEPTH inside the nested substitution, so
        // a subshell group inside it (`$(a (b) c)`) consumes its own
        // `)` first instead of mis-closing the `$(`.
        if !single
            && !double
            && case_depth > 0
            && ch == '('
            && (nested_case_subst > 0 || chars.get(index.wrapping_sub(1)) == Some(&'$'))
        {
            nested_case_subst += 1;
        }
        let closes_nested_subst = ch == ')' && !single && !double && nested_case_subst > 0;
        if closes_nested_subst {
            nested_case_subst -= 1;
        }
        // rubash#2789: a `)` that cannot close anything inside the
        // case — no pattern content in the armed region (the pattern
        // never started, e.g. right after `in` or after a `;;`), the
        // `)` glued to the `in` word itself, or the `)` glued to the
        // case subject — closes the substitution at the substitution's
        // own depth (parse.y:4519-4521), yielding the truncated body
        // for the body parser's grammar verdict. Pattern closers
        // (content seen), nested-paren closers, nested `$(` closers
        // and quoted `)` never reach this arm.
        let case_close_candidate =
            ch == ')' && !single && !double && depth == 1 && case_depth > 0 && !closes_nested_subst;
        if case_close_candidate
            && ((region_before && !content_before)
                || (word_before == "in" && stage_before == 2)
                || (case_in_stage == 2 && !region_before && !word_before.is_empty())
                || case_in_completed_empty)
        {
            let text = chars[start..=index].iter().collect();
            // rubash#3306: an empty-pattern `()` fire keeps the `)` in
            // the body source — `case x in ()` parses to the NEAR `)'`
            // marker GNU's body parser raises on the empty pattern
            // (the `)` IS the token that offends), where the excluded
            // form `case x in (` reports a newline offender instead.
            let source: String = if region_before && !content_before && sig_before == Some('(') {
                chars[start + 2..=index].iter().collect()
            } else {
                chars[start + 2..index].iter().collect()
            };
            return Some((
                command_substitution_node(text, source, false, false, false),
                index + 1,
            ));
        }
        // rubash#3306: a `)` that GNU's parser takes as the
        // substitution's shell_eof_token also arrives with the pattern
        // region disarmed — in a clause body or the `case … in`
        // preamble — and inside a case already doomed by a
        // pattern-position clause terminator. Body parens own their
        // `)` first (case_body_paren_depth); the truncated body
        // reaches the body parser, whose verdict the read-time screen
        // reports (parse.y:4519-4521, 6858-6865), instead of the
        // extractor scanning off the end of the word.
        if case_close_candidate
            && case_body_paren_depth == 0
            && (!region_before || case_clause_terminator_in_pattern)
        {
            let text = chars[start..=index].iter().collect();
            let source = chars[start + 2..index].iter().collect();
            return Some((
                command_substitution_node(text, source, false, false, false),
                index + 1,
            ));
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double && case_depth == 0 => depth += 1,
            '(' if !single && !double && case_depth > 0 && !region_before => {
                case_body_paren_depth += 1;
            }
            ')' if !single && !double && case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let text = chars[start..=index].iter().collect();
                    let source = chars[start + 2..index].iter().collect();
                    return Some((
                        command_substitution_node(text, source, false, false, false),
                        index + 1,
                    ));
                }
            }
            ')' if !single && !double && case_depth > 0 && !region_before => {
                case_body_paren_depth = case_body_paren_depth.saturating_sub(1);
            }
            _ => {}
        }
        // Quoted characters are word text handled by the quote state
        // arms; the boundary only tracks characters the live tokenizer
        // sees.
        if !single && !double {
            token_boundary =
                ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        }
        // rubash#2789: a freshly armed pattern region (at `in`, or
        // re-armed at a clause terminator) starts with empty pattern
        // bookkeeping; a disarmed region re-arms the reset.
        if case_pattern_region {
            if !case_pattern_region_seen {
                case_pattern_content = false;
                case_pattern_paren_depth = 0;
                case_pattern_opener_available = true;
                case_pattern_region_seen = true;
            }
        } else {
            case_pattern_region_seen = false;
            // A disarmed region means the case moved past the empty
            // preamble (or ended): the completion latch is stale.
            case_in_completed_empty = false;
        }
        if in_completed_here {
            case_in_completed_empty = true;
        }
        index += 1;
    }
    None
}

fn braced_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let body_start = start + 2;
    let pipe_output = chars.get(body_start) == Some(&'|');
    if !pipe_output && !chars.get(body_start).is_some_and(|ch| ch.is_whitespace()) {
        return None;
    }

    let source_start = if pipe_output {
        body_start + 1
    } else {
        body_start
    };
    let mut index = source_start;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '{' if !single && !double => depth += 1,
            '}' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let text = chars[start..=index].iter().collect();
                    let source = chars[source_start..index].iter().collect();
                    return Some((
                        command_substitution_node(text, source, false, true, pipe_output),
                        index + 1,
                    ));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_command_substitution_heredoc(
    chars: &[char],
    start: usize,
) -> Option<(usize, Option<(usize, usize)>)> {
    let mut header_end = start + 2;
    while header_end < chars.len() && chars[header_end] != '\n' {
        header_end += 1;
    }
    if header_end >= chars.len() {
        return None;
    }

    // GNU parse.y (PST_EOFTOKEN): inside `$(...)` the `)` is the
    // substitution's eof token and a shell break character, so the heredoc
    // delimiter word ends at the first unquoted `)` and that `)` closes the
    // substitution on the header line (`cat << EOF)`).  parse_comsub then
    // gathers the still-pending heredoc body from the following input lines
    // (parse.y:4564 gather_here_documents) and print_comsub reprints the
    // command with the body inside the word.  Parse the delimiter word
    // quote/escape aware so `<<\)` keeps `)` as delimiter text.
    let mut index = start + 2;
    let strip_tabs = if chars.get(index) == Some(&'-') {
        index += 1;
        true
    } else {
        false
    };
    while index < header_end && matches!(chars[index], ' ' | '\t') {
        index += 1;
    }
    let mut delimiter = String::new();
    let mut single = false;
    let mut double = false;
    while index < header_end {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            } else {
                delimiter.push(ch);
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '"' {
                double = false;
            } else {
                delimiter.push(ch);
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' => single = true,
            '"' => double = true,
            '\\' if index + 1 < header_end => {
                delimiter.push(chars[index + 1]);
                index += 1;
            }
            // GNU parse.y:5305 read_token_word / shellbreak (parse.y:5688):
            // the heredoc delimiter WORD ends at ANY unquoted shell
            // metacharacter — not just `;|&)'. `<<EOF>#c)' therefore
            // declares delimiter `EOF' with `>' a GREATER operator
            // (parse.y:3667 shellmeta) and `#c)' a comment, so the `)'
            // inside the comment never closes the substitution and the
            // input ends hunting `)' (rubash#305 lexmix family: the old
            // set swallowed `>#c' INTO the delimiter).
            c if c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')' | '(' | '<' | '>') => break,
            '#' if delimiter.is_empty() => break,
            _ => delimiter.push(ch),
        }
        index += 1;
    }
    if strip_tabs {
        delimiter = delimiter.trim_start_matches('\t').to_string();
    }
    if delimiter.is_empty() {
        return None;
    }

    // An unquoted `)` anywhere after the delimiter word on the header line
    // closes the substitution (it is the eof token, not body text). A
    // word-initial `#' after an operator comments through the end of the
    // header line (parse.y:3630-3643 parse_comment), so a `)' inside that
    // comment — the `#c)' tail — does not close anything.
    let mut header_close_paren = None;
    {
        let mut rest = index;
        let mut rest_single = false;
        let mut rest_double = false;
        while rest < header_end {
            let ch = chars[rest];
            if rest_single {
                if ch == '\'' {
                    rest_single = false;
                }
            } else if rest_double {
                if ch == '"' {
                    rest_double = false;
                }
            } else {
                match ch {
                    '\'' => rest_single = true,
                    '"' => rest_double = true,
                    '\\' => rest += 1,
                    '#' if rest == index || chars[rest - 1].is_whitespace() => break,
                    ')' => {
                        header_close_paren = Some(rest);
                        break;
                    }
                    _ => {}
                }
            }
            rest += 1;
        }
    }

    let mut line_start = header_end + 1;
    while line_start <= chars.len() {
        let mut line_end = line_start;
        while line_end < chars.len() && chars[line_end] != '\n' {
            line_end += 1;
        }
        let line = chars[line_start..line_end].iter().collect::<String>();
        let candidate = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if candidate == delimiter {
            return Some((
                (line_end + (line_end < chars.len()) as usize).min(chars.len()),
                header_close_paren.map(|paren| (paren, header_end)),
            ));
        }
        // GNU parse.y gather_here_documents reads the whole heredoc body
        // before the parser looks at the next token, so a body line that
        // starts with the delimiter and contains `)` later on ends the
        // heredoc AND pushes the remainder back into the parser input
        // (make_cmd.c:602-611 PST_EOFTOKEN, shell_ungets(line+redir_len)).
        // Resume the caller's scan right after the delimiter prefix — the
        // pushed-back `)` (and anything before it, e.g. `x` in `EOFx)`)
        // re-enters the token stream, where the `)` closes the substitution.
        // Covers `EOF)`, `EOF )`, and `))` when the delimiter is `)` itself.
        if candidate.starts_with(delimiter.as_str()) && candidate[delimiter.len()..].contains(')') {
            let resume = line_start
                + (line.chars().count() - candidate.chars().count())
                + delimiter.chars().count();
            return Some((resume, None));
        }
        if line_end >= chars.len() {
            break;
        }
        line_start = line_end + 1;
    }
    None
}

fn update_command_substitution_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    case_in_stage: &mut u8,
    case_pattern_region: &mut bool,
    prev_sig: Option<char>,
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
        // PST_CASEPAT port (rubash#380, GNU parse.y:3177-3186 +
        // parser.h:29): the pattern region disarms at the `)` closing a
        // pattern list (parse.y:3787-3788), re-arms at the `;;`/`;&`/
        // `;;&` clause terminators (parse.y:3710/3759 — a single body `;`
        // does NOT re-arm it), and a `(`/`|` pattern opener ends the
        // directly-after-`in` window of the empty-case `esac`
        // (parse.y:3433-3441 needs last_read_token == IN; phantom rule 4
        // at parse.y:3183 makes `(esac` a pattern).
        if *case_depth > 0 {
            if ch == ')' && *case_pattern_region {
                *case_pattern_region = false;
            } else if ch == ';'
                && (chars.get(index + 1) == Some(&';') || chars.get(index + 1) == Some(&'&'))
            {
                *case_pattern_region = true;
            }
            if *case_pattern_region && *case_in_stage == 3 && matches!(ch, '(' | '|') {
                *case_in_stage = 4;
            }
        }
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let completing_after_case = *case_in_stage == 1;
    if completing_after_case {
        *case_in_stage = 2;
    }
    let reserved_word_allows_next = update_command_substitution_reserved_word_depth(
        word,
        *current_word_boundary,
        case_depth,
        case_in_stage,
        case_pattern_region,
        completing_after_case,
        prev_sig,
    );
    if *case_depth > 0 {
        if ch == ')' && *case_pattern_region {
            *case_pattern_region = false;
        } else if ch == ';'
            && (chars.get(index + 1) == Some(&';') || chars.get(index + 1) == Some(&'&'))
        {
            *case_pattern_region = true;
        }
    }
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

#[allow(clippy::too_many_arguments)]
fn update_command_substitution_reserved_word_depth(
    word: &str,
    word_boundary: bool,
    case_depth: &mut usize,
    case_in_stage: &mut u8,
    case_pattern_region: &mut bool,
    completing_after_case: bool,
    prev_sig: Option<char>,
) -> bool {
    if !word_boundary {
        // GNU special_case_tokens rule 6 (parse.y:3369-3386): the `in`
        // after a case subject is IN even off a reserved-word boundary —
        // the case grammar consumes it there regardless of how the
        // preceding word ended (a plain subject word never leaves a
        // separator behind, so `$(case x in ...)` reads its `in` with
        // word_boundary false). parse.y:3379/3396 arms the pattern
        // region on that token, which is what the extractor's unclosed-
        // case close arm needs to judge a later `)` (rubash#3756:
        // without it `$(case x in a|)` scanned as unclosed and the
        // read-time body screen never saw the truncated body).
        if word == "in" && *case_in_stage == 2 {
            *case_in_stage = 3;
            *case_pattern_region = true;
            return true;
        }
        // Even a non-boundary word ends the empty-case window.
        if !completing_after_case {
            *case_in_stage = 0;
        }
        return false;
    }

    // parse.y:3177: inside the pattern region a reserved-word-looking word
    // is a PATTERN (only ESAC may still close, and the existing
    // case_pattern_starts_with_esac_chars heuristic already answers when
    // `esac` is a pattern).
    if *case_pattern_region && word != "esac" {
        if !completing_after_case {
            *case_in_stage = 0;
        }
        return false;
    }

    let result = match word {
        "case" => {
            *case_depth += 1;
            *case_in_stage = 1;
            *case_pattern_region = false;
            false
        }
        "in" if *case_in_stage == 2 => {
            // GNU special_case_tokens rule 6 (parse.y:3369-3386): the `in'
            // after a case subject is IN even off a reserved-word boundary;
            // parse.y:3379/3396 arms the pattern region.
            *case_in_stage = 3;
            *case_pattern_region = true;
            true
        }
        "esac" if *case_in_stage == 3 => {
            // parse.y:3433-3441: `esac' directly after IN closes the empty
            // case `case WORD in esac'.
            *case_depth = case_depth.saturating_sub(1);
            *case_in_stage = 0;
            *case_pattern_region = false;
            true
        }
        "esac" => {
            // rubash#3834 / parse.y:3177-3186 CHECK_FOR_RESERVED_WORD +
            // 3181/3183 (Posix rules 4): `esac' is pattern text ONLY when
            // the previous token is `|' or the pattern-list `(' —
            // witnessed by `prev_sig`, the last significant unquoted
            // char fed to this machine. Every other boundary `esac'
            // (after `;;', at a clause-body start, before a construct
            // outside this case) is the ESAC keyword. This replaces the
            // `)`-then-evidence FORWARD lookahead, whose evidence could
            // come from a construct outside the case (`;; esac)` in a
            // nested comsub) — the same replacement the skip.rs /
            // embedded_mutations machines already carry (rubash#380).
            if matches!(prev_sig, Some('|') | Some('(')) {
                *case_in_stage = 0;
                false
            } else {
                *case_depth = case_depth.saturating_sub(1);
                *case_in_stage = 0;
                *case_pattern_region = false;
                true
            }
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done" => {
            *case_in_stage = 0;
            true
        }
        _ => {
            if !completing_after_case {
                *case_in_stage = 0;
            }
            return false;
        }
    };
    result
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

fn backtick_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let mut index = start + 1;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '`' {
            let text = chars[start..=index].iter().collect();
            let source =
                decode_backtick_substitution_source(chars[start + 1..index].iter().copied());
            return Some((
                command_substitution_node(text, source, true, false, false),
                index + 1,
            ));
        }
        index += 1;
    }
    None
}

fn decode_backtick_substitution_source<I>(chars: I) -> String
where
    I: IntoIterator<Item = char>,
{
    let mut decoded = String::new();
    let mut chars = chars.into_iter().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            crate::executor::markers::DATA_BACKTICK => decoded.push('`'),
            '\\' if chars.peek().copied() == Some('`') => {
                chars.next();
                decoded.push('`');
            }
            _ => decoded.push(ch),
        }
    }

    decoded
}

fn command_substitution_node(
    text: String,
    source: String,
    backtick: bool,
    current_shell: bool,
    pipe_output: bool,
) -> CommandSubstitutionNode {
    // rubash#131: a `$(` body is parsed in place by GNU's outer parse
    // (parse.y:4451 parse_comsub -> parse_string), so the parse-time extglob
    // gate applies to it exactly as to the surrounding script (GNU 5.3.0
    // rejects `r=$(case x in ?(a)) :;; esac)' with rc 2). A backtick body
    // instead parses inside the substituted child at execution time, where
    // a syntax error only kills the child (GNU prints `command
    // substitution: ... syntax error ...' and the script continues, rc 0).
    // Rubash parses both eagerly; for backticks the gate is therefore held
    // open so a gated-off pattern keeps the pre-#131 accepted-parse
    // behavior instead of aborting the whole script.
    // The gate is consulted by both the tokenizer and the parser (the case
    // pattern reader re-checks it), so hold it open across both stages for
    // backtick bodies.
    let saved_gate = crate::lexer::parse_extended_glob();
    if backtick {
        crate::lexer::set_parse_extended_glob(true);
    }
    let tokens = crate::lexer::tokenize(&source);
    // The body is its own parse level (parse.y:4524 resets the pending
    // counter at entry) but the HEREDOC_MAX cap still applies within it
    // (parse.y:3105 push_heredoc): a 17th pending heredoc parked the fatal
    // in the lexer and truncated the token stream. Skip the doomed body
    // parse — its truncation would only add a misleading unclosed-delimiter
    // diagnostic — and leave the condition parked for the driver, which
    // reports `maximum here-document count exceeded` and exits 2 before
    // anything executes.
    let commands = if crate::lexer::heredoc_overflow_pending() {
        Vec::new()
    } else {
        parse(&tokens).commands
    };
    if backtick {
        crate::lexer::set_parse_extended_glob(saved_gate);
    }
    let (open_delimiter, operator, close_delimiter) = if backtick {
        ("`".to_string(), "`".to_string(), "`".to_string())
    } else if current_shell {
        (
            "${".to_string(),
            if pipe_output { "${|" } else { "${" }.to_string(),
            "}".to_string(),
        )
    } else {
        ("$(".to_string(), "$".to_string(), ")".to_string())
    };
    CommandSubstitutionNode {
        text,
        open_delimiter_metadata: delimiter_metadata(&open_delimiter),
        open_delimiter,
        operator_metadata: delimiter_metadata(&operator),
        operator,
        source,
        close_delimiter_metadata: delimiter_metadata(&close_delimiter),
        close_delimiter,
        commands,
        backtick,
        current_shell,
        pipe_output,
        word_index: None,
        assignment_name: None,
    }
}

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::new(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}

#[cfg(test)]
mod case_reclassification_tests {
    use super::dollar_command_substitution;

    /// The extracted body of the `$(...)` at the start of `word`,
    /// when the extractor finds one.
    fn extract(word: &str) -> Option<String> {
        let chars: Vec<char> = word.chars().collect();
        dollar_command_substitution(&chars, 0).map(|(node, _)| node.source)
    }

    // The three trigger predicates of the in-field unclosed-case
    // reclassification (GNU parse.y:4519-4521: `)` is the
    // substitution's shell_eof_token when it cannot be a case token).

    /// (a) Armed-but-empty pattern region: a clause terminator
    /// re-arms the region with no pattern content yet; the `)` there
    /// closes the substitution, truncating the body for the body
    /// parser's grammar verdict (GNU: syntax error, -c 127).
    #[test]
    fn reclassifies_armed_empty_region_after_clause_terminator() {
        assert_eq!(
            extract("$(case x in a) echo;; )").as_deref(),
            Some("case x in a) echo;; ")
        );
    }

    /// (b) The `)` glued to the `in` word itself.
    #[test]
    fn reclassifies_paren_glued_to_in() {
        assert_eq!(extract("$(case x in)").as_deref(), Some("case x in"));
    }

    /// (c) The `)` glued to the case subject (no `in` seen yet).
    #[test]
    fn reclassifies_paren_glued_to_subject() {
        assert_eq!(extract("$(case x)").as_deref(), Some("case x"));
    }

    // Four non-trigger controls: the `)` belongs to the case, and
    // the extractor must not truncate the body at the preamble.

    /// Pattern content seen: the `)` closes the pattern list and
    /// the body scan completes over the whole body.
    #[test]
    fn keeps_pattern_content_close() {
        assert_eq!(
            extract("$(case x in a) echo hi;; esac)").as_deref(),
            Some("case x in a) echo hi;; esac")
        );
    }

    /// A subshell clause body: the pattern `)` still belongs to
    /// the case even though a command-position `(` follows it.
    #[test]
    fn keeps_subshell_body_shape() {
        assert_eq!(
            extract("$(case x in a) (echo ok);; esac)").as_deref(),
            Some("case x in a) (echo ok);; esac")
        );
    }

    /// Empty case: `esac` is the case's own close; the extractor
    /// must never truncate at `in`. With the pattern region armed at
    /// the `in` token (GNU special_case_tokens rule 6,
    /// parse.y:3369-3386 — rubash#3756), `esac` is recognized as the
    /// case close and the body scan completes over the whole span;
    /// before that the unarmed region made this scan run off the end
    /// and answer None (the tokeniser still owns the shape and the
    /// executed result is identical: `echo $(case x in esac)`,
    /// `x=$(case x in esac); echo $?` and the `v=… w=…` sibling form
    /// are byte-identical to GNU 5.3.20).
    #[test]
    fn keeps_empty_case_esac_shape() {
        assert_eq!(
            extract("$(case x in esac)").as_deref(),
            Some("case x in esac")
        );
    }

    /// A parenthesised pattern list: its `)` is the case's, and
    /// the body scan completes normally.
    #[test]
    fn keeps_nested_pattern_parens() {
        assert_eq!(
            extract("$(case x in (a) echo;; esac)").as_deref(),
            Some("case x in (a) echo;; esac")
        );
    }

    /// Space variant: `in` completes at the separating space, whose
    /// completion destroys the stage/region state the glued
    /// predicates judge by; the completion latch in the arm's
    /// bookkeeping carries the fact to the `)` (GNU: syntax error,
    /// -c 127). Pattern content after a spaced `in` clears the
    /// latch again: the `)` after `a` is the pattern closer.
    #[test]
    fn reclassifies_space_variant_via_completion_latch() {
        assert_eq!(extract("$(case x in )").as_deref(), Some("case x in "));
        assert_eq!(extract("$(case x in a)"), None);
    }

    /// rubash#3605 run-comsub-posix: an unquoted nested `$(` inside a
    /// case body is a complete word unit (GNU parse.y:4173 hands the
    /// span to parse_comsub atomically), never fed char-by-char to the
    /// case word machine — otherwise the inner `)` hits the glued-paren
    /// close predicate (rubash#2789) and truncates the body at
    /// `case $(ls`.
    #[test]
    fn keeps_comsub_case_subject() {
        // `esac` right after `in` stays the empty-case shape
        // (keeps_empty_case_esac_shape), comsub subject or not.
        assert_eq!(
            extract("$(case $(ls) in esac)").as_deref(),
            Some("case $(ls) in esac")
        );
        for w in [
            "$(case $(ls) in *) echo ok;; esac)",
            "$(case a$(ls)b in *) echo ok;; esac)",
            "$(case $((1)) in *) echo ok;; esac)",
            "$(case $(case q in q) echo qq;; esac) in qq) echo n;; esac)",
            "$(if $(echo true); then echo ok; fi)",
            "$(for i in $(echo a b); do echo x; done)",
        ] {
            let inner = &w[2..w.len() - 1];
            assert_eq!(extract(w).as_deref(), Some(inner), "input: {w}");
        }
    }
}
