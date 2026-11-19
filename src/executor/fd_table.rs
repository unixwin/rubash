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
    /// rubash#2502: the on-disk file this buffered copy was
    /// materialized FROM, when the replaced endpoint was a regular
    /// file that an alias-spelling open re-opens fresh
    /// (`FileFd::reopens_fresh`). The stage-0 materialization
    /// (pipeline_exec::materialize_stage0_alias_source) replaces a
    /// drained handle-backed endpoint with this copy; without the
    /// origin, the descriptor's identity is lost and a later
    /// `< /dev/fd/N` open of the same slot falls to the copy's
    /// remainder instead of GNU's fresh open of the file
    /// (redir.c:725 — the alias spelling is a plain open of the
    /// magic link). Only the stage-0 materialization sets it; every
    /// other TextInput (heredocs, mirrors, channels) leaves it None.
    origin_path: Option<PathBuf>,
}

impl TextInput {
    /// rubash#2385: append bytes at the stream's tail — the write
    /// half of a complementary-end open (see
    /// `FdWriteEndpoint::PipeChannel`). The read cursor never moves:
    /// GNU's write() into a pipe lands behind every byte already in
    /// the stream, consumed or not (probe y05: a partially-read
    /// stream serves its remainder first, then the appended bytes).
    pub(crate) fn append_bytes(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    /// A fresh in-memory stream over already-materialized bytes
    /// (the mapfile record source's shared cursor, rubash#3660).
    pub(crate) fn from_data(data: Vec<u8>) -> Self {
        Self {
            data,
            offset: 0,
            timing: None,
            origin_path: None,
        }
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    pub(crate) fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
    }
}

/// rubash#2385: one complementary (write) end of an in-memory byte
/// channel — a handle onto the SAME `TextInput` queue a read
/// endpoint serves. Identity is the queue allocation itself
/// (pointer equality): two ends are "the same endpoint" exactly
/// when they append to the same stream, which is what the endpoint
/// equality consumers (dup bookkeeping, target merging) mean.
#[derive(Debug, Clone)]
pub(crate) struct PipeChannelEnd(pub Rc<RefCell<TextInput>>);

impl PipeChannelEnd {
    /// Append at the shared queue's tail (see
    /// `TextInput::append_bytes`).
    pub(crate) fn append(&self, bytes: &[u8]) {
        self.0.borrow_mut().append_bytes(bytes);
    }
}

impl PartialEq for PipeChannelEnd {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for PipeChannelEnd {}

/// A real kernel object behind a file-backed fd. `Rc`-shared across dup'd
/// slots and fork'd (cloned) fd tables so the file offset stays shared —
/// POSIX open file description semantics (governance doc 3.6, P6/P10).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FileFd {
    pub(crate) handle: crate::fd::HANDLE,
    /// rubash#3213: set once the kernel descriptor behind `handle`
    /// is retired at the logical close point (see close_now). GNU
    /// closes the descriptor when the redirect performs it
    /// (redir.c:1264 close_buffered_fd), not when the last
    /// userspace reference happens to drop — a snapshot's stale
    /// Rc<FileFd> (fd_var_apply_snapshot binds, pending-close
    /// undo stacks) used to keep the materialized descriptor open
    /// past `exec {v}>&-` and the varredir_close command-end drain,
    /// leaking the number into /proc/$$/fd. The flag also keeps a
    /// stale holder's later Drop from re-closing a recycled number.
    pub(crate) closed: std::cell::Cell<bool>,
    /// rubash#3917: this handle is a startup-adopted copy of an
    /// inherited descriptor (see register_inherited_process_fds).
    /// The copy is the executor's private duplicate (rubash#1838),
    /// parked on whatever kernel number the adoption dup landed
    /// on — a number the shell never promised to keep. When a
    /// materialization or placement needs that exact number, the
    /// copy is relocated out of the way (evict_adopted_copy_from)
    /// instead of refusing the bind: GNU has no such copy at all
    /// (its table names the inherited descriptor by its own
    /// number, redir.c:1160), and GNU itself relocates internal
    /// descriptors that would collide with a needed number
    /// (move_to_high_fd, general.c:702). Only adopted copies are
    /// evictable; a real binding's handle is never relocated. The
    /// copy is relocated, never borrowed: the table never names a
    /// descriptor it does not own, so no engine lifecycle can
    /// close a host or sibling-session descriptor by number
    /// (rubash#1838).
    pub(crate) adopted: bool,
    pub(crate) path: PathBuf,
}

impl FileFd {
    pub(crate) fn open_read(path: PathBuf) -> std::io::Result<Rc<Self>> {
        crate::fd::open_file_read(&path).map(|handle| {
            Rc::new(Self {
                handle,
                closed: std::cell::Cell::new(false),
                adopted: false,
                path,
            })
        })
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
        Ok(Rc::new(Self {
            handle,
            closed: std::cell::Cell::new(false),
            adopted: false,
            path,
        }))
    }

    pub(crate) fn open_readwrite(path: PathBuf) -> std::io::Result<Rc<Self>> {
        crate::fd::open_file_readwrite(&path).map(|handle| {
            Rc::new(Self {
                handle,
                closed: std::cell::Cell::new(false),
                adopted: false,
                path,
            })
        })
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
                    closed: std::cell::Cell::new(false),
                    adopted: false,
                    path: path.clone(),
                }),
                Rc::new(Self {
                    handle: write,
                    closed: std::cell::Cell::new(false),
                    adopted: false,
                    path,
                }),
            ));
        }
        let file = Self::open_readwrite(path)?;
        Ok((file.clone(), file))
    }

    /// rubash#2256: whether an open of this endpoint through its
    /// `/dev/fd/N` alias spelling is a FRESH open of `path` (a new
    /// open file description at offset 0) rather than a dup of this
    /// handle. GNU's alias open is the plain open() of the magic
    /// link — redir.c's RF_DEVFD dup special case (redir.c:603,
    /// :629) compiles only without HAVE_DEV_FD, and the oracle
    /// build defines it — so for a regular file the kernel hands
    /// back a new description, while pipes, sockets and devices
    /// cannot be re-opened that way and keep the dup (a pipe open
    /// attaches to the same stream, which the dup already models).
    /// The handle must be a disk file AND the stored path must
    /// itself name a regular file: synthetic spellings backed by a
    /// disk handle (the /dev/zero pump file) are not their target.
    pub(crate) fn reopens_fresh(&self) -> bool {
        crate::fd::is_disk_file(self.handle)
            && std::fs::metadata(&self.path).is_ok_and(|meta| meta.is_file())
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
        Rc::new(Self {
            handle,
            closed: std::cell::Cell::new(false),
            adopted: false,
            path,
        })
    }

    /// rubash#3213: retire the kernel descriptor NOW, at the
    /// logical close point — GNU's performed redirect close runs
    /// close(2) immediately (redir.c:1264 close_buffered_fd for
    /// r_close_this; the same fds disappear from /proc/$$/fd at
    /// once). Idempotent: every later reference — including this
    /// object's own eventual Drop and stale Rc holders the close
    /// outlived — observes the flag and skips the close(2), so a
    /// descriptor number the kernel recycled is never clobbered.
    pub(crate) fn close_now(&self) {
        if !self.closed.replace(true) {
            crate::fd::close_handle(self.handle);
        }
    }

    /// Whether close_now already retired the descriptor.
    pub(crate) fn is_retired(&self) -> bool {
        self.closed.get()
    }

    /// rubash#3706: duplicate this descriptor's kernel handle into a
    /// fresh FileFd naming the same open file description (shared
    /// offset and status flags) — the undo-save dup. GNU's undoable
    /// redirect saves the displaced descriptor BEFORE the redirect
    /// binds, with `fcntl (fd, F_DUPFD, SHELL_FD_BASE)` (redir.c:965
    /// for input, :1082 for duplicating, :1144 for move; SHELL_FD_BASE
    /// is 10, redir.c:86), and records the save with
    /// `add_undo_redirect` so `undo_redirections` can dup it back.
    /// The duplicate carries the table's close-on-exec discipline
    /// (unix: F_DUPFD_CLOEXEC, like `crate::fd::dup_adopted_fd`;
    /// Windows: `duplicate_handle` is already non-inheritable), so
    /// the spawn path still arranges child layouts explicitly and no
    /// saved descriptor ever leaks raw into a child.
    pub(crate) fn duplicate(&self) -> std::io::Result<Rc<Self>> {
        #[cfg(unix)]
        let handle = {
            // The SHELL_FD_BASE floor the doc comment above cites is
            // load-bearing, not cosmetic: a save parked on a free low
            // number is a phantom occupant of that number — the
            // persistent-exec placement mistakes the snapshot entry
            // holding it for a live binding there and refuses GNU's
            // unconditional dup2 (see crate::fd::dup_adopted_fd,
            // rubash#3917).
            let n = unsafe {
                libc::fcntl(
                    self.handle,
                    libc::F_DUPFD_CLOEXEC,
                    FdTable::SHELL_FD_BASE as libc::c_int,
                )
            };
            if n < 0 {
                return Err(std::io::Error::last_os_error());
            }
            n
        };
        #[cfg(windows)]
        let handle = crate::fd::duplicate_handle(self.handle)?;
        Ok(Rc::new(Self {
            handle,
            closed: std::cell::Cell::new(false),
            adopted: false,
            path: self.path.clone(),
        }))
    }
}

