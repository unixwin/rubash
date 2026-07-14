use rubash::lexer::tokenize;
use rubash::parser::{parse, RedirectKind};

#[test]
fn test_output_redirect() {
    let input = "echo hello > file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_out.is_some());
    let redirect = ast.commands[0].redirect_out.as_ref().unwrap();
    assert_eq!(redirect.operator, ">");
    assert_eq!(redirect.kind, RedirectKind::Output);
    assert!(!redirect.clobber);
}

#[test]
fn test_output_redirect_without_space_after_word() {
    let input = "echo hello>file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["echo", "hello"]);
    assert_eq!(
        ast.commands[0].redirect_out.as_ref().unwrap().target,
        "file.txt"
    );
}

#[test]
fn test_redirect_target_can_look_like_assignment_word() {
    let input = "echo hello > name=value";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["echo", "hello"]);
    assert!(command.assignments.is_empty());
    assert_eq!(command.redirect_out.as_ref().unwrap().target, "name=value");
}

#[test]
fn test_redirect_target_can_be_brace_expansion_word() {
    let input = "echo hello > {out,err}";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["echo", "hello"]);
    assert_eq!(command.redirect_out.as_ref().unwrap().target, "{out,err}");
}

#[test]
fn test_redirect_target_records_word_metadata() {
    let input = "echo hello > $dir/{out,err}.@(log|txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];
    let redirect = command.redirect_out.as_ref().unwrap();
    let metadata = &redirect.target_metadata;

    assert_eq!(redirect.target, "$dir/{out,err}.@(log|txt)");
    assert_eq!(metadata.word_index, 0);
    assert_eq!(metadata.value, "$dir/{out,err}.@(log|txt)");
    assert_eq!(metadata.raw, "$dir/{out,err}.@(log|txt)");
    assert_eq!(metadata.parameter_expansions.len(), 1);
    assert_eq!(metadata.parameter_expansions[0].text, "$dir");
    assert_eq!(metadata.brace_expansions.len(), 1);
    assert_eq!(metadata.brace_expansions[0].body, "out,err");
    assert_eq!(metadata.extglob_patterns.len(), 1);
    assert_eq!(metadata.extglob_patterns[0].text, "@(log|txt)");
    assert_eq!(metadata.extglob_patterns[0].alternatives, ["log", "txt"]);
}

#[test]
fn test_redirection_list_records_parse_order() {
    let input = "echo hello > first 2>&1 >> second < input";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];
    let redirects = command.redirects.as_slice();

    assert_eq!(redirects.len(), 4);
    assert_eq!(redirects[0].operator, ">");
    assert_eq!(redirects[0].kind, RedirectKind::Output);
    assert_eq!(redirects[0].target, "first");
    assert_eq!(redirects[1].operator, "2>&");
    assert_eq!(redirects[1].kind, RedirectKind::DuplicateOutput);
    assert_eq!(redirects[1].target, "&1");
    assert_eq!(redirects[2].operator, ">>");
    assert_eq!(redirects[2].kind, RedirectKind::Append);
    assert_eq!(redirects[2].target, "second");
    assert_eq!(redirects[3].operator, "<");
    assert_eq!(redirects[3].kind, RedirectKind::Input);
    assert_eq!(redirects[3].target, "input");

    assert_eq!(command.redirect_out.as_ref().unwrap().target, "first");
    assert_eq!(command.append.as_ref().unwrap().target, "second");
    assert_eq!(command.redirect_in.as_ref().unwrap().target, "input");
}

#[test]
fn test_clobber_output_redirect() {
    let input = "echo hello >| file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let redirect = ast.commands[0].redirect_out.as_ref().unwrap();
    assert_eq!(redirect.operator, ">|");
    assert_eq!(redirect.kind, RedirectKind::ClobberOutput);
    assert!(redirect.clobber);
}

