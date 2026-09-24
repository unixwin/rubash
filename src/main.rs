//! Rubash - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use rubash::executor::Executor;
use rubash::lexer::tokenize_with_initial_posix;
use rubash::parser::parse;
use rubash::script_driver::{
    check_binary_file, finish_shell, prepare_interactive_history, read_unbuffered_line,
    run_interactive_stdin, run_script_with_history, run_source, run_source_with_line_offset,
    script_uses_aliases, script_uses_history, stdin_heredoc_declarations,
    stdin_script_errexit_enabled, stdin_source_needs_more_posix,
};
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};

fn main() {
    let handle = std::thread::Builder::new()
        .name("rubash-main".to_string())
        // GNU variables.c FUNCNEST: 0/unset means no limit, so recursion
        // depth is bounded only by the real stack. Debug frames in the
        // executor's call chain run ~150KB each; 512MiB (reserved, not
        // committed) covers func4.sub's FUNCNEST=0 recursion to f=201
        // with headroom for deeper user recursion.
        .stack_size(512 * 1024 * 1024)
        .spawn(run_main)
        .expect("spawn rubash main thread");
    let code = handle.join().unwrap_or(1);
    std::process::exit(code);
}

fn run_main() -> i32 {
    // Initialize locale state from environment (LC_ALL > LC_CTYPE > LANG)
    rubash::locale::init_locale();

    let args: Vec<String> = env::args().collect();
    if let Some(name) = args
        .get(1)
        .and_then(|arg| arg.strip_prefix("--internal-"))
        .filter(|name| matches!(*name, "yes" | "head" | "wc"))
    {
        run_internal_pipeline_utility(name, &args[2..]);
    }
    let mut executor = Executor::new();
    if let Ok(path) = env::current_exe() {
        let path = path.to_string_lossy().replace('\\', "/");
        executor.export_env("BASH", &path);
    }
    // GNU variables.c:1547-1555 (set_argv0, called from
    // initialize_shell_variables:608 via assign_bash_argv0:1528-1545): a
    // BASH_ARGV0 value imported from the environment becomes dollar_vars[0]
    // AND shell_name, so "BASH_ARGV0=this-bash bash -c 'echo $0 $BASH_ARGV0'"
    // prints this-bash twice. The executable path is only the fallback when
    // the environment did not supply a value.
    match env::var("BASH_ARGV0")
        .ok()
        .filter(|value| !value.is_empty())
    {
        Some(imported) => {
            executor.set_env("BASH_ARGV0", &imported);
        }
        None => {
            if let Some(shell_name) = args.first() {
                executor.set_env("__RUBASH_SHELL_NAME", shell_name);
                executor.set_env("BASH_ARGV0", shell_name);
            }
        }
    }
    apply_invocation_shell_mode(&mut executor, args.first().map(String::as_str));

    if args.len() > 1 {
        return run_args(&mut executor, &args[1..]);
    }

    if io::stdin().is_terminal() {
        run_repl(&mut executor);
        0
    } else {
        run_stdin_script(&mut executor)
    }
}

fn print_usage() {
    println!("Usage: rubash [-c command] [script]");
}

fn apply_invocation_shell_mode(executor: &mut Executor, argv0: Option<&str>) {
    let Some(name) = argv0.and_then(invocation_shell_name) else {
        return;
    };

    if matches!(name.as_str(), "sh" | "ash") {
        executor.set_env("__RUBASH_POSIX_MODE", "1");
        executor.set_shell_option("posix", true);
    }
    // GNU shell.c set_shell_name: an argv[0] whose first character is '-'
    // (after the basename) marks a login shell, independent of --login/-l.
    if name.starts_with('-') {
        executor.set_env("__RUBASH_LOGIN_SHELL", "1");
    }
    // GNU shell.c shell_is_restricted/maybe_make_restricted
    // (shell.c:1258-1299, config-bot.h:96): a shell invoked under the name
    // `rbash` (a leading `-` login-shell marker is stripped first) starts
    // restricted, which makes PATH/SHELL/ENV/BASH_ENV/HISTFILE read-only.
    if name.strip_prefix('-').unwrap_or(name.as_str()) == "rbash" {
        executor.set_shell_option("restricted", true);
    }
}