impl Drop for FileFd {
    fn drop(&mut self) {
        self.close_now();
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
    /// rubash#2385: the complementary (write) end of an in-memory
    /// byte channel — the write-mode open of a pipe read end whose
    /// stream the fd model materialized (`FdReadEndpoint::Text` /
    /// `::ProcessSubstitution`). GNU's open() of the fd's magic link
    /// attaches to the pipe's other end in the kernel (redir.c
    /// redir_open's plain open, redir.c:725); a materialized stream
    /// has no kernel pipe, so the write end is a handle onto the SAME
    /// queue: writes append at the tail through the shared Rc, reads
    /// consume from the shared cursor. Minted only by the alias
    /// layer's complementary resolver
    /// (`Executor::fd_alias_complementary_write_endpoint`), never by
    /// a dup — `>&N` of a read end keeps failing EBADF exactly like
    /// GNU's dup2 of a read-only description.
    PipeChannel(PipeChannelEnd),
    /// rubash#2042: an fd bound to a 0644-class synthetic /proc
    /// content entry (`/proc/self/status`, `/proc/<pid>/cmdline`).
    /// The open succeeds (procfs 0644) and every WRITE fails EINVAL —
    /// procfs content files without a write handler answer
    /// `write error: Invalid argument` (measured on real procfs,
    /// bash 5.3.20: `echo hi >&3` after `exec 3<>/proc/self/status`
    /// fails with rc 1 in the writer's name). No Windows handle
    /// reproduces a per-write EINVAL, so — exactly like FullDevice —
    /// the endpoint is carried symbolically and the in-shell write
    /// faces produce the failure. The read side stays the generated
    /// snapshot byte channel; the failed write never alters what a
    /// later read serves (GNU's observable too).
    ProcContentWrite,
}

impl FdReadEndpoint {
    /// rubash#2212: the (handle, path label) backing this endpoint for
    /// cross-process fd delivery when the endpoint names one of the
    /// process's own standard descriptors. GNU's forked children
    /// inherit the descriptor itself (execute_cmd.c fork inheritance;
    /// subst.c:6362 for the process_substitute fork), but the symbolic
    /// endpoint carries no FileFd, so the delivery transports resolve
    /// it to the process standard handle at spawn time. `None` for
    /// endpoints that carry their own FileFd and for endpoints with
    /// no descriptor behind them (Text snapshots, carriers — those
    /// ride their own delivery passes).
    pub(crate) fn standard_backing(&self) -> Option<(crate::fd::HANDLE, &'static str)> {
        match self {
            FdReadEndpoint::InheritedProcessStdin => {
                Some((crate::fd::process_std_handle(0), "ambient:stdin"))
            }
            _ => None,
        }
    }
}

impl FdWriteEndpoint {
    /// rubash#2212: the write-side twin of
    /// `FdReadEndpoint::standard_backing` — a `N>&1` / `N>&2` dup binds
    /// the shell's own stdout/stderr description into the table as
    /// the symbolic Stdout/Stderr endpoint (bats' `exec 3<&1` in
    /// bats_tee is the driving shape).
    pub(crate) fn standard_backing(&self) -> Option<(crate::fd::HANDLE, &'static str)> {
        match self {
            FdWriteEndpoint::Stdout => Some((crate::fd::process_std_handle(1), "ambient:stdout")),
            FdWriteEndpoint::Stderr => Some((crate::fd::process_std_handle(2), "ambient:stderr")),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FdEntry {
    pub(crate) read: Option<FdReadEndpoint>,
    pub(crate) write: Option<FdWriteEndpoint>,
    pub(crate) closed: bool,
    pub(crate) dynamic: bool,
}

impl FdEntry {
    /// rubash#3706: clone this entry with every kernel-backed
    /// endpoint replaced by a fresh duplicate of its descriptor
    /// (same open file description, shared offset) — the undo-save
    /// form. A plain Rc clone shares the descriptor the bind-time
    /// retire (`retire_read_arm` / `retire_write_arm`, rubash#3213)
    /// closes when the scope's redirect binds, so a scope that saved
    /// the clone restores a dead entry; the duplicate keeps the saved
    /// descriptor alive until the scope's restore replays it, exactly
    /// like GNU's `fcntl (fd, F_DUPFD, SHELL_FD_BASE)` save
    /// (redir.c:965/1082/1144). Only the File-backed endpoints carry
    /// a kernel descriptor (File, CoprocStdout/CoprocStdin); text,
    /// directory, marker and channel endpoints are in-memory or
    /// symbolic and clone exactly. A `<>` pair's two arms share one
    /// Rc (open_readwrite_pair); that sharing is preserved — the
    /// write arm reuses the read arm's duplicate — so closing one
    /// direction still retires the shared descriptor.
    pub(crate) fn dup_file_endpoints(&self) -> std::io::Result<Self> {
        let read = match &self.read {
            // A retired kernel endpoint is already dead — its descriptor
            // number may have been recycled onto an unrelated file, so
            // there is no live descriptor to duplicate; share the dead
            // endpoint like a symbolic one (rubash#3865). Duplicating it
            // would fcntl whatever now owns the number and silently bind
            // the snapshot to the wrong open file description.
            Some(FdReadEndpoint::File(file)) if !file.is_retired() => {
                Some(FdReadEndpoint::File(file.duplicate()?))
            }
            Some(FdReadEndpoint::CoprocStdout { pid, fd }) if !fd.is_retired() => {
                Some(FdReadEndpoint::CoprocStdout {
                    pid: *pid,
                    fd: fd.duplicate()?,
                })
            }
            other => other.clone(),
        };
        let write = match &self.write {
            Some(FdWriteEndpoint::File(file)) if !file.is_retired() => {
                // A `<>` pair shares ONE open file description
                // between its arms (open_readwrite_pair mints a
                // single Rc for both); reuse the read arm's
                // duplicate so the saved entry preserves that
                // sharing — closing one direction must retire the
                // shared descriptor, as GNU's close(2) closes the
                // whole descriptor.
                match (&self.read, &read) {
                    (Some(FdReadEndpoint::File(orig)), Some(FdReadEndpoint::File(dup)))
                        if Rc::ptr_eq(orig, file) =>
                    {
                        Some(FdWriteEndpoint::File(dup.clone()))
                    }
                    _ => Some(FdWriteEndpoint::File(file.duplicate()?)),
                }
            }
            Some(FdWriteEndpoint::CoprocStdin { pid, fd }) if !fd.is_retired() => {
                Some(FdWriteEndpoint::CoprocStdin {
                    pid: *pid,
                    fd: fd.duplicate()?,
                })
            }
            other => other.clone(),
        };
        Ok(Self {
            read,
            write,
            closed: self.closed,
            dynamic: self.dynamic,
        })
    }
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
    /// rubash#3944: the live `read`'s `-d` option word first raw byte
    /// (read.def:385), mirrored from the executor beside
    /// `read_delim_word_byte` when the read acquisition begins. The
    /// fd-table record scanners key their byte compare on it; serves
    /// outside a `read` (`select`) leave it `None`.
    pub(crate) read_delim_word_byte: Option<u8>,
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
    /// rubash#3917: while the persistent `exec` redirect walk runs
    /// (execute_stdio_only_exec_redirect with place_persistent),
    /// file-backed installs are placed onto their logical kernel
    /// descriptor with dup2 — GNU redir.c:1017/:1120 — so
    /// /proc/$$/fd/N names the file even when the number was
    /// already occupied (an inherited pipe in CI). Scoped redirects
    /// never set this: a scoped dup2 would lose a foreign occupant
    /// the virtual table cannot restore (the scoped undo only
    /// replays the table, not the kernel descriptor). Off Unix the
    /// flag is inert (no kernel placement exists).
    pub(crate) kernel_place_persistent: bool,
    /// rubash#3930 (round 5): the persistent-close analogue of
    /// `kernel_place_persistent` — armed only by the `exec`
    /// wrapper (execute_stdio_only_exec_redirect) at true
    /// persistent scope, so a persistent `exec N<&-` / `N>&-`
    /// (or move-source) close of an adopted entry also closes
    /// the inherited kernel descriptor N (GNU redir.c:1239 case
    /// r_close_this, redir.c:1267 close_buffered_fd). Scoped
    /// closes, subshells and compound brackets never set this:
    /// the scope restore only replays the table, and a kernel
    /// close is irreversible — landing it there would corrupt
    /// the outer scope's descriptors. Off Unix the flag is inert
    /// (no kernel close exists).
    pub(crate) kernel_close_persistent: bool,
    /// rubash#3917: the placement undo log — GNU's undo list
    /// (redir.c add_undo_redirect) ported: before a placement dup2
    /// replaces a kernel descriptor, the displaced occupant is
    /// saved on a CLOEXEC duplicate so a caller that provisionally
    /// applied the redirects (execute_exec's option-parse scope,
    /// whose EX_BADUSAGE outcome undoes them — redir.c's
    /// exec_redirection_undo_list) can roll the kernel back with
    /// `rollback_placements`; a caller that keeps the installs
    /// calls `commit_placements`, which closes the saves. Entries
    /// are raw handles owned by THIS table instance: snapshots
    /// (saved_table) start with an empty log, so a snapshot's
    /// commit/rollback can never close the live table's saves.
    pub(crate) placement_undo: Vec<PlacementUndo>,
}

/// rubash#3917: one placement undo record (see
/// FdTable::placement_undo): the kernel number a placement dup2
/// landed on, and the displaced occupant's saved duplicate
/// (None when the number was free — rollback then just closes
/// the number).
#[derive(Debug, Clone)]
pub(crate) struct PlacementUndo {
    pub(crate) fd: u32,
    pub(crate) displaced: Option<crate::fd::HANDLE>,
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

/// What one fed byte means for the record a byte-oriented reader is
/// accumulating (the caller has already appended the byte to its
/// accumulation buffer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FdScanAction {
    /// The byte is record data; keep scanning.
    Data,
    /// The byte is record data and completes the `-n`/`-N` budget:
    /// stop here (the byte stays in the record).
    DataLimit,
    /// The byte terminates the record as the delimiter: stop here and
    /// drop `drop` bytes from the tail of the accumulation (the
    /// delimiter is consumed but never returned).
    Delimiter { drop: usize },
}

/// Incremental replay of GNU's read loop (builtins/read.def) over a
/// raw byte stream, shared by the fd_table byte readers (in-memory
/// scan, byte-wise file reader, buffered file reader).
///
/// The loop's unit is one read_mbchar consumption: in a multibyte
/// locale a whole valid character, or the bytes of an invalid
/// sequence up to and including the byte that invalidated it (a
/// truncated tail is one unit); in a byte locale every byte is a
/// unit. The delimiter is tested ONLY at a unit start, before the
/// unit is consumed (read.def:834 compares the stream byte against
/// the FIRST byte of the `-d` word) — a multibyte character's inner
/// bytes never terminate a record, but when an invalid sequence is
/// invalidated BY the delimiter byte, read_mbchar's pushback
/// (delim_ind) makes that byte a unit start of its own, so it
/// terminates after all. The previous sequence match
/// (`delimiter_seq_ending_at` over the whole UTF-8 encoding) consumed
/// the continuation byte into the delimiter in a multibyte locale:
/// for stream `a C3 A9 b C3 A9` with `-d é` the second record lost
/// its leading 0xA9 (rubash#3291); it remains the rule in byte
/// locales, where the sequence forms are how the delimiter word's
/// bytes can appear (rubash#1062/#2662).
///
/// The `-n`/`-N` budget counts POST-ESCAPE units (rubash#1328) at
/// unit COMPLETION (read.def:884-886 `nr++` after read_mbchar): an
/// escape pair costs one unit, a backslash-newline continuation zero,
/// and the budget never stops inside a character — the earlier
/// lead-byte accounting stopped after a multibyte character's first
/// byte (rubash#3294). Escape state follows the read.def order at a
/// unit start: an escaped unit is data (never a terminator), a
/// backslash is the escape introducer (never a terminator without
/// -r; rubash#1300), only then does the delimiter compare run.
pub(crate) struct FdRecordScan {
    /// Multibyte mode: the delimiter's first stream byte
    /// (read_helpers::delimiter_stream_byte). None selects the
    /// byte-locale sequence forms.
    first_byte: Option<u8>,
    seqs: ReadDelimiterSeqs,
    char_limit: Option<usize>,
    exact: bool,
    raw: bool,
    /// Post-escape unit count (the -n/-N budget).
    chars: usize,
    /// The next unit is an escaped character (a backslash unit start
    /// was just consumed).
    escaped: bool,
    /// Continuation bytes still expected for the in-progress unit;
    /// zero means the next byte starts a unit.
    unit_left: usize,
    /// First byte of the in-progress unit (its identity for the
    /// post-escape count: an escaped newline costs zero).
    unit_first: u8,
    /// The in-progress unit is an escaped character.
    unit_escaped: bool,
}

impl FdRecordScan {
    pub(crate) fn new(
        delimiter: char,
        word_first_byte: Option<u8>,
        char_limit: Option<usize>,
        exact: bool,
        raw: bool,
    ) -> Self {
        let multi_byte = crate::locale::is_multi_byte();
        FdRecordScan {
            // rubash#3944: the operative delimiter byte is the `-d` WORD's
            // first raw byte (read.def:385 `delim = (unsigned char)
            // *list_optarg`), not the first byte of the widened char's
            // UTF-8 encoding — a byte word (`-d $'\377'`) widens to a
            // Latin-1 char whose encoding starts with 0xC3, a byte GNU
            // never compares. Callers without the word (select, mapfile
            // serves) pass None and keep the derived form.
            first_byte: multi_byte.then(|| {
                word_first_byte
                    .unwrap_or_else(|| super::read_helpers::delimiter_stream_byte(delimiter, true))
            }),
            seqs: read_delimiter_seqs(delimiter),
            char_limit,
            exact,
            raw,
            chars: 0,
            escaped: false,
            unit_left: 0,
            unit_first: 0,
            unit_escaped: false,
        }
    }

    /// Length of a delimiter sequence ending at `end` (exclusive) in
    /// the caller's accumulation, in byte-locale sequence mode; the
    /// caller computes it after appending the current byte. Always
    /// None in multibyte first-byte mode.
    pub(crate) fn sequence_ending_at(&self, bytes: &[u8], end: usize) -> Option<usize> {
        if self.first_byte.is_some() {
            return None;
        }
        delimiter_seq_ending_at(&self.seqs, bytes, end)
    }

    /// Begin the unit whose first byte is `byte`; returns its action
    /// when the unit completes on this same byte (width one).
    fn begin_unit(&mut self, byte: u8, escaped: bool) -> Option<FdScanAction> {
        self.unit_first = byte;
        self.unit_escaped = escaped;
        let width = if self.first_byte.is_some() {
            super::read_helpers::utf8_lead_width(byte)
        } else {
            1
        };
        self.unit_left = width - 1;
        if self.unit_left == 0 {
            Some(self.complete_unit())
        } else {
            None
        }
    }

    /// Count the just-completed unit (post-escape rules) and report
    /// whether the budget stops the record at this byte.
    fn complete_unit(&mut self) -> FdScanAction {
        if self.unit_escaped {
            if self.unit_first != b'\n' {
                self.chars += 1;
            }
        } else {
            self.chars += 1;
        }
        if self.char_limit.is_some_and(|limit| self.chars >= limit) {
            FdScanAction::DataLimit
        } else {
            FdScanAction::Data
        }
    }

    /// Feed one byte (already appended to the caller's accumulation).
    /// `seq_end` is the caller's `sequence_ending_at` result for this
    /// byte (byte-locale mode only).
    pub(crate) fn on_byte(&mut self, byte: u8, seq_end: Option<usize>) -> FdScanAction {
        if self.unit_left > 0 {
            // Inside a unit: continuation bytes are data. A
            // non-continuation byte invalidates the sequence —
            // read_mbchar consumes it into the unit, EXCEPT the
            // delimiter byte, which is pushed back and terminates
            // the record as the next unit start (delim_ind).
            if (byte & 0xC0) == 0x80 {
                self.unit_left -= 1;
                if self.unit_left == 0 {
                    return self.complete_unit();
                }
                return FdScanAction::Data;
            }
            if !self.exact && self.first_byte == Some(byte) {
                let _ = self.complete_unit();
                return FdScanAction::Delimiter { drop: 1 };
            }
            self.unit_left = 0;
            return self.complete_unit();
        }
        // Unit start. An escaped unit is data, never a terminator.
        if self.escaped {
            self.escaped = false;
            if let Some(action) = self.begin_unit(byte, true) {
                return action;
            }
            return FdScanAction::Data;
        }
        // A backslash introduces an escape (never a terminator).
        if !self.raw && byte == b'\\' {
            self.escaped = true;
            return FdScanAction::Data;
        }
        // The delimiter compare (read.def:834), before consumption.
        if !self.exact {
            if let Some(delim) = self.first_byte {
                if byte == delim {
                    return FdScanAction::Delimiter { drop: 1 };
                }
            } else if let Some(len) = seq_end {
                return FdScanAction::Delimiter { drop: len };
            }
        }
        if let Some(action) = self.begin_unit(byte, false) {
            return action;
        }
        FdScanAction::Data
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
            read_delim_word_byte: None,
            stdout_alias_generation: std::collections::HashMap::new(),
            kernel_place_persistent: false,
            kernel_close_persistent: false,
            placement_undo: Vec::new(),
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
        // rubash#2476: a process BORN with fd 1 closed keeps it
        // closed in the table. GNU never reopens it: builtin output
        // fails at sh_chkwrite (builtins/common.c:319) and reports
        // `write error: Bad file descriptor` with status 1. The Rust
        // runtime reopens a closed standard fd as /dev/null before
        // main, so the live descriptor cannot answer this — the
        // at-exec capture in the fd layer can (crate::fd::
        // std_fd_closed_at_exec). A fd 1 genuinely redirected from
        // /dev/null was open at exec and keeps the Stdout endpoint.
        // Scoped to fd 1 here: fd 2's diagnostic routing has its
        // own inherited-state machinery, and a born-closed fd 2 is
        // observably identical (diagnostics are lost either way).
        // fd 0 is seeded immediately below (rubash#2503).
        if crate::fd::std_fd_closed_at_exec(1) {
            table.close(1);
        }
        // rubash#2503: fd 0 gets the same birth seed. The machinery
        // the #2476 scoping note set fd 0 aside for already
        // predicates on the closed bit (function_stdin_is_command_
        // source requires !is_closed(0)), and with the entry closed
        // the readers behave as GNU's do on the closed descriptor —
        // the faces an in-session `exec 0<&-` already produced:
        // read reports `read: 0: read error: Bad file descriptor`
        // (read.def:911) instead of a silent EOF, and the stdin
        // consumers surface the descriptor's EBADF instead of
        // reading the runtime's /dev/null stand-in.
        if crate::fd::std_fd_closed_at_exec(0) {
            table.close(0);
        }
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
        // rubash#2530: fd 0's birth seed above claims only the read
        // arm (InheritedProcessStdin), but a process can be BORN with
        // fd 0 open read-write — a parent shell's `<> file` spawn
        // delivery (GNU make_cmd.c:682-683 r_input_output opens
        // O_RDWR|O_CREAT, redir.c:725 redir_open), a terminal, an
        // RDWR /dev/null. GNU keeps no registry to consult: a dup
        // source is validated against the kernel itself
        // (redir.c:1160, F_GETFD in do_redirection_internal's r_dup
        // arm), so a write to fd 0 (`echo x >&0`) succeeds whenever
        // the inherited description is writable. Adopt the write arm
        // from the kernel exactly like the fd>=3 arms below: a
        // duplicate of descriptor 0 itself, so the write side shares
        // the ONE open file description — and its offset — with the
        // read side. Only the unclaimed birth seed is reconciled: a
        // slot the respawn restore claimed (a File read arm) and the
        // born-closed seed (#2503) keep their restored state.
        let fd0_unclaimed_birth_seed = self.entries.get(&0).is_some_and(|entry| {
            !entry.closed
                && entry.write.is_none()
                && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
        });
        if fd0_unclaimed_birth_seed {
            if let Some((_, true)) = crate::fd::fd_directions(0) {
                if let Some(handle) = crate::fd::dup_adopted_fd(0) {
                    let file = Rc::new(FileFd {
                        handle,
                        closed: std::cell::Cell::new(false),
                        adopted: true,
                        path: crate::fd::inherited_fd_path(0),
                    });
                    // rubash#3655: the adopted write arm joins the
                    // birth read arm on the SAME inherited
                    // description — one open_readwrite install, or
                    // the output bind would retire the read arm.
                    self.open_readwrite(
                        0,
                        FdReadEndpoint::InheritedProcessStdin,
                        FdWriteEndpoint::File(file),
                        false,
                    );
                }
            }
        }
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
                closed: std::cell::Cell::new(false),
                adopted: true,
                path: crate::fd::inherited_fd_path(fd),
            });
            if readable && writable {
                // rubash#3655: one inherited description open in
                // both directions — a single open_readwrite install,
                // never two sequential binds.
                self.open_readwrite(
                    fd,
                    FdReadEndpoint::File(file.clone()),
                    FdWriteEndpoint::File(file),
                    false,
                );
            } else if readable {
                self.open_input(fd, FdReadEndpoint::File(file.clone()), false);
            } else if writable {
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

    /// rubash#2282: the same lowest-free walk, with extra slots counted
    /// as occupied even though they are not fd-table entries — the
    /// numbered redirect slots the command's ordered walk has already
    /// bound (or, on faces that apply after the whole list, will bind).
    /// GNU's do_redirections is one ordered walk over one descriptor
    /// space (redir.c:237): a numbered entry binds its slot in the same
    /// space the REDIR_VARASSIGN F_DUPFD allocation (redir.c:940)
    /// searches, so `cmd 10<f2 {v}<f1` allocates v=11, never the
    /// pending 10. The spawn-delivery helper (fd_var_delivery_fd)
    /// already avoids these slots when it computes the child's
    /// descriptor; the applier's allocation is the same computation on
    /// the parent's side of the fork.
    pub(crate) fn allocate_dynamic_with_limit_avoiding(
        &mut self,
        limit: Option<u32>,
        avoid: &std::collections::HashSet<u32>,
    ) -> Option<u32> {
        if avoid.is_empty() {
            return self.allocate_dynamic_with_limit(limit);
        }
        let upper = limit.unwrap_or(Self::UNLIMITED_OPEN_FILES_CEILING) as usize;
        if upper <= 10 {
            return None;
        }
        // Merge the avoided slots into the occupied walk in descriptor
        // order and stop at the first gap, exactly as the base walk
        // does over table entries alone.
        let mut occupied: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, entry)| Self::occupied(entry))
            .map(|(fd, _)| *fd)
            .collect();
        occupied.extend(avoid.iter().copied());
        occupied.sort_unstable();
        occupied.dedup();
        let mut candidate: usize = 10;
        for fd in occupied {
            if (fd as usize) < candidate {
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

    /// rubash#3213: GNU's floor for varredir/save descriptors
    /// (redir.c:86 SHELL_FD_BASE). Slots at or above it are
    /// materialized as real process descriptors (see
    /// materialize_read_endpoint); 0/1/2 are the shell's virtualized
    /// stdio and low fixed fds (3-9) keep current behavior.
    const SHELL_FD_BASE: u32 = 10;

    /// rubash#3213: dup a handle-backed endpoint's descriptor onto
    /// its logical number in this process (GNU redir.c:963-968:
    /// open, then fcntl(fd, F_DUPFD, SHELL_FD_BASE), then
    /// redir_varassign binds the number; the low original is
    /// closed), so /proc/$$/fd shows the number and readlink
    /// resolves the target. The table used to keep only a virtual
    /// entry (the FileFd's raw handle sat at some other low number),
    /// visible to children only through the spawn pin pass.
    /// Returns the materialized descriptor, or None when the
    /// endpoint must stay virtual: the number is already occupied
    /// in this process (another entry's raw handle, an inherited
    /// descriptor, a transient spawn pipe) or the dup fails — never
    /// clobber a live descriptor. Callers first relocate any
    /// startup-adopted copy parked on the number
    /// (evict_adopted_copy_from): the copy is the executor's own
    /// private duplicate, never a binding, so it yields the
    /// number instead of refusing the bind.
    #[cfg(unix)]
    fn materialized_handle(fd: u32, raw: crate::fd::HANDLE) -> Option<crate::fd::HANDLE> {
        let target = fd as crate::fd::HANDLE;
        if raw == target {
            return Some(raw);
        }
        if unsafe { libc::fcntl(target, libc::F_GETFD) } >= 0 {
            return None;
        }
        if unsafe { libc::dup2(raw, target) } < 0 {
            return None;
        }
        Some(target)
    }

    /// rubash#3930 (round 5): the adopted `FileFd` backing logical
    /// fd `fd`, if either arm is a startup-adopted copy of an
    /// inherited descriptor (see register_inherited_process_fds).
    /// Read arm first, then write — a `<>` pair shares one Rc,
    /// and the fd-0 write arm is the only split case; coproc arms
    /// are shell-created, never adopted, so they are not consulted.
    /// A dup shares the source's Rc, so a dup'd entry reports the
    /// SOURCE's adopted copy — the caller must not assume the copy
    /// came from `fd` itself; the fstat identity check in
    /// close_adopted_process_fd is what proves kernel fd `fd` is
    /// that description.
    #[cfg(unix)]
    pub(crate) fn adopted_file(&self, fd: u32) -> Option<Rc<FileFd>> {
        let entry = self.entries.get(&fd)?;
        if let Some(FdReadEndpoint::File(file)) = &entry.read {
            if file.adopted {
                return Some(file.clone());
            }
        }
        if let Some(FdWriteEndpoint::File(file)) = &entry.write {
            if file.adopted {
                return Some(file.clone());
            }
        }
        None
    }

    /// rubash#3917: relocate every startup-adopted copy parked on
    /// kernel descriptor `target` to a fresh descriptor at or
    /// above SHELL_FD_BASE, updating the table entries that name
    /// it. Returns true when at least one copy was relocated.
    ///
    /// The adopted copy is a phantom occupant: GNU's table names
    /// the inherited descriptor by its own number (redir.c:1160),
    /// so no GNU number is ever blocked by bookkeeping; the copy
    /// exists only to keep the executor from closing host
    /// descriptors (rubash#1838). Blocking a materialization on
    /// it inverts that purpose — the pin face it produced was a
    /// {var} fd staying virtual because an adopted copy of an
    /// inherited pipe sat on the var's number. Relocation is a
    /// plain F_DUPFD_CLOEXEC of the copy (the same primitive GNU
    /// uses for its saves, redir.c:965/:1082/:1144, and the same
    /// idea as move_to_high_fd, general.c:702): the inherited
    /// descriptor itself, on its own number, is never touched.
    /// The victim FileFd then drops like any replaced endpoint
    /// and closes the now-redundant copy on the number, while
    /// the description it carried lives on at the relocated
    /// number under the replacement entry — the same replacement
    /// semantics materialize_read_endpoint already relies on.
    /// The copy is relocated, never borrowed: the table never
    /// names a descriptor it does not own, so no engine
    /// lifecycle can close a host or sibling-session descriptor
    /// by number (rubash#1838).
    #[cfg(unix)]
    fn evict_adopted_copy_from(&mut self, target: crate::fd::HANDLE) -> bool {
        let mut victims: Vec<Rc<FileFd>> = Vec::new();
        for entry in self.entries.values() {
            for file in [
                match &entry.read {
                    Some(FdReadEndpoint::File(f)) => Some(f),
                    _ => None,
                },
                match &entry.write {
                    Some(FdWriteEndpoint::File(f)) => Some(f),
                    _ => None,
                },
            ] {
                if let Some(f) = file {
                    if f.adopted
                        && !f.is_retired()
                        && f.handle == target
                        && !victims.iter().any(|v| Rc::ptr_eq(v, f))
                    {
                        victims.push(f.clone());
                    }
                }
            }
        }
        let mut relocated_any = false;
        for victim in victims {
            let relocated = unsafe {
                libc::fcntl(
                    target,
                    libc::F_DUPFD_CLOEXEC,
                    Self::SHELL_FD_BASE as libc::c_int,
                )
            };
            if relocated < 0 {
                continue;
            }
            let replacement = Rc::new(FileFd {
                handle: relocated,
                closed: std::cell::Cell::new(victim.closed.get()),
                adopted: true,
                path: victim.path.clone(),
            });
            for entry in self.entries.values_mut() {
                if let Some(FdReadEndpoint::File(f)) = &mut entry.read {
                    if Rc::ptr_eq(f, &victim) {
                        *f = replacement.clone();
                    }
                }
                if let Some(FdWriteEndpoint::File(f)) = &mut entry.write {
                    if Rc::ptr_eq(f, &victim) {
                        *f = replacement.clone();
                    }
                }
            }
            relocated_any = true;
        }
        relocated_any
    }

    /// rubash#3917: dup `file`'s handle onto kernel descriptor `fd`
    /// (GNU redir.c:1017/:1120 — the redirector IS the file,
    /// whatever occupied the number before), returning a fresh
    /// FileFd naming the placed descriptor, or None when the
    /// install must stay virtual. Placement covers every fixed
    /// number: rubash#3938 lifted the SHELL_FD_BASE ceiling — the
    /// free-only materialization used to leave `exec 12>file`
    /// virtual whenever 12 was already occupied (an earlier
    /// materialized holder, an inherited descriptor, a spawn
    /// transient), while GNU's dup2 (redir.c:1017) replaces the
    /// occupant unconditionally. The {var} allocations that live
    /// at/above SHELL_FD_BASE stay protected by the held scan below
    /// (a live table entry's own handle refuses the bind; their
    /// replay snapshots share the Rc and are retired by
    /// mark_displaced_by_dup2 when the number does move).
    /// Placement is refused when the number is held by another live
    /// table entry's non-retired File/Coproc handle, or when the
    /// kernel descriptor currently on the number is a directory (the
    /// cwd anchor) or a socket (the signal socketpair, a /dev/tcp
    /// socket) — those are rubash-internal or user state a
    /// replace-dup2 must not destroy. Pipes and regular files are
    /// replaced: that is the CI case (an inherited pipe on fd 3) and
    /// GNU's own behaviour. The caller drops the returned Rc's
    /// predecessor, which closes the source handle; the placed
    /// descriptor is left inheritable (dup2 clears CLOEXEC),
    /// matching GNU's `exec 3>file` and the #3213 materialized
    /// handles.
    #[cfg(unix)]
    fn place_persistent_file(&mut self, fd: u32, file: &Rc<FileFd>) -> Option<Rc<FileFd>> {
        if !self.kernel_place_persistent {
            return None;
        }
        let target = fd as crate::fd::HANDLE;
        if file.handle == target {
            return None;
        }
        // A startup-adopted copy parked on this number is not a
        // holder: relocate it first (see evict_adopted_copy_from),
        // so the held scan below only sees real bindings.
        self.evict_adopted_copy_from(target);
        // Another live entry's handle on this number: never clobber.
        let held = self.entries.iter().any(|(other, entry)| {
            *other != fd
                && (matches!(&entry.read,
                    Some(FdReadEndpoint::File(f))
                    | Some(FdReadEndpoint::CoprocStdout { fd: f, .. })
                    if !f.is_retired() && f.handle == target)
                    || matches!(&entry.write,
                        Some(FdWriteEndpoint::File(f))
                        | Some(FdWriteEndpoint::CoprocStdin { fd: f, .. })
                        if !f.is_retired() && f.handle == target))
        });
        if held {
            return None;
        }
        // A directory (cwd anchor) or socket (signal pair, /dev/tcp)
        // on the number: never clobber. Pipes and regular files are
        // replaced (the CI inherited-pipe case).
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(target, &mut st) } == 0 {
            // st_mode and the S_IF* constants disagree in width on
            // some targets (u32 vs u16 on linux/android) — compare
            // through u32 like external_file_builtins does.
            let mode = u32::from(st.st_mode) & u32::from(libc::S_IFMT);
            if mode == u32::from(libc::S_IFDIR) || mode == u32::from(libc::S_IFSOCK) {
                return None;
            }
        }
        // GNU saves the displaced descriptor BEFORE the dup2
        // whenever the redirection is undoable (redir.c
        // add_undo_redirect); the provisional exec scope rolls the
        // placement back from this save (see placement_undo). If
        // the occupant cannot be saved, stay virtual rather than
        // destroy it irrecoverably.
        let displaced = if unsafe { libc::fcntl(target, libc::F_GETFD) } >= 0 {
            let saved = unsafe {
                libc::fcntl(
                    target,
                    libc::F_DUPFD_CLOEXEC,
                    Self::SHELL_FD_BASE as crate::fd::HANDLE,
                )
            };
            if saved < 0 {
                return None;
            }
            Some(saved as crate::fd::HANDLE)
        } else {
            None
        };
        if unsafe { libc::dup2(file.handle, target) } < 0 {
            if let Some(saved) = displaced {
                unsafe { libc::close(saved) };
            }
            return None;
        }
        self.placement_undo.push(PlacementUndo { fd, displaced });
        // dup2 cleared the descriptor's close-on-exec bit; restore
        // it so the placed descriptor keeps the table's CLOEXEC
        // discipline (every executor-owned handle is CLOEXEC —
        // src/fd/unix.rs). Children receive the binding through
        // the pin/env delivery like any other entry; leaving the
        // bit clear would leak the raw descriptor into every
        // spawned child, bypassing the delivery's close/rebind
        // decisions (rubash#3917: a `{v}<&-` close for the child
        // must close a placed fd too).
        unsafe {
            libc::fcntl(target, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        Some(Rc::new(FileFd {
            handle: target,
            closed: std::cell::Cell::new(false),
            adopted: false,
            path: file.path.clone(),
        }))
    }

    /// rubash#3938 face A: the dup-redirect twin of
    /// place_persistent_file. GNU's duplicating redirector dup2's the
    /// SOURCE descriptor onto the target number (redir.c:1190, inside
    /// case r_duplicating_input/r_duplicating_output: "This is
    /// correct. 2>&1 means dup2 (1, 2);"), so after `exec 3>&1` the
    /// kernel descriptor 3 EXISTS and names the same open file
    /// description as fd 1 - `/proc/$$/fd/3` resolves like
    /// `/proc/$$/fd/1` (redir.c dup semantics). The free-only
    /// materialization (materialize_read_endpoint, rubash#3213) could
    /// not express this below SHELL_FD_BASE nor onto an occupied
    /// number, so the dup stayed a table-only alias and the number
    /// never materialized. Same guard suite as place_persistent_file:
    /// armed only by the persistent-exec walk; an adopted copy parked
    /// on the number is evicted first; a number held by another live
    /// entry's handle, a directory (cwd anchor) or a socket (signal
    /// pair) refuses the bind; the displaced occupant is saved into
    /// the placement undo log before the dup2 (redir.c:1101-1108
    /// add_undo_redirect for the dup case); the dup2 clears CLOEXEC
    /// and the table's discipline is restored after.
    ///
    /// The table arms stay the caller's clones: a kernel-backed source
    /// (File/Coproc) keeps sharing the source's Rc (the dup shares one
    /// open file description; the Rc share models that exactly), and a
    /// symbolic stdio source keeps its MARKER arm - the fd-fact marks
    /// (rubash#3432: FD_TARGET/ACCESS/STAT) are minted per endpoint
    /// kind, and a marker slot's fact faces answer from the access
    /// class without a path stat, which is what keeps
    /// `[ -e /proc/$$/fd/N ]` true for a dup of the process stdio. The
    /// caller retires the target slot's displaced arms after this
    /// (dup2 closed whatever held the number) and installs the arms
    /// without the free-only materialization step.
    #[cfg(unix)]
    fn place_persistent_dup_arms(
        &mut self,
        target: u32,
        read: Option<FdReadEndpoint>,
        write: Option<FdWriteEndpoint>,
    ) -> (Option<FdReadEndpoint>, Option<FdWriteEndpoint>, bool) {
        if !self.kernel_place_persistent {
            return (read, write, false);
        }
        // Source number: a kernel-backed arm's own descriptor, else the
        // process stdio a symbolic stdio arm stands for. A stdout
        // capture active in-process refuses: the capture is a
        // rubash-internal buffer the kernel number does not name.
        let source = match read.as_ref() {
            Some(FdReadEndpoint::File(f)) => Some(f.handle),
            Some(FdReadEndpoint::CoprocStdout { fd: f, .. }) => Some(f.handle),
            Some(FdReadEndpoint::InheritedProcessStdin) => Some(0),
            _ => match write.as_ref() {
                Some(FdWriteEndpoint::File(f)) => Some(f.handle),
                Some(FdWriteEndpoint::CoprocStdin { fd: f, .. }) => Some(f.handle),
                Some(FdWriteEndpoint::Stdout) => {
                    if crate::executor::shell_options::stdout_capture_active() {
                        None
                    } else {
                        Some(1)
                    }
                }
                Some(FdWriteEndpoint::Stderr) => Some(2),
                _ => None,
            },
        };
        let Some(source) = source else {
            return (read, write, false);
        };
        let to = target as crate::fd::HANDLE;
        if source == to {
            return (read, write, false);
        }
        // The same "never destroy what we cannot name" guards as
        // place_persistent_file.
        self.evict_adopted_copy_from(to);
        let held = self.entries.iter().any(|(other, entry)| {
            *other != target
                && (matches!(&entry.read,
                    Some(FdReadEndpoint::File(f))
                    | Some(FdReadEndpoint::CoprocStdout { fd: f, .. })
                    if !f.is_retired() && f.handle == to)
                    || matches!(&entry.write,
                        Some(FdWriteEndpoint::File(f))
                        | Some(FdWriteEndpoint::CoprocStdin { fd: f, .. })
                        if !f.is_retired() && f.handle == to))
        });
        if held {
            return (read, write, false);
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(to, &mut st) } == 0 {
            let mode = u32::from(st.st_mode) & u32::from(libc::S_IFMT);
            if mode == u32::from(libc::S_IFDIR) || mode == u32::from(libc::S_IFSOCK) {
                return (read, write, false);
            }
        }
        // rubash#3938: the dup placement lands on a FREE number only.
        // An occupied number means another descriptor of this very
        // install chain (or a foreign binding) sits there - the
        // rubash#2385 complementary-write flow opens the O_WRONLY end
        // at the slot's number and then adds the read arm through
        // dup_input; dup2-ing the read source over that number would
        // destroy the fresh description behind the caller's back and
        // the kept arm would answer EBADF. GNU's dup2 would replace
        // the occupant (redir.c:1190); that face stays with the
        // pre-#3938 virtual model for the dup direction and is
        // documented in the lane report.
        if unsafe { libc::fcntl(to, libc::F_GETFD) } >= 0 {
            return (read, write, false);
        }
        if unsafe { libc::dup2(source, to) } < 0 {
            return (read, write, false);
        }
        self.placement_undo.push(PlacementUndo {
            fd: target,
            displaced: None,
        });
        unsafe {
            libc::fcntl(to, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        (read, write, true)
    }

    /// rubash#3938 face C: land a restored scope-snapshot entry back
    /// onto its logical descriptor. The scope saves
    /// (saved_entry / saved_table, rubash#3706/#3814) duplicate every
    /// kernel-backed endpoint (F_DUPFD_CLOEXEC at/above SHELL_FD_BASE)
    /// so the bind-time retire cannot kill the description the save
    /// names; when the restore replays such an entry into a slot whose
    /// number the scope's bind vacated, the entry used to keep naming
    /// the PARKED duplicate — kernel descriptor `fd` no longer
    /// existed and the private duplicate stayed open until process end
    /// (the /proc-visible dup-relationship leak). GNU's undo replays a
    /// r_move_input record: dup2 the save back onto the number, close
    /// the save (redir.c:1399 add_undo_fd_redirect; the move's dup2 at
    /// redir.c:1190 and its source close at :1230). This lands the
    /// same way: kernel `fd` free, the entry's kernel-backed arms
    /// naming handles other than the slot's number → dup2, rebuild
    /// the arms on the slot's number, close the parked duplicate once
    /// no other entry names it. Split-model entries (distinct
    /// read/write files) land each arm only when the slot's number is
    /// free for its own dup2 — which can hold for just one arm; the
    /// other keeps its parked handle, exactly the pre-fix shape for
    /// that arm. The landing never clobbers.
    #[cfg(unix)]
    pub(crate) fn land_restored_entry(&mut self, fd: u32, mut entry: FdEntry) -> FdEntry {
        if entry.closed {
            return entry;
        }
        // Read arm first, then the write arm. A shared two-arm entry
        // (both arms name ONE duplicate - a saved `<>` pair is one Rc
        // across the arms) lands ONCE: the read pass dup2's it onto
        // the slot, and the write pass - seeing the read arm already
        // sitting on the slot's number - rebuilds itself on the same
        // landed descriptor and drops the shared duplicate. Split-model
        // entries (distinct arm descriptions) land each arm on its own
        // turn; an arm whose slot number is taken keeps its parked
        // shape. The landing never clobbers.
        let (read_handle, read_path) = match entry.read.as_ref() {
            Some(FdReadEndpoint::File(f)) => (Some(f.handle), Some(f.path.clone())),
            Some(FdReadEndpoint::CoprocStdout { fd: f, .. }) => {
                (Some(f.handle), Some(f.path.clone()))
            }
            _ => (None, None),
        };
        let write_handle = match entry.write.as_ref() {
            Some(FdWriteEndpoint::File(f)) => Some(f.handle),
            Some(FdWriteEndpoint::CoprocStdin { fd: f, .. }) => Some(f.handle),
            _ => None,
        };
        let read_landed = self
            .land_handle_onto_slot(fd, read_handle, write_handle)
            .is_some();
        if read_landed {
            let landed = Self::landed_file(fd, read_path.unwrap_or_default());
            entry.read = entry.read.take().map(|endpoint| match endpoint {
                FdReadEndpoint::File(_) => FdReadEndpoint::File(landed.clone()),
                FdReadEndpoint::CoprocStdout { pid, .. } => {
                    FdReadEndpoint::CoprocStdout { pid, fd: landed }
                }
                other => other,
            });
        }
        let (write_handle_now, write_path) = match entry.write.as_ref() {
            Some(FdWriteEndpoint::File(f)) => (Some(f.handle), Some(f.path.clone())),
            Some(FdWriteEndpoint::CoprocStdin { fd: f, .. }) => {
                (Some(f.handle), Some(f.path.clone()))
            }
            _ => (None, None),
        };
        let read_handle_now = match entry.read.as_ref() {
            Some(FdReadEndpoint::File(f)) | Some(FdReadEndpoint::CoprocStdout { fd: f, .. }) => {
                Some(f.handle)
            }
            _ => None,
        };
        let write_shared_with_landed_read = read_landed && write_handle_now == read_handle;
        if write_shared_with_landed_read {
            // The shared duplicate's description now lives at the
            // slot's number (the read pass landed it); the duplicate
            // itself is dead weight - close it unless another table
            // entry still names the number.
            let still_named = self.entries.values().any(|entry| {
                entry
                    .read
                    .as_ref()
                    .and_then(|endpoint| match endpoint {
                        FdReadEndpoint::File(f) | FdReadEndpoint::CoprocStdout { fd: f, .. } => {
                            Some(f.handle)
                        }
                        _ => None,
                    })
                    .is_some_and(|h| Some(h) == write_handle_now)
                    || entry
                        .write
                        .as_ref()
                        .and_then(|endpoint| match endpoint {
                            FdWriteEndpoint::File(f)
                            | FdWriteEndpoint::CoprocStdin { fd: f, .. } => Some(f.handle),
                            _ => None,
                        })
                        .is_some_and(|h| Some(h) == write_handle_now)
            });
            if !still_named {
                unsafe {
                    libc::close(write_handle_now.expect("shared write handle"));
                }
            }
        }
        let write_landed = self
            .land_handle_onto_slot(fd, write_handle_now, read_handle_now)
            .is_some();
        if write_landed || write_shared_with_landed_read {
            let landed = Self::landed_file(fd, write_path.unwrap_or_default());
            entry.write = entry.write.take().map(|endpoint| match endpoint {
                FdWriteEndpoint::File(_) => FdWriteEndpoint::File(landed),
                FdWriteEndpoint::CoprocStdin { pid, .. } => {
                    FdWriteEndpoint::CoprocStdin { pid, fd: landed }
                }
                other => other,
            });
        }
        entry
    }

    #[cfg(not(unix))]
    pub(crate) fn land_restored_entry(&mut self, _fd: u32, entry: FdEntry) -> FdEntry {
        entry
    }

    /// rubash#3938 face C: the kernel half of the landing for one
    /// arm. `handle` is the arm's parked descriptor, `opposite` the
    /// entry's other arm's handle. Lands when the parked handle is
    /// not the slot's own number and the slot's kernel number is
    /// free (never clobber); after the dup2 the parked duplicate is
    /// closed once no other table entry names it. Answers the entry's
    /// path so the caller can rebuild the arm on the landed number.
    #[cfg(unix)]
    fn land_handle_onto_slot(
        &self,
        fd: u32,
        handle: Option<crate::fd::HANDLE>,
        opposite: Option<crate::fd::HANDLE>,
    ) -> Option<()> {
        let handle = handle?;
        let to = fd as crate::fd::HANDLE;
        if handle == to {
            return None;
        }
        // Never clobber: the slot's kernel number must be free.
        if unsafe { libc::fcntl(to, libc::F_GETFD) } >= 0 {
            return None;
        }
        // A shared two-arm entry (both arms name one description)
        // lands once: if the opposite arm already sits on the slot's
        // number, its description IS the landed one.
        if opposite == Some(to) {
            return None;
        }
        if unsafe { libc::dup2(handle, to) } < 0 {
            return None;
        }
        // The table's executor-owned handles carry FD_CLOEXEC; dup2
        // cleared it on the landed descriptor.
        unsafe {
            libc::fcntl(to, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        let shared = self.entries.values().any(|entry| {
            entry
                .read
                .as_ref()
                .and_then(|endpoint| match endpoint {
                    FdReadEndpoint::File(f) | FdReadEndpoint::CoprocStdout { fd: f, .. } => {
                        Some(f.handle)
                    }
                    _ => None,
                })
                .is_some_and(|h| h == handle)
                || entry
                    .write
                    .as_ref()
                    .and_then(|endpoint| match endpoint {
                        FdWriteEndpoint::File(f) | FdWriteEndpoint::CoprocStdin { fd: f, .. } => {
                            Some(f.handle)
                        }
                        _ => None,
                    })
                    .is_some_and(|h| h == handle)
        });
        if !shared {
            unsafe {
                libc::close(handle);
            }
        }
        Some(())
    }

    /// rubash#3938 face C: a fresh FileFd naming the landed slot.
    #[cfg(unix)]
    fn landed_file(fd: u32, path: std::path::PathBuf) -> Rc<FileFd> {
        Rc::new(FileFd {
            handle: fd as crate::fd::HANDLE,
            closed: std::cell::Cell::new(false),
            adopted: false,
            path,
        })
    }

    /// rubash#3917: the current placement-undo depth — a caller
    /// that provisionally applies persistent redirects records
    /// this mark first, then settles the placements it caused
    /// with commit_placements / rollback_placements.
    pub(crate) fn placement_mark(&self) -> usize {
        self.placement_undo.len()
    }

    /// rubash#3917: settle every placement above `mark` as
    /// permanent: close the saved displaced occupants (GNU
    /// disposes the exec undo list once the exec stands). A mark
    /// at or above the current depth is a no-op.
    pub(crate) fn commit_placements(&mut self, mark: usize) {
        while self.placement_undo.len() > mark {
            let undo = self.placement_undo.pop().expect("undo entry");
            if let Some(saved) = undo.displaced {
                #[cfg(unix)]
                unsafe {
                    libc::close(saved);
                }
                #[cfg(not(unix))]
                let _ = saved;
            }
        }
    }

    /// rubash#3917: undo every placement above `mark`, newest
    /// first (GNU undo_exec_redirects): a saved displaced
    /// occupant is dup2'd back onto its number and its save
    /// closed; a number that was free is closed. The placed
    /// entries are marked retired so dropping them cannot close
    /// the restored descriptor a second time. The caller must
    /// discard the table's entries for the rolled-back numbers
    /// (execute_exec restores its pre-walk snapshot immediately
    /// after rolling back) — the entries still name the placed
    /// files, which the kernel no longer holds on those numbers.
    pub(crate) fn rollback_placements(&mut self, mark: usize) {
        while self.placement_undo.len() > mark {
            let undo = self.placement_undo.pop().expect("undo entry");
            #[cfg(unix)]
            {
                let target = undo.fd as crate::fd::HANDLE;
                match undo.displaced {
                    Some(saved) => unsafe {
                        libc::dup2(saved, target);
                        libc::close(saved);
                    },
                    None => unsafe {
                        libc::close(target);
                    },
                }
            }
            #[cfg(not(unix))]
            let _ = undo.fd;
            if let Some(entry) = self.entries.get_mut(&undo.fd) {
                if let Some(FdReadEndpoint::File(f) | FdReadEndpoint::CoprocStdout { fd: f, .. }) =
                    &entry.read
                {
                    if f.handle == undo.fd as crate::fd::HANDLE {
                        f.closed.set(true);
                    }
                }
                if let Some(FdWriteEndpoint::File(f) | FdWriteEndpoint::CoprocStdin { fd: f, .. }) =
                    &entry.write
                {
                    if f.handle == undo.fd as crate::fd::HANDLE {
                        f.closed.set(true);
                    }
                }
            }
        }
    }

    /// rubash#3917: after a replace-dup2 onto `fd`, the outgoing
    /// descriptor that held the number was already closed BY the
    /// dup2 — record the retirement on its FileFd so the retire
    /// step (retire_file) and a later Drop do not close the new
    /// descriptor now occupying the number. Only the FileFd whose
    /// handle IS the number is marked; a shared Rc keeps its other
    /// bindings alive (their handles differ). Off Unix this is a
    /// no-op (no placement ever runs).
    fn mark_displaced_by_dup2(file: &Rc<FileFd>, fd: u32) {
        #[cfg(unix)]
        {
            if file.handle == fd as crate::fd::HANDLE {
                file.closed.set(true);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (file, fd);
        }
    }

    /// rubash#3917: place a read endpoint's File/Coproc handle onto
    /// its logical kernel number for a persistent redirect (see
    /// place_persistent_file). Returns the endpoint to install and
    /// whether a replace-dup2 ran (the caller then marks the
    /// displaced outgoing descriptor retired — dup2 already closed
    /// it — so its retire step cannot close the new file).
    fn place_persistent_read_endpoint(
        &mut self,
        fd: u32,
        endpoint: FdReadEndpoint,
    ) -> (FdReadEndpoint, bool) {
        #[cfg(unix)]
        {
            return match endpoint {
                FdReadEndpoint::File(file) => match self.place_persistent_file(fd, &file) {
                    Some(placed) => (FdReadEndpoint::File(placed), true),
                    None => (FdReadEndpoint::File(file), false),
                },
                FdReadEndpoint::CoprocStdout { pid, fd: file } => {
                    match self.place_persistent_file(fd, &file) {
                        Some(placed) => (FdReadEndpoint::CoprocStdout { pid, fd: placed }, true),
                        None => (FdReadEndpoint::CoprocStdout { pid, fd: file }, false),
                    }
                }
                other => (other, false),
            };
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            (endpoint, false)
        }
    }

    /// rubash#3917: the write-side twin of
    /// place_persistent_read_endpoint.
    fn place_persistent_write_endpoint(
        &mut self,
        fd: u32,
        endpoint: FdWriteEndpoint,
    ) -> (FdWriteEndpoint, bool) {
        #[cfg(unix)]
        {
            return match endpoint {
                FdWriteEndpoint::File(file) => match self.place_persistent_file(fd, &file) {
                    Some(placed) => (FdWriteEndpoint::File(placed), true),
                    None => (FdWriteEndpoint::File(file), false),
                },
                FdWriteEndpoint::CoprocStdin { pid, fd: file } => {
                    match self.place_persistent_file(fd, &file) {
                        Some(placed) => (FdWriteEndpoint::CoprocStdin { pid, fd: placed }, true),
                        None => (FdWriteEndpoint::CoprocStdin { pid, fd: file }, false),
                    }
                }
                other => (other, false),
            };
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            (endpoint, false)
        }
    }

    /// rubash#3917: place a `<>` pair's shared FileFd once (see
    /// place_persistent_file) — both arms share one Rc, so the
    /// placed descriptor serves both directions on one open file
    /// description, as GNU's O_RDWR redirect does. An arm whose
    /// FileFd is a DIFFERENT file (the /dev/zero split model: read
    /// from the zero source, write to /dev/null) keeps its own
    /// endpoint — only the arm sharing the placed Rc is rebound.
    fn place_persistent_readwrite_endpoint(
        &mut self,
        fd: u32,
        read: FdReadEndpoint,
        write: FdWriteEndpoint,
    ) -> (FdReadEndpoint, FdWriteEndpoint, bool) {
        #[cfg(unix)]
        {
            // rubash#3938: refuse the placement when the OTHER arm's
            // own kernel handle currently sits ON the target number -
            // the rubash#2385 complementary write holds its OWN
            // O_WRONLY handle at the slot's number, and the placement
            // dup2 (the read arm's description onto that number)
            // would clobber it: the kept arm would answer EBADF. An
            // arm parked elsewhere (or the shared one-Rc pair) places
            // as before.
            let other_handle_on_target = match (&read, &write) {
                (FdReadEndpoint::File(read_file), FdWriteEndpoint::File(write_file)) => {
                    read_file.handle != write_file.handle
                        && write_file.handle == fd as crate::fd::HANDLE
                }
                _ => false,
            };
            if other_handle_on_target {
                return (read, write, false);
            }
            let file = match &read {
                FdReadEndpoint::File(file) => file.clone(),
                _ => match &write {
                    FdWriteEndpoint::File(file) => file.clone(),
                    _ => return (read, write, false),
                },
            };
            return match self.place_persistent_file(fd, &file) {
                Some(placed) => {
                    let read = match read {
                        FdReadEndpoint::File(f) if Rc::ptr_eq(&f, &file) => {
                            FdReadEndpoint::File(placed.clone())
                        }
                        other => other,
                    };
                    let write = match write {
                        FdWriteEndpoint::File(f) if Rc::ptr_eq(&f, &file) => {
                            FdWriteEndpoint::File(placed)
                        }
                        other => other,
                    };
                    (read, write, true)
                }
                None => (read, write, false),
            };
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            (read, write, false)
        }
    }

    /// rubash#3213: materialize a read endpoint at its logical
    /// number (see materialized_handle). The replacement FileFd owns
    /// the materialized descriptor, so every existing close path —
    /// which drops the endpoint — closes it; no new bookkeeping.
    /// Off Unix this is the identity: there is no /proc view to
    /// serve and the virtual table is the whole model.
    fn materialize_read_endpoint(fd: u32, endpoint: FdReadEndpoint) -> FdReadEndpoint {
        #[cfg(unix)]
        {
            if fd < Self::SHELL_FD_BASE {
                return endpoint;
            }
            return match endpoint {
                FdReadEndpoint::File(file) => match Self::materialized_handle(fd, file.handle) {
                    Some(handle) => {
                        // rubash#3917: when the descriptor already
                        // sat on its number (raw == target — a dup
                        // of an adopted copy onto the copy's own
                        // parking number is the live case), the
                        // replacement below owns that number now;
                        // the source object must not close it on
                        // drop.
                        Self::mark_displaced_by_dup2(&file, fd);
                        FdReadEndpoint::File(Rc::new(FileFd {
                            handle,
                            closed: std::cell::Cell::new(false),
                            adopted: false,
                            path: file.path.clone(),
                        }))
                    }
                    None => FdReadEndpoint::File(file),
                },
                FdReadEndpoint::CoprocStdout { pid, fd: file } => {
                    match Self::materialized_handle(fd, file.handle) {
                        Some(handle) => {
                            // rubash#3917: see the File arm above.
                            Self::mark_displaced_by_dup2(&file, fd);
                            FdReadEndpoint::CoprocStdout {
                                pid,
                                fd: Rc::new(FileFd {
                                    handle,
                                    closed: std::cell::Cell::new(false),
                                    adopted: false,
                                    path: file.path.clone(),
                                }),
                            }
                        }
                        None => FdReadEndpoint::CoprocStdout { pid, fd: file },
                    }
                }
                other => other,
            };
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            endpoint
        }
    }

    /// rubash#3213: the write-side twin of
    /// materialize_read_endpoint.
    fn materialize_write_endpoint(fd: u32, endpoint: FdWriteEndpoint) -> FdWriteEndpoint {
        #[cfg(unix)]
        {
            if fd < Self::SHELL_FD_BASE {
                return endpoint;
            }
            return match endpoint {
                FdWriteEndpoint::File(file) => match Self::materialized_handle(fd, file.handle) {
                    Some(handle) => {
                        // rubash#3917: see materialize_read_endpoint.
                        Self::mark_displaced_by_dup2(&file, fd);
                        FdWriteEndpoint::File(Rc::new(FileFd {
                            handle,
                            closed: std::cell::Cell::new(false),
                            adopted: false,
                            path: file.path.clone(),
                        }))
                    }
                    None => FdWriteEndpoint::File(file),
                },
                FdWriteEndpoint::CoprocStdin { pid, fd: file } => {
                    match Self::materialized_handle(fd, file.handle) {
                        Some(handle) => {
                            // rubash#3917: see materialize_read_endpoint.
                            Self::mark_displaced_by_dup2(&file, fd);
                            FdWriteEndpoint::CoprocStdin {
                                pid,
                                fd: Rc::new(FileFd {
                                    handle,
                                    closed: std::cell::Cell::new(false),
                                    adopted: false,
                                    path: file.path.clone(),
                                }),
                            }
                        }
                        None => FdWriteEndpoint::CoprocStdin { pid, fd: file },
                    }
                }
                other => other,
            };
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            endpoint
        }
    }

    /// rubash#3213: the kernel descriptor a departing arm carried
    /// is retired the moment the table drops the arm — GNU closes
    /// it when the redirect performs (redir.c:1264
    /// close_buffered_fd for r_close_this; the RX_UNDOABLE twins
    /// save a dup first but still close the ORIGINAL number), not
    /// when the last Rust reference happens to drop. Snapshot and
    /// undo stores (fd_var_apply_snapshot binds, pending-close
    /// restore stacks) clone entries and keep an Rc<FileFd> alive
    /// past the logical close; without an explicit retirement the
    /// materialized descriptor stayed open until process teardown
    /// — /proc/$$/fd listed a number GNU already reported closed —
    /// and the delayed Drop then risked closing a recycled number.
    /// Skipped while another table arm still shares the FileFd: a
    /// dup kept virtual holds ONE Rc across slots, and retiring it
    /// on one slot's close would kill the description the siblings
    /// still name. On non-unix targets there is no /proc view to
    /// keep honest, so the drop timing stays as it was.
    fn retire_read_arm(&self, fd: u32, endpoint: Option<FdReadEndpoint>) {
        #[cfg(unix)]
        {
            let file = endpoint.and_then(|endpoint| match endpoint {
                FdReadEndpoint::File(file) | FdReadEndpoint::CoprocStdout { fd: file, .. } => {
                    Some(file)
                }
                _ => None,
            });
            self.retire_file(fd, file);
        }
        #[cfg(not(unix))]
        {
            let _ = (fd, endpoint);
        }
    }

    /// The write-side twin of retire_read_arm.
    fn retire_write_arm(&self, fd: u32, endpoint: Option<FdWriteEndpoint>) {
        #[cfg(unix)]
        {
            let file = endpoint.and_then(|endpoint| match endpoint {
                FdWriteEndpoint::File(file) | FdWriteEndpoint::CoprocStdin { fd: file, .. } => {
                    Some(file)
                }
                _ => None,
            });
            self.retire_file(fd, file);
        }
        #[cfg(not(unix))]
        {
            let _ = (fd, endpoint);
        }
    }

    /// Close `file`'s descriptor at the logical close point (see
    /// retire_read_arm), unless another arm anywhere in the table
    /// still holds the same FileFd — a shared Rc is one open file
    /// description and retiring it here would dead-end the sibling
    /// bindings (their own close retires it later).
    #[cfg(unix)]
    fn retire_file(&self, fd: u32, file: Option<Rc<FileFd>>) {
        let _ = fd;
        let Some(file) = file else { return };
        if file.is_retired() {
            return;
        }
        let still_bound = self.entries.values().any(|entry| {
            let read_shares = match &entry.read {
                Some(FdReadEndpoint::File(other))
                | Some(FdReadEndpoint::CoprocStdout { fd: other, .. }) => Rc::ptr_eq(other, &file),
                _ => false,
            };
            let write_shares = match &entry.write {
                Some(FdWriteEndpoint::File(other))
                | Some(FdWriteEndpoint::CoprocStdin { fd: other, .. }) => Rc::ptr_eq(other, &file),
                _ => false,
            };
            read_shares || write_shares
        });
        if !still_bound {
            file.close_now();
        }
    }

    /// rubash#3706: snapshot slot `fd` for a scope's undo record —
    /// the entry with its kernel-backed endpoints duplicated
    /// (`FdEntry::dup_file_endpoints`), so the descriptor the scope's
    /// redirect is about to displace stays alive until the scope's
    /// restore replays it. A plain clone would share the descriptor
    /// the bind-time retire closes (retire_read_arm /
    /// retire_write_arm, rubash#3213). Fails only when the process
    /// cannot duplicate a descriptor (fd exhaustion) — GNU fails the
    /// whole redirect the same way (redir.c REDIRECTION_ERROR on a
    /// failed F_DUPFD save).
    pub(crate) fn saved_entry(&self, fd: u32) -> std::io::Result<Option<FdEntry>> {
        match self.entries.get(&fd) {
            Some(entry) => Ok(Some(entry.dup_file_endpoints()?)),
            None => Ok(None),
        }
    }

    /// rubash#3814: whole-table fork-boundary snapshot — the
    /// save/restore half of GNU's forked-child descriptor copies
    /// (execute_cmd.c:1576 execute_in_subshell; subst.c:7143
    /// command_substitute). A plain `clone()` shares every
    /// `Rc<FileFd>`, so a close inside the region
    /// (`( : ) 8<&-`) retires the kernel descriptor the parent's
    /// restored entry still names — the parent then reads EBADF.
    /// Duplicate each kernel-backed endpoint exactly like
    /// `saved_entry` does per slot; the duplicate shares the open
    /// file description (dup(2)), so offset and write visibility
    /// behave as GNU's inherited descriptors do, while the closed
    /// flag and descriptor number stay region-local. Symbolic
    /// endpoints (Text / ProcessSubstitution / markers) carry no
    /// kernel handle and clone as before. A descriptor that cannot
    /// be duplicated (fd exhaustion) keeps its shared entry — the
    /// pre-fix behaviour for that slot only.
    pub(crate) fn saved_table(&self) -> Self {
        let mut table = self.clone();
        for entry in table.entries.values_mut() {
            if let Ok(dup) = entry.dup_file_endpoints() {
                *entry = dup;
            }
        }
        // rubash#3917: the undo log holds raw saved handles owned
        // by the live table — a snapshot must never commit or roll
        // them back (it would close the live table's saves). The
        // snapshot starts with an empty log; placements made while
        // it is live log against it normally.
        table.placement_undo.clear();
        // Placement arming belongs to the live walk on the live
        // executor: a snapshot (e.g. the table a command-
        // substitution clone starts from, taken while an outer
        // walk is armed) must not inherit it, or the clone's
        // ordinary redirect opens would dup2 the process's
        // descriptors.
        table.kernel_place_persistent = false;
        // rubash#3930 (round 5): the close arming belongs to the
        // live walk exactly like the placement arming above — a
        // snapshot must not inherit it, or the clone's closes
        // would land on the kernel.
        table.kernel_close_persistent = false;
        table
    }

    /// Install a new read description on `fd` (rubash#3655): GNU
    /// dup2 replaces the descriptor WHOLESALE — the previous
    /// description dies in both directions (redir.c
    /// do_redirection_internal applies each redirect with dup2 onto
    /// its redirector). A later `14>file` must therefore retire an
    /// earlier heredoc's read arm, or `read -u 14` would still see
    /// the heredoc text on a write-only descriptor. The `<>`
    /// read-write pair is the only dual-arm install and lands
    /// through open_readwrite, never through these two.
    pub(crate) fn open_input(&mut self, fd: u32, endpoint: FdReadEndpoint, dynamic: bool) {
        // rubash#3917: place the file onto its kernel number for a
        // persistent redirect BEFORE the entry is swapped (the
        // placement scans the table, so it cannot run under the
        // entry_mut borrow below).
        let (endpoint, placed) = self.place_persistent_read_endpoint(fd, endpoint);
        // rubash#3917: a startup-adopted copy parked on this
        // number yields it before materialization consults the
        // kernel (see evict_adopted_copy_from).
        #[cfg(unix)]
        if fd >= Self::SHELL_FD_BASE {
            self.evict_adopted_copy_from(fd as crate::fd::HANDLE);
        }
        let (outgoing_read, outgoing_write);
        {
            let entry = self.entry_mut(fd, dynamic);
            // rubash#3213: drop the previous endpoints BEFORE
            // materializing the new one — dup2 onto fd closes whatever
            // descriptor held that number, and dropping the old FileFd
            // afterwards would close the new descriptor a second time.
            outgoing_read = std::mem::replace(&mut entry.read, None);
            outgoing_write = std::mem::replace(&mut entry.write, None);
            // rubash#3917: the replace-dup2 already closed the
            // outgoing descriptor that held this number — mark it
            // retired so the retire step below cannot close the new
            // file now occupying the number.
            if placed {
                if let Some(FdReadEndpoint::File(f) | FdReadEndpoint::CoprocStdout { fd: f, .. }) =
                    &outgoing_read
                {
                    Self::mark_displaced_by_dup2(f, fd);
                }
                if let Some(FdWriteEndpoint::File(f) | FdWriteEndpoint::CoprocStdin { fd: f, .. }) =
                    &outgoing_write
                {
                    Self::mark_displaced_by_dup2(f, fd);
                }
            }
            let endpoint = Self::materialize_read_endpoint(fd, endpoint);
            entry.read = Some(endpoint);
            entry.closed = false;
        }
        self.retire_read_arm(fd, outgoing_read);
        self.retire_write_arm(fd, outgoing_write);
    }

    /// Install a new write description on `fd` (rubash#3655): the
    /// read-side twin of open_input — a later `14<<A` retires an
    /// earlier `14>file` write arm the same way, so the descriptor
    /// never carries arms from two different descriptions.
    pub(crate) fn open_output(&mut self, fd: u32, endpoint: FdWriteEndpoint, dynamic: bool) {
        // rubash#3917: place the file onto its kernel number for a
        // persistent redirect BEFORE the entry is swapped (see
        // open_input).
        let (endpoint, placed) = self.place_persistent_write_endpoint(fd, endpoint);
        // rubash#3917: see open_input.
        #[cfg(unix)]
        if fd >= Self::SHELL_FD_BASE {
            self.evict_adopted_copy_from(fd as crate::fd::HANDLE);
        }
        let (outgoing_read, outgoing_write);
        {
            let entry = self.entry_mut(fd, dynamic);
            // rubash#3213: drop the previous endpoints BEFORE
            // materializing the new one (see open_input).
            outgoing_read = std::mem::replace(&mut entry.read, None);
            outgoing_write = std::mem::replace(&mut entry.write, None);
            // rubash#3917: the replace-dup2 already closed the
            // outgoing descriptor that held this number — mark it
            // retired so the retire step below cannot close the new
            // file now occupying the number.
            if placed {
                if let Some(FdReadEndpoint::File(f) | FdReadEndpoint::CoprocStdout { fd: f, .. }) =
                    &outgoing_read
                {
                    Self::mark_displaced_by_dup2(f, fd);
                }
                if let Some(FdWriteEndpoint::File(f) | FdWriteEndpoint::CoprocStdin { fd: f, .. }) =
                    &outgoing_write
                {
                    Self::mark_displaced_by_dup2(f, fd);
                }
            }
            let endpoint = Self::materialize_write_endpoint(fd, endpoint);
            entry.write = Some(endpoint);
            entry.closed = false;
            self.stdout_alias_generation.remove(&fd);
        }
        self.retire_read_arm(fd, outgoing_read);
        self.retire_write_arm(fd, outgoing_write);
    }

    /// Install BOTH arms of one open file description on `fd` at
    /// once (rubash#3655: the `<>` r_input_output form — GNU
    /// redir.c:725 redir_open opens O_RDWR, ONE descriptor serving
    /// both directions). This is the only dual-arm install:
    /// open_input/open_output each retire the opposite arm (dup2
    /// wholesale), so a `<>` pair installed as two sequential calls
    /// would end single-direction.
    pub(crate) fn open_readwrite(
        &mut self,
        fd: u32,
        read: FdReadEndpoint,
        write: FdWriteEndpoint,
        dynamic: bool,
    ) {
        // rubash#3917: place the pair's shared file onto its kernel
        // number for a persistent redirect BEFORE the entry is
        // swapped (see open_input).
        let (read, write, placed) = self.place_persistent_readwrite_endpoint(fd, read, write);
        // rubash#3917: see open_input.
        #[cfg(unix)]
        if fd >= Self::SHELL_FD_BASE {
            self.evict_adopted_copy_from(fd as crate::fd::HANDLE);
        }
        let (outgoing_read, outgoing_write);
        {
            let entry = self.entry_mut(fd, dynamic);
            // rubash#3213: drop the previous endpoints BEFORE
            // materializing the new ones (see open_input).
            outgoing_read = std::mem::replace(&mut entry.read, None);
            outgoing_write = std::mem::replace(&mut entry.write, None);
            // rubash#3917: the replace-dup2 already closed the
            // outgoing descriptor that held this number — mark it
            // retired so the retire step below cannot close the new
            // file now occupying the number.
            if placed {
                if let Some(FdReadEndpoint::File(f) | FdReadEndpoint::CoprocStdout { fd: f, .. }) =
                    &outgoing_read
                {
                    Self::mark_displaced_by_dup2(f, fd);
                }
                if let Some(FdWriteEndpoint::File(f) | FdWriteEndpoint::CoprocStdin { fd: f, .. }) =
                    &outgoing_write
                {
                    Self::mark_displaced_by_dup2(f, fd);
                }
            }
            let read = Self::materialize_read_endpoint(fd, read);
            let write = Self::materialize_write_endpoint(fd, write);
            entry.read = Some(read);
            entry.write = Some(write);
            entry.closed = false;
            self.stdout_alias_generation.remove(&fd);
        }
        self.retire_read_arm(fd, outgoing_read);
        self.retire_write_arm(fd, outgoing_write);
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
        // rubash#3917: see open_input — the dup target's number
        // is yielded by any adopted copy parked on it before the
        // copies below materialize.
        #[cfg(unix)]
        if target >= Self::SHELL_FD_BASE {
            self.evict_adopted_copy_from(target as crate::fd::HANDLE);
        }
        // rubash#3938 face A: at the persistent-exec scope the dup
        // dup2's its source onto the target number (redir.c:1190),
        // like the file placement does (place_persistent_file,
        // redir.c:1017/:1120).
        #[cfg(unix)]
        let (read, write, dup_placed) = self.place_persistent_dup_arms(target, read, write);
        #[cfg(unix)]
        if dup_placed {
            // dup2 already closed whatever held the number: retire
            // the outgoing arms that name it so the retire step below
            // cannot close the placed descriptor a second time (the
            // mark_displaced_by_dup2 discipline open_input/open_output
            // apply to their placements).
            if let Some(entry) = self.entries.get_mut(&target) {
                if let Some(FdReadEndpoint::File(f))
                | Some(FdReadEndpoint::CoprocStdout { fd: f, .. }) = &entry.read
                {
                    Self::mark_displaced_by_dup2(f, target);
                }
                if let Some(FdWriteEndpoint::File(f))
                | Some(FdWriteEndpoint::CoprocStdin { fd: f, .. }) = &entry.write
                {
                    Self::mark_displaced_by_dup2(f, target);
                }
            }
        }
        let dynamic = self.is_dynamic(target);
        let (outgoing_read, outgoing_write);
        {
            let entry = self.entry_mut(target, dynamic);
            // rubash#3213: drop the target's previous endpoints BEFORE
            // materializing the copies (see open_input): dup2 onto the
            // target closes whatever descriptor held that number, and
            // dropping the old FileFd afterwards would close the new
            // descriptor a second time.
            outgoing_read = std::mem::replace(&mut entry.read, None);
            outgoing_write = std::mem::replace(&mut entry.write, None);
            #[cfg(unix)]
            if dup_placed {
                entry.read = read;
                entry.write = write;
            } else {
                entry.read = read.map(|endpoint| Self::materialize_read_endpoint(target, endpoint));
                entry.write =
                    write.map(|endpoint| Self::materialize_write_endpoint(target, endpoint));
            }
            #[cfg(not(unix))]
            {
                entry.read = read.map(|endpoint| Self::materialize_read_endpoint(target, endpoint));
                entry.write =
                    write.map(|endpoint| Self::materialize_write_endpoint(target, endpoint));
            }
            entry.closed = false;
        }
        self.retire_read_arm(target, outgoing_read);
        self.retire_write_arm(target, outgoing_write);
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
        let outgoing;
        {
            let entry = self.entry_mut(fd, false);
            outgoing = entry.read.take();
            entry.closed = entry.write.is_none();
        }
        self.retire_read_arm(fd, outgoing);
    }

    pub(crate) fn close_output(&mut self, fd: u32) {
        let outgoing;
        {
            let entry = self.entry_mut(fd, false);
            outgoing = entry.write.take();
            entry.closed = entry.read.is_none();
            self.stdout_alias_generation.remove(&fd);
        }
        self.retire_write_arm(fd, outgoing);
    }

    pub(crate) fn close(&mut self, fd: u32) {
        let (outgoing_read, outgoing_write);
        {
            let entry = self.entry_mut(fd, false);
            outgoing_read = entry.read.take();
            outgoing_write = entry.write.take();
            entry.closed = true;
        }
        self.retire_read_arm(fd, outgoing_read);
        self.retire_write_arm(fd, outgoing_write);
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
        // once more on the in-time prefix. The scan itself is the shared
        // FdRecordScan replay of the GNU read loop (unit consumption,
        // first-byte delimiter compare, post-escape budget).
        let scan = |slice: &[u8]| -> (usize, bool, usize, bool, usize) {
            let mut scan =
                FdRecordScan::new(delimiter, self.read_delim_word_byte, char_limit, exact, raw);
            let mut consumed = 0;
            let mut delimiter_found = false;
            let mut delimiter_len = 0usize;
            let mut limit_reached = false;
            let mut result_len = 0usize;
            for (index, byte) in slice.iter().copied().enumerate() {
                let seq_end = scan.sequence_ending_at(slice, index + 1);
                consumed = index + 1;
                match scan.on_byte(byte, seq_end) {
                    FdScanAction::Data => {
                        result_len = index + 1;
                    }
                    FdScanAction::DataLimit => {
                        result_len = index + 1;
                        limit_reached = true;
                        break;
                    }
                    FdScanAction::Delimiter { drop } => {
                        result_len = index + 1 - drop;
                        delimiter_found = true;
                        delimiter_len = drop;
                        break;
                    }
                }
            }
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
        let mut consumed = false;
        let mut delimiter_found = false;
        let mut limit_reached = false;
        // The shared FdRecordScan replay of the GNU read loop (unit
        // consumption, first-byte delimiter compare, post-escape
        // budget), as in read_bytes.
        let mut scan =
            FdRecordScan::new(delimiter, self.read_delim_word_byte, char_limit, exact, raw);
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
            result.push(byte);
            let seq_end = scan.sequence_ending_at(&result, result.len());
            match scan.on_byte(byte, seq_end) {
                FdScanAction::Data => {}
                FdScanAction::DataLimit => {
                    limit_reached = true;
                    break;
                }
                FdScanAction::Delimiter { drop } => {
                    // The delimiter is consumed but not returned.
                    result.truncate(result.len() - drop);
                    delimiter_found = true;
                    break;
                }
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
    /// loop exactly (the shared FdRecordScan replay: delimiter consumed
    /// but not returned in non-exact mode; the budget counts completed
    /// read_mbchar units).
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
        let mut buffer: Vec<u8> = Vec::new();
        let mut result: Vec<u8> = Vec::new();
        let mut consumed = false;
        let mut delimiter_found = false;
        let mut limit_reached = false;
        let mut rewind: Option<u64> = None;
        // The shared FdRecordScan replay of the GNU read loop;
        // persists across chunk boundaries.
        let mut scan =
            FdRecordScan::new(delimiter, self.read_delim_word_byte, char_limit, exact, raw);
        'chunks: loop {
            let scanned_from = buffer.len();
            match crate::fd::read_some(file.handle, 8192) {
                Ok(chunk) if chunk.is_empty() => break,
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(error) => {
                    // rubash#3777: the byte-wise loop records the first
                    // hard read error on an otherwise empty record
                    // (rubash#1495); the buffered loop swallowed it, so
                    // a directory bound to the fd read as silent EOF
                    // instead of `read: 0: read error: Is a directory`
                    // (read.def:911). Bytes already consumed keep the
                    // plain partial-record path, as in the byte-wise arm.
                    if result.is_empty() && self.read_error.is_none() {
                        self.read_error = Some(crate::posix_errors::message(&error));
                    }
                    break;
                }
            }
            for (index, &byte) in buffer[scanned_from..].iter().enumerate() {
                let absolute = scanned_from + index;
                consumed = true;
                result.push(byte);
                let seq_end = scan.sequence_ending_at(&result, result.len());
                match scan.on_byte(byte, seq_end) {
                    FdScanAction::Data => {}
                    FdScanAction::DataLimit => {
                        limit_reached = true;
                        rewind = Some(origin + (absolute as u64) + 1);
                        break 'chunks;
                    }
                    FdScanAction::Delimiter { drop } => {
                        // Delimiter consumed but not returned; everything
                        // after its end position is prefetched data to
                        // rewind over.
                        result.truncate(result.len() - drop);
                        delimiter_found = true;
                        rewind = Some(origin + (absolute as u64) + 1);
                        break 'chunks;
                    }
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
                    Err(error) => {
                        // rubash#2101: a hard read error on an
                        // otherwise empty drain is a fact a caller
                        // may own — the same first-error discipline
                        // read_file_bytes keeps for the read builtin
                        // (rubash#1495). A directory bound to the fd
                        // reads EISDIR; swallowing it made a draining
                        // consumer (the emulated cat) report success
                        // where GNU's reader fails with its own
                        // diagnostic. read_bytes resets the slot at
                        // entry, so recording here cannot leak into an
                        // unrelated later read.
                        if out.is_empty() && self.read_error.is_none() {
                            self.read_error = Some(crate::posix_errors::message(&error));
                        }
                        break;
                    }
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
        endpoint.consume_deferred()
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
                    FdWriteEndpoint::ProcContentWrite => {
                        // rubash#2042: same boundary as FullDevice —
                        // the child gets an absorbed sink file; the
                        // EINVAL write profile cannot cross into a
                        // child handle (the in-shell write faces
                        // carry it exactly); GNU hands the child the
                        // real procfs entry.
                        let path = std::env::temp_dir().join(format!(
                            "rubash-proc-sink-{}-{}",
                            std::process::id(),
                            fd
                        ));
                        let _ = std::fs::File::create(&path);
                        MaterializedWrite::File(path)
                    }
                    FdWriteEndpoint::PipeChannel(_) => {
                        // rubash#2385: same boundary as FullDevice —
                        // the in-memory queue cannot cross the spawn
                        // boundary, so a child inheriting a
                        // complementary write fd gets an absorbed
                        // sink file; the in-shell write faces append
                        // to the queue exactly. (Kernel-pipe
                        // complementary ends are plain File endpoints
                        // and DO reach children as working
                        // descriptors.) GNU hands the child the real
                        // pipe write end.
                        let path = std::env::temp_dir().join(format!(
                            "rubash-channel-sink-{}-{}",
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
            // rubash#2042: all 0644-class synthetic /proc write binds
            // are interchangeable, like the device class above.
            (Self::ProcContentWrite, Self::ProcContentWrite) => true,
            (Self::File(a), Self::File(b)) => Rc::ptr_eq(a, b),
            // rubash#2385: channel identity is the shared queue
            // (PipeChannelEnd's own pointer equality).
            (Self::PipeChannel(a), Self::PipeChannel(b)) => a == b,
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
    /// rubash#2602: perform the per-endpoint effect of
    /// [`FdTable::consume_all_text`] on this endpoint directly: a File
    /// endpoint's handle seeks to end, a buffered (Text /
    /// ProcessSubstitution) endpoint's shared cursor advances to its
    /// end. The deferred operand finish (DevOperandMaterialization's
    /// Drop) holds endpoint clones, not table access, so it settles
    /// through this; the endpoints are Rc-shared with the table's
    /// entries, so the effect is the same object the table would have
    /// advanced. Returns the buffered cursor for the Text arms, like
    /// consume_all_text.
    pub(crate) fn consume_deferred(&self) -> Option<usize> {
        match self {
            FdReadEndpoint::File(file) => {
                let _ = crate::fd::seek_end(file.handle);
                None
            }
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => {
                let mut input = input.borrow_mut();
                input.offset = input.data.len();
                Some(input.offset)
            }
            _ => None,
        }
    }

    pub(crate) fn bytes(input: Vec<u8>) -> Self {
        Self::Text(Rc::new(RefCell::new(TextInput {
            data: input,
            offset: 0,
            timing: None,
            origin_path: None,
        })))
    }

    /// rubash#2502: a buffered copy that stands in for a drained
    /// regular-file endpoint (the stage-0 alias materialization) —
    /// the copy remembers the file it was drained from so an
    /// alias-spelling open of the slot can still re-open that file
    /// fresh, exactly as GNU's open of /dev/fd/N does against the
    /// live descriptor (redir.c:725).
    pub(crate) fn bytes_with_origin(input: Vec<u8>, origin_path: Option<PathBuf>) -> Self {
        Self::Text(Rc::new(RefCell::new(TextInput {
            data: input,
            offset: 0,
            timing: None,
            origin_path,
        })))
    }

    /// rubash#2502: the file a buffered copy was materialized from
    /// (see `bytes_with_origin`); None for every other endpoint.
    pub(crate) fn text_origin_path(&self) -> Option<PathBuf> {
        match self {
            Self::Text(text) => text.borrow().origin_path.clone(),
            _ => None,
        }
    }

    /// rubash#3945: zero-timeout readiness of a materialized text stream.
    /// A stream carrying a production schedule stands for GNU's live pipe
    /// (rubash#1141): ready only once a byte past the stream's own offset
    /// has arrived on the schedule's virtual clock. An unscheduled stream
    /// is fully materialized — the temp-file shape GNU reads heredocs and
    /// herestrings from — and always ready. `None` for every non-text
    /// endpoint: the caller polls those kernel objects directly.
    pub(crate) fn timed_text_stream_ready(&self) -> Option<bool> {
        match self {
            Self::Text(input) | Self::ProcessSubstitution(input) => {
                let input = input.borrow();
                let timing = input.timing.clone()?;
                let ready = timing.borrow().available_now(input.offset);
                Some(ready)
            }
            _ => None,
        }
    }

    pub(crate) fn text(input: impl Into<String>) -> Self {
        Self::bytes(input.into().into_bytes())
    }

    pub(crate) fn process_substitution(input: impl Into<String>) -> Self {
        Self::ProcessSubstitution(Rc::new(RefCell::new(TextInput {
            data: input.into().into_bytes(),
            offset: 0,
            timing: None,
            origin_path: None,
        })))
    }

    /// Byte-payload form of `process_substitution` — a substitution body
    /// may produce arbitrary bytes (rubash#355 fd-carried words).
    pub(crate) fn process_substitution_bytes(input: Vec<u8>) -> Self {
        Self::ProcessSubstitution(Rc::new(RefCell::new(TextInput {
            data: input,
            offset: 0,
            timing: None,
            origin_path: None,
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

    #[test]
    fn output_bind_retires_read_arm() {
        // rubash#3655: dup2 wholesale — `exec 14<<A 14>file` leaves
        // fd 14 write-only; the heredoc text must not survive as a
        // read arm for a later `read -u 14`.
        let mut table = FdTable::new();
        table.open_input(14, FdReadEndpoint::text("first\n"), false);
        assert!(table.is_open_for_read(14));
        table.open_output(14, FdWriteEndpoint::Stdout, false);
        assert!(!table.is_open_for_read(14));
        assert!(table.is_open_for_write(14));
        assert!(table.read_endpoint(14).is_none());
    }

    #[test]
    fn input_bind_retires_write_arm() {
        // rubash#3655: the mirror direction — `exec 14>file 14<<A`
        // leaves fd 14 read-only on the heredoc.
        let mut table = FdTable::new();
        table.open_output(14, FdWriteEndpoint::Stdout, false);
        assert!(table.is_open_for_write(14));
        table.open_input(14, FdReadEndpoint::text("first\n"), false);
        assert!(!table.is_open_for_write(14));
        assert!(table.is_open_for_read(14));
        assert!(table.output_endpoint(14).is_none());
    }

    #[test]
    fn open_readwrite_keeps_both_arms() {
        // rubash#3655: the `<>` pair is ONE description — the only
        // dual-arm install — and survives as both arms.
        let mut table = FdTable::new();
        table.open_readwrite(
            14,
            FdReadEndpoint::text("first\n"),
            FdWriteEndpoint::Stdout,
            false,
        );
        assert!(table.is_open_for_read(14));
        assert!(table.is_open_for_write(14));
        // A later single-direction bind still retires the pair
        // wholesale (`exec 14<>f 14>g` leaves fd 14 write-only).
        table.open_output(14, FdWriteEndpoint::Stderr, false);
        assert!(!table.is_open_for_read(14));
        assert!(table.is_open_for_write(14));
    }
}