#[test]
fn test_output_process_substitution_redirect() {
    let input = "echo hello > >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["echo", "hello"]);
    assert_eq!(
        ast.commands[0].redirect_out.as_ref().unwrap().target,
        ">(cat > out.txt)"
    );
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].open_delimiter, ">(");
    assert_eq!(process[0].operator, ">");
    assert_eq!(process[0].close_delimiter, ")");
    assert_eq!(process[0].source, "cat > out.txt");
    assert_eq!(process[0].commands.len(), 1);
    assert_eq!(process[0].commands[0].words, ["cat"]);
    assert_eq!(
        process[0].commands[0].redirect_out.as_ref().unwrap().target,
        "out.txt"
    );
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, None);
}

#[test]
fn test_prefixed_output_process_substitution_redirect() {
    let input = "exec 3> >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["exec"]);
    let redirect = ast.commands[0].redirect_out.as_ref().unwrap();
    assert_eq!(redirect.fd, Some(3));
    assert_eq!(redirect.operator, "3>");
    assert_eq!(redirect.target, ">(cat > out.txt)");
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].source, "cat > out.txt");
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, Some(3));
}

#[test]
fn test_append_process_substitution_redirect() {
    let input = "echo hello >> >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["echo", "hello"]);
    let redirect = ast.commands[0].append.as_ref().unwrap();
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.operator, ">>");
    assert_eq!(redirect.target, ">(cat > out.txt)");
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].source, "cat > out.txt");
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, None);
}

#[test]
fn test_prefixed_append_process_substitution_redirect() {
    let input = "exec 3>> >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["exec"]);
    let redirect = ast.commands[0].append.as_ref().unwrap();
    assert_eq!(redirect.fd, Some(3));
    assert_eq!(redirect.operator, "3>>");
    assert_eq!(redirect.target, ">(cat > out.txt)");
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].source, "cat > out.txt");
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, Some(3));
}

#[test]
fn test_dynamic_fd_append_process_substitution_redirect() {
    let input = "exec {fd}>> >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["exec", "{fd}"]);
    let redirect = ast.commands[0].append.as_ref().unwrap();
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.fd_var.as_deref(), Some("fd"));
    assert_eq!(redirect.operator, ">>");
    assert_eq!(redirect.target, ">(cat > out.txt)");
    assert_eq!(ast.commands[0].redirects[0].fd_var.as_deref(), Some("fd"));
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].source, "cat > out.txt");
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, None);
}

#[test]
fn test_dynamic_fd_output_redirect_records_fd_var() {
    let input = "exec {fd}> out";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["exec", "{fd}"]);
    let redirect = command.redirect_out.as_ref().unwrap();
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.fd_var.as_deref(), Some("fd"));
    assert_eq!(redirect.operator, ">");
    assert_eq!(redirect.target, "out");
    assert_eq!(command.redirects[0].fd_var.as_deref(), Some("fd"));
}

#[test]
fn test_stderr_process_substitution_redirect() {
    let input = "printf err >&2 2> >(cat > err.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(
        ast.commands[0].redirect_err.as_ref().unwrap().target,
        ">(cat > err.txt)"
    );
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > err.txt)");
    assert_eq!(process[0].open_delimiter, ">(");
    assert_eq!(process[0].operator, ">");
    assert_eq!(process[0].source, "cat > err.txt");
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, Some(2));
}

#[test]
fn test_stderr_append_process_substitution_redirect() {
    let input = "printf err >&2 2>> >(cat > err.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    let redirect = ast.commands[0].redirect_err_append.as_ref().unwrap();
    assert_eq!(redirect.fd, Some(2));
    assert_eq!(redirect.operator, "2>>");
    assert_eq!(redirect.target, ">(cat > err.txt)");
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > err.txt)");
    assert_eq!(process[0].source, "cat > err.txt");
    assert!(process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, Some(2));
}

#[test]
fn test_output_process_substitution_word() {
    let input = "tee >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["tee", ">(cat > out.txt)"]);
    assert!(ast.commands[0].redirect_out.is_none());
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].open_delimiter, ">(");
    assert_eq!(process[0].operator, ">");
    assert_eq!(process[0].close_delimiter, ")");
    assert_eq!(process[0].source, "cat > out.txt");
    assert_eq!(process[0].commands.len(), 1);
    assert_eq!(process[0].commands[0].words, ["cat"]);
    assert_eq!(
        process[0].commands[0].redirect_out.as_ref().unwrap().target,
        "out.txt"
    );
    assert!(process[0].output);
    assert_eq!(process[0].word_index, Some(1));
    assert_eq!(process[0].redirect_fd, None);
}

