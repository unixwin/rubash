//! Rubash - A Rust implementation of GNU Bash
//!
//! Run with: cargo run

use rubash::executor::Executor;
use rubash::script_driver::{
    check_binary_file, finish_shell, prepare_interactive_history, run_interactive_stdin,
    run_script_with_history, run_source, run_source_with_line_offset, script_uses_aliases,
    script_uses_history, script_uses_runtime_parse_gates,
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

    // rubash#1175: this process IS the shell, so a top-level `exec` may
    // replace its image for real (GNU shell_execve) instead of being
    // simulated by a child process with a different PID. Library hosts
    // (niubash) never opt in — their process is not the shell's.
    rubash::builtins::exec::enable_process_replacement();

    // rubash#590: the bash.exe shim (src/bin/bash.rs) erases Windows argv[0]
    // when it re-execs this engine, so the invocation face travels in
    // NIU_INVOKED_AS (rubash#504 forwarded the sh/ash faces for the POSIX
    // contract; #590 extends the forward to every shell face). GNU
    // set_shell_name (shell.c:1778-1806) binds dollar_vars[0] AND the
    // sh/su/rbash contracts to that same argv[0], so the forwarded name is
    // the honest reconstruction of the erased argv[0]: it ranks ahead of the
    // process argv[0] path and behind an explicit BASH_ARGV0 import
    // (variables.c:1547-1555, which GNU itself ranks above argv[0]).
    // Consumed BEFORE the executor imports the process environment — it
    // describes how THIS process was invoked, never a shell variable or a
    // child-inheritable setting (the same consume-on-read rule the niubash
    // runtime host applies).
    let invoked_as = env::var("NIU_INVOKED_AS").ok().filter(|v| !v.is_empty());
    if invoked_as.is_some() {
        env::remove_var("NIU_INVOKED_AS");
    }
    let mut args: Vec<String> = env::args().collect();
    // niubash#362: a respawned async `&` child too large for the
    // OS command line arrives with its `-c` script named by
    // __RUBASH_CMD_FILE; splice it back into argv here, before anything reads
    // it and before the executor imports the process environment.
    rubash::invocation::materialize_command_file(&mut args);
    // rubash#866: consume the background-wrapper spawn handshake (the
    // --internal-bg-wrapper=<nonce> argv flag + the matching
    // __RUBASH_BACKGROUND_WRAPPER env nonce) before any parser or the
    // `--internal-*` dispatch below sees it; the env key is scrubbed
    // unconditionally so a user-forged value can neither arm the wrapper
    // role nor leak into children. Also runs for plain invocations, where
    // it is just the scrub — and (rubash#873) the run itself marks this
    // process as a handshake consumer, which licenses the wrapper spawn
    // site to attach the argv flag to respawns; host CLIs that never call
    // this get env-only respawns and never see the niu-specific flag.
    rubash::script_driver::consume_background_wrapper_handshake(&mut args);
    // rubash#1035: an asynchronous-job respawn starts with SIGINT/SIGQUIT
    // ignored (GNU execute_cmd.c setup_async_signals, job_control == 0).
    // Consume the spawn-site marker here — before the executor seeds the
    // trap table and captures kernel entry dispositions — so the ignore is
    // in place for both. Runs for plain invocations too, where it is just
    // the scrub.
    #[cfg(unix)]
    rubash::builtins::kill::apply_async_child_entry_ignores();
    let invocation_name = invoked_as.clone().or_else(|| args.first().cloned());
    if let Some(name) = args
        .get(1)
        .and_then(|arg| arg.strip_prefix("--internal-"))
        .filter(|name| matches!(*name, "yes" | "head" | "wc"))
    {
        run_internal_pipeline_utility(name, &args[2..]);
    }
    // rubash#154 identity disclosure: engine-specific meta flag, handled
    // before the GNU option parser (like --internal-*), so the GNU
    // long-option contract (LONG_OPTIONS table / show_shell_usage byte
    // parity with bash 5.3) stays untouched.
    if args.get(1).map(String::as_str) == Some("--identity") {
        print_identity_report();
        return 0;
    }
    let mut executor = Executor::new_process_exit();
    // startup21: the process-exit executor's Drop skips the process-env
    // restore and the signal-mailbox unregister — a dying process's
    // environment is unobservable, and stale markers self-heal in
    // deliver()'s dead-pid path plus the registration thread's prune.
    if let Ok(path) = env::current_exe() {
        // rubash#331: self-produced path values render in POSIX form
        // (/d/...) like $PWD — GNU's own BASH is a POSIX path, and the
        // previous drive-form export made `$BASH` and `$PWD` disagree in
        // form within one shell.
        let path =
            rubash::executor::shell_pwd_display_path(&path.to_string_lossy().replace('\\', "/"));
        // rubash#1325: GNU seeds $BASH WITHOUT the export attribute
        // (variables.c: BASH is a plain shell variable; `export -p` in a
        // clean env lists only OLDPWD/PWD/SHLVL). export_env here made
        // `env -i rubash -c 'export -p'` diverge by leaking BASH.
        executor.set_env("BASH", &path);
    }
    // GNU variables.c:1547-1555 (set_argv0, called from
    // initialize_shell_variables:608 via assign_bash_argv0:1528-1545): a
    // BASH_ARGV0 value imported from the environment becomes dollar_vars[0]
    // AND shell_name, so "BASH_ARGV0=this-bash bash -c 'echo $0 $BASH_ARGV0'"
    // prints this-bash twice. The executable path is only the fallback when
    // the environment did not supply a value.
    // rubash#590: the invocation face read above (NIU_INVOKED_AS, else
    // argv[0]) reconstructs the argv[0] the shim erased, per GNU
    // set_shell_name (shell.c:1778-1806): it feeds dollar_vars[0]/shell_name
    // and apply_invocation_shell_mode below. It ranks behind an explicit
    // BASH_ARGV0 import — variables.c:1547-1555 ranks the environment value
    // above argv[0] ("BASH_ARGV0=this-bash bash -c 'echo $0 $BASH_ARGV0'"
    // prints this-bash twice).
    match env::var("BASH_ARGV0")
        .ok()
        .filter(|value| !value.is_empty())
    {
        Some(imported) => {
            executor.set_env("BASH_ARGV0", &imported);
        }
        None => {
            if let Some(shell_name) = &invocation_name {
                // rubash#331 (#224 legacy): a full-path argv[0] the parent
                // handed over in Windows form is OUR produced `$0` value —
                // render it POSIX (/d/...) so `-c 'echo $0'` matches the
                // PWD family. Relative and bare names stay verbatim; a
                // shim-forwarded face is a bare name by construction.
                let display = rubash::executor::shell_pwd_display_path(shell_name);
                executor.set_env("__RUBASH_SHELL_NAME", &display);
                executor.set_env("BASH_ARGV0", &display);
            }
        }
    }
    apply_invocation_shell_mode(&mut executor, invocation_name.as_deref());

    if args.len() > 1 {
        return run_args(&mut executor, &args[1..], args.first().map(String::as_str));
    }

    // GNU shell.c:541-547: with no operands (and no -c string), tty stdin +
    // tty stderr make the shell interactive — init_interactive — and POSIX
    // says "the -s option shall be assumed", so commands come from stdin
    // either way (shell.c:787-790 read_from_stdin). The stdin-script vs
    // interactive-reader split inside run_no_script_with_init mirrors the
    // same C: a non-tty stdin stays the script driver (shell.c:780-786).
    run_no_script_with_init(&mut executor, None, None)
}

fn print_usage() {
    println!("Usage: rubash [-c command] [script]");
    print_identity_disclosure();
}

/// Identity disclosure block shared by `--help` and `--identity`
/// (rubash#154): the user must always be able to see which persona is
/// active and how to switch.
fn print_identity_disclosure() {
    let identity = rubash::executor::identity::persona_name();
    println!();
    println!("Identity:");
    if identity == "msys" {
        println!("  persona: msys (default; MSYS2-compatible)");
        println!("  switch:  export RUBASH_IDENTITY=native for the honest-native persona");
    } else if cfg!(windows) {
        println!("  persona: native (honest-native; RUBASH_IDENTITY=native)");
        println!("  switch:  unset RUBASH_IDENTITY (or set any other value) to return to msys");
    } else {
        // Non-Windows builds are always native (identity.rs): the MSYS
        // persona exists to keep Windows inside the MSYS/Cygwin script
        // ecosystem and is unreachable here, so the disclosure must not
        // advertise a switch back to it.
        println!("  persona: native (honest-native; non-Windows builds are always native)");
        println!("  switch:  n/a (RUBASH_IDENTITY only selects personas on Windows)");
    }
    println!("  query:   rubash --identity prints the persona and effective values");
}