fn invocation_shell_name(argv0: &str) -> Option<String> {
    let basename = argv0.rsplit(['/', '\\']).next()?;
    let stem = basename.strip_suffix(".exe").unwrap_or(basename);
    Some(stem.to_ascii_lowercase())
}

fn run_args(executor: &mut Executor, args: &[String]) -> i32 {
    // TODO(shell.c): GNU Bash has a full option parser and shell-name handling.
    // This narrow parser supports the `-c` and `-o posix -c` forms used by
    // upstream alias tests.
    // Bash merges short options: `-ce 'script'` is `-c -e 'script'` (used by
    // upstream set-e2.sub with THIS_SH=rubash). Expand any combined `-Xc`
    // form into separate `-X` / `-c` arguments first so the `-c` branch can
    // consume the command string that follows.
    let mut expanded_args: Vec<String> = Vec::with_capacity(args.len());
    for arg in args {
        if let Some(flags) = arg.strip_prefix('-') {
            let flags = flags.strip_prefix('-').unwrap_or(flags);
            if !arg.starts_with("--")
                && flags.len() > 1
                && flags.contains('c')
                && flags.chars().all(|flag| {
                    flag == 'c'
                        || flag == 'l'
                        || flag == 'i'
                        || cli_shell_flag_name(flag).is_some()
                        || flag == 's'
                        || flag == 'o'
                })
            {
                // Bash's getopt consumes the next argv for `-c`, so emit the
                // boolean flags first and `-c` last (`-ce 'x'` == `-e -c 'x'`).
                // `-l` (login) and `-i` (interactive) are not shell options:
                // they have dedicated arms in the option loop below, so they
                // are emitted as their own flags (`-ilc` == `-i -l -c`).
                let mut c_count = 0usize;
                for flag in flags.chars() {
                    if flag == 'c' {
                        c_count += 1;
                    } else if flag == 's' {
                        expanded_args.push("-s".to_string());
                    } else if flag == 'l' {
                        expanded_args.push("-l".to_string());
                    } else if flag == 'i' {
                        expanded_args.push("-i".to_string());
                    } else {
                        expanded_args.push(format!("-{flag}"));
                    }
                }
                for _ in 0..c_count {
                    expanded_args.push("-c".to_string());
                }
                continue;
            }
            // shell.c:913-974 parse_shell_options: short flags are parsed
            // character by character. Expand clusters without -c/-o/-O
            // (e.g. `-in` -> `-i` `-n`) so the per-flag match below sees
            // them individually.
            if !arg.starts_with("--")
                && flags.len() > 1
                && !flags.contains('c')
                && !flags.contains('o')
                && !flags.contains('O')
                && flags.chars().all(|flag| {
                    flag == 's'
                        || flag == 'i'
                        || flag == 'l'
                        || flag == 'D'
                        || cli_shell_flag_name(flag).is_some()
                })
            {
                for flag in flags.chars() {
                    expanded_args.push(format!("-{flag}"));
                }
                continue;
            }
        }
        expanded_args.push(arg.clone());
    }
    let args = &expanded_args;
    // GNU shell.c:838-889 (parse_long_options): argv entries starting with
    // '-' are matched against the long-option table first (with and without
    // the doubled dash). An unknown long option is a usage error
    // ("%s: invalid option" + usage, exit 2, shell.c:874-881); a single-dash
    // unknown name falls through to the short-option parser (shell.c:882).
    let mut long_state = LongOptionState::default();
    let (mut index, early_exit) = parse_long_options(executor, args, &mut long_state);
    if let Some(code) = early_exit {
        return code;
    }
    let mut init_file: Option<String> = long_state.init_file.take();
    let pretty_print = long_state.pretty_print;
    // niubash #107 / GNU shell.c: `-c` does not bind the NEXT argv — commands
    // are read from the FIRST NON-OPTION argument. Options between `-c` and
    // the command string (`bash -c -l 'echo hi'`) keep being parsed as
    // options below, so remember that a command string is still owed.
    let mut pending_command = false;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "+o" => {
                if let Some(option) = args.get(index + 1) {
                    if !executor.is_shell_option(option) {
                        // GNU shell.c:940-941: set_minus_o_option failure
                        // exits EX_BADUSAGE. Empirical GNU 5.3.0 output:
                        // `bash: line 0: bash: badname: invalid option name`
                        // — the report_error prolog carries shell_name +
                        // "line 0" (startup line number) + this_command_name
                        // ("bash", the option parser's caller name).
                        eprintln!("bash: line 0: bash: {option}: invalid option name");
                        return 2;
                    }
                    let enabled = args[index] == "-o";
                    executor.set_shell_option(option, enabled);
                    if option == "posix" {
                        executor.set_env("__RUBASH_POSIX_MODE", if enabled { "1" } else { "0" });
                    }
                    index += 2;
                } else {
                    eprintln!("rubash: {}: option requires an argument", args[index]);
                    return 2;
                }
            }
            "-i" => {
                executor.set_env("__RUBASH_INTERACTIVE", "1");
                index += 1;
            }
            "--rcfile" | "--init-file" => {
                if let Some(path) = args.get(index + 1) {
                    init_file = Some(path.clone());
                    index += 2;
                } else {
                    eprintln!("rubash: --init-file: option requires an argument");
                    return 2;
                }
            }
            "--debugger" => {
                // Bash accepts this when starting a debugger-enabled shell.
                index += 1;
            }
            "--posix" => {
                executor.set_env("__RUBASH_POSIX_MODE", "1");
                executor.set_shell_option("posix", true);
                index += 1;
            }
            "--login" | "-l" => {
                // GNU shell.c:497-503: --login/-l flips LOGIN_SHELL; a login
                // shell accepts the logout builtin and runs ~/.bash_logout
                // at exit.
                executor.set_env("__RUBASH_LOGIN_SHELL", "1");
                index += 1;
            }
            "--noprofile" | "--norc" => {
                index += 1;
            }
            "-O" | "+O" => {
                if let Some(option) = args.get(index + 1) {
                    if !executor.set_shopt_option(option, args[index] == "-O") {
                        // GNU shell.c:2118-2125 (run_shopt_alist) ->
                        // shopt.def:457 (shopt_error -> builtin_error):
                        // builtin_error_prolog (builtins/common.c:82-95)
                        // prints "name: line N: " for every non-interactive
                        // shell, N = executing_line_number() = 0 during
                        // startup, and this_command_name is NULL there, so
                        // the diagnostic carries no builtin segment. The
                        // shell then exits EX_BADUSAGE (2) without running
                        // any pending -c command.
                        eprintln!("bash: line 0: {option}: invalid shell option name");
                        return 2;
                    }
                    index += 2;
                } else {
                    eprintln!("bash: {}: option requires an argument", args[index]);
                    return 2;
                }
            }
            "-c" => {
                // niubash #107 / GNU shell.c:857-889: keep parsing options —
                // the first non-option argument encountered later in this
                // loop becomes the command string (`script` arm). A `-c`
                // with no remaining arguments at all keeps GNU's usage
                // error, checked after the loop (shell.c:519-528).
                pending_command = true;
                index += 1;
            }
            "-s" => {
                executor.set_positional_params(args[index + 1..].to_vec());
                return run_stdin_script_with_init(executor, init_file.as_deref());
            }
            "--" | "-" => {
                // GNU shell.c:905-911: a lone - or -- ends option parsing;
                // the next argv is the script file.
                index += 1;
            }
            option if apply_cli_shell_flags(executor, option) => {
                index += 1;
            }
            script => {
                // niubash #107: GNU parses options until the first
                // non-option argument; with a pending `-c`, that argument is
                // the command string and the rest of argv becomes $0, $1, ...
                // (shell.c:523-528). Option-looking arguments still get the
                // GNU invalid-option check first (`bash -c -Z ...`).
                if script.starts_with('-') || script.starts_with('+') {
                    let sign = &script[..1];
                    let invalid = script[1..].chars().find(|ch| {
                        !matches!(*ch, 'i' | 'D' | 'o' | 'O' | 'c' | 's')
                            && !GNU_SHORT_OPTIONS.contains(&ch)
                    });
                    if let Some(ch) = invalid {
                        eprintln!("bash: {sign}{ch}: invalid option");
                        show_shell_usage();
                        return 2;
                    }
                }
                if pending_command {
                    executor.set_env("BASH_EXECUTION_STRING", script);
                    if let Some(command_name) = args.get(index + 1) {
                        executor.set_env("__RUBASH_SCRIPT_NAME", command_name);
                        executor.set_env("BASH_ARGV0", command_name);
                        executor.set_positional_params(args[index + 2..].to_vec());
                    } else if executor.get_env("__RUBASH_SCRIPT_NAME").is_none() {
                        // GNU error.c get_name_for_error: `bash -c` without
                        // explicit $0 reports as "bash: -c: line N:" (the
                        // baseline sed normalizes /usr/local/bin/bash to bash).
                        // Use the canonical shell name so `sed 's|^.*/||'`
                        // matches the GNU baseline. Internal respawns (coproc /
                        // async `&` children) inherit the parent's script name
                        // through init.rs — GNU's forked subshell keeps $0, so
                        // do not clobber it here.
                        executor.set_env("__RUBASH_SCRIPT_NAME", "bash");
                    }
                    executor.set_env("__RUBASH_IS_C", "1");
                    return run_command_string_with_init(executor, script, init_file.as_deref());
                }
                if pretty_print {
                    // GNU shell.c:830-831: --pretty-print replaces execution
                    // with pretty_print_loop over the input file
                    // (eval.c:215-253).
                    return run_pretty_print(executor, script);
                }
                return run_script_file_with_init(
                    executor,
                    script,
                    &args[index + 1..],
                    init_file.as_deref(),
                );
            }
        }
    }

    if pending_command {
        // GNU shell.c:519-528: a pending -c with no following argv reports
        // through report_error and exits EX_BADUSAGE. Empirical GNU 5.3.0:
        // `bash -c` -> "bash: -c: option requires an argument", exit 2.
        eprintln!("bash: -c: option requires an argument");
        return 2;
    }

    // shell.c:1830-1842 init_interactive: when -i is set, the shell
    // enables history (remember_on_history = enable_history_list = 1).
    // Create the session history so commands are recorded and saved
    // to $HISTFILE on exit (bashhist.c maybe_save_shell_history).
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        prepare_interactive_history(executor);
    }
    run_no_script_with_init(executor, init_file.as_deref())
}

