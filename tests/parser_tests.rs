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

    #[test]
    fn test_time_prefix_parses_for_command() {
        let input = "time -p for x in a b; do echo $x; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["time", "-p"]);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();
        assert_eq!(for_command.variable, "x");
        assert_eq!(for_command.words, ["a", "b"]);
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_time_inversion_prefix_parses_for_command() {
        let input = "time -p ! for x in a; do echo $x; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["time", "-p", "!"]);
        assert!(ast.commands[0].for_command.is_some());
    }

    #[test]
    fn test_time_prefix_parses_if_command_sequence() {
        let input = "time -p if true; then echo yes; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 3);
        assert_eq!(ast.commands[0].words, ["time", "-p", "if", "true"]);
        assert_eq!(ast.commands[1].words, ["then", "echo", "yes"]);
        assert_eq!(ast.commands[2].words, ["fi"]);
    }

    #[test]
    fn test_time_prefix_parses_brace_group() {
        let input = "time -p { echo one; echo two; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["time", "-p"]);
        let body = ast.commands[0].brace_group.as_ref().unwrap();
        assert_eq!(body[0].words, ["echo", "one"]);
        assert_eq!(body[1].words, ["echo", "two"]);
    }

    #[test]
    fn test_time_prefix_parses_subshell_group() {
        let input = "time -p ( echo one; echo two )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["time", "-p"]);
        let body = ast.commands[0].brace_group.as_ref().unwrap();
        assert_eq!(body[0].words, ["echo", "one"]);
        assert!(body[0].subshell);
        assert_eq!(body[1].words, ["echo", "two"]);
        assert!(body[1].subshell_end);
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

mod if_tests {
    use super::*;

    #[test]
    fn test_if_command_parses_then_body() {
        let input = "if true; then echo yes; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let if_command = ast.commands[0].if_command.as_ref().unwrap();
        assert_eq!(if_command.condition[0].words, ["true"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
        assert!(if_command.elif_branches.is_empty());
        assert!(if_command.else_body.is_none());
    }

    #[test]
    fn test_if_command_parses_elif_and_else() {
        let input = "if false; then echo no; elif true; then echo yes; else echo bad; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let if_command = ast.commands[0].if_command.as_ref().unwrap();
        assert_eq!(if_command.condition[0].words, ["false"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "no"]);
        assert_eq!(if_command.elif_branches.len(), 1);
        assert_eq!(if_command.elif_branches[0].condition[0].words, ["true"]);
        assert_eq!(if_command.elif_branches[0].body[0].words, ["echo", "yes"]);
        assert_eq!(
            if_command.else_body.as_ref().unwrap()[0].words,
            ["echo", "bad"]
        );
    }

    #[test]
    fn test_if_command_consumes_trailing_redirects() {
        let input = "if true; then echo yes; fi > out";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert!(ast.commands[0].if_command.is_some());
        assert_eq!(ast.commands[0].redirect_out.as_ref().unwrap().target, "out");
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
        let if_command = function.body[0].if_command.as_ref().unwrap();
        assert_eq!(if_command.condition[0].words, ["true"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
        assert_eq!(
            if_command.else_body.as_ref().unwrap()[0].words,
            ["echo", "no"]
        );
    }

    #[test]
    fn test_function_body_can_be_while_command_sequence() {
        let input = "foo() while false; do echo bad; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        let loop_command = function.body[0].loop_command.as_ref().unwrap();
        assert!(!loop_command.until);
        assert_eq!(loop_command.condition[0].words, ["false"]);
        assert_eq!(loop_command.body[0].words, ["echo", "bad"]);
    }

    #[test]
    fn test_function_body_can_be_conditional_command() {
        let input = "foo() [[ $1 == a* && $2 -gt 1 ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        let conditional = function.body[0].conditional_command.as_ref().unwrap();
        assert_eq!(
            conditional.args,
            ["$1", "==", "a*", "&&", "$2", "-gt", "1", "]]"]
        );
        assert_eq!(
            function.body[0].words,
            ["[[", "$1", "==", "a*", "&&", "$2", "-gt", "1", "]]"]
        );
    }
}

mod conditional_tests {
    use super::*;

    #[test]
    fn test_conditional_command_parses_args() {
        let input = "[[ $value == a* && -n $other ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();
        assert_eq!(
            conditional.args,
            ["$value", "==", "a*", "&&", "-n", "$other", "]]"]
        );
        assert_eq!(
            ast.commands[0].words,
            ["[[", "$value", "==", "a*", "&&", "-n", "$other", "]]"]
        );
    }

    #[test]
    fn test_conditional_command_consumes_trailing_redirects() {
        let input = "[[ -n $value ]] > out && echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].conditional_command.is_some());
        assert!(ast.commands[0].redirect_out.is_some());
        assert_eq!(ast.commands[0].and_or, Some(true));
        assert_eq!(ast.commands[1].words, ["echo", "ok"]);
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