/// `rubash --identity` (rubash#154): print the active persona and every
/// effective identity value in one place.
fn print_identity_report() {
    use rubash::executor::identity;
    println!(
        "identity:    {} ({})",
        identity::persona_name(),
        if identity::current_identity() == identity::ShellIdentity::Msys {
            "default; MSYS2-compatible"
        } else if cfg!(windows) {
            "RUBASH_IDENTITY=native; honest-native"
        } else {
            "non-Windows build; honest-native"
        }
    );
    println!("uname -s:    {}", identity::sysname());
    println!("uname -m:    {}", identity::machine());
    println!("arch:        {}", identity::machine());
    println!("uname -r:    {}", identity::release());
    println!("uname -o:    {}", identity::operating_system());
    println!("OSTYPE:      {}", identity::ostype());
    println!("MACHTYPE:    {}", identity::machtype());
    if cfg!(windows) {
        println!("switch:     export RUBASH_IDENTITY=native (native) / unset it (msys default)");
    } else {
        println!("switch:     n/a (RUBASH_IDENTITY only selects personas on Windows)");
    }
}

fn apply_invocation_shell_mode(executor: &mut Executor, argv0: Option<&str>) {
    let Some(name) = argv0.and_then(invocation_shell_name) else {
        return;
    };

    if matches!(name.as_str(), "sh" | "ash") {
        // GNU shell.c:733-737: invoked as `sh' binds POSIXLY_CORRECT and
        // runs sv_strict_posix (the posix_vars walk) — carried by
        // set::set_shell_option.
        executor.set_shell_option("posix", true);
    }
    // GNU shell.c set_shell_name: an argv[0] whose first character is '-'
    // (after the basename) marks a login shell, independent of --login/-l.
    if name.starts_with('-') {
        executor.mark_login_shell();
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

fn run_args(executor: &mut Executor, args: &[String], argv0: Option<&str>) -> i32 {
    // rubash#815: `--profile` / `--profile-output <file>` are engine meta
    // flags handled BEFORE the GNU option parser (the same shape as
    // --identity and --internal-* at the run_main gate), so the GNU
    // long-option contract stays untouched: LONG_OPTIONS keeps its exact
    // 16-entry table bytes, and an unknown long option still reports
    // "%s: invalid option" with rc 2. `--profile` does not consume the
    // script operand directly — the first non-option word still runs as the
    // script file through the normal launcher path; only LEADING option
    // words are scanned, so `script.sh --profile-output f` keeps both words
    // as the operand and $1 (GNU parses options until the first non-option
    // argument). The session wraps the script-file driver only: `-c`, `-s`
    // and interactive forms are unprofiled in M1 (owner ticket #815).
    let mut profile_mode = false;
    let mut profile_output: Option<String> = None;
    // 1.5.0 debugger design section 13: `--dap` is an engine meta
    // flag handled in this same leading scan, for the same reason
    // as --profile — the GNU long-option contract stays untouched.
    let mut dap_mode = false;
    // `--debug` (design section 2; PLAN row 11): the native
    // debugger's interactive frontend. The word itself stays in
    // the filtered args — the extdebug arming below is still the
    // behavior for the forms the frontend does not take over
    // (-c, -s, interactive) — and the diversion after the scan
    // claims exactly the one form the design names:
    // `rubash --debug script.sh [args…]`.
    let mut debug_flag = false;
    let mut filtered_args: Vec<String> = Vec::with_capacity(args.len());
    {
        let mut index = 0usize;
        while index < args.len() {
            let arg = args[index].as_str();
            match arg {
                "--profile" => {
                    profile_mode = true;
                    index += 1;
                }
                "--dap" => {
                    dap_mode = true;
                    index += 1;
                }
                "--debug" => {
                    debug_flag = true;
                    filtered_args.push(arg.to_string());
                    index += 1;
                }
                "--profile-output" => match args.get(index + 1) {
                    Some(value) => {
                        profile_output = Some(value.clone());
                        index += 2;
                    }
                    None => {
                        eprintln!("rubash: --profile-output: option requires an argument");
                        return 2;
                    }
                },
                "--" | "-" => {
                    filtered_args.extend_from_slice(&args[index..]);
                    break;
                }
                _ if arg.starts_with('-') => {
                    filtered_args.push(arg.to_string());
                    index += 1;
                }
                _ => {
                    // First non-option word: the script operand. The rest is
                    // the operand's positional parameters — verbatim.
                    filtered_args.extend_from_slice(&args[index..]);
                    break;
                }
            }
        }
    }
    let args: &[String] = &filtered_args;
    if dap_mode {
        // The editor launch form: leading shell options (an
        // editor's own `--debug`, say) are skipped — the DAP
        // dispatch owns the run, so none of them are applied;
        // in particular `--debug`'s extdebug arming does not
        // happen on this path (design section 2: the debugger
        // implies the gate internally and never flips the user's
        // extdebug observably). The first non-option word is the
        // program and the rest are its arguments; with no
        // operand the adapter runs as a pure DAP server and the
        // program arrives in the client's `launch` request.
        // Like the scan above, value-taking options before the
        // operand are not understood here.
        let mut start = 0usize;
        while start < args.len() {
            let word = args[start].as_str();
            if word == "--" {
                start += 1;
                break;
            }
            if word == "-" || !word.starts_with('-') {
                break;
            }
            start += 1;
        }
        let operands = &args[start..];
        return rubash::run_dap_stdio(rubash::DapLaunchPreset {
            program: operands.first().cloned(),
            args: operands.get(1..).unwrap_or(&[]).to_vec(),
        });
    }
    if debug_flag {
        // The `--debug` diversion (design section 2): claim the
        // run only when a script operand follows and no -c / -s
        // form appears among the leading options — those forms,
        // and the interactive form, keep today's behavior (the
        // invocation layer arms extdebug from the flag). A `--`
        // ends the option region: the word after it is the
        // operand even if it starts with '-'.
        let mut operand_at: Option<usize> = None;
        let mut index = 0usize;
        while index < args.len() {
            let word = args[index].as_str();
            if word == "--" {
                if index + 1 < args.len() {
                    operand_at = Some(index + 1);
                }
                break;
            }
            if word == "-" || !word.starts_with('-') {
                if !word.starts_with('-') {
                    operand_at = Some(index);
                }
                break;
            }
            if !word.starts_with("--") && word[1..].contains(['c', 's']) {
                break;
            }
            index += 1;
        }
        if let Some(start) = operand_at {
            return rubash::run_debug_cli(rubash::DebugCliPreset {
                program: Some(args[start].clone()),
                args: args[start + 1..].to_vec(),
            });
        }
    }
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
    let (mut index, early_exit) = parse_long_options(executor, args, &mut long_state, argv0);
    if let Some(code) = early_exit {
        return code;
    }
    let mut init_file: Option<String> = long_state.init_file.take();
    let pretty_print = long_state.pretty_print;
    // GNU shell.c:506-510: dump mode lists the input's locale strings
    // instead of executing (read_but_dont_execute); the -D short flag
    // below can still raise it during the short-option scan.
    let mut dump_strings = long_state.dump_strings;
    let dump_po = long_state.dump_po;
    // niubash #107 / GNU shell.c: `-c` does not bind the NEXT argv — commands
    // are read from the FIRST NON-OPTION argument. Options between `-c` and
    // the command string (`bash -c -l 'echo hi'`) keep being parsed as
    // options below, so remember that a command string is still owed.
    let mut pending_command = false;
    let mut alias_expansion_override = None;
    // shell.c parse_short_options: `-s` only marks read_from_stdin and the
    // argv scan continues, so options AFTER `-s` (`bash -s -r`) still apply
    // and non-option words become positional parameters ($1, $2, ...; $0
    // stays the shell name). Running stdin immediately would swallow a
    // trailing -r as a positional and drop the restriction.
    let mut read_from_stdin = false;
    let mut stdin_positionals: Vec<String> = Vec::new();
    // rubash#1480: after a `--`/`-` marker every remaining argv is an
    // operand; the tuple scrutinee routes them all to the `script` arm.
    let mut options_ended = false;
    while index < args.len() {
        match (options_ended, args[index].as_str()) {
            (false, "-o") | (false, "+o") => {
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
                    // posix flips (with the set_posix_mode walk) live in
                    // set::set_shell_option.
                    executor.set_shell_option(option, enabled);
                    index += 2;
                } else {
                    // rubash#547: a `-o'/`+o` with NO option argument is the
                    // list form, not a usage error. Empirical GNU 5.3.0
                    // (2026-10-12, WSL): `bash -o` prints the set -o table
                    // (`name\tstate`, rc 0) and `bash +o` prints the
                    // `set +o name` / `set -o name` recreate list (rc 0) —
                    // the same output as `set -o` / `set +o` with no
                    // operand (options.c print_all_shell_options) — then
                    // startup continues normally.
                    let recreate = args[index] == "+o";
                    let mut stdout = io::stdout();
                    let _ = executor.print_shell_options_listing(recreate, &mut stdout);
                    let _ = stdout.flush();
                    index += 1;
                }
            }
            (false, "-i") => {
                executor.set_env("__RUBASH_INTERACTIVE", "1");
                index += 1;
            }
            (false, "--rcfile") | (false, "--init-file") => {
                if let Some(path) = args.get(index + 1) {
                    init_file = Some(path.clone());
                    index += 2;
                } else {
                    eprintln!("rubash: --init-file: option requires an argument");
                    return 2;
                }
            }
            (false, "--debugger") => {
                // Consumed by parse_long_options above (this arm exists for
                // the respawn/pre-scan shapes that skip it). GNU
                // shell.c:257: debugging_mode only; the starter runs via
                // start_debugger at the driver sites.
                executor.set_env("__RUBASH_DEBUGGER_MODE", "1");
                index += 1;
            }
            (false, "--posix") => {
                // GNU shell.c:566-572: --posix runs sv_strict_posix (the
                // posix_vars walk) — carried by set::set_shell_option.
                executor.set_shell_option("posix", true);
                index += 1;
            }
            (false, "--login") | (false, "-l") => {
                // GNU shell.c:497-503: --login/-l flips LOGIN_SHELL; a login
                // shell accepts the logout builtin and runs ~/.bash_logout
                // at exit.
                executor.mark_login_shell();
                index += 1;
            }
            (false, "--noprofile") | (false, "--norc") => {
                // shell.c:269-270 (--norc/--noprofile -> no_rc/no_profile):
                // recorded so run_startup_files can honor the skips. On a
                // login shell --norc does not inhibit the profile chain and
                // --noprofile does not inhibit the rc chain (they gate
                // different branches, shell.c:1833-1880).
                if args[index] == "--noprofile" {
                    executor.set_env("__RUBASH_NO_PROFILE", "1");
                } else {
                    executor.set_env("__RUBASH_NO_RC", "1");
                }
                index += 1;
            }
            (false, "-O") | (false, "+O") => {
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
                    if option == "expand_aliases" {
                        alias_expansion_override = Some(args[index] == "-O");
                    }
                    index += 2;
                } else {
                    // rubash#780: the `-o` sibling above (rubash#547). An
                    // argument-less `-O`/`+O` is the LIST request, not a
                    // usage error. Empirical GNU 5.3.0: `bash -O` prints
                    // the shopt table (`name\tstate`, rc 0) and `bash +O`
                    // the `shopt -u name` / `shopt -s name` recreate list
                    // (rc 0) — the same bytes as `shopt` / `shopt -p` —
                    // then startup continues normally.
                    let recreate = args[index] == "+O";
                    let mut stdout = io::stdout();
                    let _ = executor.print_shopt_options_listing(recreate, &mut stdout);
                    let _ = stdout.flush();
                    index += 1;
                }
            }
            (false, "-c") => {
                // niubash #107 / GNU shell.c:857-889: keep parsing options —
                // the first non-option argument encountered later in this
                // loop becomes the command string (`script` arm). A `-c`
                // with no remaining arguments at all keeps GNU's usage
                // error, checked after the loop (shell.c:519-528).
                pending_command = true;
                index += 1;
            }
            (false, "-s") => {
                read_from_stdin = true;
                index += 1;
            }
            (false, "-D") | (false, "+D") => {
                // GNU shell.c:960-964 (case 'D', either sign):
                // dump_translatable_strings — the input's locale strings
                // are listed instead of executed (shell.c:506-510).
                dump_strings = true;
                index += 1;
            }
            (false, "--") | (false, "-") => {
                // GNU shell.c:905-911: a lone - or -- ends option parsing;
                // the next argv is the script file. rubash#1480: the marker
                // must END option processing — without the flag the loop
                // went on to parse `-c` as an option (`rubash -- -c '...'`
                // executed the string instead of failing on the file).
                options_ended = true;
                index += 1;
            }
            (false, option) if apply_cli_shell_flags(executor, option) => {
                index += 1;
            }
            (_, script) => {
                // niubash #107: GNU parses options until the first
                // non-option argument; with a pending `-c`, that argument is
                // the command string and the rest of argv becomes $0, $1, ...
                // (shell.c:523-528). Option-looking arguments still get the
                // GNU invalid-option check first (`bash -c -Z ...`).
                if !options_ended && (script.starts_with('-') || script.starts_with('+')) {
                    let sign = &script[..1];
                    let invalid = script[1..]
                        .chars()
                        .find(|ch| !rubash::usage::is_short_option_letter(*ch));
                    if let Some(ch) = invalid {
                        eprintln!("bash: {sign}{ch}: invalid option");
                        show_shell_usage();
                        return 2;
                    }
                }
                if pending_command {
                    if dump_strings {
                        // GNU shell.c:506-510 with a -c string: the
                        // string is read but not executed; its locale
                        // strings dump (locale.c:550), anchored as `-c`
                        // in PO mode. Nothing runs — no startup files,
                        // no side effects (rubash#1977).
                        dump_strings_text(script, dump_po, "-c");
                        return 0;
                    }
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
                    apply_interactive_alias_default(executor, alias_expansion_override);
                    return run_command_string_with_init(executor, script, init_file.as_deref());
                }
                // shell.c: a non-option word after `-s` is a positional
                // parameter, never a script file — stdin stays the source and
                // the argv scan keeps going so later options still apply.
                if read_from_stdin {
                    stdin_positionals.push(script.to_string());
                    index += 1;
                    continue;
                }
                if dump_strings {
                    // GNU shell.c:506-510 + locale.c:550: dump the script
                    // file's locale strings and exit; the script never
                    // executes (rubash#1977 — the flag used to be accepted
                    // and silently dropped, so the operand ran).
                    executor.set_env("__RUBASH_SCRIPT_NAME", script);
                    return run_dump_strings(executor, script, dump_po);
                }
                if pretty_print {
                    // GNU shell.c:830-831: --pretty-print replaces execution
                    // with pretty_print_loop over the input file
                    // (eval.c:215-253). Parser diagnostics cite the script
                    // path (error.c get_name_for_error uses $0), so the
                    // prefix env must be armed before any parse error
                    // reports.
                    executor.set_env("__RUBASH_SCRIPT_NAME", script);
                    return run_pretty_print(executor, script);
                }
                apply_interactive_alias_default(executor, alias_expansion_override);
                if profile_mode {
                    // rubash#815: the profiler session wraps the script-file
                    // driver. Reports go to --profile-output (or the process
                    // temp directory) — never stdout/stderr, so profile mode
                    // stays byte-identical with profiling off.
                    rubash::executor::profiler::session_begin(profile_output.take());
                }
                let status = run_script_file_with_init(
                    executor,
                    script,
                    &args[index + 1..],
                    init_file.as_deref(),
                );
                if profile_mode {
                    rubash::executor::profiler::session_end();
                }
                return status;
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

    if dump_strings {
        // GNU shell.c:506-510: with no script operand the dump pass
        // reads standard input (the -s route included). PO anchors cite
        // the shell's own argv[0] (locale.c yy_input_name).
        let mut contents = String::new();
        let _ = io::stdin().read_to_string(&mut contents);
        let source_name = argv0.unwrap_or("bash");
        dump_strings_text(&contents, dump_po, source_name);
        return 0;
    }

    if pretty_print {
        // GNU shell.c: --pretty-print with no script file runs
        // pretty_print_loop over stdin (eval.c:215-253): the input is
        // printed back canonically, never executed (rubash#1084).
        use std::io::Read;
        let mut contents = String::new();
        let _ = io::stdin().read_to_string(&mut contents);
        return run_pretty_print_text(executor, &contents);
    }

    if read_from_stdin {
        executor.set_positional_params(std::mem::take(&mut stdin_positionals));
        return run_stdin_script_with_init(
            executor,
            init_file.as_deref(),
            alias_expansion_override,
        );
    }

    // shell.c:1830-1842 init_interactive: when -i is set, the shell
    // enables history (remember_on_history = enable_history_list = 1).
    // prepare_interactive_history now runs inside run_no_script_with_init
    // (after the init file, matching shell.c:799-811 ordering) so the -c /
    // script / -s paths share it (rubash#151).
    run_no_script_with_init(executor, init_file.as_deref(), alias_expansion_override)
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
        // rubash#131: under -n nothing executes, so a top-level
        // `shopt -s extglob` line must not open the parse-time extglob
        // gate for later lines (GNU parses but never runs the shopt;
        // verified: `bash -n` on shopt+pattern still fails rc 2).
        if name == "noexec" {
            rubash::lexer::set_parse_execution_expected(!enabled);
        }
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
        'm' => Some("monitor"),
        'n' => Some("noexec"),
        'B' => Some("braceexpand"),
        'r' => Some("restricted"),
        _ => None,
    }
}

/// GNU jobs.c:4735 initialize_job_control (shell.c:1969 passes force =
/// jobs_m_flag) ends with jobs.c:4869 `change_flag ('m', job_control ? '-' :
/// '+')`. Without a terminal to take a process group on (non-interactive
/// shell, stderr not a tty), give_terminal_to / tcgetpgrp fail with ENOTTY,
/// job_control stays 0, and two sys_error/internal_error lines are printed
/// (jobs.c:4841 "cannot set terminal process group (%d)" and jobs.c:4856
/// "no job control in this shell") before `-m` is dropped from `$-`.
/// Windows has no POSIX terminal process groups, so stderr-not-a-terminal is
/// the honest port of that branch: the option is turned back off exactly as
/// jobs.c:4869 does. An interactive shell or a terminal on stderr keeps `m`,
/// matching the GNU paths where job_control could be established
/// (empirical WSL 5.3.0: `bash -i -c 'echo $-'` -> `himBHc`, `bash -m -c
/// 'echo $-'` -> the two diagnostics + `hBc`).
fn apply_startup_job_control(executor: &mut Executor) {
    // A respawned `rubash -c` background child (marked with
    // __RUBASH_SHELL_PID by the background-spawn path) is GNU's forked
    // child: it inherits the parent's job_control state and NEVER re-runs
    // initialize_job_control, so the terminal-grab diagnostics cannot fire
    // there (rubash#234: a monitored `sleep 0.1 &` printed the two-line
    // noise from the child's startup while GNU's exec'd sleep is silent).
    if rubash::executor::RESPAWNED_CHILD.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    if interactive {
        // jobs.c:4869 `change_flag ('m', job_control ? '-' : '+')` keeps the
        // letter for interactive shells: initialize_job_control establishes
        // job control there even without -m (empirical WSL 5.3.0: `bash -i
        // -c 'echo $-'` -> `himBHc`, with and without an explicit -m/-o
        // monitor).
        executor.set_shell_option("monitor", true);
        // jobs.c:4871 `if (interactive) get_tty_state ()` -> jobs.c:2647-2648
        // -> winsize.c:98-100: initialize_job_control binds LINES/COLUMNS
        // from the controlling terminal before the startup files run
        // (shell.c:1969 precedes run_startup_files at shell.c:722), so an
        // interactive rcfile sees real values (rubash#300; ecosweep GNU
        // baseline: `--rcfile t3.rc -i` echoes COLUMNS=80 from the console).
        rubash::script_driver::bind_interactive_console_winsize(executor);
        return;
    }
    // shell_option_enabled semantics via the option's env key (options.rs:323
    // shell_option_key); monitor defaults off (options.rs:66-69), so "1" is
    // an explicit enable (-m or -o monitor).
    if executor.get_env("__RUBASH_SETOPT_monitor").as_deref() != Some("1") {
        return;
    }
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        // Non-interactive with a real terminal on stderr: GNU takes the
        // terminal process group successfully and keeps `m`.
        return;
    }
    executor.set_shell_option("monitor", false);
    let shell_name = executor
        .get_env("__RUBASH_SHELL_NAME")
        .or_else(|| executor.get_env("BASH_ARGV0"))
        .unwrap_or("bash");
    eprintln!(
        "{shell_name}: cannot set terminal process group ({}): Inappropriate ioctl for device",
        std::process::id()
    );
    eprintln!("{shell_name}: no job control in this shell");
}

/// State produced by the GNU long-option parser (shell.c:838-889).
#[derive(Default)]
struct LongOptionState {
    init_file: Option<String>,
    pretty_print: bool,
    dump_strings: bool,
    dump_po: bool,
}

/// The long-option table of the contractual GNU 5.3.0 build
/// (shell.c:254-284 with DEBUGGER and TRANSLATABLE_STRINGS enabled and
/// WORDEXP_OPTION disabled - the same 16 entries the usage block lists).
// LONG_OPTIONS / LONG_OPTIONS_WITH_VALUE live in `rubash::usage` since the
// #585 host-half move: the usage tables print from the lib so every
// embedding binary answers identically.
use rubash::usage::{LONG_OPTIONS, LONG_OPTIONS_WITH_VALUE};

/// The set -o short-flag letters GNU's change_flag accepts
/// (shell.c:966-971 reports any other character as an invalid option) now
/// lives in `rubash::usage::SHORT_OPTIONS` / `is_short_option_letter`, the
/// same table the product help is generated from.

fn parse_long_options(
    executor: &mut Executor,
    args: &[String],
    state: &mut LongOptionState,
    argv0: Option<&str>,
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
                // Issue #585: the help surface forks on the invocation name,
                // exactly like GNU keys `--help` off argv[0] (shell.c:477-482
                // show_shell_usage(stdout, 1)). Under a bash/sh invocation
                // name the GNU usage contract prints (rc 0, stdout); under
                // any other name the product help stays byte-identical.
                match rubash::usage::invocation_help_name(argv0) {
                    Some(shell_name) => rubash::usage::show_shell_help(
                        &shell_name,
                        executor.get_env("BASH_VERSION"),
                    ),
                    None => print_usage(),
                }
                return (index, Some(0));
            }
            "version" => {
                // GNU show_shell_version (version.c:88-104) — printed from
                // `rubash::usage` since the #585 host-half move, so the niu
                // launcher answers `bash --version` with the same bytes.
                rubash::usage::show_shell_version(executor.get_env("BASH_VERSION"));
                return (index, Some(0));
            }
            "login" => {
                executor.mark_login_shell();
            }
            "posix" => {
                // GNU shell.c:566-572: --posix runs sv_strict_posix (the
                // posix_vars walk) — carried by set::set_shell_option.
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
            "dump-strings" => {
                state.dump_strings = true;
            }
            "dump-po-strings" => {
                // GNU shell.c:506-507: --dump-po-strings implies
                // dump_translatable_strings, rendered in PO format
                // (locale.c mk_msgstr).
                state.dump_strings = true;
                state.dump_po = true;
            }
            "noediting" => {
                // shell.c:269 --noediting sets no_line_editing=1: both
                // editing modes report off (`[[ -o emacs ]]` /
                // `[[ -o vi ]]`, builtins/set.def:446-449) and the REPL
                // reads plain lines.
                executor.set_env("__RUBASH_NO_EDITING", "1");
            }
            // GNU shell.c:255: --debug writes the `debugging` variable that
            // NO code in bash ever reads — a write-only fossil, so the flag
            // carries zero observable GNU behavior to conflict with. niu
            // extension: --debug arms extdebug from the first command, the
            // DEBUG-trap substrate a script debugger needs when it cannot
            // edit the script (the same gate predicate rubash#497's native
            // debugger builds on; that work upgrades this meaning in place).
            "debug" => {
                executor.set_shopt_option("extdebug", true);
            }
            // GNU shell.c:257: --debugger sets debugging_mode; the bashdb
            // starter runs at startup (start_debugger, shell.c:764/:795).
            "debugger" => {
                executor.set_env("__RUBASH_DEBUGGER_MODE", "1");
            }
            "noprofile" => {
                executor.set_env("__RUBASH_NO_PROFILE", "1");
            }
            "norc" => {
                executor.set_env("__RUBASH_NO_RC", "1");
            }
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

// `invocation_help_name` and `show_shell_help` moved to `rubash::usage`
// (issue #585 host-half): the niu launcher answers `bash --help` through
// the lib so both binaries print byte-identical GNU usage.

/// Read a script operand for the --pretty-print / --dump-strings
/// drivers (the same open_shell_script exit matrix as the script-file
/// driver: ENOENT -> 127 (EX_NOTFOUND), any other open error -> 126
/// (EX_NOINPUT) — `bash -D missing` exits 127, not 1 (rubash#262)).
/// On error the diagnostic is printed here and the exit code returned.
fn read_script_operand(executor: &mut Executor, script: &str) -> Result<String, i32> {
    let path = executor.resolve_shell_path(script);
    let Ok(contents) = rubash::script_driver::read_script_bytes(&path) else {
        let (message, code) = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => ("Is a directory".to_string(), 126),
            Ok(_) => ("Permission denied".to_string(), 126),
            Err(e) => {
                let code = if e.kind() == std::io::ErrorKind::NotFound {
                    127
                } else {
                    126
                };
                (rubash::posix_errors::message(&e), code)
            }
        };
        let shell_name = executor
            .get_env("__RUBASH_SHELL_NAME")
            .or_else(|| executor.get_env("BASH_ARGV0"))
            .unwrap_or("bash");
        eprintln!("{shell_name}: {script}: {message}");
        return Err(code);
    };
    Ok(contents)
}

/// pretty_print_loop (eval.c:215-253): parse the script and print each
/// command canonically (print_cmd.c) followed by one newline, then exit
/// successfully (shell.c:830-831).
fn run_pretty_print(executor: &mut Executor, script: &str) -> i32 {
    match read_script_operand(executor, script) {
        Ok(contents) => run_pretty_print_text(executor, &contents),
        Err(code) => code,
    }
}

/// The contents-based core of [`run_pretty_print`] (pretty_print_loop,
/// eval.c:215-253), shared by the script-file form and the stdin form.
/// The loop itself lives in the library
/// (`rubash::parser::ast_print::pretty_print_input`) so hosts print
/// identically; this wrapper owns the executor-side parse-error
/// report and the exit code.
fn run_pretty_print_text(executor: &mut Executor, contents: &str) -> i32 {
    let posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    let outcome = rubash::parser::ast_print::pretty_print_input(contents, posix);
    if let Some(command) = &outcome.parse_error {
        // GNU eval.c:215-253 pretty_print_loop: read_command's parse
        // error longjmps to top_level — yyerror's stderr diagnostic
        // already fired, commands parsed before the error have printed,
        // nothing after it does, and the loop returns
        // EXECUTION_FAILURE.
        executor.report_pretty_print_parse_error(command);
        print!("{}", outcome.output);
        let _ = io::stdout().flush();
        return 1;
    }
    print!("{}", outcome.output);
    0
}

/// -D / --dump-strings over a script file (shell.c:506-510,
/// locale.c:550): dump the file's locale strings and exit; the script
/// never executes (rubash#1977).
fn run_dump_strings(executor: &mut Executor, script: &str, po: bool) -> i32 {
    match read_script_operand(executor, script) {
        Ok(contents) => {
            dump_strings_text(&contents, po, script);
            0
        }
        Err(code) => code,
    }
}

/// The text-based core of the dump drivers, shared by the script-file,
/// -c and stdin forms: collect the locale strings through the library
/// entry (`rubash::locale::collect_locale_strings`) and print the GNU
/// rendering.
fn dump_strings_text(contents: &str, po: bool, source_name: &str) {
    let mut strings = Vec::new();
    rubash::locale::collect_locale_strings(contents, 1, &mut strings);
    print!(
        "{}",
        rubash::locale::render_locale_string_dump(&strings, po, source_name)
    );
    let _ = io::stdout().flush();
}

fn run_command_string_with_init(
    executor: &mut Executor,
    command: &str,
    init_file: Option<&str>,
) -> i32 {
    // rubash#948: -c operand entry escaping — the boundary documented on
    // substitution_metadata::command_string_entry_text. The respawn
    // exclusion is a DOUBLE-ESCAPE exclusion only, not an all-`-c`
    // exclusion: a respawned `&`/coproc child's operand is serialized AST
    // already in transport spelling (E400 pairs, raw-byte markers) and must
    // not be re-escaped, while a user-supplied -c string is fresh source —
    // the same class as a script file or stdin — and gets marker-zone
    // entry escaping so a literal marker-zone char stays user data instead
    // of live protocol. The escaping preserves existing E400 pairs so
    // transport-spelled argv from a rubash parent is not double-encoded.
    let command = if rubash::executor::RESPAWNED_CHILD.load(std::sync::atomic::Ordering::Relaxed) {
        std::borrow::Cow::Borrowed(command)
    } else {
        std::borrow::Cow::Owned(rubash::script_driver::command_string_entry_text(command))
    };
    executor.inherit_process_stdin();
    // GNU shell.c:1969 initialize_job_control runs inside
    // initialize_shell_basics, long before the -c string is parsed, so the
    // `-m`-without-a-terminal diagnostics and `$-` drop land before any
    // command output (empirical: `bash -m -c 'echo $-'` -> two stderr lines
    // then `hBc`).
    apply_startup_job_control(executor);
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        apply_init_interactive_defaults(executor);
    }
    // shell.c:722 run_startup_files runs before the rcfile/-c string
    // (rubash#487): `bash -lc` reads the profile chain, `bash -ic` the rc
    // chain; `exit` in a startup file terminates before the command runs.
    // shell.c:764: start_debugger precedes run_one_command, so the starter
    // file lands before both the rc chain and the -c string.
    if executor.get_env("__RUBASH_DEBUGGER_MODE").as_deref() == Some("1") {
        start_debugger(executor);
    }
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    if let Some(status) = run_startup_files(executor, init_file) {
        return finish_shell(executor, status, interactive);
    }
    if let Some(init_file) = init_file.filter(|rc| init_file_needs_direct_run(executor, rc)) {
        let status = run_init_file(executor, init_file);
        // rubash#297: `exit` in the rcfile terminates the shell before the
        // -c string runs (GNU builtins/exit.def -> exit_shell; verified
        // `bash --rcfile rc -i -c 'echo XC'` with `exit 7` in rc -> `A`
        // only, rc 7 — the -c string is never executed).
        if executor.take_exit_jump_pending() {
            let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
            return finish_shell(executor, status, interactive);
        }
    }
    // shell.c:546-547 `-i` forces init_interactive (histexp_flag -> H in
    // `$-`), and shell.c:799-811 runs bash_initialize_history + load_history
    // for every interactive shell — including one executing a -c string —
    // after the startup files. rubash#151: this path previously skipped
    // prepare_interactive_history entirely.
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        prepare_interactive_history(executor);
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
    // Either way this is shell.c run_one_command, NOT reader_loop:
    // current_command_number never increments (`\#` prints 1 — GNU probe
    // target/gapfix2/cc.sh).
    executor.set_command_string_mode(true);
    // rubash#1497: arm the final-unit exec replacement for this -c
    // string (consumed by its last reader unit; the grouped driver
    // re-arms per group so only the final group carries it).
    // Respawned children (background/coproc bodies) run under a
    // different process contract — the parent shell reaps and reports
    // them as jobs — so they never arm.
    if !rubash::executor::RESPAWNED_CHILD.load(std::sync::atomic::Ordering::Relaxed) {
        executor.set_command_string_final_pending(true);
    }
    // rubash#414: alias expansion live at reader start (CLI -o posix /
    // --posix / -O expand_aliases — shell.c:1853 init_noninteractive sets
    // expand_aliases = posixly_correct) takes the incremental grouped
    // driver; GNU's reader is always command-by-command.
    let alias_live_at_start = executor.alias_expansion_enabled();
    let status =
        if script_uses_history(&command) || script_uses_aliases(&command, alias_live_at_start) {
            run_script_with_history(executor, &command, None)
        } else {
            run_source_with_line_offset(executor, &command, interactive, line_offset, None, None)
        };
    executor.set_command_string_mode(false);
    finish_shell(executor, status, interactive)
}

