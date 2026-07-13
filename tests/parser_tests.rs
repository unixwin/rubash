//! Parser Tests - TDD for Bash Parser
//!
//! Run with: cargo test --test parser_tests

use rubash::lexer::tokenize;
use rubash::parser::{
    parse, CaseTerminator, CommandBodyKind, ConditionalExpressionKind, FunctionBodyKind, LoopKind,
    QuoteKind,
};

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
        assert_eq!(pipeline.operators, ["|"]);
        assert_eq!(pipeline.stages[0].words, ["ls"]);
        assert!(pipeline.stages[0].pipe.is_some());
        assert_eq!(pipeline.stages[1].words, ["grep", "foo"]);
    }

    #[test]
    fn test_stderr_pipeline_operator() {
        let input = "cmd |& grep err";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.operators, ["|&"]);
        assert_eq!(pipeline.stages[0].pipe, Some(2));
        assert_eq!(pipeline.stages[1].words, ["grep", "err"]);
    }

    #[test]
    fn test_time_prefix_wraps_pipeline_command() {
        let input = "time -p ! echo alpha | wc -l";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.prefix_words, ["-p", "!"]);
        assert!(time_command.posix_format);
        assert!(time_command.inverted);
        let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.operators, ["|"]);
        assert_eq!(pipeline.stages[0].words, ["echo", "alpha"]);
        assert_eq!(pipeline.stages[1].words, ["wc", "-l"]);
    }

    #[test]
    fn test_time_prefix_wraps_simple_command_in_and_or_list() {
        let input = "time -p ! false || echo fallback";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert_eq!(list.operators, ["||"]);
        let time_command = list.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.prefix_words, ["-p", "!"]);
        assert!(time_command.posix_format);
        assert!(time_command.inverted);
        assert_eq!(time_command.command.words, ["false"]);
        assert_eq!(list.commands[1].words, ["echo", "fallback"]);
    }

    #[test]
    fn test_inversion_wraps_time_simple_command() {
        let input = "! time -p false";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let inverted = ast.commands[0].inverted_command.as_ref().unwrap();
        let time_command = inverted.command.time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        assert!(!time_command.inverted);
        assert_eq!(time_command.command.words, ["false"]);
    }

    #[test]
    fn test_inversion_wraps_time_pipeline_command() {
        let input = "! time echo alpha | grep beta";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let inverted = ast.commands[0].inverted_command.as_ref().unwrap();
        let time_command = inverted.command.time_command.as_ref().unwrap();
        let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages[0].words, ["echo", "alpha"]);
        assert_eq!(pipeline.stages[1].words, ["grep", "beta"]);
    }

    #[test]
    fn test_time_prefix_wraps_non_initial_pipeline_stage() {
        let input = "printf alpha | time cat | wc -c";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 3);
        assert_eq!(pipeline.stages[0].words, ["printf", "alpha"]);
        let time_command = pipeline.stages[1].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.command.words, ["cat"]);
        assert_eq!(pipeline.stages[1].pipe, Some(1));
        assert_eq!(pipeline.stages[2].words, ["wc", "-c"]);
    }

    #[test]
    fn test_multiple_pipeline() {
        let input = "ls | grep foo | sort";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 3);
        assert_eq!(pipeline.operators, ["|", "|"]);
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
        assert_eq!(list.operators, ["&&"]);
        assert!(list.commands[0].pipeline_command.is_some());
        assert_eq!(list.commands[1].words, ["echo", "ok"]);
    }

    #[test]
    fn test_compound_command_pipeline_stage() {
        let input = "for value in a b; do echo $value; done | wc -l";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.operators, ["|"]);
        assert!(pipeline.stages[0].pipe.is_some());
        assert!(pipeline.stages[0].for_command.is_some());
        assert_eq!(pipeline.stages[1].words, ["wc", "-l"]);
    }

    #[test]
    fn test_and_or_list_command_preserves_mixed_connectors() {
        let input = "false || echo fallback && echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert_eq!(list.connectors, [false, true]);
        assert_eq!(list.operators, ["||", "&&"]);
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
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.prefix_words, ["-p"]);
        assert!(time_command.posix_format);
        assert!(!time_command.inverted);
        let for_command = time_command.command.for_command.as_ref().unwrap();
        assert_eq!(for_command.keyword, "for");
        assert_eq!(for_command.in_keyword.as_deref(), Some("in"));
        assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(for_command.end_keyword.as_deref(), Some("done"));
        assert_eq!(for_command.variable, "x");
        assert_eq!(for_command.words, ["a", "b"]);
        assert_eq!(for_command.list_terminator.as_deref(), Some(";"));
        assert_eq!(for_command.body_kind, CommandBodyKind::DoDone);
        assert_eq!(for_command.body_open_delimiter.as_deref(), Some("do"));
        assert_eq!(for_command.body_close_delimiter.as_deref(), Some("done"));
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_time_inversion_prefix_parses_for_command() {
        let input = "time -p ! for x in a; do echo $x; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.prefix_words, ["-p", "!"]);
        assert!(time_command.posix_format);
        assert!(time_command.inverted);
        assert!(time_command.command.for_command.is_some());
    }

    #[test]
    fn test_time_prefix_parses_arithmetic_command() {
        let input = "time -p (( 1 + 1 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.prefix_words, ["-p"]);
        assert!(time_command.posix_format);
        let arithmetic = time_command.command.arithmetic_command.as_ref().unwrap();
        assert_eq!(arithmetic.expression, "1 + 1");
    }

    #[test]
    fn test_time_prefix_parses_conditional_command() {
        let input = "time -p [[ $value == ok ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert_eq!(time_command.prefix_words, ["-p"]);
        assert!(time_command.posix_format);
        let conditional = time_command.command.conditional_command.as_ref().unwrap();
        assert_eq!(conditional.open_delimiter, "[[");
        assert_eq!(conditional.close_delimiter, "]]");
        assert_eq!(conditional.args, ["$value", "==", "ok", "]]"]);
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
        let brace_group = time_command.command.brace_group.as_ref().unwrap();
        assert_eq!(brace_group.open_delimiter, "{");
        assert_eq!(brace_group.close_delimiter, "}");
        let body = &brace_group.body;
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
        let brace_group = list.commands[0].brace_group.as_ref().unwrap();
        assert_eq!(brace_group.open_delimiter, "{");
        assert_eq!(brace_group.close_delimiter, "}");
        let body = &brace_group.body;
        assert_eq!(body[0].words, ["echo", "hi"]);
        assert_eq!(
            list.commands[0].redirect_out.as_ref().unwrap().target,
            "out"
        );
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_brace_group_command_consumes_pipe_stderr_operator() {
        let input = "{ echo hi; } |& grep hi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.operators, ["|&"]);
        assert_eq!(pipeline.stages[0].pipe, Some(2));
        assert!(pipeline.stages[0].brace_group.is_some());
        assert_eq!(pipeline.stages[1].words, ["grep", "hi"]);
    }

    #[test]
    fn test_time_prefix_parses_subshell_group() {
        let input = "time -p ( echo one; echo two )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert!(time_command.posix_format);
        let subshell = time_command.command.subshell_command.as_ref().unwrap();
        assert_eq!(subshell.open_delimiter, "(");
        assert_eq!(subshell.close_delimiter, ")");
        let body = &subshell.body;
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
        let subshell = list.commands[0].subshell_command.as_ref().unwrap();
        assert_eq!(subshell.open_delimiter, "(");
        assert_eq!(subshell.close_delimiter, ")");
        let body = &subshell.body;
        assert_eq!(body[0].words, ["echo", "hi"]);
        assert_eq!(
            list.commands[0].redirect_out.as_ref().unwrap().target,
            "out"
        );
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_subshell_command_keeps_case_pattern_parentheses() {
        let input = "( case beta in alpha) printf alpha ;; beta) printf beta ;; esac )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let subshell = ast.commands[0].subshell_command.as_ref().unwrap();

        assert_eq!(subshell.open_delimiter, "(");
        assert_eq!(subshell.close_delimiter, ")");
        assert_eq!(subshell.body.len(), 1);
        assert!(subshell.body[0].case_command.is_some());
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

mod command_body_kind_tests {
    use super::*;

    #[test]
    fn test_for_brace_body_records_body_kind() {
        let tokens = tokenize("for x in a; { echo $x; }");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();

        assert_eq!(for_command.keyword, "for");
        assert_eq!(for_command.in_keyword.as_deref(), Some("in"));
        assert_eq!(for_command.do_keyword, None);
        assert_eq!(for_command.end_keyword, None);
        assert_eq!(for_command.list_terminator.as_deref(), Some(";"));
        assert_eq!(for_command.body_kind, CommandBodyKind::BraceGroup);
        assert_eq!(for_command.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(for_command.body_close_delimiter.as_deref(), Some("}"));
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_select_body_kind_records_do_done_and_brace_group() {
        let do_done_tokens = tokenize("select x in a; do echo $x; done");
        let do_done_ast = parse(&do_done_tokens);
        let brace_tokens = tokenize("select y in b; { echo $y; }");
        let brace_ast = parse(&brace_tokens);
        let first = do_done_ast.commands[0].select_command.as_ref().unwrap();
        let second = brace_ast.commands[0].select_command.as_ref().unwrap();

        assert_eq!(first.body_kind, CommandBodyKind::DoDone);
        assert_eq!(first.keyword, "select");
        assert_eq!(first.in_keyword.as_deref(), Some("in"));
        assert_eq!(first.do_keyword.as_deref(), Some("do"));
        assert_eq!(first.end_keyword.as_deref(), Some("done"));
        assert_eq!(first.list_terminator.as_deref(), Some(";"));
        assert_eq!(first.body_open_delimiter.as_deref(), Some("do"));
        assert_eq!(first.body_close_delimiter.as_deref(), Some("done"));
        assert_eq!(first.body[0].words, ["echo", "$x"]);
        assert_eq!(second.body_kind, CommandBodyKind::BraceGroup);
        assert_eq!(second.keyword, "select");
        assert_eq!(second.in_keyword.as_deref(), Some("in"));
        assert_eq!(second.do_keyword, None);
        assert_eq!(second.end_keyword, None);
        assert_eq!(second.list_terminator.as_deref(), Some(";"));
        assert_eq!(second.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(second.body_close_delimiter.as_deref(), Some("}"));
        assert_eq!(second.body[0].words, ["echo", "$y"]);
    }

    #[test]
    fn test_loop_body_delimiters_record_do_done() {
        let while_tokens = tokenize("while false; do echo bad; done");
        let while_ast = parse(&while_tokens);
        let until_tokens = tokenize("until true; do echo ok; done");
        let until_ast = parse(&until_tokens);
        let while_command = while_ast.commands[0].loop_command.as_ref().unwrap();
        let until_command = until_ast.commands[0].loop_command.as_ref().unwrap();

        assert_eq!(while_command.kind, LoopKind::While);
        assert_eq!(while_command.keyword, "while");
        assert_eq!(while_command.do_keyword, "do");
        assert_eq!(while_command.end_keyword, "done");
        assert_eq!(while_command.body_open_delimiter, "do");
        assert_eq!(while_command.body_close_delimiter, "done");
        assert_eq!(while_command.condition_terminator.as_deref(), Some(";"));
        assert_eq!(while_command.condition[0].words, ["false"]);
        assert_eq!(while_command.body[0].words, ["echo", "bad"]);
        assert_eq!(until_command.kind, LoopKind::Until);
        assert_eq!(until_command.keyword, "until");
        assert_eq!(until_command.body_open_delimiter, "do");
        assert_eq!(until_command.body_close_delimiter, "done");
        assert_eq!(until_command.condition[0].words, ["true"]);
        assert_eq!(until_command.body[0].words, ["echo", "ok"]);
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
        assert_eq!(if_command.keyword, "if");
        assert_eq!(if_command.then_keyword, "then");
        assert_eq!(if_command.end_keyword, "fi");
        assert_eq!(if_command.condition_terminator.as_deref(), Some(";"));
        assert_eq!(if_command.condition[0].words, ["true"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
        assert!(if_command.elif_branches.is_empty());
        assert_eq!(if_command.else_keyword, None);
        assert!(if_command.else_body.is_none());
    }

    #[test]
    fn test_if_command_parses_elif_and_else() {
        let input = "if false; then echo no; elif true; then echo yes; else echo bad; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let if_command = ast.commands[0].if_command.as_ref().unwrap();
        assert_eq!(if_command.condition[0].words, ["false"]);
        assert_eq!(if_command.condition_terminator.as_deref(), Some(";"));
        assert_eq!(if_command.then_body[0].words, ["echo", "no"]);
        assert_eq!(if_command.elif_branches.len(), 1);
        assert_eq!(if_command.elif_branches[0].keyword, "elif");
        assert_eq!(if_command.elif_branches[0].then_keyword, "then");
        assert_eq!(
            if_command.elif_branches[0].condition_terminator.as_deref(),
            Some(";")
        );
        assert_eq!(if_command.elif_branches[0].condition[0].words, ["true"]);
        assert_eq!(if_command.elif_branches[0].body[0].words, ["echo", "yes"]);
        assert_eq!(if_command.else_keyword.as_deref(), Some("else"));
        assert_eq!(if_command.end_keyword, "fi");
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
        assert!(function.keyword);
        assert_eq!(function.keyword_text.as_deref(), Some("function"));
        assert!(!function.has_parentheses);
        assert_eq!(function.open_paren, None);
        assert_eq!(function.close_paren, None);
        assert_eq!(function.body_kind, FunctionBodyKind::BraceGroup);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(function.body_close_delimiter.as_deref(), Some("}"));
        assert!(function.body_start.is_some());
        assert!(function.body_end.is_some());
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
        assert!(function.keyword);
        assert_eq!(function.keyword_text.as_deref(), Some("function"));
        assert!(function.has_parentheses);
        assert_eq!(function.open_paren.as_deref(), Some("("));
        assert_eq!(function.close_paren.as_deref(), Some(")"));
        assert_eq!(function.body_kind, FunctionBodyKind::BraceGroup);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(function.body_close_delimiter.as_deref(), Some("}"));
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
        assert!(!function.keyword);
        assert_eq!(function.keyword_text, None);
        assert!(function.has_parentheses);
        assert_eq!(function.open_paren.as_deref(), Some("("));
        assert_eq!(function.close_paren.as_deref(), Some(")"));
        assert_eq!(function.body_kind, FunctionBodyKind::BraceGroup);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(function.body_close_delimiter.as_deref(), Some("}"));
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
    fn test_function_definition_consumes_and_or_connector() {
        let input = "foo() { echo hi; } && echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        assert_eq!(list.connectors, [true]);
        assert!(list.commands[0].function_command.is_some());
        assert_eq!(list.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_function_definition_consumes_pipe_operator() {
        let input = "foo() { echo hi; } | cat";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.operators, ["|"]);
        assert_eq!(pipeline.stages[0].pipe, Some(1));
        assert!(pipeline.stages[0].function_command.is_some());
        assert_eq!(pipeline.stages[1].words, ["cat"]);
    }

    #[test]
    fn test_compact_function_definition_consumes_trailing_redirects() {
        let input = "foo(){ echo hi; } > out; echo done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.body_kind, FunctionBodyKind::BraceGroup);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(function.body_close_delimiter.as_deref(), Some("}"));
        assert_eq!(ast.commands[0].redirect_out.as_ref().unwrap().target, "out");
        assert_eq!(ast.commands[1].words, ["echo", "done"]);
    }

    #[test]
    fn test_parenthesized_function_body_records_subshell_metadata() {
        let input = "foo() (echo hi)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(function.name, "foo");
        assert!(!function.keyword);
        assert!(function.has_parentheses);
        assert_eq!(function.body_kind, FunctionBodyKind::Subshell);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("("));
        assert_eq!(function.body_close_delimiter.as_deref(), Some(")"));
        assert!(function.body_start.is_some());
        assert!(function.body_end.is_some());
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_function_keyword_can_use_subshell_body_without_signature_parentheses() {
        let input = "function foo ( echo hi )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(function.name, "foo");
        assert!(function.keyword);
        assert!(!function.has_parentheses);
        assert_eq!(function.body_kind, FunctionBodyKind::Subshell);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("("));
        assert_eq!(function.body_close_delimiter.as_deref(), Some(")"));
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_parenthesized_function_body_keeps_case_pattern_parentheses() {
        let input = "foo() ( case beta in alpha) printf alpha ;; beta) printf beta ;; esac )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(function.body_kind, FunctionBodyKind::Subshell);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("("));
        assert_eq!(function.body_close_delimiter.as_deref(), Some(")"));
        assert_eq!(function.body.len(), 1);
        assert!(function.body[0].case_command.is_some());
    }

    #[test]
    fn test_function_keyword_name_can_look_like_assignment() {
        let input = "function foo=bar { echo hi; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo=bar");
        assert!(function.keyword);
        assert!(!function.has_parentheses);
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
        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(function.body_open_delimiter, None);
        assert_eq!(function.body_close_delimiter, None);
        assert_eq!(for_command.keyword, "for");
        assert_eq!(for_command.in_keyword.as_deref(), Some("in"));
        assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(for_command.end_keyword.as_deref(), Some("done"));
        assert_eq!(for_command.variable, "x");
        assert_eq!(for_command.words, ["a", "b"]);
        assert_eq!(for_command.body_kind, CommandBodyKind::DoDone);
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_function_body_can_be_arithmetic_command() {
        let input = "foo() (( 1 + 1 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(function.body_open_delimiter, None);
        assert_eq!(function.body_close_delimiter, None);
        let arithmetic = function.body[0].arithmetic_command.as_ref().unwrap();
        assert_eq!(arithmetic.open_delimiter, "((");
        assert_eq!(arithmetic.close_delimiter, "))");
        assert_eq!(arithmetic.expression, "1 + 1");
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
        assert_eq!(case_command.clauses[0].pattern_nodes[0].text, "a");
        assert!(!case_command.clauses[0].pattern_nodes[0].has_glob);
        assert_eq!(case_command.clauses[1].pattern_nodes[0].text, "*");
        assert!(case_command.clauses[1].pattern_nodes[0].has_glob);
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
        assert_eq!(function.body_kind, FunctionBodyKind::CommandSequence);
        assert_eq!(function.body_open_delimiter, None);
        assert_eq!(function.body_close_delimiter, None);
        assert!(!loop_command.until);
        assert_eq!(loop_command.kind, LoopKind::While);
        assert_eq!(loop_command.keyword, "while");
        assert_eq!(loop_command.do_keyword, "do");
        assert_eq!(loop_command.end_keyword, "done");
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
        assert_eq!(function.body_kind, FunctionBodyKind::CommandSequence);
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
        assert_eq!(conditional.open_delimiter, "[[");
        assert_eq!(conditional.close_delimiter, "]]");
        assert_eq!(
            conditional.args,
            ["$value", "==", "a*", "&&", "-n", "$other", "]]"]
        );
        assert_eq!(
            ast.commands[0].words,
            ["[[", "$value", "==", "a*", "&&", "-n", "$other", "]]"]
        );
        assert_eq!(
            conditional.expression.kind,
            ConditionalExpressionKind::Logical
        );
        assert_eq!(conditional.expression.open_delimiter, None);
        assert_eq!(conditional.expression.close_delimiter, None);
        assert_eq!(conditional.expression.operator.as_deref(), Some("&&"));
        assert_eq!(conditional.expression.children.len(), 2);
        assert_eq!(
            conditional.expression.children[0].kind,
            ConditionalExpressionKind::Binary
        );
        assert_eq!(
            conditional.expression.children[0].operator.as_deref(),
            Some("==")
        );
        assert_eq!(
            conditional.expression.children[0].operands,
            ["$value", "a*"]
        );
        assert_eq!(
            conditional.expression.children[1].kind,
            ConditionalExpressionKind::Unary
        );
        assert_eq!(
            conditional.expression.children[1].operator.as_deref(),
            Some("-n")
        );
        assert_eq!(conditional.expression.children[1].operands, ["$other"]);
    }

    #[test]
    fn test_conditional_command_records_group_and_negation_expression() {
        let input = "[[ ! ( -z $empty || $value =~ ^a ) ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();

        assert_eq!(conditional.open_delimiter, "[[");
        assert_eq!(conditional.close_delimiter, "]]");
        assert_eq!(
            conditional.expression.kind,
            ConditionalExpressionKind::Negation
        );
        let group = &conditional.expression.children[0];
        assert_eq!(group.kind, ConditionalExpressionKind::Group);
        assert_eq!(group.open_delimiter.as_deref(), Some("("));
        assert_eq!(group.close_delimiter.as_deref(), Some(")"));
        let logical = &group.children[0];
        assert_eq!(logical.kind, ConditionalExpressionKind::Logical);
        assert_eq!(logical.operator.as_deref(), Some("||"));
        assert_eq!(logical.children[0].kind, ConditionalExpressionKind::Unary);
        assert_eq!(logical.children[0].operator.as_deref(), Some("-z"));
        assert_eq!(logical.children[1].kind, ConditionalExpressionKind::Binary);
        assert_eq!(logical.children[1].operator.as_deref(), Some("=~"));
    }

    #[test]
    fn test_conditional_command_records_readonly_unary_expression() {
        let input = "[[ -R UID ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();

        assert_eq!(
            conditional.expression.kind,
            ConditionalExpressionKind::Unary
        );
        assert_eq!(conditional.expression.operator.as_deref(), Some("-R"));
        assert_eq!(conditional.expression.operands, ["UID"]);
    }

    #[test]
    fn test_conditional_command_records_file_ownership_unary_expressions() {
        let input = "[[ -G file || -N file ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();

        assert_eq!(
            conditional.expression.kind,
            ConditionalExpressionKind::Logical
        );
        assert_eq!(conditional.expression.operator.as_deref(), Some("||"));
        assert_eq!(
            conditional.expression.children[0].kind,
            ConditionalExpressionKind::Unary
        );
        assert_eq!(
            conditional.expression.children[0].operator.as_deref(),
            Some("-G")
        );
        assert_eq!(
            conditional.expression.children[1].kind,
            ConditionalExpressionKind::Unary
        );
        assert_eq!(
            conditional.expression.children[1].operator.as_deref(),
            Some("-N")
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

mod case_tests {
    use super::*;

    #[test]
    fn test_case_patterns_record_structured_metadata() {
        let input = "case $word in (x|@(foo|bar)|!(tmp)) echo hit ;& *) echo rest ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        assert_eq!(case_command.keyword, "case");
        assert_eq!(case_command.word, "$word");
        assert_eq!(case_command.in_keyword, "in");
        assert_eq!(case_command.end_keyword, "esac");
        assert_eq!(case_command.clauses.len(), 2);
        assert_eq!(
            case_command.clauses[0].terminator,
            CaseTerminator::FallThrough
        );
        assert_eq!(
            case_command.clauses[0].terminator_text.as_deref(),
            Some(";&")
        );
        assert_eq!(
            case_command.clauses[0].pattern_open_delimiter.as_deref(),
            Some("(")
        );
        assert_eq!(
            case_command.clauses[0].patterns,
            ["x", "@(foo|bar)", "!(tmp)"]
        );
        assert_eq!(case_command.clauses[0].pattern_separators, ["|", "|"]);
        assert_eq!(case_command.clauses[0].pattern_close_delimiter, ")");

        let patterns = &case_command.clauses[0].pattern_nodes;
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0].text, "x");
        assert_eq!(patterns[0].clause_index, 0);
        assert_eq!(patterns[0].pattern_index, 0);
        assert!(patterns[0].operators.is_empty());
        assert!(!patterns[0].has_glob);
        assert!(!patterns[0].has_extglob);
        assert_eq!(patterns[1].text, "@(foo|bar)");
        assert_eq!(patterns[1].operators, ["@(", "|"]);
        assert!(patterns[1].has_extglob);
        assert!(!patterns[1].negated_extglob);
        assert_eq!(patterns[2].text, "!(tmp)");
        assert_eq!(patterns[2].operators, ["!("]);
        assert!(patterns[2].has_extglob);
        assert!(patterns[2].negated_extglob);

        let fallback = &case_command.clauses[1].pattern_nodes[0];
        assert_eq!(fallback.text, "*");
        assert_eq!(fallback.clause_index, 1);
        assert_eq!(fallback.pattern_index, 0);
        assert_eq!(fallback.operators, ["*"]);
        assert!(fallback.has_glob);
        assert_eq!(case_command.clauses[1].pattern_open_delimiter, None);
        assert!(case_command.clauses[1].pattern_separators.is_empty());
        assert_eq!(case_command.clauses[1].pattern_close_delimiter, ")");
        assert_eq!(
            case_command.clauses[1].terminator_text.as_deref(),
            Some(";;")
        );
    }

    #[test]
    fn test_case_clause_records_absent_terminator() {
        let input = "case $word in x) echo hit esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let clause = &ast.commands[0].case_command.as_ref().unwrap().clauses[0];

        assert_eq!(clause.terminator, CaseTerminator::Break);
        assert_eq!(clause.terminator_text, None);
    }

    #[test]
    fn test_case_pattern_records_glob_operators() {
        let input = "case $word in src/[ab]??.rs) echo hit ;; **/*.sh) echo shell ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        let first = &case_command.clauses[0].pattern_nodes[0];
        assert_eq!(first.text, "src/[ab]??.rs");
        assert_eq!(first.operators, ["[ab]", "?", "?"]);
        assert!(first.has_glob);
        assert!(!first.has_extglob);

        let second = &case_command.clauses[1].pattern_nodes[0];
        assert_eq!(second.text, "**/*.sh");
        assert_eq!(second.operators, ["**", "*"]);
        assert!(second.has_glob);
    }

    #[test]
    fn test_case_body_keeps_nested_select_command() {
        let input =
            "case $word in x) select choice in a; do echo $choice; break; done <<< 1 ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        assert_eq!(case_command.clauses.len(), 1);
        assert!(case_command.clauses[0]
            .body
            .iter()
            .any(|command| command.select_command.is_some()));
        assert_eq!(
            case_command.clauses[0].terminator_text.as_deref(),
            Some(";;")
        );
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
        assert_eq!(arithmetic.open_delimiter, "((");
        assert_eq!(arithmetic.close_delimiter, "))");
        assert_eq!(arithmetic.expression, "n += 2");
        assert_eq!(arithmetic.variables, ["n"]);
        assert!(arithmetic.has_assignment);
        assert!(!arithmetic.has_comparison);
        assert!(!arithmetic.has_logical);
        assert!(!arithmetic.has_update);
        assert_eq!(arithmetic.operators.len(), 1);
        assert_eq!(arithmetic.operators[0].text, "+=");
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
        let arithmetic = list.commands[0].arithmetic_command.as_ref().unwrap();
        assert_eq!(arithmetic.variables, ["n"]);
        assert!(arithmetic.has_update);
        assert_eq!(arithmetic.operators[0].text, "++");
    }

    #[test]
    fn test_arithmetic_command_consumes_trailing_redirect() {
        let input = "(( n++ )) > out && echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let list = ast.commands[0].and_or_list.as_ref().unwrap();
        let arithmetic_command = &list.commands[0];

        assert_eq!(
            arithmetic_command
                .arithmetic_command
                .as_ref()
                .unwrap()
                .expression,
            "n++"
        );
        assert_eq!(
            arithmetic_command.redirect_out.as_ref().unwrap().target,
            "out"
        );
        assert_eq!(list.connectors, [true]);
        assert_eq!(list.commands[1].words, ["echo", "ok"]);
    }

    #[test]
    fn test_arithmetic_command_consumes_pipe_stderr_operator() {
        let input = "(( n++ )) |& wc -l";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.operators, ["|&"]);
        assert_eq!(pipeline.stages[0].pipe, Some(2));
        assert_eq!(
            pipeline.stages[0]
                .arithmetic_command
                .as_ref()
                .unwrap()
                .expression,
            "n++"
        );
        assert_eq!(pipeline.stages[1].words, ["wc", "-l"]);
    }

    #[test]
    fn test_arithmetic_command_records_operator_metadata() {
        let input = "(( (a += 1) > b && ! done ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let arithmetic = ast.commands[0].arithmetic_command.as_ref().unwrap();

        assert_eq!(arithmetic.open_delimiter, "((");
        assert_eq!(arithmetic.close_delimiter, "))");
        assert_eq!(arithmetic.variables, ["a", "b", "done"]);
        assert!(arithmetic.has_assignment);
        assert!(arithmetic.has_comparison);
        assert!(arithmetic.has_logical);
        assert!(!arithmetic.has_update);
        let operators = arithmetic
            .operators
            .iter()
            .map(|operator| operator.text.as_str())
            .collect::<Vec<_>>();
        assert!(operators.contains(&"+="));
        assert!(operators.contains(&">"));
        assert!(operators.contains(&"&&"));
        assert!(operators.contains(&"!"));
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
        assert_eq!(compound[0].operator, "=");
        assert!(!compound[0].append);
        assert_eq!(compound[0].word_index, None);
        assert_eq!(compound[0].elements.len(), 2);
        assert_eq!(compound[0].elements[0].subscript, None);
        assert_eq!(compound[0].elements[0].value, "one");
        assert_eq!(compound[0].elements[0].operator, None);
        assert_eq!(compound[0].elements[1].subscript, None);
        assert_eq!(compound[0].elements[1].value, "\"two words\"");
        assert_eq!(compound[0].elements[1].operator, None);
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
        assert_eq!(compound[0].operator, "+=");
        assert!(compound[0].append);
        assert_eq!(compound[0].word_index, None);
        assert_eq!(compound[0].elements.len(), 2);
        assert_eq!(compound[0].elements[0].value, "three");
        assert_eq!(compound[0].elements[1].value, "four");
    }

    #[test]
    fn test_compound_assignment_records_indexed_elements() {
        let input = "arr=([2]=two [name]+=more plain)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].name, "arr");
        assert_eq!(compound[0].elements.len(), 3);
        assert_eq!(compound[0].elements[0].subscript.as_deref(), Some("2"));
        assert_eq!(compound[0].elements[0].value, "two");
        assert_eq!(compound[0].elements[0].operator.as_deref(), Some("="));
        assert!(!compound[0].elements[0].append);
        assert_eq!(compound[0].elements[1].subscript.as_deref(), Some("name"));
        assert_eq!(compound[0].elements[1].value, "more");
        assert_eq!(compound[0].elements[1].operator.as_deref(), Some("+="));
        assert!(compound[0].elements[1].append);
        assert_eq!(compound[0].elements[2].subscript, None);
        assert_eq!(compound[0].elements[2].value, "plain");
        assert_eq!(compound[0].elements[2].operator, None);
    }

    #[test]
    fn test_array_element_assignment_records_structured_ast() {
        let input = "arr[0]=zero arr[i+1]+=more";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["arr[0]=zero", "arr[i+1]+=more"]);

        let elements = ast.commands[0].array_element_assignments.as_slice();
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].name, "arr");
        assert_eq!(elements[0].subscript, "0");
        assert_eq!(elements[0].value, "zero");
        assert_eq!(elements[0].operator, "=");
        assert!(!elements[0].append);
        assert_eq!(elements[0].word_index, Some(0));
        assert_eq!(elements[1].name, "arr");
        assert_eq!(elements[1].subscript, "i+1");
        assert_eq!(elements[1].value, "more");
        assert_eq!(elements[1].operator, "+=");
        assert!(elements[1].append);
        assert_eq!(elements[1].word_index, Some(1));
    }

    #[test]
    fn test_builtin_array_element_assignment_argument_records_word_index() {
        let input = "declare BASH_ARGV[1]=foo";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["declare", "BASH_ARGV[1]=foo"]);

        let elements = ast.commands[0].array_element_assignments.as_slice();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].name, "BASH_ARGV");
        assert_eq!(elements[0].subscript, "1");
        assert_eq!(elements[0].value, "foo");
        assert_eq!(elements[0].word_index, Some(1));
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

mod command_substitution_tests {
    use super::*;

    #[test]
    fn test_command_substitution_records_structured_ast_for_words() {
        let input = "echo $(printf hi) pre$(date)post `whoami`";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].words,
            ["echo", "$(printf hi)", "pre$(date)post", "`whoami`"]
        );

        let substitutions = ast.commands[0].command_substitutions.as_slice();
        assert_eq!(substitutions.len(), 3);
        assert_eq!(substitutions[0].text, "$(printf hi)");
        assert_eq!(substitutions[0].open_delimiter, "$(");
        assert_eq!(substitutions[0].operator, "$");
        assert_eq!(substitutions[0].close_delimiter, ")");
        assert_eq!(substitutions[0].source, "printf hi");
        assert_eq!(substitutions[0].commands.len(), 1);
        assert_eq!(substitutions[0].commands[0].words, ["printf", "hi"]);
        assert!(!substitutions[0].backtick);
        assert_eq!(substitutions[0].word_index, Some(1));
        assert_eq!(substitutions[0].assignment_name, None);
        assert_eq!(substitutions[1].text, "$(date)");
        assert_eq!(substitutions[1].open_delimiter, "$(");
        assert_eq!(substitutions[1].operator, "$");
        assert_eq!(substitutions[1].close_delimiter, ")");
        assert_eq!(substitutions[1].source, "date");
        assert_eq!(substitutions[1].commands[0].words, ["date"]);
        assert_eq!(substitutions[1].word_index, Some(2));
        assert_eq!(substitutions[2].text, "`whoami`");
        assert_eq!(substitutions[2].open_delimiter, "`");
        assert_eq!(substitutions[2].operator, "`");
        assert_eq!(substitutions[2].close_delimiter, "`");
        assert_eq!(substitutions[2].source, "whoami");
        assert_eq!(substitutions[2].commands[0].words, ["whoami"]);
        assert!(substitutions[2].backtick);
        assert_eq!(substitutions[2].word_index, Some(3));
    }

    #[test]
    fn test_assignment_command_substitution_records_structured_ast() {
        let input = "value=$(printf hi) echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].assignments.get("value").unwrap(),
            "$(printf hi)"
        );

        let substitutions = ast.commands[0].command_substitutions.as_slice();
        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].text, "$(printf hi)");
        assert_eq!(substitutions[0].open_delimiter, "$(");
        assert_eq!(substitutions[0].operator, "$");
        assert_eq!(substitutions[0].close_delimiter, ")");
        assert_eq!(substitutions[0].source, "printf hi");
        assert_eq!(substitutions[0].commands[0].words, ["printf", "hi"]);
        assert_eq!(substitutions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(substitutions[0].word_index, None);
    }

    #[test]
    fn test_command_substitution_records_nested_body_ast() {
        let input = "echo $(echo $(date); printf done)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let substitutions = ast.commands[0].command_substitutions.as_slice();

        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].source, "echo $(date); printf done");
        assert_eq!(substitutions[0].commands.len(), 2);
        assert_eq!(substitutions[0].commands[0].words, ["echo", "$(date)"]);
        assert_eq!(substitutions[0].commands[1].words, ["printf", "done"]);
        assert_eq!(
            substitutions[0].commands[0].command_substitutions[0].source,
            "date"
        );
    }

    #[test]
    fn test_command_substitution_keeps_case_pattern_parentheses() {
        let input = "echo $(case beta in alpha) printf alpha ;; beta) printf beta ;; esac)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let substitutions = ast.commands[0].command_substitutions.as_slice();

        assert_eq!(substitutions.len(), 1);
        assert_eq!(
            substitutions[0].source,
            "case beta in alpha) printf alpha ;; beta) printf beta ;; esac"
        );
        assert_eq!(substitutions[0].commands.len(), 1);
        assert!(substitutions[0].commands[0].case_command.is_some());
    }

    #[test]
    fn test_assignment_word_command_substitution_records_word_index() {
        let input = "echo value=`printf hi`";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["echo", "value=`printf hi`"]);

        let substitutions = ast.commands[0].command_substitutions.as_slice();
        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].text, "`printf hi`");
        assert_eq!(substitutions[0].open_delimiter, "`");
        assert_eq!(substitutions[0].operator, "`");
        assert_eq!(substitutions[0].close_delimiter, "`");
        assert_eq!(substitutions[0].source, "printf hi");
        assert!(substitutions[0].backtick);
        assert_eq!(substitutions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(substitutions[0].word_index, Some(1));
    }
}

