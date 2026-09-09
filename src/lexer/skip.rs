use super::dolbrace::{scan_braced_parameter, BraceContext, DolbraceState};
use super::scanner::Lexer;

impl<'a> Lexer<'a> {
    pub(super) fn skip_cmd_subst(&mut self) {
        let mut depth = 1;
        let mut case_depth = 0usize;
        let mut word = String::new();
        let mut word_boundary = true;
        let mut current_word_boundary = true;
        while let Some(c) = self.advance() {
            if c == '\\' {
                self.advance();
                continue;
            }
            if c == '#' && word_boundary {
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.advance();
                }
                word.clear();
                word_boundary = true;
                current_word_boundary = true;
                continue;
            }
            let rest = &self.input[self.position..];
            update_command_substitution_case_depth(
                c,
                false,
                false,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
                rest,
            );
            match c {
                '`' => {
                    self.skip_backtick();
                    continue;
                }
                '(' if case_depth == 0 => depth += 1,
                ')' if case_depth == 0 => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '$' if self.peek() == Some('\'') => {
                    self.advance();
                    self.skip_ansi_c_single();
                }
                '$' if self.peek() == Some('(') => {
                    self.advance();
                    if self.peek() == Some('(') {
                        self.advance();
                        self.skip_arith_paren();
                    } else {
                        self.skip_cmd_subst();
                    }
                }
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                '<' if self.peek() == Some('<') && self.peek_after(1) == Some('<') => {
                    self.advance();
                    self.advance();
                }
                '<' if self.peek() == Some('<') => {
                    if self.skip_heredoc_in_command_substitution() {
                        break;
                    }
                },
                _ => {}
            }
        }
    }

    pub(super) fn skip_arith_paren(&mut self) {
        let mut depth = 0usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        while let Some(c) = self.advance() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            match c {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '(' if !single && !double => depth += 1,
                ')' if !single && !double && depth > 0 => depth -= 1,
                ')' if !single && !double && self.peek() == Some(')') => {
                    self.advance();
                    break;
                }
                _ => {}
            }
        }
    }

    pub(super) fn skip_arith_bracket(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.advance() {
            match c {
                '[' => depth += 1,
                ']' if depth > 0 => depth -= 1,
                ']' => break,
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                '\\' => {
                    self.advance();
                }
                _ => {}
            }
        }
    }

    pub(super) fn skip_heredoc_in_command_substitution(&mut self) -> bool {
        self.advance();
        let strip_tabs = if self.peek() == Some('-') {
            self.advance();
            true
        } else {
            false
        };
        while self.peek().is_some_and(|ch| matches!(ch, ' ' | '\t')) {
            self.advance();
        }
        let delimiter_start = self.position;
        while self
            .peek()
            .is_some_and(|ch| !ch.is_whitespace() && !matches!(ch, ';' | '|' | '&' | ')'))
        {
            // A backslash quotes the next delimiter byte (`<<\)` uses a
            // literal `)` delimiter); consume the escape pair as one unit so
            // the quoted `)` is not mistaken for the substitution closer.
            if self.peek() == Some('\\') && self.peek_after(1).is_some() {
                self.advance();
            }
            self.advance();
        }
        let mut delimiter =
            self.input[delimiter_start..self.position].replace(['\'', '"', '\\'], "");
        if strip_tabs {
            delimiter = delimiter.trim_start_matches('\t').to_string();
        }
        if delimiter.is_empty() {
            return false;
        }
        let mut header_closes_command_substitution = false;
        while self.peek().is_some_and(|ch| ch != '\n') {
            if self.peek() == Some(')') {
                header_closes_command_substitution = true;
            }
            self.advance();
        }
        if self.peek() == Some('\n') {
            self.advance();
        }

        while !self.at_end() {
            let line_start = self.position;
            while self.peek().is_some_and(|ch| ch != '\n') {
                self.advance();
            }
            let line = &self.input[line_start..self.position];
            let comparable = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if comparable
                .strip_suffix(')')
                .is_some_and(|value| value == delimiter)
            {
                let leading_tabs = if strip_tabs {
                    line.chars().take_while(|ch| *ch == '\t').count()
                } else {
                    0
                };
                self.position = line_start + leading_tabs + delimiter.len();
                break;
            }
            if comparable == delimiter {
                if self.peek() == Some('\n') {
                    self.advance();
                }
                break;
            }
            // GNU make_cmd.c:602-611 (PST_EOFTOKEN): a body line that starts
            // with the delimiter and carries `)` later on the line ends the
            // heredoc as if it hit EOF; the rest of the line is pushed back
            // into the parser input, where the `)` then closes the command
            // substitution (`foo=$(cat <<EOF / hi / EOF )`). Resume the span
            // scan at that first `)`.
            if comparable.starts_with(delimiter.as_str()) {
                if let Some(paren) = comparable[delimiter.len()..].find(')') {
                    self.position = line_start + delimiter.len() + paren;
                    break;
                }
            }
            if self.peek() == Some('\n') {
                self.advance();
            }
        }
        header_closes_command_substitution
    }

    pub(super) fn skip_backtick(&mut self) {
        while let Some(c) = self.advance() {
            if c == '`' {
                break;
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    pub(super) fn skip_single(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\'' {
                break;
            }
        }
    }
    pub(super) fn skip_ansi_c_single(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\\' {
                self.advance();
            } else if c == '\'' {
                break;
            }
        }
    }
    pub(super) fn skip_double(&mut self) {
        while let Some(c) = self.advance() {
            if c == '"' {
                break;
            } else if c == '`' {
                self.skip_backtick();
            } else if c == '$' {
                match self.peek() {
                    Some('{') => {
                        self.advance();
                        self.skip_braced(true);
                    }
                    Some('(') => {
                        self.advance();
                        if self.peek() == Some('(') {
                            self.advance();
                            self.skip_arith_paren();
                        } else {
                            self.skip_cmd_subst();
                        }
                    }
                    _ => {}
                }
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    pub(super) fn skip_braced(&mut self, outer_double_quote: bool) {
        let start = self.position.saturating_sub(2);
        // Bash 5.3 funsub: `${ command; }` / `${|command;}` bodies are
        // command lists where every plain `{`/`}` (function bodies, brace
        // groups) nests the match — the parameter-expansion scanner would
        // close the span at the first unquoted `}`.
        if self
            .input
            .get(start + 2..start + 3)
            .is_some_and(|ch| ch == "|" || ch.chars().next().is_some_and(|c| c.is_whitespace()))
        {
            let mut depth = 1usize;
            let mut single = false;
            let mut double = false;
            let mut escaped = false;
            while let Some(c) = self.advance() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' && !single {
                    escaped = true;
                    continue;
                }
                match c {
                    '\'' if !double => single = !single,
                    '"' if !single => double = !double,
                    '{' if !single && !double => depth += 1,
                    '}' if !single && !double => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            return;
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        let context = BraceContext {
            outer_double_quote,
            // parse.y::parse_matched_pair keeps POSIX mode separate from the
            // surrounding quote state. The line tokenizer tracks runtime
            // `set -o posix` switches, mirroring GNU's lazy per-command parse.
            posix: self.posix,
            replacement_context: false,
            initial_state: DolbraceState::Param,
        };
        if let Some(scan) = scan_braced_parameter(&self.input[start..], context) {
            self.position = start + scan.end;
            return;
        }

        let mut depth = 1usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        while let Some(c) = self.advance() {
            if escaped {
                escaped = false;
                continue;
            }

            if c == '\\' {
                escaped = true;
                continue;
            }

            match c {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '$' if !single && self.peek() == Some('{') => {
                    self.advance();
                    depth += 1;
                }
                '$' if !single && self.peek() == Some('(') => {
                    self.advance();
                    if self.peek() == Some('(') {
                        self.advance();
                        self.skip_arith_paren();
                    } else {
                        self.skip_cmd_subst();
                    }
                }
                '$' if !single && self.peek() == Some('[') => {
                    self.advance();
                    self.skip_arith_bracket();
                }
                '}' if !single && !double => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    pub(super) fn skip_brace(&mut self) {
        let mut depth = 1usize;
        let mut case_depth = 0usize;
        let mut word = String::new();
        let mut word_boundary = true;
        let mut current_word_boundary = true;
        let mut comment_start = true;
        let mut saw_top_level_whitespace = false;
        let mut ansi_single = false;
        let mut escaped = false;
        while let Some(c) = self.advance() {
            if escaped {
                escaped = false;
                comment_start = false;
                continue;
            }
            if ansi_single {
                if c == '\\' {
                    escaped = true;
                } else if c == '\'' {
                    ansi_single = false;
                }
                comment_start = false;
                continue;
            }
            let rest = &self.input[self.position..];
            update_brace_group_case_depth(
                c,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
                rest,
            );
            if c == '\n' {
                if depth == 1 {
                    saw_top_level_whitespace = true;
                }
                comment_start = true;
                continue;
            }
            if c.is_whitespace() {
                if depth == 1 {
                    saw_top_level_whitespace = true;
                }
                comment_start = true;
                continue;
            }
            if c == '#' && comment_start {
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.advance();
                }
                continue;
            }
            match c {
                '{' if case_depth == 0 => {
                    comment_start = false;
                    depth += 1;
                }
                '}' if case_depth == 0 => {
                    comment_start = false;
                    depth -= 1;
                    if depth == 0 {
                        if self.peek() == Some('}') {
                            depth = 1;
                            continue;
                        }
                        if !saw_top_level_whitespace {
                            break;
                        }
                        if self.brace_close_can_end_compact_group() {
                            break;
                        }
                        depth = 1;
                    }
                }
                '$' => {
                    comment_start = false;
                    match self.peek() {
                        Some('{') => {
                            self.advance();
                            self.skip_braced(false);
                        }
                        Some('(') => {
                            self.advance();
                            if self.peek() == Some('(') {
                                self.advance();
                                self.skip_arith_paren();
                            } else {
                                self.skip_cmd_subst();
                            }
                        }
                        Some('\'') => {
                            self.advance();
                            ansi_single = true;
                        }
                        _ => {}
                    }
                }
                '`' => {
                    comment_start = false;
                    self.skip_backtick();
                }
                '\'' => {
                    comment_start = false;
                    self.skip_single();
                }
                '"' => {
                    comment_start = false;
                    self.skip_double();
                }
                '\\' => {
                    comment_start = false;
                    self.advance();
                }
                _ => {
                    comment_start = false;
                }
            }
        }
    }

    fn brace_close_can_end_compact_group(&self) -> bool {
        let rest = &self.input[self.position..];
        let mut saw_blank = false;
        for (index, ch) in rest.char_indices() {
            match ch {
                ' ' | '\t' | '\r' => {
                    saw_blank = true;
                    continue;
                }
                '\n' => return true,
                ';' | '|' | '&' | '<' | '>' | ')' => return true,
                _ if !saw_blank => return true,
                _ if ch.is_ascii_digit()
                    && rest[index..].chars().any(|c| matches!(c, '<' | '>')) =>
                {
                    return true;
                }
                _ => return brace_close_followed_by_reserved_word(&rest[index..]),
            }
        }
        true
    }
}

fn brace_close_followed_by_reserved_word(rest: &str) -> bool {
    const RESERVED: &[&str] = &["do", "done", "elif", "else", "esac", "fi", "then"];

    RESERVED.iter().any(|word| {
        rest.strip_prefix(word).is_some_and(|tail| {
            tail.chars().next().is_none_or(|ch| {
                ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>' | ')' | '(')
            })
        })
    })
}

fn update_brace_group_case_depth(
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    rest: &str,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if brace_group_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next =
        update_brace_group_reserved_word_depth(word, *current_word_boundary, case_depth, ch, rest);
    word.clear();
    *word_boundary = reserved_word_allows_next || brace_group_separator_allows_reserved_word(ch);
}

fn update_brace_group_reserved_word_depth(
    word: &str,
    word_boundary: bool,
    case_depth: &mut usize,
    delimiter: char,
    rest: &str,
) -> bool {
    if !word_boundary {
        return false;
    }

    match word {
        "case" => {
            *case_depth += 1;
            false
        }
        "esac" if !case_pattern_starts_with_esac_rest(delimiter, rest) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "esac" => false,
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done" => true,
        _ => false,
    }
}

fn brace_group_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | '{' | '\n')
}

fn update_command_substitution_case_depth(
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