#[test]
fn test_input_process_substitution_redirect_records_structured_ast() {
    let input = "cat < <(printf data)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["cat"]);
    assert_eq!(
        ast.commands[0].redirect_in.as_ref().unwrap().target,
        "<(printf data)"
    );
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, "<(printf data)");
    assert_eq!(process[0].open_delimiter, "<(");
    assert_eq!(process[0].operator, "<");
    assert_eq!(process[0].close_delimiter, ")");
    assert_eq!(process[0].source, "printf data");
    assert_eq!(process[0].commands.len(), 1);
    assert_eq!(process[0].commands[0].words, ["printf", "data"]);
    assert!(!process[0].output);
    assert_eq!(process[0].word_index, None);
    assert_eq!(process[0].redirect_fd, None);
}

#[test]
fn test_input_process_substitution_word_records_structured_ast() {
    let input = "diff <(printf a) expected";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["diff", "<(printf a)", "expected"]);
    assert!(ast.commands[0].redirect_in.is_none());
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, "<(printf a)");
    assert_eq!(process[0].open_delimiter, "<(");
    assert_eq!(process[0].operator, "<");
    assert_eq!(process[0].close_delimiter, ")");
    assert_eq!(process[0].source, "printf a");
    assert_eq!(process[0].commands.len(), 1);
    assert_eq!(process[0].commands[0].words, ["printf", "a"]);
    assert!(!process[0].output);
    assert_eq!(process[0].word_index, Some(1));
    assert_eq!(process[0].redirect_fd, None);
}

#[test]
fn test_process_substitution_source_preserves_quotes() {
    let input = "cat <(printf \"x\\n\")";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["cat", "<(printf \"x\\n\")"]);
    let process = ast.commands[0].process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, "<(printf \"x\\n\")");
    assert_eq!(process[0].source, "printf \"x\\n\"");
    assert_eq!(process[0].commands[0].words, ["printf", "x\\n"]);
}

#[test]
fn test_process_substitution_records_nested_body_ast() {
    let input = "cat < <(echo $(date); printf done)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let process = ast.commands[0].process_substitutions.as_slice();

    assert_eq!(process.len(), 1);
    assert_eq!(process[0].source, "echo $(date) ; printf done");
    assert_eq!(process[0].commands.len(), 2);
    assert_eq!(process[0].commands[0].words, ["echo", "$(date)"]);
    assert_eq!(process[0].commands[1].words, ["printf", "done"]);
    assert_eq!(
        process[0].commands[0].command_substitutions[0].source,
        "date"
    );
}

#[test]
fn test_empty_process_substitutions_record_null_body() {
    let input = "cat <() >()";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let process = ast.commands[0].process_substitutions.as_slice();

    assert_eq!(process.len(), 2);
    assert_eq!(process[0].target, "<()");
    assert_eq!(process[0].open_delimiter, "<(");
    assert_eq!(process[0].operator, "<");
    assert_eq!(process[0].close_delimiter, ")");
    assert_eq!(process[0].source, "");
    assert!(process[0].commands.is_empty());
    assert!(!process[0].output);
    assert_eq!(process[0].word_index, Some(1));
    assert_eq!(process[0].redirect_fd, None);

    assert_eq!(process[1].target, ">()");
    assert_eq!(process[1].open_delimiter, ">(");
    assert_eq!(process[1].operator, ">");
    assert_eq!(process[1].close_delimiter, ")");
    assert_eq!(process[1].source, "");
    assert!(process[1].commands.is_empty());
    assert!(process[1].output);
    assert_eq!(process[1].word_index, Some(2));
    assert_eq!(process[1].redirect_fd, None);
}