mod arithmetic_expansion_tests {
    use super::*;

    #[test]
    fn test_arithmetic_expansion_records_structured_ast_for_words() {
        let input = "echo $((n + 1)) pre$((1+(2*3)))post";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].words,
            ["echo", "$((n + 1))", "pre$((1+(2*3)))post"]
        );

        let expansions = ast.commands[0].arithmetic_expansions.as_slice();
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].text, "$((n + 1))");
        assert_eq!(expansions[0].open_delimiter, "$((");
        assert_eq!(expansions[0].close_delimiter, "))");
        assert_eq!(expansions[0].expression, "n + 1");
        assert_eq!(expansions[0].variables, ["n"]);
        assert_eq!(expansions[0].operators[0].text, "+");
        assert!(!expansions[0].has_assignment);
        assert!(!expansions[0].has_comparison);
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[0].assignment_name, None);
        assert_eq!(expansions[1].text, "$((1+(2*3)))");
        assert_eq!(expansions[1].expression, "1+(2*3)");
        let operators = expansions[1]
            .operators
            .iter()
            .map(|operator| operator.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(operators, ["+", "*"]);
        assert_eq!(expansions[1].word_index, Some(2));
    }

    #[test]
    fn test_assignment_arithmetic_expansion_records_structured_ast() {
        let input = "value=$((2 + 3)) echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].assignments.get("value").unwrap(),
            "$((2 + 3))"
        );

        let expansions = ast.commands[0].arithmetic_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "$((2 + 3))");
        assert_eq!(expansions[0].open_delimiter, "$((");
        assert_eq!(expansions[0].close_delimiter, "))");
        assert_eq!(expansions[0].expression, "2 + 3");
        assert_eq!(expansions[0].operators[0].text, "+");
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(expansions[0].word_index, None);
    }

    #[test]
    fn test_arithmetic_expansion_records_operator_metadata() {
        let input = "echo value=$((count += 1)) ok=$((count > 0 && ! done))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let expansions = ast.commands[0].arithmetic_expansions.as_slice();

        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(expansions[0].variables, ["count"]);
        assert!(expansions[0].has_assignment);
        assert!(expansions[0]
            .operators
            .iter()
            .any(|operator| operator.text == "+="));
        assert_eq!(expansions[1].assignment_name.as_deref(), Some("ok"));
        assert_eq!(expansions[1].variables, ["count", "done"]);
        assert!(expansions[1].has_comparison);
        assert!(expansions[1].has_logical);
        assert!(!expansions[1].has_update);
    }

    #[test]
    fn test_assignment_word_arithmetic_expansion_records_word_index() {
        let input = "echo value=$((4 * 5))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["echo", "value=$((4 * 5))"]);

        let expansions = ast.commands[0].arithmetic_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "$((4 * 5))");
        assert_eq!(expansions[0].expression, "4 * 5");
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(expansions[0].word_index, Some(1));
    }
}