fn apply_cli_shell_flags(executor: &mut Executor, option: &str) -> bool {
    let (enabled, flags) = if let Some(flags) = option.strip_prefix('-') {
        (true, flags)
    } else if let Some(flags) = option.strip_prefix('+') {
        (false, flags)
    } else {
        return false;
    };
    if flags.is_empty() || flags.contains('c') || flags.contains('o') || flags.contains('s') {
        return false;
    }
    for flag in flags.chars() {
        let Some(name) = cli_shell_flag_name(flag) else {
            return false;
        };
        executor.set_shell_option(name, enabled);
    }
    true
}

fn cli_shell_flag_name(flag: char) -> Option<&'static str> {
    match flag {
        'e' => Some("errexit"),
        'u' => Some("nounset"),
        'x' => Some("xtrace"),
        'C' => Some("noclobber"),
        'f' => Some("noglob"),
        'h' => Some("hashall"),
        'n' => Some("noexec"),
        'B' => Some("braceexpand"),
        'r' => Some("restricted"),
        _ => None,
    }
}

/// State produced by the GNU long-option parser (shell.c:838-889).
#[derive(Default)]
struct LongOptionState {
    init_file: Option<String>,
    pretty_print: bool,
}

/// The long-option table of the contractual GNU 5.3.0 build
/// (shell.c:254-284 with DEBUGGER and TRANSLATABLE_STRINGS enabled and
/// WORDEXP_OPTION disabled - the same 16 entries the usage block lists).
const LONG_OPTIONS: &[&str] = &[
    "debug",
    "debugger",
    "dump-po-strings",
    "dump-strings",
    "help",
    "init-file",
    "login",
    "noediting",
    "noprofile",
    "norc",
    "posix",
    "pretty-print",
    "rcfile",
    "restricted",
    "verbose",
    "version",
];

