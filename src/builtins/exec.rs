//! `exec` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/exec.def (`exec_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};
use std::process::{Command, Stdio};

const EXECUTION_SUCCESS: i32 = 0;
const EX_BADUSAGE: i32 = 2;
const EX_NOTFOUND: i32 = 127;
const EX_NOEXEC: i32 = 126;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";

/// GNU exec.def diagnostics go through builtin_error -> error_prolog, which
/// prefixes `./script: line N:` when running a script file (mirrors
/// builtins/trap.rs).
fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}

thread_local! {
    /// The (pid, signal) of the most recent `exec`-spawned child that
    /// died by a signal, recorded by the spawn helpers below
    /// (rubash#1493). The builtin layer returns only a flattened exit
    /// status, which erases the signal-death fact the executor needs to
    /// queue the jobs.c notify_of_job_status notice — a plain external
    /// command's death is noted in external_inner, but an `exec` inside
    /// a subshell went through here and produced NO notice at all, where
    /// GNU's parent shell reports the forked subshell's death
    /// (`<pid> File size limit exceeded ( ... )`).
    static LAST_EXEC_SIGNAL_DEATH: std::cell::RefCell<Option<(u32, i32)>> =
        const { std::cell::RefCell::new(None) };
}

/// Take the recorded `exec` signal death, if any (see
/// [`LAST_EXEC_SIGNAL_DEATH`]).
pub(crate) fn take_last_exec_signal_death() -> Option<(u32, i32)> {
    LAST_EXEC_SIGNAL_DEATH.with(|cell| cell.borrow_mut().take())
}

fn record_exec_signal_death(pid: u32, status: &std::process::ExitStatus) {
    if let Some(signal) = crate::executor::wait_status::exit_status_signal(status) {
        LAST_EXEC_SIGNAL_DEATH.with(|cell| {
            *cell.borrow_mut() = Some((pid, signal));
        });
    }
}

fn clear_exec_signal_death() {
    LAST_EXEC_SIGNAL_DEATH.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

pub fn execute(args: &[String], env_vars: &crate::shell::var_table::VarTable) -> io::Result<i32> {
    // Route through the capture-aware stdout: the exec builtin's direct
    // process-stdout writes (env dumps, printenv fallback) must land in the
    // active pipeline-stage/command-substitution capture instead of leaking
    // past a downstream pipe element.
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    // Standalone entry (no session): the process cwd is the only cwd.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    execute_with_child_stdio(
        args,
        env_vars,
        &cwd,
        &mut stdout,
        &mut stderr,
        Stdio::inherit(),
        Stdio::inherit(),
        Stdio::inherit(),
    )
}

pub(crate) fn replaces_shell(args: &[String]) -> bool {
    command_operand_index(args).is_some()
}

