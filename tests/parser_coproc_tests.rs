use rubash::lexer::tokenize;
use rubash::parser::{parse, CoprocBodyKind, LoopKind, QuoteKind};

#[test]
fn test_named_coproc_parses_split_brace_group_body() {
    let input = "coproc MYC { echo hi; }";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.keyword, "coproc");
    assert_eq!(coproc.keyword_metadata.value, "coproc");
    assert_eq!(coproc.keyword_metadata.raw, "coproc");
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    let name_metadata = coproc.name_metadata.as_ref().unwrap();
    assert_eq!(name_metadata.word_index, 0);
    assert_eq!(name_metadata.value, "MYC");
    assert_eq!(name_metadata.raw, "MYC");
    assert!(name_metadata.word_quotes.is_empty());
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::BraceGroup);
    assert_eq!(coproc.body_open_delimiter.as_deref(), Some("{"));
    assert_eq!(
        coproc.body_open_delimiter_metadata.as_ref().unwrap().value,
        "{"
    );
    assert_eq!(coproc.body_close_delimiter.as_deref(), Some("}"));
    assert_eq!(
        coproc.body_close_delimiter_metadata.as_ref().unwrap().raw,
        "}"
    );
    let body = coproc.body.as_ref().unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].words, ["echo", "hi"]);
}

#[test]
fn test_unnamed_coproc_parses_split_brace_group_body() {
    let input = "coproc { echo hi; }";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.keyword, "coproc");
    assert_eq!(coproc.name, None);
    assert!(coproc.name_metadata.is_none());
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::BraceGroup);
    assert_eq!(coproc.body_open_delimiter.as_deref(), Some("{"));
    assert_eq!(coproc.body_close_delimiter.as_deref(), Some("}"));
    assert_eq!(coproc.body.as_ref().unwrap()[0].words, ["echo", "hi"]);
}

#[test]
fn test_coproc_brace_body_keeps_case_pattern_named_like_close_brace() {
    let input = "coproc { case brace in }) echo close ;; esac; echo after; }";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let body = coproc.body.as_ref().unwrap();

    assert_eq!(
        body[0].case_command.as_ref().unwrap().clauses[0].patterns,
        ["}"]
    );
    assert_eq!(body[1].words, ["echo", "after"]);
}

#[test]
fn test_coproc_simple_command_does_not_treat_first_word_as_name() {
    let input = "coproc MYC cat";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.keyword, "coproc");
    assert_eq!(coproc.name, None);
    assert!(coproc.name_metadata.is_none());
    assert_eq!(coproc.words, ["MYC", "cat"]);
    assert_eq!(coproc.body_kind, CoprocBodyKind::SimpleCommand);
    assert_eq!(coproc.body_open_delimiter, None);
    assert!(coproc.body_open_delimiter_metadata.is_none());
    assert_eq!(coproc.body_close_delimiter, None);
    assert!(coproc.body_close_delimiter_metadata.is_none());
    assert!(coproc.body.is_none());
}

#[test]
fn test_coproc_simple_command_keeps_process_substitution_words() {
    let input = "coproc cat <(:) >(:)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name, None);
    assert_eq!(coproc.words, ["cat", "<(:)", ">(:)"]);
    assert_eq!(coproc.body_kind, CoprocBodyKind::SimpleCommand);
}

#[test]
fn test_coproc_simple_command_records_word_metadata() {
    let input = "coproc echo $value \"*.rs\" pre{a,b}";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.words, ["echo", "$value", "*.rs", "pre{a,b}"]);
    assert_eq!(coproc.word_metadata.len(), 4);
    assert_eq!(coproc.word_metadata[1].value, "$value");
    assert_eq!(coproc.word_metadata[1].parameter_expansions.len(), 1);
    assert_eq!(
        coproc.word_metadata[1].parameter_expansions[0].name,
        "value"
    );

    assert_eq!(coproc.word_metadata[2].raw, "\"*.rs\"");
    assert!(coproc.word_metadata[2].pathname_patterns.is_empty());
    assert_eq!(coproc.word_metadata[2].word_quotes.len(), 1);
    assert_eq!(
        coproc.word_metadata[2].word_quotes[0].kind,
        QuoteKind::Double
    );

    assert_eq!(coproc.word_metadata[3].brace_expansions.len(), 1);
    assert_eq!(coproc.word_metadata[3].brace_expansions[0].text, "{a,b}");
    assert_eq!(coproc.word_metadata[3].brace_expansions[0].body, "a,b");
}

