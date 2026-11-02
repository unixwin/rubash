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

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::fd_table::{FdReadEndpoint, FdWriteEndpoint, FileFd};
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

/// rubash#2473: the machine-level proof key that re-arms the #1903
/// fifo-marker rendezvous for procsub argv operands (see
/// `procsub_operand_delivery`). "1" asserts the installed WinuxCmd
/// runtime bridges `!<fifo>` markers on its applet operand READ path —
/// WinuxCmd 1.1.20 does not (paste/wc/od/comm read the 7 marker bytes
/// plainly; their own `od` does not rendezvous with their own `mkfifo`
/// marker — unixwin/WinuxCmd#1493), so the default is the byte-correct
/// drain for every consumer.
const PROCSUB_FIFO_MARKER_PROOF_ENV: &str = "RUBASH_PROCSUB_FIFO_MARKER";

enum DevOperandFlush {
    Stdout,
    Stderr,
    /// A borrowed endpoint handle (coproc stdin); written but not closed.
    Handle(crate::fd::HANDLE),
    /// A dup'd pipeline writer owned by this record; written then closed
    /// so the downstream stage sees EOF.
    OwnedHandle(crate::fd::HANDLE),
    Capture,
    /// rubash#2385: a complementary channel end — the operand temp's
    /// bytes append to the shared queue at finish, the same deferred
    /// delivery the Stdout/Stderr arms give their streams.
    Channel(crate::executor::fd_table::PipeChannelEnd),
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
    /// Unix: parent-held pipe read ends to pin at their `/dev/fd/N` numbers
    /// in the child between fork and exec (see materialize_dev_fd_operands).
    /// Closed when the record drops, like GNU closing the parent's pipe end
    /// once the forked child owns its copy.
    #[cfg(unix)]
    pub(in crate::executor) unix_fd_attach: Vec<(u32, std::os::fd::RawFd)>,
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
        #[cfg(unix)]
        for (_, source) in &self.unix_fd_attach {
            crate::fd::close_handle(*source);
        }
    }
}

impl DevOperandMaterialization {
    /// rubash#1497: deferred flushes copy buffered bytes back to the
    /// real target after the child exits — a process-replacing exec
    /// has no "after", so the spawn site must refuse exec replacement
    /// while any flush is pending.
    pub(in crate::executor) fn has_deferred_flushes(&self) -> bool {
        !self.flushes.is_empty()
    }
}