mod parameter_expansion_tests {
    use super::*;

    #[test]
    fn test_parameter_expansion_records_structured_ast_for_words() {
        let input = "echo $HOME ${USER:-guest} pre${name}post $? $10";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].words,
            [
                "echo",
                "$HOME",
                "${USER:-guest}",
                "pre${name}post",
                "$?",
                "$10"
            ]
        );

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 5);
        assert_eq!(expansions[0].text, "$HOME");
        assert_eq!(expansions[0].open_delimiter, "$");
        assert_eq!(expansions[0].close_delimiter, "");
        assert_eq!(expansions[0].parameter, "HOME");
        assert_eq!(expansions[0].name, "HOME");
        assert_eq!(expansions[0].operator, None);
        assert!(!expansions[0].operator_prefix);
        assert_eq!(expansions[0].word, None);
        assert!(!expansions[0].braced);
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[1].text, "${USER:-guest}");
        assert_eq!(expansions[1].open_delimiter, "${");
        assert_eq!(expansions[1].close_delimiter, "}");
        assert_eq!(expansions[1].parameter, "USER:-guest");
        assert_eq!(expansions[1].name, "USER");
        assert_eq!(expansions[1].operator.as_deref(), Some(":-"));
        assert!(!expansions[1].operator_prefix);
        assert_eq!(expansions[1].word.as_deref(), Some("guest"));
        assert!(expansions[1].braced);
        assert_eq!(expansions[1].word_index, Some(2));
        assert_eq!(expansions[2].text, "${name}");
        assert_eq!(expansions[2].open_delimiter, "${");
        assert_eq!(expansions[2].close_delimiter, "}");
        assert_eq!(expansions[2].parameter, "name");
        assert_eq!(expansions[2].name, "name");
        assert_eq!(expansions[2].operator, None);
        assert!(!expansions[2].operator_prefix);
        assert_eq!(expansions[2].word_index, Some(3));
        assert_eq!(expansions[3].text, "$?");
        assert_eq!(expansions[3].parameter, "?");
        assert_eq!(expansions[3].word_index, Some(4));
        assert_eq!(expansions[4].text, "$1");
        assert_eq!(expansions[4].parameter, "1");
        assert_eq!(expansions[4].word_index, Some(5));
    }

    #[test]
    fn test_assignment_parameter_expansion_records_structured_ast() {
        let input = "path=${HOME}/bin echo ok";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].assignments.get("path").unwrap(),
            "${HOME}/bin"
        );

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "${HOME}");
        assert_eq!(expansions[0].open_delimiter, "${");
        assert_eq!(expansions[0].close_delimiter, "}");
        assert_eq!(expansions[0].parameter, "HOME");
        assert!(expansions[0].braced);
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("path"));
        assert_eq!(expansions[0].word_index, None);
    }

    #[test]
    fn test_parameter_expansion_skips_command_and_arithmetic_sources() {
        let input = "echo $(echo $HOME) $((n + $delta)) $USER";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "$USER");
        assert_eq!(expansions[0].parameter, "USER");
        assert_eq!(expansions[0].word_index, Some(3));
    }

    #[test]
    fn test_assignment_word_parameter_expansion_records_word_index() {
        let input = "echo value=$USER";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["echo", "value=$USER"]);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "$USER");
        assert_eq!(expansions[0].open_delimiter, "$");
        assert_eq!(expansions[0].close_delimiter, "");
        assert_eq!(expansions[0].parameter, "USER");
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(expansions[0].word_index, Some(1));
    }

    #[test]
    fn test_parameter_expansion_records_operator_metadata() {
        let input = "echo ${file##*/} ${file%.rs} ${text//old/new} ${#array[@]} ${value:=fallback}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 5);
        assert_eq!(expansions[0].name, "file");
        assert_eq!(expansions[0].operator.as_deref(), Some("##"));
        assert!(!expansions[0].operator_prefix);
        assert_eq!(expansions[0].word.as_deref(), Some("*/"));
        assert_eq!(expansions[1].name, "file");
        assert_eq!(expansions[1].operator.as_deref(), Some("%"));
        assert!(!expansions[1].operator_prefix);
        assert_eq!(expansions[1].word.as_deref(), Some(".rs"));
        assert_eq!(expansions[2].name, "text");
        assert_eq!(expansions[2].operator.as_deref(), Some("//"));
        assert!(!expansions[2].operator_prefix);
        assert_eq!(expansions[2].word.as_deref(), Some("old/new"));
        assert_eq!(expansions[3].name, "array[@]");
        assert_eq!(expansions[3].operator.as_deref(), Some("#"));
        assert!(expansions[3].operator_prefix);
        assert_eq!(expansions[3].word, None);
        assert_eq!(expansions[4].name, "value");
        assert_eq!(expansions[4].operator.as_deref(), Some(":="));
        assert!(!expansions[4].operator_prefix);
        assert_eq!(expansions[4].word.as_deref(), Some("fallback"));
    }

    #[test]
    fn test_parameter_expansion_records_prefix_operator_shape() {
        let input = "echo ${!name} ${array#prefix}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].name, "name");
        assert_eq!(expansions[0].operator.as_deref(), Some("!"));
        assert!(expansions[0].operator_prefix);
        assert_eq!(expansions[0].word, None);
        assert_eq!(expansions[1].name, "array");
        assert_eq!(expansions[1].operator.as_deref(), Some("#"));
        assert!(!expansions[1].operator_prefix);
        assert_eq!(expansions[1].word.as_deref(), Some("prefix"));
    }
}