#[test]
fn test_coproc_simple_command_keeps_raw_quoted_arguments() {
    let input = "coproc printf '<%s>\\n' 'a b'";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();

    assert_eq!(coproc.words, ["printf", "<%s>\\n", "a b"]);
    assert_eq!(coproc.word_metadata[1].raw, "'<%s>\\n'");
    assert_eq!(coproc.word_metadata[2].raw, "'a b'");
}

#[test]
fn test_coproc_simple_command_keeps_reserved_word_arguments() {
    let input = "coproc echo alpha if done esac";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name, None);
    assert_eq!(coproc.words, ["echo", "alpha", "if", "done", "esac"]);
    assert_eq!(coproc.body_kind, CoprocBodyKind::SimpleCommand);
}

#[test]
fn test_named_coproc_parses_subshell_body() {
    let input = "coproc MYC ( echo hi )";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::Subshell);
    assert_eq!(coproc.body_open_delimiter.as_deref(), Some("("));
    assert_eq!(
        coproc.body_open_delimiter_metadata.as_ref().unwrap().value,
        "("
    );
    assert_eq!(coproc.body_close_delimiter.as_deref(), Some(")"));
    assert_eq!(
        coproc.body_close_delimiter_metadata.as_ref().unwrap().value,
        ")"
    );
    assert_eq!(coproc.body.as_ref().unwrap()[0].words, ["echo", "hi"]);
}

#[test]
fn test_named_coproc_subshell_keeps_case_pattern_parentheses() {
    let input = "coproc MYC ( case beta in alpha) printf alpha ;; beta) printf beta ;; esac )";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::Subshell);
    assert_eq!(coproc.body_open_delimiter.as_deref(), Some("("));
    assert_eq!(coproc.body_close_delimiter.as_deref(), Some(")"));
    assert!(coproc.body.as_ref().unwrap()[0].case_command.is_some());
}

#[test]
fn test_named_coproc_subshell_keeps_case_argument() {
    let input = "coproc MYC ( echo case; echo ok )";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let body = coproc.body.as_ref().unwrap();

    assert_eq!(coproc.body_kind, CoprocBodyKind::Subshell);
    assert_eq!(body[0].words, ["echo", "case"]);
    assert_eq!(body[1].words, ["echo", "ok"]);
}

#[test]
fn test_named_coproc_parses_for_body() {
    let input = "coproc MYC for x in a b; do echo $x; done";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    let for_command = coproc.body.as_ref().unwrap()[0]
        .for_command
        .as_ref()
        .unwrap();
    assert_eq!(for_command.variable, "x");
    assert_eq!(for_command.words, ["a", "b"]);
    assert_eq!(for_command.body[0].words, ["echo", "$x"]);
}

#[test]
fn test_named_coproc_parses_time_prefixed_pipeline_body() {
    let input = "coproc MYC time echo hi | wc -c";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    let time_command = coproc.body.as_ref().unwrap()[0]
        .time_command
        .as_ref()
        .unwrap();
    assert_eq!(time_command.keyword, "time");
    let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.operators, ["|"]);
    assert_eq!(pipeline.stages[0].words, ["echo", "hi"]);
    assert_eq!(pipeline.stages[1].words, ["wc", "-c"]);
}

#[test]
fn test_named_coproc_time_pipeline_keeps_inversion_prefix() {
    let input = "coproc MYC time ! echo hi | wc -c";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    let time_command = coproc.body.as_ref().unwrap()[0]
        .time_command
        .as_ref()
        .unwrap();
    assert!(time_command.inverted);
    let pipeline = time_command.command.pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.operators, ["|"]);
    assert_eq!(pipeline.stages[0].words, ["echo", "hi"]);
    assert_eq!(pipeline.stages[1].words, ["wc", "-c"]);
}