#[test]
fn test_process_substitution_keeps_case_pattern_parentheses() {
    let input = "cat <(case beta in alpha) printf alpha ;; beta) printf beta ;; esac)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let process = ast.commands[0].process_substitutions.as_slice();

    assert_eq!(process.len(), 1);
    assert_eq!(
        process[0].source,
        "case beta in alpha ) printf alpha ;; beta ) printf beta ;; esac"
    );
    assert_eq!(process[0].commands.len(), 1);
    assert!(process[0].commands[0].case_command.is_some());
}

#[test]
fn test_process_substitution_keeps_case_argument() {
    let input = "cat <(echo case; echo in) >(echo case; echo out)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let process = ast.commands[0].process_substitutions.as_slice();

    assert_eq!(process.len(), 2);
    assert_eq!(process[0].source, "echo case ; echo in");
    assert_eq!(process[0].commands.len(), 2);
    assert_eq!(process[0].commands[0].words, ["echo", "case"]);
    assert_eq!(process[0].commands[1].words, ["echo", "in"]);
    assert_eq!(process[1].source, "echo case ; echo out");
    assert_eq!(process[1].commands.len(), 2);
    assert_eq!(process[1].commands[0].words, ["echo", "case"]);
    assert_eq!(process[1].commands[1].words, ["echo", "out"]);
}

#[test]
fn test_input_redirect() {
    let input = "cat < input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_in.is_some());
}

#[test]
fn test_input_redirect_without_space() {
    let input = "cat<input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["cat"]);
    assert_eq!(
        ast.commands[0].redirect_in.as_ref().unwrap().target,
        "input.txt"
    );
}

#[test]
fn test_input_redirect_target_can_be_command_substitution_word() {
    let input = "cat < $(printf input.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["cat"]);
    assert_eq!(
        command.redirect_in.as_ref().unwrap().target,
        "$(printf input.txt)"
    );
}

#[test]
fn test_input_redirect_fd_prefix_without_space() {
    let input = "cat 0<input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["cat"]);
    assert_eq!(command.redirect_in.as_ref().unwrap().fd, Some(0));
    assert_eq!(command.redirect_in.as_ref().unwrap().target, "input.txt");
}

#[test]
fn test_input_fd_copy_redirect_maps_target_fd() {
    let tokens = tokenize("read value <&3");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_in.as_ref().unwrap().fd, None);
    assert_eq!(command.redirect_in.as_ref().unwrap().target, "&3");
    assert_eq!(command.redirect_in.as_ref().unwrap().operator, "<&");
    assert_eq!(
        command.redirect_in.as_ref().unwrap().kind,
        RedirectKind::DuplicateInput
    );
    assert_eq!(command.words, ["read", "value"]);
}

#[test]
fn test_input_fd_close_redirect_with_prefix() {
    let tokens = tokenize("read value 0<&-");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_in.as_ref().unwrap().fd, Some(0));
    assert_eq!(command.redirect_in.as_ref().unwrap().target, "&-");
    assert_eq!(command.redirect_in.as_ref().unwrap().operator, "0<&");
    assert_eq!(
        command.redirect_in.as_ref().unwrap().kind,
        RedirectKind::CloseInput
    );
    assert_eq!(command.words, ["read", "value"]);
}

#[test]
fn test_read_write_redirect_maps_to_stdin() {
    let input = "cat <> input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_in.as_ref().unwrap().target, "input.txt");
    assert_eq!(command.redirect_in.as_ref().unwrap().operator, "<>");
    assert_eq!(
        command.redirect_in.as_ref().unwrap().kind,
        RedirectKind::ReadWrite
    );
    assert!(command.redirect_in.as_ref().unwrap().append);
    assert!(command.redirect_out.is_none());
}

#[test]
fn test_read_write_redirect_fd_prefix_maps_to_stdin_fd() {
    let input = "read -u 3 value 3<>input.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    let redirect = command.redirect_in.as_ref().unwrap();
    assert_eq!(redirect.fd, Some(3));
    assert_eq!(redirect.target, "input.txt");
    assert_eq!(redirect.operator, "3<>");
    assert_eq!(redirect.kind, RedirectKind::ReadWrite);
    assert!(redirect.append);
    assert_eq!(command.words, ["read", "-u", "3", "value"]);
}

