//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

mod ansi;
mod brace_scan;
mod classification;
mod continuation;
mod heredoc;
mod heredoc_scan;
mod quotes;
mod scanner;
mod skip;
mod token;
mod word;

#[cfg(test)]
mod tests;

use brace_scan::{has_unclosed_brace_group, opens_function_body_after_previous_signature};
use continuation::{
    ends_with_unquoted_backslash, has_unclosed_command_substitution, has_unclosed_quotes,
};
use heredoc::heredoc_delimiters;
use scanner::Lexer;

pub use token::{Token, TokenKind};

pub fn tokenize(input: &str) -> Vec<Token> {
    if input.trim().is_empty() {
        return Vec::new();
    }

    let mut tokens = tokenize_with_heredocs(input);
    if tokens
        .last()
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        tokens.pop();
    }
    tokens
}

fn tokenize_with_heredocs(input: &str) -> Vec<Token> {
    // TODO(parse.y/redir.c): Bash parses here-documents after reading the
    // complete command and performs delimiter-specific expansion rules. This
    // line-oriented collector handles the simple `<<word` and `<<'word'`
    // forms used by early upstream alias tests.
    let mut output = Vec::new();
    let mut lines = input.lines();
    let mut position = 0;
    let mut line_number = 1;
    let mut logical_start_line = 1;
    let mut logical_line = String::new();
    let mut continued_line = false;

    while let Some(line) = lines.next() {
        if logical_line.is_empty() {
            logical_start_line = line_number;
        }
        if !logical_line.is_empty() && !continued_line {
            logical_line.push('\n');
        }
        continued_line = false;
        logical_line.push_str(line);
        position += line.len() + 1;
        line_number += 1;

        if ends_with_unquoted_backslash(&logical_line) {
            logical_line.pop();
            continued_line = true;
            continue;
        }

        if has_unclosed_quotes(&logical_line) {
            continue;
        }
        if has_unclosed_command_substitution(&logical_line) {
            continue;
        }
        if has_unclosed_brace_group(&logical_line)
            && !opens_function_body_after_previous_signature(&logical_line, &output)
        {
            continue;
        }

        let mut line_tokens = tokenize_plain(&logical_line);
        for token in &mut line_tokens {
            token.position = logical_start_line;
        }
        let delimiters = heredoc_delimiters(&line_tokens, &logical_line);
        output.append(&mut line_tokens);
        logical_line.clear();

        for delimiter in delimiters {
            let mut body = String::new();
            let mut continued_body_line = String::new();
            let mut found_delimiter = false;
            for body_line in lines.by_ref() {
                position += body_line.len() + 1;
                line_number += 1;
                let mut comparable = if delimiter.strip_tabs {
                    body_line.trim_start_matches('\t')
                } else {
                    body_line
                }
                .to_string();

                if !delimiter.quoted {
                    if let Some(stripped) = comparable.strip_suffix('\\') {
                        continued_body_line.push_str(stripped);
                        continue;
                    }
                    if !continued_body_line.is_empty() {
                        continued_body_line.push_str(&comparable);
                        comparable = std::mem::take(&mut continued_body_line);
                    }
                }

                if comparable == delimiter.value
                    || (delimiter.allow_closing_paren
                        && comparable
                            .strip_suffix(')')
                            .is_some_and(|value| value == delimiter.value))
                {
                    found_delimiter = true;
                    break;
                }
                body.push_str(&comparable);
                body.push('\n');
            }
            if !found_delimiter {
                body.insert(0, '\x1f');
            }
            if delimiter.quoted {
                body.insert(0, '\x1e');
            } else {
                body = body.replace("\\\n", "");
            }
            output.push(Token::new(TokenKind::HereDocBody, &body, position));
        }
        output.push(Token::new(TokenKind::Semicolon, ";", logical_start_line));
    }

    if !logical_line.is_empty() {
        let mut line_tokens = tokenize_plain(&logical_line);
        for token in &mut line_tokens {
            token.position = logical_start_line;
        }
        output.append(&mut line_tokens);
        output.push(Token::new(TokenKind::Semicolon, ";", logical_start_line));
    }

    output
}

fn tokenize_plain(input: &str) -> Vec<Token> {
    let lexer = Lexer::new(input);
    let mut tokens = Vec::new();
    for token in lexer {
        if token.kind == TokenKind::Eof {
            break;
        }
        tokens.push(token);
    }
    tokens
}
