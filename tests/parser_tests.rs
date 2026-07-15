//! Parser Tests - TDD for Bash Parser
//!
//! Run with: cargo test --test parser_tests

use rubash::lexer::tokenize;
use rubash::parser::{
    parse, CaseTerminator, CommandBodyKind, ConditionalExpressionKind, ConditionalPatternKind,
    FunctionBodyKind, LoopKind, QuoteKind,
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
        assert_eq!(pipeline.operator_metadata.len(), 1);
        assert_eq!(pipeline.operator_metadata[0].word_index, 0);
        assert_eq!(pipeline.operator_metadata[0].value, "|");
        assert_eq!(pipeline.operator_metadata[0].raw, "|");
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
        assert_eq!(pipeline.operator_metadata[0].value, "|&");
        assert_eq!(pipeline.operator_metadata[0].raw, "|&");
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
        assert_eq!(time_command.keyword_metadata.value, "time");
        assert_eq!(time_command.keyword_metadata.raw, "time");
        assert_eq!(time_command.prefix_words, ["-p", "!"]);
        assert_eq!(time_command.prefix_word_metadata.len(), 2);
        assert_eq!(time_command.prefix_word_metadata[0].word_index, 0);
        assert_eq!(time_command.prefix_word_metadata[0].value, "-p");
        assert_eq!(time_command.prefix_word_metadata[0].raw, "-p");
        assert_eq!(time_command.prefix_word_metadata[1].word_index, 1);
        assert_eq!(time_command.prefix_word_metadata[1].value, "!");
        assert_eq!(time_command.prefix_word_metadata[1].raw, "!");
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
        assert_eq!(time_command.keyword_metadata.value, "time");
        assert_eq!(time_command.prefix_words, ["-p", "!"]);
        assert!(time_command.posix_format);
        assert!(time_command.inverted);
        assert_eq!(time_command.command.words, ["false"]);
        assert_eq!(list.commands[1].words, ["echo", "fallback"]);
    }

    #[test]
    fn test_time_prefix_wraps_null_command() {
        let tokens = tokenize("time");
        let ast = parse(&tokens);

        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword, "time");
        assert!(time_command.prefix_words.is_empty());
        assert!(!time_command.posix_format);
        assert!(time_command.command.words.is_empty());
    }

    #[test]
    fn test_time_prefix_with_options_wraps_null_command_before_next_command() {
        let tokens = tokenize("time -p --; echo done");
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);

        let time_command = ast.commands[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.prefix_words, ["-p", "--"]);
        assert!(time_command.posix_format);
        assert!(time_command.command.words.is_empty());
        assert_eq!(ast.commands[1].words, ["echo", "done"]);
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
        assert_eq!(pipeline.operator_metadata.len(), 2);
        assert_eq!(pipeline.operator_metadata[1].word_index, 1);
        assert_eq!(pipeline.operator_metadata[1].value, "|");
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
        assert_eq!(list.operator_metadata[0].value, "&&");
        assert_eq!(list.operator_metadata[0].raw, "&&");
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
    fn test_arithmetic_for_command_pipeline_stage() {
        let input = "for (( i = 0; i < 2; i++ )); do echo $i; done | wc -l";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.stages.len(), 2);
        assert_eq!(pipeline.operators, ["|"]);
        assert_eq!(pipeline.stages[0].pipe, Some(1));
        assert!(pipeline.stages[0]
            .for_command
            .as_ref()
            .and_then(|for_command| for_command.arithmetic.as_ref())
            .is_some());
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
        assert_eq!(list.operator_metadata.len(), 2);
        assert_eq!(list.operator_metadata[0].word_index, 0);
        assert_eq!(list.operator_metadata[0].value, "||");
        assert_eq!(list.operator_metadata[1].word_index, 1);
        assert_eq!(list.operator_metadata[1].value, "&&");
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
        assert_eq!(time_command.keyword_metadata.value, "time");
        assert_eq!(time_command.keyword_metadata.raw, "time");
        assert_eq!(time_command.prefix_words, ["-p"]);
        assert_eq!(time_command.prefix_word_metadata.len(), 1);
        assert_eq!(time_command.prefix_word_metadata[0].word_index, 0);
        assert_eq!(time_command.prefix_word_metadata[0].value, "-p");
        assert_eq!(time_command.prefix_word_metadata[0].raw, "-p");
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
        assert_eq!(brace_group.open_delimiter_metadata.value, "{");
        assert_eq!(brace_group.open_delimiter_metadata.raw, "{");
        assert_eq!(brace_group.close_delimiter, "}");
        assert_eq!(brace_group.close_delimiter_metadata.value, "}");
        assert_eq!(brace_group.close_delimiter_metadata.raw, "}");
        let body = &brace_group.body;
        assert_eq!(body[0].words, ["echo", "one"]);
        assert_eq!(body[1].words, ["echo", "two"]);
    }

    #[test]
    fn test_time_prefix_brace_group_consumes_pipe_operator() {
        let input = "time { echo one; } | wc -l";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
        assert_eq!(pipeline.operators, ["|"]);
        assert_eq!(pipeline.stages[0].pipe, Some(1));
        let time_command = pipeline.stages[0].time_command.as_ref().unwrap();
        assert_eq!(time_command.keyword_metadata.value, "time");
        assert!(time_command.command.brace_group.is_some());
        assert_eq!(pipeline.stages[1].words, ["wc", "-l"]);
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
        assert_eq!(brace_group.open_delimiter_metadata.value, "{");
        assert_eq!(brace_group.close_delimiter, "}");
        assert_eq!(brace_group.close_delimiter_metadata.value, "}");
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
    fn test_brace_group_keeps_brace_arguments() {
        let input = "{ echo } arg; echo after; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let brace_group = ast.commands[0].brace_group.as_ref().unwrap();

        assert_eq!(brace_group.body.len(), 2);
        assert_eq!(brace_group.body[0].words, ["echo", "}", "arg"]);
        assert_eq!(brace_group.body[1].words, ["echo", "after"]);
    }

    #[test]
    fn test_brace_group_keeps_inner_ansi_c_escaped_quote_brace() {
        let input = "{\necho $'foo\\'{\nbar'\necho after\n}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let brace_group = ast.commands[0].brace_group.as_ref().unwrap();

        assert_eq!(brace_group.body.len(), 2);
        assert_eq!(brace_group.body[0].words, ["echo", "foo'{\nbar"]);
        assert_eq!(brace_group.body[1].words, ["echo", "after"]);
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
    fn test_time_prefix_parses_function_definition() {
        let keyword_tokens = tokenize("time function greet { echo hi; }");
        let keyword_ast = parse(&keyword_tokens);
        let keyword_time = keyword_ast.commands[0].time_command.as_ref().unwrap();
        let keyword_function = keyword_time.command.function_command.as_ref().unwrap();

        assert!(keyword_function.keyword);
        assert_eq!(keyword_function.name, "greet");
        assert_eq!(keyword_function.body[0].words, ["echo", "hi"]);

        let posix_tokens = tokenize("time greet() { echo hi; }");
        let posix_ast = parse(&posix_tokens);
        let posix_time = posix_ast.commands[0].time_command.as_ref().unwrap();
        let posix_function = posix_time.command.function_command.as_ref().unwrap();

        assert!(!posix_function.keyword);
        assert!(posix_function.has_parentheses);
        assert_eq!(posix_function.name, "greet");
        assert_eq!(posix_function.body[0].words, ["echo", "hi"]);
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
        assert_eq!(subshell.open_delimiter_metadata.value, "(");
        assert_eq!(subshell.open_delimiter_metadata.raw, "(");
        assert_eq!(subshell.close_delimiter, ")");
        assert_eq!(subshell.close_delimiter_metadata.value, ")");
        assert_eq!(subshell.close_delimiter_metadata.raw, ")");
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
        assert_eq!(subshell.open_delimiter_metadata.value, "(");
        assert_eq!(subshell.close_delimiter, ")");
        assert_eq!(subshell.close_delimiter_metadata.value, ")");
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

    #[test]
    fn test_subshell_command_keeps_case_pattern_starting_with_esac() {
        let input = "( case esac in\nesac) printf matched ;; esac )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let subshell = ast.commands[0].subshell_command.as_ref().unwrap();
        let case_command = subshell.body[0].case_command.as_ref().unwrap();

        assert_eq!(case_command.word, "esac");
        assert_eq!(case_command.clauses[0].patterns, ["esac"]);
        assert_eq!(case_command.clauses[0].body[0].words, ["printf", "matched"]);
    }

    #[test]
    fn test_subshell_command_keeps_case_argument() {
        let input = "( echo case; echo ok )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let subshell = ast.commands[0].subshell_command.as_ref().unwrap();

        assert_eq!(subshell.body[0].words, ["echo", "case"]);
        assert_eq!(subshell.body[1].words, ["echo", "ok"]);
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
        assert_eq!(for_command.keyword_metadata.value, "for");
        assert_eq!(for_command.keyword_metadata.raw, "for");
        assert_eq!(for_command.in_keyword.as_deref(), Some("in"));
        assert_eq!(
            for_command.in_keyword_metadata.as_ref().unwrap().value,
            "in"
        );
        assert_eq!(for_command.do_keyword, None);
        assert!(for_command.do_keyword_metadata.is_none());
        assert_eq!(for_command.end_keyword, None);
        assert!(for_command.end_keyword_metadata.is_none());
        assert_eq!(for_command.list_terminator.as_deref(), Some(";"));
        assert_eq!(
            for_command.list_terminator_metadata.as_ref().unwrap().value,
            ";"
        );
        assert_eq!(for_command.body_kind, CommandBodyKind::BraceGroup);
        assert_eq!(for_command.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(
            for_command
                .body_open_delimiter_metadata
                .as_ref()
                .unwrap()
                .value,
            "{"
        );
        assert_eq!(for_command.body_close_delimiter.as_deref(), Some("}"));
        assert_eq!(
            for_command
                .body_close_delimiter_metadata
                .as_ref()
                .unwrap()
                .raw,
            "}"
        );
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_arithmetic_for_brace_body_keeps_brace_arguments() {
        let tokens = tokenize("for ((i=0; i<1; i++)); { echo } arg; echo after; }");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();

        assert!(for_command.arithmetic.is_some());
        assert_eq!(for_command.body_kind, CommandBodyKind::BraceGroup);
        assert_eq!(for_command.body.len(), 2);
        assert_eq!(for_command.body[0].words, ["echo", "}", "arg"]);
        assert_eq!(for_command.body[1].words, ["echo", "after"]);
    }

    #[test]
    fn test_brace_bodies_keep_case_patterns_named_like_close_brace() {
        let brace_tokens = tokenize("{ case brace in }) echo close ;; esac; echo after; }");
        let brace_ast = parse(&brace_tokens);
        let for_tokens =
            tokenize("for x in brace; { case brace in }) echo for ;; esac; echo after; }");
        let for_ast = parse(&for_tokens);
        let arithmetic_for_tokens = tokenize(
            "for ((i=0; i<1; i++)); { case brace in }) echo arithmetic ;; esac; echo after; }",
        );
        let arithmetic_for_ast = parse(&arithmetic_for_tokens);
        let select_tokens =
            tokenize("select x in brace; { case brace in }) echo select ;; esac; echo after; }");
        let select_ast = parse(&select_tokens);

        let brace_group = brace_ast.commands[0].brace_group.as_ref().unwrap();
        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        let arithmetic_for_command = arithmetic_for_ast.commands[0].for_command.as_ref().unwrap();
        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();

        assert_eq!(
            brace_group.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["}"]
        );
        assert_eq!(brace_group.body[1].words, ["echo", "after"]);
        assert_eq!(
            for_command.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["}"]
        );
        assert_eq!(for_command.body[1].words, ["echo", "after"]);
        assert_eq!(
            arithmetic_for_command.body[0]
                .case_command
                .as_ref()
                .unwrap()
                .clauses[0]
                .patterns,
            ["}"]
        );
        assert_eq!(arithmetic_for_command.body[1].words, ["echo", "after"]);
        assert_eq!(
            select_command.body[0]
                .case_command
                .as_ref()
                .unwrap()
                .clauses[0]
                .patterns,
            ["}"]
        );
        assert_eq!(select_command.body[1].words, ["echo", "after"]);
    }

    #[test]
    fn test_brace_bodies_keep_esac_pattern_and_close_brace_argument() {
        let compact_tokens = tokenize("{ case esac in esac) echo } arg ;; esac; echo after; }");
        let compact_ast = parse(&compact_tokens);
        let multiline_tokens = tokenize("{\ncase esac in\nesac) echo } arg ;; esac\necho after\n}");
        let multiline_ast = parse(&multiline_tokens);

        let compact = compact_ast.commands[0].brace_group.as_ref().unwrap();
        let multiline = multiline_ast.commands[0].brace_group.as_ref().unwrap();

        assert_eq!(
            compact.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["esac"]
        );
        assert_eq!(
            compact.body[0].case_command.as_ref().unwrap().clauses[0].body[0].words,
            ["echo", "}", "arg"]
        );
        assert_eq!(compact.body[1].words, ["echo", "after"]);
        assert_eq!(
            multiline.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["esac"]
        );
        assert_eq!(
            multiline.body[0].case_command.as_ref().unwrap().clauses[0].body[0].words,
            ["echo", "}", "arg"]
        );
        assert_eq!(multiline.body[1].words, ["echo", "after"]);
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
        assert_eq!(first.list_terminator_metadata.as_ref().unwrap().value, ";");
        assert_eq!(first.body_open_delimiter.as_deref(), Some("do"));
        assert_eq!(
            first.body_open_delimiter_metadata.as_ref().unwrap().value,
            "do"
        );
        assert_eq!(first.body_close_delimiter.as_deref(), Some("done"));
        assert_eq!(
            first.body_close_delimiter_metadata.as_ref().unwrap().value,
            "done"
        );
        assert_eq!(first.body[0].words, ["echo", "$x"]);
        assert_eq!(second.body_kind, CommandBodyKind::BraceGroup);
        assert_eq!(second.keyword, "select");
        assert_eq!(second.in_keyword.as_deref(), Some("in"));
        assert_eq!(second.do_keyword, None);
        assert_eq!(second.end_keyword, None);
        assert_eq!(second.list_terminator.as_deref(), Some(";"));
        assert_eq!(second.list_terminator_metadata.as_ref().unwrap().raw, ";");
        assert_eq!(second.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(
            second.body_open_delimiter_metadata.as_ref().unwrap().value,
            "{"
        );
        assert_eq!(second.body_close_delimiter.as_deref(), Some("}"));
        assert_eq!(
            second.body_close_delimiter_metadata.as_ref().unwrap().value,
            "}"
        );
        assert_eq!(second.body[0].words, ["echo", "$y"]);
    }

    #[test]
    fn test_for_and_select_words_keep_process_substitution() {
        let for_tokens = tokenize("for x in <(printf a) >(cat); do :; done");
        let for_ast = parse(&for_tokens);
        let select_tokens = tokenize("select x in <(printf a) >(cat); do :; done");
        let select_ast = parse(&select_tokens);

        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();

        assert_eq!(for_command.words, ["<(printf a)", ">(cat)"]);
        assert_eq!(select_command.words, ["<(printf a)", ">(cat)"]);
        assert_eq!(for_command.word_metadata.len(), 2);
        assert_eq!(for_command.word_metadata[0].process_substitutions.len(), 1);
        assert_eq!(
            for_command.word_metadata[0].process_substitutions[0].target,
            "<(printf a)"
        );
        assert_eq!(
            for_command.word_metadata[0].process_substitutions[0]
                .open_delimiter_metadata
                .value,
            "<("
        );
        assert_eq!(
            for_command.word_metadata[0].process_substitutions[0]
                .close_delimiter_metadata
                .value,
            ")"
        );
        assert_eq!(
            for_command.word_metadata[0].process_substitutions[0].source,
            "printf a"
        );
        assert!(!for_command.word_metadata[0].process_substitutions[0].output);
        assert_eq!(
            for_command.word_metadata[0].process_substitutions[0].word_index,
            Some(0)
        );
        assert_eq!(
            for_command.word_metadata[0].process_substitutions[0].commands[0].words,
            ["printf", "a"]
        );
        assert_eq!(for_command.word_metadata[1].process_substitutions.len(), 1);
        assert_eq!(
            for_command.word_metadata[1].process_substitutions[0].target,
            ">(cat)"
        );
        assert_eq!(
            for_command.word_metadata[1].process_substitutions[0]
                .open_delimiter_metadata
                .value,
            ">("
        );
        assert!(for_command.word_metadata[1].process_substitutions[0].output);
        assert_eq!(
            for_command.word_metadata[1].process_substitutions[0].word_index,
            Some(1)
        );
        assert_eq!(select_command.word_metadata.len(), 2);
        assert_eq!(
            select_command.word_metadata[0].process_substitutions[0].target,
            "<(printf a)"
        );
        assert_eq!(
            select_command.word_metadata[1].process_substitutions[0].target,
            ">(cat)"
        );
    }

    #[test]
    fn test_for_and_select_words_keep_reserved_words() {
        let for_tokens = tokenize("for x in do done; do echo $x; done");
        let for_ast = parse(&for_tokens);
        let select_tokens = tokenize("select x in do done; do echo $x; done");
        let select_ast = parse(&select_tokens);

        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();

        assert_eq!(for_command.words, ["do", "done"]);
        assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
        assert_eq!(select_command.words, ["do", "done"]);
        assert_eq!(select_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(select_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_for_and_select_allow_newline_before_in_keyword() {
        let for_tokens = tokenize("for x\nin a b\ndo echo $x\ndone");
        let for_ast = parse(&for_tokens);
        let select_tokens = tokenize("select x\nin a b\ndo echo $x\ndone");
        let select_ast = parse(&select_tokens);

        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();

        assert_eq!(for_command.variable, "x");
        assert_eq!(for_command.in_keyword.as_deref(), Some("in"));
        assert_eq!(for_command.words, ["a", "b"]);
        assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(for_command.end_keyword.as_deref(), Some("done"));
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);
        assert_eq!(select_command.variable, "x");
        assert_eq!(select_command.in_keyword.as_deref(), Some("in"));
        assert_eq!(select_command.words, ["a", "b"]);
        assert_eq!(select_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(select_command.end_keyword.as_deref(), Some("done"));
        assert_eq!(select_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_for_and_select_require_shell_name_variables() {
        let invalid_for_ast = parse(&tokenize("for 1bad in a; do echo $x; done"));
        let variable_for_ast = parse(&tokenize("for $name in a; do echo $name; done"));
        let invalid_select_ast = parse(&tokenize("select 1bad in a; do echo $x; done"));
        let variable_select_ast = parse(&tokenize("select $name in a; do echo $name; done"));

        assert!(invalid_for_ast
            .commands
            .iter()
            .all(|command| command.for_command.is_none()));
        assert!(variable_for_ast
            .commands
            .iter()
            .all(|command| command.for_command.is_none()));
        assert!(invalid_select_ast
            .commands
            .iter()
            .all(|command| command.select_command.is_none()));
        assert!(variable_select_ast
            .commands
            .iter()
            .all(|command| command.select_command.is_none()));
    }

    #[test]
    fn test_for_and_select_default_positional_without_semicolon() {
        let for_ast = parse(&tokenize("for x do echo $x; done"));
        let select_ast = parse(&tokenize("select x do echo $x; done"));

        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        assert_eq!(for_command.variable, "x");
        assert!(for_command.default_positional);
        assert_eq!(for_command.in_keyword, None);
        assert_eq!(for_command.list_terminator, None);
        assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(for_command.body[0].words, ["echo", "$x"]);

        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();
        assert_eq!(select_command.variable, "x");
        assert!(select_command.default_positional);
        assert_eq!(select_command.in_keyword, None);
        assert_eq!(select_command.list_terminator, None);
        assert_eq!(select_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(select_command.body[0].words, ["echo", "$x"]);
    }

    #[test]
    fn test_arithmetic_for_records_empty_expression_slots() {
        let tokens = tokenize("for (( ; ; i++ )); do echo $i; done");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();
        let arithmetic = for_command.arithmetic.as_ref().unwrap();

        assert_eq!(arithmetic.init, "");
        assert_eq!(arithmetic.test, "");
        assert_eq!(arithmetic.update, "i++");
        assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
        assert_eq!(for_command.body[0].words, ["echo", "$i"]);
    }

    #[test]
    fn test_arithmetic_for_records_all_empty_expression_slots() {
        let tokens = tokenize("for (( ; ; )); do :; done");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();
        let arithmetic = for_command.arithmetic.as_ref().unwrap();

        assert_eq!(arithmetic.init, "");
        assert_eq!(arithmetic.test, "");
        assert_eq!(arithmetic.update, "");
        assert_eq!(for_command.body[0].words, [":"]);
    }

    #[test]
    fn test_for_and_select_words_record_metadata() {
        let for_tokens = tokenize("for x in ${one:-1} pre{a,b} src/[ab]? \"*.rs\"; do :; done");
        let for_ast = parse(&for_tokens);
        let select_tokens = tokenize("select x in $((i+1)) @(yes|no) ~+/bin; do echo $x; done");
        let select_ast = parse(&select_tokens);

        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        assert_eq!(
            for_command.words,
            ["${one:-1}", "pre{a,b}", "src/[ab]?", "*.rs"]
        );
        assert_eq!(for_command.keyword_metadata.value, "for");
        assert_eq!(for_command.keyword_metadata.raw, "for");
        assert_eq!(
            for_command.in_keyword_metadata.as_ref().unwrap().value,
            "in"
        );
        assert_eq!(
            for_command.do_keyword_metadata.as_ref().unwrap().value,
            "do"
        );
        assert_eq!(
            for_command.end_keyword_metadata.as_ref().unwrap().value,
            "done"
        );
        assert_eq!(for_command.variable, "x");
        assert_eq!(for_command.variable_metadata.word_index, 0);
        assert_eq!(for_command.variable_metadata.value, "x");
        assert_eq!(for_command.variable_metadata.raw, "x");
        assert_eq!(for_command.word_metadata.len(), 4);
        assert_eq!(for_command.word_metadata[0].word_index, 0);
        assert_eq!(
            for_command.word_metadata[0].parameter_expansions[0].text,
            "${one:-1}"
        );
        assert_eq!(
            for_command.word_metadata[0].parameter_expansions[0].name,
            "one"
        );
        assert_eq!(
            for_command.word_metadata[1].brace_expansions[0].text,
            "{a,b}"
        );
        assert_eq!(
            for_command.word_metadata[2].pathname_patterns[0].operators,
            ["[ab]", "?"]
        );
        assert!(for_command.word_metadata[3].pathname_patterns.is_empty());
        assert_eq!(for_command.word_metadata[3].word_quotes[0].text, "\"*.rs\"");
        assert_eq!(
            for_command.word_metadata[3].word_quotes[0].kind,
            QuoteKind::Double
        );

        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();
        assert_eq!(select_command.words, ["$((i+1))", "@(yes|no)", "~+/bin"]);
        assert_eq!(select_command.keyword_metadata.value, "select");
        assert_eq!(select_command.keyword_metadata.raw, "select");
        assert_eq!(
            select_command.in_keyword_metadata.as_ref().unwrap().value,
            "in"
        );
        assert_eq!(
            select_command.do_keyword_metadata.as_ref().unwrap().value,
            "do"
        );
        assert_eq!(
            select_command.end_keyword_metadata.as_ref().unwrap().value,
            "done"
        );
        assert_eq!(select_command.variable, "x");
        assert_eq!(select_command.variable_metadata.word_index, 0);
        assert_eq!(select_command.variable_metadata.value, "x");
        assert_eq!(select_command.variable_metadata.raw, "x");
        assert_eq!(select_command.word_metadata.len(), 3);
        assert_eq!(
            select_command.word_metadata[0].arithmetic_expansions[0].expression,
            "i+1"
        );
        assert_eq!(
            select_command.word_metadata[0].arithmetic_expansions[0].variables,
            ["i"]
        );
        assert_eq!(
            select_command.word_metadata[1].extglob_patterns[0].operator,
            '@'
        );
        assert_eq!(
            select_command.word_metadata[1].extglob_patterns[0].alternatives,
            ["yes", "no"]
        );
        assert_eq!(
            select_command.word_metadata[2].tilde_expansions[0].prefix,
            "~+"
        );
        assert_eq!(
            select_command.word_metadata[2].tilde_expansions[0].suffix,
            "/bin"
        );
    }

    #[test]
    fn test_for_word_metadata_records_command_substitution() {
        let tokens = tokenize("for x in $(printf a) `printf b`; do :; done");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();

        assert_eq!(for_command.words, ["$(printf a)", "`printf b`"]);
        assert_eq!(for_command.word_metadata.len(), 2);
        assert_eq!(for_command.word_metadata[0].command_substitutions.len(), 1);
        let dollar = &for_command.word_metadata[0].command_substitutions[0];
        assert_eq!(dollar.text, "$(printf a)");
        assert_eq!(dollar.source, "printf a");
        assert!(!dollar.backtick);
        assert_eq!(dollar.word_index, Some(0));
        assert_eq!(dollar.commands[0].words, ["printf", "a"]);

        assert_eq!(for_command.word_metadata[1].command_substitutions.len(), 1);
        let backtick = &for_command.word_metadata[1].command_substitutions[0];
        assert_eq!(backtick.text, "`printf b`");
        assert_eq!(backtick.source, "printf b");
        assert!(backtick.backtick);
        assert_eq!(backtick.word_index, Some(1));
        assert_eq!(backtick.commands[0].words, ["printf", "b"]);
    }

    #[test]
    fn test_loop_bodies_keep_case_patterns_named_like_delimiters() {
        let for_tokens = tokenize("for x in one; do case done in done) echo for ;; esac; done");
        let for_ast = parse(&for_tokens);
        let while_tokens = tokenize("while true; do case done in done) echo while ;; esac; done");
        let while_ast = parse(&while_tokens);
        let select_tokens =
            tokenize("select x in one; do case done in done) echo select ;; esac; done");
        let select_ast = parse(&select_tokens);

        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        let while_command = while_ast.commands[0].loop_command.as_ref().unwrap();
        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();

        assert!(for_command.body[0].case_command.is_some());
        assert_eq!(
            for_command.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["done"]
        );
        assert!(while_command.body[0].case_command.is_some());
        assert_eq!(
            while_command.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["done"]
        );
        assert!(select_command.body[0].case_command.is_some());
        assert_eq!(
            select_command.body[0]
                .case_command
                .as_ref()
                .unwrap()
                .clauses[0]
                .patterns,
            ["done"]
        );
    }

    #[test]
    fn test_compound_scanners_keep_reserved_word_arguments() {
        let if_tokens = tokenize("if echo then; then echo else; else echo fi; fi");
        let if_ast = parse(&if_tokens);
        let while_tokens = tokenize("while echo do; do echo done; break; done");
        let while_ast = parse(&while_tokens);
        let for_tokens = tokenize("for x in one; do echo done; done");
        let for_ast = parse(&for_tokens);
        let select_tokens = tokenize("select x in one; do echo done; done");
        let select_ast = parse(&select_tokens);

        let if_command = if_ast.commands[0].if_command.as_ref().unwrap();
        let while_command = while_ast.commands[0].loop_command.as_ref().unwrap();
        let for_command = for_ast.commands[0].for_command.as_ref().unwrap();
        let select_command = select_ast.commands[0].select_command.as_ref().unwrap();

        assert_eq!(if_command.condition[0].words, ["echo", "then"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "else"]);
        assert_eq!(
            if_command.else_body.as_ref().unwrap()[0].words,
            ["echo", "fi"]
        );
        assert_eq!(while_command.condition[0].words, ["echo", "do"]);
        assert_eq!(while_command.body[0].words, ["echo", "done"]);
        assert_eq!(for_command.body[0].words, ["echo", "done"]);
        assert_eq!(select_command.body[0].words, ["echo", "done"]);
    }

    #[test]
    fn test_for_do_done_body_accepts_brace_group_before_done() {
        let tokens = tokenize("for x in one; do { echo $x; } done");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();

        assert!(for_command
            .body
            .iter()
            .any(|command| command.brace_group.is_some()));
        assert_eq!(for_command.end_keyword.as_deref(), Some("done"));
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
        assert_eq!(while_command.keyword_metadata.value, "while");
        assert_eq!(while_command.keyword_metadata.raw, "while");
        assert_eq!(while_command.do_keyword, "do");
        assert_eq!(while_command.do_keyword_metadata.value, "do");
        assert_eq!(while_command.do_keyword_metadata.raw, "do");
        assert_eq!(while_command.end_keyword, "done");
        assert_eq!(while_command.end_keyword_metadata.value, "done");
        assert_eq!(while_command.end_keyword_metadata.raw, "done");
        assert_eq!(while_command.body_open_delimiter, "do");
        assert_eq!(while_command.body_open_delimiter_metadata.value, "do");
        assert_eq!(while_command.body_open_delimiter_metadata.raw, "do");
        assert_eq!(while_command.body_close_delimiter, "done");
        assert_eq!(while_command.body_close_delimiter_metadata.value, "done");
        assert_eq!(while_command.body_close_delimiter_metadata.raw, "done");
        assert_eq!(while_command.condition_terminator.as_deref(), Some(";"));
        assert_eq!(
            while_command
                .condition_terminator_metadata
                .as_ref()
                .unwrap()
                .value,
            ";"
        );
        assert_eq!(
            while_command
                .condition_terminator_metadata
                .as_ref()
                .unwrap()
                .raw,
            ";"
        );
        assert_eq!(while_command.condition[0].words, ["false"]);
        assert_eq!(while_command.body[0].words, ["echo", "bad"]);
        assert_eq!(until_command.kind, LoopKind::Until);
        assert_eq!(until_command.keyword, "until");
        assert_eq!(until_command.keyword_metadata.value, "until");
        assert_eq!(until_command.keyword_metadata.raw, "until");
        assert_eq!(until_command.body_open_delimiter, "do");
        assert_eq!(until_command.body_open_delimiter_metadata.value, "do");
        assert_eq!(until_command.body_close_delimiter, "done");
        assert_eq!(until_command.body_close_delimiter_metadata.value, "done");
        assert_eq!(until_command.condition_terminator.as_deref(), Some(";"));
        assert_eq!(
            until_command
                .condition_terminator_metadata
                .as_ref()
                .unwrap()
                .value,
            ";"
        );
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
        assert_eq!(if_command.keyword_metadata.value, "if");
        assert_eq!(if_command.keyword_metadata.raw, "if");
        assert_eq!(if_command.then_keyword, "then");
        assert_eq!(if_command.then_keyword_metadata.value, "then");
        assert_eq!(if_command.then_keyword_metadata.raw, "then");
        assert_eq!(if_command.end_keyword, "fi");
        assert_eq!(if_command.end_keyword_metadata.value, "fi");
        assert_eq!(if_command.end_keyword_metadata.raw, "fi");
        assert_eq!(if_command.condition_terminator.as_deref(), Some(";"));
        assert_eq!(
            if_command
                .condition_terminator_metadata
                .as_ref()
                .unwrap()
                .value,
            ";"
        );
        assert_eq!(
            if_command
                .condition_terminator_metadata
                .as_ref()
                .unwrap()
                .raw,
            ";"
        );
        assert_eq!(if_command.condition[0].words, ["true"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
        assert!(if_command.elif_branches.is_empty());
        assert_eq!(if_command.else_keyword, None);
        assert!(if_command.else_keyword_metadata.is_none());
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
        assert_eq!(if_command.elif_branches[0].keyword_metadata.value, "elif");
        assert_eq!(if_command.elif_branches[0].keyword_metadata.raw, "elif");
        assert_eq!(if_command.elif_branches[0].then_keyword, "then");
        assert_eq!(
            if_command.elif_branches[0].then_keyword_metadata.value,
            "then"
        );
        assert_eq!(
            if_command.elif_branches[0].then_keyword_metadata.raw,
            "then"
        );
        assert_eq!(
            if_command.elif_branches[0].condition_terminator.as_deref(),
            Some(";")
        );
        assert_eq!(
            if_command.elif_branches[0]
                .condition_terminator_metadata
                .as_ref()
                .unwrap()
                .value,
            ";"
        );
        assert_eq!(if_command.elif_branches[0].condition[0].words, ["true"]);
        assert_eq!(if_command.elif_branches[0].body[0].words, ["echo", "yes"]);
        assert_eq!(if_command.else_keyword.as_deref(), Some("else"));
        let else_keyword_metadata = if_command.else_keyword_metadata.as_ref().unwrap();
        assert_eq!(else_keyword_metadata.value, "else");
        assert_eq!(else_keyword_metadata.raw, "else");
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
        assert_eq!(function.name_metadata.word_index, 0);
        assert_eq!(function.name_metadata.value, "greet");
        assert_eq!(function.name_metadata.raw, "greet");
        assert!(function.name_metadata.word_quotes.is_empty());
        assert!(function.keyword);
        assert_eq!(function.keyword_text.as_deref(), Some("function"));
        assert_eq!(
            function.keyword_metadata.as_ref().unwrap().value,
            "function"
        );
        assert_eq!(function.keyword_metadata.as_ref().unwrap().raw, "function");
        assert!(!function.has_parentheses);
        assert_eq!(function.open_paren, None);
        assert!(function.open_paren_metadata.is_none());
        assert_eq!(function.close_paren, None);
        assert!(function.close_paren_metadata.is_none());
        assert_eq!(function.body_kind, FunctionBodyKind::BraceGroup);
        assert_eq!(function.body_open_delimiter.as_deref(), Some("{"));
        assert_eq!(
            function
                .body_open_delimiter_metadata
                .as_ref()
                .unwrap()
                .value,
            "{"
        );
        assert_eq!(function.body_close_delimiter.as_deref(), Some("}"));
        assert_eq!(
            function
                .body_close_delimiter_metadata
                .as_ref()
                .unwrap()
                .value,
            "}"
        );
        assert!(function.body_start.is_some());
        assert!(function.body_end.is_some());
        assert_eq!(function.body.len(), 1);
        assert_eq!(function.body[0].words, ["echo", "hi"]);
    }

    #[test]
    fn test_function_body_keeps_brace_arguments() {
        let input = "function greet { echo } arg; echo after; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(function.body.len(), 2);
        assert_eq!(function.body[0].words, ["echo", "}", "arg"]);
        assert_eq!(function.body[1].words, ["echo", "after"]);
    }

    #[test]
    fn test_function_brace_body_keeps_case_pattern_named_like_close_brace() {
        let input = "function greet { case brace in }) echo close ;; esac; echo after; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);

        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(
            function.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["}"]
        );
        assert_eq!(function.body[1].words, ["echo", "after"]);
    }

    #[test]
    fn test_multiline_function_brace_body_keeps_case_pattern_named_like_close_brace() {
        let input = "greet()\n{\ncase brace in }) echo close ;; esac\necho after\n}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);

        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(
            function.body[0].case_command.as_ref().unwrap().clauses[0].patterns,
            ["}"]
        );
        assert_eq!(function.body[1].words, ["echo", "after"]);
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
        assert_eq!(
            function.keyword_metadata.as_ref().unwrap().value,
            "function"
        );
        assert!(function.has_parentheses);
        assert_eq!(function.open_paren.as_deref(), Some("("));
        assert_eq!(function.open_paren_metadata.as_ref().unwrap().value, "(");
        assert_eq!(function.close_paren.as_deref(), Some(")"));
        assert_eq!(function.close_paren_metadata.as_ref().unwrap().value, ")");
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
        assert_eq!(function.name_metadata.value, "foo-a");
        assert_eq!(function.name_metadata.raw, "foo-a");
        assert!(function.name_metadata.pathname_patterns.is_empty());
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
        assert!(function.keyword_metadata.is_none());
        assert_eq!(function.open_paren_metadata.as_ref().unwrap().value, "(");
        assert_eq!(function.close_paren_metadata.as_ref().unwrap().value, ")");
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
        assert_eq!(
            function
                .body_open_delimiter_metadata
                .as_ref()
                .unwrap()
                .value,
            "("
        );
        assert_eq!(function.body_close_delimiter.as_deref(), Some(")"));
        assert_eq!(
            function
                .body_close_delimiter_metadata
                .as_ref()
                .unwrap()
                .value,
            ")"
        );
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
    fn test_parenthesized_function_body_keeps_case_argument() {
        let input = "foo() ( echo case; echo ok )";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let function = ast.commands[0].function_command.as_ref().unwrap();

        assert_eq!(function.body_kind, FunctionBodyKind::Subshell);
        assert_eq!(function.body[0].words, ["echo", "case"]);
        assert_eq!(function.body[1].words, ["echo", "ok"]);
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
    fn test_function_body_can_be_select_command() {
        let input = "foo() select choice in alpha beta; do echo $choice; break; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        let select_command = function.body[0].select_command.as_ref().unwrap();

        assert_eq!(function.name, "foo");
        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(select_command.variable, "choice");
        assert_eq!(select_command.words, ["alpha", "beta"]);
        assert_eq!(select_command.body[0].words, ["echo", "$choice"]);
    }

    #[test]
    fn test_function_body_can_be_if_command() {
        let input = "foo() if true; then echo yes; else echo no; fi";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        let if_command = function.body[0].if_command.as_ref().unwrap();
        assert_eq!(if_command.condition[0].words, ["true"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
        assert_eq!(
            if_command.else_body.as_ref().unwrap()[0].words,
            ["echo", "no"]
        );
    }

    #[test]
    fn test_function_body_can_be_while_command() {
        let input = "foo() while false; do echo bad; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        assert_eq!(function.name, "foo");
        let loop_command = function.body[0].loop_command.as_ref().unwrap();
        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
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
    fn test_function_body_can_be_until_command() {
        let input = "foo() until true; do echo never; done";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        let loop_command = function.body[0].loop_command.as_ref().unwrap();

        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(loop_command.kind, LoopKind::Until);
        assert!(loop_command.until);
        assert_eq!(loop_command.keyword, "until");
        assert_eq!(loop_command.condition[0].words, ["true"]);
        assert_eq!(loop_command.body[0].words, ["echo", "never"]);
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
        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(
            conditional.args,
            ["$1", "==", "a*", "&&", "$2", "-gt", "1", "]]"]
        );
        assert_eq!(
            function.body[0].words,
            ["[[", "$1", "==", "a*", "&&", "$2", "-gt", "1", "]]"]
        );
    }

    #[test]
    fn test_function_body_can_be_time_command() {
        let simple_tokens = tokenize("foo() time -p echo hi");
        let simple_ast = parse(&simple_tokens);
        let simple_function = simple_ast.commands[0].function_command.as_ref().unwrap();
        let simple_time = simple_function.body[0].time_command.as_ref().unwrap();

        assert_eq!(simple_function.body_kind, FunctionBodyKind::CompoundCommand);
        assert!(simple_time.posix_format);
        assert_eq!(simple_time.prefix_words, ["-p"]);
        assert_eq!(simple_time.command.words, ["echo", "hi"]);

        let brace_tokens = tokenize("bar() time { echo one; echo two; }");
        let brace_ast = parse(&brace_tokens);
        let brace_function = brace_ast.commands[0].function_command.as_ref().unwrap();
        let brace_time = brace_function.body[0].time_command.as_ref().unwrap();

        assert_eq!(brace_function.body_kind, FunctionBodyKind::CompoundCommand);
        assert!(brace_time.command.brace_group.is_some());
        assert_eq!(
            brace_time.command.brace_group.as_ref().unwrap().body[0].words,
            ["echo", "one"]
        );
    }

    #[test]
    fn test_function_conditional_body_keeps_quoted_closing_delimiter_word() {
        let input = "foo() [[ value == \"]]\" ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let function = ast.commands[0].function_command.as_ref().unwrap();
        let conditional = function.body[0].conditional_command.as_ref().unwrap();

        assert_eq!(function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(conditional.args, ["value", "==", "]]", "]]"]);
        assert_eq!(conditional.expression.operands, ["value", "]]"]);
    }

    #[test]
    fn test_function_sequence_body_keeps_reserved_word_arguments() {
        let if_tokens = tokenize("foo() if echo then; then echo fi; fi");
        let if_ast = parse(&if_tokens);
        let if_function = if_ast.commands[0].function_command.as_ref().unwrap();
        let if_command = if_function.body[0].if_command.as_ref().unwrap();

        let loop_tokens = tokenize("bar() while echo do; do echo done; break; done");
        let loop_ast = parse(&loop_tokens);
        let loop_function = loop_ast.commands[0].function_command.as_ref().unwrap();
        let loop_command = loop_function.body[0].loop_command.as_ref().unwrap();

        assert_eq!(if_function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(if_command.condition[0].words, ["echo", "then"]);
        assert_eq!(if_command.then_body[0].words, ["echo", "fi"]);
        assert_eq!(loop_function.body_kind, FunctionBodyKind::CompoundCommand);
        assert_eq!(loop_command.condition[0].words, ["echo", "do"]);
        assert_eq!(loop_command.body[0].words, ["echo", "done"]);
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
        assert_eq!(conditional.open_delimiter_metadata.value, "[[");
        assert_eq!(conditional.open_delimiter_metadata.raw, "[[");
        assert_eq!(conditional.close_delimiter, "]]");
        assert_eq!(conditional.close_delimiter_metadata.value, "]]");
        assert_eq!(conditional.close_delimiter_metadata.raw, "]]");
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
        assert!(conditional.expression.open_delimiter_metadata.is_none());
        assert_eq!(conditional.expression.close_delimiter, None);
        assert!(conditional.expression.close_delimiter_metadata.is_none());
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
            conditional.expression.children[0]
                .pattern_operand
                .as_ref()
                .unwrap()
                .kind,
            ConditionalPatternKind::Glob
        );
        assert_eq!(
            conditional.expression.children[0]
                .pattern_operand
                .as_ref()
                .unwrap()
                .text,
            "a*"
        );
        assert_eq!(
            conditional.expression.children[0]
                .pattern_operand
                .as_ref()
                .unwrap()
                .operators,
            ["*"]
        );
        assert!(
            conditional.expression.children[0]
                .pattern_operand
                .as_ref()
                .unwrap()
                .has_glob
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
        assert_eq!(group.open_delimiter_metadata.as_ref().unwrap().value, "(");
        assert_eq!(group.open_delimiter_metadata.as_ref().unwrap().raw, "(");
        assert_eq!(group.close_delimiter.as_deref(), Some(")"));
        assert_eq!(group.close_delimiter_metadata.as_ref().unwrap().value, ")");
        assert_eq!(group.close_delimiter_metadata.as_ref().unwrap().raw, ")");
        let logical = &group.children[0];
        assert_eq!(logical.kind, ConditionalExpressionKind::Logical);
        assert_eq!(logical.operator.as_deref(), Some("||"));
        assert_eq!(logical.children[0].kind, ConditionalExpressionKind::Unary);
        assert_eq!(logical.children[0].operator.as_deref(), Some("-z"));
        assert_eq!(logical.children[1].kind, ConditionalExpressionKind::Binary);
        assert_eq!(logical.children[1].operator.as_deref(), Some("=~"));
    }

    #[test]
    fn test_conditional_command_joins_pattern_rhs_fragments() {
        let extglob_tokens = tokenize("[[ foo == @(foo|bar) ]]");
        let extglob_ast = parse(&extglob_tokens);
        let regex_tokens = tokenize("[[ shellmath_add =~ shellmath_(add|subtract|multiply)$ ]]");
        let regex_ast = parse(&regex_tokens);
        let capture_tokens = tokenize("[[ '2:bad' =~ ^([0-9]+):(.*) ]]");
        let capture_ast = parse(&capture_tokens);

        let extglob = extglob_ast.commands[0]
            .conditional_command
            .as_ref()
            .unwrap();
        let regex = regex_ast.commands[0].conditional_command.as_ref().unwrap();
        let capture = capture_ast.commands[0]
            .conditional_command
            .as_ref()
            .unwrap();

        assert_eq!(extglob.expression.kind, ConditionalExpressionKind::Binary);
        assert_eq!(extglob.expression.operator.as_deref(), Some("=="));
        assert_eq!(extglob.expression.operands, ["foo", "@(foo|bar)"]);
        assert_eq!(
            extglob
                .expression
                .pattern_operand
                .as_ref()
                .unwrap()
                .operators,
            ["@(", "|"]
        );
        assert_eq!(
            extglob
                .expression
                .pattern_operand
                .as_ref()
                .unwrap()
                .extglob_patterns
                .len(),
            1
        );
        assert_eq!(
            extglob
                .expression
                .pattern_operand
                .as_ref()
                .unwrap()
                .extglob_patterns[0]
                .alternatives,
            ["foo", "bar"]
        );
        assert!(
            extglob
                .expression
                .pattern_operand
                .as_ref()
                .unwrap()
                .has_extglob
        );
        assert_eq!(regex.expression.kind, ConditionalExpressionKind::Binary);
        assert_eq!(regex.expression.operator.as_deref(), Some("=~"));
        assert_eq!(
            regex.expression.operands,
            ["shellmath_add", "shellmath_(add|subtract|multiply)$"]
        );
        assert_eq!(
            regex.expression.pattern_operand.as_ref().unwrap().kind,
            ConditionalPatternKind::Regex
        );
        assert_eq!(
            regex.expression.pattern_operand.as_ref().unwrap().text,
            "shellmath_(add|subtract|multiply)$"
        );
        assert!(regex
            .expression
            .pattern_operand
            .as_ref()
            .unwrap()
            .operators
            .is_empty());
        assert!(regex
            .expression
            .pattern_operand
            .as_ref()
            .unwrap()
            .extglob_patterns
            .is_empty());
        assert!(regex
            .expression
            .pattern_operand
            .as_ref()
            .unwrap()
            .brace_expansions
            .is_empty());
        assert_eq!(capture.expression.kind, ConditionalExpressionKind::Binary);
        assert_eq!(capture.expression.operator.as_deref(), Some("=~"));
        assert_eq!(capture.expression.operands, ["2:bad", "^([0-9]+):(.*)"]);
        assert_eq!(
            capture.expression.pattern_operand.as_ref().unwrap().kind,
            ConditionalPatternKind::Regex
        );
    }

    #[test]
    fn test_conditional_command_records_nested_extglob_pattern_operand() {
        let input = "[[ name == @(src|+(test|bench)) ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();
        let pattern = conditional.expression.pattern_operand.as_ref().unwrap();

        assert_eq!(pattern.kind, ConditionalPatternKind::Glob);
        assert_eq!(pattern.text, "@(src|+(test|bench))");
        assert_eq!(pattern.operators, ["@(", "|", "+(", "|"]);
        assert_eq!(pattern.extglob_patterns.len(), 2);
        assert_eq!(pattern.extglob_patterns[0].text, "@(src|+(test|bench))");
        assert_eq!(
            pattern.extglob_patterns[0].alternatives,
            ["src", "+(test|bench)"]
        );
        assert_eq!(pattern.extglob_patterns[1].text, "+(test|bench)");
        assert_eq!(pattern.extglob_patterns[1].alternatives, ["test", "bench"]);
        assert!(pattern.has_extglob);
    }

    #[test]
    fn test_conditional_command_records_brace_pattern_operand() {
        let input = "[[ name == src/{bin,{test,bench}}/*.rs ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();
        let pattern = conditional.expression.pattern_operand.as_ref().unwrap();

        assert_eq!(pattern.kind, ConditionalPatternKind::Glob);
        assert_eq!(pattern.text, "src/{bin,{test,bench}}/*.rs");
        assert_eq!(pattern.operators, ["*"]);
        assert_eq!(pattern.brace_expansions.len(), 2);
        assert_eq!(pattern.brace_expansions[0].text, "{bin,{test,bench}}");
        assert_eq!(pattern.brace_expansions[0].operators, [","]);
        assert_eq!(pattern.brace_expansions[1].text, "{test,bench}");
        assert_eq!(pattern.brace_expansions[1].operators, [","]);
        assert!(pattern.has_glob);
    }

    #[test]
    fn test_conditional_command_records_parameter_pattern_operand() {
        let glob_tokens = tokenize("[[ path == ${prefix:-src}/${file:-*.rs} ]]");
        let glob_ast = parse(&glob_tokens);
        let regex_tokens = tokenize("[[ value =~ ${re:-^a+$} ]]");
        let regex_ast = parse(&regex_tokens);

        let glob = glob_ast.commands[0]
            .conditional_command
            .as_ref()
            .unwrap()
            .expression
            .pattern_operand
            .as_ref()
            .unwrap();
        let regex = regex_ast.commands[0]
            .conditional_command
            .as_ref()
            .unwrap()
            .expression
            .pattern_operand
            .as_ref()
            .unwrap();

        assert_eq!(glob.kind, ConditionalPatternKind::Glob);
        assert_eq!(glob.parameter_expansions.len(), 2);
        assert_eq!(glob.parameter_expansions[0].text, "${prefix:-src}");
        assert_eq!(glob.parameter_expansions[0].name, "prefix");
        assert_eq!(glob.parameter_expansions[0].operator.as_deref(), Some(":-"));
        assert_eq!(glob.parameter_expansions[0].word.as_deref(), Some("src"));
        assert_eq!(glob.parameter_expansions[1].text, "${file:-*.rs}");
        assert_eq!(glob.parameter_expansions[1].name, "file");

        assert_eq!(regex.kind, ConditionalPatternKind::Regex);
        assert_eq!(regex.parameter_expansions.len(), 1);
        assert_eq!(regex.parameter_expansions[0].text, "${re:-^a+$}");
        assert_eq!(regex.parameter_expansions[0].name, "re");
        assert_eq!(regex.parameter_expansions[0].word.as_deref(), Some("^a+$"));
    }

    #[test]
    fn test_conditional_command_records_arithmetic_pattern_operand() {
        let glob_tokens = tokenize("[[ file == item-$((i+=1))-$[j*2] ]]");
        let glob_ast = parse(&glob_tokens);
        let regex_tokens = tokenize("[[ value =~ ^$((start+1))$ ]]");
        let regex_ast = parse(&regex_tokens);

        let glob = glob_ast.commands[0]
            .conditional_command
            .as_ref()
            .unwrap()
            .expression
            .pattern_operand
            .as_ref()
            .unwrap();
        let regex = regex_ast.commands[0]
            .conditional_command
            .as_ref()
            .unwrap()
            .expression
            .pattern_operand
            .as_ref()
            .unwrap();

        assert_eq!(glob.kind, ConditionalPatternKind::Glob);
        assert_eq!(glob.arithmetic_expansions.len(), 2);
        assert_eq!(glob.arithmetic_expansions[0].text, "$((i+=1))");
        assert_eq!(glob.arithmetic_expansions[0].expression, "i+=1");
        assert!(glob.arithmetic_expansions[0].has_assignment);
        assert_eq!(glob.arithmetic_expansions[0].variables, ["i"]);
        assert_eq!(glob.arithmetic_expansions[1].open_delimiter, "$[");
        assert_eq!(glob.arithmetic_expansions[1].expression, "j*2");
        assert_eq!(glob.arithmetic_expansions[1].operators[0].text, "*");

        assert_eq!(regex.kind, ConditionalPatternKind::Regex);
        assert_eq!(regex.arithmetic_expansions.len(), 1);
        assert_eq!(regex.arithmetic_expansions[0].text, "$((start+1))");
        assert_eq!(regex.arithmetic_expansions[0].variables, ["start"]);
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
    fn test_conditional_command_keeps_process_substitution_operand() {
        let input = "[[ -e <(:) ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();

        assert_eq!(conditional.args, ["-e", "<(:)", "]]"]);
        assert_eq!(ast.commands[0].words, ["[[", "-e", "<(:)", "]]"]);
        assert_eq!(
            conditional.expression.kind,
            ConditionalExpressionKind::Unary
        );
        assert_eq!(conditional.expression.operator.as_deref(), Some("-e"));
        assert_eq!(conditional.expression.operands, ["<(:)"]);
    }

    #[test]
    fn test_conditional_command_keeps_quoted_closing_delimiter_word() {
        let input = "[[ value == \"]]\" ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();

        assert_eq!(conditional.args, ["value", "==", "]]", "]]"]);
        assert_eq!(conditional.close_delimiter, "]]");
        assert_eq!(
            conditional.expression.kind,
            ConditionalExpressionKind::Binary
        );
        assert_eq!(conditional.expression.operator.as_deref(), Some("=="));
        assert_eq!(conditional.expression.operands, ["value", "]]"]);
    }

    #[test]
    fn test_conditional_command_records_arg_metadata() {
        let input = "[[ $value == \"*.rs\" && $((count+1)) -gt 2 ]]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let conditional = ast.commands[0].conditional_command.as_ref().unwrap();

        assert_eq!(
            conditional.args,
            [
                "$value",
                "==",
                "*.rs",
                "&&",
                "$((count+1))",
                "-gt",
                "2",
                "]]"
            ]
        );
        assert_eq!(conditional.arg_metadata.len(), conditional.args.len());
        assert_eq!(conditional.arg_metadata[0].word_index, 0);
        assert_eq!(conditional.arg_metadata[0].value, "$value");
        assert_eq!(conditional.arg_metadata[0].parameter_expansions.len(), 1);
        assert_eq!(
            conditional.arg_metadata[0].parameter_expansions[0].name,
            "value"
        );

        assert_eq!(conditional.arg_metadata[2].value, "*.rs");
        assert_eq!(conditional.arg_metadata[2].raw, "\"*.rs\"");
        assert!(conditional.arg_metadata[2].pathname_patterns.is_empty());
        assert_eq!(conditional.arg_metadata[2].word_quotes.len(), 1);
        assert_eq!(
            conditional.arg_metadata[2].word_quotes[0].kind,
            QuoteKind::Double
        );

        assert_eq!(conditional.arg_metadata[4].arithmetic_expansions.len(), 1);
        assert_eq!(
            conditional.arg_metadata[4].arithmetic_expansions[0].expression,
            "count+1"
        );
        assert_eq!(
            conditional.arg_metadata[4].arithmetic_expansions[0].variables,
            ["count"]
        );
        assert_eq!(conditional.arg_metadata[7].value, "]]");
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
        assert_eq!(case_command.keyword_metadata.value, "case");
        assert_eq!(case_command.keyword_metadata.raw, "case");
        assert_eq!(case_command.word, "$word");
        assert_eq!(case_command.word_metadata.value, "$word");
        assert_eq!(case_command.word_metadata.word_index, 0);
        assert_eq!(case_command.word_metadata.parameter_expansions.len(), 1);
        assert_eq!(
            case_command.word_metadata.parameter_expansions[0].text,
            "$word"
        );
        assert_eq!(
            case_command.word_metadata.parameter_expansions[0].name,
            "word"
        );
        assert_eq!(case_command.in_keyword, "in");
        assert_eq!(case_command.in_keyword_metadata.value, "in");
        assert_eq!(case_command.in_keyword_metadata.raw, "in");
        assert_eq!(case_command.end_keyword, "esac");
        assert_eq!(case_command.end_keyword_metadata.value, "esac");
        assert_eq!(case_command.end_keyword_metadata.raw, "esac");
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
            case_command.clauses[0]
                .terminator_metadata
                .as_ref()
                .unwrap()
                .value,
            ";&"
        );
        assert_eq!(
            case_command.clauses[0].pattern_open_delimiter.as_deref(),
            Some("(")
        );
        assert_eq!(
            case_command.clauses[0]
                .pattern_open_delimiter_metadata
                .as_ref()
                .unwrap()
                .value,
            "("
        );
        assert_eq!(
            case_command.clauses[0].patterns,
            ["x", "@(foo|bar)", "!(tmp)"]
        );
        assert_eq!(case_command.clauses[0].pattern_separators, ["|", "|"]);
        assert_eq!(case_command.clauses[0].pattern_separator_metadata.len(), 2);
        assert_eq!(
            case_command.clauses[0].pattern_separator_metadata[0].value,
            "|"
        );
        assert_eq!(
            case_command.clauses[0].pattern_separator_metadata[1].word_index,
            1
        );
        assert_eq!(case_command.clauses[0].pattern_close_delimiter, ")");
        assert_eq!(
            case_command.clauses[0]
                .pattern_close_delimiter_metadata
                .value,
            ")"
        );

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
        assert_eq!(patterns[1].operator_metadata[0].value, "@(");
        assert_eq!(patterns[1].operator_metadata[0].raw, "@(");
        assert_eq!(patterns[1].operator_metadata[1].value, "|");
        assert_eq!(patterns[1].operator_metadata[1].raw, "|");
        assert_eq!(patterns[1].extglob_patterns.len(), 1);
        assert_eq!(patterns[1].extglob_patterns[0].text, "@(foo|bar)");
        assert_eq!(patterns[1].extglob_patterns[0].alternatives, ["foo", "bar"]);
        assert!(patterns[1].has_extglob);
        assert!(!patterns[1].negated_extglob);
        assert_eq!(patterns[2].text, "!(tmp)");
        assert_eq!(patterns[2].operators, ["!("]);
        assert_eq!(patterns[2].extglob_patterns.len(), 1);
        assert_eq!(patterns[2].extglob_patterns[0].operator, '!');
        assert_eq!(patterns[2].extglob_patterns[0].pattern, "tmp");
        assert!(patterns[2].has_extglob);
        assert!(patterns[2].negated_extglob);

        let fallback = &case_command.clauses[1].pattern_nodes[0];
        assert_eq!(fallback.text, "*");
        assert_eq!(fallback.clause_index, 1);
        assert_eq!(fallback.pattern_index, 0);
        assert_eq!(fallback.operators, ["*"]);
        assert_eq!(fallback.operator_metadata[0].value, "*");
        assert_eq!(fallback.operator_metadata[0].raw, "*");
        assert!(fallback.has_glob);
        assert_eq!(case_command.clauses[1].pattern_open_delimiter, None);
        assert!(case_command.clauses[1]
            .pattern_open_delimiter_metadata
            .is_none());
        assert!(case_command.clauses[1].pattern_separators.is_empty());
        assert_eq!(case_command.clauses[1].pattern_close_delimiter, ")");
        assert_eq!(
            case_command.clauses[1].pattern_close_delimiter_metadata.raw,
            ")"
        );
        assert_eq!(
            case_command.clauses[1].terminator_text.as_deref(),
            Some(";;")
        );
        assert_eq!(
            case_command.clauses[1]
                .terminator_metadata
                .as_ref()
                .unwrap()
                .raw,
            ";;"
        );
    }

    #[test]
    fn test_case_clause_records_absent_terminator() {
        let input = "case $word in x) echo hit; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let clause = &ast.commands[0].case_command.as_ref().unwrap().clauses[0];

        assert_eq!(clause.terminator, CaseTerminator::Break);
        assert_eq!(clause.terminator_text, None);
        assert!(clause.terminator_metadata.is_none());
    }

    #[test]
    fn test_case_word_keeps_process_substitution() {
        let input = "case <(:) in /dev/*) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        assert_eq!(case_command.word, "<(:)");
        assert_eq!(case_command.in_keyword, "in");
        assert_eq!(case_command.clauses.len(), 1);
    }

    #[test]
    fn test_case_word_records_quote_metadata() {
        let input = "case \"*.rs\" in *.rs) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        assert_eq!(case_command.word, "*.rs");
        assert_eq!(case_command.word_metadata.value, "*.rs");
        assert_eq!(case_command.word_metadata.raw, "\"*.rs\"");
        assert!(case_command.word_metadata.pathname_patterns.is_empty());
        assert_eq!(case_command.word_metadata.word_quotes.len(), 1);
        assert_eq!(case_command.word_metadata.word_quotes[0].text, "\"*.rs\"");
        assert_eq!(case_command.word_metadata.word_quotes[0].body, "*.rs");
        assert_eq!(
            case_command.word_metadata.word_quotes[0].kind,
            QuoteKind::Double
        );
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
        assert_eq!(first.operator_metadata[0].value, "[ab]");
        assert_eq!(first.operator_metadata[0].raw, "[ab]");
        assert_eq!(first.operator_metadata[1].value, "?");
        assert_eq!(first.operator_metadata[2].value, "?");
        assert!(first.has_glob);
        assert!(!first.has_extglob);

        let second = &case_command.clauses[1].pattern_nodes[0];
        assert_eq!(second.text, "**/*.sh");
        assert_eq!(second.operators, ["**", "*"]);
        assert_eq!(second.operator_metadata[0].value, "**");
        assert_eq!(second.operator_metadata[0].raw, "**");
        assert_eq!(second.operator_metadata[1].value, "*");
        assert!(second.has_glob);
    }

    #[test]
    fn test_case_pattern_ignores_quoted_glob_operators() {
        let input = "case $word in \"*\"|'?'|$'[ab]'|plain\"*\"?.rs) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();
        let patterns = &case_command.clauses[0].pattern_nodes;

        assert_eq!(patterns[0].text, "*");
        assert_eq!(patterns[0].raw_text, "\"*\"");
        assert!(patterns[0].operators.is_empty());
        assert!(!patterns[0].has_glob);
        assert_eq!(patterns[1].text, "?");
        assert!(patterns[1].operators.is_empty());
        assert!(!patterns[1].has_glob);
        assert_eq!(patterns[2].text, "[ab]");
        assert!(patterns[2].operators.is_empty());
        assert!(!patterns[2].has_glob);
        assert_eq!(patterns[3].text, "plain*?.rs");
        assert_eq!(patterns[3].operators, ["?"]);
        assert!(patterns[3].has_glob);
    }

    #[test]
    fn test_case_pattern_records_nested_extglob_nodes() {
        let input = "case $word in @(a|+(b|c))) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        let pattern = &case_command.clauses[0].pattern_nodes[0];
        assert_eq!(pattern.text, "@(a|+(b|c))");
        assert_eq!(pattern.operators, ["@(", "|", "+(", "|"]);
        assert_eq!(pattern.operator_metadata[0].value, "@(");
        assert_eq!(pattern.operator_metadata[1].value, "|");
        assert_eq!(pattern.operator_metadata[2].value, "+(");
        assert_eq!(pattern.operator_metadata[3].value, "|");
        assert_eq!(pattern.extglob_patterns.len(), 2);
        assert_eq!(pattern.extglob_patterns[0].text, "@(a|+(b|c))");
        assert_eq!(pattern.extglob_patterns[0].alternatives, ["a", "+(b|c)"]);
        assert_eq!(pattern.extglob_patterns[1].text, "+(b|c)");
        assert_eq!(pattern.extglob_patterns[1].alternatives, ["b", "c"]);
        assert!(pattern.has_extglob);
    }

    #[test]
    fn test_case_pattern_keeps_command_substitution_and_brace_expansion() {
        let input = "case $word in $(printf x)|{a,b}) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        let clause = &case_command.clauses[0];
        assert_eq!(clause.patterns, ["$(printf x)", "{a,b}"]);
        assert_eq!(clause.pattern_separators, ["|"]);
        assert_eq!(clause.pattern_nodes[0].text, "$(printf x)");
        assert_eq!(clause.pattern_nodes[1].text, "{a,b}");
        assert!(clause.pattern_nodes[0].brace_expansions.is_empty());
        assert_eq!(clause.pattern_nodes[1].brace_expansions.len(), 1);
        assert_eq!(clause.pattern_nodes[1].brace_expansions[0].text, "{a,b}");
        assert_eq!(clause.pattern_nodes[1].brace_expansions[0].operators, [","]);
    }

    #[test]
    fn test_case_pattern_records_nested_brace_expansion_nodes() {
        let input = "case $word in {a,{b,c}}) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        let pattern = &case_command.clauses[0].pattern_nodes[0];
        assert_eq!(pattern.text, "{a,{b,c}}");
        assert_eq!(pattern.brace_expansions.len(), 2);
        assert_eq!(pattern.brace_expansions[0].text, "{a,{b,c}}");
        assert_eq!(pattern.brace_expansions[0].body, "a,{b,c}");
        assert_eq!(pattern.brace_expansions[1].text, "{b,c}");
        assert_eq!(pattern.brace_expansions[1].body, "b,c");
    }

    #[test]
    fn test_case_pattern_records_parameter_expansion_nodes() {
        let input = "case $word in ${prefix:-src}/*|${outer:-${inner}}) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        let first = &case_command.clauses[0].pattern_nodes[0];
        assert_eq!(first.text, "${prefix:-src}/*");
        assert_eq!(first.parameter_expansions.len(), 1);
        assert_eq!(first.parameter_expansions[0].text, "${prefix:-src}");
        assert_eq!(first.parameter_expansions[0].name, "prefix");
        assert_eq!(
            first.parameter_expansions[0].operator.as_deref(),
            Some(":-")
        );
        assert_eq!(first.parameter_expansions[0].word.as_deref(), Some("src"));

        let second = &case_command.clauses[0].pattern_nodes[1];
        assert_eq!(second.text, "${outer:-${inner}}");
        assert_eq!(second.parameter_expansions.len(), 2);
        assert_eq!(second.parameter_expansions[0].text, "${outer:-${inner}}");
        assert_eq!(second.parameter_expansions[0].name, "outer");
        assert_eq!(second.parameter_expansions[1].text, "${inner}");
        assert_eq!(second.parameter_expansions[1].name, "inner");
    }

    #[test]
    fn test_case_pattern_records_arithmetic_expansion_nodes() {
        let input = "case $word in $((i+=1))|$[j*2]) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();

        let first = &case_command.clauses[0].pattern_nodes[0];
        assert_eq!(first.text, "$((i+=1))");
        assert_eq!(first.arithmetic_expansions.len(), 1);
        assert_eq!(first.arithmetic_expansions[0].text, "$((i+=1))");
        assert_eq!(first.arithmetic_expansions[0].expression, "i+=1");
        assert!(first.arithmetic_expansions[0].has_assignment);
        assert_eq!(first.arithmetic_expansions[0].variables, ["i"]);

        let second = &case_command.clauses[0].pattern_nodes[1];
        assert_eq!(second.text, "$[j*2]");
        assert_eq!(second.arithmetic_expansions.len(), 1);
        assert_eq!(second.arithmetic_expansions[0].open_delimiter, "$[");
        assert_eq!(second.arithmetic_expansions[0].expression, "j*2");
        assert_eq!(second.arithmetic_expansions[0].operators[0].text, "*");
    }

    #[test]
    fn test_case_pattern_keeps_reserved_word_text() {
        let input = "case done in done|fi|esac) echo hit ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();
        let clause = &case_command.clauses[0];

        assert_eq!(case_command.word, "done");
        assert_eq!(clause.patterns, ["done", "fi", "esac"]);
        assert_eq!(clause.pattern_separators, ["|", "|"]);
    }

    #[test]
    fn test_case_pattern_can_start_with_esac_text() {
        let single_tokens = tokenize("case esac in esac) echo single ;; esac");
        let single_ast = parse(&single_tokens);
        let single_case = single_ast.commands[0].case_command.as_ref().unwrap();

        assert_eq!(single_case.word, "esac");
        assert_eq!(single_case.clauses.len(), 1);
        assert_eq!(single_case.clauses[0].patterns, ["esac"]);
        assert_eq!(single_case.clauses[0].body[0].words, ["echo", "single"]);

        let multi_tokens = tokenize("case esac in esac|fi) echo multi ;; esac");
        let multi_ast = parse(&multi_tokens);
        let multi_case = multi_ast.commands[0].case_command.as_ref().unwrap();

        assert_eq!(multi_case.clauses.len(), 1);
        assert_eq!(multi_case.clauses[0].patterns, ["esac", "fi"]);
        assert_eq!(multi_case.clauses[0].pattern_separators, ["|"]);
        assert_eq!(multi_case.clauses[0].body[0].words, ["echo", "multi"]);
    }

    #[test]
    fn test_case_body_keeps_reserved_word_arguments() {
        let input = "case x in x) echo if for done ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();
        let clause = &case_command.clauses[0];

        assert_eq!(clause.body.len(), 1);
        assert_eq!(clause.body[0].words, ["echo", "if", "for", "done"]);
        assert_eq!(clause.terminator_text.as_deref(), Some(";;"));
    }

    #[test]
    fn test_case_body_keeps_quoted_terminator_words() {
        let input = "case $x in x) echo \";;\"; echo after ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();
        let clause = &case_command.clauses[0];

        assert_eq!(case_command.clauses.len(), 1);
        assert_eq!(clause.terminator, CaseTerminator::Break);
        assert_eq!(clause.body.len(), 2);
        assert_eq!(clause.body[0].words, ["echo", ";;"]);
        assert_eq!(clause.body[1].words, ["echo", "after"]);
    }

    #[test]
    fn test_case_body_keeps_esac_after_quoted_terminator_word() {
        let input = "case $x in x) echo \";;\" esac; echo after ;; esac";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let case_command = ast.commands[0].case_command.as_ref().unwrap();
        let clause = &case_command.clauses[0];

        assert_eq!(case_command.clauses.len(), 1);
        assert_eq!(clause.body.len(), 2);
        assert_eq!(clause.body[0].words, ["echo", ";;", "esac"]);
        assert_eq!(clause.body[1].words, ["echo", "after"]);
        assert_eq!(clause.terminator_text.as_deref(), Some(";;"));
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
        assert_eq!(arithmetic.open_delimiter_metadata.value, "((");
        assert_eq!(arithmetic.open_delimiter_metadata.raw, "((");
        assert_eq!(arithmetic.close_delimiter, "))");
        assert_eq!(arithmetic.close_delimiter_metadata.value, "))");
        assert_eq!(arithmetic.close_delimiter_metadata.raw, "))");
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
        assert_eq!(arithmetic.open_delimiter_metadata.value, "((");
        assert_eq!(arithmetic.close_delimiter, "))");
        assert_eq!(arithmetic.close_delimiter_metadata.value, "))");
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
    fn test_arithmetic_command_records_comma_operator() {
        let input = "(( i = 0, j = i + 1 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let arithmetic = ast.commands[0].arithmetic_command.as_ref().unwrap();

        assert_eq!(arithmetic.expression, "i = 0, j = i + 1");
        assert_eq!(arithmetic.variables, ["i", "j"]);
        assert!(arithmetic.has_assignment);
        let operators = arithmetic
            .operators
            .iter()
            .map(|operator| operator.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(operators, ["=", ",", "=", "+"]);
    }

    #[test]
    fn test_arithmetic_command_records_exponent_assignment_operator() {
        let input = "(( n **= 3 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let arithmetic = ast.commands[0].arithmetic_command.as_ref().unwrap();

        assert_eq!(arithmetic.expression, "n **= 3");
        assert_eq!(arithmetic.variables, ["n"]);
        assert!(arithmetic.has_assignment);
        assert_eq!(arithmetic.operators.len(), 1);
        assert_eq!(arithmetic.operators[0].text, "**=");
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
        assert_eq!(compound[0].name_metadata.value, "arr");
        assert_eq!(compound[0].name_metadata.raw, "arr");
        assert_eq!(compound[0].value, "(one \"two words\")");
        assert_eq!(compound[0].operator, "=");
        assert_eq!(compound[0].operator_metadata.value, "=");
        assert_eq!(compound[0].operator_metadata.raw, "=");
        assert!(!compound[0].append);
        assert_eq!(compound[0].open_delimiter, "(");
        assert_eq!(compound[0].open_delimiter_metadata.value, "(");
        assert_eq!(compound[0].close_delimiter, ")");
        assert_eq!(compound[0].close_delimiter_metadata.raw, ")");
        assert_eq!(compound[0].word_index, None);
        assert_eq!(compound[0].elements.len(), 2);
        assert_eq!(compound[0].elements[0].subscript, None);
        assert_eq!(compound[0].elements[0].value, "one");
        assert_eq!(compound[0].elements[0].operator, None);
        assert_eq!(compound[0].elements[0].element_index, 0);
        assert_eq!(compound[0].elements[1].subscript, None);
        assert_eq!(compound[0].elements[1].value, "\"two words\"");
        assert_eq!(compound[0].elements[1].operator, None);
        assert_eq!(compound[0].elements[1].element_index, 1);
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
        assert_eq!(compound[0].name_metadata.value, "arr");
        assert_eq!(compound[0].value, "(three four)");
        assert_eq!(compound[0].operator, "+=");
        assert_eq!(compound[0].operator_metadata.value, "+=");
        assert!(compound[0].append);
        assert_eq!(compound[0].open_delimiter_metadata.raw, "(");
        assert_eq!(compound[0].close_delimiter_metadata.value, ")");
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
        assert_eq!(compound[0].elements[0].element_index, 0);
        assert_eq!(compound[0].elements[1].subscript.as_deref(), Some("name"));
        assert_eq!(compound[0].elements[1].value, "more");
        assert_eq!(compound[0].elements[1].operator.as_deref(), Some("+="));
        assert!(compound[0].elements[1].append);
        assert_eq!(compound[0].elements[1].element_index, 1);
        assert_eq!(compound[0].elements[2].subscript, None);
        assert_eq!(compound[0].elements[2].value, "plain");
        assert_eq!(compound[0].elements[2].operator, None);
        assert_eq!(compound[0].elements[2].element_index, 2);
    }

    #[test]
    fn test_compound_assignment_records_empty_indexed_elements() {
        let input = "arr=([empty]= [more]+=)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].name, "arr");
        assert_eq!(compound[0].value, "([empty]= [more]+=)");
        assert_eq!(compound[0].elements.len(), 2);
        assert_eq!(compound[0].elements[0].subscript.as_deref(), Some("empty"));
        assert_eq!(compound[0].elements[0].value, "");
        assert_eq!(compound[0].elements[0].operator.as_deref(), Some("="));
        assert!(!compound[0].elements[0].append);
        assert_eq!(compound[0].elements[1].subscript.as_deref(), Some("more"));
        assert_eq!(compound[0].elements[1].value, "");
        assert_eq!(compound[0].elements[1].operator.as_deref(), Some("+="));
        assert!(compound[0].elements[1].append);
    }

    #[test]
    fn test_compound_assignment_ignores_quoted_subscript_operators() {
        let input = "arr=([\"a]=b\"]=value [\"c]+=d\"]+=more)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].elements.len(), 2);
        assert_eq!(
            compound[0].elements[0].subscript.as_deref(),
            Some("\"a]=b\"")
        );
        assert_eq!(compound[0].elements[0].value, "value");
        assert_eq!(compound[0].elements[0].operator.as_deref(), Some("="));
        assert!(!compound[0].elements[0].append);
        assert_eq!(
            compound[0].elements[1].subscript.as_deref(),
            Some("\"c]+=d\"")
        );
        assert_eq!(compound[0].elements[1].value, "more");
        assert_eq!(compound[0].elements[1].operator.as_deref(), Some("+="));
        assert!(compound[0].elements[1].append);
    }

    #[test]
    fn test_compound_assignment_records_spaced_subscript_elements() {
        let input = "arr=([ \"a]=b\" ] = value [ \"c]+=d\" ] += more [ empty ] =)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].elements.len(), 3);
        assert_eq!(
            compound[0].elements[0].subscript.as_deref(),
            Some("\"a]=b\"")
        );
        assert_eq!(compound[0].elements[0].value, "value");
        assert_eq!(compound[0].elements[0].operator.as_deref(), Some("="));
        assert_eq!(
            compound[0].elements[1].subscript.as_deref(),
            Some("\"c]+=d\"")
        );
        assert_eq!(compound[0].elements[1].value, "more");
        assert_eq!(compound[0].elements[1].operator.as_deref(), Some("+="));
        assert!(compound[0].elements[1].append);
        assert_eq!(compound[0].elements[2].subscript.as_deref(), Some("empty"));
        assert_eq!(compound[0].elements[2].value, "");
        assert_eq!(compound[0].elements[2].operator.as_deref(), Some("="));
    }

    #[test]
    fn test_compound_assignment_keeps_reserved_word_elements() {
        let input = "arr=(if done [case]=esac [then]+=fi)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].value, "(if done [case]=esac [then]+=fi)");
        assert_eq!(compound[0].elements.len(), 4);
        assert_eq!(compound[0].elements[0].value, "if");
        assert_eq!(compound[0].elements[1].value, "done");
        assert_eq!(compound[0].elements[2].subscript.as_deref(), Some("case"));
        assert_eq!(compound[0].elements[2].value, "esac");
        assert_eq!(compound[0].elements[3].subscript.as_deref(), Some("then"));
        assert_eq!(compound[0].elements[3].value, "fi");
    }

    #[test]
    fn test_compound_assignment_keeps_brace_expansion_element() {
        let input = "arr=(pre{a,b})";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].name, "arr");
        assert_eq!(compound[0].value, "(pre{a,b})");
        assert_eq!(compound[0].elements.len(), 1);
        assert_eq!(compound[0].elements[0].value, "pre{a,b}");
        assert_eq!(compound[0].elements[0].brace_expansions.len(), 1);
        assert_eq!(compound[0].elements[0].brace_expansions[0].text, "{a,b}");
        assert_eq!(compound[0].elements[0].brace_expansions[0].body, "a,b");
        assert_eq!(compound[0].elements[0].brace_expansions[0].operators, [","]);
    }

    #[test]
    fn test_compound_assignment_records_extglob_elements() {
        let input = "arr=(@(foo|bar) [name]=+(test|bench))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].elements.len(), 2);

        let first = &compound[0].elements[0];
        assert_eq!(first.value, "@(foo|bar)");
        assert_eq!(first.extglob_patterns.len(), 1);
        assert_eq!(first.extglob_patterns[0].text, "@(foo|bar)");
        assert_eq!(first.extglob_patterns[0].operator, '@');
        assert_eq!(first.extglob_patterns[0].alternatives, ["foo", "bar"]);

        let second = &compound[0].elements[1];
        assert_eq!(second.subscript.as_deref(), Some("name"));
        assert_eq!(second.value, "+(test|bench)");
        assert_eq!(second.extglob_patterns.len(), 1);
        assert_eq!(second.extglob_patterns[0].operator, '+');
        assert_eq!(second.extglob_patterns[0].alternatives, ["test", "bench"]);
    }

    #[test]
    fn test_compound_assignment_records_pathname_pattern_elements() {
        let input = "arr=(*.rs [src]=src/[ab]? **/*.txt)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].elements.len(), 3);

        let first = &compound[0].elements[0];
        assert_eq!(first.value, "*.rs");
        assert_eq!(first.pathname_patterns.len(), 1);
        assert_eq!(first.pathname_patterns[0].text, "*.rs");
        assert_eq!(first.pathname_patterns[0].operators, ["*"]);
        assert!(first.pathname_patterns[0].has_star);

        let second = &compound[0].elements[1];
        assert_eq!(second.subscript.as_deref(), Some("src"));
        assert_eq!(second.value, "src/[ab]?");
        assert_eq!(second.pathname_patterns.len(), 1);
        assert_eq!(second.pathname_patterns[0].operators, ["[ab]", "?"]);
        assert!(second.pathname_patterns[0].has_bracket);
        assert!(second.pathname_patterns[0].has_question);

        let globstar = &compound[0].elements[2];
        assert_eq!(globstar.value, "**/*.txt");
        assert_eq!(globstar.pathname_patterns.len(), 1);
        assert_eq!(globstar.pathname_patterns[0].operators, ["**", "*"]);
        assert!(globstar.pathname_patterns[0].globstar);
    }

    #[test]
    fn test_compound_assignment_records_tilde_and_quote_elements() {
        let input = "arr=(~/src [home]=~+/bin \"two words\" 'line two')";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].elements.len(), 4);

        let home = &compound[0].elements[0];
        assert_eq!(home.value, "~/src");
        assert_eq!(home.tilde_expansions.len(), 1);
        assert_eq!(home.tilde_expansions[0].text, "~/src");
        assert_eq!(home.tilde_expansions[0].prefix, "~");
        assert_eq!(home.tilde_expansions[0].suffix, "/src");

        let indexed = &compound[0].elements[1];
        assert_eq!(indexed.subscript.as_deref(), Some("home"));
        assert_eq!(indexed.value, "~+/bin");
        assert_eq!(indexed.tilde_expansions.len(), 1);
        assert_eq!(indexed.tilde_expansions[0].prefix, "~+");
        assert_eq!(indexed.tilde_expansions[0].suffix, "/bin");

        let quoted = &compound[0].elements[2];
        assert_eq!(quoted.value, "\"two words\"");
        assert!(quoted.tilde_expansions.is_empty());
        assert_eq!(quoted.word_quotes.len(), 1);
        assert_eq!(quoted.word_quotes[0].text, "\"two words\"");
        assert_eq!(quoted.word_quotes[0].body, "two words");
        assert_eq!(quoted.word_quotes[0].kind, QuoteKind::Double);

        let single_quoted = &compound[0].elements[3];
        assert_eq!(single_quoted.value, "\"line two\"");
        assert_eq!(single_quoted.word_quotes.len(), 1);
        assert_eq!(single_quoted.word_quotes[0].text, "\"line two\"");
        assert_eq!(single_quoted.word_quotes[0].body, "line two");
        assert_eq!(single_quoted.word_quotes[0].kind, QuoteKind::Double);
    }

    #[test]
    fn test_compound_assignment_records_subscript_expansions() {
        let input = "arr=([${key:-fallback}]=value [$((i+1))]+=next [pre{a,b}]=brace plain)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].elements.len(), 4);

        let parameter = &compound[0].elements[0];
        assert_eq!(parameter.subscript.as_deref(), Some("${key:-fallback}"));
        assert_eq!(parameter.subscript_parameter_expansions.len(), 1);
        assert_eq!(
            parameter.subscript_parameter_expansions[0].text,
            "${key:-fallback}"
        );
        assert_eq!(parameter.subscript_parameter_expansions[0].name, "key");
        assert_eq!(
            parameter.subscript_parameter_expansions[0]
                .operator
                .as_deref(),
            Some(":-")
        );
        assert_eq!(
            parameter.subscript_parameter_expansions[0].word.as_deref(),
            Some("fallback")
        );

        let arithmetic = &compound[0].elements[1];
        assert_eq!(arithmetic.subscript.as_deref(), Some("$((i+1))"));
        assert!(arithmetic.append);
        assert_eq!(arithmetic.subscript_arithmetic_expansions.len(), 1);
        assert_eq!(
            arithmetic.subscript_arithmetic_expansions[0].text,
            "$((i+1))"
        );
        assert_eq!(
            arithmetic.subscript_arithmetic_expansions[0].expression,
            "i+1"
        );
        assert_eq!(
            arithmetic.subscript_arithmetic_expansions[0].variables,
            ["i"]
        );

        let brace = &compound[0].elements[2];
        assert_eq!(brace.subscript.as_deref(), Some("pre{a,b}"));
        assert_eq!(brace.subscript_brace_expansions.len(), 1);
        assert_eq!(brace.subscript_brace_expansions[0].text, "{a,b}");
        assert_eq!(brace.subscript_brace_expansions[0].body, "a,b");

        let plain = &compound[0].elements[3];
        assert_eq!(plain.value, "plain");
        assert!(plain.subscript_parameter_expansions.is_empty());
        assert!(plain.subscript_arithmetic_expansions.is_empty());
        assert!(plain.subscript_brace_expansions.is_empty());
    }

    #[test]
    fn test_compound_assignment_keeps_process_substitution_elements() {
        let input = "arr=(<(:) >(:) [two]=<(:))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(compound[0].value, "(<(:) >(:) [two]=<(:))");
        assert_eq!(compound[0].elements.len(), 3);
        assert_eq!(compound[0].elements[0].value, "<(:)");
        assert_eq!(compound[0].elements[1].value, ">(:)");
        assert_eq!(compound[0].elements[2].subscript.as_deref(), Some("two"));
        assert_eq!(compound[0].elements[2].value, "<(:)");
    }

    #[test]
    fn test_compound_assignment_preserves_single_quoted_elements_and_subscripts() {
        let input = "arr=('one two' [key]='value here' plain)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(
            compound[0].value,
            "(\"one two\" [key]=\"value here\" plain)"
        );
        assert_eq!(compound[0].elements.len(), 3);
        assert_eq!(compound[0].elements[0].subscript, None);
        assert_eq!(compound[0].elements[0].value, "\"one two\"");
        assert_eq!(compound[0].elements[1].subscript.as_deref(), Some("key"));
        assert_eq!(compound[0].elements[1].operator.as_deref(), Some("="));
        assert_eq!(compound[0].elements[1].value, "\"value here\"");
        assert_eq!(compound[0].elements[2].value, "plain");
    }

    #[test]
    fn test_compound_assignment_preserves_nested_expansion_elements() {
        let input = "arr=($(printf \"a b\") ${value:-five six} $[count + 1])";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let compound = ast.commands[0].compound_assignments.as_slice();
        assert_eq!(compound.len(), 1);
        assert_eq!(
            compound[0].value,
            "(\"$(printf \\\"a b\\\")\" \"${value:-five six}\" \"$[count + 1]\")"
        );
        assert_eq!(compound[0].elements.len(), 3);
        assert_eq!(compound[0].elements[0].value, "\"$(printf \\\"a b\\\")\"");
        assert_eq!(compound[0].elements[1].value, "\"${value:-five six}\"");
        assert_eq!(compound[0].elements[1].parameter_expansions.len(), 1);
        assert_eq!(
            compound[0].elements[1].parameter_expansions[0].text,
            "${value:-five six}"
        );
        assert_eq!(
            compound[0].elements[1].parameter_expansions[0].name,
            "value"
        );
        assert_eq!(
            compound[0].elements[1].parameter_expansions[0]
                .operator
                .as_deref(),
            Some(":-")
        );
        assert_eq!(
            compound[0].elements[1].parameter_expansions[0]
                .word
                .as_deref(),
            Some("five six")
        );
        assert_eq!(compound[0].elements[2].value, "\"$[count + 1]\"");
        assert_eq!(compound[0].elements[2].arithmetic_expansions.len(), 1);
        assert_eq!(
            compound[0].elements[2].arithmetic_expansions[0].open_delimiter,
            "$["
        );
        assert_eq!(
            compound[0].elements[2].arithmetic_expansions[0].expression,
            "count + 1"
        );
        assert_eq!(
            compound[0].elements[2].arithmetic_expansions[0].variables,
            ["count"]
        );
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
        assert_eq!(elements[0].name_metadata.value, "arr");
        assert_eq!(elements[0].open_delimiter, "[");
        assert_eq!(elements[0].open_delimiter_metadata.raw, "[");
        assert_eq!(elements[0].subscript, "0");
        assert_eq!(elements[0].subscript_metadata.value, "0");
        assert_eq!(elements[0].close_delimiter, "]");
        assert_eq!(elements[0].close_delimiter_metadata.value, "]");
        assert_eq!(elements[0].value, "zero");
        assert_eq!(elements[0].operator, "=");
        assert_eq!(elements[0].operator_metadata.value, "=");
        assert!(!elements[0].append);
        assert_eq!(elements[0].word_index, Some(0));
        assert_eq!(elements[1].name, "arr");
        assert_eq!(elements[1].name_metadata.raw, "arr");
        assert_eq!(elements[1].subscript, "i+1");
        assert_eq!(elements[1].subscript_metadata.value, "i+1");
        assert_eq!(elements[1].value, "more");
        assert_eq!(elements[1].operator, "+=");
        assert_eq!(elements[1].operator_metadata.raw, "+=");
        assert!(elements[1].append);
        assert_eq!(elements[1].word_index, Some(1));
    }

    #[test]
    fn test_array_element_assignment_records_expansions() {
        let input = "echo arr[${key:-fallback}]=$value arr[$((i+1))]+=pre{a,b} nums[0]=$((n+1))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let elements = ast.commands[0].array_element_assignments.as_slice();
        assert_eq!(elements.len(), 3);

        let parameter = &elements[0];
        assert_eq!(parameter.name, "arr");
        assert_eq!(parameter.subscript, "${key:-fallback}");
        assert_eq!(parameter.value, "$value");
        assert_eq!(parameter.word_index, Some(1));
        assert_eq!(parameter.subscript_parameter_expansions.len(), 1);
        assert_eq!(
            parameter.subscript_parameter_expansions[0].text,
            "${key:-fallback}"
        );
        assert_eq!(parameter.subscript_parameter_expansions[0].name, "key");
        assert_eq!(parameter.parameter_expansions.len(), 1);
        assert_eq!(parameter.parameter_expansions[0].text, "$value");
        assert_eq!(parameter.parameter_expansions[0].name, "value");

        let arithmetic = &elements[1];
        assert_eq!(arithmetic.subscript, "$((i+1))");
        assert!(arithmetic.append);
        assert_eq!(arithmetic.subscript_arithmetic_expansions.len(), 1);
        assert_eq!(
            arithmetic.subscript_arithmetic_expansions[0].expression,
            "i+1"
        );
        assert_eq!(
            arithmetic.subscript_arithmetic_expansions[0].variables,
            ["i"]
        );
        assert_eq!(arithmetic.brace_expansions.len(), 1);
        assert_eq!(arithmetic.brace_expansions[0].text, "{a,b}");
        assert_eq!(arithmetic.brace_expansions[0].body, "a,b");

        let value_arithmetic = &elements[2];
        assert_eq!(value_arithmetic.name, "nums");
        assert_eq!(value_arithmetic.subscript, "0");
        assert_eq!(value_arithmetic.value, "$((n+1))");
        assert_eq!(value_arithmetic.arithmetic_expansions.len(), 1);
        assert_eq!(value_arithmetic.arithmetic_expansions[0].expression, "n+1");
        assert_eq!(value_arithmetic.arithmetic_expansions[0].variables, ["n"]);
        assert!(value_arithmetic.subscript_parameter_expansions.is_empty());
        assert!(value_arithmetic.subscript_arithmetic_expansions.is_empty());
        assert!(value_arithmetic.subscript_brace_expansions.is_empty());
    }

    #[test]
    fn test_array_element_assignment_records_pattern_and_quote_metadata() {
        let input = "echo home[0]=~+/bin glob[1]=src/[ab]? ext[2]=@(src|tests) quoted[3]=\"*.rs\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let elements = ast.commands[0].array_element_assignments.as_slice();
        assert_eq!(elements.len(), 4);

        let home = &elements[0];
        assert_eq!(home.name, "home");
        assert_eq!(home.value, "~+/bin");
        assert_eq!(home.tilde_expansions.len(), 1);
        assert_eq!(home.tilde_expansions[0].prefix, "~+");
        assert_eq!(home.tilde_expansions[0].suffix, "/bin");

        let glob = &elements[1];
        assert_eq!(glob.name, "glob");
        assert_eq!(glob.value, "src/[ab]?");
        assert_eq!(glob.pathname_patterns.len(), 1);
        assert_eq!(glob.pathname_patterns[0].operators, ["[ab]", "?"]);
        assert!(glob.pathname_patterns[0].has_bracket);
        assert!(glob.pathname_patterns[0].has_question);

        let extglob = &elements[2];
        assert_eq!(extglob.name, "ext");
        assert_eq!(extglob.value, "@(src|tests)");
        assert_eq!(extglob.extglob_patterns.len(), 1);
        assert_eq!(extglob.extglob_patterns[0].operator, '@');
        assert_eq!(extglob.extglob_patterns[0].alternatives, ["src", "tests"]);

        let quoted = &elements[3];
        assert_eq!(quoted.name, "quoted");
        assert_eq!(quoted.value, "*.rs");
        assert!(quoted.pathname_patterns.is_empty());
        assert_eq!(quoted.word_quotes.len(), 1);
        assert_eq!(quoted.word_quotes[0].text, "\"*.rs\"");
        assert_eq!(quoted.word_quotes[0].body, "*.rs");
        assert_eq!(quoted.word_quotes[0].kind, QuoteKind::Double);
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
        assert_eq!(substitutions[0].open_delimiter_metadata.value, "$(");
        assert_eq!(substitutions[0].open_delimiter_metadata.raw, "$(");
        assert_eq!(substitutions[0].operator, "$");
        assert_eq!(substitutions[0].operator_metadata.value, "$");
        assert_eq!(substitutions[0].operator_metadata.raw, "$");
        assert_eq!(substitutions[0].close_delimiter, ")");
        assert_eq!(substitutions[0].close_delimiter_metadata.value, ")");
        assert_eq!(substitutions[0].close_delimiter_metadata.raw, ")");
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
        assert_eq!(substitutions[2].open_delimiter_metadata.value, "`");
        assert_eq!(substitutions[2].operator, "`");
        assert_eq!(substitutions[2].operator_metadata.value, "`");
        assert_eq!(substitutions[2].operator_metadata.raw, "`");
        assert_eq!(substitutions[2].close_delimiter, "`");
        assert_eq!(substitutions[2].close_delimiter_metadata.value, "`");
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
    fn test_command_substitution_keeps_inner_ansi_c_escaped_quote() {
        let input = "echo \"$(printf $'foo\\'\nbar')\"";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let substitutions = ast.commands[0].command_substitutions.as_slice();

        assert_eq!(ast.commands[0].words, ["echo", "$(printf $'foo\\'\nbar')"]);
        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].text, "$(printf $'foo\\'\nbar')");
        assert_eq!(substitutions[0].source, "printf $'foo\\'\nbar'");
        assert_eq!(substitutions[0].commands[0].words, ["printf", "foo'\nbar"]);
    }

    #[test]
    fn test_empty_command_substitution_records_null_body() {
        let input = "echo before$()after";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let substitutions = ast.commands[0].command_substitutions.as_slice();

        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].text, "$()");
        assert_eq!(substitutions[0].open_delimiter, "$(");
        assert_eq!(substitutions[0].operator, "$");
        assert_eq!(substitutions[0].close_delimiter, ")");
        assert_eq!(substitutions[0].source, "");
        assert!(substitutions[0].commands.is_empty());
        assert_eq!(substitutions[0].word_index, Some(1));
        assert_eq!(substitutions[0].assignment_name, None);
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
    fn test_command_substitution_keeps_case_pattern_starting_with_esac() {
        let input = "echo $(case esac in\nesac) printf matched ;; esac)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let substitutions = ast.commands[0].command_substitutions.as_slice();

        assert_eq!(substitutions.len(), 1);
        assert_eq!(
            substitutions[0].source,
            "case esac in\nesac) printf matched ;; esac"
        );
        let case_command = substitutions[0].commands[0].case_command.as_ref().unwrap();
        assert_eq!(case_command.word, "esac");
        assert_eq!(case_command.clauses[0].patterns, ["esac"]);
        assert_eq!(case_command.clauses[0].body[0].words, ["printf", "matched"]);
    }

    #[test]
    fn test_command_substitution_keeps_case_argument() {
        let input = "echo $(echo case; echo ok)";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let substitutions = ast.commands[0].command_substitutions.as_slice();

        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].source, "echo case; echo ok");
        assert_eq!(substitutions[0].commands.len(), 2);
        assert_eq!(substitutions[0].commands[0].words, ["echo", "case"]);
        assert_eq!(substitutions[0].commands[1].words, ["echo", "ok"]);
    }

    #[test]
    fn test_braced_command_substitution_records_current_shell_ast() {
        let input = "echo ${ echo hi; }";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let command = &ast.commands[0];
        let substitutions = command.command_substitutions.as_slice();

        assert_eq!(command.words, ["echo", "${ echo hi; }"]);
        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].text, "${ echo hi; }");
        assert_eq!(substitutions[0].open_delimiter, "${");
        assert_eq!(substitutions[0].operator, "${");
        assert_eq!(substitutions[0].operator_metadata.value, "${");
        assert_eq!(substitutions[0].operator_metadata.raw, "${");
        assert_eq!(substitutions[0].close_delimiter, "}");
        assert_eq!(substitutions[0].source, " echo hi; ");
        assert!(!substitutions[0].backtick);
        assert!(substitutions[0].current_shell);
        assert!(!substitutions[0].pipe_output);
        assert_eq!(substitutions[0].commands[0].words, ["echo", "hi"]);
        assert!(command.parameter_expansions.is_empty());
    }

    #[test]
    fn test_pipe_braced_command_substitution_records_operator() {
        let input = "echo ${| REPLY=hi; } ${USER:-guest}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let command = &ast.commands[0];
        let substitutions = command.command_substitutions.as_slice();

        assert_eq!(substitutions.len(), 1);
        assert_eq!(substitutions[0].text, "${| REPLY=hi; }");
        assert_eq!(substitutions[0].open_delimiter, "${");
        assert_eq!(substitutions[0].operator, "${|");
        assert_eq!(substitutions[0].operator_metadata.value, "${|");
        assert_eq!(substitutions[0].operator_metadata.raw, "${|");
        assert_eq!(substitutions[0].close_delimiter, "}");
        assert_eq!(substitutions[0].source, " REPLY=hi; ");
        assert!(substitutions[0].current_shell);
        assert!(substitutions[0].pipe_output);
        assert_eq!(substitutions[0].commands[0].assignments["REPLY"], "hi");
        assert_eq!(command.parameter_expansions.len(), 1);
        assert_eq!(command.parameter_expansions[0].text, "${USER:-guest}");
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
        assert_eq!(expansions[0].open_delimiter_metadata.value, "$((");
        assert_eq!(expansions[0].open_delimiter_metadata.raw, "$((");
        assert_eq!(expansions[0].close_delimiter, "))");
        assert_eq!(expansions[0].close_delimiter_metadata.value, "))");
        assert_eq!(expansions[0].close_delimiter_metadata.raw, "))");
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
    fn test_arithmetic_expansion_records_comma_operator() {
        let input = "echo $(( i = 0, j = i + 1 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let expansion = &ast.commands[0].arithmetic_expansions[0];

        assert_eq!(expansion.expression, " i = 0, j = i + 1 ");
        assert_eq!(expansion.variables, ["i", "j"]);
        assert!(expansion.has_assignment);
        let operators = expansion
            .operators
            .iter()
            .map(|operator| operator.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(operators, ["=", ",", "=", "+"]);
    }

    #[test]
    fn test_arithmetic_expansion_records_exponent_assignment_operator() {
        let input = "echo $(( n **= 3 ))";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let expansion = &ast.commands[0].arithmetic_expansions[0];

        assert_eq!(expansion.expression, " n **= 3 ");
        assert_eq!(expansion.variables, ["n"]);
        assert!(expansion.has_assignment);
        assert_eq!(expansion.operators.len(), 1);
        assert_eq!(expansion.operators[0].text, "**=");
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

    #[test]
    fn test_bracket_arithmetic_expansion_records_structured_ast() {
        let input = "echo $[count += 1] value=$[array[0] + 2]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let command = &ast.commands[0];

        assert_eq!(
            command.words,
            ["echo", "$[count += 1]", "value=$[array[0] + 2]"]
        );
        let expansions = command.arithmetic_expansions.as_slice();
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].text, "$[count += 1]");
        assert_eq!(expansions[0].open_delimiter, "$[");
        assert_eq!(expansions[0].open_delimiter_metadata.value, "$[");
        assert_eq!(expansions[0].close_delimiter, "]");
        assert_eq!(expansions[0].close_delimiter_metadata.raw, "]");
        assert_eq!(expansions[0].expression, "count += 1");
        assert_eq!(expansions[0].variables, ["count"]);
        assert!(expansions[0].has_assignment);
        assert_eq!(expansions[0].operators[0].text, "+=");
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[0].assignment_name, None);
        assert_eq!(expansions[1].text, "$[array[0] + 2]");
        assert_eq!(expansions[1].expression, "array[0] + 2");
        assert_eq!(expansions[1].word_index, Some(2));
        assert_eq!(expansions[1].assignment_name.as_deref(), Some("value"));
        assert!(command.pathname_patterns.is_empty());
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
        assert_eq!(expansions[0].open_delimiter_metadata.value, "$");
        assert_eq!(expansions[0].open_delimiter_metadata.raw, "$");
        assert_eq!(expansions[0].close_delimiter, "");
        assert_eq!(expansions[0].close_delimiter_metadata.value, "");
        assert_eq!(expansions[0].close_delimiter_metadata.raw, "");
        assert_eq!(expansions[0].parameter, "HOME");
        assert_eq!(expansions[0].name, "HOME");
        assert_eq!(expansions[0].operator, None);
        assert!(!expansions[0].operator_prefix);
        assert_eq!(expansions[0].word, None);
        assert!(!expansions[0].braced);
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[1].text, "${USER:-guest}");
        assert_eq!(expansions[1].open_delimiter, "${");
        assert_eq!(expansions[1].open_delimiter_metadata.value, "${");
        assert_eq!(expansions[1].open_delimiter_metadata.raw, "${");
        assert_eq!(expansions[1].close_delimiter, "}");
        assert_eq!(expansions[1].close_delimiter_metadata.value, "}");
        assert_eq!(expansions[1].close_delimiter_metadata.raw, "}");
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

    #[test]
    fn test_parameter_expansion_records_name_prefix_match_suffix() {
        let input = "echo ${!prefix*} ${!prefix@} ${!array[@]}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 3);
        assert_eq!(expansions[0].name, "prefix");
        assert_eq!(expansions[0].operator.as_deref(), Some("!"));
        assert!(expansions[0].operator_prefix);
        assert_eq!(expansions[0].word.as_deref(), Some("*"));
        assert_eq!(expansions[1].name, "prefix");
        assert_eq!(expansions[1].operator.as_deref(), Some("!"));
        assert!(expansions[1].operator_prefix);
        assert_eq!(expansions[1].word.as_deref(), Some("@"));
        assert_eq!(expansions[2].name, "array[@]");
        assert_eq!(expansions[2].operator.as_deref(), Some("!"));
        assert!(expansions[2].operator_prefix);
        assert_eq!(expansions[2].word, None);
    }

    #[test]
    fn test_parameter_expansion_keeps_braced_special_parameters_as_names() {
        let input = "echo ${#} ${!} ${?} ${-} ${#array[@]} ${!name}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 6);
        assert_eq!(expansions[0].name, "#");
        assert_eq!(expansions[0].operator, None);
        assert!(!expansions[0].operator_prefix);
        assert_eq!(expansions[1].name, "!");
        assert_eq!(expansions[1].operator, None);
        assert!(!expansions[1].operator_prefix);
        assert_eq!(expansions[2].name, "?");
        assert_eq!(expansions[2].operator, None);
        assert_eq!(expansions[3].name, "-");
        assert_eq!(expansions[3].operator, None);
        assert_eq!(expansions[4].name, "array[@]");
        assert_eq!(expansions[4].operator.as_deref(), Some("#"));
        assert!(expansions[4].operator_prefix);
        assert_eq!(expansions[5].name, "name");
        assert_eq!(expansions[5].operator.as_deref(), Some("!"));
        assert!(expansions[5].operator_prefix);
    }

    #[test]
    fn test_parameter_expansion_records_transform_operators() {
        let input = "echo ${name^} ${name^^[a-z]} ${name,} ${name,,[A-Z]} ${path~glob} ${name@Q}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 6);
        assert_eq!(expansions[0].name, "name");
        assert_eq!(expansions[0].operator.as_deref(), Some("^"));
        assert_eq!(expansions[0].word.as_deref(), Some(""));
        assert_eq!(expansions[1].name, "name");
        assert_eq!(expansions[1].operator.as_deref(), Some("^^"));
        assert_eq!(expansions[1].word.as_deref(), Some("[a-z]"));
        assert_eq!(expansions[2].name, "name");
        assert_eq!(expansions[2].operator.as_deref(), Some(","));
        assert_eq!(expansions[2].word.as_deref(), Some(""));
        assert_eq!(expansions[3].name, "name");
        assert_eq!(expansions[3].operator.as_deref(), Some(",,"));
        assert_eq!(expansions[3].word.as_deref(), Some("[A-Z]"));
        assert_eq!(expansions[4].name, "path");
        assert_eq!(expansions[4].operator.as_deref(), Some("~"));
        assert_eq!(expansions[4].word.as_deref(), Some("glob"));
        assert_eq!(expansions[5].name, "name");
        assert_eq!(expansions[5].operator.as_deref(), Some("@"));
        assert_eq!(expansions[5].word.as_deref(), Some("Q"));
    }

    #[test]
    fn test_parameter_expansion_records_substring_operator() {
        let input = "echo ${name:1} ${name:1:3} ${array[@]: -2:1} ${value:-fallback}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 4);
        assert_eq!(expansions[0].name, "name");
        assert_eq!(expansions[0].operator.as_deref(), Some(":"));
        assert_eq!(expansions[0].word.as_deref(), Some("1"));
        assert_eq!(expansions[1].name, "name");
        assert_eq!(expansions[1].operator.as_deref(), Some(":"));
        assert_eq!(expansions[1].word.as_deref(), Some("1:3"));
        assert_eq!(expansions[2].name, "array[@]");
        assert_eq!(expansions[2].operator.as_deref(), Some(":"));
        assert_eq!(expansions[2].word.as_deref(), Some(" -2:1"));
        assert_eq!(expansions[3].name, "value");
        assert_eq!(expansions[3].operator.as_deref(), Some(":-"));
        assert_eq!(expansions[3].word.as_deref(), Some("fallback"));
    }

    #[test]
    fn test_parameter_expansion_records_nested_expansions() {
        let input =
            "echo ${outer:-${inner:-fallback}} ${name/${old:-x}/${new:-y}} ${array[${idx:-0}]}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 7);
        assert_eq!(expansions[0].text, "${outer:-${inner:-fallback}}");
        assert_eq!(expansions[0].name, "outer");
        assert_eq!(expansions[0].operator.as_deref(), Some(":-"));
        assert_eq!(expansions[0].word.as_deref(), Some("${inner:-fallback}"));
        assert_eq!(expansions[1].text, "${inner:-fallback}");
        assert_eq!(expansions[1].name, "inner");
        assert_eq!(expansions[1].operator.as_deref(), Some(":-"));
        assert_eq!(expansions[2].text, "${name/${old:-x}/${new:-y}}");
        assert_eq!(expansions[2].name, "name");
        assert_eq!(expansions[2].operator.as_deref(), Some("/"));
        assert_eq!(expansions[2].word.as_deref(), Some("${old:-x}/${new:-y}"));
        assert_eq!(expansions[3].text, "${old:-x}");
        assert_eq!(expansions[3].name, "old");
        assert_eq!(expansions[4].text, "${new:-y}");
        assert_eq!(expansions[4].name, "new");
        assert_eq!(expansions[5].text, "${array[${idx:-0}]}");
        assert_eq!(expansions[5].name, "array[${idx:-0}]");
        assert_eq!(expansions[5].operator, None);
        assert_eq!(expansions[6].text, "${idx:-0}");
        assert_eq!(expansions[6].name, "idx");
    }

    #[test]
    fn test_parameter_expansion_does_not_treat_array_at_as_transform_operator() {
        let input = "echo ${array[@]} ${array[*]@Q}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].name, "array[@]");
        assert_eq!(expansions[0].operator, None);
        assert_eq!(expansions[0].word, None);
        assert_eq!(expansions[1].name, "array[*]");
        assert_eq!(expansions[1].operator.as_deref(), Some("@"));
        assert_eq!(expansions[1].word.as_deref(), Some("Q"));
    }

    #[test]
    fn test_parameter_expansion_keeps_inner_ansi_c_escaped_quote() {
        let input = "echo ${v:-$'foo\\'\nbar'}";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].parameter_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "${v:-$'foo\\'\nbar'}");
        assert_eq!(expansions[0].parameter, "v:-$'foo\\'\nbar'");
        assert_eq!(expansions[0].name, "v");
        assert_eq!(expansions[0].operator.as_deref(), Some(":-"));
        assert_eq!(expansions[0].word.as_deref(), Some("$'foo\\'\nbar'"));
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
        assert_eq!(expansions[0].open_delimiter_metadata.value, "{");
        assert_eq!(expansions[0].open_delimiter_metadata.raw, "{");
        assert_eq!(expansions[0].close_delimiter, "}");
        assert_eq!(expansions[0].close_delimiter_metadata.value, "}");
        assert_eq!(expansions[0].close_delimiter_metadata.raw, "}");
        assert_eq!(expansions[0].body, "a,b");
        assert_eq!(expansions[0].operators, [","]);
        assert_eq!(expansions[0].operator_metadata[0].value, ",");
        assert_eq!(expansions[0].operator_metadata[0].raw, ",");
        assert!(!expansions[0].range);
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[0].assignment_name, None);
        assert_eq!(expansions[1].text, "{1..3}");
        assert_eq!(expansions[1].open_delimiter, "{");
        assert_eq!(expansions[1].open_delimiter_metadata.value, "{");
        assert_eq!(expansions[1].open_delimiter_metadata.raw, "{");
        assert_eq!(expansions[1].close_delimiter, "}");
        assert_eq!(expansions[1].close_delimiter_metadata.value, "}");
        assert_eq!(expansions[1].close_delimiter_metadata.raw, "}");
        assert_eq!(expansions[1].body, "1..3");
        assert_eq!(expansions[1].operators, [".."]);
        assert_eq!(expansions[1].operator_metadata[0].value, "..");
        assert_eq!(expansions[1].operator_metadata[0].raw, "..");
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
        assert_eq!(expansions[0].operator_metadata[0].value, ",");
        assert_eq!(expansions[0].operator_metadata[1].value, ",");
        assert!(!expansions[0].range);
    }

    #[test]
    fn test_brace_expansion_records_nested_expansions() {
        let input = "echo {a,{b,c}} pre{1..3,{x,y}}post";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].brace_expansions.as_slice();
        assert_eq!(expansions.len(), 4);
        assert_eq!(expansions[0].text, "{a,{b,c}}");
        assert_eq!(expansions[0].body, "a,{b,c}");
        assert_eq!(expansions[0].operators, [","]);
        assert_eq!(expansions[0].word_index, Some(1));
        assert_eq!(expansions[1].text, "{b,c}");
        assert_eq!(expansions[1].body, "b,c");
        assert_eq!(expansions[1].operators, [","]);
        assert_eq!(expansions[1].word_index, Some(1));
        assert_eq!(expansions[2].text, "{1..3,{x,y}}");
        assert_eq!(expansions[2].body, "1..3,{x,y}");
        assert_eq!(expansions[2].operators, ["..", ","]);
        assert!(!expansions[2].range);
        assert_eq!(expansions[2].word_index, Some(2));
        assert_eq!(expansions[3].text, "{x,y}");
        assert_eq!(expansions[3].body, "x,y");
        assert_eq!(expansions[3].operators, [","]);
        assert_eq!(expansions[3].word_index, Some(2));
    }

    #[test]
    fn test_quoted_brace_text_does_not_record_brace_expansion() {
        let input = "echo {a,b} $'{c,d}' '{e,f}'";
        let tokens = tokenize(input);
        let ast = parse(&tokens);

        let expansions = ast.commands[0].brace_expansions.as_slice();
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].text, "{a,b}");
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
        assert_eq!(patterns.len(), 4);
        assert_eq!(patterns[0].text, "@(alpha|beta)");
        assert_eq!(patterns[0].open_delimiter, "@(");
        assert_eq!(patterns[0].open_delimiter_metadata.value, "@(");
        assert_eq!(patterns[0].open_delimiter_metadata.raw, "@(");
        assert_eq!(patterns[0].close_delimiter, ")");
        assert_eq!(patterns[0].close_delimiter_metadata.value, ")");
        assert_eq!(patterns[0].close_delimiter_metadata.raw, ")");
        assert_eq!(patterns[0].operator, '@');
        assert_eq!(patterns[0].operator_metadata.value, "@");
        assert_eq!(patterns[0].operator_metadata.raw, "@");
        assert_eq!(patterns[0].pattern, "alpha|beta");
        assert_eq!(patterns[0].operators, ["|"]);
        assert_eq!(patterns[0].alternative_operator_metadata[0].value, "|");
        assert_eq!(patterns[0].alternative_operator_metadata[0].raw, "|");
        assert_eq!(patterns[0].alternatives, ["alpha", "beta"]);
        assert_eq!(patterns[0].word_index, Some(1));
        assert_eq!(patterns[1].text, "!(.tmp)");
        assert_eq!(patterns[1].open_delimiter, "!(");
        assert_eq!(patterns[1].open_delimiter_metadata.value, "!(");
        assert_eq!(patterns[1].open_delimiter_metadata.raw, "!(");
        assert_eq!(patterns[1].close_delimiter, ")");
        assert_eq!(patterns[1].close_delimiter_metadata.value, ")");
        assert_eq!(patterns[1].close_delimiter_metadata.raw, ")");
        assert_eq!(patterns[1].operator, '!');
        assert_eq!(patterns[1].operator_metadata.value, "!");
        assert_eq!(patterns[1].operator_metadata.raw, "!");
        assert!(patterns[1].operators.is_empty());
        assert_eq!(patterns[1].alternatives, [".tmp"]);
        assert_eq!(patterns[1].word_index, Some(2));
        assert_eq!(patterns[2].text, "@(a|+(b|c))");
        assert_eq!(patterns[2].pattern, "a|+(b|c)");
        assert_eq!(patterns[2].operators, ["|"]);
        assert_eq!(patterns[2].alternative_operator_metadata[0].value, "|");
        assert_eq!(patterns[2].alternative_operator_metadata[0].raw, "|");
        assert_eq!(patterns[2].alternatives, ["a", "+(b|c)"]);
        assert_eq!(patterns[2].word_index, Some(3));
        assert_eq!(patterns[3].text, "+(b|c)");
        assert_eq!(patterns[3].pattern, "b|c");
        assert_eq!(patterns[3].operators, ["|"]);
        assert_eq!(patterns[3].alternatives, ["b", "c"]);
        assert_eq!(patterns[3].word_index, Some(3));
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
        assert_eq!(patterns[0].alternative_operator_metadata[0].value, "|");
        assert_eq!(patterns[0].alternative_operator_metadata[1].value, "|");
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
    fn test_quoted_extglob_pattern_text_is_not_recorded() {
        let input =
            "echo @(plain|pattern) '@(single|quoted)' \"!(double|quoted)\" $'+(ansi|quoted)'";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].extglob_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "@(plain|pattern)");
        assert_eq!(patterns[0].word_index, Some(1));
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
        let input = "echo ~ ~/src ~+ ~- ~+1/stack ~user/bin literal~";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].tilde_expansions.as_slice();
        assert_eq!(expansions.len(), 6);
        assert_eq!(expansions[0].text, "~");
        assert_eq!(expansions[0].open_delimiter, "~");
        assert_eq!(expansions[0].open_delimiter_metadata.value, "~");
        assert_eq!(expansions[0].open_delimiter_metadata.raw, "~");
        assert_eq!(expansions[0].close_delimiter, "");
        assert_eq!(expansions[0].close_delimiter_metadata.value, "");
        assert_eq!(expansions[0].close_delimiter_metadata.raw, "");
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
        assert_eq!(expansions[2].open_delimiter_metadata.value, "~");
        assert_eq!(expansions[2].open_delimiter_metadata.raw, "~");
        assert_eq!(expansions[2].close_delimiter, "");
        assert_eq!(expansions[2].close_delimiter_metadata.value, "");
        assert_eq!(expansions[2].close_delimiter_metadata.raw, "");
        assert_eq!(expansions[2].word_index, Some(3));
        assert_eq!(expansions[3].prefix, "~-");
        assert_eq!(expansions[3].word_index, Some(4));
        assert_eq!(expansions[4].text, "~+1/stack");
        assert_eq!(expansions[4].prefix, "~+1");
        assert_eq!(expansions[4].suffix, "/stack");
        assert_eq!(expansions[4].word_index, Some(5));
        assert_eq!(expansions[5].text, "~user/bin");
        assert_eq!(expansions[5].prefix, "~user");
        assert_eq!(expansions[5].suffix, "/bin");
        assert_eq!(expansions[5].word_index, Some(6));
    }

    #[test]
    fn test_assignment_tilde_expansion_records_colon_segments() {
        let input = "PATH=~/bin:~+/sbin:~+1/lib echo target=~-/tmp";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let expansions = ast.commands[0].tilde_expansions.as_slice();
        assert_eq!(expansions.len(), 4);
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
        assert_eq!(expansions[2].assignment_name.as_deref(), Some("PATH"));
        assert_eq!(expansions[2].text, "~+1/lib");
        assert_eq!(expansions[2].prefix, "~+1");
        assert_eq!(expansions[2].suffix, "/lib");
        assert_eq!(expansions[2].word_index, None);
        assert!(expansions[2].after_colon);
        assert_eq!(expansions[3].assignment_name.as_deref(), Some("target"));
        assert_eq!(expansions[3].text, "~-/tmp");
        assert_eq!(expansions[3].prefix, "~-");
        assert_eq!(expansions[3].suffix, "/tmp");
        assert_eq!(expansions[3].word_index, Some(1));
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
        assert_eq!(patterns[0].operator_metadata[0].value, "*");
        assert_eq!(patterns[0].operator_metadata[0].raw, "*");
        assert!(patterns[0].has_star);
        assert!(!patterns[0].has_question);
        assert!(!patterns[0].has_bracket);
        assert!(!patterns[0].globstar);
        assert_eq!(patterns[0].word_index, Some(1));
        assert_eq!(patterns[1].text, "src/[mp]*.rs");
        assert_eq!(patterns[1].operators, ["[mp]", "*"]);
        assert_eq!(patterns[1].operator_metadata[0].value, "[mp]");
        assert_eq!(patterns[1].operator_metadata[0].raw, "[mp]");
        assert_eq!(patterns[1].operator_metadata[1].value, "*");
        assert_eq!(patterns[1].operator_metadata[1].raw, "*");
        assert!(patterns[1].has_star);
        assert!(patterns[1].has_bracket);
        assert_eq!(patterns[1].word_index, Some(2));
        assert_eq!(patterns[2].text, "docs/??.md");
        assert_eq!(patterns[2].operators, ["?", "?"]);
        assert_eq!(patterns[2].operator_metadata[0].value, "?");
        assert_eq!(patterns[2].operator_metadata[0].raw, "?");
        assert_eq!(patterns[2].operator_metadata[1].value, "?");
        assert_eq!(patterns[2].operator_metadata[1].raw, "?");
        assert!(patterns[2].has_question);
        assert_eq!(patterns[2].word_index, Some(3));
        assert_eq!(patterns[3].text, "src/**/mod.rs");
        assert_eq!(patterns[3].operators, ["**"]);
        assert_eq!(patterns[3].operator_metadata[0].value, "**");
        assert_eq!(patterns[3].operator_metadata[0].raw, "**");
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

    #[test]
    fn test_quoted_pathname_operators_are_not_recorded() {
        let input = "echo \"*.rs\" '\\?' $'[*]' $\"**\" plain*.rs";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].pathname_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "plain*.rs");
        assert_eq!(patterns[0].operators, ["*"]);
        assert_eq!(patterns[0].word_index, Some(5));
    }

    #[test]
    fn test_only_unquoted_pathname_operators_are_recorded() {
        let input = "echo prefix\"*\"?.rs";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);

        let patterns = ast.commands[0].pathname_patterns.as_slice();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].text, "prefix*?.rs");
        assert_eq!(patterns[0].operators, ["?"]);
        assert!(!patterns[0].has_star);
        assert!(patterns[0].has_question);
        assert_eq!(patterns[0].word_index, Some(1));
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
        assert_eq!(quotes[0].open_delimiter_metadata.value, "'");
        assert_eq!(quotes[0].open_delimiter_metadata.raw, "'");
        assert_eq!(quotes[0].body, "one two");
        assert_eq!(quotes[0].close_delimiter, "'");
        assert_eq!(quotes[0].close_delimiter_metadata.value, "'");
        assert_eq!(quotes[0].close_delimiter_metadata.raw, "'");
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
        assert_eq!(quotes[2].open_delimiter_metadata.value, "$'");
        assert_eq!(quotes[2].open_delimiter_metadata.raw, "$'");
        assert_eq!(quotes[2].body, "line\\n");
        assert_eq!(quotes[2].close_delimiter, "'");
        assert_eq!(quotes[2].close_delimiter_metadata.value, "'");
        assert_eq!(quotes[2].close_delimiter_metadata.raw, "'");
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
        assert_eq!(background.operator_metadata.value, "&");
        assert_eq!(background.operator_metadata.raw, "&");
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
        assert_eq!(background.operator_metadata.value, "&");
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
        assert_eq!(inverted.operator_metadata.value, "!");
        assert_eq!(inverted.operator_metadata.raw, "!");
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
        assert_eq!(inverted.operator_metadata.value, "!");
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

    #[test]
    fn test_inverted_command_wraps_function_definition() {
        let keyword_tokens = tokenize("! function f { echo hi; }");
        let keyword_ast = parse(&keyword_tokens);
        let keyword_inverted = keyword_ast.commands[0].inverted_command.as_ref().unwrap();
        let keyword_function = keyword_inverted.command.function_command.as_ref().unwrap();

        assert_eq!(keyword_inverted.operator, "!");
        assert!(keyword_function.keyword);
        assert_eq!(keyword_function.name, "f");
        assert_eq!(keyword_function.body[0].words, ["echo", "hi"]);

        let posix_tokens = tokenize("! f() { echo hi; }");
        let posix_ast = parse(&posix_tokens);
        let posix_inverted = posix_ast.commands[0].inverted_command.as_ref().unwrap();
        let posix_function = posix_inverted.command.function_command.as_ref().unwrap();

        assert_eq!(posix_inverted.operator, "!");
        assert!(!posix_function.keyword);
        assert!(posix_function.has_parentheses);
        assert_eq!(posix_function.name, "f");
        assert_eq!(posix_function.body[0].words, ["echo", "hi"]);
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
