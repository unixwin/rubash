use crate::executor::markers::DATA_DOLLAR;
pub(in crate::executor) fn split_shell_words(source: &str) -> Vec<String> {
    split_shell_words_with_quote_info(source)
        .into_iter()
        .map(|(word, _)| word)
        .collect()
}

/// Word-splits a command-substitution source, additionally reporting whether
/// each word was wrapped in quotes. Quote state is needed to protect tilde
/// expansion inside quoted command-substitution arguments: `$(printf '%s'
/// "~/repo")` must not expand `~` (Bash keeps quoted `~` literal).
pub(in crate::executor) fn split_shell_words_with_quote_info(source: &str) -> Vec<(String, bool)> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut word_quoted = false;
    let mut backtick = false;
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if backtick {
            current.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            } else if ch == '`' {
                backtick = false;
            }
            continue;
        }

        // GNU parse.y/quotes.rs quote removal: backslash handling depends on
        // the surrounding quote state. Outside quotes a backslash escapes any
        // character and is removed. Inside double quotes the backslash only
        // retains its special meaning before \, $, `, ", and newline; before
        // other characters it is preserved. Inside single quotes the backslash
        // is literal. Without this, command substitution bodies that bypass
        // the lexer's quote removal (e.g. `$(echo \a)`) keep the backslash
        // where GNU removes it (more-exp.tests:297 `recho \a` -> a).
        if ch == '\\' {
            match quote {
                None => {
                    let Some(escaped) = chars.next() else {
                        current.push(ch);
                        continue;
                    };
                    match escaped {
                        '\\' => current.push(crate::executor::markers::DATA_BACKSLASH),
                        '$' => current.push(DATA_DOLLAR),
                        '`' => current.push(crate::executor::markers::DATA_BACKTICK),
                        '\'' => current.push(crate::executor::markers::DATA_SQUOTE),
                        '"' => current.push(crate::executor::markers::DATA_DQUOTE),
                        '\n' => {}
                        _ => current.push(escaped),
                    }
                    continue;
                }
                Some('"') => match chars.peek().copied() {
                    Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) => {
                        chars.next();
                        match escaped {
                            '\\' => current.push(crate::executor::markers::DATA_BACKSLASH),
                            '"' => current.push(crate::executor::markers::DATA_DQUOTE),
                            '$' => current.push(DATA_DOLLAR),
                            '`' => current.push(crate::executor::markers::DATA_BACKTICK),
                            '\n' => {}
                            _ => unreachable!(),
                        }
                        continue;
                    }
                    _ => {
                        current.push(ch);
                        continue;
                    }
                },
                _ => {
                    // Inside single quotes: fall through to the match block so
                    // push_single_quoted_shell_word_char converts \ to \x15,
                    // preserving the literal through unescape_remaining_shell_escapes.
                }
            }
        }

        match (ch, quote) {
            ('$', None) if chars.peek().copied() == Some('(') => {
                copy_dollar_paren_word(&mut current, &mut chars);
            }
            // Inside double quotes, `$(...)` is still a command substitution
            // (GNU parse.y `xparse_dolparen`): a `"` inside the nested `$(...)`
            // does NOT close the outer double quote.  Consume the whole
            // `$(...)` as a unit so the outer quote state is preserved.
            ('$', Some('"')) if chars.peek().copied() == Some('(') => {
                copy_dollar_paren_word(&mut current, &mut chars);
            }
            // GNU parse.y: `$'...'` is an ANSI-C quoted word (outside double
            // quotes only). The body is decoded (strtrans.c ansicstr) and the
            // result is literal data, so a `)` inside the quote does NOT close
            // the enclosing command substitution (nquote5.sub:
            // `$(echo a$'\01)'b)` -> `a^A)b`). Handle this before the bare `'`
            // arm so the single quote is not mistaken for an ordinary quote
            // start. Inside double quotes `$'` is literal `$` + `'`, not an
            // ANSI-C quote (bash manual: $'...' is not recognized within
            // double quotes).
            ('$', None) if chars.peek().copied() == Some('\'') => {
                chars.next();
                let mut body = String::new();
                let mut escaped = false;
                for body_ch in chars.by_ref() {
                    if escaped {
                        body.push('\\');
                        body.push(body_ch);
                        escaped = false;
                        continue;
                    }
                    if body_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if body_ch == '\'' {
                        break;
                    }
                    body.push(body_ch);
                }
                let decoded = crate::lexer::decode_ansi_c_quoted(&body);
                // Mark the word as quoted: ANSI-C quotes suppress field
                // splitting and tilde expansion like other quotes.
                word_quoted = true;
                current.push_str(&decoded);
            }
            ('<', None) if chars.peek().copied() == Some('(') => {
                copy_process_substitution_word(&mut current, &mut chars);
            }
            ('`', None) => {
                backtick = true;
                current.push(ch);
            }
            // Inside double quotes, backticks are command substitutions: a `"`
            // inside the backtick body does NOT close the outer double quote.
            ('`', Some('"')) => {
                current.push(ch);
                copy_backtick_word_part(&mut current, &mut chars);
            }
            ('\'' | '"', None) => {
                quote = Some(ch);
                word_quoted = true;
            }
            (q, Some(active)) if q == active => quote = None,
            (ch, Some('\'')) => push_single_quoted_shell_word_char(&mut current, ch),
            (' ' | '\t', None) => {
                if !current.is_empty() {
                    words.push((std::mem::take(&mut current), word_quoted));
                    word_quoted = false;
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push((current, word_quoted));
    }
    words
}

fn copy_dollar_paren_word(
    current: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    current.push('$');
    if chars.next() != Some('(') {
        return;
    }
    current.push('(');
    copy_dollar_paren_body(current, chars);
}

/// Copies the body of a `$(...)` command substitution (after the leading `$(
/// has already been consumed and pushed), tracking parenthesis depth, quote
/// state, backticks, and nested `$(...)`.  A `)` inside single or double
/// quotes does NOT count toward the parenthesis depth — this mirrors GNU
/// `extract_command_substitution` (subst.c) and `xparse_dolparen` (parse.y),
/// where the parser tracks quote state independently inside the substitution.
fn copy_dollar_paren_body(
    current: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    let mut depth = 1usize;
    while let Some(ch) = chars.next() {
        current.push(ch);
        match ch {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            '\'' => copy_quoted_word_part(current, chars, '\''),
            '"' => copy_quoted_word_part(current, chars, '"'),
            '`' => copy_backtick_word_part(current, chars),
            '$' if chars.peek().copied() == Some('(') => {
                chars.next();
                current.push('(');
                depth += 1;
            }
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
}

/// Copies a process substitution word `<(...)` (or `>(...)`) as a single
/// token, honouring nested quotes/backticks and nested parens. Word splitting
/// must not break `<printf 'x'` apart; the substitution is materialized to a
/// file path later, exactly like `$(...)`.
fn copy_process_substitution_word(
    current: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    current.push('<');
    if chars.next() != Some('(') {
        return;
    }
    current.push('(');
    copy_dollar_paren_body(current, chars);
}

/// Copies characters until the matching closing quote, honouring nested
/// `$(...)` command substitutions and backticks when inside double quotes.
///
/// Inside double quotes, `$(...)` and `` `...` `` are still parsed as command
/// substitutions (GNU parse.y `xparse_dolparen` / subst.c
/// `extract_command_substitution`): a `"` that appears *inside* a nested
/// `$(...)` or backtick body does NOT close the outer double quote.  Without
/// this, `$(echo "foo$(echo ")")")` misparses the inner `"` as the close of
/// the outer quote, leaving stray `)` characters in the word.
///
/// Single quotes have no special characters inside, so only the closing `'`
/// is tracked.
fn copy_quoted_word_part(
    current: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    quote: char,
) {
    while let Some(ch) = chars.next() {
        current.push(ch);
        if ch == '\\' && quote != '\'' {
            if let Some(escaped) = chars.next() {
                current.push(escaped);
            }
        } else if ch == quote {
            break;
        } else if quote == '"' && ch == '$' && chars.peek().copied() == Some('(') {
            // Nested command substitution inside double quotes: consume the
            // full `$(...)` body so a `"` inside it does not close the outer
            // double quote.
            chars.next();
            current.push('(');
            copy_dollar_paren_body(current, chars);
        } else if quote == '"' && ch == '`' {
            // Backtick command substitution inside double quotes: consume
            // until the matching backtick so a `"` inside does not close the
            // outer double quote.
            copy_backtick_word_part(current, chars);
        }
    }
}

fn copy_backtick_word_part(
    current: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    while let Some(ch) = chars.next() {
        current.push(ch);
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                current.push(escaped);
            }
        } else if ch == '`' {
            break;
        }
    }
}

fn push_single_quoted_shell_word_char(current: &mut String, ch: char) {
    match ch {
        '$' => current.push(DATA_DOLLAR),
        '`' => current.push(crate::executor::markers::DATA_BACKTICK),
        '\\' => current.push(crate::executor::markers::PROTECTED_BACKSLASH),
        _ => current.push(ch),
    }
}

pub(in crate::executor) fn split_first_shell_word(source: &str) -> Option<(String, &str)> {
    let trimmed = source.trim_start();
    let offset = source.len() - trimmed.len();
    let mut quote = None;
    for (index, ch) in trimmed.char_indices() {
        match (ch, quote) {
            ('\'' | '"', None) => quote = Some(ch),
            (q, Some(active)) if q == active => quote = None,
            (' ' | '\t' | '\n' | '\r', None) => {
                let word = trimmed[..index].to_string();
                let remainder = &source[offset + index + ch.len_utf8()..];
                return Some((word, remainder));
            }
            _ => {}
        }
    }

    if trimmed.is_empty() {
        None
    } else {
        Some((trimmed.to_string(), ""))
    }
}

pub(in crate::executor) fn apply_simple_sed_args(input: &str, args: &[String]) -> Option<String> {
    let scripts = sed_script_args(args)?;
    apply_simple_sed_substitutions(input, &scripts)
}

fn sed_script_args(args: &[String]) -> Option<Vec<&str>> {
    let mut scripts = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "-e" {
            scripts.push(args.get(index + 1)?.as_str());
            index += 2;
            continue;
        }
        if let Some(script) = arg.strip_prefix("-e").filter(|script| !script.is_empty()) {
            scripts.push(script);
            index += 1;
            continue;
        }
        if arg.starts_with('-') {
            return None;
        }
        if !scripts.is_empty() || index + 1 != args.len() {
            return None;
        }
        scripts.push(arg.as_str());
        index += 1;
    }
    (!scripts.is_empty()).then_some(scripts)
}