#[test]
fn test_append_redirect() {
    let input = "echo hello >> file.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].append.is_some());
}

#[test]
fn test_clobber_stderr_redirect() {
    let input = "echo hello 2>| err.txt";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert!(ast.commands[0].redirect_err.as_ref().unwrap().clobber);
}

#[test]
fn test_here_string_redirect() {
    let input = "read x <<<\"alpha\"";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].here_string.as_deref(), Some("alpha"));
}

#[test]
fn test_here_string_process_substitution_word() {
    let input = "read x <<< <(printf data)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["read", "x"]);
    assert_eq!(command.here_string.as_deref(), Some("<(printf data)"));
    assert_eq!(command.process_substitutions.len(), 1);
    assert_eq!(command.process_substitutions[0].target, "<(printf data)");
    assert_eq!(command.process_substitutions[0].source, "printf data");
    assert!(!command.process_substitutions[0].output);
    assert_eq!(command.process_substitutions[0].word_index, None);
    assert_eq!(command.process_substitutions[0].redirect_fd, None);
}

#[test]
fn test_here_string_output_process_substitution_word() {
    let input = "read x <<< >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["read", "x"]);
    assert_eq!(command.here_string.as_deref(), Some(">(cat > out.txt)"));
    assert_eq!(command.process_substitutions.len(), 1);
    assert_eq!(command.process_substitutions[0].target, ">(cat > out.txt)");
    assert_eq!(command.process_substitutions[0].source, "cat > out.txt");
    assert!(command.process_substitutions[0].output);
    assert_eq!(command.process_substitutions[0].word_index, None);
    assert_eq!(command.process_substitutions[0].redirect_fd, None);
}

#[test]
fn test_fd_here_string_process_substitution_word() {
    let input = "read -u 3 x 3<<< <(printf data)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert!(command.here_string.is_none());
    assert_eq!(command.heredoc_redirects.len(), 1);
    assert_eq!(command.heredoc_redirects[0].fd, Some(3));
    assert_eq!(
        command.heredoc_redirects[0].body.as_deref(),
        Some("\x1d<(printf data)")
    );
    assert_eq!(command.process_substitutions.len(), 1);
    assert_eq!(command.process_substitutions[0].target, "<(printf data)");
    assert_eq!(command.process_substitutions[0].redirect_fd, Some(3));
}

#[test]
fn test_trailing_here_string_process_substitution_word() {
    let input = "{ read x; } <<< <(printf data)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert!(command.brace_group.is_some());
    assert_eq!(command.here_string.as_deref(), Some("<(printf data)"));
    assert_eq!(command.process_substitutions.len(), 1);
    assert_eq!(command.process_substitutions[0].target, "<(printf data)");
    assert_eq!(command.process_substitutions[0].source, "printf data");
}

#[test]
fn test_trailing_here_string_output_process_substitution_word() {
    let input = "{ read x; } <<< >(cat > out.txt)";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert!(command.brace_group.is_some());
    assert_eq!(command.here_string.as_deref(), Some(">(cat > out.txt)"));
    assert_eq!(command.process_substitutions.len(), 1);
    assert_eq!(command.process_substitutions[0].target, ">(cat > out.txt)");
    assert_eq!(command.process_substitutions[0].source, "cat > out.txt");
    assert!(command.process_substitutions[0].output);
}

#[test]
fn test_trailing_redirect_target_can_look_like_assignment_word() {
    let input = "{ echo hello; } > name=value";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert!(command.brace_group.is_some());
    assert_eq!(command.redirect_out.as_ref().unwrap().target, "name=value");
}

