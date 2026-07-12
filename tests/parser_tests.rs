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
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.stages[0].words, ["ls"]);
        assert!(pipeline.stages[0].pipe.is_some());
        assert_eq!(pipeline.stages[1].words, ["grep", "foo"]);
    }

    #[test]
    fn test_multiple_pipeline() {
        let input = "ls | grep foo | sort";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 3);
        assert_eq!(pipeline.stages[2].words, ["sort"]);
    }

    #[test]
    fn test_pipeline_command_preserves_connector() {
        let input = "printf a | grep a && echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert_eq!(list.connectors, [true]);
        assert!(list.commands[0].pipeline_command.is_some());
        assert_eq!(list.commands[1].words, ["echo", "ok"]);
    }

    #[test]
    fn test_and_or_list_command_preserves_mixed_connectors() {
        let input = "false || echo fallback && echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert_eq!(list.connectors, [false, true]);
        assert_eq!(list.commands[0].words, ["false"]);
        assert_eq!(list.commands[1].words, ["echo", "fallback"]);
        assert_eq!(list.commands[2].words, ["echo", "done"]);
    }

    #[test]
    fn test_time_prefix_parses_for_command() {
        let input = "time -p for x in a b; do echo $x; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        assert!(!time_command.inverted);
        let for_command = time_command.command.for_command.as_ref().unwrap();
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
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        assert!(time_command.inverted);
        assert!(time_command.command.for_command.is_some());
    }

    #[test]
    fn test_time_prefix_parses_if_command_sequence() {
        let input = "time -p if true; then echo yes; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        let if_command = time_command.command.if_command.as_ref().unwrap();
        assert_eq!(if_command.condition[0].words, ["true"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
    }

    #[test]
    fn test_time_prefix_parses_brace_group() {
        let input = "time -p { echo one; echo two; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        let body = &time_command.command.brace_group.as_ref().unwrap().body;
        assert_eq!(body[0].words, ["echo", "one"]);
        assert_eq!(body[1].words, ["echo", "two"]);
    }

    #[test]
    fn test_brace_group_command_consumes_redirect_and_connector() {
        let input = "{ echo hi; } > out && echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        let body = &list.commands[0].brace_group.as_ref().unwrap().body;
        assert_eq!(body[0].words, ["echo", "hi"]);
        assert_eq!(
            list.commands[0].redirect_out.as_ref().unwrap().target,
            "out"
        );
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_time_prefix_parses_subshell_group() {
        let input = "time -p ( echo one; echo two )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        let body = &time_command.command.subshell_command.as_ref().unwrap().body;
        assert_eq!(body[0].words, ["echo", "one"]);
        assert_eq!(body[1].words, ["echo", "two"]);
    }

    #[test]
    fn test_subshell_command_consumes_redirect_and_connector() {
        let input = "( echo hi ) > out && echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        let body = &list.commands[0].subshell_command.as_ref().unwrap().body;
        assert_eq!(body[0].words, ["echo", "hi"]);
        assert_eq!(
            list.commands[0].redirect_out.as_ref().unwrap().target,
            "out"
        );
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "done"]);
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
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert!(list.commands[0].conditional_command.is_some());
        assert!(list.commands[0].redirect_out.is_some());
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "ok"]);
    }
}

mod arithmetic_command_tests {
    use super::*;

    #[test]
    fn test_arithmetic_command_parses_expression() {
        let input = "(( n += 2 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let arithmetic = ast.commands[0].arithmetic_command.as_ref().unwrap();
        assert_eq!(arithmetic.expression, "n += 2");
        assert_eq!(ast.commands[0].words, ["((", "n += 2", "))"]);
    }

    #[test]
    fn test_arithmetic_command_preserves_and_or_connector() {
        let input = "(( n++ )) && echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert_eq!(
            list.commands[0]
                .arithmetic_command
                .as_ref()
                .unwrap()
                .expression,
            "n++"
        );
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "ok"]);
    }

    #[test]
    fn test_arithmetic_and_or_list_skips_connector_newline() {
        let input =
            "if (( integerPart2 == 0 )) &&\n  (( fractionalPart2 == 0 ))\nthen echo bad; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let if_command = ast.commands[0].if_command.as_ref().unwrap();
        assert_eq!(if_command.condition.len(), 1);
        let list = if_command.condition[0].and_or_list.as_ref().unwrap();
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands.len(), 2);
        assert_eq!(
            list.commands[0]
                .arithmetic_command
                .as_ref()
                .unwrap()
                .expression,
            "integerPart2 == 0"
        );
        assert_eq!(
            list.commands[1]
                .arithmetic_command
                .as_ref()
                .unwrap()
                .expression,
            "fractionalPart2 == 0"
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
    fn test_compound_assignment_records_structured_ast() {
        let input = "arr=(one \"two words\")";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].assignments.get("arr").unwrap(),
            "\x1e(one \"two words\")"
        );

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].name, "arr");
        assert_eq!(compound[0].value, "(one \"two words\")");
        assert!(!compound[0].append);
        assert_eq!(compound[0].word_index, None);
    }

    #[test]
    fn test_compound_append_assignment_records_structured_ast() {
        let input = "arr+=(three four)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].assignments.get("arr+").unwrap(),
            "\x1e(three four)"
        );

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].name, "arr");
        assert_eq!(compound[0].value, "(three four)");
        assert!(compound[0].append);
        assert_eq!(compound[0].word_index, None);
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

mod background_tests {
    use super::*;

    #[test]
    fn test_background_command_wraps_simple_command() {
        let input = "false & echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        let background = ast.commands[0].background_command.as_ref().unwrap();
        assert_eq!(background.command.words, ["false"]);
        assert_eq!(ast.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_background_command_wraps_pipeline() {
        let input = "printf hi | cat & echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        let background = ast.commands[0].background_command.as_ref().unwrap();
        let pipeline = background.command.pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(ast.commands[1].words, ["echo", "done"]);
    }
}

mod inverted_tests {
    use super::*;

    #[test]
    fn test_inverted_command_wraps_simple_command() {
        let input = "! false";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let inverted = ast.commands[0].inverted_command.as_ref().unwrap();
        assert_eq!(inverted.command.words, ["false"]);
    }

    #[test]
    fn test_inverted_command_wraps_pipeline() {
        let input = "! false | true";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let inverted = ast.commands[0].inverted_command.as_ref().unwrap();
        let pipeline = inverted.command.pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.stages[0].words, ["false"]);
        assert_eq!(pipeline.stages[1].words, ["true"]);
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
        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].name, "assoc");
        assert_eq!(
            compound[0].value,
            "(one \"two words\" three \"four words\")"
        );
        assert!(!compound[0].append);
        assert_eq!(compound[0].word_index, Some(2));
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