#[cfg(unix)]
/// Pin every recorded `/dev/fd/N` pipe read end at its own descriptor number
/// in the child between fork and exec (std Command pre_exec): the child then
/// resolves the literal argv word through the OS /dev/fd layer exactly as a
/// GNU-bash forked child does (execute_cmd.c hands argv to execve unchanged;
/// the descriptor's existence is the whole contract). Runs in the child —
/// dup2/close/fcntl only.
pub(in crate::executor) fn attach_unix_dev_fd_operands(
    process: &mut std::process::Command,
    m: &DevOperandMaterialization,
) {
    if m.unix_fd_attach.is_empty() {
        return;
    }
    use std::os::unix::process::CommandExt;
    let attach = m.unix_fd_attach.clone();
    unsafe {
        process.pre_exec(move || {
            for &(number, source) in attach.iter() {
                let number = number as libc::c_int;
                // A fresh pipe fd can land on the target number itself in
                // the parent; dup2(fd, fd) is a no-op that does NOT clear
                // FD_CLOEXEC, so exec would close the descriptor the argv
                // word names — clear the flag explicitly instead.
                if source == number {
                    let flags = libc::fcntl(source, libc::F_GETFD);
                    if flags < 0
                        || libc::fcntl(source, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                } else if libc::dup2(source, number) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

/// rubash#2484: park an attach source above every descriptor number
/// the spawn machinery can target, BEFORE the record crosses to the
/// spawn site. The recorded source is a raw descriptor number in THIS
/// process, and the child's pin pass (`pin_ambient_fds_in_child`,
/// registered ahead of the attach closure at every spawn site) dup2's
/// the shell's ambient bindings onto their fd numbers in the child
/// first. A source recorded at a number inside that range is
/// overwritten before the attach pass reads it — systematically the
/// bats_tee shape: the shell's fd 3 is virtual (no OS descriptor), so
/// a carrier handle opened in the parent lands on the parent's lowest
/// free number, 3, and the child's fd-3 pin (the stage pipe) clobbers
/// it; the attach then copies the PIPE onto the operand number and
/// the consumer writes into the pipe instead of the carrier.
/// F_DUPFD_CLOEXEC to the lowest free number above the fd table's
/// range makes the source unreachable by any pin target (pin targets
/// are fd-table numbers) and by pin staging (which only takes FREE
/// numbers, and this one is occupied). A failed park keeps the
/// original handle — the pre-#2484 behavior — rather than failing
/// the spawn.
#[cfg(unix)]
fn park_attach_source(handle: crate::fd::HANDLE, floor: libc::c_int) -> crate::fd::HANDLE {
    let parked = unsafe { libc::fcntl(handle as libc::c_int, libc::F_DUPFD_CLOEXEC, floor) };
    if parked >= 0 {
        crate::fd::close_handle(handle);
        parked as crate::fd::HANDLE
    } else {
        handle
    }
}

/// Whether `arg` is (or `KEY=` carries) a `/dev/std*` / `/dev/fd/N` /
/// `/proc/self/fd/N` operand naming `fd`.
pub(in crate::executor) fn dev_operand_targets_fd(arg: &str, fd: u32) -> bool {
    matches!(split_dev_operand(arg), Some((_, value)) if dev_operand_fd(value) == Some(fd))
}

/// The operand word-shape discipline shared by the operand passes
/// (the dev-fd pass here, the synthetic-/proc pass in proc_operands):
/// a word carries an operand when it IS the operand spelling, or when
/// it carries it after `=` under an option-looking key. `value_ok`
/// judges whether a (whole or post-`=`) word is the pass's operand
/// spelling.
pub(in crate::executor) fn split_keyed_operand<'a>(
    arg: &'a str,
    value_ok: impl Fn(&str) -> bool,
) -> Option<(&'a str, &'a str)> {
    if value_ok(arg) {
        return Some(("", arg));
    }
    // `dd of=/dev/stdout`, `tar --file=/dev/stdin` and similar carry the
    // operand after `=`. Option-looking keys (`--x`, lowercase dd-style
    // keys) rewrite; `FOO=/dev/stdin` environment assignments stay literal.
    let (key, value) = arg.split_once('=')?;
    if !value_ok(value) {
        return None;
    }
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

fn split_dev_operand(arg: &str) -> Option<(&str, &str)> {
    split_keyed_operand(arg, |value| dev_operand_fd(value).is_some())
}

/// The descriptor a `/dev/std*` / `/dev/fd/N` / `/proc/self/fd/N` operand
/// names. `pub(in crate::executor)` so in-process file builtins (emulated
/// `cat`, `sed`, ...) can resolve the same names against the fd table —
/// they have no child descriptor table, so the endpoint itself is read.
///
/// The spelling judgment itself is the registry's
/// (`special_paths::fd_alias_target`): this face used to re-parse the
/// tail with `str::parse`, which also accepted `/dev/fd/+3` — a spelling
/// GNU rejects on every face (the open fails ENOENT) — so the two
/// answers drifted apart (redundancy audit R3/D1). The registry also
/// owns the own-pid spelling (rubash#1974: `/proc/<pid>/fd/N` with the
/// shell's own pid is the same alias, exactly as procfs serves it);
/// this face previously carried the only private copy of that rewrite.
/// Foreign pids stay plain filesystem paths.
pub(in crate::executor) fn dev_operand_fd(value: &str) -> Option<u32> {
    crate::special_paths::fd_alias_target(value)
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
            0 => cmd.redirect_in.as_deref(),
            1 => cmd.redirect_out.as_deref().or(cmd.append.as_deref()),
            2 => cmd
                .redirect_err
                .as_deref()
                .or(cmd.redirect_err_append.as_deref()),
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

/// How a procsub read end delivered to an external child as an argv word
/// reaches that child (rubash#2225 gate on the #1959 rendezvous).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProcsubOperandDelivery {
    /// The #1903 fifo-marker rendezvous: the word names an on-disk
    /// `!<fifo>` marker and the child bridges it to the live pipe.
    /// Streaming — but only a consumer whose own runtime bridges
    /// markers ever sees the producer's bytes through it.
    FifoMarker,
    /// The parent drains the producer into a temp file before spawn
    /// (the pre-#2156 general shape). Byte-correct for every consumer;
    /// the consumer starts only after the producer exits.
    DrainToTemp,
}

/// rubash#2225: choose the delivery for a procsub argv operand from the
/// CONSUMER's class. #2156 sent every argv-word procsub through the
/// fifo-marker rendezvous; a consumer that is not marker-aware (Windows'
/// own findstr.exe is the niu#720/#723 shape) opens the marker path with
/// a plain CreateFile and reads the 7 marker bytes instead of the
/// producer's stream — GNU subst.c:6362 process_substitute names a live
/// pipe whose bytes ANY consumer opening the name must receive, so
/// streaming may never change the bytes a non-aware consumer reads.
///
/// rubash#2473: the aware CLASS is no longer PROOF of bridging. Every
/// `<(...)` occurrence is its own substitution — expand_word_internal
/// calls process_substitute per occurrence (subst.c:11364-11385), each
/// with its own pipe (subst.c:6381) under its own /dev/fd name
/// (move_to_high_fd + make_dev_fd_filename, subst.c:6392-6394) — and the
/// contract for every one of them is the same: whatever consumer opens
/// the name receives the producer's bytes. Identity-based awareness
/// ("the binary IS the WinuxCmd runtime") predicted that the runtime's
/// applet reads bridge `!<fifo>` markers; measured against WinuxCmd
/// 1.1.20 the prediction is false on the READ side for the applets that
/// matter (`paste <(echo 1) <(echo x)` printed `!<fifo>\t!<fifo>`;
/// `wc -c <(printf hello)` counted the 7 marker bytes; od/comm the
/// same), and WinuxCmd's own runtime does not rendezvous with its own
/// mkfifo marker (its `od` reads the marker bytes plainly while the
/// writer blocks in the pipe open). A wrong prediction here is silent
/// byte corruption in the operand, so the marker rendezvous now
/// requires an explicit machine-level proof: `RUBASH_PROCSUB_FIFO_MARKER=1`
/// asserts "the installed WinuxCmd runtime bridges `!<fifo>` markers on
/// its applet operand read path" (the WinuxCmd-side read-open gap is
/// unixwin/WinuxCmd#1493; a version probe can replace the manual key
/// once a bridging release exists). Without the key every consumer —
/// including the WinuxCmd class — takes the byte-correct drain; the
/// failure direction costs the #1959 streaming win, never bytes,
/// exactly the #2225 safe direction.
fn procsub_operand_delivery(
    consumer_program: Option<&Path>,
    env_vars: &HashMap<String, String>,
) -> ProcsubOperandDelivery {
    #[cfg(windows)]
    {
        if env_vars.get(PROCSUB_FIFO_MARKER_PROOF_ENV) == Some(&"1".to_string()) {
            if let Some(program) = consumer_program {
                if super::path::winuxcmd_fifo_marker_aware(program, env_vars) {
                    return ProcsubOperandDelivery::FifoMarker;
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = consumer_program;
        let _ = env_vars;
    }
    ProcsubOperandDelivery::DrainToTemp
}

/// rubash#1959: the Windows rendezvous for a procsub read end delivered
/// as an argv word to an external child (`head -1 <(body)`, `$(cat
/// <(body))`, and the pipeline word form alike — every spawn site funnels
/// through `dev_fd_table_operand`). The anonymous pipe's read end has no
/// openable path and the child cannot inherit numbered handles into argv
/// names, so the face used to drain the producer to EOF into a temp file
/// before spawn — the consumer started only after the producer exited.
/// GNU's contract (subst.c:6362 process_substitute) is a live pipe under
/// the operand name: the child reads while the producer still runs.
///
/// The endpoint is the #1903 WinuxCmd FIFO marker rendezvous (the same
/// design piece the issue names): the argv word carries a path to an
/// on-disk `!<fifo>` marker whose reader opens bridge to a deterministic
/// named pipe. The external child's own runtime (WinuxCmd
/// file_io.cppm open_binary_file → open_fifo_read_handle) creates the
/// INBOUND server for the marker and blocks in ConnectNamedPipe; the
/// pump thread below is the WRITER client (`open_winuxcmd_fifo_write`,
/// GNU's blocking O_WRONLY fifo open), copying the procsub read end
/// into the pipe byte-wise. A consumer that is not marker-aware reads
/// the 7 marker bytes — the same degradation every WinuxCmd fifo shows
/// a non-protocol consumer. rubash#2225: for procsub argv words that
/// degradation is NOT acceptable (GNU's contract is the producer's
/// bytes for any consumer), so this rendezvous is only reached when
/// `procsub_operand_delivery` has classified the consumer as
/// marker-aware; every other consumer takes the drain in
/// `dev_fd_table_operand` and never sees a marker path.
///
/// Ownership and lifetime: the pump owns a DuplicateHandle reference of
/// the read end, so fd-table cleanup after the child exits (consume's
/// seek-to-end, the command-scoped close of the procsub fd, FileFd drop)
/// cannot race its reads. Closing the writer client is the child's EOF;
/// a consumer gone early (GNU's SIGPIPE shape, `head -1`) fails the
/// pump's writes and tears the pump down, which breaks the producer's
/// pipe the same way. If no marker-aware reader ever opens the path, the
/// pump parks in the write open and the producer parks on its full pipe
/// — the same running-producer shape GNU leaves behind for an
/// unconsumed substitution; nothing in the parent blocks on either. The
/// marker file itself is removed with the materialization record (after
/// the child exits, like every operand temp).
#[cfg(windows)]
fn procsub_fifo_marker_operand(file: &FileFd, m: &mut DevOperandMaterialization) -> Option<String> {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash-procsub-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if crate::winuxcmd_fifo::create_winuxcmd_fifo(&path, false).is_err() {
        return None;
    }
    let source = match crate::fd::duplicate_handle(file.handle) {
        Ok(source) => source,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return None;
        }
    };
    let pump_path = path.clone();
    std::thread::spawn(move || {
        // Blocks until the child's reader creates the endpoint (and on
        // ERROR_PIPE_BUSY waits its slice) — no parent-side thread is
        // involved in that wait, so a consumer that never arrives costs
        // only this parked thread, never a shell stall.
        match crate::winuxcmd_fifo::open_winuxcmd_fifo_write(&pump_path, &|| false) {
            Ok(sink) => {
                use std::os::windows::io::IntoRawHandle;
                let sink = sink.into_raw_handle() as crate::fd::HANDLE;
                loop {
                    match crate::fd::read_some(source, 65536) {
                        Ok(chunk) if chunk.is_empty() => break,
                        Ok(chunk) => {
                            if crate::fd::write_all(sink, &chunk).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                crate::fd::close_handle(sink);
            }
            Err(_) => {}
        }
        crate::fd::close_handle(source);
    });
    m.temps.push(path.clone());
    Some(path.to_string_lossy().into_owned())
}

impl Executor {
    /// Rewrite `/dev/std*`-family operand words in `args` to concrete paths.
    /// `stdin`/`stdout` describe how the caller wires the child's fd 0/1
    /// when the fd table does not own them (pipeline stages, capture
    /// pipes). `consumer_program` is the spawn site's RESOLVED program
    /// path (None when the site cannot offer one — internal pipeline
    /// stages, an unresolvable exec target): rubash#2225's gate chooses
    /// each procsub argv operand's delivery from the consumer's class.
    /// The returned record must be finished after the child exits
    /// via `finish_dev_fd_operands`.
    pub(in crate::executor) fn materialize_dev_fd_operands(
        &self,
        args: &[String],
        mut stdin: DevOperandStdin<'_>,
        stdout: DevOperandStdout,
        consumer_program: Option<&Path>,
    ) -> (Vec<String>, DevOperandMaterialization) {
        let mut m = DevOperandMaterialization::default();
        if !cfg!(windows) {
            // POSIX children resolve the literal `/dev/fd/N` word through
            // their own /dev/fd layer once descriptor N exists at exec time
            // (GNU execute_cmd.c passes argv to execve unchanged). rubash's
            // virtual fd table is not the child's descriptor table, though —
            // a buffered substitution endpoint has NO real descriptor, which
            // is why `diff <(echo a) <(echo a)` failed ENOENT (rubash#438).
            // Back each virtual read operand with a real anonymous pipe
            // carrying the endpoint's remaining bytes and pin the read end
            // at fd N in the child (attach_unix_dev_fd_operands, called by
            // the spawn paths with this record). File-backed endpoints keep
            // the literal word: the OS resolves it natively.
            // rubash#2484: every handle recorded for the child attach
            // is parked above the fd table's number range (see
            // park_attach_source) so the child's pin pass cannot
            // overwrite a source before the attach pass reads it.
            #[cfg(unix)]
            let attach_floor = self
                .fd_table
                .entries
                .keys()
                .max()
                .copied()
                .unwrap_or(0)
                .max(63) as libc::c_int
                + 1;
            #[cfg(unix)]
            for arg in args {
                let Some((_prefix, value)) = split_dev_operand(arg) else {
                    continue;
                };
                let Some(fd) = dev_operand_fd(value) else {
                    continue;
                };
                let Some(entry) = self.fd_table.entries.get(&fd).filter(|e| !e.closed) else {
                    continue;
                };
                // rubash#1952: a File read endpoint (a streamed
                // procsub's pipe, an exec-bound file) is a real
                // descriptor in THIS process only — the child does
                // not inherit it by default, so pin a dup at the
                // operand's number like the buffered arms below.
                // The descriptor is shared, not consumed: the child
                // reads advance the same offset, as under GNU.
                // rubash#2171: pin a DUPLICATE of the handle, never
                // the endpoint's own — the record owns what it
                // attaches and its Drop closes every recorded handle
                // once the child exits, so recording the parent's
                // live handle closed the parent's FileFd with the
                // record (a mere `ls -l /proc/self/fd/0` inspection
                // left the parent's fd dead; later reads failed
                // EBADF where GNU keeps the descriptor — EISDIR for
                // a directory binding). A dup shares the open file
                // description, offset included, so the child's view
                // is unchanged.
                if let Some(FdReadEndpoint::File(file)) = &entry.read {
                    if let Ok(dup) = crate::fd::duplicate_handle(file.handle) {
                        m.unix_fd_attach
                            .push((fd, park_attach_source(dup, attach_floor)));
                    }
                    continue;
                }
                let buffered = match &entry.read {
                    Some(FdReadEndpoint::Text(_) | FdReadEndpoint::ProcessSubstitution(_)) => self
                        .fd_table
                        .input_snapshot_bytes(fd)
                        .map(|(data, offset)| data[offset..].to_vec())
                        .unwrap_or_default(),
                    _ => {
                        // Output process-substitution endpoints
                        // (`>(...)` spelled as an operand word,
                        // rubash#1460): the endpoint is backed by a real
                        // temp file the shell feeds to the body after the
                        // command finishes, but the child only receives
                        // the `/dev/fd/N` spelling — with no descriptor N
                        // in the child, opening it fails ENOENT
                        // (`tee >(cat > out)` wrote nothing). Open the
                        // backing file here and pin it at fd N in the
                        // child through the same attach machinery as the
                        // read side; the child's opens of /dev/fd/N then
                        // land on the file and the post-command drain
                        // sees the bytes. GNU subst.c:6362
                        // process_substitute hands the child the real
                        // pipe write end under the same /dev/fd/N name.
                        if let Some(FdWriteEndpoint::ProcessSubstitution { path, .. }) =
                            &entry.write
                        {
                            if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
                                use std::os::unix::io::IntoRawFd;
                                let raw = file.into_raw_fd();
                                m.unix_fd_attach
                                    .push((fd, park_attach_source(raw, attach_floor)));
                            }
                        }
                        continue;
                    }
                };
                m.consume_fds.push(fd);
                let Ok((read_end, write_end)) = crate::fd::create_capture_pipe() else {
                    continue;
                };
                // The pipe kernel buffer is finite: feed the payload from a
                // thread so a large substitution cannot block the shell on a
                // reader that only exists after exec. An early child exit
                // surfaces as EPIPE in the thread and is ignored.
                std::thread::spawn(move || {
                    // EPIPE must stay a plain error in this thread (see
                    // the spawn comment); SIGPIPE is SIG_DFL process-wide
                    // since rubash#1333, so block it here.
                    #[cfg(unix)]
                    crate::builtins::kill::block_sigpipe_on_this_thread();
                    let _ = crate::fd::write_all(write_end, &buffered);
                    crate::fd::close_handle(write_end);
                });
                m.unix_fd_attach
                    .push((fd, park_attach_source(read_end, attach_floor)));
            }
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
                        DevOperandStdin::FdTable => {
                            self.dev_fd_table_operand(fd, &mut m, consumer_program)
                        }
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
                        DevOperandStdout::FdTable => {
                            self.dev_fd_table_operand(fd, &mut m, consumer_program)
                        }
                    },
                    _ => self.dev_fd_table_operand(fd, &mut m, consumer_program),
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
    /// `consumer_program` only steers the procsub argv-word delivery
    /// (rubash#2225's gate, see `procsub_operand_delivery`).
    fn dev_fd_table_operand(
        &self,
        fd: u32,
        m: &mut DevOperandMaterialization,
        consumer_program: Option<&Path>,
    ) -> Option<String> {
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
                    // A pipe-backed endpoint (a process substitution's
                    // read end, rubash#1952) carries only a display
                    // name, not a filesystem path. Real file endpoints
                    // keep handing over their path.
                    if file.path.to_string_lossy().starts_with("<(") {
                        // rubash#1959: deliver the live stream through
                        // the #1903 fifo-marker rendezvous — the child
                        // opens the marker path and its own runtime
                        // bridges to the live pipe, instead of the
                        // parent draining the producer to a temp file
                        // before spawn (GNU subst.c:6362
                        // process_substitute hands the child the pipe
                        // read end itself). Fall back to the drain
                        // when the rendezvous cannot be set up.
                        //
                        // rubash#2225: the rendezvous is sound only for
                        // a consumer known to bridge markers. A
                        // non-aware consumer opens the marker path
                        // plainly and reads the 7 marker bytes in place
                        // of the producer's stream (niu#720/#723:
                        // findstr.exe printing `!<fifo>`), so the
                        // marker delivery is gated on the consumer's
                        // class — every other consumer takes the drain
                        // below, exactly the pre-#2156 shape.
                        #[cfg(windows)]
                        {
                            if procsub_operand_delivery(
                                consumer_program,
                                &self.shell_state.env_vars,
                            ) == ProcsubOperandDelivery::FifoMarker
                            {
                                if let Some(path) = procsub_fifo_marker_operand(file, m) {
                                    // No consume record: the pump thread owns
                                    // the stream now, and the word fd is
                                    // command-scoped (removed at this very
                                    // command's end). A finish-time seek would
                                    // be dead semantics AND would serialize
                                    // against the pump's parked ReadFile on
                                    // the same pipe object — synchronous
                                    // handles allow one operation at a time,
                                    // so the shell would block until the
                                    // producer's next write (observed: the
                                    // whole producer lifetime).
                                    return Some(path);
                                }
                            }
                        }
                        m.consume_fds.push(fd);
                        drain_handle_to_temp(file.handle, m)
                    } else {
                        // rubash#2256: the child opens the path itself,
                        // so a regular file re-opens fresh in the child
                        // and the parent must NOT record the consume
                        // that would seek this endpoint to EOF when the
                        // operands finish — under the fresh model fd
                        // N's offset never moves. Non-regular file
                        // endpoints (a named fifo's pipe handle) keep
                        // the consume record.
                        if !file.reopens_fresh() {
                            m.consume_fds.push(fd);
                        }
                        Some(file.path.to_string_lossy().into_owned())
                    }
                }
                FdReadEndpoint::InheritedProcessStdin => {
                    drain_handle_to_temp(crate::fd::process_std_handle(0), m)
                }
                FdReadEndpoint::CoprocStdout { fd: file, .. } => {
                    drain_handle_to_temp(file.handle, m)
                }
                // rubash#1937: a directory endpoint cannot be
                // materialized into an openable operand — decline
                // and let the caller keep the /dev/fd/N spelling
                // (on Unix the child opens the real directory and
                // its reads fail EISDIR natively).
                FdReadEndpoint::Directory => None,
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
            // rubash#1223: `tee >(body)` resolves /dev/fd/N to a write
            // endpoint backed by the substitution's temp carrier — an
            // ordinary file the external child can open and write, which
            // the consumer then reads when finish_process_substitutions
            // runs it. GNU redir.c: the fd is the write end of the procsub
            // pipe; the carrier file is rubash's stand-in for it.
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                Some(path.to_string_lossy().into_owned())
            }
            // rubash#1898: an operand naming a /dev/full binding —
            // the child gets a throwaway temp whose contents are
            // never flushed anywhere (absorbed, like the device's
            // contract minus the ENOSPC the child would see from
            // the real device; the in-shell faces carry that shape).
            FdWriteEndpoint::FullDevice => dev_operand_temp(&[], m),
            // rubash#2042: an operand naming a 0644-class synthetic
            // /proc binding — same absorbed-temp treatment; the
            // EINVAL write profile the child would see from the real
            // procfs entry cannot cross the boundary (the in-shell
            // faces carry that shape).
            FdWriteEndpoint::ProcContentWrite => dev_operand_temp(&[], m),
            // rubash#2385: an operand naming a complementary channel
            // end — the child writes a temp, and finish appends its
            // bytes to the shared queue (the deferred-delivery shape
            // the Stdout/Stderr arms above use for their streams;
            // GNU's child writes the pipe itself).
            FdWriteEndpoint::PipeChannel(channel) => {
                let temp = dev_operand_temp(&[], m)?;
                m.flushes.push((
                    PathBuf::from(&temp),
                    DevOperandFlush::Channel(channel.clone()),
                ));
                Some(temp)
            }
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
                // rubash#2385: the operand child's bytes join the
                // channel's stream at the tail.
                DevOperandFlush::Channel(channel) => channel.append(&bytes),
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
    /// (emulated cat/sed have no child descriptor table). For stream
    /// endpoints GNU hands the child a dup of the descriptor, so the
    /// in-process equivalent is the fd endpoint's remaining bytes,
    /// consuming the shared offset; a regular-file endpoint is the
    /// exception — the operand open re-opens it fresh (rubash#2256),
    /// served whole from offset 0 by the fs::read branch below, and
    /// the descriptor's own offset does not move.
    /// `None` for an unbound or write-only-pipe fd — callers report ENOENT.
    /// rubash#1952: the live File endpoint a `/dev/fd/N` operand
    /// names, when the command does not rebind that descriptor
    /// itself — a streaming consumer (the in-process cat) copies it
    /// chunk-wise as bytes arrive instead of draining it whole (GNU
    /// cat.c reads the dup'd descriptor in a byte-copy loop).
    /// Buffered endpoints and command-rebound descriptors keep the
    /// byte serve (dev_fd_operand_bytes_for_command), which stays
    /// authoritative for them.
    pub(in crate::executor) fn dev_fd_operand_live_file(
        &self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Option<Rc<FileFd>> {
        if command_redirect_for_fd(cmd, fd).is_some() {
            return None;
        }
        match self.fd_table.read_endpoint(fd) {
            Some(FdReadEndpoint::File(file)) => {
                // rubash#2256: a regular-file endpoint streams from a
                // FRESH open (offset 0) and its own offset does not
                // move; other file endpoints (pipes) keep the live
                // endpoint itself.
                if file.reopens_fresh() {
                    if let Ok(fresh) = FileFd::open_read(file.path.clone()) {
                        return Some(fresh);
                    }
                }
                // rubash#2276: a socket endpoint's spelling-open
                // fails ENXIO in GNU — decline the live serve (the
                // byte serve below declines too, and the caller
                // reports the alias layer's ENXIO).
                if crate::fd::is_socket_handle(file.handle) {
                    return None;
                }
                Some(file)
            }
            _ => None,
        }
    }

    pub(in crate::executor) fn dev_fd_operand_bytes(&mut self, fd: u32) -> Option<Vec<u8>> {
        let entry = self.fd_table.entries.get(&fd)?.clone();
        if entry.closed {
            return None;
        }
        // rubash#2276: a socket endpoint's spelling-open fails ENXIO
        // in GNU — never drain the stream for an operand. Callers
        // that report the failure consult fd_alias_spelling_open_error
        // for the reason.
        if self.fd_alias_spelling_open_error(fd).is_some() {
            return None;
        }
        if let Some(read) = &entry.read {
            // rubash#2256 (wave 2): fd 0 inherited from the process is
            // not exempt from the alias rule — when the process stdin
            // is a regular file, a child's open("/dev/fd/0") is fresh
            // (the kernel resolves the fd magic link to the file), so
            // the operand serves the whole file from offset 0 and the
            // process offset does not move. The resolver's fd-0 arm
            // names exactly that case (a regular-file endpoint is
            // covered by the File arm below as well); the drain of the
            // real fd 0 further down remains for pipes and terminals,
            // where GNU's open attaches to the same stream.
            if let Some(path) = self.fd_alias_reopen_path(fd) {
                if let Ok(bytes) = std::fs::read(path) {
                    return Some(bytes);
                }
            }
            // rubash#2256: a regular-file endpoint re-opens fresh for
            // an operand — the whole file from offset 0, and the
            // descriptor's own offset does not move (the stream serve
            // below would hand out only the remaining bytes and
            // consume them). A failed fresh read falls through to the
            // stream serve, as the alias open does.
            if let FdReadEndpoint::File(file) = read {
                if file.reopens_fresh() {
                    if let Ok(bytes) = std::fs::read(&file.path) {
                        return Some(bytes);
                    }
                }
            }
            if let Some(bytes) = self.fd_table.read_all_bytes(fd) {
                return Some(bytes);
            }
            // InheritedProcessStdin has no table-owned bytes: drain the
            // process's real fd 0 (GNU's dup shares it, so reading it here
            // advances the parent's offset the same way). Only stream
            // stdins reach this arm — a regular-file stdin was served
            // fresh by the resolver check above.
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
            FdWriteEndpoint::CoprocStdin { .. }
            | FdWriteEndpoint::ProcessSubstitution { .. }
            // rubash#1898: a /dev/full write endpoint holds no
            // readable bytes.
            | FdWriteEndpoint::FullDevice
            // rubash#2042: neither does a 0644-class synthetic /proc
            // write binding (the entry's bytes ride the read side).
            | FdWriteEndpoint::ProcContentWrite
            // rubash#2385: neither does a complementary channel end
            // by itself — the queue's bytes ride the READ endpoint
            // the open paired it with (served by the read-side arms
            // above whenever the entry carries one).
            | FdWriteEndpoint::PipeChannel(_) => None,
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

#[cfg(test)]
mod tests {
    use super::{
        dev_operand_fd, procsub_operand_delivery, ProcsubOperandDelivery,
        PROCSUB_FIFO_MARKER_PROOF_ENV,
    };
    use std::collections::HashMap;
    use std::path::Path;

    /// Redundancy audit R3/D1: the spelling judgment is the registry's;
    /// this face's own `str::parse` tail-parse used to accept the `+N`
    /// spellings GNU rejects on every face (the literal open fails
    /// ENOENT in procfs; redir.c's no-dev-fd fallback likewise requires
    /// all_digits).
    #[test]
    fn r3_dev_operand_fd_spelling_verdicts() {
        assert_eq!(dev_operand_fd("/dev/stdin"), Some(0));
        assert_eq!(dev_operand_fd("/dev/stdout"), Some(1));
        assert_eq!(dev_operand_fd("/dev/stderr"), Some(2));
        assert_eq!(dev_operand_fd("/dev/fd/63"), Some(63));
        assert_eq!(dev_operand_fd("/proc/self/fd/2"), Some(2));
        assert_eq!(dev_operand_fd("/dev/fd/+3"), None);
        assert_eq!(dev_operand_fd("/proc/self/fd/+3"), None);
        assert_eq!(dev_operand_fd("/dev/null"), None);
        assert_eq!(dev_operand_fd("/etc/passwd"), None);
    }

    /// The one layer that stays local: the shell's own numeric-pid
    /// spelling rewrites to `self` (GNU resolves /proc/$$/fd/N through
    /// procfs); foreign pids are plain filesystem paths.
    #[test]
    fn r3_dev_operand_fd_self_pid_layer() {
        let own = format!("/proc/{}/fd/3", std::process::id());
        assert_eq!(dev_operand_fd(&own), Some(3));
        let own_plus = format!("/proc/{}/fd/+3", std::process::id());
        assert_eq!(dev_operand_fd(&own_plus), None);
        assert_eq!(dev_operand_fd("/proc/4294967294/fd/3"), None);
    }

    /// rubash#2225: the delivery gate's safe direction holds on every
    /// platform — no consumer offered, and consumers outside the
    /// WinuxCmd runtime class, always take the byte-correct drain; the
    /// marker rendezvous is a Windows-only face whose aware route (the
    /// dispatcher name) exists only there. The applet-identity route is
    /// pinned where the file-identity APIs live
    /// (path::tests::issue2225_fifo_marker_aware_classification).
    ///
    /// rubash#2473: the aware CLASS alone is no longer sufficient —
    /// WinuxCmd 1.1.20's applet reads do not bridge markers, so the
    /// marker route additionally requires the machine-level proof key.
    #[test]
    fn issue2225_procsub_operand_delivery_falls_to_drain() {
        let env = HashMap::new();
        assert_eq!(
            procsub_operand_delivery(None, &env),
            ProcsubOperandDelivery::DrainToTemp
        );
        // The niu#720/#723 consumer class: a System32 tool.
        let findstr = if cfg!(windows) {
            Path::new(r"C:\Windows\System32\findstr.exe")
        } else {
            Path::new("/usr/bin/findstr")
        };
        assert_eq!(
            procsub_operand_delivery(Some(findstr), &env),
            ProcsubOperandDelivery::DrainToTemp
        );
        assert_eq!(
            procsub_operand_delivery(Some(Path::new("/usr/bin/cat")), &env),
            ProcsubOperandDelivery::DrainToTemp
        );
        // The dispatcher name is the one route decidable from the path
        // alone — and only on Windows, where the face exists. rubash#2473:
        // without the proof key even the aware class drains (WinuxCmd
        // 1.1.20 applet reads do not bridge markers); with the key the
        // #2225 class verdict stands.
        let dispatcher = Path::new(r"C:\tools\winuxcmd\usr\bin\winuxcmd.exe");
        assert_eq!(
            procsub_operand_delivery(Some(dispatcher), &env),
            ProcsubOperandDelivery::DrainToTemp
        );
        let mut proof = HashMap::new();
        proof.insert(PROCSUB_FIFO_MARKER_PROOF_ENV.to_string(), "1".to_string());
        let expected = if cfg!(windows) {
            ProcsubOperandDelivery::FifoMarker
        } else {
            ProcsubOperandDelivery::DrainToTemp
        };
        assert_eq!(procsub_operand_delivery(Some(dispatcher), &proof), expected);
        // The key never rescues a consumer outside the runtime class.
        assert_eq!(
            procsub_operand_delivery(Some(findstr), &proof),
            ProcsubOperandDelivery::DrainToTemp
        );
    }
}