mod brace_expansion_tests {
    use super::*;

    #[test]
    fn test_brace_expansion_records_structured_ast_for_words() {
        let input = "echo {a,b} pre{1..3}post ${not_brace}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].words,
            ["echo", "{a,b}", "pre{1..3}post", "${not_brace}"]
        );

        let expansions = ast.commands[0].brace_expansions.as_slice();
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].text, "{a,b}");
        assert_eq!(expansions[0].open_delimiter, "{");
        assert_eq!(expansions[0].close_delimiter, "}");
        assert_eq!(expansions[0].body, "a,b");
        assert_eq!(expansions[0].operators, [","]);
        assert!(!expansions[0].range);
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[0].assignment_name, None);
        assert_eq!(expansions[1].text, "{1..3}");
        assert_eq!(expansions[1].open_delimiter, "{");
        assert_eq!(expansions[1].close_delimiter, "}");
        assert_eq!(expansions[1].body, "1..3");
        assert_eq!(expansions[1].operators, [".."]);
        assert!(expansions[1].range);
        assert_eq!(expansions[1].word_index, Some(2));
    }

    #[test]
    fn test_brace_expansion_records_repeated_list_operators() {
        let input = "echo {a,b,c}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].brace_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "{a,b,c}");
        assert_eq!(expansions[0].operators, [",", ","]);
        assert!(!expansions[0].range);
    }

    #[test]
    fn test_brace_expansion_skips_command_substitution_source() {
        let input = "echo $(echo {a,b}) {x,y}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].brace_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "{x,y}");
        assert_eq!(expansions[0].body, "x,y");
        assert_eq!(expansions[0].word_index, Some(2));
    }

    #[test]
    fn test_assignment_word_brace_expansion_records_word_index() {
        let input = "echo value={left,right}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["echo", "value={left,right}"]);

        let expansions = ast.commands[0].brace_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "{left,right}");
        assert_eq!(expansions[0].open_delimiter, "{");
        assert_eq!(expansions[0].close_delimiter, "}");
        assert_eq!(expansions[0].body, "left,right");
        assert_eq!(expansions[0].operators, [","]);
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("value"));
        assert_eq!(expansions[0].word_index, Some(1));
    }
}

