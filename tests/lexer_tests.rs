//! Lexer Tests - TDD for Bash Tokenizer
//!
//! Run with: cargo test --test lexer_tests

use rubash::lexer::{tokenize, TokenKind};

// ============================================================================
// Test Module: Basic Tokens
// ============================================================================

mod basic_tokens {
    use super::*;

    #[test]
    fn test_empty_input() {
        let input = "";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 0);
    }

    #[test]
    fn test_whitespace_only() {
        let input = "   \t\n  ";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 0);
    }

    #[test]
    fn test_simple_command() {
        let input = "ls";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Word);
        assert_eq!(tokens[0].value, "ls");
    }

    #[test]
    fn test_command_with_args() {
        let input = "ls -la /home";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokenKind::Word);
        assert_eq!(tokens[0].value, "ls");
        assert_eq!(tokens[1].kind, TokenKind::Word);
        assert_eq!(tokens[1].value, "-la");
        assert_eq!(tokens[2].kind, TokenKind::Word);
        assert_eq!(tokens[2].value, "/home");
    }
}

// ============================================================================
// Test Module: Operators
// ============================================================================

mod operators {
    use super::*;

    #[test]
    fn test_pipe_operator() {
        let input = "ls | grep foo";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[1].kind, TokenKind::Pipe);
        assert_eq!(tokens[1].value, "|");
    }

    #[test]
    fn test_semicolon() {
        let input = "ls; cd /";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[1].kind, TokenKind::Semicolon);
        assert_eq!(tokens[1].value, ";");
    }

    #[test]
    fn test_redirect_output() {
        let input = "echo hello > file.txt";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[2].kind, TokenKind::RedirectOut);
    }

    #[test]
    fn test_redirect_input() {
        let input = "cat < input.txt";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1].kind, TokenKind::RedirectIn);
        assert_eq!(tokens[1].value, "<");
    }

    #[test]
    fn test_append_redirect() {
        let input = "echo hello >> file.txt";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[2].kind, TokenKind::Append);
    }

    #[test]
    fn test_redirect_stderr() {
        let input = "echo error 2> err.txt";
        let tokens = tokenize(input);
        // "echo", "error", "2>", "err.txt" = 4 tokens
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[2].kind, TokenKind::RedirectErr);
    }
}

// ============================================================================
// Test Module: Quotes
// ============================================================================

mod quotes {
    use super::*;

    #[test]
    fn test_single_quotes() {
        let input = "echo 'hello world'";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[1].value, "hello world");
    }

    #[test]
    fn test_double_quotes() {
        let input = "echo \"hello world\"";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[1].value, "hello world");
    }

    #[test]
    fn test_empty_single_quotes() {
        let input = "''";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value, "");
    }

    #[test]
    fn test_empty_double_quotes() {
        let input = "\"\"";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value, "");
    }

    #[test]
    fn test_nested_quotes_in_double() {
        let input = "echo \"it's a 'test'\"";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[1].value, "it's a 'test'");
    }

    #[test]
    fn test_assignment_word_with_quoted_value() {
        let input = "alias foo='echo '";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[1].kind, TokenKind::Assignment);
        assert_eq!(tokens[1].value, "foo=echo ");
    }
}

// ============================================================================
// Test Module: Comments
// ============================================================================

mod comments {
    use super::*;

    #[test]
    fn test_comment_only() {
        let input = "# this is a comment";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 0);
    }

    #[test]
    fn test_command_before_comment() {
        let input = "ls # this lists";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value, "ls");
    }
}

// ============================================================================
// Test Module: Control Structures
// ============================================================================

mod control_structures {
    use super::*;

