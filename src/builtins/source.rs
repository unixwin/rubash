//! source module.
//!
//! GNU Bash source ownership:
// - builtins/source.def
// - execute_cmd.c
// - redir.c
// - subst.c

mod execution;
mod flow;
mod if_alias;
mod invocation;
mod pipe_source;
mod simple_if;

pub use execution::{execute_text, execute_text_with_args};
pub(crate) use flow::normalize_inline_compound_commands;
pub use pipe_source::execute_pipe_into_source;
pub use simple_if::execute_simple_if;

use invocation::{SourceInvocation, SourceParseError};

use crate::executor::{ExecuteError, Executor};
use crate::parser::CommandNode;
use std::io::Write;

pub fn execute(executor: &mut Executor, args: &[String]) -> Result<(), ExecuteError> {
    execute_named(executor, "source", args)
}

pub fn execute_named(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    execute_named_with_io(executor, command_name, args, &mut std::io::stderr().lock())
}

pub fn execute_named_with_io<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    execute_named_with_io_impl(executor, command_name, args, stderr, None)
}

pub fn execute_named_with_io_and_redirects<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
    redirect_cmd: &CommandNode,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    execute_named_with_io_impl(executor, command_name, args, stderr, Some(redirect_cmd))
}

fn execute_named_with_io_impl<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
    redirect_cmd: Option<&CommandNode>,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    // TODO(builtins/source.def): GNU Bash `source_builtin` uses unwind/trap
    // machinery around `source_file`.
    let invocation = match SourceInvocation::parse(args) {
        Ok(invocation) => invocation,
        Err(error) => {
            match error {
                SourceParseError::MissingFilename => {
                    writeln!(
                        stderr,
                        "{}{command_name}: filename argument required",
                        executor.diagnostic_prefix()
                    )?;
                }
                SourceParseError::MissingPathArgument => {
                    writeln!(
                        stderr,
                        "{}{command_name}: -p: option requires an argument",
                        executor.diagnostic_prefix()
                    )?;
                }
                SourceParseError::InvalidOption(option) => {
                    writeln!(
                        stderr,
                        "{}{command_name}: -{option}: invalid option",
                        executor.diagnostic_prefix()
                    )?;
                }
            }
            writeln!(
                stderr,
                "{command_name}: usage: {command_name} [-p path] filename [arguments]"
            )?;
            executor.set_exit_code(2);
            return Ok(());
        }
    };
    let filename = invocation.filename;

    if let Some(process_source) = filename
        .strip_prefix("<(")
        .and_then(|filename| filename.strip_suffix(')'))
    {
        if let Some(source) = executor.process_substitution_output(process_source) {
            return execution::execute_text_maybe_redirected(
                executor,
                &source,
                invocation.args,
                redirect_cmd,
                None,
            );
        }
    }

    if is_null_device(filename) {
        executor.set_exit_code(0);
        return Ok(());
    }

    if filename == "echo" {
        // TODO(subst.c/execute_cmd.c): Process substitution should create a
        // /dev/fd path whose content is the command's stdout. The current
        // parser sees `. <(echo "echo two - OK")` as `source echo ...`; source
        // that generated text directly until process substitution is parsed.
        let source = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        if !source.is_empty() {
            return execution::execute_text_maybe_redirected(
                executor,
                &source,
                &[],
                redirect_cmd,
                None,
            );
        }
    }

    if matches!(filename, "/dev/stdin" | "/proc/self/fd/0" | "/dev/fd/0") {
        // GNU source.def opens /dev/stdin, which in a pipeline is the pipe
        // read end inherited as stdin ("echo x | . /dev/stdin" from
        // source6.sub). Read the shell's stdin instead of resolving a
        // filesystem path (Windows has no /dev node for it).
        // Inside a pipeline stage the upstream text is carried in
        // __RUBASH_FUNCTION_STDIN (execute_builtin_pipeline_stage); prefer it
        // over the process stdin, which is not wired for builtin stages.
        let piped = executor
            .get_env(crate::executor::types::FUNCTION_STDIN)
            .map(str::to_string);
        let buffer = match piped {
            Some(text) => text,
            None => {
                use std::io::Read;
                let mut buffer = Vec::new();
                match std::io::stdin().read_to_end(&mut buffer) {
                    Ok(_) => crate::script_driver::bytes_to_script_text(&buffer),
                    Err(_) => {
                        executor.set_exit_code(1);
                        return Ok(());
                    }
                }
            }
        };
        return execution::execute_text_maybe_redirected(
            executor,
            &buffer,
            invocation.args,
            redirect_cmd,
            Some(filename),
        );
    }

    // rubash#396: a `/dev/fd/N` word whose bytes live in the executor's fd
    // table (the expansion-materialized `source <(cmd)` shape). GNU
    // source.def hands the word to _evalfile and evalfile.c:104 open()s it
    // — on Linux /dev/fd/N reopens the process_substitute pipe end parked
    // at fd >= 64 (subst.c:6392 move_to_high_fd); on Windows the endpoint
    // is in the fd table, so read it there. No live endpoint falls through
    // to the filesystem path below.
    if let Some(text) = executor.sourced_dev_fd_text(filename) {
        return execution::execute_text_maybe_redirected(
            executor,
            &text,
            invocation.args,
            redirect_cmd,
            Some(filename),
        );
    }

    let Some(source_path) = invocation.resolve_path(executor) else {
        if invocation.path.is_some() || posix_plain_name_lookup(executor, filename) {
            writeln!(
                stderr,
                "{}.: {filename}: file not found",
                executor.diagnostic_prefix()
            )?;
        } else {
            writeln!(
                stderr,
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            )?;
        }
        executor.set_exit_code(1);
        if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
            return Err(ExecuteError::ExitCode(1));
        }
        return Ok(());
    };

    // GNU `source` has no UTF-8 validity gate either (builtins/source.def
    // reads the file as raw bytes); rubash#132: invalid-sequence bytes ride
    // as raw-byte marker pairs instead of failing the read as a spurious
    // "No such file or directory".
    let source = match crate::script_driver::read_script_bytes(&source_path) {
        Ok(source) => source,
        Err(_) => {
            writeln!(
                stderr,
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            )?;
            executor.set_exit_code(1);
            if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
                return Err(ExecuteError::ExitCode(1));
            }
            return Ok(());
        }
    };

    // GNU builtins/evalfile.c:184-212 (evalfile_internal; source_file's
    // flags carry FEVAL_BUILTIN but not FEVAL_CHECKBINARY, so the 80-byte
    // check_binary_file gate does NOT apply to `.`): when the buffer holds
    // NUL bytes, evalfile strips them while scanning and refuses the file
    // once more than 256 NULs were removed -- "probably a binary file" --
    // with `<filename>: cannot execute binary file` and EX_BINARY_FILE
    // (shell.h:63 = 126). Sourcing the shell binary (`. ${THIS_SH}` in
    // execscript.tests) otherwise feeds megabytes of ELF to the parser.
    let source = match evalfile_nul_stripped(&source) {
        Ok(text) => text,
        Err(_) => {
            writeln!(
                stderr,
                "{}{command_name}: {filename}: cannot execute binary file",
                executor.diagnostic_prefix()
            )?;
            executor.set_exit_code(126);
            return Ok(());
        }
    };

    execution::execute_text_maybe_redirected(
        executor,
        &source,
        invocation.args,
        redirect_cmd,
        Some(filename),
    )
}