mod extglob_pattern_tests {
    use super::*;

    #[test]
    fn test_extglob_pattern_records_structured_ast_for_words() {
        let input = "echo @(alpha|beta) file!(.tmp) nested@(a|+(b|c))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(
            ast.commands[0].words,
            ["echo", "@(alpha|beta)", "file!(.tmp)", "nested@(a|+(b|c))"]
        );

        let patterns = ast.commands[0].extglob_patterns.as_slice();
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0].text, "@(alpha|beta)");
        assert_eq!(patterns[0].open_delimiter, "@(");
        assert_eq!(patterns[0].close_delimiter, ")");
        assert_eq!(patterns[0].operator, '@');
        assert_eq!(patterns[0].pattern, "alpha|beta");
        assert_eq!(patterns[0].operators, ["|"]);
        assert_eq!(patterns[0].alternatives, ["alpha", "beta"]);
        assert_eq!(patterns[0].word_index, Some(1));
        assert_eq!(patterns[1].text, "!(.tmp)");
        assert_eq!(patterns[1].open_delimiter, "!(");
        assert_eq!(patterns[1].close_delimiter, ")");
        assert_eq!(patterns[1].operator, '!');
        assert!(patterns[1].operators.is_empty());
        assert_eq!(patterns[1].alternatives, [".tmp"]);
        assert_eq!(patterns[1].word_index, Some(2));
        assert_eq!(patterns[2].text, "@(a|+(b|c))");
        assert_eq!(patterns[2].pattern, "a|+(b|c)");
        assert_eq!(patterns[2].operators, ["|"]);
        assert_eq!(patterns[2].alternatives, ["a", "+(b|c)"]);
        assert_eq!(patterns[2].word_index, Some(3));
    }

    #[test]
    fn test_extglob_pattern_records_repeated_alternative_operators() {
        let input = "echo @(a|b|c) @(a[|]b|c)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].extglob_patterns.as_slice();
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0].text, "@(a|b|c)");
        assert_eq!(patterns[0].operators, ["|", "|"]);
        assert_eq!(patterns[0].alternatives, ["a", "b", "c"]);
        assert_eq!(patterns[1].text, "@(a[|]b|c)");
        assert_eq!(patterns[1].operators, ["|"]);
        assert_eq!(patterns[1].alternatives, ["a[|]b", "c"]);
    }

    #[test]
    fn test_extglob_pattern_skips_command_substitution_source() {
        let input = "echo $(echo @(hidden|source)) ?(visible)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].extglob_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "?(visible)");
        assert_eq!(patterns[0].operator, '?');
        assert_eq!(patterns[0].word_index, Some(2));
    }

    #[test]
    fn test_assignment_word_extglob_pattern_records_word_index() {
        let input = "echo pattern=*(src|tests)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, ["echo", "pattern=*(src|tests)"]);

        let patterns = ast.commands[0].extglob_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "*(src|tests)");
        assert_eq!(patterns[0].open_delimiter, "*(");
        assert_eq!(patterns[0].close_delimiter, ")");
        assert_eq!(patterns[0].operator, '*');
        assert_eq!(patterns[0].pattern, "src|tests");
        assert_eq!(patterns[0].operators, ["|"]);
        assert_eq!(patterns[0].assignment_name.as_deref(), Some("pattern"));
        assert_eq!(patterns[0].word_index, Some(1));
    }
}

