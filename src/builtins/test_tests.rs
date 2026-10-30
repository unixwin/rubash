use super::*;
use std::fs;
use std::io::IsTerminal;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
fn make_fifo(path: &str) {
    let path = std::ffi::CString::new(path).unwrap();
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0);
}

fn run(args: &[&str], bracket: bool) -> (i32, String) {
    let env_vars = crate::shell::var_table::VarTable::default();
    run_with_env(args, bracket, &env_vars)
}

fn run_with_env(
    args: &[&str],
    bracket: bool,
    env_vars: &crate::shell::var_table::VarTable,
) -> (i32, String) {
    let mut stderr = Vec::new();
    let status = execute_with_stderr(args, bracket, env_vars, &mut stderr).unwrap();
    (status, String::from_utf8(stderr).unwrap())
}

#[test]
fn empty_expression_is_false() {
    assert_eq!(run(&[], false).0, EXECUTION_FAILURE);
}

#[test]
fn single_non_empty_string_is_true() {
    assert_eq!(run(&["hello"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&[""], false).0, EXECUTION_FAILURE);
}

#[test]
fn supports_string_and_numeric_binary_operators() {
    assert_eq!(run(&["a", "=", "a"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["2", "-lt", "3"], false).0, EXECUTION_SUCCESS);
}

#[test]
#[ignore]
fn bench_arith_three_paths() {
    let env_vars = crate::shell::var_table::VarTable::default();
    for (label, args) in [
        ("canonical-fast", ["1", "-eq", "2", "]"]),
        ("leading-zero-slow", ["007", "-eq", "7", "]"]),
        ("string-eq-parser", ["x", "=", "x", "]"]),
    ] {
        let start = std::time::Instant::now();
        let mut sink = 0;
        for _ in 0..200_000 {
            let mut stderr = Vec::new();
            sink += execute_with_stderr(&args, true, &env_vars, &mut stderr).unwrap();
        }
        eprintln!("{}: {:?} (sink={})", label, start.elapsed(), sink);
    }
}

#[test]
fn decimal_fast_path_matches_slow_path_semantics() {
    // Canonical pure decimals take the whitelist fast path (all six ops,
    // both polarities, negatives, zero, `-0 == 0`, 18-digit max width).
    for (args, expected) in [
        (["1", "-eq", "1"], EXECUTION_SUCCESS),
        (["1", "-eq", "2"], EXECUTION_FAILURE),
        (["1", "-ne", "2"], EXECUTION_SUCCESS),
        (["1", "-lt", "2"], EXECUTION_SUCCESS),
        (["2", "-le", "2"], EXECUTION_SUCCESS),
        (["3", "-gt", "2"], EXECUTION_SUCCESS),
        (["2", "-ge", "2"], EXECUTION_SUCCESS),
        (["-5", "-lt", "-2"], EXECUTION_SUCCESS),
        (["-5", "-eq", "-5"], EXECUTION_SUCCESS),
        (["0", "-eq", "-0"], EXECUTION_SUCCESS),
        (["999999999999999999", "-gt", "1"], EXECUTION_SUCCESS),
        (["-999999999999999999", "-lt", "1"], EXECUTION_SUCCESS),
    ] {
        assert_eq!(run(&args, false).0, expected, "test {}", args.join(" "));
        // Same through the `[` entry (bracket trim + 3-arg fast path).
        let closed = [args[0], args[1], args[2], "]"];
        assert_eq!(run(&closed, true).0, expected, "[ {} ]", args.join(" "));
    }
    // Non-canonical operands fall back to `valid_number` (GNU strtoimax
    // semantics): leading zeros compare by value, surrounding whitespace
    // and `+` are accepted, 19-digit in-range values work.
    for (args, expected) in [
        (["007", "-eq", "7"], EXECUTION_SUCCESS),
        (["08", "-lt", "9"], EXECUTION_SUCCESS),
        (["00", "-eq", "0"], EXECUTION_SUCCESS),
        (["-01", "-eq", "-1"], EXECUTION_SUCCESS),
        ([" 1", "-eq", "1"], EXECUTION_SUCCESS),
        (["1 ", "-eq", "1"], EXECUTION_SUCCESS),
        (["+1", "-eq", "1"], EXECUTION_SUCCESS),
        (["9223372036854775807", "-gt", "1"], EXECUTION_SUCCESS),
        (["-9223372036854775808", "-lt", "1"], EXECUTION_SUCCESS),
    ] {
        assert_eq!(run(&args, false).0, expected, "test {}", args.join(" "));
    }
    // Invalid integers keep the `integer expected` diagnostic, exit 2.
    for args in [
        ["", "-eq", "0"],
        ["1", "-eq", ""],
        ["abc", "-eq", "1"],
        ["99999999999999999999999", "-eq", "1"],
        ["-", "-eq", "1"],
    ] {
        let (status, stderr) = run(&args, false);
        assert_eq!(status, EX_BADUSAGE, "test {}", args.join(" "));
        assert!(stderr.contains("integer expected"), "stderr: {stderr}");
    }
}

#[test]
fn supports_not_and_logical_operators() {
    assert_eq!(run(&["!", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["x", "-a", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["x", "-o", ""], false).0, EXECUTION_SUCCESS);
}

#[test]
fn supports_shell_option_unary_operator() {
    let mut env_vars = crate::shell::var_table::VarTable::default();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    crate::builtins::set::set_with_io(
        ["-o", "errexit"],
        &mut env_vars,
        &mut stdout,
        &mut stderr,
        &crate::shell::var_table::TempenvListingUnderlay::new(),
    )
    .unwrap();

    assert_eq!(
        run_with_env(&["-o", "errexit"], false, &env_vars).0,
        EXECUTION_SUCCESS
    );
    crate::builtins::set::set_with_io(
        ["+o", "errexit"],
        &mut env_vars,
        &mut stdout,
        &mut stderr,
        &crate::shell::var_table::TempenvListingUnderlay::new(),
    )
    .unwrap();
    assert_eq!(
        run_with_env(&["-o", "errexit"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run_with_env(&["-o", "no_such_option"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
}

#[test]
fn leading_file_operator_is_unary_not_logical_and() {
    assert_eq!(run(&["-a", "Cargo.toml"], false).0, EXECUTION_SUCCESS);
}

#[test]
fn null_device_has_virtual_character_device_semantics() {
    assert_eq!(run(&["-e", "/dev/null"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-r", "/dev/null"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-w", "/dev/null"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-c", "/dev/null"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-f", "/dev/null"], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-s", "/dev/null"], false).0, EXECUTION_FAILURE);
}

#[test]
fn modified_since_read_unary_operator_checks_existing_file() {
    let path = "target/rubash-test-n-unary.txt";
    let missing = "target/rubash-test-n-unary-missing.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(missing);
    fs::write(path, "data").unwrap();
    // GNU test.c: -N is mtime > atime. Hosts with atime updates disabled
    // (NTFS disablelastaccess=1 default) stamp atime == mtime on write, so
    // pin atime explicitly to keep the assertion deterministic.
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_accessed(std::time::UNIX_EPOCH))
        .unwrap();

    assert_eq!(run(&["-N", path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-N", missing], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(path);
}

#[test]
fn ownership_unary_operators_check_existing_file() {
    let path = "target/rubash-test-owner-unary.txt";
    let missing = "target/rubash-test-owner-unary-missing.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(missing);
    fs::write(path, "data").unwrap();

    // GNU test.c:561-567: -O/-G are exactly `st_uid == euid` /
    // `st_gid == egid` — nothing about who created the file. A file
    // created inside a setgid directory inherits the directory's
    // group, which need not be the process's egid (under an
    // id-mapped home the inherited group has no mapping and reads
    // back as the overflow gid, and GNU answers -G false there
    // too). Assert against the stat() ground truth instead of
    // assuming self-ownership, so the pin holds at full strength
    // in every environment: on a plain home both expectations are
    // true, under a foreign setgid parent the -G expectation flips
    // and the pin demands the failure verdict GNU also gives.
    #[cfg(unix)]
    let (expect_owned_user, expect_owned_group) = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).unwrap();
        (
            metadata.uid() == unsafe { libc::geteuid() },
            metadata.gid() == unsafe { libc::getegid() },
        )
    };
    // Off unix the predicates fall back to existence (see
    // file_owned_by_effective_user / file_owned_by_effective_group
    // in test.rs), so an existing file passes both.
    #[cfg(not(unix))]
    let (expect_owned_user, expect_owned_group) = (true, true);
    let verdict = |owned: bool| {
        if owned {
            EXECUTION_SUCCESS
        } else {
            EXECUTION_FAILURE
        }
    };

    assert_eq!(run(&["-O", path], false).0, verdict(expect_owned_user));
    assert_eq!(run(&["-G", path], false).0, verdict(expect_owned_group));
    assert_eq!(run(&["-O", missing], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-G", missing], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(path);
}

#[cfg(unix)]
#[test]
fn unix_file_unary_operators_check_file_types_and_mode_bits() {
    let base = std::env::temp_dir().join(format!("rubash-test-file-kind-{}", std::process::id()));
    let fifo_path = base.join("fifo");
    let socket_path = base.join("socket");
    let mode_path = base.join("mode.txt");
    let sticky_dir = base.join("sticky-dir");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let fifo_arg = path_arg(&fifo_path);
    let socket_arg = path_arg(&socket_path);
    let mode_arg = path_arg(&mode_path);
    let sticky_arg = path_arg(&sticky_dir);

    make_fifo(&fifo_arg);
    let _socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    fs::write(&mode_path, "data").unwrap();
    let mut permissions = fs::metadata(&mode_path).unwrap().permissions();
    permissions.set_mode(0o7600);
    fs::set_permissions(&mode_path, permissions).unwrap();
    fs::create_dir_all(&sticky_dir).unwrap();
    let mut permissions = fs::metadata(&sticky_dir).unwrap().permissions();
    permissions.set_mode(0o1700);
    fs::set_permissions(&sticky_dir, permissions).unwrap();

    assert_eq!(run(&["-p", &fifo_arg], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-S", &socket_arg], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-u", &mode_arg], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-g", &mode_arg], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-k", &sticky_arg], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-b", &mode_arg], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-c", &mode_arg], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-p", &mode_arg], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-S", &mode_arg], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_dir_all(&base);
}

#[cfg(unix)]
fn path_arg(path: &PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(not(unix))]
#[test]
fn unix_file_unary_operators_default_false_on_non_unix() {
    let path = "target/rubash-test-file-kind-non-unix.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    fs::write(path, "data").unwrap();

    for op in ["-b", "-c", "-g", "-k", "-p", "-S", "-u"] {
        assert_eq!(run(&[op, path], false).0, EXECUTION_FAILURE);
    }

    let _ = fs::remove_file(path);
}

#[test]
fn terminal_unary_operator_checks_standard_fds() {
    let expected_stdin = if std::io::stdin().is_terminal() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };
    let expected_stdout = if std::io::stdout().is_terminal() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };
    let expected_stderr = if std::io::stderr().is_terminal() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };

    assert_eq!(run(&["-t", "0"], false).0, expected_stdin);
    assert_eq!(run(&["-t", "1"], false).0, expected_stdout);
    assert_eq!(run(&["-t", "2"], false).0, expected_stderr);
    assert_eq!(run(&["-t", "9999"], false).0, EXECUTION_FAILURE);
    // GNU 5.3: `test -t nope` is a syntax error (integer expected), exit 2.
    assert_eq!(run(&["-t", "nope"], false).0, EX_BADUSAGE);
}

#[test]
fn supports_parenthesized_logical_expressions() {
    assert_eq!(
        run(&["(", "", "-o", "x", ")", "-a", ""], false).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run(&["(", "", "-o", "x", ")", "-a", "x"], false).0,
        EXECUTION_SUCCESS
    );
}

#[test]
fn parenthesized_subexpression_with_posixtest_fast_path() {
    // GNU 5.3 test.c:287-296 uses posixtest(nargs) for parenthesized
    // sub-expressions with ≤4 arguments.  `test true -a ( ! -a )` has
    // 2 arguments inside the parens; the fast-path prevents the inner
    // `expr()` from consuming the closing `)`.
    assert_eq!(
        run(&["true", "-a", "(", "!", "-a", ")"], false).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run(&["true", "-a", "(", "-n", "foo", ")"], false).0,
        EXECUTION_SUCCESS
    );
    assert_eq!(
        run(&["true", "-a", "(", "foo", ")"], false).0,
        EXECUTION_SUCCESS
    );
}

#[test]
fn virtual_device_stdout_stderr_are_writable() {
    // /dev/fd/1, /dev/fd/2, /dev/stdout, /dev/stderr are writable
    // virtual devices (matching GNU's sh_eaccess on open fds).
    for path in &["/dev/fd/1", "/dev/fd/2", "/dev/stdout", "/dev/stderr"] {
        assert_eq!(run(&["-w", path], false).0, EXECUTION_SUCCESS);
        assert_eq!(run(&["-e", path], false).0, EXECUTION_SUCCESS);
    }
}

#[test]
fn bracket_requires_closing_bracket() {
    let (status, stderr) = run(&["x"], true);

    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("missing `]'"));
}

// Semantic edge regressions (GNU bash 5.3 baseline, byte-verified against
// local bash 5.3.20 2026-10-20; WSL 5.3.0 re-check by the main session):
// `test`'s integer path is decimal-only (valid_number, general.c), `-t`
// without an operand is a plain non-empty string, and `-a` binds tighter
// than `-o`.

/// T1: leading-zero operands are DECIMAL (010 == 10, not octal 8).
#[test]
fn t1_leading_zero_is_decimal() {
    assert_eq!(run(&["010", "-eq", "10"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["010", "-eq", "8"], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["010", "-eq", "010"], false).0, EXECUTION_SUCCESS);
}

/// T2: `0x` is not a valid test integer (exit 2); `08` passes as decimal 8.
#[test]
fn t2_hex_is_integer_expected_but_08_passes() {
    let (status, stderr) = run(&["0x10", "-eq", "16"], false);
    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("integer expected"), "stderr: {stderr}");
    let (status, stderr) = run(&["0x10", "-eq", "0x10"], false);
    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("integer expected"), "stderr: {stderr}");
    assert_eq!(run(&["08", "-eq", "8"], false).0, EXECUTION_SUCCESS);
}

/// T3: empty operands to a numeric binary operator are exit 2, not 0/1.
#[test]
fn t3_empty_operand_is_integer_expected() {
    let (status, stderr) = run(&["", "-eq", ""], false);
    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("integer expected"), "stderr: {stderr}");
    let (status, _) = run(&["", "-eq", "0"], false);
    assert_eq!(status, EX_BADUSAGE);
}

/// T4: i64 overflow/underflow are exit 2 (strtoimax ERANGE parity).
#[test]
fn t4_overflow_is_integer_expected() {
    assert_eq!(
        run(
            &["9223372036854775807", "-eq", "9223372036854775807"],
            false
        )
        .0,
        EXECUTION_SUCCESS
    );
    let (status, stderr) = run(&["9223372036854775808", "-eq", "0"], false);
    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("integer expected"), "stderr: {stderr}");
    let (status, _) = run(&["-9223372036854775809", "-eq", "0"], false);
    assert_eq!(status, EX_BADUSAGE);
}

/// T5: `-t` four shapes — bare `-t` is a non-empty string (true), `-t N`
/// probes the fd, `-t nope` is exit 2, `-t -a x` is string `-a` string.
#[test]
fn t5_dash_t_four_shapes() {
    assert_eq!(run(&["-t"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-t", "9999"], false).0, EXECUTION_FAILURE);
    let (status, stderr) = run(&["-t", "nope"], false);
    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("integer expected"), "stderr: {stderr}");
    assert_eq!(run(&["-t", "-a", "x"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-t", "-a", ""], false).0, EXECUTION_FAILURE);
}

/// T6: a single argument is always a string test — even operator-looking
/// spellings are true when non-empty.
#[test]
fn t6_single_argument_is_string_truth() {
    for arg in ["-e", "=", "-ne", "!", "(", ")", "-v", "-R", "-o", "-z"] {
        assert_eq!(run(&[arg], false).0, EXECUTION_SUCCESS, "arg: {arg}");
    }
    assert_eq!(run(&[""], false).0, EXECUTION_FAILURE);
}

/// T7: `!` negation, including stacked `! !`.
#[test]
fn t7_bang_negation() {
    assert_eq!(run(&["!", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["!", "hello"], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["!", "!", "hello"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["!", "!", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["!", "-n", "hello"], false).0, EXECUTION_FAILURE);
}

/// T8: parenthesized groups and their traps (unterminated / trailing args).
#[test]
fn t8_paren_group_traps() {
    assert_eq!(run(&["(", "hello", ")"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["(", "", ")"], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["(", "-n", "foo", ")"], false).0, EXECUTION_SUCCESS);
    let (status, _) = run(&["(", "hello"], false);
    assert_eq!(status, EX_BADUSAGE);
    let (status, stderr) = run(&["(", "hello", ")", "extra"], false);
    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("too many arguments"), "stderr: {stderr}");
}

/// T9: `-a` binds tighter than `-o`.
#[test]
fn t9_and_binds_tighter_than_or() {
    assert_eq!(run(&["", "-a", "", "-o", "x"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["x", "-o", "", "-a", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["", "-o", "x", "-a", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["", "-o", "", "-a", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["x", "-a", "x", "-o", ""], false).0, EXECUTION_SUCCESS);
}

/// T10: combined `!`, parens, `-a`, `-o`.
#[test]
fn t10_combined_negation_group_and_or() {
    assert_eq!(
        run(&["!", "(", "x", "-a", "", ")", "-o", "y"], false).0,
        EXECUTION_SUCCESS
    );
    assert_eq!(
        run(&["!", "(", "x", "-a", "y", ")", "-o", ""], false).0,
        EXECUTION_FAILURE
    );
}

/// Arity matrix 0..6: 0 is false; 1 is string truth; 2 is `!`/unary;
/// 3 is binary; 4 is `!`-binary; 5+ is expr with `-a` > `-o`.
#[test]
fn arity_matrix_zero_through_six() {
    assert_eq!(run(&[], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&[""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["hello"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["!", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-n", "hello"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["a", "=", "a"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["a", "=", "b"], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["!", "a", "=", "a"], false).0, EXECUTION_FAILURE);
    assert_eq!(
        run(&["a", "=", "a", "-a", "b", "=", "b"], false).0,
        EXECUTION_SUCCESS
    );
    assert_eq!(
        run(
            &["a", "=", "a", "-o", "b", "=", "c", "-a", "d", "=", "d"],
            false
        )
        .0,
        EXECUTION_SUCCESS
    );
    assert_eq!(
        run(
            &["a", "=", "b", "-o", "c", "=", "c", "-a", "d", "=", "e"],
            false
        )
        .0,
        EXECUTION_FAILURE
    );
}

/// `-t`/`-v` complement: `-v` honors set state, `-R` is false for plain
/// vars, `-o` reports a disabled option as false.
#[test]
fn dash_t_dash_v_complement() {
    let mut values = std::collections::HashMap::new();
    values.insert("PROBE_SET_VAR".to_string(), "1".to_string());
    let env_vars = crate::shell::var_table::VarTable::from_values(values);
    assert_eq!(
        run_with_env(&["-v", "PROBE_SET_VAR"], false, &env_vars).0,
        EXECUTION_SUCCESS
    );
    assert_eq!(
        run_with_env(&["-v", "PROBE_MISSING_VAR"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
    assert_eq!(run(&["-v", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(
        run_with_env(&["-R", "PROBE_SET_VAR"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
    assert_eq!(run(&["-z", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-n", ""], false).0, EXECUTION_FAILURE);
}