fn is_null_device(path: &str) -> bool {
    crate::executor::path::is_shell_null_device(path)
}

/// GNU builtins/evalfile.c:184-212 (evalfile_internal): when the sourced
/// buffer contains NUL bytes (`strlen(string) < nr`), evalfile strips them
/// with memmove while scanning and aborts with "cannot execute binary file"
/// once the removed-NUL count passes 256 (`++nnull > 256`, the guard is
/// active for every FEVAL_BUILTIN caller, i.e. the `.`/`source` builtin).
/// The scan is byte-exact with the C loop, including its quirk that the
/// byte shifted into a removed NUL's slot is never re-examined (the loop
/// `i++` skips it), so a RUN of adjacent NULs removes and counts only every
/// other byte (257 contiguous NULs strip 129 and succeed; GNU measured the
/// same). `Ok` returns the stripped text; `Err` carries the count that
/// exceeded the limit.
pub(crate) fn evalfile_nul_stripped(source: &str) -> Result<String, usize> {
    if !source.as_bytes().contains(&0) {
        return Ok(source.to_string());
    }
    let mut bytes = source.as_bytes().to_vec();
    let mut nr = bytes.len();
    let mut nnull = 0usize;
    let mut i = 0usize;
    while i < nr {
        if bytes[i] == 0 {
            bytes.copy_within(i + 1..nr, i);
            nr -= 1;
            nnull += 1;
            if nnull > 256 {
                return Err(nnull);
            }
        }
        i += 1;
    }
    // evalfile.c:212+ hands the stripped buffer to parse_and_execute as a C
    // string (builtins/evalstring.c), so a NUL that survived the strip
    // (every other byte of an adjacent run) terminates the sourced text:
    // GNU sources only up to the first surviving NUL.
    if let Some(first) = bytes[..nr].iter().position(|byte| *byte == 0) {
        nr = first;
    }
    // Removing NUL bytes cannot invalidate UTF-8 (NUL is a complete
    // single-byte sequence), so the lossy decode is a lossless clone.
    Ok(String::from_utf8_lossy(&bytes[..nr]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::evalfile_nul_stripped;

    #[test]
    fn plain_text_passes_through_unchanged() {
        let text = "echo hello\nexit 0\n";
        assert_eq!(evalfile_nul_stripped(text), Ok(text.to_string()));
    }

    #[test]
    fn scattered_nuls_are_stripped_and_sourced() {
        // evalfile.c:184-206: a NUL followed by a non-NUL byte is removed
        // while scanning; the byte memmove shifts into the removed slot is
        // skipped, so one NUL of an adjacent pair survives -- and that
        // survivor then ends the sourced text (C-string parse,
        // evalstring.c): `echo a\0\0 b` sources as `echo a`.
        let text = "echo a\0\0 b\necho c\0\n";
        assert_eq!(evalfile_nul_stripped(text), Ok("echo a".to_string()));
        let lone = "x\0y\0z\n";
        assert_eq!(evalfile_nul_stripped(lone), Ok("xyz\n".to_string()));
    }

    #[test]
    fn contiguous_nul_run_strips_only_every_other_byte() {
        // The C loop's i++ skips the byte memmove shifts into the removed
        // slot, so a run of adjacent NULs removes only every other byte
        // (GNU Bash 5.3.0 measured: `. file-with-257-contiguous-NULs`
        // exits 0). 257 NULs -> 129 removed, and the first surviving NUL
        // then ends the sourced text (C-string parse, evalstring.c), so
        // only the text before the run is sourced.
        let run = format!("x{}\necho tail\n", "\0".repeat(257));
        assert_eq!(evalfile_nul_stripped(&run), Ok("x".to_string()));
    }

    #[test]
    fn more_than_256_removed_nuls_is_refused_as_binary() {
        // execscript.tests `. ${THIS_SH}`: an ELF image is NUL-dense; GNU
        // reports "cannot execute binary file" / EX_BINARY_FILE (126).
        let binary = format!("\u{7f}ELF\x02\x01\x01\0{}", "x\0".repeat(300));
        assert!(evalfile_nul_stripped(&binary).is_err());
        let sparse = format!("{}\0", "echo ok\0\n".repeat(300));
        assert!(evalfile_nul_stripped(&sparse).is_err());
    }
}

fn posix_plain_name_lookup(executor: &Executor, filename: &str) -> bool {
    executor.get_env("__RUBASH_POSIX_MODE") == Some("1")
        && !filename.contains('/')
        && !filename.contains('\\')
}
