use std::str::from_utf8;

use super::scanner::Lexer;

impl<'a> Lexer<'a> {
    pub(super) fn skip_cmd_subst(&mut self) {
        let mut depth = 1;
        let mut case_depth = 0usize;
        let mut word = String::new();
        let mut word_boundary = true;
        let mut current_word_boundary = true;
        while let Some(c) = self.advance() {
            let rest = from_utf8(&self.input[self.position..]).unwrap_or("");
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
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                '<' if self.peek() == Some('<') && self.peek_after(1) == Some('<') => {
                    self.advance();
                    self.advance();
                }
                '<' if self.peek() == Some('<') => self.skip_heredoc_in_command_substitution(),
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

    pub(super) fn skip_heredoc_in_command_substitution(&mut self) {
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
            self.advance();
        }
        let mut delimiter = from_utf8(&self.input[delimiter_start..self.position])
            .unwrap_or("")
            .replace(['\'', '"', '\\'], "");
        if strip_tabs {
            delimiter = delimiter.trim_start_matches('\t').to_string();
        }
        if delimiter.is_empty() {
            return;
        }
        while self.peek().is_some_and(|ch| ch != '\n') {
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
            let line = from_utf8(&self.input[line_start..self.position]).unwrap_or("");
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
            if self.peek() == Some('\n') {
                self.advance();
            }
        }
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
            } else if c == '$' {
                match self.peek() {
                    Some('{') => {
                        self.advance();
                        self.skip_braced();
                    }
                    Some('(') => {
                        self.advance();
                        self.skip_cmd_subst();
                    }
                    _ => {}
                }
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    pub(super) fn skip_braced(&mut self) {
        let mut depth = 1usize;
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
                '"' => double = !double,
                '$' if self.peek() == Some('{') => {
                    self.advance();
                    depth += 1;
                }
                '$' if self.peek() == Some('(') => {
                    self.advance();
                    self.skip_cmd_subst();
                }
                '$' if self.peek() == Some('[') => {
                    self.advance();
                    self.skip_arith_bracket();
                }
                '}' if !double => {
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
        while let Some(c) = self.advance() {
            let rest = from_utf8(&self.input[self.position..]).unwrap_or("");
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
                            self.skip_braced();
                        }
                        Some('(') => {
                            self.advance();
                            self.skip_cmd_subst();
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
        let rest = from_utf8(&self.input[self.position..]).unwrap_or("");
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
            false
        }
        "esac" => false,
        "then" | "do" | "else" | "elif" | "in" => true,
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
            false
        }
        "then" | "do" | "else" | "elif" | "in" if *current_word_boundary => true,
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | '\n')
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
        if ch == ')' {
            return false;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        word.clear();
        word_boundary = matches!(ch, ';' | '&' | '|' | '(' | '\n');
        scan += 1;
    }

    word == "esac" && word_boundary
}