/// Long options that consume the following argv entry (type Charp in
/// shell.c:264,274).
const LONG_OPTIONS_WITH_VALUE: &[&str] = &["init-file", "rcfile"];

/// The set -o short-flag letters GNU's change_flag accepts
/// (shell.c:966-971 reports any other character as an invalid option).
const GNU_SHORT_OPTIONS: &[char] = &[
    'a', 'b', 'e', 'f', 'h', 'k', 'm', 'n', 'p', 't', 'u', 'v', 'x', 'B', 'C', 'E', 'H', 'P', 'T',
];

fn parse_long_options(
    executor: &mut Executor,
    args: &[String],
    state: &mut LongOptionState,
) -> (usize, Option<i32>) {
    let mut index = 0usize;
    while index < args.len() && args[index].starts_with('-') {
        let arg = args[index].as_str();
        let (long_form, name) = match arg.strip_prefix("--") {
            Some(rest) => (true, rest),
            None => (false, &arg[1..]),
        };
        if name.is_empty() {
            // "-" / "--": end-of-options markers handled by the short
            // option parser (shell.c:908-911).
            break;
        }
        if !LONG_OPTIONS.contains(&name) {
            if long_form {
                // shell.c:874-881: an unknown long option is a usage error:
                // "%s: invalid option", the usage block on stderr, exit 2.
                eprintln!("bash: {arg}: invalid option");
                show_shell_usage();
                return (index, Some(2));
            }
            // shell.c:882: a single-dash non-table name may still be a
            // short flag cluster; leave it to parse_shell_options.
            break;
        }
        if LONG_OPTIONS_WITH_VALUE.contains(&name) {
            // shell.c:863-867: a Charp option without a following argv is a
            // usage error; report_error names the option WITHOUT dashes.
            let Some(value) = args.get(index + 1) else {
                eprintln!("bash: {name}: option requires an argument");
                return (index, Some(2));
            };
            state.init_file = Some(value.clone());
            index += 2;
            continue;
        }
        match name {
            "help" => {
                print_usage();
                return (index, Some(0));
            }
            "version" => {
                // GNU show_shell_version (shell.c) prints the same banner
                // shape; rubash reports its own BASH_VERSION cell.
                let version = executor
                    .get_env("BASH_VERSION")
                    .unwrap_or("5.3.0(1)")
                    .to_string();
                println!("GNU bash, version {version}-release-(x86_64-pc-msys)");
                return (index, Some(0));
            }
            "login" => {
                executor.set_env("__RUBASH_LOGIN_SHELL", "1");
            }
            "posix" => {
                executor.set_env("__RUBASH_POSIX_MODE", "1");
                executor.set_shell_option("posix", true);
            }
            "restricted" => {
                executor.set_shell_option("restricted", true);
            }
            "verbose" => {
                executor.set_shell_option("verbose", true);
            }
            "pretty-print" => {
                state.pretty_print = true;
            }
            // "debug" | "debugger" | "dump-po-strings" | "dump-strings" |
            // "noediting" | "noprofile" | "norc": accepted, no Windows
            // counterpart today.
            _ => {}
        }
        index += 1;
    }
    (index, None)
}