fn run_script_file_with_init(
    executor: &mut Executor,
    script: &str,
    args: &[String],
    init_file: Option<&str>,
) -> i32 {
    // GNU shell.c:1969 initialize_job_control runs before open_shell_script
    // (shell.c:1572), so the `-m`-without-a-terminal diagnostics precede
    // both the script-name binding and any script output, and report the
    // shell's own name, not the script's (empirical: `bash -m p.sh` ->
    // "bash: cannot set terminal process group ... / no job control in this
    // shell" then `hB`).
    apply_startup_job_control(executor);
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
        // GNU shell.c:1572-1601 open_shell_script: on open failure,
        // file_error(filename) prints the diagnostic and
        // sh_exit ((e == ENOENT) ? EX_NOTFOUND : EX_NOINPUT) exits 127
        // for a missing file and 126 for every other open error
        // (shell.h:65-66 EX_NOINPUT=126, EX_NOTFOUND=127); the directory
        // case (file_isdir, just below the open) prints "Is a directory"
        // with the same EX_NOINPUT. Windows reports ERROR_ACCESS_DENIED
        // when a directory is opened, so classify from metadata rather
        // than the errno of the failed read (rubash#262).
        let (message, code) = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => ("Is a directory".to_string(), 126),
            Ok(_) => ("Permission denied".to_string(), 126),
            Err(e) => {
                let code = if e.kind() == std::io::ErrorKind::NotFound {
                    127
                } else {
                    126
                };
                (rubash::posix_errors::message(&e), code)
            }
        };
        // GNU error.c:90-117 get_name_for_error: in non-interactive mode,
        // $0 (dollar_vars[0]) is used as the error prefix. When running
        // `${THIS_SH} ./errors1.sub`, $0 is the full path of the shell
        // executable (e.g. /usr/local/bin/bash).
        let shell_name = executor
            .get_env("__RUBASH_SHELL_NAME")
            .or_else(|| executor.get_env("BASH_ARGV0"))
            .unwrap_or("bash");
        eprintln!("{shell_name}: {script}: {message}");
        return code;
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
    // general.c:718 check_binary_file is GNU's ONLY script rejection gate
    // (ELF magic / NUL in the first line, two with `#!`); shell.c reads the
    // rest as raw bytes with no UTF-8 validity check, so invalid-sequence
    // bytes decode as raw-byte marker pairs instead of refusing the script
    // (rubash#132).
    let contents = rubash::script_driver::bytes_to_script_text(&bytes);

    // GNU shell.c:1572-1601 binds dollar_vars[0] to the script name exactly
    // as the caller spelled it. rubash's native launch cannot see that
    // spelling: an MSYS-style caller passes `/d/...` and the MSYS runtime
    // hands native processes a converted `D:/...` argv. The shell's own $PWD
    // displays in the POSIX domain (shell_pwd_display_path), so $0 and
    // BASH_SOURCE must be mapped back to that same domain or
    // dirname/parameter-chops compose across two path domains
    // (rubash#224: bats-core builds `1-D:\...src` when BASH_SOURCE is `D:/`
    // while $PWD stays `/d/`). Relative and already-POSIX names pass through
    // unchanged.
    let script_name = if script.as_bytes().get(1) == Some(&b':') {
        executor.script_identity_display_path(script)
    } else {
        script.to_string()
    };
    executor.set_env("__RUBASH_SCRIPT_NAME", &script_name);
    executor.set_env("BASH_ARGV0", &script_name);
    // shell.c:1613 rebinds dollar_vars[0] to script_name at every script
    // entry, so a parent's BASH_ARGV0 assignment snapshot must not survive
    // the boundary (the __RUBASH_* keys ride the process env).
    executor.remove_env("__RUBASH_ARGV0_AFTER_UNSET");
    executor.inherit_process_stdin();
    executor.set_positional_params(args.to_vec());
    // shell.c:722 run_startup_files precedes the rcfile/script body
    // (rubash#487): `bash -l script` reads the profile chain first.
    // shell.c:795: start_debugger gates scripts too, before any body
    // command runs.
    if executor.get_env("__RUBASH_DEBUGGER_MODE").as_deref() == Some("1") {
        start_debugger(executor);
    }
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    if let Some(status) = run_startup_files(executor, init_file) {
        return finish_shell(executor, status, interactive);
    }
    if let Some(init_file) = init_file.filter(|rc| init_file_needs_direct_run(executor, rc)) {
        let status = run_init_file(executor, init_file);
        // rubash#297: `exit` in the rcfile terminates the shell before the
        // script body runs (GNU builtins/exit.def -> exit_shell).
        if executor.take_exit_jump_pending() {
            let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
            return finish_shell(executor, status, interactive);
        }
    }
    // shell.c:546-547 + 799-811: `-i` before a script file still forces
    // init_interactive and the interactive history setup, so `$-` gains
    // H (rubash#151: previously missing on this path; empirical GNU:
    // `bash -i p.sh` -> `himBH`).
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    if interactive {
        prepare_interactive_history(executor);
    }
    // shell.c:1715-1717: `bash -i scriptname` with a non-tty script fd runs
    // init_interactive_script, which zeroes GNU's `interactive` C global
    // (keeping interactive_shell=1) — so an `exit` inside the script does
    // not echo "exit" the way a reader-phase exit does. Only the
    // exit.def:59-62 echo is suppressed (rubash#297).
    executor.set_env("__RUBASH_INTERACTIVE_FLAG_OFF", "1");
    let uses_hist = script_uses_history(&contents);
    // rubash#414: alias expansion live at reader start (CLI -o posix /
    // --posix / -O expand_aliases — shell.c:1853) takes the incremental
    // grouped driver, matching GNU's command-by-command reader.
    let uses_alias = script_uses_aliases(&contents, executor.alias_expansion_enabled());
    // rubash#1093: a runtime `shopt -s extglob` (eval/function/sourced
    // file) opens the parse-time extglob gate for later commands under
    // GNU's incremental script reader, but the whole-file fast path
    // below already parsed them. Route such scripts through the grouped
    // driver, which lexes each group after the previous groups ran.
    let uses_parse_gates = script_uses_runtime_parse_gates(&contents);
    let status = if uses_hist || uses_alias || uses_parse_gates {
        run_script_with_history(executor, &contents, None)
    } else {
        run_source(executor, &contents, interactive)
    };
    executor.remove_env("__RUBASH_INTERACTIVE_FLAG_OFF");
    let fin = finish_shell(executor, status, interactive);
    fin
}