#[test]
fn test_heredoc_redirect_records_operator_metadata() {
    let input = "cat <<EOF\nalpha\nEOF";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.heredoc_redirects.len(), 1);
    let redirect = &command.heredoc_redirects[0];
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.fd_var, None);
    assert_eq!(redirect.operator, "<<");
    assert_eq!(redirect.delimiter, "EOF");
    assert!(!redirect.strip_tabs);
    assert!(!redirect.quoted_delimiter);
    assert!(!redirect.here_string);
    assert_eq!(redirect.body.as_deref(), Some("alpha\n"));
}

#[test]
fn test_heredoc_redirect_is_in_parse_order_list() {
    let input = "cat > out <<EOF 2>> err\nalpha\nEOF";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];
    let redirects = command.redirects.as_slice();

    assert_eq!(redirects.len(), 3);
    assert_eq!(redirects[0].operator, ">");
    assert_eq!(redirects[0].kind, RedirectKind::Output);
    assert_eq!(redirects[0].target, "out");
    assert_eq!(redirects[1].operator, "<<");
    assert_eq!(redirects[1].kind, RedirectKind::HereDoc);
    assert_eq!(redirects[1].target, "EOF");
    assert_eq!(redirects[2].operator, "2>>");
    assert_eq!(redirects[2].kind, RedirectKind::Append);
    assert_eq!(redirects[2].target, "err");
    assert_eq!(command.heredoc_redirects.len(), 1);
    assert_eq!(
        command.heredoc_redirects[0].body.as_deref(),
        Some("alpha\n")
    );
}

#[test]
fn test_heredoc_redirect_records_quoted_strip_tabs_metadata() {
    let input = "cat <<-'EOF'\n\talpha\n\tEOF";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.heredoc_redirects.len(), 1);
    let redirect = &command.heredoc_redirects[0];
    assert_eq!(redirect.operator, "<<-");
    assert_eq!(redirect.delimiter, "EOF");
    assert!(redirect.strip_tabs);
    assert!(redirect.quoted_delimiter);
    assert_eq!(redirect.body.as_deref(), Some("\x1ealpha\n"));
}

#[test]
fn test_fd_heredoc_redirect_records_operator_metadata() {
    let input = "read -u 3 value 3<<EOF\nalpha\nEOF";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.heredoc_redirects.len(), 1);
    let redirect = &command.heredoc_redirects[0];
    assert_eq!(redirect.fd, Some(3));
    assert_eq!(redirect.fd_var, None);
    assert_eq!(redirect.operator, "3<<");
    assert_eq!(redirect.delimiter, "EOF");
    assert!(!redirect.strip_tabs);
    assert!(!redirect.quoted_delimiter);
    assert!(!redirect.here_string);
}

#[test]
fn test_here_string_fd_prefix_maps_to_fd_input() {
    let input = "read -u 3 value 3<<<alpha";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert!(command.here_string.is_none());
    assert_eq!(command.heredoc_redirects.len(), 1);
    assert_eq!(command.heredoc_redirects[0].fd, Some(3));
    assert_eq!(command.heredoc_redirects[0].fd_var, None);
    assert_eq!(command.heredoc_redirects[0].operator, "3<<<");
    assert_eq!(command.heredoc_redirects[0].delimiter, "<<<");
    assert!(!command.heredoc_redirects[0].strip_tabs);
    assert!(!command.heredoc_redirects[0].quoted_delimiter);
    assert!(command.heredoc_redirects[0].here_string);
    assert_eq!(
        command.heredoc_redirects[0].body.as_deref(),
        Some("\x1dalpha")
    );
}

#[test]
fn test_dynamic_fd_here_string_redirect_records_fd_var() {
    let input = "exec {fd}<<<alpha";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["exec", "{fd}"]);
    let redirect = &command.redirects[0];
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.fd_var.as_deref(), Some("fd"));
    assert_eq!(redirect.operator, "<<<");
    assert_eq!(redirect.kind, RedirectKind::HereString);
    assert_eq!(redirect.target, "alpha");
    assert_eq!(command.here_string.as_deref(), Some("alpha"));
}