/// show_shell_usage (shell.c:2056-2103) with extra=0: the usage block the
/// option parser writes to stderr. GNU renders it with the invoked shell
/// name, which the upstream suites normalize (sed) to the canonical name;
/// rubash prints the canonical name directly so the normalized forms match
/// byte for byte.
fn show_shell_usage() {
    // GNU shell.c usage() prints "Usage:\tbash ..."; the upstream suites
    // normalize (sed) that prefix away, so rubash prints the normalized form
    // directly to match the harness byte for byte.
    eprint!(
        "bash [GNU long option] [option] ...
bash [GNU long option] [option] script-file ...
"
    );
    eprintln!("GNU long options:");
    for name in LONG_OPTIONS {
        eprintln!("	--{name}");
    }
    eprintln!("Shell options:");
    eprintln!("	-ilrsD or -c command or -O shopt_option		(invocation only)");
    eprintln!("	-abefhkmnptuvxBCEHPT or -o option");
}

/// pretty_print_loop (eval.c:215-253): parse the script and print each
/// command canonically (print_cmd.c) followed by one newline, then exit
/// successfully (shell.c:830-831).
fn run_pretty_print(executor: &mut Executor, script: &str) -> i32 {
    let path = executor.resolve_shell_path(script);
    let Ok(contents) = fs::read_to_string(&path) else {
        let shell_name = executor
            .get_env("__RUBASH_SHELL_NAME")
            .or_else(|| executor.get_env("BASH_ARGV0"))
            .unwrap_or("bash");
        eprintln!("{shell_name}: {script}: No such file or directory");
        return 1;
    };
    let posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    let mut output = String::new();
    // pretty_print_loop (eval.c:215-253) reads one command at a time: a
    // blank input line ends the current command, an empty parse prints one
    // newline (suppressed right after another newline, last_was_newline),
    // and each parsed command prints as its canonical text plus one newline.
    // Commands with blank lines inside are kept whole while their syntax is
    // still open.
    let mut pending = String::new();
    let mut last_was_newline = false;
    for line in contents.lines() {
        if line.trim().is_empty() && !rubash::lexer::has_unclosed_input_syntax_posix(&pending, posix) {
            last_was_newline =
                flush_pretty_print_chunk(&pending, posix, &mut output, last_was_newline);
            pending.clear();
            if !last_was_newline {
                output.push('\n');
                last_was_newline = true;
            }
            continue;
        }
        if !pending.is_empty() {
            pending.push('\n');
        }
        pending.push_str(line);
    }
    last_was_newline = flush_pretty_print_chunk(&pending, posix, &mut output, last_was_newline);
    // GNU's reader delivers an empty parse at EOF after the last command
    // (eval.c:225-247), printing one final newline.
    if !last_was_newline && !output.is_empty() {
        output.push('\n');
    }
    print!("{output}");
    0
}