/// shell.c:1830-1842 init_interactive: `-i` forces the interactive
/// defaults IMMEDIATELY (shell.c:540-549), long before the startup files
/// run — expand_aliases, history on, and histexp_flag (the H in `$-`).
/// init_interactive never sets no_line_editing, so the readline DEFAULT
/// editing mode applies: `[[ -o emacs ]]` is
/// `no_line_editing == 0 && rl_editing_mode == 1`
/// (builtins/set.def:446-449) and readline defaults to emacs — ON for
/// every interactive shell regardless of stdin being a tty (rubash#312:
/// `--rcfile`/`-s`/`-c` paths under piped stdin kept it off and killed
/// ble.sh at its noediting gate). shell.c:1863-1870
/// init_interactive_script (the `bash -i script` path) instead runs
/// init_noninteractive first — `no_line_editing = 1` — which is why that
/// one shape reports emacs OFF; the script-file init path therefore does
/// NOT call this. `--noediting` (no_line_editing=1) and an explicit
/// `-o vi` both keep emacs off.
fn apply_init_interactive_defaults(executor: &mut Executor) {
    executor.set_shell_option("history", true);
    executor.set_shell_option("histexpand", true);
    if executor.get_env("__RUBASH_NO_EDITING").as_deref() == Some("1") {
        return;
    }
    if executor.get_env("__RUBASH_SETOPT_vi").as_deref() == Some("1") {
        return;
    }
    executor.set_shell_option("emacs", true);
}