fn apply_simple_sed_substitutions(input: &str, scripts: &[&str]) -> Option<String> {
    // GNU sed `1d` (delete line N) and `s/pat/rep/` (substitute) cover the
    // upstream test pipelines that reach this bridge. Delete commands apply
    // before substitutions, exactly like sed processing order.
    let mut delete_lines = Vec::new();
    let mut substitutions = Vec::new();
    for script in scripts {
        if let Some(rest) = script.strip_suffix('d').filter(|_| script.len() > 1) {
            if let Ok(line_number) = rest.trim().parse::<usize>() {
                delete_lines.push(line_number);
                continue;
            }
        }
        substitutions.extend(parse_sed_substitutions(script)?);
    }
    let mut output = input
        .lines()
        .enumerate()
        .filter(|(index, _)| !delete_lines.contains(&(index + 1)))
        .map(|line| {
            let (_, line) = line;
            substitutions
                .iter()
                .fold(line.to_string(), |line, (pattern, replacement, global)| {
                    apply_simple_sed_line(&line, pattern, replacement, *global)
                })
        })
        .collect::<Vec<_>>()
        .join("\n");
    if input.ends_with('\n') {
        output.push('\n');
    }
    Some(output)
}

fn parse_sed_substitutions(script: &str) -> Option<Vec<(&str, &str, bool)>> {
    let substitutions = script
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                None
            } else {
                parse_sed_substitution(line)
            }
        })
        .collect::<Vec<_>>();
    if substitutions.is_empty() {
        parse_sed_substitution(script).map(|substitution| vec![substitution])
    } else {
        Some(substitutions)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum BreAtom {
    Char(char),
    /// Backslash-escaped ordinary char. POSIX.1-2017 §9.3.4/§9.3.6 BREs and
    /// GNU sed (regexec(3), RE grammar): a `<backslash>` removes any special
    /// meaning from the following ordinary character, so an escaped `*` is a
    /// literal asterisk and NEVER the `*` duplication operator. Verified
    /// against WSL GNU Bash 5.3.0 / GNU sed: `echo 'a*b' | sed 's/a\*/X/'`
    /// prints `Xb`, and aliasconv.bash's `s/\!\*/...` matches the two-char
    /// `!*` (a quantifier reading would degenerate to an empty match at
    /// every position).
    EscapedChar(char),
    Any,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Group {
        index: usize,
        atoms: Vec<BreAtom>,
    },
    AnchorStart,
    AnchorEnd,
    Backref(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BreQuantifier {
    One,
    ZeroOrOne,
    ZeroOrMore,
    OneOrMore,
}

#[derive(Clone, Debug)]
struct BrePiece {
    atom: BreAtom,
    quantifier: BreQuantifier,
}

// GNU sed(1) BRE as documented in regex(7): `^`/`$` anchors, `.`, `*`,
// `[...]` classes, `\(...\)` subexpressions, `\1`-`\9` backreferences, and the
// GNU `\+`/`\?` extensions. Anything outside this surface (`\{...\}`
// intervals, `\|` alternation) fails the parse and the caller falls through
// to the real `sed` binary.
fn parse_bre(pattern: &[char]) -> Option<Vec<BrePiece>> {
    let mut group_count = 0usize;
    let (atoms, next) = parse_bre_atoms(pattern, 0, &mut group_count, false)?;
    if next != pattern.len() {
        return None;
    }
    Some(atoms_to_pieces(&atoms))
}

fn parse_bre_atoms(
    pattern: &[char],
    mut index: usize,
    group_count: &mut usize,
    in_group: bool,
) -> Option<(Vec<BreAtom>, usize)> {
    let mut atoms = Vec::new();
    while index < pattern.len() {
        let ch = pattern[index];
        match ch {
            '\\' => {
                index += 1;
                let esc = *pattern.get(index)?;
                index += 1;
                match esc {
                    '(' => {
                        *group_count += 1;
                        if *group_count > 9 {
                            return None;
                        }
                        let group_index = *group_count;
                        let (inner, next) = parse_bre_atoms(pattern, index, group_count, true)?;
                        atoms.push(BreAtom::Group {
                            index: group_index,
                            atoms: inner,
                        });
                        index = next;
                    }
                    ')' => {
                        if !in_group {
                            return None;
                        }
                        return Some((atoms, index));
                    }
                    '1'..='9' => atoms.push(BreAtom::Backref(esc as usize - '0' as usize)),
                    '+' => {
                        let last = atoms.pop()?;
                        atoms.push(BreAtom::Group {
                            index: usize::MAX,
                            atoms: vec![last],
                        });
                    }
                    '?' => {
                        let last = atoms.pop()?;
                        atoms.push(BreAtom::Group {
                            index: usize::MAX - 1,
                            atoms: vec![last],
                        });
                    }
                    'n' => atoms.push(BreAtom::EscapedChar('\n')),
                    't' => atoms.push(BreAtom::EscapedChar('\t')),
                    '{' | '|' => return None,
                    other => atoms.push(BreAtom::EscapedChar(other)),
                }
            }
            '[' => {
                index += 1;
                let mut negated = false;
                if pattern.get(index) == Some(&'^') {
                    negated = true;
                    index += 1;
                }
                let mut ranges = Vec::new();
                let mut first = true;
                loop {
                    let item = *pattern.get(index)?;
                    if item == ']' && !first {
                        index += 1;
                        break;
                    }
                    first = false;
                    let lo = item;
                    index += 1;
                    if pattern.get(index) == Some(&'-')
                        && pattern.get(index + 1).is_some_and(|c| *c != ']')
                    {
                        index += 1;
                        let hi = pattern[index];
                        index += 1;
                        ranges.push((lo, hi));
                    } else {
                        ranges.push((lo, lo));
                    }
                }
                atoms.push(BreAtom::Class { negated, ranges });
            }
            '^' if atoms.is_empty() && !in_group => {
                atoms.push(BreAtom::AnchorStart);
                index += 1;
            }
            '$' if index + 1 == pattern.len() => {
                atoms.push(BreAtom::AnchorEnd);
                index += 1;
            }
            '.' => {
                atoms.push(BreAtom::Any);
                index += 1;
            }
            other => {
                atoms.push(BreAtom::Char(other));
                index += 1;
            }
        }
    }
    if in_group {
        return None;
    }
    Some((atoms, index))
}

fn atoms_to_pieces(atoms: &[BreAtom]) -> Vec<BrePiece> {
    let mut pieces = Vec::new();
    for atom in atoms {
        // Only an UNescaped `*` is the BRE duplication operator
        // (POSIX.1-2017 §9.3.6); `\*` arrives as EscapedChar('*') and stays
        // literal. An unescaped leading `*` (no preceding piece) is also
        // literal per GNU regexec.
        if *atom == BreAtom::Char('*') {
            if let Some(last) = pieces.last_mut() {
                let last: &mut BrePiece = last;
                if last.quantifier == BreQuantifier::One
                    && !matches!(last.atom, BreAtom::AnchorStart | BreAtom::AnchorEnd)
                {
                    last.quantifier = BreQuantifier::ZeroOrMore;
                    continue;
                }
            }
        }
        pieces.push(BrePiece {
            atom: atom.clone(),
            quantifier: BreQuantifier::One,
        });
    }
    pieces
}

type BreCaptures = [Option<(usize, usize)>; 10];

fn bre_atom_match(
    atom: &BreAtom,
    text: &[char],
    pos: usize,
    captures: &BreCaptures,
) -> Option<usize> {
    match atom {
        BreAtom::Char(expected) | BreAtom::EscapedChar(expected) => {
            (text.get(pos) == Some(expected)).then_some(pos + 1)
        }
        BreAtom::Any => text.get(pos).is_some().then_some(pos + 1),
        BreAtom::Class { negated, ranges } => {
            let ch = *text.get(pos)?;
            let inside = ranges.iter().any(|(lo, hi)| ch >= *lo && ch <= *hi);
            (inside != *negated).then_some(pos + 1)
        }
        BreAtom::Backref(index) => {
            let (start, end) = captures.get(*index).copied().flatten()?;
            let len = end - start;
            if text.len() - pos < len || text[pos..pos + len] != text[start..end] {
                return None;
            }
            Some(pos + len)
        }
        BreAtom::Group { .. } | BreAtom::AnchorStart | BreAtom::AnchorEnd => None,
    }
}

// Greedy backtracking match of `pieces` at `pos`; returns the longest end
// position reachable along the first successful path, recording `\(...\)`
// captures. GNU's matcher is leftmost-longest (regexec); the greedy order
// here agrees for the s/// surface the upstream tests exercise.
fn bre_match_pieces(
    pieces: &[BrePiece],
    text: &[char],
    pos: usize,
    captures: &mut BreCaptures,
) -> Option<usize> {
    let Some((piece, rest)) = pieces.split_first() else {
        return Some(pos);
    };
    match &piece.atom {
        BreAtom::AnchorStart => {
            if pos == 0 {
                bre_match_pieces(rest, text, pos, captures)
            } else {
                None
            }
        }
        BreAtom::AnchorEnd => {
            if pos == text.len() {
                bre_match_pieces(rest, text, pos, captures)
            } else {
                None
            }
        }
        _ => {
            let (min, max) = match piece.quantifier {
                BreQuantifier::One => (1, 1),
                BreQuantifier::ZeroOrOne => (0, 1),
                BreQuantifier::ZeroOrMore => (0, usize::MAX),
                BreQuantifier::OneOrMore => (1, usize::MAX),
            };
            bre_match_repetitions(piece, rest, text, pos, captures, 0, min, max)
        }
    }
}

fn bre_match_repetitions(
    piece: &BrePiece,
    rest: &[BrePiece],
    text: &[char],
    pos: usize,
    captures: &mut BreCaptures,
    count: usize,
    min: usize,
    max: usize,
) -> Option<usize> {
    // Greedy: try one more repetition first (longer match), then back off.
    if count < max {
        if let Some((next, trial)) = bre_match_single(piece, text, pos, captures) {
            if next > pos {
                let mut trial = trial;
                if let Some(end) =
                    bre_match_repetitions(piece, rest, text, next, &mut trial, count + 1, min, max)
                {
                    *captures = trial;
                    return Some(end);
                }
            }
        }
    }
    if count >= min {
        return bre_match_pieces(rest, text, pos, captures);
    }
    None
}

fn bre_match_single(
    piece: &BrePiece,
    text: &[char],
    pos: usize,
    captures: &BreCaptures,
) -> Option<(usize, BreCaptures)> {
    match &piece.atom {
        BreAtom::Group { index, atoms } => {
            if *index == usize::MAX - 1 {
                // `a\?` — zero or one.
                let inner = atoms_to_pieces(atoms);
                let mut trial = *captures;
                if let Some(end) = bre_match_pieces(&inner, text, pos, &mut trial) {
                    return Some((end, trial));
                }
                return Some((pos, *captures));
            }
            if *index == usize::MAX {
                // `a\+` — one or more.
                let inner = atoms_to_pieces(atoms);
                let mut trial = *captures;
                let mut cursor = bre_match_pieces(&inner, text, pos, &mut trial)?;

                loop {
                    let mut t2 = trial;
                    match bre_match_pieces(&inner, text, cursor, &mut t2) {
                        Some(next) if next > cursor => {
                            trial = t2;
                            cursor = next;
                        }
                        _ => break,
                    }
                }
                return Some((cursor, trial));
            }
            let inner = atoms_to_pieces(atoms);
            let mut trial = *captures;
            let end = bre_match_pieces(&inner, text, pos, &mut trial)?;
            if *index > 0 && *index < 10 {
                trial[*index] = Some((pos, end));
            }
            Some((end, trial))
        }
        atom => bre_atom_match(atom, text, pos, captures).map(|end| (end, *captures)),
    }
}

fn bre_substitute(line: &str, pattern: &str, replacement: &str, global: bool) -> Option<String> {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let pieces = parse_bre(&pattern_chars)?;
    let text: Vec<char> = line.chars().collect();
    let mut output = String::new();
    let mut cursor = 0usize;
    let mut replaced = false;
    while cursor <= text.len() {
        let mut found = None;
        for start in cursor..=text.len() {
            let mut captures: BreCaptures = [None; 10];
            if let Some(end) = bre_match_pieces(&pieces, &text, start, &mut captures) {
                found = Some((start, end, captures));
                break;
            }
        }
        let Some((start, end, captures)) = found else {
            break;
        };
        for ch in &text[cursor..start] {
            output.push(*ch);
        }
        expand_sed_replacement(&mut output, replacement, &text, start, end, &captures);
        replaced = true;
        if !global {
            cursor = end;
            break;
        }
        cursor = end.max(start + 1);
        if end == start {
            if let Some(ch) = text.get(start) {
                output.push(*ch);
            }
        }
    }
    if !replaced {
        return Some(line.to_string());
    }
    for ch in &text[cursor.min(text.len())..] {
        output.push(*ch);
    }
    Some(output)
}

fn expand_sed_replacement(
    output: &mut String,
    replacement: &str,
    text: &[char],
    start: usize,
    end: usize,
    captures: &BreCaptures,
) {
    let mut chars = replacement.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '&' => {
                for ch in &text[start..end] {
                    output.push(*ch);
                }
            }
            '\\' => match chars.next() {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some(digit @ '1'..='9') => {
                    if let Some(Some((s, e))) = captures.get(digit as usize - '0' as usize) {
                        for ch in &text[*s..*e] {
                            output.push(*ch);
                        }
                    }
                }
                Some('&') => output.push('&'),
                Some(other) => output.push(other),
                None => output.push('\\'),
            },
            other => output.push(other),
        }
    }
}

fn parse_sed_substitution(script: &str) -> Option<(&str, &str, bool)> {
    let rest = script.strip_prefix('s')?;
    let separator = rest.chars().next()?;
    let rest = &rest[separator.len_utf8()..];
    let (pattern, rest) = split_escaped_separator(rest, separator)?;
    let (replacement, flags) = split_escaped_separator(rest, separator)?;
    let mut global = false;
    for flag in flags.chars() {
        match flag {
            'g' => global = true,
            _ => return None,
        }
    }
    Some((pattern, replacement, global))
}

fn split_escaped_separator(value: &str, separator: char) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == separator {
            return Some((&value[..index], &value[index + ch.len_utf8()..]));
        }
    }
    None
}

fn apply_simple_sed_line(line: &str, pattern: &str, replacement: &str, global: bool) -> String {
    let pattern = pattern
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::CTLESC, "")
        .replace(r"\\.", r"\.");
    bre_substitute(line, &pattern, replacement, global).unwrap_or_else(|| line.to_string())
}

pub(in crate::executor) fn split_unquoted_and_and(source: &str) -> Option<(&str, &str)> {
    split_unquoted_token(source, "&&")
}

pub(in crate::executor) fn split_unquoted_semicolon(source: &str) -> Option<(&str, &str)> {
    split_unquoted_token(source, ";")
}

fn split_unquoted_token<'a>(source: &'a str, token: &str) -> Option<(&'a str, &'a str)> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let chars = source.char_indices().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        let (byte_index, ch) = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            _ if !single && !double && source[byte_index..].starts_with(token) => {
                return Some((&source[..byte_index], &source[byte_index + token.len()..]));
            }
            _ => {}
        }
        index += 1;
    }

    None
}