mod tilde_expansion_tests {
    use super::*;

    #[test]
    fn test_tilde_expansion_records_structured_ast_for_words() {
        let input = "echo ~ ~/src ~+ ~- ~user/bin literal~";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].tilde_expansions.as_slice();
        assert_eq!(expansions.len(), 5);
        assert_eq!(expansions[0].text, "~");
        assert_eq!(expansions[0].open_delimiter, "~");
        assert_eq!(expansions[0].close_delimiter, "");
        assert_eq!(expansions[0].prefix, "~");
        assert_eq!(expansions[0].suffix, "");
        assert_eq!(expansions[0].word_index, Some(1));
        assert!(!expansions[0].after_colon);
        assert_eq!(expansions[1].text, "~/src");
        assert_eq!(expansions[1].prefix, "~");
        assert_eq!(expansions[1].suffix, "/src");
        assert_eq!(expansions[1].word_index, Some(2));
        assert_eq!(expansions[2].prefix, "~+");
        assert_eq!(expansions[2].open_delimiter, "~");
        assert_eq!(expansions[2].close_delimiter, "");
        assert_eq!(expansions[2].word_index, Some(3));
        assert_eq!(expansions[3].prefix, "~-");
        assert_eq!(expansions[3].word_index, Some(4));
        assert_eq!(expansions[4].text, "~user/bin");
        assert_eq!(expansions[4].prefix, "~user");
        assert_eq!(expansions[4].suffix, "/bin");
        assert_eq!(expansions[4].word_index, Some(5));
    }

    #[test]
    fn test_assignment_tilde_expansion_records_colon_segments() {
        let input = "PATH=~/bin:~+/sbin echo target=~-/tmp";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].tilde_expansions.as_slice();
        assert_eq!(expansions.len(), 3);
        assert_eq!(expansions[0].assignment_name.as_deref(), Some("PATH"));
        assert_eq!(expansions[0].text, "~/bin");
        assert_eq!(expansions[0].open_delimiter, "~");
        assert_eq!(expansions[0].close_delimiter, "");
        assert_eq!(expansions[0].prefix, "~");
        assert_eq!(expansions[0].suffix, "/bin");
        assert_eq!(expansions[0].word_index, None);
        assert!(!expansions[0].after_colon);
        assert_eq!(expansions[1].assignment_name.as_deref(), Some("PATH"));
        assert_eq!(expansions[1].text, "~+/sbin");
        assert_eq!(expansions[1].prefix, "~+");
        assert_eq!(expansions[1].suffix, "/sbin");
        assert_eq!(expansions[1].word_index, None);
        assert!(expansions[1].after_colon);
        assert_eq!(expansions[2].assignment_name.as_deref(), Some("target"));
        assert_eq!(expansions[2].text, "~-/tmp");
        assert_eq!(expansions[2].prefix, "~-");
        assert_eq!(expansions[2].suffix, "/tmp");
        assert_eq!(expansions[2].word_index, Some(1));
    }

    #[test]
    fn test_quoted_assignment_tilde_is_not_recorded() {
        let input = "echo \"target=~/tmp\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert!(ast.commands[0].tilde_expansions.is_empty());
    }
}

