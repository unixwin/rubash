use super::*;
use crate::lexer::{tokenize, Token};

#[test]
fn test_parse_simple() {
    let tokens = tokenize("ls -la");
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words.len(), 2);
}

#[test]
fn test_parse_pipeline() {
    let tokens = tokenize("ls | grep foo");
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.stages.len(), 2);
    assert_eq!(pipeline.operators, ["|"]);
    assert_eq!(pipeline.stages[0].words, ["ls"]);
    assert_eq!(pipeline.stages[1].words, ["grep", "foo"]);
}

#[test]
fn test_parse_empty() {
    let tokens: Vec<Token> = vec![];
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 0);
}

#[test]
fn test_parse_heredoc_delimiter() {
    let tokens = tokenize("cat <<EOF\nbody\nEOF");
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].heredoc_delimiter.as_deref(), Some("EOF"));
    assert_eq!(ast.commands[0].heredoc.as_deref(), Some("body\n"));
}

#[test]
fn test_parse_multiple_heredoc_redirects_with_fd() {
    let tokens = tokenize("done <<EOF1 3<<EOF2\none\nEOF1\ntwo\nEOF2");
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, vec!["done"]);
    assert_eq!(ast.commands[0].heredoc_redirects.len(), 2);
    assert_eq!(ast.commands[0].heredoc_redirects[0].fd, None);
    assert_eq!(
        ast.commands[0].heredoc_redirects[0].body.as_deref(),
        Some("one\n")
    );
    assert_eq!(ast.commands[0].heredoc_redirects[1].fd, Some(3));
    assert_eq!(
        ast.commands[0].heredoc_redirects[1].body.as_deref(),
        Some("two\n")
    );
}

#[test]
fn test_parse_piped_heredoc_body_belongs_to_left_command() {
    let tokens = tokenize("cat <<EOF | sort -u\nbody\nEOF");
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert_eq!(pipeline.stages[0].words, vec!["cat"]);
    assert_eq!(pipeline.stages[0].heredoc.as_deref(), Some("body\n"));
    assert_eq!(pipeline.stages[1].words, vec!["sort", "-u"]);
    assert!(pipeline.stages[1].heredoc.is_none());
}

#[test]
fn test_parse_arithmetic_loop_conditions_as_condition_words() {
    let tokens =
        tokenize("while (( n < 3 )); do (( n++ )); done; until (( n == 5 )); do (( n++ )); done");
    let ast = parse(&tokens);
    let loops = ast
        .commands
        .iter()
        .filter_map(|command| command.loop_command.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(loops.len(), 2);

    let while_command = loops[0];
    assert!(!while_command.until);
    assert_eq!(while_command.kind, LoopKind::While);
    assert_eq!(while_command.condition[0].words, ["((", "n < 3", "))"]);
    assert_eq!(
        while_command.condition[0]
            .arithmetic_command
            .as_ref()
            .unwrap()
            .expression,
        "n < 3"
    );
    assert_eq!(while_command.body[0].words, ["((", "n++", "))"]);
    assert_eq!(
        while_command.body[0]
            .arithmetic_command
            .as_ref()
            .unwrap()
            .expression,
        "n++"
    );

    let until_command = loops[1];
    assert!(until_command.until);
    assert_eq!(until_command.kind, LoopKind::Until);
    assert_eq!(until_command.condition[0].words, ["((", "n == 5", "))"]);
    assert_eq!(
        until_command.condition[0]
            .arithmetic_command
            .as_ref()
            .unwrap()
            .expression,
        "n == 5"
    );
    assert_eq!(until_command.body[0].words, ["((", "n++", "))"]);
}

#[test]
fn test_parse_arithmetic_bitwise_assignment_operators() {
    let tokens = tokenize("(( n &= 10 )); (( n |= 1 )); (( n <<= 2 )); (( n >>= 1 ))");
    let ast = parse(&tokens);
    let words: Vec<Vec<String>> = ast
        .commands
        .iter()
        .filter(|command| !command.words.is_empty())
        .map(|command| command.words.clone())
        .collect();

    assert_eq!(
        words,
        vec![
            vec!["((", "n &= 10", "))"],
            vec!["((", "n |= 1", "))"],
            vec!["((", "n <<= 2", "))"],
            vec!["((", "n >>= 1", "))"],
        ]
    );
    let expressions = ast
        .commands
        .iter()
        .filter_map(|command| command.arithmetic_command.as_ref())
        .map(|command| command.expression.as_str())
        .collect::<Vec<_>>();
    assert_eq!(expressions, ["n &= 10", "n |= 1", "n <<= 2", "n >>= 1"]);
}

#[test]
fn test_parse_grouped_arithmetic_command_expression() {
    let tokens = tokenize("(( (n = 3) )); (( ((m = 0)) ))");
    let ast = parse(&tokens);
    let words: Vec<Vec<String>> = ast
        .commands
        .iter()
        .filter(|command| !command.words.is_empty())
        .map(|command| command.words.clone())
        .collect();

    assert_eq!(
        words,
        vec![
            vec!["((", "( n = 3 )", "))"],
            vec!["((", "( ( m = 0 ) )", "))"],
        ]
    );
    let expressions = ast
        .commands
        .iter()
        .filter_map(|command| command.arithmetic_command.as_ref())
        .map(|command| command.expression.as_str())
        .collect::<Vec<_>>();
    assert_eq!(expressions, ["( n = 3 )", "( ( m = 0 ) )"]);
}

#[test]
fn test_parse_arithmetic_for_command() {
    let tokens = tokenize("for (( i = 0; i < 3; i++ )); do echo $i; done");
    let ast = parse(&tokens);
    let for_command = ast.commands[0].for_command.as_ref().unwrap();
    let arithmetic = for_command.arithmetic.as_ref().unwrap();

    assert_eq!(arithmetic.init, "i = 0");
    assert_eq!(arithmetic.test, "i < 3");
    assert_eq!(arithmetic.update, "i++");
    assert_eq!(for_command.body_kind, CommandBodyKind::DoDone);
    assert_eq!(for_command.body[0].words, ["echo", "$i"]);
}
