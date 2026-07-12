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
fn test_named_coproc_parses_conditional_body() {
    let input = "coproc MYC [[ $value == ok ]]";
    let tokens = tokenize(input);
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert_eq!(coproc.name.as_deref(), Some("MYC"));
    assert!(coproc.words.is_empty());
    assert_eq!(coproc.body_kind, CoprocBodyKind::CommandSequence);
    assert_eq!(
        coproc.body.as_ref().unwrap()[0].words,
        ["[[", "$value", "==", "ok", "]]"]
    );
}