mod pathname_pattern_tests {
    use super::*;

    #[test]
    fn test_pathname_patterns_record_structured_ast_for_words() {
        let input = "echo *.rs src/[mp]*.rs docs/??.md src/**/mod.rs literal";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].pathname_patterns.as_slice();
        assert_eq!(patterns.len(), 4);
        assert_eq!(patterns[0].text, "*.rs");
        assert_eq!(patterns[0].operators, ["*"]);
        assert!(patterns[0].has_star);
        assert!(!patterns[0].has_question);
        assert!(!patterns[0].has_bracket);
        assert!(!patterns[0].globstar);
        assert_eq!(patterns[0].word_index, Some(1));
        assert_eq!(patterns[1].text, "src/[mp]*.rs");
        assert_eq!(patterns[1].operators, ["[mp]", "*"]);
        assert!(patterns[1].has_star);
        assert!(patterns[1].has_bracket);
        assert_eq!(patterns[1].word_index, Some(2));
        assert_eq!(patterns[2].text, "docs/??.md");
        assert_eq!(patterns[2].operators, ["?", "?"]);
        assert!(patterns[2].has_question);
        assert_eq!(patterns[2].word_index, Some(3));
        assert_eq!(patterns[3].text, "src/**/mod.rs");
        assert_eq!(patterns[3].operators, ["**"]);
        assert!(patterns[3].has_star);
        assert!(patterns[3].globstar);
        assert_eq!(patterns[3].word_index, Some(4));
    }

    #[test]
    fn test_pathname_pattern_skips_command_substitution_source() {
        let input = "echo $(echo *.rs) visible?.rs";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].pathname_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "visible?.rs");
        assert!(patterns[0].has_question);
        assert_eq!(patterns[0].word_index, Some(2));
    }

    #[test]
    fn test_assignment_like_word_does_not_record_pathname_pattern() {
        let input = "echo target=*.rs plain*.rs";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].pathname_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "plain*.rs");
        assert_eq!(patterns[0].word_index, Some(2));
    }
}

