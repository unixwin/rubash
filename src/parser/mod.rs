//! Parser Module - Bash Parser
//!
//! Transforms tokens into an AST.

use crate::lexer::{Token, TokenKind};

/// Represents a redirect specification
#[derive(Debug, Clone, PartialEq)]
pub struct Redirect {
    pub fd: Option<u32>,
    pub target: String,
    pub append: bool,
}

/// Represents a narrow `for` compound command.
#[derive(Debug, Clone)]
pub struct ForCommand {
    pub variable: String,
    pub words: Vec<String>,
    pub body: Vec<CommandNode>,
}

/// Represents a narrow `case` compound command.
#[derive(Debug, Clone)]
pub struct CaseCommand {
    pub word: String,
    pub clauses: Vec<CaseClause>,
}

#[derive(Debug, Clone)]
pub struct CaseClause {
    pub patterns: Vec<String>,
    pub body: Vec<CommandNode>,
}

/// Represents a parsed command
#[derive(Debug, Clone)]
pub struct CommandNode {
    /// The command words (first is the command name)
    pub words: Vec<String>,
    /// Variable assignments
    pub assignments: std::collections::HashMap<String, String>,
    /// Input redirect
    pub redirect_in: Option<Redirect>,
    /// Output redirect
    pub redirect_out: Option<Redirect>,
    /// Append redirect
    pub append: Option<Redirect>,
    /// Stderr redirect
    pub redirect_err: Option<Redirect>,
    /// Stderr append redirect
    pub redirect_err_append: Option<Redirect>,
    /// Here-document stdin body
    pub heredoc: Option<String>,
    /// Pipe to next command
    pub pipe: Option<usize>,
    /// Background execution (&)
    pub background: bool,
    /// `for name in words; do ...; done`
    pub for_command: Option<ForCommand>,
    /// `case word in pattern) ... ;; esac`
    pub case_command: Option<CaseCommand>,
    /// Script line number where this command starts, when known.
    pub line: Option<usize>,
}

impl CommandNode {
    pub fn new() -> Self {
        Self {
            words: Vec::new(),
            assignments: std::collections::HashMap::new(),
            redirect_in: None,
            redirect_out: None,
            append: None,
            redirect_err: None,
            redirect_err_append: None,
            heredoc: None,
            pipe: None,
            background: false,
            for_command: None,
            case_command: None,
            line: None,
        }
    }

    /// Returns Some(true) for &&, Some(false) for ||, None otherwise
    pub fn and_or(&self) -> Option<bool> {
        None
    }
}