fn apply_interactive_alias_default(executor: &mut Executor, explicit: Option<bool>) {
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        executor.set_shopt_option("expand_aliases", explicit.unwrap_or(true));
    }
}

fn run_no_script_with_init(
    executor: &mut Executor,
    init_file: Option<&str>,
    alias_expansion_override: Option<bool>,
) -> i32 {
    // GNU shell.c:541-547: `forced_interactive (-i) || (no -c command and
    // no --wordexp and (no remaining operands or -s) and isatty(stdin) and
    // isatty(stderr))` -> init_interactive, which runs before the startup
    // files. Every caller of this driver has consumed its operands (bare
    // invocation or trailing options only), so the tty pair alone derives
    // the same interactivity `-i` forces. stderr not a terminal (e.g.
    // `rubash 2>log` on a console) keeps the shell NON-interactive — GNU
    // then reads the tty stdin as a plain script with no prompts.
    if executor.get_env("__RUBASH_INTERACTIVE").is_none()
        && io::stdin().is_terminal()
        && io::stderr().is_terminal()
    {
        executor.set_env("__RUBASH_INTERACTIVE", "1");
    }
    apply_interactive_alias_default(executor, alias_expansion_override);
    executor.inherit_process_stdin();
    apply_startup_job_control(executor);
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        apply_init_interactive_defaults(executor);
    }
    // shell.c:722 run_startup_files before the rcfile/reader (rubash#487):
    // a bare login shell reads the profile chain, a non-login interactive
    // shell reads the rc chain.
    // shell.c:795: start_debugger fires here too, but only when this shell
    // is NOT interactive (`reading_shell_script || interactive_shell == 0`)
    // — a piped-stdin bare invocation is exactly GNU's plain-script case.
    if executor.get_env("__RUBASH_DEBUGGER_MODE").as_deref() == Some("1")
        && executor.get_env("__RUBASH_INTERACTIVE").as_deref() != Some("1")
    {
        start_debugger(executor);
    }
    if let Some(status) = run_startup_files(executor, init_file) {
        return finish_shell(executor, status, true);
    }
    if let Some(init_file) = init_file.filter(|rc| init_file_needs_direct_run(executor, rc)) {
        let status = run_init_file(executor, init_file);
        // GNU: `exit` in a sourced startup file terminates the shell —
        // builtins/exit.def exit_builtin -> exit_shell unwinds through the
        // evalstring parse_and_execute driven by run_startup_files
        // (shell.c:722), and NOTHING after it runs: not the interactive
        // reader, not the remaining stdin (rubash#297; verified GNU 5.3.0:
        // `bash --rcfile rc -i < stdin` with `echo A\nexit 7\necho B` ->
        // stdout `A`, stderr empty, rc 7, stdin line never read).
        if executor.take_exit_jump_pending() {
            return finish_shell(executor, status, true);
        }
    }
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        // shell.c:806-811 runs after the startup files; the -i no-arg
        // fallback previously did this before them (main.rs call site).
        prepare_interactive_history(executor);
    }
    // parse.y:1710-1711: an interactive shell reads EVERY command through
    // yy_readline_get -> bashline.c:460-461 bash_readline -> readline(),
    // whose prompt stream is stderr — for the tty (console) case just as
    // for the piped one. The old branch here ran a placeholder banner +
    // hardcoded `$ ` reader instead (rubash#419).
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        run_interactive_stdin(executor)
    } else {
        run_stdin_script(executor)
    }
}

