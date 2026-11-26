//! unixwin/rubash#705 R7.3: `-o` / `+o` with no option word is the LIST form.
//!
//! GNU shell.c:931-938: `case 'o': o_option = argv[next_arg]; if (o_option
//! == 0) { set_option_defaults (); list_minus_o_opts (-1, on_or_off == '-'
//! ? 0 : 1); reset_option_defaults (); break; }` — an argument-less `-o`
//! prints the `name<TAB>state` table and `+o` the `set +o name` recreate
//! list, then option parsing CONTINUES. It is not a usage error. The engine
//! binary already listed through its own parser (rubash#547); the shared
//! `ShellInvocation` surface (the niu host's parser) still rejected it, so
//! `niu -o` printed `bash: -o: option requires an argument`, rc 2.
//!
//! The listing is emitted during option application so every host that
//! applies the invocation gets GNU's bytes; `write_shell_options_listing`
//! exposes the writer used there for byte-level assertions.

use std::path::{Path, PathBuf};
use std::process::Command;

use rubash::executor::Executor;
use rubash::invocation::ShellInvocation;

fn invocation(args: &[&str]) -> ShellInvocation {
    ShellInvocation::parse(
        &args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>(),
    )
    .expect("parse invocation")
}

fn listing(invocation: &ShellInvocation, executor: &Executor) -> String {
    let mut out = Vec::new();
    invocation
        .write_shell_options_listing(executor, &mut out)
        .expect("write listing");
    String::from_utf8(out).expect("utf-8 listing")
}

/// The argument-less forms parse okay and record the request; the named form
/// stays a normal option pair.
#[test]
fn argumentless_o_is_a_listing_not_an_error() {
    let parsed = invocation(&["-o"]);
    assert_eq!(parsed.list_shell_options, Some(false));
    assert!(parsed.shell_flags.is_empty());

    let parsed = invocation(&["+o"]);
    assert_eq!(parsed.list_shell_options, Some(true));
    assert!(parsed.shell_flags.is_empty());

    let parsed = invocation(&["-o", "pipefail"]);
    assert_eq!(parsed.list_shell_options, None);
    assert!(parsed.shell_flags.contains(&("pipefail".into(), true)));

    let parsed = invocation(&["+o", "pipefail"]);
    assert_eq!(parsed.list_shell_options, None);
    assert!(parsed.shell_flags.contains(&("pipefail".into(), false)));
}

/// `-o` renders the set -o name/state table, `+o` the recreate list — the
/// same bytes `set -o` / `set +o` print (options.c print_all_shell_options).
#[test]
fn listing_matches_the_set_o_table_and_recreate_form() {
    let executor = Executor::new();

    let table = listing(&invocation(&["-o"]), &executor);
    assert!(table.starts_with("allexport      \toff\n"), "{table:?}");
    assert!(table.contains("errexit        \toff\n"), "{table:?}");
    assert!(table.contains("braceexpand    \ton\n"), "{table:?}");

    let recreate = listing(&invocation(&["+o"]), &executor);
    assert!(recreate.starts_with("set +o allexport\n"), "{recreate:?}");
    assert!(recreate.contains("set +o errexit\n"), "{recreate:?}");
}

/// Flags parsed before the request are applied first, so the table shows
/// their live state, exactly like GNU's left-to-right option walk.
#[test]
fn listing_reflects_flags_parsed_before_it() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["-e", "-o"], &["errexit        \ton\n"]),
        (
            &["-euo"],
            &["errexit        \ton\n", "nounset        \ton\n"],
        ),
        (
            &["-eu", "-o"],
            &["errexit        \ton\n", "nounset        \ton\n"],
        ),
    ];
    for (args, expected_lines) in cases {
        let parsed = invocation(args);
        assert_eq!(parsed.list_shell_options, Some(false), "{args:?}");
        let mut executor = Executor::new();
        parsed
            .apply_to_executor_before_startup(&mut executor)
            .expect("apply");
        let table = listing(&parsed, &executor);
        for line in *expected_lines {
            assert!(table.contains(line), "{args:?}: {table:?}");
        }
    }
}

/// A `-o <name>` invocation writes nothing (the writer is a no-op).
#[test]
fn named_option_form_writes_no_listing() {
    let executor = Executor::new();
    assert_eq!(listing(&invocation(&["-o", "pipefail"]), &executor), "");
    assert_eq!(listing(&invocation(&["-c", "true"]), &executor), "");
}

/// Engine route: `rubash -o` / `+o` list with rc 0, and the named form still
/// validates the option name.
#[test]
fn engine_lists_argumentless_o() {
    let dir = sandbox();
    let (rc, stdout, stderr) = run(&dir, &["-o"]);
    assert_eq!((rc, stderr.as_str()), (0, ""), "-o");
    assert!(stdout.starts_with("allexport      	off\n"), "{stdout:?}");

    let (rc, stdout, stderr) = run(&dir, &["+o"]);
    assert_eq!((rc, stderr.as_str()), (0, ""), "+o");
    assert!(stdout.starts_with("set +o allexport\n"), "{stdout:?}");

    let (rc, _stdout, stderr) = run(&dir, &["-o", "bogusopt"]);
    assert_eq!(rc, 2);
    assert!(stderr.contains("invalid option name"), "{stderr:?}");
}

fn sandbox() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-705-optlist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("HOME", dir)
        .output()
        .expect("spawn rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