impl Default for CommandNode {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents a parsed AST
#[derive(Debug, Clone)]
pub struct Ast {
    /// List of commands
    pub commands: Vec<CommandNode>,
}

/// Parse tokens into an AST
pub fn parse(tokens: &[Token]) -> Ast {
    let mut ast = Ast {
        commands: Vec::new(),
    };
    let mut current_cmd = CommandNode::new();

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        if token.kind == TokenKind::Keyword
            && token.value == "for"
            && command_is_empty(&current_cmd)
        {
            if let Some((for_cmd, next_i)) = parse_for_command(tokens, i) {
                ast.commands.push(for_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        if token.kind == TokenKind::Keyword
            && token.value == "case"
            && command_is_empty(&current_cmd)
        {
            if let Some((case_cmd, next_i)) = parse_case_command(tokens, i) {
                ast.commands.push(case_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        match token.kind {
            TokenKind::Word | TokenKind::Variable | TokenKind::CommandSubst => {
                note_command_line(&mut current_cmd, token);
                current_cmd.words.push(token.value.clone());
            }
            TokenKind::Assignment => {
                note_command_line(&mut current_cmd, token);
                if let Some(pos) = token.value.find('=') {
                    if current_cmd.words.is_empty() {
                        let var_name = token.value[..pos].to_string();
                        let mut var_value = token.value[pos + 1..].to_string();
                        if var_value.is_empty() {
                            if let Some((compound_value, next_i)) =
                                collect_compound_assignment(tokens, i)
                            {
                                var_value = compound_value;
                                i = next_i;
                            }
                        }
                        current_cmd.assignments.insert(var_name, var_value);
                    } else {
                        current_cmd.words.push(token.value.clone());
                    }
                }
            }
            TokenKind::Pipe => {
                // Save current command with pipe flag
                current_cmd.pipe = Some(1);
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::Semicolon => {
                // Command separator
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::RedirectIn => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_in = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectOut => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_out = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::Append => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.append = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: true,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectErr => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_err = Some(Redirect {
                            fd: Some(2),
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectErrAppend => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_err_append = Some(Redirect {
                            fd: Some(2),
                            target: tokens[i + 1].value.clone(),
                            append: true,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::HereDoc => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    i += 1;
                }
            }
            TokenKind::HereDocBody => {
                note_command_line(&mut current_cmd, token);
                current_cmd.heredoc = Some(token.value.clone());
            }
            TokenKind::And | TokenKind::Or => {
                // TODO: Handle logical operators
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::Keyword => {
                // TODO(parse.y): Reserved words are only reserved in specific
                // parser states. If an ordinary command has already started,
                // keep the token text so alias expansion can reparse it later.
                if !matches!(token.value.as_str(), "(" | ")" | "{" | "}") {
                    note_command_line(&mut current_cmd, token);
                    current_cmd.words.push(token.value.clone());
                }
            }
            TokenKind::Eof => {
                break;
            }
            _ => {
                // Skip other token types (keywords, variables, etc.)
            }
        }

        i += 1;
    }

    // Don't forget the last command
    if !command_is_empty(&current_cmd) {
        ast.commands.push(current_cmd);
    }

    ast
}

fn parse_for_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports all `for_command`
    // grammar alternatives, nested compound lists, redirections on compound
    // commands, `"$@"` default words, and reserved-word parsing state. This
    // maps the simple upstream alias test form: `for name in words; do body; done`.
    let variable = tokens.get(start + 1)?.value.clone();
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable
    ) {
        return None;
    }

    let mut i = start + 2;
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    i += 1;

    let mut words = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, "do") {
        if tokens[i].kind == TokenKind::Semicolon {
            i += 1;
            continue;
        }
        if matches!(
            tokens[i].kind,
            TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
        ) {
            words.push(tokens[i].value.clone());
        }
        i += 1;
    }

    if !is_keyword(tokens, i, "do") {
        return None;
    }
    i += 1;

    let body_start = i;
    while i < tokens.len() && !is_keyword(tokens, i, "done") {
        i += 1;
    }

    if !is_keyword(tokens, i, "done") {
        return None;
    }

    let body = parse(&tokens[body_start..i]).commands;
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(ForCommand {
        variable,
        words,
        body,
    });
    Some((command, i + 1))
}

fn collect_compound_assignment(tokens: &[Token], start: usize) -> Option<(String, usize)> {
    // TODO(parse.y/arrayfunc.c): Bash parses `name=(...)` as a compound array
    // assignment WORD and later expands it with `assign_array_var_from_string`.
    // This preserves the simple parenthesized value shape used by alias.tests.
    if !is_keyword(tokens, start + 1, "(") {
        return None;
    }

    let mut i = start + 2;
    let mut values = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, ")") {
        if matches!(
            tokens[i].kind,
            TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
        ) {
            values.push(tokens[i].value.clone());
        }
        i += 1;
    }

    if !is_keyword(tokens, i, ")") {
        return None;
    }

    Some((format!("({})", values.join(" ")), i))
}

fn parse_case_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports `;&`, `;;&`, `|`-joined
    // pattern lists, extglob patterns, nested compound lists, and redirections
    // on the compound command. This covers the simple upstream alias3.sub
    // `case word in pattern) list ;; *) list ;; esac` shape.
    let word = tokens.get(start + 1)?.value.clone();
    let mut i = start + 2;
    while i < tokens.len() && !is_keyword(tokens, i, "in") {
        i += 1;
    }
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    i += 1;

    let mut clauses = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, "esac") {
        while i < tokens.len() && tokens[i].kind == TokenKind::Semicolon {
            i += 1;
        }
        if is_keyword(tokens, i, "esac") {
            break;
        }

        let mut patterns = Vec::new();
        while i < tokens.len() && !is_keyword(tokens, i, ")") {
            if matches!(
                tokens[i].kind,
                TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
            ) {
                patterns.push(tokens[i].value.clone());
            }
            i += 1;
        }
        if !is_keyword(tokens, i, ")") {
            return None;
        }
        i += 1;

        let body_start = i;
        while i < tokens.len()
            && !is_keyword(tokens, i, "esac")
            && !(tokens[i].kind == TokenKind::Word && tokens[i].value == ";;")
        {
            i += 1;
        }
        let body = parse(&tokens[body_start..i]).commands;
        clauses.push(CaseClause { patterns, body });

        if i < tokens.len() && tokens[i].kind == TokenKind::Word && tokens[i].value == ";;" {
            i += 1;
        }
    }

    if !is_keyword(tokens, i, "esac") {
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.case_command = Some(CaseCommand { word, clauses });
    Some((command, i + 1))
}

fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
}

fn is_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == value)
}

fn command_is_empty(cmd: &CommandNode) -> bool {
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.heredoc.is_none()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.for_command.is_none()
        && cmd.case_command.is_none()
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::lexer::tokenize;

    #[test]
    fn test_parse_simple() {
        let tokens = tokenize("ls -la");
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words.len(), 2);
    }

    #[test]
    fn test_parse_pipeline() {
        let tokens = tokenize("ls | grep foo");
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
    }

    #[test]
    fn test_parse_empty() {
        let tokens: Vec<Token> = vec![];
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 0);
    }
}
