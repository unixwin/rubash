use super::*;
use crate::lexer::{tokenize, Token};

#[test]
fn dynamic_fd_loop_redirect_attaches_to_compound_command() {
    let ast = parse(&tokenize(
        "while read -r -u ${fd}; do echo ok; done {fd}</tmp/x",
    ));
    assert_eq!(ast.commands.len(), 1);
    let command = &ast.commands[0];
    assert!(command.loop_command.is_some());
    assert_eq!(command.redirects.len(), 1);
    assert_eq!(command.redirects[0].fd_var.as_deref(), Some("fd"));
    assert_eq!(command.redirects[0].target, "/tmp/x");
}

#[test]
fn test_parse_simple() {
    let tokens = tokenize("ls -la");
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words.len(), 2);
}

#[test]
fn test_extglob_enabled_after_parse_is_rejected() {
    let ast = parse(&tokenize("shopt -s extglob; echo @(x)"));

    assert_eq!(ast.commands.len(), 2);
    assert_eq!(
        ast.commands[1]
            .get_assignment("__RUBASH_PARSE_ERROR__")
            .map(String::as_str),
        Some("unexpected token `('")
    );
}

#[test]
fn test_extglob_on_next_input_line_remains_valid() {
    let ast = parse(&tokenize("shopt -s extglob\necho @(x)"));

    assert!(ast
        .commands
        .iter()
        .all(|command| !command.has_assignment("__RUBASH_PARSE_ERROR__")));
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
    let tokens = tokenize("cat <<EOF1 3<<EOF2\none\nEOF1\ntwo\nEOF2");
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, vec!["cat"]);
    assert_eq!(ast.commands[0].heredoc_redirects.len(), 2);
    assert_eq!(ast.commands[0].heredoc_redirects[0].fd, None);
    assert_eq!(ast.commands[0].heredoc_redirects[0].fd_var, None);
    assert_eq!(
        ast.commands[0].heredoc_redirects[0].body.as_deref(),
        Some("one\n")
    );
    assert_eq!(ast.commands[0].heredoc_redirects[1].fd, Some(3));
    assert_eq!(ast.commands[0].heredoc_redirects[1].fd_var, None);
    assert_eq!(
        ast.commands[0].heredoc_redirects[1].body.as_deref(),
        Some("two\n")
    );
}