mod word_quote_tests {
    use super::*;

    #[test]
    fn test_word_quotes_record_structured_ast_for_words() {
        let input = "printf 'one two' \"three $HOME\" $'line\\n' $\"locale\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let quotes = ast.commands[0].word_quotes.as_slice();
        assert_eq!(quotes.len(), 4);
        assert_eq!(quotes[0].text, "'one two'");
        assert_eq!(quotes[0].open_delimiter, "'");
        assert_eq!(quotes[0].body, "one two");
        assert_eq!(quotes[0].close_delimiter, "'");
        assert_eq!(quotes[0].kind, QuoteKind::Single);
        assert_eq!(quotes[0].word_index, Some(1));
        assert_eq!(quotes[1].text, "\"three $HOME\"");
        assert_eq!(quotes[1].open_delimiter, "\"");
        assert_eq!(quotes[1].body, "three $HOME");
        assert_eq!(quotes[1].close_delimiter, "\"");
        assert_eq!(quotes[1].kind, QuoteKind::Double);
        assert_eq!(quotes[1].word_index, Some(2));
        assert_eq!(quotes[2].text, "$'line\\n'");
        assert_eq!(quotes[2].open_delimiter, "$'");
        assert_eq!(quotes[2].body, "line\\n");
        assert_eq!(quotes[2].close_delimiter, "'");
        assert_eq!(quotes[2].kind, QuoteKind::AnsiC);
        assert_eq!(quotes[2].word_index, Some(3));
        assert_eq!(quotes[3].text, "$\"locale\"");
        assert_eq!(quotes[3].open_delimiter, "$\"");
        assert_eq!(quotes[3].body, "locale");
        assert_eq!(quotes[3].close_delimiter, "\"");
        assert_eq!(quotes[3].kind, QuoteKind::Locale);
        assert_eq!(quotes[3].word_index, Some(4));
    }

    #[test]
    fn test_assignment_word_quotes_record_assignment_metadata() {
        let input = "name='value one' echo target=\"two words\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let quotes = ast.commands[0].word_quotes.as_slice();
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].text, "'value one'");
        assert_eq!(quotes[0].open_delimiter, "'");
        assert_eq!(quotes[0].body, "value one");
        assert_eq!(quotes[0].close_delimiter, "'");
        assert_eq!(quotes[0].kind, QuoteKind::Single);
        assert_eq!(quotes[0].assignment_name.as_deref(), Some("name"));
        assert_eq!(quotes[0].word_index, None);
        assert_eq!(quotes[1].text, "\"two words\"");
        assert_eq!(quotes[1].open_delimiter, "\"");
        assert_eq!(quotes[1].body, "two words");
        assert_eq!(quotes[1].close_delimiter, "\"");
        assert_eq!(quotes[1].kind, QuoteKind::Double);
        assert_eq!(quotes[1].assignment_name.as_deref(), Some("target"));
        assert_eq!(quotes[1].word_index, Some(1));
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
        assert_eq!(background.operator, "&");
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
        assert_eq!(background.operator, "&");
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
        assert_eq!(inverted.operator, "!");
        assert_eq!(inverted.command.words, ["false"]);
    }

    #[test]
    fn test_inverted_command_wraps_pipeline() {
        let input = "! false | true";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let inverted = ast.commands[0].inverted_command.as_ref().unwrap();
        assert_eq!(inverted.operator, "!");
        let pipeline = inverted.command.pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.stages[0].words, ["false"]);
        assert_eq!(pipeline.stages[1].words, ["true"]);
    }

    #[test]
    fn test_inverted_command_wraps_compound_command() {
        let input = "! for value in one; do false; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let inverted = ast.commands[0].inverted_command.as_ref().unwrap();
        assert_eq!(inverted.operator, "!");
        let for_command = inverted.command.for_command.as_ref().unwrap();
        assert_eq!(for_command.keyword, "for");
        assert_eq!(for_command.variable, "value");
        assert_eq!(for_command.words, ["one"]);
        assert_eq!(for_command.body[0].words, ["false"]);
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