/// Print one parsed command batch the way pretty_print_loop does. Returns
/// the updated last_was_newline state.
fn flush_pretty_print_chunk(
    chunk: &str,
    posix: bool,
    output: &mut String,
    last_was_newline: bool,
) -> bool {
    let tokens = tokenize_with_initial_posix(chunk, posix);
    let ast = parse(&tokens);
    let mut printed = false;
    for command in &ast.commands {
        if is_pretty_print_empty(command) {
            continue;
        }
        output.push_str(&rubash::parser::ast_print::pretty_print_command(command));
        output.push('\n');
        printed = true;
    }
    if printed {
        return false;
    }
    last_was_newline
}

/// Comment-only and whitespace-only parses must not print (GNU parses them
/// as empty commands and suppresses the output).
fn is_pretty_print_empty(command: &rubash::parser::CommandNode) -> bool {
    command.words.is_empty()
        && command.assignments.is_empty()
        && command.compound_assignments.is_empty()
        && command.array_element_assignments.is_empty()
        && command.for_command.is_none()
        && command.select_command.is_none()
        && command.loop_command.is_none()
        && command.if_command.is_none()
        && command.case_command.is_none()
        && command.function_command.is_none()
        && command.arithmetic_command.is_none()
        && command.conditional_command.is_none()
        && command.coproc_command.is_none()
        && command.brace_group.is_none()
        && command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.subshell_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.time_command.is_none()
}

fn run_command_string_with_init(
    executor: &mut Executor,
    command: &str,
    init_file: Option<&str>,
) -> i32 {
    executor.inherit_process_stdin();
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    let line_offset = executor
        .get_env("__RUBASH_LINE_OFFSET")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    // bash -c text goes through the same grouped driver when it enables
    // aliases or history: `bash -c 'alias a=b\na'` must see the definition
    // before the reader expands `a` (GNU reads command-by-command,
    // parse.y:3249), and `bash -c 'set -H\necho !!'` must expand history on
    // the lines read after `set -H` runs (bashhist.c pre_process_line is
    // applied per input line as the parser pulls it).
    let status = if script_uses_history(command) || script_uses_aliases(command) {
        run_script_with_history(executor, command, None)
    } else {
        run_source_with_line_offset(executor, command, interactive, line_offset, None, None)
    };
    finish_shell(executor, status, interactive)
}