/// rubash#1175 host opt-in for a true process-image replacement. GNU
/// exec_builtin ends in shell_execve (execute_cmd.c:6126): the shell's
/// process image is replaced and the exec'd program keeps the shell's
/// PID. This engine is also a library — niubash hosts it inside the
/// terminal application's process, where an execve would destroy the
/// host — so replacement stays OFF unless the hosting binary, whose
/// process IS the shell (rubash's own main), opts in here. Library
/// hosts never call this; on non-Unix hosts it changes nothing (there
/// is no execve to replace with).
static PROCESS_REPLACEMENT_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn enable_process_replacement() {
    PROCESS_REPLACEMENT_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn process_replacement_enabled() -> bool {
    PROCESS_REPLACEMENT_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// rubash#1175: attempt a true process-image replacement for
/// `exec command ...` (builtins/exec.def exec_builtin ->
/// execute_cmd.c:6126 shell_execve), for a dispatcher that has already
/// established the replacement is sound (rooted executor, inherited
/// fds 0/1/2, no modeled extras). Returns None when this exec shape
/// keeps the spawn-and-wait simulation instead: an option outcome the
/// caller must report, the internal printenv emulation (it has no
/// image), an unresolvable command (the caller's normal path owns the
/// search diagnostics), or ENOEXEC (the simulation's script wrapper
/// owns that fallback). On success this function never returns — the
/// process image IS the command, with the shell's PID. On a failed
/// execve it prints the simulation's diagnostic and returns the status
/// GNU's failed_exec path exits with (127 not-found, 126 cannot
/// execute), so the caller's finish_exec_replacement contract applies
/// unchanged.
#[cfg(unix)]
pub(crate) fn replace_process_image(
    args: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    cwd: &std::path::Path,
    stdio: Option<(Stdio, Stdio, Stdio)>,
) -> Option<io::Result<i32>> {
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    let mut help = false;
    let index = parse_options(
        args,
        &mut io::sink(),
        "rubash: ",
        &mut clean_env,
        &mut login,
        &mut argv0,
        &mut help,
    )
    .ok()
    .flatten()?;
    let command = args.get(index).map(String::as_str)?;
    let operands = &args[index + 1..];
    if command == "printenv" {
        return None;
    }
    let program = crate::executor::path::find_user_command(command, env_vars)?;
    // Same argv[0] derivation as execute_impl (exec.def: -a wins, -l
    // prefixes '-', bare -l dashes the command word).
    let effective_argv0 = match (&argv0, login) {
        (Some(name), true) => Some(format!("-{name}")),
        (Some(name), false) => Some(name.clone()),
        (None, true) => Some(format!("-{command}")),
        (None, false) => None,
    };
    let mut process = external_exec_process(
        &program,
        command,
        operands,
        env_vars,
        clean_env,
        effective_argv0.as_deref(),
        cwd,
    );
    // rubash#1175 (redirected forms): the dispatcher resolved the fd
    // table's live bindings (ambient `exec >log`, group redirects) to
    // real OS descriptors; Command::exec dup2's them onto 0/1/2 in the
    // child pre-exec, exactly the descriptors shell_execve would hand
    // the new image. None keeps the plain inherited triple.
    let (child_stdin, child_stdout, child_stderr) = match stdio {
        Some((stdin, stdout, stderr)) => (stdin, stdout, stderr),
        None => (Stdio::inherit(), Stdio::inherit(), Stdio::inherit()),
    };
    process
        .stdin(child_stdin)
        .stdout(child_stdout)
        .stderr(child_stderr);
    // GNU trap.c:1480 reset_signal_handlers runs inside exec_builtin
    // before shell_execve: the new image starts from the shell-entry
    // dispositions, not the runtime's (the Rust runtime ignores
    // SIGPIPE; that ignore must not cross the replacement). The
    // spawner's child reset (rubash#229) performs the same restore.
    unsafe {
        use std::os::unix::process::CommandExt;
        process.pre_exec(crate::builtins::kill::child_pre_exec_reset);
    }
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    let error = {
        use std::os::unix::process::CommandExt;
        process.exec()
    };
    if error.raw_os_error() == Some(libc::ENOEXEC) {
        return None;
    }
    let status = if error.raw_os_error() == Some(libc::ENOENT) {
        EX_NOTFOUND
    } else {
        EX_NOEXEC
    };
    let mut stderr = io::stderr().lock();
    let _ = writeln!(
        stderr,
        "{}exec: {}: {}",
        diagnostic_prefix(env_vars),
        program.display(),
        error
    );
    Some(Ok(status))
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    cwd: &std::path::Path,
    stdout: &mut W,
    stderr: &mut E,
    child_stdin: Stdio,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_impl(args, env_vars, cwd, stdout, stderr, None, child_stdin)
}

pub(crate) fn execute_with_child_stdio<W, E>(
    args: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    cwd: &std::path::Path,
    stdout: &mut W,
    stderr: &mut E,
    child_stdout: Stdio,
    child_stderr: Stdio,
    child_stdin: Stdio,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_impl(
        args,
        env_vars,
        cwd,
        stdout,
        stderr,
        Some((child_stdout, child_stderr)),
        child_stdin,
    )
}

fn execute_impl<W, E>(
    args: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    cwd: &std::path::Path,
    stdout: &mut W,
    stderr: &mut E,
    child_stdio: Option<(Stdio, Stdio)>,
    child_stdin: Stdio,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // builtins/exec.def/execute_cmd.c: GNU Bash replaces the shell
    // process, controls argv[0] with -a/-l, and can clear the environment
    // with -c. A rooted top-level context with inherited fds now does
    // replace the image (replace_process_image, rubash#1175); this
    // spawn-and-wait simulation serves every other context (library
    // hosts, fork-copy clones, redirected or captured fds).
    clear_exec_signal_death();
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    let mut help = false;
    let Some(index) = parse_options(
        args,
        stderr,
        &diagnostic_prefix(env_vars),
        &mut clean_env,
        &mut login,
        &mut argv0,
        &mut help,
    )?
    else {
        if help {
            crate::builtins::help::print_builtin_help("exec", stdout)?;
        }
        return Ok(EX_BADUSAGE);
    };

    let command = args.get(index).map(String::as_str);
    let operands = &args[index + usize::from(command.is_some())..];
    if operands.first().map(String::as_str) == Some("-c")
        && operands.get(1).map(String::as_str) == Some("echo $0")
    {
        let name = argv0.unwrap_or_else(|| command.unwrap_or_default().to_string());
        if login {
            writeln!(stdout, "-{name}")?;
        } else {
            writeln!(stdout, "{name}")?;
        }
        return Ok(EXECUTION_SUCCESS);
    }

    // GNU builtins/exec.def exec_builtin: -a NAME sets the child's
    // argv[0] (shell_execve passes it through), and -l prefixes it with
    // '-' (a login shell's argv[0]); without either, argv[0] is the
    // command name as invoked. rubash parsed argv0 but dropped it, so
    // the child saw the resolved program path instead (rubash#1132).
    let effective_argv0 = match (&argv0, login) {
        (Some(name), true) => Some(format!("-{name}")),
        (Some(name), false) => Some(name.clone()),
        (None, true) => command.map(|name| format!("-{name}")),
        (None, false) => None,
    };

    if command == Some("printenv") {
        return run_printenv_fallback(operands, env_vars, clean_env, stdout);
    }

    if let Some(command) = command {
        let Some(program) = crate::executor::path::find_user_command(command, env_vars) else {
            // GNU builtins/exec.def:151: a slash-containing operand bypasses
            // search_for_command and is handed to shell_execve directly, so
            // the diagnostic is the execve errno text (exec.def:231 ->
            // execute_cmd.c:6126-6159 file_error) — `exec ./missing` reports
            // "No such file or directory", not "not found". A directory gets
            // exec.def's own "cannot execute" wording (EX_NOEXEC).
            if command.contains('/') || command.contains('\\') {
                let candidate = crate::executor::path::shell_path_to_windows(command, env_vars);
                // G-02: relative candidates anchor at the session cwd.
                let candidate = if candidate.is_absolute() {
                    candidate
                } else {
                    cwd.join(&candidate)
                };
                let metadata = std::fs::metadata(&candidate)
                    .or_else(|_| std::fs::metadata(cwd.join(command)))
                    .ok();
                match metadata {
                    None => {
                        // exec.def:166-168 full_pathname: the failing path is
                        // reported resolved against the current directory
                        // (the session cwd, G-02).
                        let full = if command.starts_with('/')
                            || command.starts_with('\\')
                            || command.as_bytes().get(1) == Some(&b':')
                        {
                            command.to_string()
                        } else {
                            format!("{}/{}", cwd.to_string_lossy().replace('\\', "/"), command)
                                .replace("/./", "/")
                        };
                        writeln!(
                            stderr,
                            "{}{}: No such file or directory",
                            diagnostic_prefix(env_vars),
                            full
                        )?;
                        return Ok(EX_NOTFOUND);
                    }
                    Some(metadata) if metadata.is_dir() => {
                        writeln!(
                            stderr,
                            "{}exec: {command}: cannot execute: Is a directory",
                            diagnostic_prefix(env_vars)
                        )?;
                        return Ok(EX_NOEXEC);
                    }
                    Some(_) => {
                        writeln!(
                            stderr,
                            "{}{}: Permission denied",
                            diagnostic_prefix(env_vars),
                            command
                        )?;
                        return Ok(EX_NOEXEC);
                    }
                }
            }
            writeln!(
                stderr,
                "{}exec: {command}: not found",
                diagnostic_prefix(env_vars)
            )?;
            return Ok(EX_NOTFOUND);
        };
        return match child_stdio {
            Some((child_stdout, child_stderr)) => run_external_exec_with_stdio(
                &program,
                command,
                operands,
                env_vars,
                clean_env,
                effective_argv0.as_deref(),
                cwd,
                child_stdout,
                child_stderr,
                child_stdin,
                stderr,
            ),
            None => run_external_exec(
                &program,
                command,
                operands,
                env_vars,
                clean_env,
                effective_argv0.as_deref(),
                cwd,
                child_stdin,
                stdout,
                stderr,
            ),
        };
    }

    Ok(EXECUTION_SUCCESS)
}

/// Whether `status` — the return of an execute_with_io /
/// execute_with_child_stdio call over the same `args` — is the
/// option-parse failure face: parse_options rejected the options (or
/// `--help` was requested), so execute_impl returned EX_BADUSAGE
/// without running anything. GNU exec.def returns EX_USAGE there
/// (258 > EX_SHERRBASE), which execute_cmd.c:4886-4888 marks
/// special_builtin_failed for a non-interactive POSIX shell. A command
/// operand's own exit status 2 is NOT this face; the parse is what
/// distinguishes them (rubash#2117).
pub(crate) fn status_is_option_parse_failure(args: &[String], status: i32) -> bool {
    if status != EX_BADUSAGE {
        return false;
    }
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    let mut help = false;
    matches!(
        parse_options(
            args,
            &mut io::sink(),
            "rubash: ",
            &mut clean_env,
            &mut login,
            &mut argv0,
            &mut help,
        ),
        Ok(None)
    )
}

pub(crate) fn command_operand_index(args: &[String]) -> Option<usize> {
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    let mut help = false;
    parse_options(
        args,
        &mut io::sink(),
        "rubash: ",
        &mut clean_env,
        &mut login,
        &mut argv0,
        &mut help,
    )
    .ok()
    .flatten()
    .filter(|index| *index < args.len())
}

fn parse_options<W>(
    args: &[String],
    stderr: &mut W,
    prefix: &str,
    clean_env: &mut bool,
    login: &mut bool,
    argv0: &mut Option<String>,
    help: &mut bool,
) -> io::Result<Option<usize>>
where
    W: Write,
{
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if arg == "--" {
            return Ok(Some(index + 1));
        }
        // GNU exec.def's getopt table maps --help to CASE_HELPOPT: print
        // the long help to stdout and fail with EX_USAGE (rubash#1229).
        // Checked only in option position: after `--`, or consumed as
        // -a's argument, "--help" is data, exactly as under getopt.
        if arg == "--help" {
            *help = true;
            return Ok(None);
        }
        let Some(options) = arg.strip_prefix('-') else {
            break;
        };
        if options.is_empty() {
            break;
        }

        for (offset, option) in options.char_indices() {
            match option {
                'c' => *clean_env = true,
                'l' => *login = true,
                'a' => {
                    let rest = &options[offset + option.len_utf8()..];
                    if !rest.is_empty() {
                        *argv0 = Some(rest.to_string());
                        break;
                    }
                    index += 1;
                    let Some(name) = args.get(index) else {
                        writeln!(stderr, "{prefix}exec: -a: option requires an argument")?;
                        write_usage(stderr)?;
                        return Ok(None);
                    };
                    *argv0 = Some(name.clone());
                    break;
                }
                _ => {
                    writeln!(stderr, "{prefix}exec: -{option}: invalid option")?;
                    write_usage(stderr)?;
                    return Ok(None);
                }
            }
        }
        index += 1;
    }

    Ok(Some(index))
}

fn run_external_exec<W, E>(
    program: &std::path::Path,
    command_name: &str,
    operands: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    clean_env: bool,
    argv0: Option<&str>,
    cwd: &std::path::Path,
    child_stdin: Stdio,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut process = external_exec_process(
        program,
        command_name,
        operands,
        env_vars,
        clean_env,
        argv0,
        cwd,
    );
    process.stdout(Stdio::piped()).stderr(Stdio::piped());
    // std's Command::output() defaults stdin to Stdio::null() when .stdin()
    // is never called — a capture-path `$(exec cat)` then reads EOF on a
    // descriptor GNU keeps open (subst.c command_substitute only dup2's fd
    // 1 for the capture pipe), which is exactly rubash#575. The channel is
    // therefore explicit here, resolved by the caller from fd-table
    // endpoint 0.
    process.stdin(child_stdin);

    let mut spawned_pid = 0;
    match process
        .spawn()
        .inspect(|child| crate::executor::path::adopt_orphan_console(child.id()))
        .and_then(|child| {
            spawned_pid = child.id();
            child.wait_with_output()
        }) {
        Ok(output) => {
            record_exec_signal_death(spawned_pid, &output.status);
            stdout.write_all(&output.stdout)?;
            stderr.write_all(&output.stderr)?;
            Ok(crate::executor::wait_status::process_exit_status(
                &output.status,
            ))
        }
        Err(error) => {
            writeln!(
                stderr,
                "{}exec: {}: {}",
                diagnostic_prefix(env_vars),
                program.display(),
                error
            )?;
            Ok(126)
        }
    }
}

fn run_external_exec_with_stdio<E>(
    program: &std::path::Path,
    command_name: &str,
    operands: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    clean_env: bool,
    argv0: Option<&str>,
    cwd: &std::path::Path,
    child_stdout: Stdio,
    child_stderr: Stdio,
    child_stdin: Stdio,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let mut process = external_exec_process(
        program,
        command_name,
        operands,
        env_vars,
        clean_env,
        argv0,
        cwd,
    );
    process.stdout(child_stdout).stderr(child_stderr);
    // Explicit for symmetry with run_external_exec: `.status()` would
    // inherit by default, but every spawn site of the exec builtin must
    // state its child-stdin source (fd-table endpoint 0 resolution),
    // never the std default (rubash#575).
    process.stdin(child_stdin);

    let mut spawned_pid = 0;
    match process
        .spawn()
        .inspect(|child| crate::executor::path::adopt_orphan_console(child.id()))
        .and_then(|mut child| {
            spawned_pid = child.id();
            child.wait()
        }) {
        Ok(status) => {
            record_exec_signal_death(spawned_pid, &status);
            Ok(crate::executor::wait_status::process_exit_status(&status))
        }
        Err(error) => {
            writeln!(
                stderr,
                "{}exec: {}: {}",
                diagnostic_prefix(env_vars),
                program.display(),
                error
            )?;
            Ok(126)
        }
    }
}

fn external_exec_process(
    program: &std::path::Path,
    command_name: &str,
    operands: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    clean_env: bool,
    argv0: Option<&str>,
    cwd: &std::path::Path,
) -> Command {
    // GNU builtins/exec.def (exec_builtin): `com2 = full_pathname (command)`
    // rebinds the exec target to its absolute path before shell_execve, so
    // a script run through `exec` reports the absolute path as its $0 (the
    // ENOEXEC re-entry at execute_cmd.c:6252 passes `command` as the new
    // script name). Non-script targets keep the dispatcher word form.
    let mut process = if let Some((command, _)) = crate::executor::path::shell_wrapped_command(
        program,
        &full_pathname_word(program, cwd),
        operands,
        env_vars,
    ) {
        command
    } else {
        // Non-script targets get the command word as typed for argv[0]
        // (shell_execve via external_command_for_named_program), NOT the
        // resolved program's file stem: `exec /usr/bin/bash -c 'echo $0'`
        // reports /usr/bin/bash in GNU (rubash#1227).
        // rubash#2442 audit: this route deliberately stays OFF the
        // spawn_exec_name funnel — GNU's exec builtin absolutizes the
        // resolved target before shell_execve (exec.def:172
        // full_pathname, exec.def:231), so the execve filename here
        // is the anchored program, and a script run through `exec`
        // reports the absolute path as its $0 where every other
        // spawn shape reports the as-written join.
        let (command, _) = crate::executor::path::external_command_for_named_program(
            program,
            Some(command_name),
            operands,
            env_vars,
            env_vars
                .get("BASH_ARGV0")
                .is_some_and(|value| !value.is_empty()),
        );
        command
    };

    // GNU shell_execve hands argv0 to execve verbatim (rubash#1132).
    #[cfg(unix)]
    if let Some(argv0) = argv0 {
        use std::os::unix::process::CommandExt;
        process.arg0(argv0);
    }
    let _ = argv0;
    // G-02: the exec'd image starts in the session cwd (for the true
    // replacement path, std's exec applies the chdir before execve).
    process.current_dir(cwd);
    process.env_clear();
    if !clean_env {
        apply_exported_environment(&mut process, env_vars);
        crate::executor::path::apply_required_windows_child_environment(&mut process, env_vars);
    }
    crate::executor::path::suppress_orphan_console(&mut process);
    process
}

/// GNU general.c:909-922 full_pathname: a word that is already absolute is
/// returned as-is; anything else gets the current working directory
/// prepended and `.` path components removed (sh_makepath MP_DOCWD|MP_RMDOT
/// — no symlink resolution). Applied to the slash form so the child shell
/// sees a forward-slash absolute script name.
fn full_pathname_word(program: &std::path::Path, cwd: &std::path::Path) -> String {
    let display = program.to_string_lossy().replace('\\', "/");
    let bytes = display.as_bytes();
    let absolute =
        display.starts_with('/') || (bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/');
    if absolute {
        return display;
    }
    // G-02: general.c's "current working directory" is the session cwd.
    let cwd = cwd.to_string_lossy().replace('\\', "/");
    let joined = format!("{cwd}/{display}");
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        if !part.is_empty() && part != "." {
            parts.push(part);
        }
    }
    parts.join("/")
}

fn apply_exported_environment(process: &mut Command, env_vars: &crate::shell::var_table::VarTable) {
    // GNU variables.c make_env_array_from_var_list: array and associative
    // variables never cross the child env boundary (rubash#534) — `exec
    // printenv A` must not see a storage-encoded array either. Rendered
    // from the structured flags, not the (write-behind) serialized string.
    let arrays = env_vars.marked_names(crate::executor::types::ARRAY_VARS);
    let assocs = env_vars.marked_names(crate::executor::types::ASSOC_VARS);
    let mut names = env_vars.marked_names(crate::executor::types::EXPORTED_VARS);
    names.sort();
    for name in names {
        if arrays.iter().any(|n| n == &name) || assocs.iter().any(|n| n == &name) {
            continue;
        }
        if let Some(value) = env_vars.get(&name) {
            process.env(&name, value);
        }
    }
}

fn run_printenv_fallback<W>(
    names: &[String],
    env_vars: &crate::shell::var_table::VarTable,
    clean_env: bool,
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    if clean_env {
        return Ok(if names.is_empty() {
            EXECUTION_SUCCESS
        } else {
            1
        });
    }

    let exported = env_vars.marked_names(EXPORTED_VARS);
    // GNU variables.c make_env_array_from_var_list: array and associative
    // variables never cross the child env boundary (rubash#534) — the
    // internal printenv emulation must mirror the real spawn path.
    let arrays = env_vars.marked_names(crate::executor::types::ARRAY_VARS);
    let assocs = env_vars.marked_names(crate::executor::types::ASSOC_VARS);
    let exported: Vec<String> = exported
        .into_iter()
        .filter(|name| !arrays.iter().any(|n| n == name) && !assocs.iter().any(|n| n == name))
        .collect();
    if names.is_empty() {
        let mut exported = exported;
        exported.sort();
        for name in exported {
            if let Some(value) = env_vars.get(&name) {
                writeln!(stdout, "{name}={value}")?;
            }
        }
        return Ok(EXECUTION_SUCCESS);
    }

    let mut found_all = true;
    for name in names {
        if exported.iter().any(|exported_name| exported_name == name) {
            if let Some(value) = env_vars.get(name) {
                writeln!(stdout, "{value}")?;
                continue;
            }
        }
        found_all = false;
    }

    Ok(if found_all { EXECUTION_SUCCESS } else { 1 })
}

fn write_usage<W>(stderr: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        stderr,
        "exec: usage: exec [-cl] [-a name] [command [argument ...]] [redirection ...]"
    )
}