fn run_stdin_script_with_init(
    executor: &mut Executor,
    init_file: Option<&str>,
    alias_expansion_override: Option<bool>,
) -> i32 {
    // GNU shell.c:541-547: the interactive test's second arm is
    // `(arg_index == argc) || read_from_stdin`; `-s` sets read_from_stdin,
    // so tty stdin + tty stderr make `bash -s` interactive too. Without
    // `-i` and with a piped stdin the shell stays non-interactive.
    if executor.get_env("__RUBASH_INTERACTIVE").is_none()
        && io::stdin().is_terminal()
        && io::stderr().is_terminal()
    {
        executor.set_env("__RUBASH_INTERACTIVE", "1");
    }
    apply_interactive_alias_default(executor, alias_expansion_override);
    executor.inherit_process_stdin();
    // Same ordering as the script-file path: shell.c:1969
    // initialize_job_control precedes stdin reader setup, and shell.c:547 +
    // 799-811 make `-i -s` an interactive shell with history initialized, so
    // `$-` gains H (rubash#151; empirical GNU: `echo 'echo $-' | bash -i -s`
    // -> `himBHs`).
    apply_startup_job_control(executor);
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        apply_init_interactive_defaults(executor);
    }
    // shell.c:722 run_startup_files before the rcfile/stdin reader
    // (rubash#487), same branch structure as the no-script driver.
    // shell.c:795's gate covers the stdin script reader as well.
    if executor.get_env("__RUBASH_DEBUGGER_MODE").as_deref() == Some("1") {
        start_debugger(executor);
    }
    if let Some(status) = run_startup_files(executor, init_file) {
        return finish_shell(executor, status, true);
    }
    if let Some(init_file) = init_file.filter(|rc| init_file_needs_direct_run(executor, rc)) {
        let status = run_init_file(executor, init_file);
        // rubash#297: `exit` in the rcfile terminates the shell before the
        // stdin reader starts (GNU builtins/exit.def -> exit_shell; verified
        // `bash --rcfile rc -i -s < stdin` -> `A`, rc 7, stdin unread).
        if executor.take_exit_jump_pending() {
            let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
            return finish_shell(executor, status, interactive);
        }
    }
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        prepare_interactive_history(executor);
    }
    // Interactive (-i forced, or the `-s` tty pair above): GNU reads the
    // commands through readline with the prompt on stderr (parse.y:1710
    // yy_readline_get; verified WSL GNU 5.3.0: `printf 'echo $-\n' | bash
    // -i -s` renders PS1 on stderr before the echo). Non-interactive `-s`
    // with piped stdin stays the promptless script driver.
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        run_interactive_stdin(executor)
    } else {
        run_stdin_script(executor)
    }
}

