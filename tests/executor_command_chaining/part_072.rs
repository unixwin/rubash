use super::super::*;
use std::fs;
use std::io::IsTerminal;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn make_fifo(path: &str) {
    let path = std::ffi::CString::new(path).unwrap();
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0);
}

#[test]
fn test_conditional_string_order_operators_are_not_redirects() {
    let output_path = "target/rubash-conditional-string-order-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "left=abc; right=def; [[ $left < $right ]]; echo $? > {output_path}; [[ $right > $left ]]; echo $? >> {output_path}; [[ $right < $left ]]; echo $? >> {output_path}; [[ $left > $right ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n1\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_command_pipeline_stage_executes_and_feeds_next_stage() {
    let output_path = "target/rubash-conditional-command-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=yes; [[ $value == yes ]] | cat > {output_path}; \
         echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let pipeline = ast.commands[1].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].conditional_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_string_equality_uses_shell_patterns() {
    let output_path = "target/rubash-conditional-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abcdef; pattern='a*'; [[ $value == a* ]]; echo $? > {output_path}; [[ $value = a?c* ]]; echo $? >> {output_path}; [[ $value == a[b-d]cdef ]]; echo $? >> {output_path}; [[ $value != z* ]]; echo $? >> {output_path}; [[ $value != a* ]]; echo $? >> {output_path}; [[ $value == $pattern ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n0\n0\n0\n1\n0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_quoted_rhs_compares_literal_pattern_text() {
    let output_path = "target/rubash-conditional-quoted-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ abc == a* ]]; echo unquoted:$? > {output_path}; \
         [[ abc == \"a*\" ]]; echo quoted:$? >> {output_path}; \
         [[ abc != \"a*\" ]]; echo quoted_ne:$? >> {output_path}; \
         pattern='a*'; [[ abc == \"$pattern\" ]]; echo quoted_var:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unquoted:0\nquoted:1\nquoted_ne:0\nquoted_var:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_quoted_rhs_inside_logical_expressions_stays_literal() {
    let output_path = "target/rubash-conditional-quoted-logical-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ abc == \"a*\" || abc == nomatch ]]; echo or:$? > {output_path}; \
         [[ !( abc == \"a*\" ) ]]; echo neg:$? >> {output_path}; \
         [[ abc == \"a*\" && abc == a* ]]; echo and:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "or:1\nneg:0\nand:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_string_equality_honors_nocasematch() {
    let output_path = "target/rubash-conditional-nocasematch-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s nocasematch extglob; \
         [[ Alpha == alpha ]]; echo literal:$? > {output_path}; \
         [[ A == [a-z] ]]; echo range:$? >> {output_path}; \
         [[ BAR == @(foo|bar) ]]; echo extglob:$? >> {output_path}; \
         [[ a == [[:upper:]] ]]; echo upper:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "literal:0\nrange:0\nextglob:0\nupper:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_string_equality_uses_negated_extglob_patterns() {
    let output_path = "target/rubash-conditional-negated-extglob-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s extglob; \
         [[ file.txt == !(*.tmp) ]]; echo txt:$? > {output_path}; \
         [[ file.tmp == !(*.tmp) ]]; echo tmp:$? >> {output_path}; \
         [[ file.tmp != !(*.tmp) ]]; echo not_tmp:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "txt:0\ntmp:1\nnot_tmp:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_string_equality_uses_extglob_without_shopt() {
    let output_path = "target/rubash-conditional-extglob-without-shopt-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ alpha == @(alpha|beta) ]]; echo match:$? > {output_path}; \
         [[ gamma == @(alpha|beta) ]]; echo miss:$? >> {output_path}; \
         [[ gamma != @(alpha|beta) ]]; echo not_match:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "match:0\nmiss:1\nnot_match:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_single_word_tests_non_empty_expansion() {
    let output_path = "target/rubash-conditional-single-word-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc; empty=; \
         [[ $value ]]; echo nonempty:$? > {output_path}; \
         [[ $empty ]]; echo empty:$? >> {output_path}; \
         [[ literal ]]; echo literal:$? >> {output_path}; \
         [[ $missing ]]; echo missing:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "nonempty:0\nempty:1\nliteral:0\nmissing:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_regex_match_sets_bash_rematch() {
    let output_path = "target/rubash-conditional-regex-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc123; pattern='([a-z]+)([0-9]+)'; [[ $value =~ $pattern ]]; echo $? ${{BASH_REMATCH[0]}} ${{BASH_REMATCH[1]}} ${{BASH_REMATCH[2]}} > {output_path}; [[ $value =~ z+ ]]; echo $? >> {output_path}; [[ $value =~ '[' ]]; echo $? >> {output_path}; [[ $value =~ [ ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 abc123 abc 123\n1\n1\n2\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_quoted_regex_rhs_matches_literal_text() {
    let output_path = "target/rubash-conditional-quoted-regex-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ abc =~ a.* ]]; echo unquoted:$? > {output_path}; \
         [[ abc =~ \"a.*\" ]]; echo quoted:$? >> {output_path}; \
         pattern='a.*'; [[ abc =~ \"$pattern\" ]]; echo quoted_var:$? >> {output_path}; \
         [[ \"a.*\" =~ \"a.*\" ]]; echo literal:$? >> {output_path}; \
         [[ abcdef =~ ^\"abc\".* ]]; echo partial:$? >> {output_path}; \
         [[ ^abczzz =~ \"^abc\".* ]]; echo quoted_anchor:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unquoted:0\nquoted:1\nquoted_var:1\nliteral:0\npartial:0\nquoted_anchor:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_regex_honors_nocasematch() {
    let output_path = "target/rubash-conditional-regex-nocasematch-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ ABC =~ ^abc$ ]]; echo before:$? > {output_path}; \
         shopt -s nocasematch; \
         [[ ABC =~ ^abc$ ]]; echo after:$? >> {output_path}; \
         [[ ABC =~ ^(ab)(c)$ ]]; echo caps:$?:${{BASH_REMATCH[1]}}:${{BASH_REMATCH[2]}} >> {output_path}; \
         [[ ABC =~ \"^abc$\" ]]; echo quoted:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before:1\nafter:0\ncaps:0:AB:C\nquoted:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_bare_regex_groups_set_bash_rematch() {
    let output_path = "target/rubash-conditional-bare-regex-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ '2:bad' =~ ^([0-9]+):(.*) ]]; echo $? ${{BASH_REMATCH[0]}} ${{BASH_REMATCH[1]}} ${{BASH_REMATCH[2]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0 2:bad 2 bad\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_conditional_regex_groups_set_bash_rematch() {
    let output_path = "target/rubash-if-conditional-regex-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "if [[ '1.000' =~ ^[-]?([0-9]*)\\.([0-9]+)$ ]]; then echo \"${{BASH_REMATCH[1]}}/${{BASH_REMATCH[2]}}\" > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1/000\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_conditional_regex_preserves_alternation() {
    let output_path = "target/rubash-if-regex-alternation-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "if [[ shellmath_add =~ shellmath_(add|subtract|multiply|divide)$ ]]; then echo yes > {output_path}; else echo no > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_logical_operators_stay_inside_expression() {
    let output_path = "target/rubash-conditional-logical-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc; empty=; [[ -n $value && -z $empty ]]; echo $? > {output_path}; [[ -n $empty || $value = abc ]]; echo $? >> {output_path}; [[ -n $empty || -z $value && $value = abc ]]; echo $? >> {output_path}; [[ ! -n $empty && $value = abc ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_parentheses_group_logical_expressions() {
    let output_path = "target/rubash-conditional-parentheses-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc; empty=; [[ -n $value || -n $empty && -z $value ]]; echo $? > {output_path}; [[ ( -n $value || -n $empty ) && -z $value ]]; echo $? >> {output_path}; [[ ! ( -n $empty || -z $value ) ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_command_redirect_creates_output_file() {
    let output_path = "target/rubash-conditional-redirect-output.txt";
    let status_path = "target/rubash-conditional-redirect-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "[[ value == value ]] > {output_path}; echo true:$? > {status_path}; \
         [[ value == other ]] >> {output_path}; echo false:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "true:0\nfalse:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_conditional_file_unary_checks_paths() {
    let output_path = "target/rubash-conditional-file-unary-output.txt";
    let file_path = "target/rubash-conditional-file-unary.txt";
    let dir_path = "target/rubash-conditional-file-unary-dir";
    let missing_path = "target/rubash-conditional-file-unary-missing";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(file_path);
    let _ = fs::remove_dir_all(dir_path);
    fs::write(file_path, "data").unwrap();
    fs::create_dir_all(dir_path).unwrap();
    let input = format!(
        "[[ -e {file_path} ]]; echo $? > {output_path}; [[ -f {file_path} ]]; echo $? >> {output_path}; [[ -d {dir_path} ]]; echo $? >> {output_path}; [[ -s {file_path} ]]; echo $? >> {output_path}; [[ -e {missing_path} ]]; echo $? >> {output_path}; [[ -d {file_path} ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n0\n0\n0\n1\n1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(file_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_conditional_modified_since_read_unary_checks_paths() {
    let output_path = "target/rubash-conditional-n-unary-output.txt";
    let file_path = "target/rubash-conditional-n-unary.txt";
    let missing_path = "target/rubash-conditional-n-unary-missing.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(file_path);
    let _ = fs::remove_file(missing_path);
    fs::write(file_path, "data").unwrap();
    let input = format!(
        "[[ -N {file_path} ]]; echo $? > {output_path}; \
         [[ -N {missing_path} ]]; echo $? >> {output_path}; \
         test -N {file_path}; echo $? >> {output_path}; \
         test -N {missing_path}; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n0\n1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(file_path);
}

#[test]
fn test_conditional_ownership_unary_checks_paths() {
    let output_path = "target/rubash-conditional-owner-unary-output.txt";
    let file_path = "target/rubash-conditional-owner-unary.txt";
    let missing_path = "target/rubash-conditional-owner-unary-missing.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(file_path);
    let _ = fs::remove_file(missing_path);
    fs::write(file_path, "data").unwrap();
    let input = format!(
        "[[ -O {file_path} ]]; echo $? > {output_path}; \
         [[ -G {file_path} ]]; echo $? >> {output_path}; \
         test -O {file_path}; echo $? >> {output_path}; \
         test -G {file_path}; echo $? >> {output_path}; \
         [[ -O {missing_path} ]]; echo $? >> {output_path}; \
         [[ -G {missing_path} ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    let expected_existing = if cfg!(unix) { "0" } else { "1" };
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!(
            "{expected_existing}\n{expected_existing}\n{expected_existing}\n{expected_existing}\n1\n1\n"
        )
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(file_path);
}

#[cfg(unix)]
#[test]
fn test_conditional_unix_file_unary_checks_paths() {
    let output_path = "target/rubash-conditional-file-kind-output.txt";
    let fifo_path = "target/rubash-conditional-file-kind-fifo";
    let socket_path = "target/rubash-conditional-file-kind-socket";
    let mode_path = "target/rubash-conditional-file-kind-mode.txt";
    let sticky_dir = "target/rubash-conditional-file-kind-sticky-dir";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(fifo_path);
    let _ = fs::remove_file(socket_path);
    let _ = fs::remove_file(mode_path);
    let _ = fs::remove_dir_all(sticky_dir);

    make_fifo(fifo_path);
    let _socket = std::os::unix::net::UnixListener::bind(socket_path).unwrap();
    fs::write(mode_path, "data").unwrap();
    let mut permissions = fs::metadata(mode_path).unwrap().permissions();
    permissions.set_mode(0o7600);
    fs::set_permissions(mode_path, permissions).unwrap();
    fs::create_dir_all(sticky_dir).unwrap();
    let mut permissions = fs::metadata(sticky_dir).unwrap().permissions();
    permissions.set_mode(0o1700);
    fs::set_permissions(sticky_dir, permissions).unwrap();
    let input = format!(
        "[[ -p {fifo_path} ]]; echo $? > {output_path}; \
         [[ -S {socket_path} ]]; echo $? >> {output_path}; \
         [[ -u {mode_path} ]]; echo $? >> {output_path}; \
         [[ -g {mode_path} ]]; echo $? >> {output_path}; \
         [[ -k {sticky_dir} ]]; echo $? >> {output_path}; \
         test -p {mode_path}; echo $? >> {output_path}; \
         test -S {mode_path}; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n0\n0\n0\n0\n1\n1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(fifo_path);
    let _ = fs::remove_file(socket_path);
    let _ = fs::remove_file(mode_path);
    let _ = fs::remove_dir_all(sticky_dir);
}

#[test]
fn test_conditional_terminal_unary_checks_fds() {
    let output_path = "target/rubash-conditional-terminal-unary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ -t 0 ]]; echo $? > {output_path}; \
         test -t 1; echo $? >> {output_path}; \
         [[ -t 2 ]]; echo $? >> {output_path}; \
         [[ -t 9999 ]]; echo $? >> {output_path}; \
         [[ -t nope ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    let stdin_status = if std::io::stdin().is_terminal() {
        "0"
    } else {
        "1"
    };
    let stdout_status = if std::io::stdout().is_terminal() {
        "0"
    } else {
        "1"
    };
    let stderr_status = if std::io::stderr().is_terminal() {
        "0"
    } else {
        "1"
    };
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("{stdin_status}\n{stdout_status}\n{stderr_status}\n1\n1\n")
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_file_binary_checks_paths() {
    let output_path = "target/rubash-conditional-file-binary-output.txt";
    let older_path = "target/rubash-conditional-file-binary-older.txt";
    let newer_path = "target/rubash-conditional-file-binary-newer.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(older_path);
    let _ = fs::remove_file(newer_path);
    fs::write(older_path, "old").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(25));
    fs::write(newer_path, "new").unwrap();
    let input = format!(
        "[[ {newer_path} -nt {older_path} ]]; echo $? > {output_path}; [[ {older_path} -ot {newer_path} ]]; echo $? >> {output_path}; [[ {older_path} -ef {older_path} ]]; echo $? >> {output_path}; test {newer_path} -nt {older_path}; echo $? >> {output_path}; [[ {older_path} -nt {newer_path} ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n0\n0\n1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(older_path);
    let _ = fs::remove_file(newer_path);
}

#[test]
fn test_test_builtin_parenthesizes_logical_expressions() {
    let output_path = "target/rubash-test-parentheses-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "test \\( x = x -o '' \\) -a ''; echo $? > {output_path}; test \\( x = y -o ok = ok \\) -a yes = yes; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_negates_supported_expressions() {
    let output_path = "target/rubash-conditional-negation-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc; empty=; [[ ! -n $value ]]; echo $? > {output_path}; [[ ! -n $empty ]]; echo $? >> {output_path}; [[ ! 3 -gt 4 ]]; echo $? >> {output_path}; [[ ! $value = abc ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n0\n1\n");
    let _ = fs::remove_file(output_path);
}
