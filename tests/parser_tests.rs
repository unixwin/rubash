//! Parser Tests - TDD for Bash Parser
//!
//! Run with: cargo test --test parser_tests

use bashrs::lexer::tokenize;
use bashrs::parser::{parse, Ast, CommandNode};

mod simple_commands {
    use super::*;

    #[test]
    fn test_empty_input() {
        let input = "";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 0);
    }

    #[test]
    fn test_single_command() {
        let input = "ls -la";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words.len(), 2);
        assert_eq!(ast.commands[0].words[0], "ls");
    }

    #[test]
    fn test_command_with_args() {
        let input = "ls -la /home";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words.len(), 3);
    }
}

mod pipeline_tests {
    use super::*;

    #[test]
    fn test_simple_pipeline() {
        let input = "ls | grep foo";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].pipe.is_some());
    }

    #[test]
    fn test_multiple_pipeline() {
        let input = "ls | grep foo | sort";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 3);
    }
}

mod semicolon_tests {
    use super::*;

    #[test]
    fn test_sequential_commands() {
        let input = "ls; cd /";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
    }
}

mod assignment_tests {
    use super::*;

    #[test]
    fn test_variable_assignment() {
        let input = "VAR=value";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].assignments.contains_key("VAR"));
        assert_eq!(ast.commands[0].assignments.get("VAR"), Some(&"value".to_string()));
    }

    #[test]
    fn test_command_with_assignment() {
        let input = "X=5 echo hello";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].assignments.contains_key("X"));
    }
}

mod redirection_tests {
    use super::*;

    #[test]
    fn test_output_redirect() {
        let input = "echo hello > file.txt";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].redirect_out.is_some());
    }

    #[test]
    fn test_input_redirect() {
        let input = "cat < input.txt";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].redirect_in.is_some());
    }

    #[test]
    fn test_append_redirect() {
        let input = "echo hello >> file.txt";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].append.is_some());
    }
}

mod quote_preservation {
    use super::*;

    #[test]
    fn test_single_quotes_preserved() {
        let input = "echo 'hello world'";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words[1], "'hello world'");
    }

    #[test]
    fn test_double_quotes_preserved() {
        let input = "echo \"hello world\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words[1], "\"hello world\"");
    }
}