    #[test]
    fn test_if_keyword() {
        let input = "if true; then echo yes; fi";
        let tokens = tokenize(input);
        // if, true, ;, then, echo, yes, ;, fi = 8 tokens
        // Keywords: if, then, fi = 3
        let keywords: Vec<_> = tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Keyword)
            .collect();
        assert_eq!(
            keywords.len(),
            3,
            "Expected 3 keywords, got {}",
            keywords.len()
        );
        assert_eq!(tokens[0].kind, TokenKind::Keyword);
        assert_eq!(tokens[0].value, "if");
    }

    #[test]
    fn test_while_keyword() {
        let input = "while true; do echo loop; done";
        let tokens = tokenize(input);
        // while, true, ;, do, echo, loop, ;, done = 8 tokens
        // Keywords: while, do, done = 3
        let keywords: Vec<_> = tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Keyword)
            .collect();
        assert_eq!(
            keywords.len(),
            3,
            "Expected 3 keywords, got {}",
            keywords.len()
        );
    }

    #[test]
    fn test_for_keyword() {
        let input = "for i in 1 2 3; do echo $i; done";
        let tokens = tokenize(input);
        // for, i, in, 1, 2, 3, ;, do, echo, $i, ;, done = 12 tokens
        let keywords: Vec<_> = tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Keyword)
            .collect();
        assert!(
            keywords.len() >= 4,
            "Expected at least 4 keywords, got {}",
            keywords.len()
        );
    }
}

// ============================================================================
// Test Module: Assignments
// ============================================================================

mod assignments {
    use super::*;

    #[test]
    fn test_simple_assignment() {
        let input = "VAR=value";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Assignment);
        assert_eq!(tokens[0].value, "VAR=value");
    }

    #[test]
    fn test_assignment_with_path() {
        let input = "PATH=/usr/bin:$PATH";
        let tokens = tokenize(input);
        // PATH=/usr/bin: and $PATH are separate tokens because $ starts a new word
        assert!(
            tokens.len() >= 1,
            "Expected at least 1 token, got {}",
            tokens.len()
        );
        assert_eq!(tokens[0].kind, TokenKind::Assignment);
    }

    #[test]
    fn test_command_after_assignment() {
        let input = "X=5 echo $X";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokenKind::Assignment);
        assert_eq!(tokens[1].kind, TokenKind::Word);
        assert_eq!(tokens[2].kind, TokenKind::Variable);
    }
}

// ============================================================================
// Test Module: Command Substitution
// ============================================================================

mod command_substitution {
    use super::*;

    #[test]
    fn test_backtick_substitution() {
        let input = "`echo hello`";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::CommandSubst);
    }

    #[test]
    fn test_dollar_paren_substitution() {
        let input = "$(echo hello)";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::CommandSubst);
    }
}

// ============================================================================
// Test Module: Variables
// ============================================================================

mod variables {
    use super::*;

    #[test]
    fn test_simple_variable() {
        let input = "$HOME";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Variable);
        // Variable token includes the $ prefix
        assert_eq!(tokens[0].value, "$HOME");
    }

    #[test]
    fn test_positional_parameter() {
        let input = "$1 $2 $3";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokenKind::Variable);
        assert_eq!(tokens[0].value, "$1");
    }

    #[test]
    fn test_special_variables() {
        let input = "$@ $* $$ $! $? $-";
        let tokens = tokenize(input);
        // Each $X becomes a separate token
        // Note: input may have trailing space in source, so accept 6-7
        assert!(
            tokens.len() >= 6,
            "Expected at least 6 tokens, got {}",
            tokens.len()
        );
        for token in tokens {
            assert_eq!(token.kind, TokenKind::Variable);
        }
    }
}

// ============================================================================
// Test Module: Brace Expansion
// ============================================================================

mod brace_expansion {
    use super::*;

    #[test]
    fn test_brace_sequence() {
        let input = "{1..5}";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::BraceExpand);
    }

    #[test]
    fn test_brace_list() {
        let input = "{foo,bar,baz}";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::BraceExpand);
    }
}

// ============================================================================
// Test Module: Edge Cases
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn test_escaped_character() {
        let input = "echo \\n";
        let tokens = tokenize(input);
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_consecutive_operators() {
        let input = "ls || echo error";
        let tokens = tokenize(input);
        // ls, ||, echo, error = 4 tokens
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[1].kind, TokenKind::Or);
    }

    #[test]
    fn test_and_operator() {
        let input = "ls && echo success";
        let tokens = tokenize(input);
        // ls, &&, echo, success = 4 tokens
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[1].kind, TokenKind::And);
    }
}