#[test]
fn test_named_coproc_time_pipeline_allows_compound_later_stages() {
    let if_tokens = tokenize("coproc MYC time echo hi | if read value; then echo $value; fi");
    let if_ast = parse(&if_tokens);
    let subshell_tokens = tokenize("coproc MYC time echo hi | ( read value; echo $value )");
    let subshell_ast = parse(&subshell_tokens);
    let brace_tokens = tokenize("coproc MYC time echo hi | { read value; echo $value; }");
    let brace_ast = parse(&brace_tokens);

    let if_time = if_ast.commands[0]
        .coproc_command
        .as_ref()
        .unwrap()
        .body
        .as_ref()
        .unwrap()[0]
        .time_command
        .as_ref()
        .unwrap();
    let subshell_time = subshell_ast.commands[0]
        .coproc_command
        .as_ref()
        .unwrap()
        .body
        .as_ref()
        .unwrap()[0]
        .time_command
        .as_ref()
        .unwrap();
    let brace_time = brace_ast.commands[0]
        .coproc_command
        .as_ref()
        .unwrap()
        .body
        .as_ref()
        .unwrap()[0]
        .time_command
        .as_ref()
        .unwrap();

    let if_pipeline = if_time.command.pipeline_command.as_ref().unwrap();
    let subshell_pipeline = subshell_time.command.pipeline_command.as_ref().unwrap();
    let brace_pipeline = brace_time.command.pipeline_command.as_ref().unwrap();

    assert_eq!(if_pipeline.operators, ["|"]);
    assert_eq!(subshell_pipeline.operators, ["|"]);
    assert_eq!(brace_pipeline.operators, ["|"]);
    assert!(if_pipeline.stages[1].if_command.is_some());
    assert!(subshell_pipeline.stages[1].subshell_command.is_some());
    assert!(brace_pipeline.stages[1].brace_group.is_some());
}

#[test]
fn test_named_coproc_parses_time_prefixed_compound_body() {
    let cases = [
        ("coproc MYC time -p for x in a; do echo $x; done", "for"),
        ("coproc MYC time (( 1 ))", "arithmetic"),
        ("coproc MYC time { echo hi; }", "brace"),
    ];

    for (input, body_kind) in cases {
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
        let time_command = coproc.body.as_ref().expect(input)[0]
            .time_command
            .as_ref()
            .expect(input);

        assert_eq!(coproc.name.as_deref(), Some("MYC"), "{input}");
        assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand, "{input}");
        match body_kind {
            "for" => assert!(time_command.command.for_command.is_some(), "{input}"),
            "arithmetic" => assert!(time_command.command.arithmetic_command.is_some(), "{input}"),
            "brace" => assert!(time_command.command.brace_group.is_some(), "{input}"),
            _ => unreachable!(),
        }
    }
}

#[test]
fn test_named_coproc_parses_function_body() {
    let input = "coproc MYC foo() { echo hi; }";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let function = coproc.body.as_ref().unwrap()[0]
        .function_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(function.name, "foo");
    assert!(function.has_parentheses);
    assert_eq!(function.body[0].words, ["echo", "hi"]);
}

#[test]
fn test_named_coproc_parses_time_prefixed_function_body() {
    let input = "coproc MYC time foo() { echo hi; }";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let time_command = coproc.body.as_ref().unwrap()[0]
        .time_command
        .as_ref()
        .unwrap();
    let function = time_command.command.function_command.as_ref().unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(function.name, "foo");
    assert!(function.has_parentheses);
    assert_eq!(function.body[0].words, ["echo", "hi"]);
}

#[test]
fn test_named_coproc_parses_arithmetic_body() {
    let input = "coproc MYC (( 1 + 1 ))";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    let arithmetic = coproc.body.as_ref().unwrap()[0]
        .arithmetic_command
        .as_ref()
        .unwrap();
    assert_eq!(arithmetic.expression, "1 + 1");
}

#[test]
fn test_named_coproc_parses_conditional_body() {
    let input = "coproc MYC [[ $value == ok ]]";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    let conditional = coproc.body.as_ref().unwrap()[0]
        .conditional_command
        .as_ref()
        .unwrap();
    assert_eq!(conditional.args, ["$value", "==", "ok", "]]"]);
}

#[test]
fn test_named_coproc_parses_case_body() {
    let input = "coproc MYC case beta in alpha) echo alpha ;; beta) echo beta ;; esac";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let case_command = coproc.body.as_ref().unwrap()[0]
        .case_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(case_command.word, "beta");
    assert_eq!(case_command.clauses.len(), 2);
    assert_eq!(case_command.clauses[1].patterns, ["beta"]);
    assert_eq!(case_command.clauses[1].body[0].words, ["echo", "beta"]);
}

#[test]
fn test_named_coproc_parses_if_body() {
    let input = "coproc MYC if true; then echo yes; else echo no; fi";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let if_command = coproc.body.as_ref().unwrap()[0]
        .if_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(if_command.condition[0].words, ["true"]);
    assert_eq!(if_command.then_body[0].words, ["echo", "yes"]);
    assert_eq!(
        if_command.else_body.as_ref().unwrap()[0].words,
        ["echo", "no"]
    );
}

