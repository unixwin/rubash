//! Parser Tests - TDD for Bash Parser
//!
//! Run with: cargo test --test parser_tests

use rubash::lexer::tokenize;
use rubash::parser::parse;

#[path = "parser_coproc_tests.rs"]
mod coproc_tests;
#[path = "parser_redirection_tests.rs"]
mod redirection_tests;

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

mod function_tests {
    use super::*;

    #[test]
    fn test_function_keyword_definition() {
        let input = "function greet { echo hi; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "greet");
        assert_eq!(function.body.len(), 1);
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_function_keyword_with_parentheses() {
        let input = "function greet() { echo hi; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "greet");
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_bash_function_name_can_contain_hyphen() {
        let input = "foo-a() { echo hi; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo-a");
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_function_definition_consumes_trailing_redirects() {
        let input = "foo() { echo hi; } 2> err; echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].function_command.is_some());
        assert_eq!(ast.commands[0].redirect_err.as_ref().unwrap().target, "err");
        assert_eq!(ast.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_compact_function_definition_consumes_trailing_redirects() {
        let input = "foo(){ echo hi; } > out; echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].function_command.is_some());
        assert_eq!(ast.commands[0].redirect_out.as_ref().unwrap().target, "out");
        assert_eq!(ast.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_function_keyword_name_can_look_like_assignment() {
        let input = "function foo=bar { echo hi; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo=bar");
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_function_body_can_be_for_command() {
        let input = "foo() for x in a b; do echo $x; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        let for_command = function.body[0].for_command.as_ref().unwrap();
        assert_eq!(for_command.variable, "x");
        assert_eq!(for_command.words, ["a", "b"]);
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_function_body_can_be_case_command() {
        let input = "foo() case $1 in a) echo alpha ;; *) echo other ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        let case_command = function.body[0].case_command.as_ref().unwrap();
        assert_eq!(case_command.word, "$1");
        assert_eq!(case_command.clauses.len(), 2);
    }

    #[test]
    fn test_function_body_can_be_if_command_sequence() {
        let input = "foo() if true; then echo yes; else echo no; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        assert_eq!(function.body[0].words, ["if", "true"]);
        assert_eq!(function.body[1].words, ["then", "echo", "yes"]);
        assert_eq!(function.body[3].words, ["fi"]);
    }

    #[test]
    fn test_function_body_can_be_while_command_sequence() {
        let input = "foo() while false; do echo bad; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        assert_eq!(function.body[0].words, ["while", "false"]);
        assert_eq!(function.body[1].words, ["do", "echo", "bad"]);
        assert_eq!(function.body[2].words, ["done"]);
    }

    #[test]
    fn test_function_body_can_be_conditional_command() {
        let input = "foo() [[ $1 == a* && $2 -gt 1 ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        assert_eq!(
            function.body[0].words,
            ["[[", "$1", "==", "a*", "&&", "$2", "-gt", "1", "]]"]
        );
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
        assert_eq!(
            ast.commands[0].assignments.get("VAR"),
            Some(&"value".to_string())
        );
    }

    #[test]
    fn test_command_with_assignment() {
        let input = "X=5 echo hello";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].assignments.contains_key("X"));
    }

    #[test]
    fn test_escaped_equals_is_command_word_not_assignment() {
        let input = "foo\\=bar > out.txt";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].assignments.is_empty());
        assert_eq!(ast.commands[0].words, ["foo=bar"]);
        assert!(ast.commands[0].redirect_out.is_some());
    }
}

mod variable_tests {
    use super::*;

    #[test]
    fn test_variable_words_are_preserved_for_expansion() {
        let input = "echo alias: $?";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words, vec!["echo", "alias:", "$?"]);
    }

    #[test]
    fn test_braced_variable_can_be_command_word() {
        let input = "${THIS_SH} ./script";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words, vec!["${THIS_SH}", "./script"]);
    }
}

mod quote_removal {
    use super::*;

    #[test]
    fn test_single_quotes_removed() {
        let input = "echo 'hello world'";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words[1], "hello world");
    }

    #[test]
    fn test_double_quotes_removed() {
        let input = "echo \"hello world\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words[1], "hello world");
    }

    #[test]
    fn test_assignment_after_command_is_word() {
        let input = "alias foo='echo '";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words, vec!["alias", "foo=\x1cecho "]);
        assert!(ast.commands[0].assignments.is_empty());
    }

    #[test]
    fn test_declare_compound_assignment_preserves_quoted_word_boundaries() {
        let input = "declare -A assoc=(one \"two words\" three \"four words\")";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(
            ast.commands[0].words,
            vec![
                "declare",
                "-A",
                "assoc=\x1e(one \"two words\" three \"four words\")"
            ]
        );
    }

    #[test]
    fn test_compound_assignment_preserves_quoted_subscript_boundaries() {
        let input = "declare -A assoc=([\"two words\"]=\"value here\")";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(
            ast.commands[0].words,
            vec![
                "declare",
                "-A",
                "assoc=\x1e([\"two words\"]=\"value here\")"
            ]
        );
    }
}
