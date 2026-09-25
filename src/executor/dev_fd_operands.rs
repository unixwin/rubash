//! Materialize `/dev/std*` / `/dev/fd/N` / `/proc/self/fd/N` operands into
//! concrete paths for external children.
//!
//! GNU passes these words through literally (execute_cmd.c shell_execve
//! hands argv to execve unchanged); the child resolves them because POSIX
//! systems expose /dev/fd/N as an alias for descriptor N. Windows has no
//! such OS-level contract, so the literal only works when the child's own
//! emulation understands the inherited handle — MSYS2 tools cannot reopen
//! a foreign anonymous pipe through /proc/self/fd, which is why
//! `printf | cat /dev/stdin` hung. The equivalent of dup(N) here is a
//! concrete path: a bound file endpoint yields its real path, buffered or
//! inherited input is drained into a temp file, and a write-only std
//! endpoint becomes a temp file whose bytes flush into the endpoint after
//! the child exits.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::fd_table::{FdReadEndpoint, FdWriteEndpoint};
use super::redirect_target_fd;
use super::Executor;
use crate::parser::CommandNode;

/// How the child's fd 0 is wired when it does not come from the fd table
/// (pipeline stages receive a buffered payload or a live pipe reader).
pub(in crate::executor) enum DevOperandStdin<'a> {
    FdTable,
    /// fd 0 is this buffered payload (sequential pipeline stage input);
    /// consumed by the first fd-0 operand, like GNU's shared offset.
    Payload(Vec<u8>),
    /// fd 0 is a real file the caller already opened for the child.
    Path(PathBuf),
    /// fd 0 is a live pipeline reader destined for the child; an fd-0
    /// operand drains it into a temp file (upstream runs to EOF first,
    /// same bytes the child's dup'd fd 0 would have delivered).
    Reader(&'a mut dyn Read),
}

/// How the child's fd 1 is wired when it does not come from the fd table.
pub(in crate::executor) enum DevOperandStdout {
    FdTable,
    /// fd 1 is a real file the caller already opened for the child.
    Path(PathBuf),
    /// A duplicate of the pipeline writer handed to the child as fd 1;
    /// the operand temp flushes into it after the child exits.
    PipeWriter(crate::fd::HANDLE),
    /// The caller captures fd 1; flushed operand bytes are handed back.
    Capture,
}

enum DevOperandFlush {
    Stdout,
    Stderr,
    /// A borrowed endpoint handle (coproc stdin); written but not closed.
    Handle(crate::fd::HANDLE),
    /// A dup'd pipeline writer owned by this record; written then closed
    /// so the downstream stage sees EOF.
    OwnedHandle(crate::fd::HANDLE),
    Capture,
}

/// Deferred work from operand materialization: GNU's dup shares the fd's
/// offset, so reads by the child must advance the parent's endpoint —
/// modelled by consuming the fd after the child exits. Temp files are
/// removed and owned handles closed either way (Drop).
#[derive(Default)]
pub(in crate::executor) struct DevOperandMaterialization {
    temps: Vec<PathBuf>,
    consume_fds: Vec<u32>,
    flushes: Vec<(PathBuf, DevOperandFlush)>,
}

impl Drop for DevOperandMaterialization {
    fn drop(&mut self) {
        for temp in &self.temps {
            let _ = std::fs::remove_file(temp);
        }
        for (_, target) in &self.flushes {
            if let DevOperandFlush::OwnedHandle(handle) = target {
                crate::fd::close_handle(*handle);
            }
        }
    }
}

/// Whether `arg` is (or `KEY=` carries) a `/dev/std*` / `/dev/fd/N` /
/// `/proc/self/fd/N` operand naming `fd`.
pub(in crate::executor) fn dev_operand_targets_fd(arg: &str, fd: u32) -> bool {
    matches!(split_dev_operand(arg), Some((_, value)) if dev_operand_fd(value) == Some(fd))
}

fn split_dev_operand(arg: &str) -> Option<(&str, &str)> {
    if dev_operand_fd(arg).is_some() {
        return Some(("", arg));
    }
    // `dd of=/dev/stdout`, `tar --file=/dev/stdin` and similar carry the
    // operand after `=`. Option-looking keys (`--x`, lowercase dd-style
    // keys) rewrite; `FOO=/dev/stdin` environment assignments stay literal.
    let (key, value) = arg.split_once('=')?;
    dev_operand_fd(value)?;
    if key.starts_with('-')
        || key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        let split = key.len() + 1;
        Some((&arg[..split], &arg[split..]))
    } else {
        None
    }
}

