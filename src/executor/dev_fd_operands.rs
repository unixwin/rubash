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
use super::Executor;
use super::{is_closed_redirect_target, redirect_target_fd};
use crate::parser::{CommandNode, Redirect, RedirectKind};

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

/// The resolved content of a `/dev/fd/N`-family operand together with the
/// flag GNU cat.c needs: whether the operand lands on the same file the
/// command's fd 1 writes to ("input file is output file").
pub(in crate::executor) struct DevFdOperandRead {
    pub bytes: Vec<u8>,
    pub stdout_file: bool,
}

/// The redirect a command applies to descriptor `fd` — the ordered
/// `redirects` list carries every redirection (a `None` fd means the
/// kind's default descriptor), while `2>` is kept only in `redirect_err`.
/// The list wins on overlap because it preserves the do_redirections
/// left-to-right order the dedicated fields flatten.
fn command_redirect_for_fd<'c>(cmd: &'c CommandNode, fd: u32) -> Option<&'c Redirect> {
    cmd.redirects
        .iter()
        .rev()
        .find(|redirect| redirect.fd_var.is_none() && redirect_binds_fd(redirect, fd))
        .or(match fd {
            0 => cmd.redirect_in.as_ref(),
            1 => cmd.redirect_out.as_ref().or(cmd.append.as_ref()),
            2 => cmd
                .redirect_err
                .as_ref()
                .or(cmd.redirect_err_append.as_ref()),
            _ => None,
        })
}

/// Whether `redirect` binds descriptor `fd`: an explicit `N` matches
/// directly; `fd: None` means the kind's default (input-ish kinds bind
/// fd 0, output kinds fd 1, `&>`/`&>>` both fd 1 and fd 2).
fn redirect_binds_fd(redirect: &Redirect, fd: u32) -> bool {
    match redirect.fd {
        Some(n) => n == fd,
        None => match redirect.kind {
            RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::DuplicateInput
            | RedirectKind::CloseInput
            | RedirectKind::HereDoc
            | RedirectKind::HereString => fd == 0,
            RedirectKind::CombinedOutput | RedirectKind::CombinedAppend => fd == 1 || fd == 2,
            _ => fd == 1,
        },
    }
}

