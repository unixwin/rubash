use rubash::executor::Executor;
use rubash::invocation::ShellInvocation;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn invocation(args: &[&str]) -> ShellInvocation {
    ShellInvocation::parse(
        &args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn run(executor: &mut Executor, source: &str) -> i32 {
    let tokens = tokenize(source);
    let ast = parse(&tokens);
    executor.execute_ast(&ast).unwrap();
    executor.last_exit_code()
}

#[test]
fn library_restricted_short_and_long_flags_execute_command() {
    for flag in ["-r", "--restricted"] {
        let parsed = invocation(&[flag, "-c", "flags=$-; cd .", "demo", "arg"]);
        let mut executor = Executor::new();
        parsed.apply_to_executor(&mut executor).unwrap();
        assert_eq!(run(&mut executor, parsed.command.as_deref().unwrap()), 1);
        assert!(executor.get_env("flags").unwrap().contains('r'));
        assert_eq!(run(&mut executor, "identity=$0; first=$1"), 0);
        assert_eq!(executor.get_env("identity"), Some("demo"));
        assert_eq!(executor.get_env("first"), Some("arg"));
    }
}

#[test]
fn restricted_is_not_a_named_shell_option() {
    let parsed = invocation(&["-r", "-c", "true"]);
    let mut executor = Executor::new();
    parsed.apply_to_executor(&mut executor).unwrap();
    assert!(!executor.is_shell_option("restricted"));
    assert!(!executor
        .get_env("SHELLOPTS")
        .unwrap()
        .split(':')
        .any(|name| name == "restricted"));
    for recreate in [false, true] {
        let mut output = Vec::new();
        executor
            .print_shell_options_listing(recreate, &mut output)
            .unwrap();
        assert!(!String::from_utf8(output).unwrap().contains("restricted"));
    }
    assert_eq!(run(&mut executor, "set -o restricted"), 2);
    assert_eq!(run(&mut executor, "set +o restricted"), 2);
}

#[test]
fn invocation_named_restricted_option_remains_invalid() {
    for option in ["-o", "+o"] {
        for args in [
            vec![option, "restricted", "-c", "true"],
            vec!["-c", option, "restricted", "true"],
        ] {
            let parsed = invocation(&args);
            assert_eq!(
                parsed.apply_to_executor(&mut Executor::new()),
                Err("restricted: invalid shell option name".to_string())
            );
        }
    }
}

#[test]
fn combined_restricted_flags_preserve_other_invocation_options() {
    for args in [
        vec!["-erc", "flags=$-"],
        vec!["-c", "-er", "flags=$-"],
        vec!["--restricted", "-ec", "flags=$-"],
    ] {
        let parsed = invocation(&args);
        let mut executor = Executor::new();
        parsed.apply_to_executor(&mut executor).unwrap();
        assert_eq!(run(&mut executor, parsed.command.as_deref().unwrap()), 0);
        let flags = executor.get_env("flags").unwrap();
        assert!(flags.contains('e') && flags.contains('r'), "{flags}");
    }
}

#[test]
fn startup_runs_before_invocation_restrictions_are_activated() {
    for flag in ["-r", "--restricted"] {
        let parsed = invocation(&[flag, "-c", "true"]);
        let mut executor = Executor::new();
        parsed
            .apply_to_executor_before_startup(&mut executor)
            .unwrap();
        assert_eq!(run(&mut executor, "startup_flags=$-; cd ."), 0);
        assert!(!executor.get_env("startup_flags").unwrap().contains('r'));
        assert_eq!(run(&mut executor, "PATH=$PATH; startup_completed=yes"), 0);
        assert_eq!(executor.get_env("startup_completed"), Some("yes"));
        parsed.finish_startup(&mut executor);
        assert_eq!(run(&mut executor, "body_flags=$-; cd ."), 1);
        assert!(executor.get_env("body_flags").unwrap().contains('r'));
        assert_ne!(run(&mut executor, "PATH=$PATH"), 0);
    }
}

#[test]
fn unrestricted_invocation_does_not_undo_startup_set_r() {
    let parsed = invocation(&["-c", "true"]);
    let mut executor = Executor::new();
    parsed
        .apply_to_executor_before_startup(&mut executor)
        .unwrap();
    assert_eq!(run(&mut executor, "set -r"), 0);
    parsed.finish_startup(&mut executor);
    assert_eq!(run(&mut executor, "flags=$-; cd ."), 1);
    assert!(executor.get_env("flags").unwrap().contains('r'));
}

#[test]
fn unrestricted_invocation_preserves_existing_options() {
    let parsed = invocation(&["-eu", "-o", "pipefail", "-c", "flags=$-; cd ."]);
    let mut executor = Executor::new();
    parsed.apply_to_executor(&mut executor).unwrap();
    assert_eq!(run(&mut executor, parsed.command.as_deref().unwrap()), 0);
    let flags = executor.get_env("flags").unwrap();
    assert!(flags.contains('e') && flags.contains('u') && !flags.contains('r'));
    assert!(executor.get_env("SHELLOPTS").unwrap().contains("pipefail"));
}

#[test]
fn restricted_invocation_cannot_be_cancelled_by_plus_r() {
    for args in [vec!["-r", "+r"], vec!["-c", "-r", "+r", "true"]] {
        assert!(ShellInvocation::parse(
            &args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        )
        .is_err());
    }
}