#[test]
fn test_named_coproc_parses_while_body() {
    let input = "coproc MYC while true; do echo loop; break; done";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let loop_command = coproc.body.as_ref().unwrap()[0]
        .loop_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(loop_command.kind, LoopKind::While);
    assert_eq!(loop_command.condition[0].words, ["true"]);
    assert_eq!(loop_command.body[0].words, ["echo", "loop"]);
    assert_eq!(loop_command.body[1].words, ["break"]);
}

#[test]
fn test_named_coproc_parses_until_body() {
    let input = "coproc MYC until false; do echo loop; break; done";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let loop_command = coproc.body.as_ref().unwrap()[0]
        .loop_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(loop_command.kind, LoopKind::Until);
    assert_eq!(loop_command.condition[0].words, ["false"]);
    assert_eq!(loop_command.body[0].words, ["echo", "loop"]);
    assert_eq!(loop_command.body[1].words, ["break"]);
}

#[test]
fn test_named_coproc_parses_select_body() {
    let input = "coproc MYC select choice in alpha beta; do echo $choice; break; done";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let select = coproc.body.as_ref().unwrap()[0]
        .select_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(select.variable, "choice");
    assert_eq!(select.words, ["alpha", "beta"]);
    assert_eq!(select.body[0].words, ["echo", "$choice"]);
}

#[test]
fn test_coproc_conditional_body_keeps_quoted_closing_delimiter_word() {
    let input = "coproc MYC [[ value == \"]]\" ]]";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    let conditional = coproc.body.as_ref().unwrap()[0]
        .conditional_command
        .as_ref()
        .unwrap();

    assert_eq!(coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(conditional.args, ["value", "==", "]]", "]]"]);
    assert_eq!(conditional.expression.operands, ["value", "]]"]);
}

#[test]
fn test_coproc_sequence_body_keeps_reserved_word_arguments() {
    let if_tokens = tokenize("coproc MYC if echo then; then echo fi; fi");
    let if_ast = parse(&if_tokens);
    let if_coproc = if_ast.commands[0].coproc_command.as_ref().unwrap();
    let if_command = if_coproc.body.as_ref().unwrap()[0]
        .if_command
        .as_ref()
        .unwrap();

    let loop_tokens = tokenize("coproc MYC while echo do; do echo done; break; done");
    let loop_ast = parse(&loop_tokens);
    let loop_coproc = loop_ast.commands[0].coproc_command.as_ref().unwrap();
    let loop_command = loop_coproc.body.as_ref().unwrap()[0]
        .loop_command
        .as_ref()
        .unwrap();

    let until_tokens = tokenize("coproc MYC until echo do; do echo done; break; done");
    let until_ast = parse(&until_tokens);
    let until_coproc = until_ast.commands[0].coproc_command.as_ref().unwrap();
    let until_command = until_coproc.body.as_ref().unwrap()[0]
        .loop_command
        .as_ref()
        .unwrap();

    assert_eq!(if_coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(if_command.condition[0].words, ["echo", "then"]);
    assert_eq!(if_command.then_body[0].words, ["echo", "fi"]);
    assert_eq!(loop_coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert_eq!(loop_command.condition[0].words, ["echo", "do"]);
    assert_eq!(loop_command.body[0].words, ["echo", "done"]);
    assert_eq!(until_coproc.body_kind, CoprocBodyKind::CompoundCommand);
    assert!(until_command.until);
    assert_eq!(until_command.condition[0].words, ["echo", "do"]);
    assert_eq!(until_command.body[0].words, ["echo", "done"]);
}

#[test]
fn test_coproc_command_consumes_pipe_stderr_operator() {
    let input = "coproc MYC { echo hi; } |& grep hi";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.operators, ["|&"]);
    assert_eq!(pipeline.stages[0].pipe, Some(2));
    assert!(pipeline.stages[0].coproc_command.is_some());
    assert_eq!(pipeline.stages[1].words, ["grep", "hi"]);
}

#[test]
fn test_coproc_command_consumes_and_or_connector() {
    let input = "coproc MYC { echo hi; } && echo done";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let list = ast.commands[0].and_or_list.as_ref().unwrap();
    assert_eq!(list.connectors, [true]);
    assert!(list.commands[0].coproc_command.is_some());
    assert_eq!(list.commands[1].words, ["echo", "done"]);
}

#[test]
fn test_coproc_command_consumes_trailing_stdout_redirect() {
    let input = "coproc MYC { echo hi; } > out.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let command = &ast.commands[0];
    let coproc = command.coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert_eq!(coproc.body_kind, CoprocBodyKind::BraceGroup);
    assert_eq!(command.redirect_out.as_ref().unwrap().target, "out.txt");
}