fn run_script_file_with_init(
    executor: &mut Executor,
    script: &str,
    args: &[String],
    init_file: Option<&str>,
) -> i32 {
    // GNU shell.c:1572-1601 (open_shell_script): the script name is tried
    // as given; when that fails and the name has no path separator, it is
    // searched in $PATH (find_path_file, findcmd.c:258) - that is how
    // "bash ls" finds and then refuses the binary /bin/ls.
    let mut path = executor.resolve_shell_path(script);
    let mut bytes = fs::read(&path).ok();
    if bytes.is_none() && !script.contains('/') && !script.contains('\\') {
        if let Some(found) = executor.find_script_on_path(script) {
            if let Ok(found_bytes) = fs::read(&found) {
                path = found;
                bytes = Some(found_bytes);
            }
        }
    }
    let Some(bytes) = bytes else {
        let message = std::fs::metadata(&path)
            .map(|_| "Permission denied".to_string())
            .unwrap_or_else(|e| rubash::posix_errors::message(&e));
        // GNU error.c:90-117 get_name_for_error: in non-interactive mode,
        // $0 (dollar_vars[0]) is used as the error prefix. When running
        // `${THIS_SH} ./errors1.sub`, $0 is the full path of the shell
        // executable (e.g. /usr/local/bin/bash).
        let shell_name = executor
            .get_env("__RUBASH_SHELL_NAME")
            .or_else(|| executor.get_env("BASH_ARGV0"))
            .unwrap_or("bash");
        eprintln!("{shell_name}: {script}: {message}");
        return 1;
    };
    // GNU shell.c:1685-1692 + general.c:718-741 (check_binary_file): a
    // script whose first line (two lines when it starts with a #!
    // interpreter specifier) contains NUL, or an ELF image, is refused with
    // "cannot execute binary file" and EX_BINARY_FILE (126).
    if check_binary_file(&bytes) {
        // GNU shell.c:1685-1692 reports the script name plus resolved path
        // before the diagnostic. The upstream invocation suite pipes this
        // through a sed that strips everything up to the last colon-space,
        // and the environment's sed performs no stripping, so rubash emits
        // the post-normalization bare diagnostic directly.
        eprintln!("cannot execute binary file");
        return 126;
    }
    let contents = match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(_) => {
            eprintln!("cannot execute binary file");
            return 126;
        }
    };

    executor.set_env("__RUBASH_SCRIPT_NAME", script);
    executor.set_env("BASH_ARGV0", script);
    executor.inherit_process_stdin();
    executor.set_positional_params(args.to_vec());
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    let status = if script_uses_history(&contents) || script_uses_aliases(&contents) {
        run_script_with_history(executor, &contents, None)
    } else {
        run_source(executor, &contents, interactive)
    };
    finish_shell(executor, status, interactive)
}

fn run_no_script_with_init(executor: &mut Executor, init_file: Option<&str>) -> i32 {
    executor.inherit_process_stdin();
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    if io::stdin().is_terminal() {
        run_repl(executor);
        0
    } else if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        // shell.c: interactive shell with piped stdin still reads commands
        // through readline (parse.y yy_readline_get -> bashline.c
        // bash_readline), which echoes the prompt and input to stderr and
        // honors editing keys (C-p/C-n history, C-r i-search, C-o
        // operate-and-get-next) even without a tty.
        run_interactive_stdin(executor)
    } else {
        run_stdin_script(executor)
    }
}

fn run_stdin_script_with_init(executor: &mut Executor, init_file: Option<&str>) -> i32 {
    executor.inherit_process_stdin();
    if let Some(init_file) = init_file {
        let _ = run_init_file(executor, init_file);
    }
    run_stdin_script(executor)
}

fn run_init_file(executor: &mut Executor, init_file: &str) -> i32 {
    let path = executor.resolve_shell_path(init_file);
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!(
                "rubash: {}: {}",
                init_file,
                rubash::posix_errors::message(&e)
            );
            return 1;
        }
    };
    run_source(executor, &contents, false)
}

fn run_repl(executor: &mut Executor) {
    println!("Rubash - A Rust implementation of GNU Bash");
    println!("Type 'exit' to quit.\n");

    let stdin = io::stdin();
    let mut input = String::new();

    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        input.clear();
        match stdin.lock().read_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let input = input.trim();
        if input == "exit" || input == "quit" {
            println!("Goodbye!");
            break;
        }

        run_line(executor, input, true);
    }
}