#[test]
fn test_dynamic_fd_heredoc_redirect_records_fd_var() {
    let input = "exec {fd}<<EOF\nalpha\nEOF";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.words, ["exec", "{fd}"]);
    let redirect = &command.redirects[0];
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.fd_var.as_deref(), Some("fd"));
    assert_eq!(redirect.operator, "<<");
    assert_eq!(redirect.kind, RedirectKind::HereDoc);
    assert_eq!(redirect.target, "EOF");
    assert_eq!(command.heredoc_redirects[0].fd, None);
    assert_eq!(command.heredoc_redirects[0].fd_var.as_deref(), Some("fd"));
    assert_eq!(
        command.heredoc_redirects[0].body.as_deref(),
        Some("alpha\n")
    );
}

#[test]
fn test_combined_stdout_stderr_redirect() {
    let tokens = tokenize("echo both &> out.txt");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_out.as_ref().unwrap().target, "out.txt");
    assert_eq!(
        command.redirect_out.as_ref().unwrap().kind,
        RedirectKind::CombinedOutput
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().target,
        "out.txt"
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().kind,
        RedirectKind::CombinedOutput
    );
}

#[test]
fn test_combined_stdout_stderr_append_redirect() {
    let tokens = tokenize("echo both &>> out.txt");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.append.as_ref().unwrap().target, "out.txt");
    assert_eq!(
        command.append.as_ref().unwrap().kind,
        RedirectKind::CombinedAppend
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().target,
        "out.txt"
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().kind,
        RedirectKind::CombinedAppend
    );
}

#[test]
fn test_combined_output_process_substitution_redirect() {
    let tokens = tokenize("echo both &> >(cat > out.txt)");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(
        command.redirect_out.as_ref().unwrap().target,
        ">(cat > out.txt)"
    );
    assert_eq!(
        command.redirect_out.as_ref().unwrap().kind,
        RedirectKind::CombinedOutput
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().target,
        ">(cat > out.txt)"
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().kind,
        RedirectKind::CombinedOutput
    );
    let process = command.process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].source, "cat > out.txt");
    assert!(process[0].output);
}

#[test]
fn test_combined_append_process_substitution_redirect() {
    let tokens = tokenize("echo both &>> >(cat > out.txt)");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.append.as_ref().unwrap().target, ">(cat > out.txt)");
    assert_eq!(
        command.append.as_ref().unwrap().kind,
        RedirectKind::CombinedAppend
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().target,
        ">(cat > out.txt)"
    );
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().kind,
        RedirectKind::CombinedAppend
    );
    let process = command.process_substitutions.as_slice();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0].target, ">(cat > out.txt)");
    assert_eq!(process[0].source, "cat > out.txt");
    assert!(process[0].output);
}

#[test]
fn test_stderr_fd_copy_inherits_previous_stdout_redirect() {
    let tokens = tokenize("echo both > out.txt 2>&1");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_out.as_ref().unwrap().target, "out.txt");
    assert_eq!(
        command.redirect_err_append.as_ref().unwrap().target,
        "out.txt"
    );
}

#[test]
fn test_stderr_fd_copy_before_stdout_redirect_keeps_fd_target() {
    let tokens = tokenize("echo both 2>&1 > out.txt");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_err.as_ref().unwrap().target, "&1");
    assert_eq!(command.redirect_out.as_ref().unwrap().target, "out.txt");
    assert!(command.redirect_err_append.is_none());
}

#[test]
fn test_stderr_fd_close_redirect() {
    let tokens = tokenize("echo error 2>&-");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_err.as_ref().unwrap().target, "&-");
    assert_eq!(
        command.redirect_err.as_ref().unwrap().kind,
        RedirectKind::CloseOutput
    );
    assert_eq!(command.words, ["echo", "error"]);
}

#[test]
fn test_stdout_fd_close_redirect_with_prefix() {
    let tokens = tokenize("echo hidden 1>&-");
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.redirect_out.as_ref().unwrap().target, "&-");
    assert_eq!(command.redirect_out.as_ref().unwrap().fd, Some(1));
    assert_eq!(
        command.redirect_out.as_ref().unwrap().kind,
        RedirectKind::CloseOutput
    );
    assert_eq!(command.words, ["echo", "hidden"]);
}