#[test]
fn test_parse_sequential_command_heredocs_keep_body_order() {
    let tokens = tokenize("cat <<A; cat <<B\none\nA\ntwo\nB");
    let ast = parse(&tokens);

    assert_eq!(ast.commands.len(), 2);
    assert_eq!(ast.commands[0].heredoc.as_deref(), Some("one\n"));
    assert_eq!(ast.commands[1].heredoc.as_deref(), Some("two\n"));
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
fn test_command_substitution_ignores_parentheses_in_heredoc_body() {
    let tokens = tokenize("echo $(\ncat <<eof\nhere doc with )\neof\n)");
    let ast = parse(&tokens);
    let substitution = &ast.commands[0].command_substitutions[0];

    assert_eq!(substitution.source, "\ncat <<eof\nhere doc with )\neof\n");
    assert_eq!(
        substitution.commands[0].heredoc.as_deref(),
        Some("here doc with )\n")
    );
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
    assert_eq!(while_command.keyword, "while");
    assert_eq!(while_command.do_keyword, "do");
    assert_eq!(while_command.end_keyword, "done");
    assert_eq!(while_command.condition_terminator.as_deref(), Some(";"));
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
    assert_eq!(until_command.keyword, "until");
    assert_eq!(until_command.do_keyword, "do");
    assert_eq!(until_command.end_keyword, "done");
    assert_eq!(until_command.condition_terminator.as_deref(), Some(";"));
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

    assert_eq!(for_command.keyword, "for");
    assert_eq!(for_command.in_keyword, None);
    assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
    assert_eq!(for_command.end_keyword.as_deref(), Some("done"));
    assert_eq!(for_command.list_terminator.as_deref(), Some(";"));
    assert_eq!(
        for_command.list_terminator_metadata.as_ref().unwrap().value,
        ";"
    );
    assert_eq!(arithmetic.open_delimiter, "((");
    assert_eq!(arithmetic.init, "i = 0");
    // Raw-display contract (arithmetic_aliases): the parser records each
    // section's whitespace-carrying text verbatim; leading_ws is preserved
    // in the metadata expression and skipped only at display time.
    assert_eq!(arithmetic.init_metadata.expression, " i = 0");
    assert_eq!(arithmetic.init_metadata.variables, ["i"]);
    assert_eq!(arithmetic.init_metadata.operators[0].text, "=");
    assert!(arithmetic.init_metadata.has_assignment);
    assert!(!arithmetic.init_metadata.has_comparison);
    assert_eq!(arithmetic.separators, [";", ";"]);
    assert_eq!(arithmetic.separator_metadata.len(), 2);
    assert_eq!(arithmetic.separator_metadata[0].value, ";");
    assert_eq!(arithmetic.separator_metadata[0].word_index, 0);
    assert_eq!(arithmetic.separator_metadata[1].value, ";");
    assert_eq!(arithmetic.separator_metadata[1].word_index, 1);
    assert_eq!(arithmetic.test, "i < 3");
    assert_eq!(arithmetic.test_metadata.expression, " i < 3");
    assert_eq!(arithmetic.test_metadata.variables, ["i"]);
    assert_eq!(arithmetic.test_metadata.operators[0].text, "<");
    assert!(arithmetic.test_metadata.has_comparison);
    assert_eq!(arithmetic.update, "i++");
    assert_eq!(arithmetic.update_metadata.expression, " i++ ");
    assert_eq!(arithmetic.update_metadata.variables, ["i"]);
    assert_eq!(arithmetic.update_metadata.operators[0].text, "++");
    assert!(arithmetic.update_metadata.has_update);
    assert_eq!(arithmetic.close_delimiter, "))");
    assert_eq!(arithmetic.open_delimiter_metadata.value, "((");
    assert_eq!(arithmetic.open_delimiter_metadata.raw, "((");
    assert_eq!(arithmetic.close_delimiter_metadata.value, "))");
    assert_eq!(arithmetic.close_delimiter_metadata.raw, "))");
    assert_eq!(for_command.body_kind, CommandBodyKind::DoDone);
    assert_eq!(for_command.body_open_delimiter.as_deref(), Some("do"));
    assert_eq!(
        for_command
            .body_open_delimiter_metadata
            .as_ref()
            .unwrap()
            .value,
        "do"
    );
    assert_eq!(for_command.body_close_delimiter.as_deref(), Some("done"));
    assert_eq!(
        for_command
            .body_close_delimiter_metadata
            .as_ref()
            .unwrap()
            .value,
        "done"
    );
    assert_eq!(for_command.body[0].words, ["echo", "$i"]);
}

#[test]
fn test_parse_compact_arithmetic_for_empty_test() {
    let tokens = tokenize("for ((i=0;;i++)); do echo $i; done");
    let ast = parse(&tokens);
    let for_command = ast.commands[0].for_command.as_ref().unwrap();
    let arithmetic = for_command.arithmetic.as_ref().unwrap();

    assert_eq!(arithmetic.init, "i=0");
    assert_eq!(arithmetic.init_metadata.expression, "i=0");
    assert!(arithmetic.init_metadata.has_assignment);
    assert_eq!(arithmetic.test, "");
    assert!(arithmetic.test_metadata.operators.is_empty());
    assert!(arithmetic.test_metadata.variables.is_empty());
    assert_eq!(arithmetic.update, "i++");
    assert!(arithmetic.update_metadata.has_update);
    assert_eq!(for_command.do_keyword.as_deref(), Some("do"));
    assert_eq!(for_command.end_keyword.as_deref(), Some("done"));
    assert_eq!(for_command.body[0].words, ["echo", "$i"]);
}

#[test]
fn spaced_compound_assignment_is_marked_as_syntax_error() {
    let ast = parse(&tokenize("a= (1 2)"));
    assert!(ast.commands[0].has_assignment("__RUBASH_PARSE_ERROR__"));
}

#[test]
fn function_body_opening_brace_with_trailing_blanks_parses_body() {
    // GNU parse.y reads the reserved word '{' and treats trailing blanks as
    // a token separator. The lexer emits the '{' + TAB line in more-exp.tests'
    // "b2()" / "{" TAB body as one Keyword token whose value is "{" TAB;
    // the exact is_keyword check in matching_brace_group_end rejected it, the
    // body scan returned None, and the caller's last-"}" fallback silently
    // absorbed the rest of the script into the dead function body (rc=0, no
    // diagnostics). The trailing '}' the lexer splits out of
    // ${abc:-G { I } K } further down the stream is what the old fallback
    // used to grab, so keep a later '}'-carrying command in this test.
    let source = "b1()\n{\n\tb2 ${1+\"$@\"}\n}\n\nb2()\n{\t\n\trecho $*\n\trecho ${#}\n}\nrecho ${abc:-G { I } K }\necho after";
    let ast = parse(&tokenize(source));
    assert_eq!(ast.commands.len(), 4);
    assert!(ast.commands[0].function_command.is_some());
    let function = ast.commands[1].function_command.as_ref().unwrap();
    assert_eq!(function.body.len(), 2);
    assert_eq!(ast.commands[3].words, ["echo", "after"]);
}
