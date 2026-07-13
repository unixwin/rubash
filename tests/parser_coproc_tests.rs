use rubash::lexer::tokenize;
use rubash::parser::{parse, CoprocBodyKind};

#[test]
fn test_named_coproc_parses_split_brace_group_body() {
    let input = "coproc MYC { echo hi; }";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.keyword, "coproc");
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::BraceGroup);
    assert_eq!(coproc.body_open_delimiter.as_deref(), Some("{"));
    assert_eq!(coproc.body_close_delimiter.as_deref(), Some("}"));
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
    assert_eq!(coproc.words, ["MYC", "cat"]);
    assert_eq!(coproc.body_kind, CoprocBodyKind::SimpleCommand);
    assert_eq!(coproc.body_open_delimiter, None);
    assert_eq!(coproc.body_close_delimiter, None);
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
    assert_eq!(coproc.body_close_delimiter.as_deref(), Some(")"));
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