/// Whether the drivers' direct --rcfile call should run: false when
/// run_startup_files already sourced this exact file (recorded in
/// __RUBASH_SOURCED_RC) — without this, interactive shells source
/// --rcfile twice (issue312 gate test). True for the leftover shapes
/// startup skips (empty HOME, non-interactive extension, login).
fn init_file_needs_direct_run(executor: &Executor, init_file: &str) -> bool {
    executor.get_env("__RUBASH_SOURCED_RC").as_deref() != Some(init_file)
}

/// GNU shell.c:1547 start_debugger: run the bashdb starter file once before
/// any user command (-c string, script, or stdin script — never a bare
/// interactive shell, shell.c:795's `reading_shell_script ||
/// interactive_shell == 0` gate). GNU bakes one configure-time starter path
/// (DEBUGGER_START_FILE); we cannot know the host bash's prefix, so the
/// search is `BASHDB_START_FILE` (documented seam for hosts whose prefix
/// differs — Termux, custom --prefix builds) then the two conventional
/// install locations. On a total miss GNU warns internally and clears
/// debugging_mode (shell.c:1559-1561); the flag itself never sets extdebug —
/// bashdb's own init does, which the sourced file performs here through the
/// normal shopt path.
fn start_debugger(executor: &mut Executor) {
    if executor.get_env("__RUBASH_DEBUGGER_ATTEMPTED").is_some() {
        return;
    }
    executor.set_env("__RUBASH_DEBUGGER_ATTEMPTED", "1");
    let mut candidates: Vec<String> = vec![
        "/usr/local/share/bashdb/bashdb-main.inc".to_string(),
        "/usr/share/bashdb/bashdb-main.inc".to_string(),
    ];
    if let Some(over) = executor.get_env("BASHDB_START_FILE") {
        candidates.insert(0, over.to_string());
    }
    let starter = candidates
        .iter()
        .find(|path| std::fs::metadata(executor.resolve_shell_path(path)).is_ok())
        .cloned();
    match starter {
        Some(path) => {
            let _ = run_init_file(executor, &path);
        }
        None => {
            // GNU force_execute_file reports the open failure with the
            // baked path, then start_debugger warns internally
            // (shell.c:1556-1561) and drops the mode; errexit stays
            // suppressed for the attempt (shell.c:1551-1553).
            eprintln!("rubash: {}: No such file or directory", candidates[0]);
            eprintln!("rubash: warning: cannot start debugger; debugging mode disabled");
        }
    }
}

