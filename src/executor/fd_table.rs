//! Virtual file-descriptor state for shell-owned redirections.
//!
//! GNU Bash references: `redir.c`, `redir.h`, and `tests/vredir*.sub`.
//! Native Windows handles remain an executor/backend concern.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::executor::substitution_metadata::bytes_to_shell_text;

#[derive(Debug, Clone)]
pub(crate) struct TextInput {
    data: Vec<u8>,
    offset: usize,
    /// rubash#1141: production schedule of this materialized stream
    /// (process-substitution bodies), judged against the armed read
    /// deadline in `read_bytes` — see timed_input.rs.
    timing: Option<super::timed_input::SharedTimedInput>,
}

/// A real kernel object behind a file-backed fd. `Rc`-shared across dup'd
/// slots and fork'd (cloned) fd tables so the file offset stays shared —
/// POSIX open file description semantics (governance doc 3.6, P6/P10).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FileFd {
    pub(crate) handle: crate::fd::HANDLE,
    pub(crate) path: PathBuf,
}

impl FileFd {
    pub(crate) fn open_read(path: PathBuf) -> std::io::Result<Rc<Self>> {
        crate::fd::open_file_read(&path).map(|handle| Rc::new(Self { handle, path }))
    }

    pub(crate) fn open_write(
        path: PathBuf,
        append: bool,
        create_new: bool,
    ) -> std::io::Result<Rc<Self>> {
        let handle = if create_new {
            crate::fd::open_file_create_new(&path)
        } else if append {
            crate::fd::open_file_append(&path)
        } else {
            crate::fd::open_file_write_trunc(&path)
        }?;
        Ok(Rc::new(Self { handle, path }))
    }

    pub(crate) fn open_readwrite(path: PathBuf) -> std::io::Result<Rc<Self>> {
        crate::fd::open_file_readwrite(&path).map(|handle| Rc::new(Self { handle, path }))
    }

    /// `[N]<>file` endpoint pair — O_RDWR|O_CREAT on ONE open file
    /// description for regular paths, so both directions of the slot
    /// share a single offset (GNU redir.c r_input_output). niubash#519:
    /// the console multiplex device CON rejects GENERIC_READ|
    /// GENERIC_WRITE (ERROR_INVALID_PARAMETER — a console is two kernel
    /// objects, not one seekable file), so `<>/dev/tty` binds CONIN$ to
    /// the read endpoint and CONOUT$ to the write endpoint; together
    /// they cover GNU's read+write terminal descriptor.
    pub(crate) fn open_readwrite_pair(path: PathBuf) -> std::io::Result<(Rc<Self>, Rc<Self>)> {
        #[cfg(windows)]
        if crate::fd::is_dos_device_path(&path)
            && path.to_string_lossy().eq_ignore_ascii_case("CON")
        {
            let read = crate::fd::open_file_read(std::path::Path::new("CONIN$"))?;
            let write = crate::fd::open_file_write(std::path::Path::new("CONOUT$"))?;
            return Ok((
                Rc::new(Self {
                    handle: read,
                    path: path.clone(),
                }),
                Rc::new(Self {
                    handle: write,
                    path,
                }),
            ));
        }
        let file = Self::open_readwrite(path)?;
        Ok((file.clone(), file))
    }

    /// Adopt an already-open std File (a connected socket for
    /// /dev/tcp|/dev/udp redirection, rubash#1026) as a FileFd whose
    /// `path` is the shell-level target name.
    pub(crate) fn from_file(file: std::fs::File, path: PathBuf) -> Rc<Self> {
        #[cfg(unix)]
        let handle = {
            use std::os::unix::io::IntoRawFd;
            file.into_raw_fd()
        };
        #[cfg(windows)]
        let handle = {
            use std::os::windows::io::IntoRawHandle;
            file.into_raw_handle() as crate::fd::HANDLE
        };
        Rc::new(Self { handle, path })
    }
}

