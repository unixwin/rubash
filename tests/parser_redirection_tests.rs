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
fn test_heredoc_redirect_records_operator_metadata() {
    let input = "cat <<EOF\nalpha\nEOF";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let command = &ast.commands[0];

    assert_eq!(command.heredoc_redirects.len(), 1);
    let redirect = &command.heredoc_redirects[0];
    assert_eq!(redirect.fd, None);
    assert_eq!(redirect.operator, "<<");
    assert_eq!(redirect.delimiter, "EOF");
    assert!(!redirect.strip_tabs);
    assert!(!redirect.quoted_delimiter);
    assert!(!redirect.here_string);
    assert_eq!(redirect.body.as_deref(), Some("alpha\n"));
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
