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
    /// Pipe to next command
    pub pipe: Option<usize>,
    /// Background execution (&)
    pub background: bool,
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
            pipe: None,
            background: false,
        }
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
    let mut ast = Ast { commands: Vec::new() };
    let mut current_cmd = CommandNode::new();

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        match token.kind {
            TokenKind::Word => {
                current_cmd.words.push(token.value.clone());
            }
            TokenKind::Assignment => {
                if let Some(pos) = token.value.find('=') {
                    let var_name = token.value[..pos].to_string();
                    let var_value = token.value[pos+1..].to_string();
                    current_cmd.assignments.insert(var_name, var_value);
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
                if i + 1 < tokens.len() {
                    if let TokenKind::Word = tokens[i + 1].kind {
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
                if i + 1 < tokens.len() {
                    if let TokenKind::Word = tokens[i + 1].kind {
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
                if i + 1 < tokens.len() {
                    if let TokenKind::Word = tokens[i + 1].kind {
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
                if i + 1 < tokens.len() {
                    if let TokenKind::Word = tokens[i + 1].kind {
                        current_cmd.redirect_err = Some(Redirect {
                            fd: Some(2),
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::And | TokenKind::Or => {
                // TODO: Handle logical operators
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
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
    if !current_cmd.words.is_empty() || !current_cmd.assignments.is_empty() {
        ast.commands.push(current_cmd);
    }

    ast
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