/// Input-side redirect kinds fail the operand lookup when their target is
/// missing (GNU aborts the command on the failed redirection); output
/// kinds create their target, so a missing file reads as empty.
fn redirect_input_kind(kind: &RedirectKind) -> bool {
    matches!(
        kind,
        RedirectKind::Input | RedirectKind::ReadWrite | RedirectKind::DuplicateInput
    )
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
    /// here-string bound to fd 3. Unnumbered redirects (`<`, `>`,
    /// `<<<`) land in `cmd.redirects` with `fd: None` (the kind's default
    /// descriptor) and `2>` lives only in `redirect_err`, so the lookup
    /// consults both stores. Numbered `N<file`/`N<&M` chains resolve
    /// likewise before falling back to the shell fd table.
    pub(in crate::executor) fn dev_fd_operand_bytes_for_command(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Option<DevFdOperandRead> {
        if let Some(input) = self.external_fd_heredoc_input(cmd, fd) {
            // The stored body may still carry the STORAGE_WORD_PREFIX
            // quoting marker; fd bytes are the dequoted content.
            let input = input.strip_prefix('\u{1d}').unwrap_or(&input);
            return Some(DevFdOperandRead {
                bytes: crate::executor::substitution_metadata::shell_text_to_raw_bytes(input),
                stdout_file: false,
            });
        }
        let stdout_target = self.command_stdout_file_target(cmd);
        let mut fd = fd;
        let mut seen = vec![fd];
        loop {
            // The last redirect bound to this descriptor wins
            // (do_redirections runs them left to right).
            let Some(redirect) = command_redirect_for_fd(cmd, fd) else {
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
                    if fd == 0 {
                        // fd-0 here-strings/heredocs live in here_string/
                        // heredoc/heredoc_redirects (fd=None), not under
                        // Some(0) — resolve through the same last-wins
                        // stdin resolver the fd-0 binding itself uses
                        // (it appends the `<<<` newline, shell-input-file).
                        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
                            return Some(DevFdOperandRead {
                                bytes: input.into_bytes(),
                                stdout_file: false,
                            });
                        }
                    }
                    break;
                }
                _ => {
                    // Input/Output/Append/ReadWrite and friends all leave fd
                    // bound to the target file — GNU's /dev/fd/N symlink
                    // reopens that file, so read it.
                    let target = self.expand_redirect_target(redirect);
                    if let Some(source) = redirect_target_fd(&target) {
                        // `2>&1`-style dup words stored under a file kind.
                        if seen.contains(&source) {
                            return None;
                        }
                        seen.push(source);
                        fd = source;
                        continue;
                    }
                    if is_closed_redirect_target(&target) {
                        return None;
                    }
                    let path =
                        crate::executor::shell_path_to_windows(&target, &self.shell_state.env_vars);
                    let stdout_file = stdout_target
                        .as_deref()
                        .is_some_and(|out| out == path.as_path());
                    return Some(DevFdOperandRead {
                        bytes: match std::fs::read(&path) {
                            Ok(bytes) => bytes,
                            // An output redirect has already created (or
                            // would create) its target — GNU reads an empty
                            // file rather than failing the lookup.
                            Err(_) if !redirect_input_kind(&redirect.kind) => Vec::new(),
                            Err(_) => return None,
                        },
                        stdout_file,
                    });
                }
            }
        }
        let bytes = self.dev_fd_operand_bytes(fd)?;
        // fd-table fallback: the operand lands on fd 1's File endpoint when
        // the descriptor space binds fd 1 there (e.g. `exec >file`).
        let stdout_file = stdout_target.is_some() && fd == 1;
        Some(DevFdOperandRead { bytes, stdout_file })
    }

    /// The file path the command's own redirections bind fd 1 to, or the
    /// fd table's fd-1 File endpoint when the command does not redirect
    /// stdout itself — GNU cat.c's "input file is output file" check needs
    /// it to compare an fd operand's resolved file against stdout.
    fn command_stdout_file_target(&mut self, cmd: &CommandNode) -> Option<PathBuf> {
        let mut fd = 1u32;
        let mut seen = vec![fd];
        loop {
            let Some(redirect) = command_redirect_for_fd(cmd, fd) else {
                break;
            };
            match redirect.kind {
                crate::parser::RedirectKind::DuplicateInput
                | crate::parser::RedirectKind::DuplicateOutput => {
                    let target = self.expand_redirect_target(redirect);
                    let source = redirect_target_fd(&target)?;
                    if seen.contains(&source) {
                        return None;
                    }
                    seen.push(source);
                    fd = source;
                }
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString => {
                    return None
                }
                _ => {
                    let target = self.expand_redirect_target(redirect);
                    if let Some(source) = redirect_target_fd(&target) {
                        if seen.contains(&source) {
                            return None;
                        }
                        seen.push(source);
                        fd = source;
                        continue;
                    }
                    if is_closed_redirect_target(&target) {
                        return None;
                    }
                    return Some(crate::executor::shell_path_to_windows(
                        &target,
                        &self.shell_state.env_vars,
                    ));
                }
            }
        }
        match self.fd_table.write_endpoint(fd)? {
            FdWriteEndpoint::File(file) => Some(file.path.clone()),
            // An inherited stdout/stderr bound to a disk file is still the
            // output FILE for cat.c's same-inode check — resolve the OS
            // handle's path so `niu ... >> seeded` reports like GNU.
            FdWriteEndpoint::Stdout => crate::fd::disk_file_path(crate::fd::process_std_handle(1)),
            FdWriteEndpoint::Stderr => crate::fd::disk_file_path(crate::fd::process_std_handle(2)),
            _ => None,
        }
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
            // GNU /dev/stdout is open("/proc/self/fd/1", O_RDONLY): a FRESH
            // open of the descriptor's target. When the inherited
            // stdout/stderr OS handle is a disk file, reopening reads its
            // bytes from offset 0. For pipes/consoles GNU's read blocks
            // (the reopened read end waits on the same pipe, or the tty
            // waits on the console) — empty keeps the operand
            // well-defined without hanging.
            FdWriteEndpoint::Stdout | FdWriteEndpoint::Stderr => {
                let os_fd = if matches!(entry.write, Some(FdWriteEndpoint::Stdout)) {
                    1
                } else {
                    2
                };
                match crate::fd::disk_file_path(crate::fd::process_std_handle(os_fd)) {
                    Some(path) => std::fs::read(path).ok(),
                    None => Some(Vec::new()),
                }
            }
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