impl Drop for FileFd {
    fn drop(&mut self) {
        crate::fd::close_handle(self.handle);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FdReadEndpoint {
    Text(Rc<RefCell<TextInput>>),
    File(Rc<FileFd>),
    InheritedProcessStdin,
    ProcessSubstitution(Rc<RefCell<TextInput>>),
    CoprocStdout {
        pid: u32,
        fd: Rc<FileFd>,
    },
    /// A descriptor bound to a DIRECTORY (rubash#1937): GNU's open of
    /// a directory succeeds (O_RDONLY) and every read fails EISDIR —
    /// the read builtin reports `read error: Is a directory`
    /// (read.def:911). The variant exists for the Windows synthetic
    /// -link bind of /proc/self/cwd, where no directory handle can be
    /// opened at all: the bind succeeds with this endpoint and reads
    /// fabricate the EISDIR failure, so the in-shell readers report
    /// exactly GNU's shape. Off Windows a real directory FileFd
    /// serves the same descriptor.
    Directory,
}

#[derive(Debug, Clone)]
pub(crate) enum FdWriteEndpoint {
    Stdout,
    Stderr,
    File(Rc<FileFd>),
    CoprocStdin {
        pid: u32,
        fd: Rc<FileFd>,
    },
    ProcessSubstitution {
        path: PathBuf,
        command: String,
        /// rubash#1022: fork-placeholder pid registered as $! for a
        /// persistent `>(...)` output substitution (set at open, completed
        /// at close). `None` for the per-command temp form, whose $! is
        /// registered with its finished status at finish time instead.
        /// Bookkeeping only — excluded from endpoint identity below.
        procsub_pid: Option<u32>,
    },
    /// rubash#1898: an fd bound to /dev/full. The bind succeeds;
    /// every write fails ENOSPC, which the write faces render through
    /// sh_chkwrite (`<name>: write error: No space left on device`,
    /// builtins/common.c:320-334). No Windows handle has that failure
    /// profile, so the endpoint is carried symbolically and the
    /// faces produce the failure — the registry's answer travels
    /// with the descriptor instead of being re-derived per face.
    FullDevice,
}

#[derive(Debug, Clone)]
pub(crate) struct FdEntry {
    pub(crate) read: Option<FdReadEndpoint>,
    pub(crate) write: Option<FdWriteEndpoint>,
    pub(crate) closed: bool,
    pub(crate) dynamic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaterializedFd {
    pub(crate) read: Option<MaterializedRead>,
    pub(crate) write: Option<MaterializedWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MaterializedRead {
    Bytes(Vec<u8>),
    InheritedProcessStdin,
    File(PathBuf),
    CoprocStdout(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MaterializedWrite {
    Stdout,
    Stderr,
    File(PathBuf),
    CoprocStdin(u32),
    ProcessSubstitution { path: PathBuf, command: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FdError {
    Closed,
    NotOpenForRead,
    NotOpenForWrite,
}

#[derive(Debug, Clone)]
pub(crate) struct FdTable {
    pub(crate) entries: BTreeMap<u32, FdEntry>,
    /// builtins/read.def read_timeout: while `read -t N` runs, handle-backed
    /// reads wait at most until this deadline for input. `None` means an
    /// unbounded read. Only the blocking endpoints (File/CoprocStdout)
    /// consult it; buffered text endpoints answer instantly.
    pub(crate) read_deadline: Option<std::time::Instant>,
    /// Set when a bounded read expired mid-record — GNU still assigns the
    /// partial input and returns 128+SIGALRM (read.def:539-562).
    pub(crate) read_timed_out: bool,
    /// Set when a record read exhausted its input before finding the
    /// delimiter — GNU read.def:949 `retval = eof ? EXECUTION_FAILURE :
    /// EXECUTION_SUCCESS` fails the builtin while still assigning the
    /// partial record.
    pub(crate) read_eof_no_delimiter: bool,
    /// The strerror text of the first hard read(2) failure since the
    /// current record read began (e.g. EISDIR "Is a directory" when the
    /// descriptor names a directory). GNU read.def:911 reports it as
    /// `read: <fd>: read error: <strerror>` and fails the builtin;
    /// treating it as plain EOF made `read x < dir` silent
    /// (rubash#1495). Cleared at the top of every record read.
    pub(crate) read_error: Option<String>,
    /// dup2 snapshots for `FdWriteEndpoint::Stdout` markers (rubash#223):
    /// `N>&1` hands fd N the stdout OBJECT fd 1 held at dup time — the
    /// active capture buffer's generation when one was active, or the real
    /// process stdout (`None`) when none was — so later nested captures
    /// rebinding fd 1 never move it. Absent entry = the live fd-1 alias
    /// (the substitution's own fd-1 marker). `write_fd_endpoint` and the
    /// ordered-output model resolve writes through it (rubash#223: inside
    /// `OUT="$(inner 2>&1 1>/dev/null)"` a `$(... 3>&1 1>&4)` body's
    /// stderr writes landed in the INNER capture instead of the outer).
    pub(crate) stdout_alias_generation: std::collections::HashMap<u32, Option<usize>>,
}

/// The byte sequences that terminate a `read` record for `delimiter`
/// (builtins/read.def zread matches raw bytes): the delimiter's UTF-8
/// encoding, plus — for a delimiter char in U+0080..=U+00FF, which a
/// byte-oriented locale yields from ONE raw byte — that single byte. The
/// old `delimiter as u8` truncation searched for a byte that never occurs
/// in a UTF-8 stream, so `read -d é` over a captured pipeline record ran
/// to EOF and reported failure (rubash#1062).
pub(crate) struct ReadDelimiterSeqs {
    seqs: [[u8; 4]; 2],
    lens: [usize; 2],
    count: usize,
}

pub(crate) fn read_delimiter_seqs(delimiter: char) -> ReadDelimiterSeqs {
    let mut seqs = [[0u8; 4]; 2];
    let mut lens = [0usize; 2];
    let mut count = 0;
    let mut buf = [0u8; 4];
    let utf8 = delimiter.encode_utf8(&mut buf).as_bytes().to_vec();
    seqs[count][..utf8.len()].copy_from_slice(&utf8);
    lens[count] = utf8.len();
    count += 1;
    let code = delimiter as u32;
    if (0x80..=0xFF).contains(&code) && utf8.len() > 1 {
        seqs[count][0] = code as u8;
        lens[count] = 1;
        count += 1;
    }
    ReadDelimiterSeqs { seqs, lens, count }
}

/// Length of a delimiter sequence ending at `end` (exclusive) in `bytes`,
/// if any.
pub(crate) fn delimiter_seq_ending_at(
    seqs: &ReadDelimiterSeqs,
    bytes: &[u8],
    end: usize,
) -> Option<usize> {
    for i in 0..seqs.count {
        let len = seqs.lens[i];
        if len <= end && &bytes[end - len..end] == &seqs.seqs[i][..len] {
            return Some(len);
        }
    }
    None
}

/// Escape state for non-raw `read` delimiter matching (GNU
/// builtins/read.def: without -r, a backslash consumes the next
/// character as an escaped literal — zread/read_mbchar — and an escaped
/// character can never terminate the record; that includes the
/// delimiter `\` itself, which only -r makes matchable). Byte-oriented
/// readers feed each byte through `suppresses_match` before testing
/// the delimiter: the backslash byte itself and every byte of the
/// escaped character report "suppressed" (rubash#1300).
pub(crate) struct ReadEscapeTracker {
    enabled: bool,
    escaped_char_bytes_left: usize,
    escape_pending: bool,
}

impl ReadEscapeTracker {
    pub(crate) fn new(enabled: bool) -> Self {
        ReadEscapeTracker {
            enabled,
            escaped_char_bytes_left: 0,
            escape_pending: false,
        }
    }

    /// True when a delimiter match ending at `byte` must be ignored
    /// because the byte is an escape introducer or part of an escaped
    /// character. The byte is consumed either way.
    pub(crate) fn suppresses_match(&mut self, byte: u8) -> bool {
        if !self.enabled {
            return false;
        }
        if self.escaped_char_bytes_left > 0 {
            self.escaped_char_bytes_left -= 1;
            return true;
        }
        if self.escape_pending {
            self.escape_pending = false;
            self.escaped_char_bytes_left = utf8_first_byte_len(byte).saturating_sub(1);
            return true;
        }
        if byte == b'\\' {
            self.escape_pending = true;
            return true;
        }
        false
    }
}

/// Byte length of the UTF-8 character a leading byte introduces
/// (invalid leading bytes count as one byte, matching the
/// non-continuation-byte character accounting the readers use).
fn utf8_first_byte_len(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

impl FdTable {
    pub(crate) fn new() -> Self {
        let mut table = Self {
            entries: BTreeMap::new(),
            read_deadline: None,
            read_timed_out: false,
            read_eof_no_delimiter: false,
            read_error: None,
            stdout_alias_generation: std::collections::HashMap::new(),
        };
        table.entries.insert(
            0,
            FdEntry {
                read: Some(FdReadEndpoint::InheritedProcessStdin),
                write: None,
                closed: false,
                dynamic: false,
            },
        );
        table.entries.insert(
            1,
            FdEntry {
                read: None,
                write: Some(FdWriteEndpoint::Stdout),
                closed: false,
                dynamic: false,
            },
        );
        table.entries.insert(
            2,
            FdEntry {
                read: None,
                write: Some(FdWriteEndpoint::Stderr),
                closed: false,
                dynamic: false,
            },
        );
        table
    }

    /// rubash#1557: adopt the descriptors the process inherited open
    /// (fd >= 3), snapshotted by `crate::fd::inherited_open_fds` at the
    /// top of Executor::new. GNU keeps no registry to consult — redir.c
    /// validates a dup source against the kernel itself
    /// (`fcntl (redirector, F_GETFD, 0) != -1`, redir.c:1160), so an
    /// inherited descriptor is simply usable (`rubash -c 'echo x >&3'
    /// 3>file`). The virtual table must therefore be seeded from kernel
    /// state: each inherited fd becomes a File endpoint over the
    /// executor's own duplicate of the descriptor (see
    /// `crate::fd::dup_adopted_fd`), the read/write arms following the
    /// F_GETFL access mode, both arms sharing ONE FileFd for O_RDWR (a
    /// single open file description — the offset is shared, as dup
    /// requires).
    ///
    /// Slots already claimed (the `__RUBASH_FD_*` respawn restore in
    /// init.rs runs first) keep their restored endpoints. A snapshotted
    /// fd is re-validated before adoption: a transient descriptor that
    /// has since closed is skipped, and the live direction wins over
    /// the snapshot's. The FileFd drop closes the duplicate when the
    /// slot's last reference goes away — exactly the `exec N>&-`
    /// contract from the shell's point of view (GNU closes the kernel
    /// descriptor, and later references report Bad file descriptor),
    /// while the host process's original descriptor is never closed
    /// by the executor (rubash#1838: an embedded Executor does not own
    /// the host's descriptors).
    ///
    /// On Windows the snapshot is always empty (handle inheritance is
    /// opt-in per handle; see the annotated stubs in
    /// fd/windows_impl.rs), so this is a no-op there.
    pub(crate) fn register_inherited_process_fds(&mut self, snapshot: &[(u32, bool, bool)]) {
        for &(fd, _, _) in snapshot {
            if self.entries.contains_key(&fd) {
                continue;
            }
            let Some((readable, writable)) = crate::fd::fd_directions(fd) else {
                continue;
            };
            let Some(handle) = crate::fd::dup_adopted_fd(fd) else {
                continue;
            };
            let file = Rc::new(FileFd {
                handle,
                path: crate::fd::inherited_fd_path(fd),
            });
            if readable {
                self.open_input(fd, FdReadEndpoint::File(file.clone()), false);
            }
            if writable {
                self.open_output(fd, FdWriteEndpoint::File(file), false);
            }
        }
    }

    /// Dynamic-fd ceiling when no `ulimit -n` value is in force. GNU has no
    /// table cap of its own — fcntl F_DUPFD (redir.c:940) hands out the
    /// lowest free descriptor >= SHELL_FD_BASE below the RLIMIT_NOFILE soft
    /// limit — so the honest Windows emulation default is the oracle's own
    /// soft limit (WSL GNU bash 5.3.0, 2026-10-19 probe: soft 10240, hard
    /// 1048576), not a homemade 1024 (rubash#773: the old cap surfaced as
    /// "cannot duplicate fd: Invalid argument" after exactly 1014 `{fd}`
    /// allocations).
    pub(crate) const DEFAULT_OPEN_FILES_SOFT_LIMIT: u32 = 10240;

    /// Dynamic-fd ceiling for the unlimited case (`ulimit -Sn unlimited`
    /// over an unlimited hard). Pinned at the oracle's hard default — on
    /// Windows each slot is one real HANDLE, so the process handle budget,
    /// not this number, is the true wall.
    pub(crate) const UNLIMITED_OPEN_FILES_CEILING: u32 = 1_048_576;

    pub(crate) fn allocate_dynamic(&mut self) -> u32 {
        // Bash's F_DUPFD requests the lowest available descriptor at or above
        // SHELL_FD_BASE. Closed dynamic entries are reusable immediately.
        //
        // NOTE: this low-first policy is what GNU uses for `{v}` redirection
        // allocation (`exec {a}</dev/null; exec {b}</dev/null` -> 10, 11, and
        // after closing both the next `{c}` goes back to 10). Coproc does NOT
        // use this path: GNU moves coproc pipe ends to the highest free fd
        // below 64 (move_to_high_fd, maxfd 64) giving the stable `63 60` that
        // coproc.tests golden output shows. Coproc's high-fd choice lives in
        // compound_exec.rs so this general allocator stays low-first.
        self.allocate_dynamic_with_limit(None).unwrap_or(10)
    }

    /// GNU fcntl F_DUPFD with SHELL_FD_BASE=10 (redir.c:940, do_redirection_
    /// internal REDIR_VARASSIGN): the kernel hands out the lowest free
    /// descriptor >= 10 below the RLIMIT_NOFILE *soft* limit, failing with
    /// EMFILE when every slot below it is taken and with EINVAL when the
    /// base itself sits at/above the limit (`ulimit -n 6` then
    /// `exec {v}</dev/null` leaves v unset — vredir6.sub). `limit` is that
    /// soft limit (None = unlimited); walk the occupied slots in fd order
    /// and stop at the first gap, so closed dynamic entries are reusable
    /// exactly like freed kernel descriptors and the capacity follows the
    /// limit instead of a homemade ceiling (rubash#773).
    pub(crate) fn allocate_dynamic_with_limit(&mut self, limit: Option<u32>) -> Option<u32> {
        let upper = limit.unwrap_or(Self::UNLIMITED_OPEN_FILES_CEILING) as usize;
        if upper <= 10 {
            return None;
        }
        let mut candidate: usize = 10;
        for (&fd, entry) in &self.entries {
            if (fd as usize) < candidate || !Self::occupied(entry) {
                continue;
            }
            if fd as usize == candidate {
                candidate += 1;
            } else {
                break;
            }
        }
        if candidate >= upper {
            return None;
        }
        Some(candidate as u32)
    }

    pub(crate) fn open_input(&mut self, fd: u32, endpoint: FdReadEndpoint, dynamic: bool) {
        let entry = self.entry_mut(fd, dynamic);
        entry.read = Some(endpoint);
        entry.closed = false;
    }

    pub(crate) fn open_output(&mut self, fd: u32, endpoint: FdWriteEndpoint, dynamic: bool) {
        let entry = self.entry_mut(fd, dynamic);
        entry.write = Some(endpoint);
        entry.closed = false;
        self.stdout_alias_generation.remove(&fd);
    }

    /// `N<&M` / `N>&M` — POSIX dup2 semantics: the descriptor is copied
    /// wholesale, both directions. GNU redir.c dup_redirects only uses the
    /// operator to pick the default fd; `exec 8<&1` makes fd 8 share fd 1's
    /// (write-only) open file description, so `echo >&8` works and
    /// `read <&8` fails. Endpoint sharing (Rc clone) makes the kernel file
    /// object — and its offset — common to both slots.
    fn dup_entry(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        let (read, write) = self
            .entries
            .get(&source)
            .filter(|entry| !entry.closed)
            .filter(|entry| entry.read.is_some() || entry.write.is_some())
            .map(|entry| (entry.read.clone(), entry.write.clone()))
            .ok_or(FdError::Closed)?;
        // dup2 snapshot bookkeeping (rubash#223): copying the live `Stdout`
        // marker freezes WHICH stdout object the fd holds — the active
        // capture's generation, or the real process stdout when no capture
        // is active — so nested command substitutions rebinding fd 1 never
        // hijack the write. A dup ONTO fd 1 propagates the source's record
        // (the fd takes the source's object wholesale); fd 1 without a
        // record stays the live alias.
        if matches!(write, Some(FdWriteEndpoint::Stdout)) {
            if target == 1 {
                match self.stdout_alias_generation.get(&source) {
                    Some(record) => {
                        self.stdout_alias_generation.insert(target, *record);
                    }
                    None => {
                        self.stdout_alias_generation.remove(&target);
                    }
                }
            } else {
                let record = if crate::executor::shell_options::stdout_capture_active() {
                    Some(crate::executor::shell_options::stdout_capture_generation())
                } else {
                    None
                };
                self.stdout_alias_generation.insert(target, record);
            }
        } else {
            self.stdout_alias_generation.remove(&target);
        }
        let dynamic = self.is_dynamic(target);
        let entry = self.entry_mut(target, dynamic);
        entry.read = read;
        entry.write = write;
        entry.closed = false;
        Ok(())
    }

    pub(crate) fn dup_input(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        self.dup_entry(target, source)
    }

    pub(crate) fn dup_output(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        self.dup_entry(target, source)
    }

    #[allow(dead_code)]
    pub(crate) fn move_input(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        self.dup_input(target, source)?;
        self.close(source);
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn move_output(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        self.dup_output(target, source)?;
        self.close(source);
        Ok(())
    }

    pub(crate) fn close_input(&mut self, fd: u32) {
        let entry = self.entry_mut(fd, false);
        entry.read = None;
        entry.closed = entry.write.is_none();
    }

    pub(crate) fn close_output(&mut self, fd: u32) {
        let entry = self.entry_mut(fd, false);
        entry.write = None;
        entry.closed = entry.read.is_none();
        self.stdout_alias_generation.remove(&fd);
    }

    pub(crate) fn close(&mut self, fd: u32) {
        let entry = self.entry_mut(fd, false);
        entry.read = None;
        entry.write = None;
        entry.closed = true;
    }

    pub(crate) fn is_open_for_read(&self, fd: u32) -> bool {
        self.entries
            .get(&fd)
            .map_or(false, |entry| !entry.closed && entry.read.is_some())
    }

    pub(crate) fn is_open_for_write(&self, fd: u32) -> bool {
        self.entries
            .get(&fd)
            .map_or(false, |entry| !entry.closed && entry.write.is_some())
    }

    /// Open in either direction — POSIX descriptors are not directional.
    pub(crate) fn is_open(&self, fd: u32) -> bool {
        self.entries.get(&fd).map_or(false, |entry| {
            !entry.closed && (entry.read.is_some() || entry.write.is_some())
        })
    }

    pub(crate) fn is_closed(&self, fd: u32) -> bool {
        self.entries.get(&fd).map_or(false, |entry| entry.closed)
    }

    pub(crate) fn is_dynamic(&self, fd: u32) -> bool {
        self.entries.get(&fd).map_or(false, |entry| entry.dynamic)
    }

    pub(crate) fn has_entry(&self, fd: u32) -> bool {
        self.entries.contains_key(&fd)
    }

    pub(crate) fn read_endpoint(&self, fd: u32) -> Option<FdReadEndpoint> {
        self.entries
            .get(&fd)
            .filter(|entry| !entry.closed)
            .and_then(|entry| entry.read.clone())
    }

    pub(crate) fn write_endpoint(&self, fd: u32) -> Option<FdWriteEndpoint> {
        self.entries
            .get(&fd)
            .filter(|entry| !entry.closed)
            .and_then(|entry| entry.write.clone())
    }

    /// True when fd 0's read endpoint is a descriptor the shell owns — a
    /// real file (the enclosing compound's `done < file` binding,
    /// command_input_scope.rs compound_fd0_stream_file), a virtual text
    /// fd, a procsub stream or a coproc pipe — rather than the process's
    /// inherited stdin. GNU builtin_read reads exactly ONE descriptor: fd
    /// 0 as do_redirections bound it (read.def:294; redir.c:767-955), so
    /// in this state an exhausted record means EOF on that descriptor —
    /// read.def:762-768 breaks with eof=1 and :949 returns
    /// EXECUTION_FAILURE. There is no second transport to fall through
    /// to: reading the inherited process stdin instead blocks forever on
    /// the parent's still-open stdin pipe (the readhang regression:
    /// `while read; done < list.txt` under a piped-open stdin).
    pub(crate) fn fd0_bound_away_from_process_stdin(&self) -> bool {
        !matches!(
            self.read_endpoint(0),
            None | Some(FdReadEndpoint::InheritedProcessStdin)
        )
    }

    pub(crate) fn read_text(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        self.read_bytes(fd, delimiter, char_limit, exact, escape_aware, raw)
            .map(|bytes| bytes_to_shell_text(&bytes))
    }

    pub(crate) fn read_bytes(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<Vec<u8>> {
        self.read_error = None;
        let endpoint = self.entries.get(&fd)?.read.clone()?;
        // rubash#1937: a directory descriptor fails every read with
        // EISDIR (read.def:911) — record the strerror the read
        // builtin's error channel renders, and serve no record.
        if matches!(endpoint, FdReadEndpoint::Directory) {
            self.read_error = Some("Is a directory".to_string());
            return None;
        }
        let file = match &endpoint {
            FdReadEndpoint::File(file) | FdReadEndpoint::CoprocStdout { fd: file, .. } => {
                Some(file.clone())
            }
            _ => None,
        };
        if let Some(file) = file {
            return self.read_file_bytes(file, delimiter, char_limit, exact, escape_aware, raw);
        }
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let mut input = input.borrow_mut();
        // rubash#1141: a materialized stream (process substitution) whose
        // production was timed is judged against the armed read deadline
        // exactly like a real pipe: GNU's consumer blocks in zread
        // (read.def) and the alarm fires at the deadline, so a record
        // whose bytes — or the EOF itself — only "arrive" after the
        // deadline is a timeout (128+SIGALRM at the read builtin), and
        // only the prefix that arrived in time may be served. The
        // schedule is consulted on every read (deadline or not) so the
        // virtual clock keeps accounting untimed consumption.
        let timing = input.timing.clone();
        if input.offset >= input.data.len() {
            if let Some(timing) = &timing {
                let mut timing = timing.borrow_mut();
                if timing.matches_len(input.data.len())
                    && matches!(
                        timing.judge(None, input.offset, self.read_deadline),
                        super::timed_input::TimedVerdict::Timeout { .. }
                    )
                {
                    self.read_timed_out = true;
                }
            }
            return None;
        }
        if char_limit == Some(0) {
            return Some(Vec::new());
        }
        // One scan of a byte window, returning (consumed, delimiter
        // found, delimiter length, char limit reached, payload length).
        // Runs once on the full remaining slice and, on a timing timeout,
        // once more on the in-time prefix.
        let scan = |slice: &[u8]| -> (usize, bool, usize, bool, usize) {
            let seqs = read_delimiter_seqs(delimiter);
            let mut consumed = 0;
            let mut chars = 0;
            let mut delimiter_found = false;
            let mut delimiter_len = 0usize;
            let mut limit_reached = false;
            let mut escapes = ReadEscapeTracker::new(escape_aware && !exact);
            // Post-escape counting state (rubash#1328), shared with the
            // other readers: an unquoted backslash itself is not a stored
            // character; the character it quotes completes one unit (zero
            // for a newline continuation), and a delimiter byte ending an
            // escape pair is data, not a terminator.
            let mut escaped = false;
            for (index, byte) in slice.iter().copied().enumerate() {
                let lead = byte & 0xc0 != 0x80;
                let was_escaped = escaped;
                if lead {
                    if escaped {
                        if byte != b'\n' {
                            chars += 1;
                        }
                        escaped = false;
                    } else if !raw && byte == b'\\' {
                        escaped = true;
                    } else {
                        chars += 1;
                    }
                }
                consumed = index + 1;
                let suppressed = escapes.suppresses_match(byte);
                if !exact && !suppressed {
                    if let Some(len) = delimiter_seq_ending_at(&seqs, slice, consumed) {
                        delimiter_found = true;
                        delimiter_len = len;
                        break;
                    }
                }
                if char_limit.is_some_and(|limit| chars >= limit) {
                    limit_reached = true;
                    break;
                }
            }
            let result_len = if delimiter_found {
                consumed - delimiter_len
            } else {
                consumed
            };
            (
                consumed,
                delimiter_found,
                delimiter_len,
                limit_reached,
                result_len,
            )
        };
        let slice = &input.data[input.offset..];
        let (consumed, delimiter_found, _delimiter_len, limit_reached, result_len) = scan(slice);
        // read.def:949 eof rule: the slice ran out without the delimiter
        // (and without a -n/-N limit ending the record). Served-record
        // invariant: only a returned record may set the eof flag (see
        // read_file_bytes).
        if consumed == 0 {
            return None;
        }
        if let Some(timing) = &timing {
            let mut timing = timing.borrow_mut();
            if timing.matches_len(input.data.len()) {
                // A record that runs to the end of the materialized data
                // without delimiter or limit is bounded by the EOF, whose
                // arrival is the production duration.
                let record_end = if delimiter_found || limit_reached {
                    Some(input.offset + consumed)
                } else {
                    None
                };
                if let super::timed_input::TimedVerdict::Timeout { available_end } =
                    timing.judge(record_end, input.offset, self.read_deadline)
                {
                    let prefix = &input.data[input.offset..available_end];
                    let (p_consumed, _, _, _, p_result_len) = scan(prefix);
                    self.read_timed_out = true;
                    self.read_eof_no_delimiter = false;
                    let result = prefix[..p_result_len].to_vec();
                    input.offset += p_consumed;
                    return Some(result);
                }
            }
        }
        self.read_eof_no_delimiter = !delimiter_found && !limit_reached;
        let result = slice[..result_len].to_vec();
        input.offset += consumed;
        Some(result)
    }

    /// rubash#1141: preset a materialized endpoint's read offset (an
    /// fd-0 binding serving a process-substitution stream starts at the
    /// stream's shared offset, not at 0). No-op for other endpoint
    /// kinds.
    pub(crate) fn set_input_offset(&mut self, fd: u32, offset: usize) {
        if let Some(entry) = self.entries.get(&fd) {
            if let Some(read) = &entry.read {
                if let FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) =
                    read
                {
                    let mut input = input.borrow_mut();
                    input.offset = offset.min(input.data.len());
                }
            }
        }
    }

    /// rubash#1141: attach a production schedule to a materialized
    /// Text/ProcessSubstitution endpoint (registered after the endpoint
    /// is opened, e.g. for a process-substitution fd). No-op for other
    /// endpoint kinds.
    pub(crate) fn set_input_timing(
        &mut self,
        fd: u32,
        timing: super::timed_input::SharedTimedInput,
    ) {
        if let Some(entry) = self.entries.get(&fd) {
            if let Some(read) = &entry.read {
                if let FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) =
                    read
                {
                    input.borrow_mut().timing = Some(timing);
                }
            }
        }
    }

    pub(crate) fn read_all_text(&mut self, fd: u32) -> Option<String> {
        self.read_all_bytes(fd)
            .map(|bytes| bytes_to_shell_text(&bytes))
    }

    /// Byte-wise ReadFile loop for handle-backed endpoints. Reads exactly
    /// up to the delimiter / character limit without over-consuming — the
    /// offset is shared across every duplicate of this file object, so
    /// prefetching would steal bytes from sibling slots (GNU zread reads
    /// one byte at a time off unbuffered fds for the same reason).
    fn read_file_bytes(
        &mut self,
        file: Rc<FileFd>,
        delimiter: char,
        char_limit: Option<usize>,
        exact: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<Vec<u8>> {
        if char_limit == Some(0) {
            return Some(Vec::new());
        }
        // Seekable disk file without a read deadline: GNU buffers this
        // case (read.def:664-671 keeps unbuffered_read == 0 for non-tty,
        // non-pipe input and read.def:737 zreadc serves from a buffer),
        // then rewinds the descriptor to the consumed boundary at exit
        // (read.def:940 zsyncfd) so the shared offset lands exactly past
        // the record. Chunked reads + explicit rewind reproduce that
        // syscall profile — a 1-byte ReadFile loop would make a
        // while-read over a large file O(bytes) syscalls where GNU is
        // O(bytes/chunk). Deadline reads stay byte-wise (per-byte
        // wait_readable, read.def check_read_timeout).
        if self.read_deadline.is_none() && crate::fd::is_disk_file(file.handle) {
            return self.read_file_bytes_buffered(
                file,
                delimiter,
                char_limit,
                exact,
                escape_aware,
                raw,
            );
        }
        let mut result = Vec::new();
        let mut chars = 0usize;
        let mut consumed = false;
        let mut delimiter_found = false;
        let mut limit_reached = false;
        // Post-escape counting state (rubash#1328), as in read_bytes.
        let mut escaped = false;
        let seqs = read_delimiter_seqs(delimiter);
        let mut escapes = ReadEscapeTracker::new(escape_aware);
        loop {
            // read.def check_read_timeout + shtimer_select: each byte read
            // is bounded by the remaining deadline.
            if let Some(deadline) = self.read_deadline {
                let now = std::time::Instant::now();
                let timed_out = now >= deadline
                    || crate::fd::wait_readable(file.handle, deadline - now)
                        == crate::fd::ReadWait::Timeout;
                if timed_out {
                    self.read_timed_out = true;
                    break;
                }
            }
            let byte = match crate::fd::read_some(file.handle, 1) {
                Ok(buf) if buf.is_empty() => break, // EOF
                Ok(buf) => buf[0],
                Err(error) => {
                    // A hard read failure (EISDIR on a directory
                    // descriptor, EIO, ...) is NOT EOF: GNU read.def:911
                    // reports it (`read: 0: read error: Is a directory`)
                    // and fails the builtin. Record the first such
                    // failure for the read builtin's None handling
                    // (rubash#1495); bytes already consumed keep the
                    // plain partial-record path.
                    if result.is_empty() && self.read_error.is_none() {
                        self.read_error = Some(crate::posix_errors::message(&error));
                    }
                    break;
                }
            };
            consumed = true;
            let lead = byte & 0xc0 != 0x80;
            let was_escaped = escaped;
            if lead {
                if escaped {
                    if byte != b'\n' {
                        chars += 1;
                    }
                    escaped = false;
                } else if !raw && byte == b'\\' {
                    escaped = true;
                } else {
                    chars += 1;
                }
            }
            result.push(byte);
            let suppressed = escapes.suppresses_match(byte);
            if !exact && !suppressed {
                // The delimiter is consumed but not returned: when the
                // accumulated bytes end with one of its sequences, drop
                // the sequence back off the record (rubash#1062).
                if let Some(len) = delimiter_seq_ending_at(&seqs, &result, result.len()) {
                    result.truncate(result.len() - len);
                    delimiter_found = true;
                    break;
                }
            }
            if char_limit.is_some_and(|limit| chars >= limit) {
                limit_reached = true;
                break;
            }
        }
        // EOF with zero bytes read must surface as None — `read` at EOF
        // exits 1, distinct from an empty line (delimiter consumed).
        // read.def:949 eof rule: the stream ended without the delimiter
        // (a -n/-N limit ending the record is not EOF). The flag is a
        // property of the SERVED record only: a reader that yields no
        // record must not leave it set, or a later fallback path's record
        // (e.g. FUNCTION_STDIN after this endpoint was probed empty —
        // `exec 0</dev/null; while read w; done <<EOF`) inherits a bogus
        // EOF status and the loop exits after zero iterations.
        if !consumed {
            return None;
        }
        self.read_eof_no_delimiter = !delimiter_found && !limit_reached && !self.read_timed_out;
        Some(result)
    }

    /// Chunked record read for seekable disk files (see read_file_bytes):
    /// read 8 KiB chunks, scan for the delimiter / char limit, then seek
    /// the shared handle back to the consumed boundary — zreadc + zsyncfd
    /// parity (read.def:737, :940). Byte accounting matches the byte-wise
    /// loop exactly (delimiter consumed but not returned in non-exact
    /// mode; chars counted on non-continuation bytes).
    fn read_file_bytes_buffered(
        &mut self,
        file: Rc<FileFd>,
        delimiter: char,
        char_limit: Option<usize>,
        exact: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<Vec<u8>> {
        let Ok(origin) = crate::fd::file_position(file.handle) else {
            return None;
        };
        let seqs = read_delimiter_seqs(delimiter);
        let mut escapes = ReadEscapeTracker::new(escape_aware);
        let mut buffer: Vec<u8> = Vec::new();
        let mut result: Vec<u8> = Vec::new();
        let mut chars = 0usize;
        let mut consumed = false;
        let mut delimiter_found = false;
        let mut limit_reached = false;
        let mut rewind: Option<u64> = None;
        // Post-escape counting state (rubash#1328), as in read_bytes;
        // persists across chunk boundaries.
        let mut escaped = false;
        'chunks: loop {
            let scanned_from = buffer.len();
            match crate::fd::read_some(file.handle, 8192) {
                Ok(chunk) if chunk.is_empty() => break,
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(_) => break,
            }
            for (index, &byte) in buffer[scanned_from..].iter().enumerate() {
                let absolute = scanned_from + index;
                consumed = true;
                let lead = byte & 0xc0 != 0x80;
                let was_escaped = escaped;
                if lead {
                    if escaped {
                        if byte != b'\n' {
                            chars += 1;
                        }
                        escaped = false;
                    } else if !raw && byte == b'\\' {
                        escaped = true;
                    } else {
                        chars += 1;
                    }
                }
                result.push(byte);
                let suppressed = escapes.suppresses_match(byte);
                if !exact && !suppressed {
                    if let Some(len) = delimiter_seq_ending_at(&seqs, &result, result.len()) {
                        // Delimiter consumed but not returned; everything
                        // after its end position is prefetched data to
                        // rewind over.
                        result.truncate(result.len() - len);
                        delimiter_found = true;
                        rewind = Some(origin + (absolute as u64) + 1);
                        break 'chunks;
                    }
                }
                if char_limit.is_some_and(|limit| chars >= limit) {
                    limit_reached = true;
                    rewind = Some(origin + (absolute as u64) + 1);
                    break 'chunks;
                }
            }
        }
        if let Some(position) = rewind {
            let _ = crate::fd::seek_absolute(file.handle, position);
        }
        if !consumed {
            return None;
        }
        // read.def:949 eof rule: the stream ended before the delimiter (a
        // -n/-N limit ending the record is not EOF). The byte-wise path
        // (:553) and the in-memory path (:454) both set this; the seekable
        // fast path used to serve the trailing partial record with the flag
        // still clear, so `{ read l; } < f` and `read l <&9` reported
        // success at EOF-without-newline and `while read` ran one extra
        // iteration (niubash#701).
        self.read_eof_no_delimiter = !delimiter_found && !limit_reached && !self.read_timed_out;
        Some(result)
    }

    pub(crate) fn read_all_bytes(&mut self, fd: u32) -> Option<Vec<u8>> {
        let endpoint = self.entries.get(&fd)?.read.clone()?;
        if let FdReadEndpoint::File(file) | FdReadEndpoint::CoprocStdout { fd: file, .. } = endpoint
        {
            let mut out = Vec::new();
            loop {
                match crate::fd::read_some(file.handle, 8192) {
                    Ok(buf) if buf.is_empty() => break,
                    Ok(buf) => out.extend_from_slice(&buf),
                    Err(_) => break,
                }
            }
            return Some(out);
        }
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let mut input = input.borrow_mut();
        let result = input.data.get(input.offset..)?.to_vec();
        input.offset = input.data.len();
        Some(result)
    }

    /// GNU redir.c: a consumer handed this fd's stream through a dup owns
    /// the shared offset — bytes it took are gone for the next reader
    /// (procsub.tests count_lines: five `wc -l < $1` calls see 1,0,0,0,0).
    /// `&self` variant of consume_all_text for the pure-read serving
    /// paths (Text/ProcessSubstitution carry their offset in a RefCell).
    pub(crate) fn drain_input_to_eof(&self, fd: u32) {
        let endpoint = self.entries.get(&fd).and_then(|entry| entry.read.clone());
        if let Some(FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input)) =
            endpoint
        {
            let mut input = input.borrow_mut();
            input.offset = input.data.len();
        }
    }

    pub(crate) fn consume_all_text(&mut self, fd: u32) -> Option<usize> {
        let endpoint = self.entries.get(&fd)?.read.clone()?;
        if let FdReadEndpoint::File(file) = endpoint {
            let _ = crate::fd::seek_end(file.handle);
            return None;
        }
        if let FdReadEndpoint::CoprocStdout { .. } = endpoint {
            return None;
        }
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let mut input = input.borrow_mut();
        input.offset = input.data.len();
        Some(input.offset)
    }

    /// Current read offset of a materialized (Text /
    /// ProcessSubstitution) endpoint, without copying its bytes.
    /// Callers that only mirror the cursor (the `read` builtin's
    /// per-record offset write-back, scope restore, existence probes)
    /// must use this rather than [`Self::input_snapshot`]: GNU serves
    /// each record off the shared descriptor cursor in O(record)
    /// (read.def zreadc), so consulting the cursor after a record
    /// must not clone and re-convert the whole stream — doing so made
    /// every `read` on a procsub endpoint cost O(stream), i.e.
    /// `while read; done < <(...)` quadratic in the stream size
    /// (perf-round F6: subst.lad_procsub_read@20000 at 27x GNU).
    pub(crate) fn input_offset(&self, fd: u32) -> Option<usize> {
        let endpoint = self.entries.get(&fd)?.read.as_ref()?;
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        Some(input.borrow().offset)
    }

    pub(crate) fn input_snapshot_bytes(&self, fd: u32) -> Option<(Vec<u8>, usize)> {
        let endpoint = self.entries.get(&fd)?.read.as_ref()?;
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let input = input.borrow();
        Some((input.data.clone(), input.offset))
    }

    /// rubash#1952: drain a live File read endpoint (a streamed
    /// procsub's pipe, an exec-bound file) from its current offset to
    /// EOF through the shared OS handle — the &self counterpart of
    /// read_all_bytes for expansion-time readers (the comsub
    /// quick-read ladder) that cannot take &mut. The reads advance
    /// the shared offset, exactly like a GNU fork child's reads.
    pub(crate) fn read_file_endpoint_bytes(&self, fd: u32) -> Option<Vec<u8>> {
        let FdReadEndpoint::File(file) = self.read_endpoint(fd)? else {
            return None;
        };
        let mut out = Vec::new();
        loop {
            match crate::fd::read_some(file.handle, 8192) {
                Ok(buf) if buf.is_empty() => break,
                Ok(buf) => out.extend_from_slice(&buf),
                Err(_) => break,
            }
        }
        Some(out)
    }

    pub(crate) fn input_snapshot(&self, fd: u32) -> Option<(String, usize)> {
        self.input_snapshot_bytes(fd)
            .map(|(bytes, offset)| (bytes_to_shell_text(&bytes), offset))
    }

    /// Pull one `\n`-terminated line from a buffered text read endpoint,
    /// advancing the shared offset so the `read` builtin and the script
    /// driver consume the fd sequentially like GNU's buffered fd-0 stream
    /// (input.c bash_input). Returns `None` when the fd is not a buffered
    /// text endpoint, `Some(vec![])` at end of the buffer.
    pub(crate) fn take_buffered_input_line(&self, fd: u32) -> Option<Vec<u8>> {
        if let Some(FdReadEndpoint::File(file) | FdReadEndpoint::CoprocStdout { fd: file, .. }) =
            self.entries
                .get(&fd)
                .filter(|entry| !entry.closed)
                .and_then(|entry| entry.read.clone())
        {
            let mut line = Vec::new();
            loop {
                match crate::fd::read_some(file.handle, 1) {
                    Ok(buf) if buf.is_empty() => break,
                    Ok(buf) => {
                        line.push(buf[0]);
                        if buf[0] == b'\n' {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if line.is_empty() {
                return None;
            }
            return Some(line);
        }
        let input = match self
            .entries
            .get(&fd)
            .filter(|entry| !entry.closed)?
            .read
            .as_ref()?
        {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => {
                input.clone()
            }
            _ => return None,
        };
        let mut input = input.borrow_mut();
        if input.offset >= input.data.len() {
            return Some(Vec::new());
        }
        let rest = &input.data[input.offset..];
        let line_len = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(rest.len(), |position| position + 1);
        let line = rest[..line_len].to_vec();
        input.offset += line_len;
        Some(line)
    }

    pub(crate) fn output_endpoint(&self, fd: u32) -> Option<FdWriteEndpoint> {
        self.entries.get(&fd)?.write.clone()
    }

    pub(crate) fn materialize_for_child(&self) -> BTreeMap<u32, MaterializedFd> {
        self.entries
            .iter()
            .filter(|(_, entry)| !entry.closed)
            .map(|(fd, entry)| {
                let read = entry.read.as_ref().map(|endpoint| match endpoint {
                    FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => {
                        let input = input.borrow();
                        MaterializedRead::Bytes(input.data[input.offset..].to_vec())
                    }
                    FdReadEndpoint::File(file) => MaterializedRead::File(file.path.clone()),
                    FdReadEndpoint::InheritedProcessStdin => {
                        MaterializedRead::InheritedProcessStdin
                    }
                    FdReadEndpoint::CoprocStdout { pid, .. } => {
                        MaterializedRead::CoprocStdout(*pid)
                    }
                    FdReadEndpoint::Directory => {
                        // rubash#1937: the snapshot has no directory
                        // shape; the rebound fd reads as empty
                        // instead of failing EISDIR (cross-boundary
                        // residual — the in-shell faces are exact).
                        MaterializedRead::Bytes(Vec::new())
                    }
                });
                let write = entry.write.as_ref().map(|endpoint| match endpoint {
                    FdWriteEndpoint::Stdout => MaterializedWrite::Stdout,
                    FdWriteEndpoint::Stderr => MaterializedWrite::Stderr,
                    FdWriteEndpoint::File(file) => MaterializedWrite::File(file.path.clone()),
                    FdWriteEndpoint::CoprocStdin { pid, .. } => {
                        MaterializedWrite::CoprocStdin(*pid)
                    }
                    FdWriteEndpoint::ProcessSubstitution { path, command, .. } => {
                        MaterializedWrite::ProcessSubstitution {
                            path: path.clone(),
                            command: command.clone(),
                        }
                    }
                    FdWriteEndpoint::FullDevice => {
                        // rubash#1898: materializing a /dev/full
                        // binding for a child stage hands it an
                        // absorbed sink file — the ENOSPC failure
                        // profile cannot cross this boundary (the
                        // in-shell write faces carry it exactly);
                        // GNU hands the child the real device.
                        let path = std::env::temp_dir().join(format!(
                            "rubash-full-sink-{}-{}",
                            std::process::id(),
                            fd
                        ));
                        let _ = std::fs::File::create(&path);
                        MaterializedWrite::File(path)
                    }
                });
                (*fd, MaterializedFd { read, write })
            })
            .collect()
    }

    fn entry_mut(&mut self, fd: u32, dynamic: bool) -> &mut FdEntry {
        self.entries.entry(fd).or_insert_with(|| FdEntry {
            read: None,
            write: None,
            closed: false,
            dynamic,
        })
    }

    fn occupied(entry: &FdEntry) -> bool {
        !entry.closed && (entry.read.is_some() || entry.write.is_some())
    }
}

impl PartialEq for FdReadEndpoint {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => Rc::ptr_eq(a, b),
            (Self::File(a), Self::File(b)) => Rc::ptr_eq(a, b),
            (Self::InheritedProcessStdin, Self::InheritedProcessStdin) => true,
            (Self::ProcessSubstitution(a), Self::ProcessSubstitution(b)) => Rc::ptr_eq(a, b),
            (Self::CoprocStdout { pid: a, .. }, Self::CoprocStdout { pid: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl PartialEq for FdWriteEndpoint {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Stdout, Self::Stdout) | (Self::Stderr, Self::Stderr) => true,
            // rubash#1898: /dev/full endpoints are interchangeable.
            (Self::FullDevice, Self::FullDevice) => true,
            (Self::File(a), Self::File(b)) => Rc::ptr_eq(a, b),
            (Self::CoprocStdin { pid: a, .. }, Self::CoprocStdin { pid: b, .. }) => a == b,
            (
                Self::ProcessSubstitution {
                    path: p1,
                    command: c1,
                    ..
                },
                Self::ProcessSubstitution {
                    path: p2,
                    command: c2,
                    ..
                },
            ) => p1 == p2 && c1 == c2,
            _ => false,
        }
    }
}

impl FdReadEndpoint {
    pub(crate) fn bytes(input: Vec<u8>) -> Self {
        Self::Text(Rc::new(RefCell::new(TextInput {
            data: input,
            offset: 0,
            timing: None,
        })))
    }

    pub(crate) fn text(input: impl Into<String>) -> Self {
        Self::bytes(input.into().into_bytes())
    }

    pub(crate) fn process_substitution(input: impl Into<String>) -> Self {
        Self::ProcessSubstitution(Rc::new(RefCell::new(TextInput {
            data: input.into().into_bytes(),
            offset: 0,
            timing: None,
        })))
    }

    /// Byte-payload form of `process_substitution` — a substitution body
    /// may produce arbitrary bytes (rubash#355 fd-carried words).
    pub(crate) fn process_substitution_bytes(input: Vec<u8>) -> Self {
        Self::ProcessSubstitution(Rc::new(RefCell::new(TextInput {
            data: input,
            offset: 0,
            timing: None,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_slots_reuse_closed_entries() {
        let mut table = FdTable::new();
        let first = table.allocate_dynamic();
        table.open_input(first, FdReadEndpoint::text("a\n"), true);
        let second = table.allocate_dynamic();
        table.open_input(second, FdReadEndpoint::text("b\n"), true);
        table.close(first);
        table.close(second);
        assert_eq!(table.allocate_dynamic(), first);
        table.open_input(first, FdReadEndpoint::text("c\n"), true);
        assert_eq!(table.allocate_dynamic(), second);
    }

    #[test]
    fn input_dup_shares_offset_and_move_closes_source() {
        let mut table = FdTable::new();
        table.open_input(10, FdReadEndpoint::text("one\ntwo\n"), true);
        assert_eq!(
            table
                .read_text(10, '\n', None, false, false, false)
                .as_deref(),
            Some("one")
        );
        table.move_input(11, 10).unwrap();
        assert!(table.is_closed(10));
        assert_eq!(
            table
                .read_text(11, '\n', None, false, false, false)
                .as_deref(),
            Some("two")
        );
    }

    #[test]
    fn input_offset_tracks_consumption_without_a_snapshot() {
        // perf-round F6: the per-record offset write-back must consult
        // the cursor through input_offset; it tracks read consumption
        // exactly like the snapshot's offset half, and stays None for
        // fds with no materialized endpoint.
        let mut table = FdTable::new();
        assert_eq!(table.input_offset(10), None);
        table.open_input(
            10,
            FdReadEndpoint::process_substitution_bytes(b"one\ntwo\n".to_vec()),
            true,
        );
        assert_eq!(table.input_offset(10), Some(0));
        assert_eq!(
            table.read_text(10, '\n', None, false, false, true),
            Some("one".to_string())
        );
        assert_eq!(table.input_offset(10), Some(4));
        assert_eq!(
            table.input_offset(10),
            Some(table.input_snapshot(10).unwrap().1)
        );
        assert_eq!(
            table.read_text(10, '\n', None, false, false, true),
            Some("two".to_string())
        );
        assert_eq!(table.input_offset(10), Some(8));
        assert_eq!(table.read_text(10, '\n', None, false, false, true), None);
        assert_eq!(table.input_offset(99), None);
    }

    #[test]
    fn materialization_does_not_consume_input() {
        let mut table = FdTable::new();
        table.open_input(10, FdReadEndpoint::text("value\n"), true);
        let materialized = table.materialize_for_child();
        assert_eq!(
            materialized[&10].read,
            Some(MaterializedRead::Bytes(b"value\n".to_vec()))
        );
        assert_eq!(table.input_snapshot(10).unwrap().1, 0);
    }

    #[test]
    fn materialization_preserves_invalid_input_bytes() {
        let mut table = FdTable::new();
        table.open_input(10, FdReadEndpoint::bytes(vec![0xff, b'\n']), true);
        let materialized = table.materialize_for_child();
        assert_eq!(
            materialized[&10].read,
            Some(MaterializedRead::Bytes(vec![0xff, b'\n']))
        );
        assert_eq!(
            table.input_snapshot_bytes(10).unwrap(),
            (vec![0xff, b'\n'], 0)
        );
    }
}