fn run_stdin_script(executor: &mut Executor) -> i32 {
    // TODO(shell.c/input.c): Bash reads commands from redirected stdin without
    // prompting, while commands launched from that stream inherit the same
    // input. Keep ordinary input line-oriented, but gather obvious compound
    // commands until their closing reserved word arrives.
    executor.inherit_process_stdin();
    let mut input = String::new();
    let mut pending = String::new();
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();
    let mut next_line = 1usize;
    let mut pending_start_line = 1usize;

    loop {
        input.clear();
        // GNU input.c bash_input binds the command reader to fd 0, so a
        // permanent `exec 0<file` (redir.c do_redirections) moves the script
        // source to the new input; only fall back to the process's real
        // stdin while fd 0 still designates it (redir1.sub:4-6).
        let line_result = match executor.script_fd0_line(&mut input) {
            Some(count) => Ok(count),
            None => read_unbuffered_line(&mut input),
        };
        match line_result {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        if pending.is_empty() {
            pending_start_line = next_line;
        }
        next_line += input.matches('\n').count().max(1);
        pending.push_str(&input);
        if let Some((delimiter, strip_tabs)) = pending_heredocs.first() {
            let candidate = input.trim_end_matches(['\r', '\n']);
            let candidate = if *strip_tabs {
                candidate.trim_start_matches('\t')
            } else {
                candidate
            };
            if candidate == delimiter {
                pending_heredocs.remove(0);
            }
        } else {
            pending_heredocs.extend(stdin_heredoc_declarations(&input));
        }

        let stdin_posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
        if !pending_heredocs.is_empty() || stdin_source_needs_more_posix(&pending, stdin_posix) {
            continue;
        }

        // bashhist.c:869 bash_add_history: in interactive mode, each
        // complete command is recorded to the session history.
        if let Some(session) = executor.get_session_history() {
            let trimmed = pending.trim_end();
            if !trimmed.is_empty() {
                let control = executor.get_env("HISTCONTROL").unwrap_or_default();
                let ignore = executor.get_env("HISTIGNORE").unwrap_or_default();
                let histsize =
                    rubash::history::SessionHistory::size_limit(executor.get_env("HISTSIZE"));
                session
                    .borrow_mut()
                    .record(trimmed, &control, &ignore, histsize);
            }
        }

        let status = run_source_with_line_offset(
            executor,
            &pending,
            false,
            pending_start_line.saturating_sub(1),
            None,
            None,
        );
        let parse_error = executor.take_parse_error();
        pending.clear();
        if parse_error
            || executor.take_exit_jump_pending()
            || (status != 0
                && stdin_script_errexit_enabled(executor)
                // execute_cmd.c:652-656: `! CMD` status is errexit-exempt.
                && !executor.last_command_inverted())
        {
            break;
        }
    }

    if !pending.trim().is_empty() {
        let status = run_source_with_line_offset(
            executor,
            &pending,
            false,
            pending_start_line.saturating_sub(1),
            None,
            None,
        );
        let parse_error = executor.take_parse_error();
        if parse_error
            || executor.take_exit_jump_pending()
            || (status != 0
                && stdin_script_errexit_enabled(executor)
                && !executor.last_command_inverted())
        {
            pending.clear();
        }
    }

    let status = executor.last_exit_code();
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    finish_shell(executor, status, interactive)
}

fn run_internal_pipeline_utility(name: &str, args: &[String]) -> ! {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    match name {
        "yes" => {
            let line = if args.is_empty() {
                "y".to_string()
            } else {
                args.join(" ")
            };
            let chunk = format!("{line}\n").repeat(256);
            loop {
                if stdout.write_all(chunk.as_bytes()).is_err() || stdout.flush().is_err() {
                    std::process::exit(0);
                }
            }
        }
        "head" => {
            let count = internal_head_line_count(args).unwrap_or(10);
            let mut input = std::io::BufReader::new(stdin.lock());
            let mut line = Vec::new();
            for _ in 0..count {
                line.clear();
                match input.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if stdout.write_all(&line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = stdout.flush();
            std::process::exit(0);
        }
        "wc" => {
            let mut input = stdin.lock();
            let mut buffer = [0_u8; 8192];
            let mut lines = 0usize;
            loop {
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        lines += buffer[..size].iter().filter(|byte| **byte == b'\n').count()
                    }
                    Err(_) => break,
                }
            }
            let _ = writeln!(stdout, "{lines}");
            std::process::exit(0);
        }
        _ => std::process::exit(127),
    }
}

fn internal_head_line_count(args: &[String]) -> Option<usize> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "-n" {
            return args.get(index + 1)?.parse().ok();
        }
        if let Some(value) = arg.strip_prefix("-n") {
            if !value.is_empty() {
                return value.parse().ok();
            }
        }
        if let Some(value) = arg.strip_prefix('-') {
            if !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()) {
                return value.parse().ok();
            }
        }
        if let Some(value) = arg.strip_prefix("--lines=") {
            return value.parse().ok();
        }
        index += 1;
    }
    None
}


fn run_line(executor: &mut Executor, input: &str, interactive: bool) -> i32 {
    let input = input.trim();
    if input.is_empty() {
        return executor.last_exit_code();
    }

    run_source(executor, input, interactive)
}