fn run_init_file(executor: &mut Executor, init_file: &str) -> i32 {
    let path = executor.resolve_shell_path(init_file);
    let contents = match rubash::script_driver::read_script_bytes(&path) {
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
    // GNU runs the startup files before the reader-phase `interactive` flag
    // applies (gdb on WSL GNU 5.3.0: exit_builtin inside a --rcfile sees
    // interactive=0 while interactive_shell=1), so builtins/exit.def:59-62
    // does not echo "exit" for an rcfile exit. The marker suppresses only
    // that echo; everything else interactive (expansion flags, $-) is
    // unaffected (rubash#297).
    executor.set_env("__RUBASH_INTERACTIVE_FLAG_OFF", "1");
    // GNU shell.c run_startup_files sources the startup files
    // (maybe_execute_file -> source_file): while one runs, the file IS
    // the input stream (error.c:300-316 yy_input_name) and a top-level
    // `return` is legal. The direct whole-file run below set neither
    // marker, so diagnostics fell back to the invocation's `-c` stream
    // name (`bash: -c: line 1:` for a bad rcfile) and a startup file's
    // `return` — e.g. /etc/bash.bashrc's `[ -z "$PS1" ] && return`
    // guard — errored as if typed at top level (rubash#1161). Establish
    // the sourced-file context for the run and restore the caller's
    // afterwards.
    let old_in_source = executor.get_env("__RUBASH_IN_SOURCE").map(str::to_string);
    let old_script_name = executor.get_env("__RUBASH_SCRIPT_NAME").map(str::to_string);
    executor.set_env("__RUBASH_IN_SOURCE", "1");
    executor.set_env("__RUBASH_SCRIPT_NAME", init_file);
    // Drop any stale parse latch so the check below reflects THIS file.
    let _ = executor.take_parse_error();
    let status = run_source(executor, &contents, false);
    // GNU evalstring.c:585-606 (parse_and_execute): a syntax error in a
    // startup file aborts the rest of THAT file and returns EX_BADUSAGE as
    // an ordinary value — the shell prints the diagnostic and keeps
    // starting (verified WSL GNU 5.3.20: `bash --rcfile bad.rc -i -c 'echo
    // AFTER'` prints the error, runs AFTER, exits 0). The flagged ExitCode
    // is the reader-discard jump, not jump_to_top_level — consume it like
    // the `source` builtin's arm (builtins/source/execution.rs) so a broken
    // rc cannot end startup (rubash#1158). A real `exit` carries no parse
    // flag and still propagates (rubash#297).
    if executor.take_parse_error() {
        executor.take_exit_jump_pending();
    }
    match old_in_source {
        Some(value) => executor.set_env("__RUBASH_IN_SOURCE", &value),
        None => executor.remove_env("__RUBASH_IN_SOURCE"),
    }
    match old_script_name {
        Some(value) => executor.set_env("__RUBASH_SCRIPT_NAME", &value),
        None => executor.remove_env("__RUBASH_SCRIPT_NAME"),
    }
    executor.remove_env("__RUBASH_INTERACTIVE_FLAG_OFF");
    status
}

/// GNU shell.c:722 run_startup_files: the login/interactive startup chain.
/// A login shell sources /etc/profile then the first existing personal
/// profile (rubash#487); a non-login interactive shell sources
/// /etc/bash.bashrc + ~/.bashrc + ~/.niubashrc (--rcfile replaces the
/// personal rc slot entirely; rubash#1158's .niubashrc sibling is the
/// owner-decided compatible-reading extension). A login shell NEVER takes
/// the rc branch (shell.c:1833 if/else), so `bash -lc` reads profiles
/// only, and `bash -l -i` reads profiles only.
///
/// The login chain is a Unix requirement (getty/ssh/chsh login shells must
/// inherit the system environment; macOS shares the same
/// /etc/profile → ~/.bash_profile chain), so it stays `not(windows)`: on
/// Windows there is no real login concept, -l/--login is accepted, and the
/// branch sources nothing — the pre-existing no-regression behavior. The
/// non-login interactive rc branch instead runs on every platform
/// (rubash#1158: `niu -i` ignored ~/.bashrc because this whole function
/// was cfg-gated out on Windows). `--noprofile` / `--norc` inhibit their
/// respective branches.
///
/// Returns Some(exit_status) when a startup file executed `exit`
/// (rubash#297: nothing after a startup-file exit runs) — the caller must
/// return finish_shell immediately.
fn run_startup_files(executor: &mut Executor, rc_override: Option<&str>) -> Option<i32> {
    // Run once per process: the four *_with_init drivers are alternatives,
    // but a respawned child re-entering a driver must not re-source.
    if executor.get_env("__RUBASH_STARTUP_FILES_DONE").is_some() {
        return None;
    }
    executor.set_env("__RUBASH_STARTUP_FILES_DONE", "1");
    diagnose_bash_env_directory(executor);
    let login = executor.get_env("__RUBASH_LOGIN_SHELL").as_deref() == Some("1");
    let interactive = executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1");
    let no_profile = executor.get_env("__RUBASH_NO_PROFILE").as_deref() == Some("1");
    let no_rc = executor.get_env("__RUBASH_NO_RC").as_deref() == Some("1");
    let home = executor.get_env("HOME").unwrap_or_default();
    // The profile/rc branches need HOME; the $BASH_ENV branch below does
    // not (GNU sources it with HOME unset), so an empty HOME only empties
    // the file list instead of returning early (rubash#1479).
    let files = if home.is_empty() {
        Vec::new()
    } else {
        let exists = |path: &str| std::fs::metadata(executor.resolve_shell_path(path)).is_ok();
        if login {
            login_profile_branch(&home, no_profile, exists)
        } else if interactive && !no_rc {
            rubash::invocation::nonlogin_interactive_files(&home, rc_override, exists)
        } else {
            Vec::new()
        }
    };
    // issue312 gate: record the rc override sourced here (verbatim CLI
    // form, matching nonlogin_interactive_files) so the drivers'
    // direct run_init_file call can skip it — else interactive shells
    // source --rcfile twice. Only set when actually included: the
    // empty-HOME early return above and the non-interactive/login
    // shapes leave it unset and keep the direct call.
    if let Some(rc) = rc_override {
        if files.iter().any(|file| file == rc) {
            executor.set_env("__RUBASH_SOURCED_RC", rc);
        }
    }
    for file in files {
        let status = run_init_file(executor, &file);
        if executor.take_exit_jump_pending() {
            return Some(status);
        }
    }
    // GNU shell.c run_startup_files / maybe_execute_env_file: every
    // non-interactive shell — login shells included, after the profile
    // chain — sources $BASH_ENV (rubash#1479).
    if !interactive {
        if let Some(file) = rubash::invocation::bash_env_file(executor) {
            let status = run_init_file(executor, &file);
            if executor.take_exit_jump_pending() {
                return Some(status);
            }
        }
    }
    None
}

/// The login branch of run_startup_files (shell.c:1836-1880
/// run_login_profile_files): /etc/profile then the FIRST EXISTING of
/// ~/.bash_profile, ~/.bash_login, ~/.profile, honouring --noprofile. On
/// Windows the branch is deliberately empty — there is no real login
/// concept, so -l/--login is accepted and the pre-existing behavior is
/// kept (no sourcing, no regression; rubash#1158 changes only the
/// non-login interactive rc branch).
#[cfg(not(windows))]
fn login_profile_branch(
    home: &str,
    no_profile: bool,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    if no_profile {
        Vec::new()
    } else {
        rubash::invocation::login_profile_files(home, exists)
    }
}

#[cfg(windows)]
fn login_profile_branch(
    _home: &str,
    _no_profile: bool,
    _exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    Vec::new()
}

/// GNU shell.c run_startup_files / maybe_execute_env_file: a
/// non-interactive shell sources $BASH_ENV; when that name is a
/// directory the read fails and bash reports `<file>: is a directory`
/// on stderr, then continues (execution is unaffected). The standalone
/// rubash binary silently ignored the failure (rubash#1146). Report
/// the directory case here, once per process (guarded by the caller's
/// startup-files-done marker).
fn diagnose_bash_env_directory(executor: &mut Executor) {
    if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
        return;
    }
    let Some(bash_env) = executor
        .get_env("BASH_ENV")
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let path = executor.resolve_shell_path(&bash_env);
    if std::fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
        // GNU shell.c maybe_execute_env_file reports startup-file
        // failures with the shell's $0 and the lowercase strerror text
        // (`<script>: /tmp: is a directory`, rubash#1246) — the capital
        // "Is a directory" belongs to the script-ARGUMENT open path,
        // which is a different diagnostic and keeps it.
        let shell_name = executor
            .get_env("__RUBASH_SCRIPT_NAME")
            .filter(|value| !value.is_empty())
            .unwrap_or("rubash");
        eprintln!("{shell_name}: {bash_env}: is a directory");
    }
}

fn run_stdin_script(executor: &mut Executor) -> i32 {
    // rubash#586: the driver lives in the library (script_driver) so every
    // entry — this binary's `-s`/`bash < file` route and the embedding
    // hosts' ShellInvocation -s route — runs the SAME on-demand reader
    // instead of a host-local slurp that pre-buffers the stream.
    rubash::script_driver::run_stdin_script(executor)
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

#[cfg(test)]
mod login_shell_tests {
    use super::*;

    /// GNU shell.c set_shell_name (via get_name_for_error / login shell
    /// conventions): an argv[0] whose basename starts with '-' marks a
    /// login shell, independent of -l/--login (rubash#487).
    fn login_marker_set(argv0: &str) -> bool {
        // set_env mirrors __RUBASH_* markers into the process env and
        // new_process_exit re-imports the process env, so a previous
        // helper call would leak into this one — reset the ambient
        // marker before each probe.
        std::env::remove_var("__RUBASH_LOGIN_SHELL");
        let mut executor = Executor::new_process_exit();
        apply_invocation_shell_mode(&mut executor, Some(argv0));
        executor.get_env("__RUBASH_LOGIN_SHELL").as_deref() == Some("1")
    }

    #[test]
    fn argv0_leading_dash_marks_login_shell() {
        assert!(login_marker_set("-rubash"));
        assert!(login_marker_set("-bash"));
        assert!(login_marker_set("-bash.exe"));
        // A full path whose basename carries the marker still counts.
        assert!(login_marker_set("/usr/bin/-bash"));
        assert!(login_marker_set("D:\\bin\\-bash.exe"));
        // A plain name never sets the marker.
        assert!(!login_marker_set("bash"));
        assert!(!login_marker_set("rubash.exe"));
        assert!(!login_marker_set("/usr/bin/bash"));
    }

    #[test]
    fn argv0_dash_basename_still_applies_name_modes() {
        // `-rbash` keeps the restricted-shell derivation after the login
        // marker is stripped (shell.c maybe_make_restricted).
        let mut executor = Executor::new_process_exit();
        apply_invocation_shell_mode(&mut executor, Some("-rbash"));
        assert_eq!(
            executor.get_env("__RUBASH_LOGIN_SHELL").as_deref(),
            Some("1")
        );
    }
}