/// The descriptor a `/dev/std*` / `/dev/fd/N` / `/proc/self/fd/N` operand
/// names. `pub(in crate::executor)` so in-process file builtins (emulated
/// `cat`, `sed`, ...) can resolve the same names against the fd table —
/// they have no child descriptor table, so the endpoint itself is read.
pub(in crate::executor) fn dev_operand_fd(value: &str) -> Option<u32> {
    if let Some(rest) = value.strip_prefix("/dev/") {
        return match rest {
            "stdin" => Some(0),
            "stdout" => Some(1),
            "stderr" => Some(2),
            _ => rest.strip_prefix("fd/")?.parse().ok(),
        };
    }
    let rest = value.strip_prefix("/proc/")?;
    let rest = rest.strip_prefix("self/").or_else(|| {
        let (pid, tail) = rest.split_once('/')?;
        (pid.parse::<u32>().ok()? == std::process::id()).then_some(tail)
    })?;
    rest.strip_prefix("fd/")?.parse().ok()
}

fn dev_operand_temp(bytes: &[u8], m: &mut DevOperandMaterialization) -> Option<String> {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash-devfd-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, bytes).ok()?;
    m.temps.push(path.clone());
    Some(path.to_string_lossy().into_owned())
}

fn drain_handle_to_temp(
    handle: crate::fd::HANDLE,
    m: &mut DevOperandMaterialization,
) -> Option<String> {
    let mut bytes = Vec::new();
    loop {
        match crate::fd::read_some(handle, 65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    dev_operand_temp(&bytes, m)
}

fn drain_reader_to_temp(
    reader: &mut dyn Read,
    m: &mut DevOperandMaterialization,
) -> Option<String> {
    let mut bytes = Vec::new();
    let _ = reader.read_to_end(&mut bytes);
    dev_operand_temp(&bytes, m)
}

impl Executor {
    /// Rewrite `/dev/std*`-family operand words in `args` to concrete paths.
    /// `stdin`/`stdout` describe how the caller wires the child's fd 0/1
    /// when the fd table does not own them (pipeline stages, capture
    /// pipes). The returned record must be finished after the child exits
    /// via `finish_dev_fd_operands`.
    pub(in crate::executor) fn materialize_dev_fd_operands(
        &self,
        args: &[String],
        mut stdin: DevOperandStdin<'_>,
        stdout: DevOperandStdout,
    ) -> (Vec<String>, DevOperandMaterialization) {
        let mut m = DevOperandMaterialization::default();
        // POSIX children resolve these names through their own /dev/fd
        // emulation; only Windows children need the materialized endpoint.
        if !cfg!(windows) {
            return (args.to_vec(), m);
        }
        let rewritten = args
            .iter()
            .map(|arg| {
                let Some((prefix, value)) = split_dev_operand(arg) else {
                    return arg.clone();
                };
                let Some(fd) = dev_operand_fd(value) else {
                    return arg.clone();
                };
                let path = match fd {
                    0 => match &mut stdin {
                        DevOperandStdin::Payload(bytes) => {
                            dev_operand_temp(&std::mem::take(bytes), &mut m)
                        }
                        DevOperandStdin::Path(path) => Some(path.to_string_lossy().into_owned()),
                        DevOperandStdin::Reader(reader) => drain_reader_to_temp(*reader, &mut m),
                        DevOperandStdin::FdTable => self.dev_fd_table_operand(fd, &mut m),
                    },
                    1 => match &stdout {
                        DevOperandStdout::Path(path) => Some(path.to_string_lossy().into_owned()),
                        DevOperandStdout::PipeWriter(handle) => {
                            dev_operand_temp(&[], &mut m).map(|temp| {
                                m.flushes.push((
                                    PathBuf::from(&temp),
                                    DevOperandFlush::OwnedHandle(*handle),
                                ));
                                temp
                            })
                        }
                        DevOperandStdout::Capture => dev_operand_temp(&[], &mut m).map(|temp| {
                            m.flushes
                                .push((PathBuf::from(&temp), DevOperandFlush::Capture));
                            temp
                        }),
                        DevOperandStdout::FdTable => self.dev_fd_table_operand(fd, &mut m),
                    },
                    _ => self.dev_fd_table_operand(fd, &mut m),
                };
                match path {
                    Some(path) => format!("{prefix}{path}"),
                    None => arg.clone(),
                }
            })
            .collect();
        (rewritten, m)
    }

    /// Resolve a `/dev/fd/N` operand through the fd table: read endpoints
    /// hand the child the fd's remaining input, write endpoints a real
    /// path or a temp file flushed into the endpoint after exit.
    fn dev_fd_table_operand(&self, fd: u32, m: &mut DevOperandMaterialization) -> Option<String> {
        let entry = self.fd_table.entries.get(&fd).filter(|e| !e.closed)?;
        if let Some(read) = &entry.read {
            return match read {
                FdReadEndpoint::Text(_) | FdReadEndpoint::ProcessSubstitution(_) => {
                    let bytes = self
                        .fd_table
                        .input_snapshot_bytes(fd)
                        .map(|(data, offset)| data[offset..].to_vec())
                        .unwrap_or_default();
                    m.consume_fds.push(fd);
                    dev_operand_temp(&bytes, m)
                }
                FdReadEndpoint::File(file) => {
                    m.consume_fds.push(fd);
                    Some(file.path.to_string_lossy().into_owned())
                }
                FdReadEndpoint::InheritedProcessStdin => {
                    drain_handle_to_temp(crate::fd::process_std_handle(0), m)
                }
                FdReadEndpoint::CoprocStdout { fd: file, .. } => {
                    drain_handle_to_temp(file.handle, m)
                }
            };
        }
        match entry.write.as_ref()? {
            FdWriteEndpoint::File(file) => Some(file.path.to_string_lossy().into_owned()),
            FdWriteEndpoint::Stdout => {
                let temp = dev_operand_temp(&[], m)?;
                m.flushes
                    .push((PathBuf::from(&temp), DevOperandFlush::Stdout));
                Some(temp)
            }
            FdWriteEndpoint::Stderr => {
                let temp = dev_operand_temp(&[], m)?;
                m.flushes
                    .push((PathBuf::from(&temp), DevOperandFlush::Stderr));
                Some(temp)
            }
            FdWriteEndpoint::CoprocStdin { fd: file, .. } => {
                let temp = dev_operand_temp(&[], m)?;
                m.flushes
                    .push((PathBuf::from(&temp), DevOperandFlush::Handle(file.handle)));
                Some(temp)
            }
            FdWriteEndpoint::ProcessSubstitution { .. } => None,
        }
    }

    /// Apply deferred fd semantics once the child has exited: flush operand
    /// temps into their write endpoints, advance consumed read endpoints
    /// (GNU's dup shares the file offset), and let Drop remove the temps
    /// and close dup'd writers. Bytes collected for a `Capture` fd-1
    /// context are returned for the caller to append to captured output.
    pub(in crate::executor) fn finish_dev_fd_operands(
        &mut self,
        mut m: DevOperandMaterialization,
    ) -> Vec<u8> {
        let mut captured = Vec::new();
        for (temp, target) in &m.flushes {
            let Ok(bytes) = std::fs::read(temp) else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            match target {
                DevOperandFlush::Stdout => {
                    let _ = self.write_default_stdout(&bytes);
                }
                DevOperandFlush::Stderr => {
                    let _ = self.write_default_stderr(&bytes);
                }
                DevOperandFlush::Handle(handle) | DevOperandFlush::OwnedHandle(handle) => {
                    let _ = crate::fd::write_all(*handle, &bytes);
                }
                DevOperandFlush::Capture => captured.extend_from_slice(&bytes),
            }
        }
        for fd in &m.consume_fds {
            self.fd_table.consume_all_text(*fd);
        }
        captured
    }

    /// Resolve a `/dev/fd/N`-family operand the way command `cmd` would see
    /// descriptor `fd`: GNU applies the command's own redirections first
    /// (redir.c do_redirections — the descriptor space the child opens
    /// `/dev/fd/N` in already has them), so `cat /dev/fd/3 3<<<x` reads the
    /// here-string bound to fd 3. Numbered `N<file`/`N<&M` redirects on the
    /// command resolve likewise before falling back to the shell fd table.
    pub(in crate::executor) fn dev_fd_operand_bytes_for_command(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Option<Vec<u8>> {
        if let Some(input) = self.external_fd_heredoc_input(cmd, fd) {
            // The stored body may still carry the STORAGE_WORD_PREFIX
            // quoting marker; fd bytes are the dequoted content.
            let input = input.strip_prefix('\u{1d}').unwrap_or(&input);
            return Some(crate::executor::substitution_metadata::shell_text_to_raw_bytes(input));
        }
        let mut fd = fd;
        let mut seen = vec![fd];
        loop {
            // The last redirect bound to this descriptor wins
            // (do_redirections runs them left to right).
            let Some(redirect) = cmd
                .redirects
                .iter()
                .rev()
                .find(|redirect| redirect.fd == Some(fd) && redirect.fd_var.is_none())
            else {
                break;
            };
            match redirect.kind {
                crate::parser::RedirectKind::DuplicateInput
                | crate::parser::RedirectKind::DuplicateOutput => {
                    let target = self.expand_redirect_target(redirect);
                    let Some(source) = redirect_target_fd(&target) else {
                        return None;
                    };
                    if seen.contains(&source) {
                        return None;
                    }
                    seen.push(source);
                    fd = source;
                }
                crate::parser::RedirectKind::CloseInput
                | crate::parser::RedirectKind::CloseOutput => return None,
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString => {
                    break
                }
                _ => {
                    // Input/Output/Append/ReadWrite and friends all leave fd
                    // bound to the target file — GNU's /dev/fd/N symlink
                    // reopens that file, so read it.
                    let target = self.expand_redirect_target(redirect);
                    return std::fs::read(crate::executor::shell_path_to_windows(
                        &target,
                        &self.shell_state.env_vars,
                    ))
                    .ok();
                }
            }
        }
        self.dev_fd_operand_bytes(fd)
    }

    /// Read a `/dev/fd/N`-family operand for an in-process file builtin
    /// (emulated cat/sed have no child descriptor table — GNU hands the
    /// child a dup of the descriptor, so the in-process equivalent is the
    /// fd endpoint's remaining bytes, consuming the shared offset).
    /// `None` for an unbound or write-only-pipe fd — callers report ENOENT.
    pub(in crate::executor) fn dev_fd_operand_bytes(&mut self, fd: u32) -> Option<Vec<u8>> {
        let entry = self.fd_table.entries.get(&fd)?.clone();
        if entry.closed {
            return None;
        }
        if let Some(read) = &entry.read {
            if let Some(bytes) = self.fd_table.read_all_bytes(fd) {
                return Some(bytes);
            }
            // InheritedProcessStdin has no table-owned bytes: drain the
            // process's real fd 0 (GNU's dup shares it, so reading it here
            // advances the parent's offset the same way).
            return Some(match read {
                FdReadEndpoint::InheritedProcessStdin => {
                    let mut bytes = Vec::new();
                    let handle = crate::fd::process_std_handle(0);
                    loop {
                        match crate::fd::read_some(handle, 65536) {
                            Ok(chunk) if chunk.is_empty() => break,
                            Ok(chunk) => bytes.extend_from_slice(&chunk),
                            Err(_) => break,
                        }
                    }
                    bytes
                }
                _ => Vec::new(),
            });
        }
        match entry.write.as_ref()? {
            FdWriteEndpoint::File(file) => std::fs::read(&file.path).ok(),
            // Reading the process's own stdout/stderr handle has no
            // portable answer; GNU would block on a tty/pipe. Empty keeps
            // the operand well-defined without hanging.
            FdWriteEndpoint::Stdout | FdWriteEndpoint::Stderr => Some(Vec::new()),
            FdWriteEndpoint::CoprocStdin { .. } | FdWriteEndpoint::ProcessSubstitution { .. } => {
                None
            }
        }
    }

    /// `&self` variant for command-substitution fast paths (the
    /// command_substitution chain is `&self` end to end): collects the
    /// `Capture` flush bytes for the caller to append to the child's
    /// captured stdout. fd-0 consumption is covered separately by the
    /// caller's FUNCTION_STDIN writeback, and temp/handle cleanup still
    /// runs through Drop.
    pub(in crate::executor) fn collect_dev_fd_capture(
        &self,
        m: &DevOperandMaterialization,
    ) -> Vec<u8> {
        let mut captured = Vec::new();
        for (temp, target) in &m.flushes {
            if !matches!(target, DevOperandFlush::Capture) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(temp) {
                captured.extend_from_slice(&bytes);
            }
        }
        captured
    }
}
