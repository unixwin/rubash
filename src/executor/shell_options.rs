use super::types::FUNCTION_STDIN;
use super::*;
use crate::executor::glob::{pathname_expand_word, PathnameExpansion};

/// Resolve a /dev/tcp|udp port word the way GNU netopen's getaddrinfo
/// does: a numeric word is the port (an out-of-range numeric value is
/// truncated into the 16-bit sin_port by the resolver — GNU connects
/// somewhere for `99999` instead of failing), and a non-numeric word is
/// a service name looked up in /etc/services (getservbyname). Returns
/// None when the service name is unknown (EAI_SERVICE).
/// The system services database getservbyname walks: /etc/services on
/// unix; the resolver file the Windows TCP/IP stack reads on Windows
/// (rubash#1775 — resolve_net_service_port used to read /etc/services
/// unconditionally, so every service name failed on Windows with
/// "Servname not supported" even though the Windows file lists http).
/// Same file src/builtins/complete.rs consults for service-name
/// completion (rubash#1044), same fallback chain.
#[cfg(unix)]
fn services_db_path() -> std::path::PathBuf {
    std::path::PathBuf::from("/etc/services")
}

#[cfg(windows)]
fn services_db_path() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    root.join(r"System32\drivers\etc\services")
}

fn resolve_net_service_port(port_text: &str, udp: bool) -> Option<u16> {
    if let Ok(port) = port_text.parse::<u16>() {
        return Some(port);
    }
    if !port_text.is_empty() && port_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return port_text
            .parse::<u32>()
            .ok()
            .map(|value| (value % 65536) as u16);
    }
    let proto = if udp { "udp" } else { "tcp" };
    let services = std::fs::read_to_string(services_db_path()).ok()?;
    for line in services.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(name), Some(port_proto)) = (fields.next(), fields.next()) else {
            continue;
        };
        if name != port_text {
            continue;
        }
        let Some((port, service_proto)) = port_proto.split_once('/') else {
            continue;
        };
        if service_proto == proto {
            return port.parse().ok();
        }
    }
    None
}

/// A connected net-redirection endpoint in its native socket form
/// (rubash#1026). Reads go through the socket API (WSARecv/recv);
/// see Executor::connect_net_stream for why byte consumers use this
/// form instead of the handle-adopted File.
pub(in crate::executor) enum NetStream {
    Tcp(std::net::TcpStream),
    Udp(std::net::UdpSocket),
}

impl io::Read for NetStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            NetStream::Tcp(stream) => stream.read(buf),
            // UdpSocket has no Read impl; recv is its read (one
            // datagram per call, so read_to_end keeps collecting
            // until the peer closes or a read would block — the
            // same EOF profile GNU's read_comsub sees on a UDP
            // descriptor).
            NetStream::Udp(socket) => socket.recv(buf),
        }
    }
}

impl Executor {
    pub(in crate::executor) fn open_input_redirect(&self, target: &str) -> io::Result<File> {
        self.open_input_redirect_impl(target, true)
    }

    /// Redirect-open validity probe (GNU redir.c open_redir_file: the open
    /// itself never reads). A registered `<(cmd)` path must not be drained
    /// by a probe — the consumer (read/cat/child stdin) does that.
    pub(in crate::executor) fn probe_input_redirect(&self, target: &str) -> io::Result<File> {
        self.open_input_redirect_impl(target, false)
    }

    /// `<> word` validity probe: r_input_output opens O_RDWR|O_CREAT with
    /// mode 0666 (make_cmd.c:682, consumed by redir.c:702 redir_open), so
    /// probing a missing name CREATES it — the open is part of the
    /// redirection's side effects (rubash#264), not just a check.
    ///
    /// The probe asks the registry first (rubash#1935/#1963/#1964):
    /// the classes with no filesystem file behind the mapped path get
    /// their own open semantics instead of a raw open of a literal
    /// path that cannot exist — a net target connects, a closed fd
    /// alias fails ENOENT (the kernel open of the alias name decides,
    /// exactly like the input ladder), a synthetic /proc entry opens
    /// or refuses by its write class, a synthetic link opens its
    /// resolved target. Only plain filesystem spellings (and the null
    /// device, whose mapped open is the device) reach the raw open.
    /// Errors are raw: callers apply posix_errors::path_error.
    pub(in crate::executor) fn probe_input_redirect_readwrite(
        &self,
        target: &str,
    ) -> io::Result<File> {
        use crate::special_paths::SpecialPathClass;
        match crate::special_paths::classify_special_path(target) {
            SpecialPathClass::NetRedirect => return self.open_net_redirect_raw(target),
            // rubash#1898: the devices — <> on /dev/zero reads zeros
            // and discards writes; <> on /dev/full reads zeros (the
            // probe's File is dropped by every caller; the sink
            // stand-in carries the write side of the bind contract).
            SpecialPathClass::ZeroDevice => return Self::zero_device_file(&target),
            SpecialPathClass::FullDevice => return Self::synthetic_proc_write_sink(false),
            SpecialPathClass::FdAlias(fd) => {
                if super::redirection::dup_source_fd_open(
                    &self.fd_table,
                    fd,
                    &std::collections::HashMap::new(),
                ) {
                    // The alias is open: GNU's open succeeds (a dup of
                    // the descriptor). The probe's File is dropped by
                    // every caller, so the null device stands in.
                    return self.create_redirect_output("/dev/null", true);
                }
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "No such file or directory",
                ));
            }
            SpecialPathClass::SyntheticProc => {
                match crate::special_paths::synthetic_proc_write_allowed(target) {
                    Some(false) => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Permission denied",
                        ));
                    }
                    Some(true) => return Self::synthetic_proc_write_sink(false),
                    None => {}
                }
            }
            SpecialPathClass::SyntheticLink => {
                match crate::special_paths::synthetic_link_target(
                    target,
                    Some(&crate::proc_vfs::self_cwd_forms(self.session_cwd())),
                ) {
                    Some(link) => {
                        return OpenOptions::new().read(true).write(true).open(&link.native);
                    }
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "No such file or directory",
                        ));
                    }
                }
            }
            SpecialPathClass::NullDevice | SpecialPathClass::Filesystem => {}
        }
        let path = self.session_path(shell_path_to_windows(target, &self.shell_state.env_vars));
        // niubash#519: CON has no read+write handle (GENERIC_READ|
        // GENERIC_WRITE -> ERROR_INVALID_PARAMETER); the real `<>` bind
        // splits it into CONIN$/CONOUT$ (FileFd::open_readwrite_pair),
        // so probe the console's input half.
        if is_con_device_path(&path) {
            return File::open("CONIN$");
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
    }

    /// Formatted variant: the returned error already carries the
    /// `<target>: <strerror>` shape (posix_errors::path_error), matching
    /// the other open paths in this module.
    pub(in crate::executor) fn open_net_redirect(&self, target: &str) -> io::Result<File> {
        self.open_net_redirect_raw(target)
            .map_err(|e| crate::posix_errors::path_error(target, e))
    }

    /// Raw variant for callers that apply posix_errors::path_error
    /// themselves (e.g. set_fd_readwrite_file's `<>` path).
    pub(in crate::executor) fn open_net_redirect_raw(&self, target: &str) -> io::Result<File> {
        match self.connect_net_stream(target)? {
            NetStream::Tcp(stream) => Ok(tcp_stream_file(stream)),
            NetStream::Udp(socket) => Ok(udp_socket_file(socket)),
        }
    }

    /// Parse and connect a net redirect target, returning the
    /// endpoint in its native socket form. The File form
    /// (open_net_redirect_raw) adopts the SOCKET as a raw handle for
    /// the fd machinery, but std File reads on an adopted overlapped
    /// socket fail on Windows (ERROR_INVALID_PARAMETER), so byte
    /// consumers read through this socket form, whose reads go
    /// through WSARecv/recv. The diagnostic pre-lines (service-name
    /// failure, refused connect; rubash#1237) are emitted here so
    /// both forms report identically.
    pub(in crate::executor) fn connect_net_stream(&self, target: &str) -> io::Result<NetStream> {
        let (udp, rest) = if let Some(rest) = target.strip_prefix("/dev/tcp/") {
            (false, rest)
        } else if let Some(rest) = target.strip_prefix("/dev/udp/") {
            (true, rest)
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "No such file or directory",
            ));
        };
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "Invalid argument");
        let (host, port_text) = rest.rsplit_once('/').ok_or_else(invalid)?;
        let port: u16 = match resolve_net_service_port(port_text, udp) {
            Some(port) => port,
            None => {
                // GNU netopen's getaddrinfo failure is reported by
                // sys_error BEFORE the redirection_error line the caller
                // adds: `{name}: line {n}: {service}: Servname not
                // supported for ai_socktype` (glibc's gai_strerror text
                // for EAI_SERVICE), then `{target}: Invalid argument`
                // (rubash#1237).
                eprintln!(
                    "{}{port_text}: Servname not supported for ai_socktype",
                    self.error_prefix()
                );
                return Err(invalid());
            }
        };
        if host.is_empty() {
            return Err(invalid());
        }
        if udp {
            let bind_addr = if host.contains(':') {
                "[::]:0"
            } else {
                "0.0.0.0:0"
            };
            let socket = std::net::UdpSocket::bind(bind_addr)?;
            socket.connect((host, port))?;
            Ok(NetStream::Udp(socket))
        } else {
            let stream = std::net::TcpStream::connect((host, port)).map_err(|error| {
                // GNU netopen reports a refused connect through sys_error
                // with the script name only (no line segment) BEFORE the
                // redirection_error line the caller adds:
                // `{name}: connect: Connection refused` (rubash#1237).
                if error.kind() == io::ErrorKind::ConnectionRefused {
                    eprintln!("{}connect: Connection refused", self.script_name_prefix());
                }
                error
            })?;
            Ok(NetStream::Tcp(stream))
        }
    }

    /// The canonical special-path dispatch for input opens (vfs audit
    /// §3.3 ladder convergence, §4.5 slice S2): walk the special-path
    /// registry in canonical order — net, null, fd alias, synthetic
    /// content — and construct the read endpoint for the target's
    /// class. Returns `None` when the target is an ordinary filesystem
    /// path, and also for a closed fd alias, whose legacy fall-through
    /// to the caller's literal open (reporting GNU's ENOENT for a
    /// dangling alias) the caller performs; returns `Some` with the
    /// open result for every other special class. Constructing each
    /// class's endpoint lives here exactly once; open points keep
    /// only their semantic pre-steps.
    fn open_classified_input(&self, target: &str) -> Option<io::Result<File>> {
        // rubash#1026: /dev/tcp|udp targets connect a socket, never a
        // filesystem open (see open_net_redirect).
        if crate::special_paths::is_net_redirect_target(target) {
            return Some(self.open_net_redirect(target));
        }
        if is_null_device(target) {
            return Some(File::open(shell_path_to_windows(
                "/dev/null",
                &self.shell_state.env_vars,
            )));
        }
        // rubash#1898: /dev/zero reads yield an unbounded stream of
        // NUL bytes, and /dev/full reads yield the same zeros — both
        // serve from the pump-fed zero source (a real handle, so
        // external children can inherit it). On Unix the construction
        // is equivalent to opening the real device.
        if crate::special_paths::is_zero_device(target)
            || crate::special_paths::is_full_device(target)
        {
            return Some(Self::zero_device_file(&target));
        }
        // GNU redir.c resolves /dev/std*, /dev/fd/N, /proc/self/fd/N through
        // the OS fd-alias layer — a dup of fd N, not a filesystem path.
        // Windows has no /dev, so resolve against the virtual fd table
        // (niubash#118: `< /dev/stdin` used to open CONIN$ and block on the
        // console even when fd 0 was a pipeline).
        if let Some(fd) = dev_stdio_redirect_fd(target) {
            // rubash#545 (cell 2): a CLOSED /dev/fd/N falls through to the
            // literal open, whose failure reports ENOENT — GNU's errno for
            // a dangling /proc/self/fd magic symlink (redir.c
            // redirection_error; the output side has the same fall-through,
            // command_input_scope.rs). A command-scoped process substitution
            // fd dies with its creating command, so `< $p` with a stored
            // `/dev/fd/63` fails `No such file or directory` like GNU.
            if self
                .fd_table
                .entries
                .get(&fd)
                .is_some_and(|entry| !entry.closed)
            {
                return Some(self.open_fd_read_endpoint(fd, target));
            }
            return None;
        }
        // SyntheticLink (rubash#1900): /proc/<pid>/exe and .../cwd are
        // magic links — the open goes to the resolved target file. The
        // session cwd (G-02; never the process-global cwd) answers for
        // self/cwd. A directory target (cwd) fails the OS open here,
        // which the caller reports like any unreadable redirect source.
        if let Some(link) = crate::special_paths::synthetic_link_target(
            target,
            Some(&crate::proc_vfs::self_cwd_forms(self.session_cwd())),
        ) {
            return Some(File::open(&link.native));
        }
        // Q11 /proc P1 (docs/proc-vfs-plan.md hook B): synthetic /proc files
        // serve as a pre-filled pipe converted to a File; reads hit EOF at
        // content end, matching procfs static-snapshot semantics.
        if let Some(content) = crate::special_paths::synthetic_proc_content(target) {
            return Some(Self::synthetic_content_file(target, &content));
        }
        None
    }

    /// Materialize synthetic content as a readable File: a pre-filled
    /// pipe whose write side is already closed, so reads hit EOF at
    /// content end.
    fn synthetic_content_file(target: &str, content: &[u8]) -> io::Result<File> {
        use std::io::Write as _;
        let (reader, mut writer) = std::io::pipe()?;
        writer
            .write_all(content)
            .map_err(|e| crate::posix_errors::path_error(target, e))?;
        drop(writer); // close write side: reader sees EOF after content
        #[cfg(windows)]
        {
            use std::os::windows::io::{FromRawHandle as _, IntoRawHandle as _};
            let handle = reader.into_raw_handle();
            Ok(unsafe { File::from_raw_handle(handle) })
        }
        #[cfg(unix)]
        {
            use std::os::unix::io::{FromRawFd as _, IntoRawFd as _};
            let fd = reader.into_raw_fd();
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    /// rubash#1935: the write side of a 0644-class synthetic /proc
    /// entry (`/proc/self/status`, `/proc/self/cmdline`). GNU's open
    /// of the real entry succeeds and only the WRITE fails (EINVAL),
    /// but no Windows handle reproduces a per-write EINVAL, so writes
    /// land in a throwaway temp file. That removes the two failure
    /// shapes this family exists for — ENOENT (no service face at
    /// all) and a phantom enclave file created at the mapped literal
    /// path — while keeping the observable contract: the open
    /// succeeds, and the generated snapshot stays the only read truth
    /// (on GNU the failed write never alters what a later read of
    /// the entry serves either). Unique per call: two opens of the
    /// entry are two open file descriptions. Delete-on-close where
    /// the platform supports it.
    pub(in crate::executor) fn synthetic_proc_write_sink(append: bool) -> io::Result<File> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SINK_SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SINK_SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rubash-proc-sink-{}-{seq}", std::process::id()));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_FLAG_DELETE_ON_CLOSE: the sink must not outlive
            // the descriptor that owns it.
            options.custom_flags(0x0400_0000);
        }
        let file = options.open(&path)?;
        #[cfg(unix)]
        {
            // The descriptor stays valid after the name is unlinked.
            let _ = std::fs::remove_file(&path);
        }
        Ok(file)
    }

    /// rubash#1898: a readable File that yields an unbounded stream
    /// of NUL bytes — the /dev/zero (and /dev/full) read face. Unix
    /// hosts have the real devices: opening the target IS GNU's
    /// semantics, an unbounded kernel-owned zero source. Windows has
    /// no zero device, so a pump thread feeds a pipe, following the
    /// process-substitution streaming precedent: consumers get a
    /// real handle (external children can inherit it), reads never
    /// see EOF, and the pump exits on a broken pipe once the last
    /// reader closes.
    pub(in crate::executor) fn zero_device_file(target: &str) -> io::Result<File> {
        #[cfg(unix)]
        {
            File::open(target)
        }
        #[cfg(windows)]
        {
            let _ = target;
            use std::io::Write as _;
            let (reader, mut writer) = std::io::pipe()?;
            std::thread::spawn(move || {
                const ZEROS: [u8; 8192] = [0u8; 8192];
                while writer.write_all(&ZEROS).is_ok() {}
            });
            use std::os::windows::io::{FromRawHandle as _, IntoRawHandle as _};
            let handle = reader.into_raw_handle();
            Ok(unsafe { File::from_raw_handle(handle) })
        }
    }

    fn open_input_redirect_impl(&self, target: &str, consume: bool) -> io::Result<File> {
        if let Some(opened) = self.open_classified_input(target) {
            return opened;
        }
        // niubash#177: an input operand under the virtual system root may
        // exist only under its Windows executable spelling (`/usr/bin/seq`
        // backs onto `seq.exe`). Resolve with the same exists-first,
        // existence-checked rule the argv funnel applies, so `< /usr/bin/seq`
        // opens the same file `test -f /usr/bin/seq` says exists; the
        // POSIX target word is kept for diagnostics. A path that exists
        // (or misses under both spellings) keeps the plain mapped form.
        let win_path = self.session_path(crate::executor::path::windows_operand_file_path(
            shell_path_to_windows(target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        // A spawned child reading a `<(cmd)` carrier must get the stream's
        // REMAINING bytes — GNU hands it a dup of the shared pipe offset,
        // so the parent's next open sees what the child left
        // (subst.c:7143). Serve the remainder through a fresh temp and
        // advance the shared offset.
        if consume {
            if let Some(bytes) = self.procsub_stream_take(&win_path) {
                let path = self
                    .write_process_substitution_temp_bytes(&bytes)
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::Other, "failed to materialize input")
                    })?;
                // rubash#1691: this fresh carrier backs the returned
                // File only — park the path so the per-command procsub
                // finish removes it once the consumer is done.
                self.shell_state
                    .procsub_adhoc_carriers
                    .borrow_mut()
                    .push(path.clone());
                return File::open(&path).map_err(|e| crate::posix_errors::path_error(target, e));
            }
        }
        // rubash#1820: a probe (consume == false) must not open a FIFO.
        // The throwaway open consumes the rendezvous pairing the bind
        // phase's one real open — GNU's only open (redir.c
        // do_redirection_internal) — then waits for in vain; even a
        // nonblocking probe open/close could release a writer parked
        // in its own bind open and strand it readerless. Validate with
        // access(2) instead: the same existence/permission verdict, no
        // fd taken. Every probe caller drops the returned File unread,
        // so the /dev/null stand-in is inert.
        #[cfg(unix)]
        if !consume && redirect_path_is_fifo(&win_path) {
            if let Some(error) = redirect_fifo_access_error(&win_path, false) {
                return Err(crate::posix_errors::path_error(target, error));
            }
            return File::open("/dev/null").map_err(|e| crate::posix_errors::path_error(target, e));
        }
        File::open(win_path).map_err(|e| crate::posix_errors::path_error(target, e))
    }

    pub(in crate::executor) fn open_fd_read_endpoint(
        &self,
        fd: u32,
        target: &str,
    ) -> io::Result<File> {
        match self.fd_table.read_endpoint(fd) {
            // DuplicateHandle shares the kernel file object: the child sees
            // the fd at its current offset (POSIX open file description).
            Some(FdReadEndpoint::File(file_fd)) => crate::fd::duplicate_handle(file_fd.handle)
                .map(crate::fd::handle_to_file)
                .map_err(|e| crate::posix_errors::path_error(&file_fd.path.to_string_lossy(), e)),
            // rubash#1659: a coproc pipe read end is handle-backed
            // exactly like a file (its FileFd carries the pipe
            // handle); every sibling reader (read_all_bytes,
            // take_buffered_input_line, inherited_async_stdin)
            // already serves it, but this helper dropped it to
            // Bad file descriptor — so mapfile's `<&${COPROC[0]}`
            // string channel failed and fell through to reading
            // inherited process stdin (a hang on a held-open
            // pipe, or 0 lines on /dev/null).
            Some(FdReadEndpoint::CoprocStdout { fd: file_fd, .. }) => {
                crate::fd::duplicate_handle(file_fd.handle)
                    .map(crate::fd::handle_to_file)
                    .map_err(|e| {
                        crate::posix_errors::path_error(&file_fd.path.to_string_lossy(), e)
                    })
            }
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                let bytes = self
                    .virtual_fd_stdin_remaining_bytes(fd)
                    .unwrap_or_default();
                // GNU hands the child a dup of the same open file
                // description: bytes the child reads move the shared
                // offset — drain the source so the next consumer sees
                // the remainder, not a replay (procsub.tests
                // count_lines → 1,0,0,0,0).
                self.fd_table.drain_input_to_eof(fd);
                let path = self
                    .write_process_substitution_temp_bytes(&bytes)
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::Other, "failed to materialize fd input")
                    })?;
                // rubash#1691: same ad-hoc carrier bookkeeping as
                // open_input_redirect's consume branch.
                self.shell_state
                    .procsub_adhoc_carriers
                    .borrow_mut()
                    .push(path.clone());
                File::open(&path)
            }
            Some(FdReadEndpoint::InheritedProcessStdin) => {
                super::inherited_stdin::duplicate_process_stdin()
            }
            _ => {
                // rubash#1963: no read endpoint, but the descriptor
                // may be open write-only (`exec 63>f; : < /dev/fd/63`).
                // GNU's /dev/fd open is a fresh open of the
                // descriptor's TARGET, judged against the target
                // file's mode, not the descriptor's: a file binding
                // reopens its path (its stored path is the native
                // form it was opened from), and any other open
                // descriptor's read-open succeeds against the null
                // device (reads see EOF — the stdio write ends'
                // content contract is the dev-operand face's
                // documented decline). Only a descriptor with no
                // endpoint at all keeps the EBADF answer.
                match self.fd_table.write_endpoint(fd) {
                    Some(FdWriteEndpoint::File(file)) => File::open(&file.path)
                        .map_err(|e| crate::posix_errors::path_error(target, e)),
                    Some(_) => File::open(shell_path_to_windows(
                        "/dev/null",
                        &self.shell_state.env_vars,
                    )),
                    None => Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{target}: Bad file descriptor"),
                    )),
                }
            }
        }
    }

    /// niubash#529: fd 0's `test -t` verdict (GNU test.c isatty). The
    /// inherited-process-stdin endpoint names the process stdin only while
    /// no virtual input buffer is bound: a pipeline element or redirected
    /// body reads FUNCTION_STDIN — pipe content in GNU terms
    /// (execute_cmd.c execute_pipeline binds the pipe to fd 0) — never the
    /// terminal, whatever the process stdin happens to be.
    // pub(in crate::executor): the jobs refresh (job_builtins.rs)
    // also consults it — job control live on a terminal is one of
    // GNU's state-publication events (rubash#1612).
    pub(in crate::executor) fn stdin_source_is_terminal(&self) -> bool {
        if self.shell_state.env_vars.contains_key(FUNCTION_STDIN) {
            return false;
        }
        crate::fd::is_console_handle(crate::fd::process_std_handle(0))
    }

    /// niubash#529: fd 1's `test -t` verdict. The Stdout endpoint resolves
    /// to the active capture while one is installed — the command
    /// substitution pipe (subst.c:7143 command_substitute: the child's
    /// stdout is the capture pipe, never the caller's fd 1) or a pipeline
    /// stage's staging buffer — and to the process stdout only when no
    /// capture owns it. Judging it by the process stdout answers Y inside
    /// `$( [ -t 1 ] )` on a terminal, where GNU answers N.
    fn stdout_sink_is_terminal(&self) -> bool {
        if self.stdout_capture.is_some() || stdout_capture_active() {
            return false;
        }
        crate::fd::is_console_handle(crate::fd::process_std_handle(1))
    }

    /// GNU test.c: `-t fd` answers isatty(fd) — whether the descriptor's
    /// current target is a terminal. The builtin only receives env_vars,
    /// so mirror the fd table plus the command's own (not yet bound)
    /// redirects into __RUBASH_FD_TERMINAL_<fd> right before it runs.
    /// Rewritten fresh each call so a stale mark can never outlive the
    /// binding it described.
    pub(in crate::executor) fn sync_fd_terminal_marks(&mut self, cmd: Option<&CommandNode>) {
        self.shell_state
            .env_vars
            .retain(|key, _| !key.starts_with(FD_TERMINAL_PREFIX));
        let mut marks: BTreeMap<u32, bool> = BTreeMap::new();
        for (fd, entry) in &self.fd_table.entries {
            if entry.closed {
                continue;
            }
            let read_tty = match &entry.read {
                Some(FdReadEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                Some(FdReadEndpoint::InheritedProcessStdin) => self.stdin_source_is_terminal(),
                _ => false,
            };
            let write_tty = match &entry.write {
                Some(FdWriteEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                Some(FdWriteEndpoint::Stdout) => self.stdout_sink_is_terminal(),
                Some(FdWriteEndpoint::Stderr) => {
                    crate::fd::is_console_handle(crate::fd::process_std_handle(2))
                }
                _ => false,
            };
            marks.insert(*fd, read_tty || write_tty);
        }
        if let Some(cmd) = cmd {
            // do_redirections would leave the dup'd target live while the
            // builtin runs; the virtual-stdin model never bound it, so
            // judge the redirect's effective endpoint directly.
            if let Some(redirect) = &cmd.redirect_in {
                let fd = redirect.fd.unwrap_or(0);
                let target = self.expand_redirect_target(redirect);
                if is_closed_redirect_target(&target) {
                    marks.insert(fd, false);
                } else if let Some(tty) = self.redirect_target_is_terminal(&target) {
                    marks.insert(fd, tty);
                }
            }
            for redirect in [
                &cmd.redirect_out,
                &cmd.append,
                &cmd.redirect_err,
                &cmd.redirect_err_append,
            ]
            .into_iter()
            .flatten()
            {
                let fd = redirect.fd.unwrap_or(1);
                let target = self.expand_redirect_target(redirect);
                if let Some(tty) = self.redirect_target_is_terminal(&target) {
                    marks.insert(fd, tty);
                }
            }
        }
        for (fd, tty) in marks {
            self.shell_state
                .env_vars
                .insert(fd_terminal_key(fd), if tty { "1" } else { "0" }.to_string());
        }
    }

    /// GNU test.c: a `/dev/fd/N` operand names the shell's OWN descriptor
    /// N (via the magic symlink). Process-substitution and coproc
    /// descriptors are virtual in this executor — stat() on the path
    /// would interrogate the host process's unrelated fd N — so mirror
    /// the pipe-like endpoints into __RUBASH_FD_PIPE_<fd> right before
    /// `test`/`[` runs (rubash#1467; the verdict matrix lives in
    /// builtins/test.rs fd_pipe_test, shared with the [[ ]] path).
    /// Rewritten fresh each call, like the terminal marks.
    pub(in crate::executor) fn sync_fd_pipe_marks(&mut self) {
        self.shell_state
            .env_vars
            .retain(|key, _| !key.starts_with(FD_PIPE_PREFIX));
        for (fd, entry) in &self.fd_table.entries {
            if entry.closed {
                continue;
            }
            let read_pipe = matches!(
                entry.read,
                Some(
                    FdReadEndpoint::ProcessSubstitution(_)
                        | FdReadEndpoint::Text(_)
                        | FdReadEndpoint::CoprocStdout { .. }
                )
            ) || matches!(
                &entry.read,
                Some(FdReadEndpoint::File(file))
                    if crate::fd::is_pipe_handle(file.handle)
            );
            // A File endpoint can also back the slot with a real OS pipe
            // (`exec N< <(...)` stores the spawned child's pipe reader
            // as a File) — classify by the handle, not the variant, so
            // regular files stay unmarked (rubash#1467).
            let write_pipe = matches!(
                entry.write,
                Some(
                    FdWriteEndpoint::ProcessSubstitution { .. }
                        | FdWriteEndpoint::CoprocStdin { .. }
                )
            ) || matches!(
                &entry.write,
                Some(FdWriteEndpoint::File(file))
                    if crate::fd::is_pipe_handle(file.handle)
            );
            if read_pipe || write_pipe {
                self.shell_state
                    .env_vars
                    .insert(format!("{FD_PIPE_PREFIX}{fd}"), "1".to_string());
            }
        }
    }

    /// isatty() verdict for an expanded redirect target: `&N` and the
    /// /dev/fd/N aliases consult fd N's bound endpoint; a path is probed
    /// read-only (no create/truncate side effect) and judged by
    /// GetConsoleMode — true for CON, false for NUL and disk files.
    fn redirect_target_is_terminal(&self, target: &str) -> Option<bool> {
        if let Some(source_fd) = redirect_target_fd(target) {
            return Some(match self.fd_table.read_endpoint(source_fd) {
                Some(FdReadEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                Some(FdReadEndpoint::InheritedProcessStdin) => self.stdin_source_is_terminal(),
                Some(_) => false,
                None => match self.fd_table.output_endpoint(source_fd) {
                    Some(FdWriteEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                    Some(FdWriteEndpoint::Stdout) => self.stdout_sink_is_terminal(),
                    Some(FdWriteEndpoint::Stderr) => {
                        crate::fd::is_console_handle(crate::fd::process_std_handle(2))
                    }
                    _ => false,
                },
            });
        }
        let path = shell_path_to_windows(target, &self.shell_state.env_vars);
        File::open(&path).ok().map(|file| {
            #[cfg(windows)]
            let raw = {
                use std::os::windows::io::AsRawHandle;
                file.as_raw_handle() as crate::fd::HANDLE
            };
            #[cfg(unix)]
            let raw = {
                use std::os::unix::io::AsRawFd;
                file.as_raw_fd() as crate::fd::HANDLE
            };
            crate::fd::is_console_handle(raw)
        })
    }

    /// rubash#1312: measured GNU rule (glibc bash 5.3.20 oracle, script
    /// file under a PTY): with `set -C`, `>` refuses only an existing
    /// REGULAR file — `echo hi >/dev/tty` and `>/dev/null` both succeed.
    /// A raw O_CREAT|O_EXCL open returns EEXIST for Linux device files,
    /// so the refusal must be gated on the target's file type, not on
    /// mere existence. Unix-only gate: the Windows device-name layer
    /// keeps the blanket create_new behavior (see the #519 family).
    #[cfg(unix)]
    fn noclobber_target_is_device_like(path: &std::path::Path) -> bool {
        std::fs::metadata(path)
            .map(|m| !m.is_file())
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    fn noclobber_target_is_device_like(_path: &std::path::Path) -> bool {
        false
    }

    /// G-02: absolutize a host path against the session cwd.
    /// `shell_path_to_windows` deliberately keeps relative spellings
    /// relative; every open performed by the session itself must anchor
    /// them at `ShellState.cwd`, never at the process cwd.
    pub(in crate::executor) fn session_path(&self, path: std::path::PathBuf) -> std::path::PathBuf {
        if path.is_absolute() {
            path
        } else {
            self.shell_state.cwd.join(path)
        }
    }

    pub(in crate::executor) fn create_redirect_output(
        &self,
        target: &str,
        clobber: bool,
    ) -> io::Result<File> {
        // rubash#1026: /dev/tcp|udp targets connect a socket (see
        // open_net_redirect); noclobber has no meaning for them.
        if crate::special_paths::is_net_redirect_target(target) {
            return self.open_net_redirect(target);
        }
        if is_null_device(target) {
            return OpenOptions::new().write(true).open(shell_path_to_windows(
                "/dev/null",
                &self.shell_state.env_vars,
            ));
        }
        // rubash#1898: /dev/zero discards writes exactly like the
        // null device; opens route to the same sink.
        if crate::special_paths::is_zero_device(target) {
            return OpenOptions::new().write(true).open(shell_path_to_windows(
                "/dev/null",
                &self.shell_state.env_vars,
            ));
        }
        // rubash#1898: /dev/full ACCEPTS the open — GNU fails only
        // the write (ENOSPC). The direct builtin faces route their
        // flush through OutputTarget::FullDevice, which owns the
        // write-error shape; the File returned here is the stand-in
        // for the probe/bind/external contracts that need the open
        // itself to succeed (their writes are absorbed — the
        // documented residual of those faces).
        if crate::special_paths::is_full_device(target) {
            return Self::synthetic_proc_write_sink(false);
        }
        // rubash#1935: synthetic /proc content has no filesystem file
        // behind the mapped path — without this step `> /proc/...`
        // either failed ENOENT or created a phantom enclave file at
        // the literal path. Ask the entry's write class instead: the
        // 0444 class refuses the open EACCES; the 0644 class accepts
        // it and the writes are absorbed (see
        // synthetic_proc_write_sink).
        if let Some(allowed) = crate::special_paths::synthetic_proc_write_allowed(target) {
            if !allowed {
                return Err(crate::posix_errors::path_error(
                    target,
                    io::Error::new(io::ErrorKind::PermissionDenied, "Permission denied"),
                ));
            }
            return Self::synthetic_proc_write_sink(false);
        }
        // rubash#1963: an fd-alias spelling is an open of fd N's
        // open file description (redir.c redir_open over the /dev/fd
        // layer), never a filesystem create at the mapped literal
        // path — the literal create made a phantom enclave file for
        // a spelling GNU fails ENOENT when the fd is closed. Closed
        // alias: ENOENT, pre-shaped like the synthetic arm above
        // (this function's callers render the io message verbatim).
        // Open alias: resolve through the endpoint like the append
        // face (open_append_target's FdAlias arm); endpoint shapes
        // that cannot take a handle dup fall back to the null sink —
        // the routing walk owns exact delivery for fds 1/2 through
        // its dup shortcut, and this function's contract is the
        // open itself.
        if let Some(fd) = crate::special_paths::fd_alias_target(target) {
            if !super::redirection::dup_source_fd_open(
                &self.fd_table,
                fd,
                &std::collections::HashMap::new(),
            ) {
                return Err(crate::posix_errors::path_error(
                    target,
                    io::Error::new(io::ErrorKind::NotFound, "No such file or directory"),
                ));
            }
            return match self.open_output_fd_append(target) {
                Ok(file) => Ok(file),
                Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                    OpenOptions::new().write(true).open(shell_path_to_windows(
                        "/dev/null",
                        &self.shell_state.env_vars,
                    ))
                }
                Err(error) => Err(error),
            };
        }
        let target = self.redirect_output_path_target(target);
        let path = self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars));
        if !clobber
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "noclobber")
            && !Self::noclobber_target_is_device_like(&path)
        {
            // GNU wording (bash builtins/common.c): "<target>: cannot
            // overwrite existing file"; the redirect machinery prints the
            // payload after its script/line prefix.
            return OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| {
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("{target}: cannot overwrite existing file"),
                        )
                    } else {
                        e
                    }
                });
        }
        if crate::executor::execution_misc::is_dos_device_path(&path) {
            // niubash#519 family: GNU redir_open's single open() succeeds
            // on a character device because O_TRUNC is a no-op there; the
            // Windows adaptation must not let File::create's
            // truncate-bearing open shape fail the redirect on hosts
            // that reject it for device objects. A DOS device target
            // takes the same plain write open as the append arm
            // (open_redirect_append).
            return OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|e| crate::posix_errors::path_error(&target, e));
        }
        File::create(path).map_err(|e| crate::posix_errors::path_error(&target, e))
    }

    /// rubash#1025: prepare-phase variant of [`Self::create_redirect_output`]
    /// for compound commands. GNU do_redirection_internal opens a
    /// compound's redirect ONCE; rubash's prepare phase (create/truncate
    /// here, splice appends into the body) and bind phase
    /// (open_compound_output_redirects) open it twice, so a successful
    /// noclobber O_EXCL create is recorded here and the bind phase
    /// consumes the record instead of failing on the file this same
    /// command just created.
    pub(in crate::executor) fn create_redirect_output_prepared(
        &self,
        target: &str,
        clobber: bool,
    ) -> io::Result<File> {
        // rubash#1820: on a FIFO the prepare phase's create is a
        // semantic no-op (O_TRUNC/O_CREAT do nothing to a fifo) whose
        // blocking open consumes the rendezvous pairing the bind phase
        // (open_compound_output_redirects) then waits for in vain.
        // Skip the physical open — the bind performs GNU's single
        // open — after the same write-access validation the create's
        // failure would have surfaced. Every caller drops the returned
        // File, so the /dev/null stand-in is inert.
        #[cfg(unix)]
        {
            let path = self.session_path(shell_path_to_windows(
                &self.redirect_output_path_target(target),
                &self.shell_state.env_vars,
            ));
            if redirect_path_is_fifo(&path) {
                if let Some(error) = redirect_fifo_access_error(&path, true) {
                    return Err(crate::posix_errors::path_error(target, error));
                }
                return File::open("/dev/null");
            }
        }
        let file = self.create_redirect_output(target, clobber)?;
        if !clobber
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "noclobber")
        {
            self.noclobber_prepared_targets
                .borrow_mut()
                .insert(target.to_string());
        }
        Ok(file)
    }

    /// Consume a prepare-phase noclobber record (rubash#1025): true when
    /// `target` was already created by the current compound's prepare
    /// phase, so the bind phase must not run the O_EXCL open again.
    pub(in crate::executor) fn take_noclobber_prepared_target(&self, target: &str) -> bool {
        self.noclobber_prepared_targets.borrow_mut().remove(target)
    }

    pub(in crate::executor) fn redirect_output_path_target(&self, target: &str) -> String {
        if self.posix_mode_enabled() {
            return target.to_string();
        }

        match pathname_expand_word(target, &self.shell_state.env_vars, &self.shell_state.cwd) {
            PathnameExpansion::Matches(matches) if matches.len() == 1 => matches[0].clone(),
            _ => target.to_string(),
        }
    }

    pub(in crate::executor) fn open_output_fd_append(&self, target: &str) -> io::Result<File> {
        let fd = redirect_target_fd(target)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad file descriptor"))?;
        if self.fd_table.is_closed(fd) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "bad file descriptor",
            ));
        }
        let endpoint = self
            .fd_table
            .output_endpoint(fd)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "fd is not writable"))?;
        // Handle-backed endpoints duplicate the real kernel object, keeping
        // the shared file offset (POSIX open file description) instead of
        // reopening the path at offset 0.
        if let FdWriteEndpoint::File(file_fd) = endpoint {
            return crate::fd::duplicate_handle(file_fd.handle).map(crate::fd::handle_to_file);
        }
        let path = match endpoint {
            FdWriteEndpoint::ProcessSubstitution { path, .. } => path,
            FdWriteEndpoint::Stdout | FdWriteEndpoint::Stderr => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "stdio file descriptor",
                ));
            }
            FdWriteEndpoint::CoprocStdin { .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "coprocess file descriptor",
                ));
            }
            FdWriteEndpoint::FullDevice => {
                // rubash#1898: an append-dup of a /dev/full binding —
                // GNU's open succeeds and its writes fail ENOSPC; no
                // handle reproduces that, so the append lands on the
                // absorbed sink (a deep edge; the bound write faces
                // themselves carry the exact ENOSPC shape).
                return Self::synthetic_proc_write_sink(true);
            }
            FdWriteEndpoint::File(_) => unreachable!("handled above"),
        };
        if is_null_device(&path.to_string_lossy()) {
            return OpenOptions::new()
                .write(true)
                .append(true)
                .open(shell_path_to_windows(
                    "/dev/null",
                    &self.shell_state.env_vars,
                ));
        }
        OpenOptions::new().create(true).append(true).open(path)
    }

    /// The one append opener every face asks (rubash#1964). GNU
    /// redir.c r_appending_to performs ONE open of the word
    /// (redir.c:702 redir_open, O_WRONLY|O_CREAT|O_APPEND) — for a
    /// /dev/tcp word that open IS the connection, for /dev/fd/N it
    /// resolves through the descriptor, for a synthetic /proc entry
    /// it is the entry's write-class open. The append faces used to
    /// raw-open the mapped literal path instead, so a net target was
    /// unreachable through them (the mapped name is not a file) and
    /// an append probe could fail shapes the real open serves — up
    /// to preempting an external command whose own redirect handling
    /// would have connected. Classify first; only plain filesystem
    /// spellings reach the mapped append open. Errors are raw:
    /// callers apply posix_errors::path_error.
    pub(in crate::executor) fn open_append_target(&self, target: &str) -> io::Result<File> {
        use crate::special_paths::SpecialPathClass;
        match crate::special_paths::classify_special_path(target) {
            SpecialPathClass::NetRedirect => self.open_net_redirect_raw(target),
            SpecialPathClass::NullDevice => {
                OpenOptions::new()
                    .write(true)
                    .append(true)
                    .open(shell_path_to_windows(
                        "/dev/null",
                        &self.shell_state.env_vars,
                    ))
            }
            // rubash#1898: /dev/zero appends discard like the null
            // device; /dev/full appends take the sink stand-in (the
            // open succeeds — GNU fails only the write; the direct
            // builtin faces own that shape via OutputTarget).
            SpecialPathClass::ZeroDevice => {
                OpenOptions::new()
                    .write(true)
                    .append(true)
                    .open(shell_path_to_windows(
                        "/dev/null",
                        &self.shell_state.env_vars,
                    ))
            }
            SpecialPathClass::FullDevice => Self::synthetic_proc_write_sink(true),
            SpecialPathClass::FdAlias(fd) => {
                if !super::redirection::dup_source_fd_open(
                    &self.fd_table,
                    fd,
                    &std::collections::HashMap::new(),
                ) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "No such file or directory",
                    ));
                }
                // The descriptor is open: the append open resolves
                // through its endpoint. Endpoint shapes that cannot
                // take an append dup (the process stdio family) fall
                // back to the mapped device open, which is the shape
                // those spellings have always taken.
                match self.open_output_fd_append(target) {
                    Ok(file) => Ok(file),
                    Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                        crate::executor::execution_misc::open_redirect_append(&self.session_path(
                            shell_path_to_windows(target, &self.shell_state.env_vars),
                        ))
                    }
                    Err(error) => Err(error),
                }
            }
            SpecialPathClass::SyntheticProc => {
                match crate::special_paths::synthetic_proc_write_allowed(target) {
                    Some(false) => Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Permission denied",
                    )),
                    Some(true) => Self::synthetic_proc_write_sink(true),
                    None => {
                        crate::executor::execution_misc::open_redirect_append(&self.session_path(
                            shell_path_to_windows(target, &self.shell_state.env_vars),
                        ))
                    }
                }
            }
            SpecialPathClass::SyntheticLink => {
                match crate::special_paths::synthetic_link_target(
                    target,
                    Some(&crate::proc_vfs::self_cwd_forms(self.session_cwd())),
                ) {
                    Some(link) => crate::executor::execution_misc::open_redirect_append(
                        &std::path::PathBuf::from(&link.native),
                    ),
                    None => Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "No such file or directory",
                    )),
                }
            }
            SpecialPathClass::Filesystem => crate::executor::execution_misc::open_redirect_append(
                &self.session_path(shell_path_to_windows(target, &self.shell_state.env_vars)),
            ),
        }
    }

    pub(in crate::executor) fn write_output_fd_redirect(
        &mut self,
        target: &str,
        output: &[u8],
    ) -> Result<bool, ExecuteError> {
        let Some(fd) = redirect_target_fd(target) else {
            return Ok(false);
        };
        if self.fd_table.is_closed(fd) {
            return Ok(false);
        }
        let Some(endpoint) = self.fd_table.output_endpoint(fd) else {
            return Ok(false);
        };
        match endpoint {
            // Same dup2 open-file-description semantics as write_fd_endpoint:
            // a write to an fd bound to Stdout/Stderr follows the recorded
            // snapshot binding (capture generation / real stdout,
            // rubash#223), or the live capture when the fd is the alias.
            // The `None` record is the REAL process stdout snapshotted at
            // dup time (redir.c:1170) — it must bypass the active capture,
            // not re-resolve through it (rubash#368).
            FdWriteEndpoint::Stdout => match self.fd_table.stdout_alias_generation.get(&fd) {
                Some(Some(generation)) => {
                    let _ = write_stdout_capture_at_generation(output, *generation);
                }
                Some(None) => {
                    write_real_stdout_uncaptured(output)?;
                }
                None => {
                    if stdout_capture_active() {
                        stdout_capture_write_mapped(output)?;
                    } else if let Some(capture) = &mut self.stdout_capture {
                        capture.write_all(output)?;
                        capture_timing_note_field(output, capture.len());
                    } else {
                        write_stdout_bytes(output)?;
                    }
                }
            },
            FdWriteEndpoint::Stderr => {
                if let Some(capture) = &mut self.stderr_capture {
                    capture.write_all(output)?;
                } else {
                    write_stderr_bytes(output)?;
                }
            }
            FdWriteEndpoint::CoprocStdin { fd, .. } => {
                crate::fd::write_all(fd.handle, output)?;
            }
            FdWriteEndpoint::File(file_fd) => {
                crate::fd::write_all(file_fd.handle, output)?;
            }
            FdWriteEndpoint::FullDevice => {
                // rubash#1898: every write to a /dev/full binding
                // fails ENOSPC; the payload carries the strerror so
                // the sh_chkwrite rendering is exact on every
                // platform (Windows' error table has no ENOSPC).
                return Err(io::Error::new(io::ErrorKind::Other, "No space left on device").into());
            }
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(output)?;
            }
        }
        Ok(true)
    }

    pub(in crate::executor) fn output_fd_redirects_to_stdout(&self, target: &str) -> bool {
        if redirect_target_fd(target).map_or(false, |fd| {
            matches!(
                self.fd_table.output_endpoint(fd),
                Some(FdWriteEndpoint::Stdout)
            )
        }) {
            return true;
        }
        false
    }

    /// GNU redir.c opens `/dev/std*`, `/dev/fd/N` and `/proc/self/fd/N`
    /// through the OS fd-alias layer: the kernel resolves the open() to a
    /// dup of fd N's CURRENT open file description
    /// (execution_misc::dev_stdio_redirect_fd; the input side is
    /// niubash#118). For a built-in writer this means: an explicit fd-table
    /// binding writes through `write_output_fd_redirect`, and an IMPLICIT
    /// std fd (no entry — the shell's own stdout/stderr) writes to that
    /// channel's default stream. Re-opening the alias as a literal Windows
    /// path instead bound the write to the CONOUT$ console device, which
    /// either bypassed the real — possibly redirected or captured — stream
    /// (output vanished under pipes) or failed outright with Permission
    /// denied (rubash#216: `u > /dev/stderr` with a heredoc `cat` body).
    /// Returns None when the target is not an fd alias (caller opens the
    /// path) or the alias refers to a closed/unbound non-std fd (caller's
    /// literal open reports the failure the way GNU's open() would).
    pub(in crate::executor) fn write_dev_stdio_redirect_output(
        &mut self,
        target: &str,
        output: &[u8],
    ) -> Option<Result<(), ExecuteError>> {
        let fd = redirect_target_fd(target)?;
        if self.fd_table.is_closed(fd) {
            return None;
        }
        if self.fd_table.output_endpoint(fd).is_some() {
            return Some(self.write_output_fd_redirect(target, output).map(|_| ()));
        }
        match fd {
            1 => Some(self.write_default_stdout(output)),
            2 => Some(self.write_default_stderr(output)),
            _ => None,
        }
    }

    pub(in crate::executor) fn output_fd_redirects_to_stderr(&self, target: &str) -> bool {
        if redirect_target_fd(target).map_or(false, |fd| {
            matches!(
                self.fd_table.output_endpoint(fd),
                Some(FdWriteEndpoint::Stderr)
            )
        }) {
            return true;
        }
        false
    }

    pub(in crate::executor) fn input_fd_redirects_to_process_stdin(&self, target: &str) -> bool {
        if redirect_target_fd(target).map_or(false, |fd| {
            matches!(
                self.fd_table
                    .entries
                    .get(&fd)
                    .and_then(|entry| entry.read.as_ref()),
                Some(FdReadEndpoint::InheritedProcessStdin)
            )
        }) {
            return true;
        }
        false
    }

    pub(in crate::executor) fn write_default_stdout(
        &mut self,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        // fd 1's bound endpoint decides the destination (dup2 snapshot
        // semantics): the Stdout arm inside write_fd_endpoint resolves to
        // the active capture or raw stdio, and a `1>&2` binding correctly
        // follows fd 2 instead.
        self.write_fd_endpoint(1, output)?;
        Ok(())
    }

    pub(in crate::executor) fn write_default_stderr(
        &mut self,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        // Same: fd 2 may be bound to Stdout by `2>&1`, so the capture
        // buffers are consulted inside the endpoint arms, not first.
        self.write_fd_endpoint(2, output)?;
        Ok(())
    }

    /// GNU error.c report_error writes diagnostics to fd 2 *as currently
    /// bound by the shell's redirections* — an enclosing subshell/group
    /// redirect or `2>&1` dup2 has already moved fd 2 before the expansion
    /// runs, so `( eval 'bad ${}' ) 2>/dev/null` silences the message.
    /// Expansion diagnostics raised from `&self` deep-expansion methods
    /// cannot use write_fd_endpoint (its capture arms need &mut), so this
    /// mirrors its fd-2 dispatch with interior-immutable writes only: the
    /// fd table's endpoint decides between drop (closed), the bound file,
    /// fd 1's destination (`2>&1`), and the process stderr. Raw `eprintln!`
    /// at those call sites bypassed every enclosing redirect (rubash#167).
    pub(in crate::executor) fn write_diagnostic_fd2(&self, output: &[u8]) {
        if self.fd_table.is_closed(2) {
            return;
        }
        let Some(endpoint) = self.fd_table.output_endpoint(2) else {
            return;
        };
        let result = match endpoint {
            // fd 2 bound to fd 1's object by `2>&1` carries the same dup2
            // snapshot contract as write_fd_endpoint (redir.c:1170): the
            // recorded capture generation when the dup ran inside a
            // capture, the REAL process stdout when it ran outside one
            // (rubash#368 — `exec 2>&1; v=$(${x?boom})` reports boom on
            // the real stdout, outside the substitution), and only a
            // record-less marker the live capture-first resolution.
            FdWriteEndpoint::Stdout => match self.fd_table.stdout_alias_generation.get(&2) {
                Some(Some(generation)) => write_stdout_capture_at_generation(output, *generation),
                Some(None) => write_real_stdout_uncaptured(output),
                None => write_global_stdout(output),
            },
            FdWriteEndpoint::Stderr => write_stderr_bytes(output),
            // A stderr fd bound to a coprocess input or process-substitution
            // path is not a diagnostics destination; report on the terminal
            // stream like GNU's stderr fallback.
            FdWriteEndpoint::CoprocStdin { .. } | FdWriteEndpoint::ProcessSubstitution { .. } => {
                write_stderr_bytes(output)
            }
            // rubash#1898: fd 2 bound to /dev/full — the diagnostic
            // write fails ENOSPC and is lost, exactly GNU's outcome
            // for a failing builtin_error write.
            FdWriteEndpoint::FullDevice => Ok(()),
            FdWriteEndpoint::File(file_fd) => crate::fd::write_all(file_fd.handle, output),
        };
        let _ = result;
    }

    pub(in crate::executor) fn has_output_fd_target(&self, target: &str) -> bool {
        redirect_target_fd(target).is_some_and(|fd| {
            !self.fd_table.is_closed(fd) && self.fd_table.output_endpoint(fd).is_some()
        })
    }

    pub(in crate::executor) fn write_fd_endpoint(
        &mut self,
        fd: u32,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        if self.fd_table.is_closed(fd) {
            return Ok(());
        }
        let Some(endpoint) = self.fd_table.output_endpoint(fd) else {
            return Ok(());
        };
        match endpoint {
            // GNU dup2 (`2>&1`) copies the open file description: a write to
            // an fd bound to Stdout must land wherever fd 1 currently goes —
            // the active stdout capture/pipe — not the raw process stdout.
            // An fd whose marker was dup'd from fd 1 earlier holds a
            // snapshot: that capture generation's buffer, or the real
            // process stdout when it was bound outside any capture — never
            // a nested capture that later owns fd 1 (rubash#223). The
            // `None` record therefore writes the REAL process stdout
            // directly: routing it through write_stdout_bytes would
            // re-resolve against the active command-substitution capture
            // and swallow bytes that GNU sends outside the substitution
            // (rubash#368: `exec 3>&1; v=$(echo x >&3)` prints x outside).
            FdWriteEndpoint::Stdout => {
                // niubash#345: while a shell-side pipeline producer's capture
                // frame is active, its fd-1 writes stream into the next
                // stage's OS pipe instead of any in-memory capture. This must
                // win over the thread-local capture the compound-stage
                // boundary installs, or an unbounded `while`/`for` writer
                // would buffer forever. A broken pipe (the downstream consumer
                // exited) surfaces as a SIGPIPE-shaped termination of the
                // producing stage. Nested frames carry `None`, so an inner
                // pipeline's stage bodies keep capturing.
                if fd == 1 {
                    if let Some(stream) = active_capture_stream() {
                        return write_pipeline_stream(stream, output);
                    }
                }
                match self.fd_table.stdout_alias_generation.get(&fd) {
                    Some(Some(generation)) => {
                        let _ = crate::executor::shell_options::write_stdout_capture_at_generation(
                            output,
                            *generation,
                        );
                    }
                    Some(None) => {
                        write_real_stdout_uncaptured(output)?;
                    }
                    None => {
                        if stdout_capture_active() {
                            stdout_capture_write_mapped(output)?;
                        } else if let Some(capture) = &mut self.stdout_capture {
                            capture.write_all(output)?;
                            capture_timing_note_field(output, capture.len());
                        } else {
                            write_stdout_bytes(output)?;
                        }
                    }
                }
            }
            FdWriteEndpoint::Stderr => {
                if let Some(capture) = &mut self.stderr_capture {
                    capture.write_all(output)?;
                } else {
                    write_stderr_bytes(output)?;
                }
            }
            FdWriteEndpoint::CoprocStdin { fd, .. } => {
                crate::fd::write_all(fd.handle, output).map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "coprocess input is closed")
                })?;
            }
            FdWriteEndpoint::File(file_fd) => {
                crate::fd::write_all(file_fd.handle, output)?;
            }
            FdWriteEndpoint::FullDevice => {
                // rubash#1898: every write to a /dev/full binding
                // fails ENOSPC; the payload carries the strerror so
                // the sh_chkwrite rendering is exact on every
                // platform (Windows' error table has no ENOSPC).
                return Err(io::Error::new(io::ErrorKind::Other, "No space left on device").into());
            }
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(output)?;
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn apply_simple_set_flags(&mut self, args: &[String]) -> bool {
        if args.is_empty() {
            return false;
        }

        // GNU set.def:671-691 validates every flag word (internal_getopt
        // pre-scan) before applying any flag: `set -e -Z` reports the error
        // with errexit untouched. Bail BEFORE applying anything when any
        // word is not a pure supported-short-flag word, so the slow path
        // (set_with_io, which reproduces the pre-scan) reports the failure
        // with nothing applied (rubash#358).
        for arg in args {
            let Some(_prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                return false;
            };
            let flags = &arg[1..];
            if flags.is_empty()
                || flags
                    .chars()
                    .any(|flag| !self.is_supported_short_set_flag(flag))
            {
                return false;
            }
        }

        for arg in args {
            let prefix = arg.chars().next().expect("validated above");
            let enabled = prefix == '-';
            for flag in arg[1..].chars() {
                crate::builtins::set::apply_short_set_flag(
                    &mut self.shell_state.env_vars,
                    flag,
                    enabled,
                );
            }
        }

        true
    }

    pub(in crate::executor) fn apply_set_positional_operands(&mut self, args: &[String]) -> bool {
        if args.is_empty() {
            return false;
        }

        let mut flag_updates = Vec::new();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if arg == "--" {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index + 1..].to_vec());
                return true;
            }

            if arg == "-" {
                self.apply_set_flag_updates(&flag_updates);
                crate::builtins::set::set_shell_option(
                    &mut self.shell_state.env_vars,
                    "xtrace",
                    false,
                );
                if index + 1 < args.len() {
                    self.shell_state.dollar_vars_changed_by_set = true;
                    self.set_positional_params(args[index + 1..].to_vec());
                }
                return true;
            }

            let Some(prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index..].to_vec());
                return true;
            };

            let flags = &arg[1..];
            if flags.is_empty() {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index + 1..].to_vec());
                return true;
            }

            if flags == "o" {
                let Some(option_name) = args.get(index + 1) else {
                    return false;
                };
                if !crate::builtins::set::is_shell_option(option_name) {
                    return false;
                }
                // GNU flags.c change_flag (flags.c:227-235) refuses
                // "set +o restricted" in a restricted shell with FLAG_ERROR,
                // and set.def:493-497 turns that into sh_invalidoptname
                // ("invalid option name", EX_USAGE) without changing the
                // restriction. Bail to the full set parser so the refusal is
                // reported instead of being silently applied here.
                if option_name == "restricted"
                    && prefix == '+'
                    && crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "restricted",
                    )
                {
                    return false;
                }
                let enabled = prefix == '-';
                crate::builtins::set::set_shell_option(
                    &mut self.shell_state.env_vars,
                    option_name,
                    enabled,
                );
                if option_name == "ignoreeof" {
                    // set.def:388-399 set_ignoreeof binds/unbinds IGNOREEOF —
                    // mirror the env write into the typed owner expansion
                    // reads first.
                    if enabled {
                        let _ = self
                            .shell_state
                            .variables
                            .set_scalar("IGNOREEOF", "10".to_string());
                    } else {
                        self.shell_state.variables.remove("IGNOREEOF");
                    }
                }
                // The posix derived-option walk (set_posix_mode ->
                // posix_initialize) lives in set::set_shell_option, the
                // single funnel every posix setter uses.
                index += 2;
                continue;
            }

            if flags
                .chars()
                .any(|flag| !self.is_supported_short_set_flag(flag))
            {
                return false;
            }

            flag_updates.push((prefix, flags.to_string()));
            index += 1;
        }

        false
    }

    pub(in crate::executor) fn apply_set_flag_updates(&mut self, flag_updates: &[(char, String)]) {
        for (prefix, flags) in flag_updates {
            let enabled = *prefix == '-';
            for flag in flags.chars() {
                crate::builtins::set::apply_short_set_flag(
                    &mut self.shell_state.env_vars,
                    flag,
                    enabled,
                );
            }
        }
    }

    pub(in crate::executor) fn is_supported_short_set_flag(&self, flag: char) -> bool {
        matches!(flag, 'e' | 'x' | 'u' | 'C' | 'f' | 'n') || short_set_flag_option(flag).is_some()
    }

    pub(in crate::executor) fn expand_case_word(&mut self, word: &str) -> String {
        let mut expanded = if let Some(value) =
            tilde_expand::expand_word_prefix(word, &self.shell_state.env_vars)
        {
            value
        } else {
            self.expand_word(word)
        };
        if expanded.contains("<(") || expanded.contains(">(") {
            expanded = self
                .materialize_assignment_process_substitutions(&expanded)
                .unwrap_or(expanded);
        }
        expanded
    }

    /// Fingerprint of a FUNCTION_STDIN buffer so a deferred command-
    /// substitution cursor write-back can verify it still applies to the
    /// buffer it was recorded against.
    pub(in crate::executor) fn function_stdin_fingerprint(text: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }

    /// Apply a deferred command-substitution stdin consumption: the comsub
    /// child ran on `&self` and could not write env back, so the next
    /// `&mut` consumer folds the child's cursor into FUNCTION_STDIN_OFFSET
    /// (GNU subst.c:7143 — the forked body shares fd 0).
    pub(in crate::executor) fn apply_comsub_stdin_writeback(&mut self) {
        let Some((offset, fingerprint)) = self.comsub_stdin_writeback.take() else {
            return;
        };
        let same_buffer = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN)
            .map(|text| Self::function_stdin_fingerprint(text) == fingerprint)
            .unwrap_or(false);
        if same_buffer {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), offset.to_string());
        }
    }

    /// GNU builtins/read.def: `read` and an external command in the same
    /// `{ ...; }` group share one fd 0 cursor — after `read -d '|'` stops at
    /// the delimiter, `cat` sees only the remainder. FUNCTION_STDIN keeps a
    /// FUNCTION_STDIN_OFFSET cursor for shell reads; feeding a child process
    /// must hand over only the unread tail.
    pub(in crate::executor) fn function_stdin_remaining(&self) -> Option<String> {
        let input = self.shell_state.env_vars.get(FUNCTION_STDIN)?;
        if input.is_empty() {
            return None;
        }
        let offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            .min(input.len());
        Some(input[offset..].to_string())
    }

    /// rubash#1651: the winning fd-0 stdin body for a coproc child,
    /// if the winning redirect is memory-backed (here-string or
    /// heredoc). Same ordering as stdin_string_for_command_mut
    /// (GNU redir.c do_redirections: the last fd-0 input redirect
    /// wins); file and dup winners return None so their existing
    /// paths stay untouched. The mut variant matters: its
    /// here-string expansion strips quotes segment-wise
    /// (expand_here_string_mut), while the read-only fallback
    /// leaves them (`coproc cat <<< "hello"` fed `"hello"`
    /// with quotes). For a well-formed body the mut variant
    /// returns from the body branches before its FUNCTION_STDIN
    /// drain tail, so the drain never fires here.
    pub(crate) fn coproc_stdin_body(&mut self, cmd: &CommandNode) -> Option<String> {
        match fd0_stdin_redirect_winner(cmd) {
            Some(crate::parser::RedirectKind::HereDoc)
            | Some(crate::parser::RedirectKind::HereString) => {
                self.stdin_string_for_command_mut(cmd)
            }
            _ => None,
        }
    }

    pub(in crate::executor) fn stdin_string_for_command_mut(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        self.apply_comsub_stdin_writeback();
        // fd-0 stdin source: GNU redir.c do_redirections applies
        // redirections left to right, so the LAST fd-0 input redirect in
        // cmd.redirects decides — `<<`, `0<<`, `<<<`, `0<<<`, `< file` and
        // `<&N` all compete for fd 0 (`cat <<A 0<<B` reads B's body, `cat
        // <<A <file` reads the file). When no ordered redirect info exists
        // (synthesized commands), fall back to the legacy field checks.
        let last_fd0 = fd0_stdin_redirect_winner(cmd);
        let stdin_body_kind = match last_fd0 {
            Some(kind) => match kind {
                crate::parser::RedirectKind::HereDoc => Some(crate::parser::RedirectKind::HereDoc),
                crate::parser::RedirectKind::HereString => {
                    Some(crate::parser::RedirectKind::HereString)
                }
                // A later `<`/`<&`/`<>`/`<&-` outranks the stdin bodies.
                _ => return self.stdin_string_for_command(cmd),
            },
            None if cmd.heredoc.is_some()
                || cmd.here_string.is_some()
                || cmd.heredoc_redirects.iter().any(|r| r.fd == Some(0)) =>
            {
                None
            }
            None => None,
        };
        let wants_here_string = stdin_body_kind == Some(crate::parser::RedirectKind::HereString)
            || (stdin_body_kind.is_none()
                && cmd.heredoc.is_none()
                && !cmd
                    .heredoc_redirects
                    .iter()
                    .any(|r| !r.here_string && (r.fd.is_none() || r.fd == Some(0))));
        if !wants_here_string {
            if let Some(redirect) = cmd.heredoc_redirects.iter().rev().find(|redirect| {
                !redirect.here_string && (redirect.fd.is_none() || redirect.fd == Some(0))
            }) {
                if redirect.body_carrier.is_some() {
                    return Some(self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier));
                }
                if let Some(body) = redirect.body.clone() {
                    return Some(self.expand_heredoc_body_mut(&body));
                }
            }
            if cmd.heredoc_body.is_some() {
                return Some(self.expand_heredoc_body_mut_from_carrier(&cmd.heredoc_body));
            }
            if let Some(body) = cmd.heredoc.clone() {
                return Some(self.expand_heredoc_body_mut(&body));
            }
        }
        // Here-string content already had quote removal applied by the parser
        // (single quotes stripped); a literal " that lived inside them is now
        // bare data. Expand only substitutions with quotes-as-data semantics so
        // that literal survives (cat <<< 'double"quote' => double"quote).
        // Numbered `0<<<` carries its word in heredoc_redirects instead.
        if stdin_body_kind != Some(crate::parser::RedirectKind::HereDoc) && cmd.heredoc.is_none()
            || wants_here_string
        {
            if let Some(redirect) = cmd
                .heredoc_redirects
                .iter()
                .rev()
                .find(|redirect| redirect.here_string && redirect.fd == Some(0))
            {
                if redirect.body_carrier.is_some() {
                    let mut input =
                        self.expand_here_string_mut_from_carrier(&redirect.body_carrier);
                    input.push('\n');
                    return Some(input);
                }
                if let Some(body) = redirect.body.clone() {
                    if let Some(word) = body.strip_prefix('\u{1d}') {
                        // rubash#1965: a STORAGE_WORD_PREFIX here-string body
                        // is already-decoded text whose $'...' blanks carry
                        // the ANSI_C_IFS_GUARD carrier. The canonical funnel
                        // (expand_stored_here_string_word, the
                        // expand_heredoc_redirect_text ladder) strips the
                        // guard at this value boundary; decoding/expanding
                        // here without the strip leaked U+E401 into the
                        // delivered stream on the pipeline staging route.
                        // Route through the funnel function itself.
                        let mut input = self.expand_stored_here_string_word(word);
                        input.push('\n');
                        return Some(input);
                    }
                    return Some(self.expand_heredoc_body_mut(&body));
                }
            }
            if cmd.here_string_carrier.is_some() {
                let mut input = self.expand_here_string_mut_from_carrier(&cmd.here_string_carrier);
                input.push('\n');
                return Some(input);
            }
            if let Some(word) = cmd.here_string.clone() {
                let decoded = decode_ansi_c_quoted_word(&word);
                let mut input = decoded.unwrap_or_else(|| self.expand_here_string_mut(&word));
                input.push('\n');
                return Some(input);
            }
        }
        let function_stdin_is_source = self.function_stdin_is_command_source(cmd);
        let result = self.stdin_string_for_command(cmd);
        if result.is_some() && function_stdin_is_source {
            // The child drains the stream from the cursor onward; GNU's
            // shared fd 0 means a subsequent `read` sees EOF.
            let end = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN)
                .map(|input| input.len())
                .unwrap_or(0);
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), end.to_string());
        }
        result
    }

    /// True when the command's effective fd 0 resolves to the shared
    /// FUNCTION_STDIN buffer: no explicit `<` redirect wins and no virtual
    /// fd-0 endpoint shadows it (same predicate the drain above applies).
    pub(in crate::executor) fn function_stdin_is_command_source(&self, cmd: &CommandNode) -> bool {
        cmd.redirect_in.is_none()
            && self.virtual_fd_stdin_remaining(0).is_none()
            // rubash#1102/#1103: an exec-rebound fd 0 (file endpoint or
            // closed entry) is the command's source, not FUNCTION_STDIN.
            && !self.fd_table.is_closed(0)
            && !matches!(self.fd_table.read_endpoint(0), Some(FdReadEndpoint::File(_)))
            && self.function_stdin_remaining().is_some()
    }

    pub(in crate::executor) fn stdin_string_for_command(
        &self,
        cmd: &CommandNode,
    ) -> Option<String> {
        // Same fd-0 ordering as the mut variant (GNU redir.c
        // do_redirections): the last fd-0 input redirect in cmd.redirects
        // decides stdin; a `<`/`<&` after `<<`/`<<<` outranks the bodies.
        let last_fd0 = fd0_stdin_redirect_winner(cmd);
        match last_fd0.as_ref() {
            Some(crate::parser::RedirectKind::HereDoc) => {
                if let Some(redirect) = cmd.heredoc_redirects.iter().rev().find(|redirect| {
                    !redirect.here_string && (redirect.fd.is_none() || redirect.fd == Some(0))
                }) {
                    if redirect.body_carrier.is_some() {
                        return Some(
                            self.expand_heredoc_body_readback_from_carrier(
                                &redirect.body_carrier,
                                redirect.body.as_deref(),
                            )
                            .text_lossy(),
                        );
                    }
                    if let Some(body) = redirect.body.as_deref() {
                        return Some(self.expand_heredoc_body_readback(body).text_lossy());
                    }
                }
            }
            Some(crate::parser::RedirectKind::HereString) => {
                if let Some(redirect) = cmd
                    .heredoc_redirects
                    .iter()
                    .rev()
                    .find(|redirect| redirect.here_string && redirect.fd == Some(0))
                {
                    if redirect.body_carrier.is_some() {
                        let mut input = self
                            .expand_heredoc_body_readback_from_carrier(
                                &redirect.body_carrier,
                                redirect.body.as_deref(),
                            )
                            .text_lossy();
                        input.push('\n');
                        return Some(input);
                    }
                    if let Some(body) = redirect.body.as_deref() {
                        if let Some(word) = body.strip_prefix('\u{1d}') {
                            // rubash#1965: the same storage arm as the mut
                            // ladder above — its decode-or-expand result must
                            // pass the value-boundary ANSI_C_IFS_GUARD strip
                            // the canonical funnel applies
                            // (expand_stored_here_string_word, &mut-only, so
                            // this &self ladder applies the identical strip).
                            let mut input = crate::executor::markers::strip_ansi_c_ifs_guard(
                                &decode_ansi_c_quoted_word(word)
                                    .unwrap_or_else(|| self.expand_word(word)),
                            );
                            input.push('\n');
                            return Some(input);
                        }
                        return Some(self.expand_heredoc_body_readback(body).text_lossy());
                    }
                }
                // Unnumbered `<<<` keeps its word in cmd.here_string; it is
                // handled at the bottom of this function.
                return cmd.here_string.as_ref().and_then(|word| {
                    if let Some(carrier) = &cmd.here_string_carrier {
                        if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                            let mut input = text.clone();
                            input.push('\n');
                            return Some(input);
                        }
                    }
                    // Legacy fallback (should not be reached with typed carrier)
                    let mut input = if let Some(pre) = preexpanded_stdin_body(word) {
                        pre.to_string()
                    } else {
                        decode_ansi_c_quoted_word(word)
                            .unwrap_or_else(|| self.expand_embedded_parameters_for_heredoc(word))
                    };
                    input.push('\n');
                    Some(input)
                });
            }
            // A later `< file` / `<&N` / `<>` outranks stdin bodies.
            Some(_) => {}
            // No ordered redirect info (synthesized commands): legacy
            // body-first behavior.
            None => {
                if let Some(redirect) = cmd.heredoc_redirects.iter().rev().find(|redirect| {
                    !redirect.here_string && (redirect.fd.is_none() || redirect.fd == Some(0))
                }) {
                    if let Some(carrier) = &redirect.body_carrier {
                        if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                            let mut input = text.clone();
                            input.push('\n');
                            return Some(input);
                        }
                    }
                    if let Some(body) = redirect.body.as_deref() {
                        if let Some(word) = body.strip_prefix('\u{1d}') {
                            let mut input = decode_ansi_c_quoted_word(word)
                                .unwrap_or_else(|| self.expand_word(word));
                            input.push('\n');
                            return Some(input);
                        }
                        return Some(self.expand_heredoc_body_readback(body).text_lossy());
                    }
                }
                if let Some(carrier) = &cmd.heredoc_body {
                    if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                        let mut input = text.clone();
                        input.push('\n');
                        return Some(input);
                    }
                }
                if let Some(body) = &cmd.heredoc {
                    return Some(self.expand_heredoc_body_readback(body).text_lossy());
                }
            }
        }

        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) != 0 {
                return None;
            }
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                return None;
            }
            // `<&N` / `< /dev/fd/N` / `< /dev/std{N}` are fd aliases: GNU
            // dup2 makes fd 0 read from fd N's current target, i.e. the
            // shell's virtual stdin/pipe — never the host path `/dev/stdin`
            // maps to (niubash#118: `cat < /dev/stdin` must consume the
            // pipe input, not block opening CONIN$).
            if let Some(source_fd) = redirect_target_fd(&target) {
                if let Some(input) = self.virtual_fd_stdin_remaining(source_fd) {
                    // GNU dup2 hands the consumer the live stream: bytes
                    // it took are gone for the next reader (procsub.tests
                    // count_lines `wc -l < $1` five times → 1,0,0,0,0).
                    self.fd_table.drain_input_to_eof(source_fd);
                    return Some(input);
                }
                if source_fd == 0 {
                    if let Some(input) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
                        return Some(input.clone());
                    }
                }
                // No virtual input on the fd: duplicate its real endpoint.
                // fd 0 inherited from the process dups the real stdin
                // handle — a terminal blocks, matching GNU; a null/closed
                // stdin yields EOF.
                if let Ok(mut file) = self.open_fd_read_endpoint(source_fd, &target) {
                    // Raw fd bytes are not necessarily UTF-8 (fd_redirects
                    // c_external_cat_reads_raw_bytes: `exec 3<bin; cat <&3`
                    // must stream 0xff through). Read bytes and re-encode to
                    // shell text; callers decode on the way out.
                    let mut buf = Vec::new();
                    let _ = file.read_to_end(&mut buf);
                    return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                        &buf,
                    ));
                }
                return None;
            }
            // The string channel never slurps a socket (vfs audit
            // §3.3, slice S2): a net target's bytes exist only as the
            // command reads them, and a connected UDP socket has no
            // EOF to slurp to. Decline by registry class — the bind
            // machinery performs the netopen itself and surfaces its
            // errors there. (Before the registry, this decline
            // happened only by accident, when the mapped pseudo-path
            // failed to open as a file.)
            if crate::special_paths::is_net_redirect_target(&target) {
                return None;
            }
            let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path);
            }
            // Q11 /proc P1 (docs/proc-vfs-plan.md hook B): synthetic files
            // are served before the filesystem (read/cat/heredoc stdin path).
            if let Some(bytes) = crate::special_paths::synthetic_proc_content(&target) {
                return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                    &bytes,
                ));
            }
            // rubash#1103: the null device is an immediate EOF, not a live
            // character device — declining here made `cat </dev/null`
            // inside a pipeline-stage group fall through to the ambient
            // stage buffer and print the pipe bytes. GNU reads /dev/null
            // as empty, always.
            if is_null_device(&target) {
                return Some(String::new());
            }
            // GNU redir.c dup2's the descriptor — a character device has
            // no EOF, so slurping blocks forever on the console
            // (test.tests `t -t 0 < /dev/tty` hung via
            // function_call_stdin). Decline the text channel; the caller
            // binds the live fd for the command's duration instead.
            //
            // rubash#1820: the device check and the slurp below must use
            // ONE open. The check used to File::open the target and drop
            // it, and fs::read_to_string then opened it again; on a FIFO
            // the throwaway check open consumes the rendezvous pairing
            // and the slurp open waits for a second writer (or reads
            // EOF in the zero-writer gap), while GNU opens a redirect
            // target exactly once (redir.c do_redirection_internal).
            // A `<(cmd)` temp path is a pipe carrier, not a file: reopening
            // it must resume at the shared offset, not replay the contents
            // (subst.c:7143 command_substitute / redir.c dup semantics).
            // (Consulted before the open, as before: a carrier path need
            // not be openable as a plain path at all.)
            if let Some(bytes) = self.procsub_stream_take(&path) {
                return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                    &bytes,
                ));
            }
            let mut file = File::open(&path).ok()?;
            {
                #[cfg(unix)]
                use std::os::unix::io::AsRawFd;
                #[cfg(windows)]
                use std::os::windows::io::AsRawHandle;
                #[cfg(windows)]
                let raw = file.as_raw_handle() as crate::fd::HANDLE;
                #[cfg(unix)]
                let raw = file.as_raw_fd() as crate::fd::HANDLE;
                if crate::fd::is_char_device_handle(raw) {
                    return None;
                }
            }
            use std::io::Read as _;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).ok()?;
            // fs::read_to_string equivalence: invalid UTF-8 declines
            // the text channel exactly as the two-open version did.
            return String::from_utf8(buf).ok();
        }

        if let Some(input) = self.virtual_fd_stdin_remaining(0) {
            return Some(input);
        }

        // rubash#1102/#1103: an exec-installed fd 0 in the fd table shadows
        // the ambient FUNCTION_STDIN buffer. Inside a pipeline stage,
        // `exec <file` / `exec <&-` rebinds fd 0 for the rest of the stage
        // (GNU redir.c: the redirection is permanent in the forked stage
        // shell), so string-stdin consumers must read the rebound endpoint,
        // not the pipe bytes that were current before the exec. A closed
        // fd 0 is EOF; a file endpoint streams its remaining bytes
        // through the shared-offset dup in open_fd_read_endpoint.
        if self.fd_table.is_closed(0) {
            return Some(String::new());
        }
        if matches!(
            self.fd_table.read_endpoint(0),
            Some(FdReadEndpoint::File(_))
        ) {
            if let Ok(mut file) = self.open_fd_read_endpoint(0, "fd 0") {
                use std::io::Read as _;
                let mut buf = Vec::new();
                let _ = file.read_to_end(&mut buf);
                return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                    &buf,
                ));
            }
        }

        // Check for FUNCTION_STDIN (set by pipeline execution for builtins)
        if let Some(input) = self.function_stdin_remaining() {
            return Some(input);
        }

        let word = cmd.here_string.as_ref()?;
        if let Some(carrier) = &cmd.here_string_carrier {
            if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                let mut input = text.clone();
                input.push('\n');
                return Some(input);
            }
        }
        // Legacy fallback (should not be reached with typed carrier)
        let mut input = if let Some(pre) = preexpanded_stdin_body(word) {
            pre.to_string()
        } else {
            decode_ansi_c_quoted_word(word)
                .unwrap_or_else(|| self.expand_embedded_parameters_for_heredoc(word))
        };
        input.push('\n');
        Some(input)
    }

    /// rubash#1642: the stdin text `source /dev/stdin` must read for a
    /// command — GNU source.def opens /dev/stdin, a dup of fd 0, so the
    /// bytes are whatever fd 0 is bound to for this command (its own
    /// `< file` / `<<<` / `<<EOF` redirections, an enclosing scope's fd-0
    /// binding, or the pipeline stage buffer), resolved by the same
    /// ladder every other stdin consumer uses. None when no channel
    /// binds stdin (plain inherited process stdin).
    pub(crate) fn command_stdin_text(&self, cmd: &CommandNode) -> Option<String> {
        self.stdin_string_for_command(cmd)
    }

    /// rubash#1642: whether the command carries its own fd-0 input
    /// redirection. When it does but resolves to no text (missing file,
    /// closed fd), `source /dev/stdin` must source nothing rather than
    /// fall through to a process stdin GNU never consults for it.
    pub(crate) fn command_has_fd0_input_redirect(&self, cmd: &CommandNode) -> bool {
        fd0_stdin_redirect_winner(cmd).is_some()
    }
}

/// The fd-0 input redirect that wins under GNU left-to-right application
/// (redir.c do_redirections): the last cmd.redirects entry targeting fd 0
/// whose kind supplies stdin (`<`, `<&`, `<>`, `<&-`, `<<`, `<<<`).
/// Returns None when the command carries no ordered redirect info.
pub(in crate::executor) fn fd0_stdin_redirect_winner(
    cmd: &CommandNode,
) -> Option<crate::parser::RedirectKind> {
    cmd.redirects
        .iter()
        .rev()
        .find(|redirect| {
            redirect.fd.unwrap_or(0) == 0
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Input
                        | crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::ReadWrite
                        | crate::parser::RedirectKind::CloseInput
                        | crate::parser::RedirectKind::HereDoc
                        | crate::parser::RedirectKind::HereString
                )
        })
        .map(|redirect| redirect.kind.clone())
}

pub(crate) fn write_stdout_bytes(output: &[u8]) -> io::Result<()> {
    // Builtins that write directly to the process stdout (set -o, declare,
    // alias, ...) must still be captured by command substitution and
    // pipeline stage capture. Thread-local capture makes that visible to
    // every writer, not just the Executor-aware write_default_stdout path.
    if stdout_capture_active() {
        return stdout_capture_write(output);
    }
    write_real_stdout_uncaptured(output)
}

/// Write to the REAL process stdout object, ignoring any active thread-local
/// capture. This is the destination a `Stdout` endpoint's dup record `None`
/// names: GNU `N>&M` (redir.c:1169-1170 `dup2 (redir_fd, redirector)`)
/// copies the open file description CURRENT at dup time, so a dup made while
/// fd 1 was the process stdout keeps referring to that object. The
/// command-substitution child later installs its capture pipe with
/// `dup2 (fildes[1], 1)` (subst.c:7320) — a NEW open file description that
/// replaces only fd 1 — so writes through the dup'd fd still reach the
/// process stdout and escape the substitution (rubash#368:
/// `{ v=$(cmd 3>&1 1>&4); } 4>&1` must print the fd-1 half outside).
pub(crate) fn write_real_stdout_uncaptured(output: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    {
        return trace_stdio_write("stdout", output.len(), || {
            windows_raw_stdio::write_stdout(output)
        });
    }

    #[cfg(not(windows))]
    {
        trace_stdio_write("stdout", output.len(), || {
            std::io::stdout().lock().write_all(output)
        })
    }
}

/// Global stdout write honoring the thread-local capture used by command
/// substitution and pipeline stages. Builtins that write through a plain
/// `std::io::Stdout` handle (set -o, declare -p, shopt) must route through
/// here so their output is captured too.
pub(crate) fn write_global_stdout(output: &[u8]) -> io::Result<()> {
    if stdout_capture_active() {
        return stdout_capture_write(output);
    }
    write_stdout_bytes(output)
}

/// A `std::io::Write` adapter over the global capture-aware stdout.
pub(crate) struct GlobalStdout;

impl std::io::Write for GlobalStdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_global_stdout(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

thread_local! {
    static STDOUT_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
    /// Outer capture buffers parked by nested `begin_stdout_capture`
    /// scopes, oldest first. A dup2'd fd (`2>&1`) keeps writing to the
    /// pipe it was dup'd under — the parked buffer of its generation —
    /// even while a nested command substitution owns the active slot
    /// (rubash#223).
    static STDOUT_CAPTURE_PARKED: std::cell::RefCell<Vec<Vec<u8>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// niubash#345: the shell-side pipeline producer's fd-1 destination,
    /// attached to the capture FRAME that `run_shell_writer_pipeline` opened
    /// for the producing stage. While this frame is on top, fd-1 writes go
    /// to the next stage's OS pipe (SIGPIPE-shaped 141 when the consumer
    /// exits). Nested capture scopes (an inner pipeline's stage bodies, a
    /// command substitution) get `None`, so only the producing element's own
    /// stream — and its pipeline's final placement — leaves through the pipe.
    static STDOUT_CAPTURE_STREAM: std::cell::RefCell<Option<crate::fd::HANDLE>> =
        const { std::cell::RefCell::new(None) };
    /// The enclosing frames' stream targets, parked by `begin_stdout_capture`
    /// and restored by `restore_stdout_capture` (parallel to
    /// `STDOUT_CAPTURE_PARKED`).
    static STDOUT_STREAM_PARKED: std::cell::RefCell<Vec<Option<crate::fd::HANDLE>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// rubash#1141: the armed production-timing recorder (timed_input.rs)
    /// for the producer currently executing on this thread — a pipeline
    /// stage or a process-substitution body. Capture appends note their
    /// arrival against it; nested producers park the outer recorder.
    static CAPTURE_TIMING: std::cell::RefCell<Option<super::timed_input::SharedCaptureTiming>> =
        const { std::cell::RefCell::new(None) };
    static CAPTURE_TIMING_PARKED: std::cell::RefCell<
        Vec<Option<super::timed_input::SharedCaptureTiming>>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

/// rubash#1141: arm a fresh production-timing recorder for the producer
/// about to run (a pipeline stage in the sequential driver, or a
/// process-substitution body), parking any outer one. Returns the
/// recorder the caller finishes into a `TimedInput` once production
/// completes; pair with `end_capture_timing`.
pub(in crate::executor) fn begin_capture_timing() -> super::timed_input::SharedCaptureTiming {
    let recorder = std::rc::Rc::new(std::cell::RefCell::new(
        super::timed_input::CaptureTiming::new(),
    ));
    CAPTURE_TIMING.with(|slot| {
        let previous = slot.borrow_mut().replace(recorder.clone());
        CAPTURE_TIMING_PARKED.with(|parked| parked.borrow_mut().push(previous));
    });
    recorder
}

/// rubash#1141: disarm the current timing recorder, restoring the
/// parked outer one (counterpart of `begin_capture_timing`).
pub(in crate::executor) fn end_capture_timing() {
    CAPTURE_TIMING.with(|slot| {
        let previous = CAPTURE_TIMING_PARKED.with(|parked| parked.borrow_mut().pop());
        *slot.borrow_mut() = previous.flatten();
    });
}

fn capture_timing_with(f: impl FnOnce(&mut super::timed_input::CaptureTiming)) {
    CAPTURE_TIMING.with(|slot| {
        let recorder = slot.borrow().clone();
        if let Some(recorder) = recorder {
            f(&mut recorder.borrow_mut());
        }
    });
}

/// rubash#1141: note one append to the thread-local capture against the
/// armed timing recorder (no-op when none is armed).
pub(in crate::executor) fn capture_timing_note_thread(appended: &[u8], new_raw_len: usize) {
    capture_timing_with(|timing| timing.note_thread(appended, new_raw_len));
}

/// rubash#1141: note one append to an executor's field capture against
/// the armed timing recorder (no-op when none is armed).
pub(in crate::executor) fn capture_timing_note_field(appended: &[u8], new_raw_len: usize) {
    capture_timing_with(|timing| timing.note_field(appended, new_raw_len));
}

/// rubash#1141: snapshot the field (prefix) sink's totals at a
/// field++thread capture assembly merge point.
pub(in crate::executor) fn capture_timing_set_prefix(field_bytes: &[u8]) {
    capture_timing_with(|timing| timing.set_prefix(field_bytes));
}

/// niubash#345: the active capture frame's pipeline-pipe target, if this
/// frame belongs to a shell-side pipeline producer.
pub(in crate::executor) fn active_capture_stream() -> Option<crate::fd::HANDLE> {
    STDOUT_CAPTURE_STREAM.with(|stream| *stream.borrow())
}

/// niubash#345: bind (or clear) the active capture frame's pipeline-pipe
/// target. Called by the pipeline stage helpers right after they open their
/// capture frame, so a producing element streams while every nested frame
/// (inner pipeline stages, substitutions) keeps capturing.
pub(in crate::executor) fn set_capture_stream(stream: Option<crate::fd::HANDLE>) {
    STDOUT_CAPTURE_STREAM.with(|slot| *slot.borrow_mut() = stream);
}

/// Background-output capture pipe for async children spawned while a stdout
/// capture is active (rubash#169). GNU's command-substitution child holds a
/// REAL pipe as fd 1 (subst.c:7143 command_substitute), so `{ cmd; } & wait`
/// inside `$( )` captures the async child's output: every forked async child
/// inherits the same pipe write end and the kernel merges their writes.
/// rubash runs substitution bodies in-process against a Vec capture, so an
/// OS-spawned background child cannot write into it — instead the first
/// background spawn under an active capture creates ONE pipe for the capture
/// scope; every background child's stdout is a duplicate of its write end
/// (kernel-order merge, exactly GNU), and the read end is drained into the
/// capture buffer when the writers are reaped (`wait`) or when the capture
/// scope ends — the blocking drain-to-EOF reproduces GNU's semantics where
/// the parent reading the substitution pipe waits for every async child that
/// inherited fd 1.
struct BackgroundCapture {
    read: std::fs::File,
    /// The pipe's original write handle, kept open for the scope so later
    /// background spawns can duplicate it. Closed by the drain before
    /// reading (EOF then means "every spawned writer has exited").
    write: crate::fd::HANDLE,
    write_open: bool,
    writer_pids: std::collections::HashSet<u32>,
}

impl BackgroundCapture {
    /// Hand out one more write end (owned File) for a background child.
    fn duplicate_write(&self) -> Option<std::fs::File> {
        if !self.write_open {
            return None;
        }
        let dup = crate::fd::duplicate_handle(self.write).ok()?;
        Some(crate::fd::handle_to_file(dup))
    }

    /// Read every byte currently sitting in the pipe WITHOUT waiting for EOF
    /// (the scope's original write end is still open, so a blocking read
    /// would never see EOF). Called when the last registered child exits:
    /// everything a dead child wrote is already in the pipe, so a
    /// read-until-empty loop collects exactly its output at the moment the
    /// shell observed the exit — the ordering GNU's shared pipe gets from
    /// the kernel.
    fn drain_available(&mut self) -> std::io::Result<Vec<u8>> {
        crate::fd::read_available(&self.read)
    }

    /// Close the original write end, then read to EOF. EOF arrives once every
    /// background child that holds a duplicated write end has exited — the
    /// same condition GNU's substitution-pipe read blocks on.
    fn drain(&mut self) -> std::io::Result<Vec<u8>> {
        if self.write_open {
            crate::fd::close_handle(self.write);
            self.write_open = false;
        }
        let mut buffer = Vec::new();
        self.read.read_to_end(&mut buffer)?;
        Ok(buffer)
    }
}

thread_local! {
    static BG_CAPTURE: std::cell::RefCell<Option<BackgroundCapture>> =
        const { std::cell::RefCell::new(None) };
    static BG_CAPTURE_SAVED: std::cell::RefCell<Vec<Option<BackgroundCapture>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Reserve a stdout pipe write end for a background child spawned under an
/// active capture. Creates the scope's pipe on first use; subsequent calls
/// duplicate the same write end so every background child of the capture
/// scope shares one pipe (kernel-order merge).
pub(in crate::executor) fn background_capture_write_end() -> Option<std::fs::File> {
    if !stdout_capture_active() {
        return None;
    }
    BG_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        if capture.is_none() {
            let (read, write) = crate::fd::create_capture_pipe().ok()?;
            *capture = Some(BackgroundCapture {
                read: crate::fd::handle_to_file(read),
                write,
                write_open: true,
                writer_pids: std::collections::HashSet::new(),
            });
        }
        capture
            .as_ref()
            .and_then(BackgroundCapture::duplicate_write)
    })
}

/// Record that the just-spawned background child `pid` holds a capture write
/// end (called after a successful spawn).
pub(in crate::executor) fn register_background_capture_writer(pid: u32) {
    BG_CAPTURE.with(|capture| {
        if let Some(state) = capture.borrow_mut().as_mut() {
            state.writer_pids.insert(pid);
        }
    });
}

/// A background child that held a capture write end left the live set
/// (reaped by `wait` / refresh). When it was the last writer, drain the pipe
/// now — bytes already written land in the capture buffer at the point the
/// shell observed the child's exit, preserving the `& ... wait; echo after`
/// ordering GNU's single shared pipe produces. Returns the drained bytes (empty
/// when writers remain or no capture pipe exists).
pub(in crate::executor) fn retire_background_capture_writer(pid: u32) -> Vec<u8> {
    BG_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        let Some(state) = capture.as_mut() else {
            return Vec::new();
        };
        let was_writer = state.writer_pids.remove(&pid);
        if was_writer && state.writer_pids.is_empty() {
            state.drain_available().unwrap_or_default()
        } else {
            Vec::new()
        }
    })
}

pub(in crate::executor) fn stdout_capture_active() -> bool {
    STDOUT_CAPTURE.with(|capture| capture.borrow().is_some())
}

/// Write one chunk to a capture stream (a frame's armed pipeline pipe),
/// normalizing a broken pipe to `io::ErrorKind::BrokenPipe` so callers
/// that translate io errors can recover the SIGPIPE shape.
fn write_capture_stream(stream: crate::fd::HANDLE, output: &[u8]) -> io::Result<()> {
    crate::fd::write_all(stream, output).map_err(|error| {
        if is_broken_pipe_error(&error) {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pipeline consumer closed the pipe",
            )
        } else {
            error
        }
    })
}

pub(in crate::executor) fn stdout_capture_write(output: &[u8]) -> io::Result<()> {
    // rubash#1237 (item 4): a capture frame armed with a pipeline stream
    // (the niubash#345 shell-writer producer frame, or a streaming tail)
    // has the OS pipe as its fd-1 destination. GNU binds the element's
    // pipe to fd 1 and dups fd 2 onto the SAME open file description
    // (execute_cmd.c:2702-2723 execute_pipeline, redir.c:1169-1170 dup2),
    // so every write resolving to the element's stdout object lands on
    // the one channel in write order. Buffering this frame's writes
    // while the fd-1 endpoint arm streams its writes live splits the
    // merged fd1/fd2 pair across two sinks flushed at different times
    // (the stream in real time, the frame at stage end) and destroys
    // the interleave — `(echo o1; echo e >&2; echo o2) 2>&1 | cat`
    // printed o1/o2/e. Route the frame's writes to its stream at write
    // time, whatever fd or helper carried them; unarmed frames keep
    // buffering exactly as before.
    if let Some(stream) = active_capture_stream() {
        return write_capture_stream(stream, output);
    }
    STDOUT_CAPTURE.with(|capture| {
        if let Some(buffer) = capture.borrow_mut().as_mut() {
            buffer.write_all(output)?;
            capture_timing_note_thread(output, buffer.len());
        }
        Ok(())
    })
}

/// `stdout_capture_write` with the SIGPIPE shape restored for
/// `ExecuteError` contexts: when the frame's stream reports a broken
/// pipe, the write is the element's SIGPIPE death
/// (`ExecuteError::BrokenPipe`, status 141), not a generic io failure —
/// the buffered frame could never fail before its writes learned to
/// stream (rubash#1237 item 4).
pub(in crate::executor) fn stdout_capture_write_mapped(output: &[u8]) -> Result<(), ExecuteError> {
    stdout_capture_write(output).map_err(|error| {
        if error.kind() == io::ErrorKind::BrokenPipe && active_capture_stream().is_some() {
            ExecuteError::BrokenPipe
        } else {
            ExecuteError::IoError(error)
        }
    })
}

/// Zero-based generation of the currently active capture scope (0 = first).
/// `FdTable::stdout_alias_generation` records this at dup time so a dup2'd
/// `Stdout` marker can resolve back to the buffer that was active then
/// (rubash#223).
pub(in crate::executor) fn stdout_capture_generation() -> usize {
    STDOUT_CAPTURE_PARKED.with(|parked| parked.borrow().len())
}

/// Write to the capture buffer of `generation` — the active one when it is
/// the current scope, the parked outer buffer while a nested capture owns
/// the active slot. A generation beyond every live scope (its reader is
/// gone, or the fd crossed onto a worker thread with its own capture
/// stack) falls back to the live resolution rather than dropping
/// (rubash#223).
pub(in crate::executor) fn write_stdout_capture_at_generation(
    output: &[u8],
    generation: usize,
) -> io::Result<()> {
    let parked_len = stdout_capture_generation();
    if generation == parked_len {
        return stdout_capture_write(output);
    }
    if generation < parked_len {
        // A parked frame that was armed with a pipeline stream has the
        // pipe as its destination (see stdout_capture_write): a dup2'd fd
        // holding this generation writes the pipe in write order, not
        // the parked copy (rubash#1237 item 4). The stream stack is
        // parked in parallel with the buffer stack, same index.
        let parked_stream =
            STDOUT_STREAM_PARKED.with(|parked| parked.borrow().get(generation).copied().flatten());
        if let Some(stream) = parked_stream {
            return write_capture_stream(stream, output);
        }
        return STDOUT_CAPTURE_PARKED.with(|parked| {
            if let Some(buffer) = parked.borrow_mut().get_mut(generation) {
                buffer.write_all(output)?;
            }
            Ok(())
        });
    }
    if stdout_capture_active() {
        stdout_capture_write(output)
    } else {
        write_stdout_bytes(output)
    }
}

/// niubash#345: write a shell-side pipeline producer's fd-1 bytes to the
/// next stage's OS pipe. A broken pipe means the downstream consumer closed
/// its read end — GNU's SIGPIPE — so the producing stage terminates with the
/// synthesized SIGPIPE status (141), matching `lingerer_sigpipe_status` and
/// the external-member contract. Any other IO error propagates normally.
pub(in crate::executor) fn write_pipeline_stream(
    handle: crate::fd::HANDLE,
    output: &[u8],
) -> Result<(), ExecuteError> {
    match crate::fd::write_all(handle, output) {
        Ok(()) => Ok(()),
        Err(error) if is_broken_pipe_error(&error) => Err(ExecuteError::BrokenPipe),
        Err(error) => Err(ExecuteError::IoError(error)),
    }
}

/// True when `path` names an existing FIFO (unix). A FIFO open carries
/// rendezvous semantics — a blocking open returns only when the peer
/// side opens — so every throwaway probe/prepare open of a FIFO
/// consumes a pairing the one real open needs (GNU opens a redirect
/// target exactly once: redir.c do_redirection_internal). Windows has
/// no filesystem FIFOs.
pub(in crate::executor) fn redirect_path_is_fifo(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        return std::fs::metadata(path)
            .map(|meta| meta.file_type().is_fifo())
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// The reject gate's `< word` validity probe (rubash#1820): a plain
/// `File::open` is validation-pure for regular files, but on a FIFO the
/// throwaway O_RDONLY open consumes the rendezvous pairing (and blocks
/// until a writer appears), so an existing FIFO validates through
/// access(2) — the same existence/permission verdict, no fd taken.
pub(in crate::executor) fn probe_input_redirect_gate(
    path: &std::path::Path,
) -> std::io::Result<()> {
    #[cfg(unix)]
    if redirect_path_is_fifo(path) {
        if let Some(error) = redirect_fifo_access_error(path, false) {
            return Err(error);
        }
        return Ok(());
    }
    std::fs::File::open(path).map(|_| ())
}

/// access(2) validation for a FIFO redirect target whose real open is
/// deferred to the bind phase (rubash#1820): proves existence and the
/// requested access without taking an fd, so no rendezvous pairing is
/// consumed. Returns the io error the corresponding open would have
/// surfaced, or None when access is granted.
#[cfg(unix)]
pub(in crate::executor) fn redirect_fifo_access_error(
    path: &std::path::Path,
    write: bool,
) -> Option<std::io::Error> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mode = if write { libc::W_OK } else { libc::R_OK };
    if unsafe { libc::access(c_path.as_ptr(), mode) } == 0 {
        None
    } else {
        Some(std::io::Error::last_os_error())
    }
}

/// The setup walk's output probe (create/truncate side effect plus
/// error surfacing) with the rubash#1820 FIFO carve-out: on a FIFO the
/// create is a no-op whose blocking open consumes the bind open's
/// rendezvous pairing, so validate write access only — the bind phase
/// performs the one real open GNU performs.
pub(in crate::executor) fn probe_output_redirect(
    executor: &Executor,
    target: &str,
    clobber: bool,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let path = executor.session_path(shell_path_to_windows(
            &executor.redirect_output_path_target(target),
            &executor.shell_state.env_vars,
        ));
        if redirect_path_is_fifo(&path) {
            if let Some(error) = redirect_fifo_access_error(&path, true) {
                return Err(crate::posix_errors::path_error(target, error));
            }
            return Ok(());
        }
    }
    executor.create_redirect_output(target, clobber).map(|_| ())
}

/// Append-arm twin of [`probe_output_redirect`]. The open itself
/// classifies through the registry (Executor::open_append_target,
/// rubash#1964): the probe IS an append open, so it must succeed and
/// fail for exactly the shapes the bind's open does — a net target
/// connects, a synthetic /proc entry answers by write class, a
/// closed fd alias fails ENOENT.
pub(in crate::executor) fn probe_output_redirect_append(
    executor: &Executor,
    target: &str,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let path = executor.session_path(shell_path_to_windows(
            target,
            &executor.shell_state.env_vars,
        ));
        if redirect_path_is_fifo(&path) {
            if let Some(error) = redirect_fifo_access_error(&path, true) {
                return Err(crate::posix_errors::path_error(target, error));
            }
            return Ok(());
        }
    }
    executor
        .open_append_target(target)
        .map(|_| ())
        .map_err(|e| crate::posix_errors::path_error(target, e))
}

/// Feed a spawned child's stdin pipe with the shell's virtual-stdin
/// payload (heredoc / here-string / FUNCTION_STDIN text materialized by
/// the in-process model). GNU has no such post-spawn feed: the element's
/// fd 0 IS the pipe or file (execute_cmd.c:2620 execute_pipeline binds
/// the fildes; redirections are applied inside the child by
/// do_redirection_internal), so a consumer that exits without reading
/// simply leaves the bytes in the pipe and its own exit status governs.
/// The feed is therefore a transport detail of this port, and its two
/// failure faces must not leak into the command's result:
///
/// - a broken pipe means the child closed stdin (in practice it already
///   exited) — the feed just ends, exactly like the writer-thread feed
///   sites swallow it (pipeline member spawn, background spawn);
/// - SIGPIPE is SIG_DFL process-wide (rubash#1333), so an unguarded
///   feed on a non-pipeline executor thread would kill the SHELL when
///   the child wins the exit race (`cmd <<< x` with a fast-exiting
///   `cmd`) — hold the scoped block across the write so the race can
///   only ever surface as the swallowed EPIPE above.
///
/// Any other io error is a genuine environment failure and propagates
/// (rubash#1822).
pub(in crate::executor) fn feed_child_stdin(
    stdin: &mut impl std::io::Write,
    payload: &[u8],
) -> Result<(), ExecuteError> {
    #[cfg(unix)]
    let _sigpipe_block = crate::builtins::kill::sigpipe_block_guard();
    match stdin.write_all(payload) {
        Ok(()) => Ok(()),
        Err(error) if is_broken_pipe_error(&error) => Ok(()),
        Err(error) => Err(ExecuteError::IoError(error)),
    }
}

/// True when a write failed because the read end of the pipe is gone —
/// Windows ERROR_BROKEN_PIPE (109) or a POSIX EPIPE.
pub(in crate::executor) fn is_broken_pipe_error(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        return true;
    }
    #[cfg(windows)]
    {
        const ERROR_BROKEN_PIPE: i32 = 109;
        const ERROR_NO_DATA: i32 = 232;
        if let Some(code) = error.raw_os_error() {
            return code == ERROR_BROKEN_PIPE || code == ERROR_NO_DATA;
        }
    }
    #[cfg(unix)]
    {
        if let Some(code) = error.raw_os_error() {
            return code == 32; // EPIPE
        }
    }
    false
}

/// Begins thread-local stdout capture, returning the previous capture buffer
/// (if any) so callers can nest captures and restore afterwards. The
/// background-capture pipe state is saved alongside: a nested capture scope
/// is a fresh substitution pipe in GNU terms, so background children spawned
/// inside it bind to the inner scope's pipe, never the outer one's.
pub(in crate::executor) fn begin_stdout_capture() -> Option<Vec<u8>> {
    let previous = STDOUT_CAPTURE.with(|capture| {
        let previous = capture.borrow_mut().take();
        *capture.borrow_mut() = Some(Vec::new());
        previous
    });
    // Park a copy of the outer buffer for dup2'd fds still holding its
    // generation (rubash#223): their writes during this nested scope append
    // to the parked copy, and `restore_stdout_capture` merges it back.
    STDOUT_CAPTURE_PARKED.with(|parked| {
        parked
            .borrow_mut()
            .push(previous.clone().unwrap_or_default());
    });
    let saved_bg = BG_CAPTURE.with(|state| state.borrow_mut().take());
    BG_CAPTURE_SAVED.with(|saved| saved.borrow_mut().push(saved_bg));
    // niubash#345: a nested capture frame starts with no pipeline-pipe
    // target of its own; the enclosing frame's target is parked and comes
    // back in `restore_stdout_capture`.
    let enclosing_stream = STDOUT_CAPTURE_STREAM.with(|slot| slot.borrow_mut().take());
    STDOUT_STREAM_PARKED.with(|parked| parked.borrow_mut().push(enclosing_stream));
    previous
}

/// Ends thread-local stdout capture and returns the captured bytes. Any
/// background child still holding a capture write end is drained first —
/// read-to-EOF blocks until it exits, which is exactly what GNU's
/// substitution-pipe reader does (the parent's read completes only after
/// every async child that inherited fd 1 is gone).
pub(in crate::executor) fn take_stdout_capture() -> Vec<u8> {
    let mut buffer = STDOUT_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default());
    let drained = BG_CAPTURE.with(|state| {
        let drained = state
            .borrow_mut()
            .as_mut()
            .map(BackgroundCapture::drain)
            .and_then(Result::ok)
            .unwrap_or_default();
        *state.borrow_mut() = None;
        drained
    });
    buffer.extend_from_slice(&drained);
    buffer
}

/// Restores a previously saved capture buffer (used after nested captures).
pub(in crate::executor) fn restore_stdout_capture(previous: Option<Vec<u8>>) {
    // Merge the parked outer-generation copy back (rubash#223): it began as
    // a clone of `previous` and only ever APPENDED (dup2'd fds writing
    // during the nested scope), so when it is a superset it carries the
    // nested scope's marker writes and becomes the restored active buffer.
    // An unrelated buffer (asymmetric begin/restore use) falls back to the
    // caller's `previous` unchanged.
    let parked = STDOUT_CAPTURE_PARKED.with(|parked| parked.borrow_mut().pop());
    let merged = match (previous, parked) {
        (Some(previous), Some(parked)) if parked.len() >= previous.len() => Some(parked),
        (previous, _) => previous,
    };
    STDOUT_CAPTURE.with(|capture| {
        *capture.borrow_mut() = merged;
    });
    let saved_bg = BG_CAPTURE_SAVED.with(|saved| saved.borrow_mut().pop().flatten());
    if let Some(state) = saved_bg {
        BG_CAPTURE.with(|state_slot| *state_slot.borrow_mut() = Some(state));
    }
    // niubash#345: reinstate the enclosing capture frame's pipeline-pipe
    // target (parallel to the buffer popped above).
    let enclosing_stream = STDOUT_STREAM_PARKED
        .with(|parked| parked.borrow_mut().pop())
        .flatten();
    STDOUT_CAPTURE_STREAM.with(|slot| *slot.borrow_mut() = enclosing_stream);
}

/// Runs `body` under a fresh thread-local stdout capture and returns the
/// captured bytes together with the body's result.
///
/// Command substitutions, process substitutions, and pipeline stages run
/// bodies on executors whose `stdout_capture` field only intercepts the
/// `write_default_stdout` path. Builtins that write the process stdout
/// directly (`write_stdout_bytes` / `write_global_stdout`) consult the
/// thread-local capture — which, when the substitution runs inside a
/// pipeline stage, belongs to the enclosing stage, leaking substitution
/// output into the stage's pipe (GNU subst.c command_substitute forks and
/// captures the fd itself). Installing a fresh capture for the body keeps
/// every write in the substitution's own output; the outer buffer is
/// restored afterwards.
pub(in crate::executor) fn capture_stdout<R>(body: impl FnOnce() -> R) -> (Vec<u8>, R) {
    let saved = begin_stdout_capture();
    let result = body();
    let captured = take_stdout_capture();
    restore_stdout_capture(saved);
    (captured, result)
}

/// niubash#345: `capture_stdout` for a shell-side pipeline producer. The
/// frame opened here carries `stream` as its fd-1 destination, so the body's
/// writes go to the next stage's OS pipe instead of the capture buffer; a
/// nested frame the body opens (an inner pipeline stage, a substitution)
/// still captures normally.
pub(in crate::executor) fn capture_stdout_streamed<R>(
    stream: Option<crate::fd::HANDLE>,
    body: impl FnOnce() -> R,
) -> (Vec<u8>, R) {
    let saved = begin_stdout_capture();
    set_capture_stream(stream);
    let result = body();
    let captured = take_stdout_capture();
    restore_stdout_capture(saved);
    (captured, result)
}

pub(crate) fn write_stderr_bytes(output: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    {
        return windows_raw_stdio::write_stderr(output);
    }

    #[cfg(not(windows))]
    {
        std::io::stderr().lock().write_all(output)
    }
}

fn trace_stdio_write(
    stream: &str,
    bytes: usize,
    write: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    // The trace switch is a debug env var fixed for the process lifetime,
    // so read it once. A raw getenv/std::env::var_os on every builtin
    // write was a visible per-iteration cost in loops that run a builtin
    // a few times per command (`while [ ... ]` -> execute_test_words ->
    // write_buffered_builtin_output -> here): each call took the
    // std::env lock and scanned the (potentially large) environment
    // (rubash#437). OnceLock collapses it to one relaxed load.
    static TRACE_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *TRACE_ENABLED.get_or_init(|| std::env::var_os("RUBASH_STDIO_TRACE").is_some());
    if !enabled {
        return write();
    }

    let start = std::time::Instant::now();
    let result = write();
    if bytes >= 1024 {
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let _ = writeln!(
            std::io::stderr().lock(),
            "rubash_stdio_trace stream={stream} bytes={bytes} elapsed_ms={elapsed_ms:.1}"
        );
    }
    result
}

#[cfg(windows)]
mod windows_raw_stdio {
    use std::ffi::c_void;
    use std::io;
    use std::ptr;

    const CP_UTF8: u32 = 65001;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;
    const WRITE_CHUNK_SIZE: usize = 1024 * 1024;

    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
        fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
        fn WriteFile(
            hFile: *mut c_void,
            lpBuffer: *const c_void,
            nNumberOfBytesToWrite: u32,
            lpNumberOfBytesWritten: *mut u32,
            lpOverlapped: *mut c_void,
        ) -> i32;
    }

    pub(super) fn write_stdout(output: &[u8]) -> io::Result<()> {
        write_handle(STD_OUTPUT_HANDLE, output)
    }

    pub(super) fn write_stderr(output: &[u8]) -> io::Result<()> {
        write_handle(STD_ERROR_HANDLE, output)
    }

    fn write_handle(std_handle: u32, mut output: &[u8]) -> io::Result<()> {
        if output.is_empty() {
            return Ok(());
        }

        let handle = unsafe { GetStdHandle(std_handle) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        unsafe {
            SetConsoleOutputCP(CP_UTF8);
        }

        while !output.is_empty() {
            let chunk_len = output.len().min(WRITE_CHUNK_SIZE);
            let mut written = 0u32;
            let ok = unsafe {
                WriteFile(
                    handle,
                    output.as_ptr().cast(),
                    chunk_len as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write stdout bytes",
                ));
            }
            output = &output[written as usize..];
        }

        Ok(())
    }
}

/// Connected TCP stream as a std File (rubash#1026).
#[cfg(unix)]
fn tcp_stream_file(stream: std::net::TcpStream) -> File {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    unsafe { File::from_raw_fd(stream.into_raw_fd()) }
}

/// Connected TCP stream as a std File (rubash#1026).
#[cfg(windows)]
fn tcp_stream_file(stream: std::net::TcpStream) -> File {
    // A SOCKET is a kernel handle: File adopts it through FromRawHandle
    // (FromRawSocket is only implemented for the std socket types, not File).
    use std::os::windows::io::{FromRawHandle, IntoRawSocket};
    unsafe { File::from_raw_handle(stream.into_raw_socket() as _) }
}

/// Connected UDP socket as a std File (rubash#1026).
#[cfg(unix)]
fn udp_socket_file(socket: std::net::UdpSocket) -> File {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    unsafe { File::from_raw_fd(socket.into_raw_fd()) }
}

/// Connected UDP socket as a std File (rubash#1026).
#[cfg(windows)]
fn udp_socket_file(socket: std::net::UdpSocket) -> File {
    use std::os::windows::io::{FromRawHandle, IntoRawSocket};
    unsafe { File::from_raw_handle(socket.into_raw_socket() as _) }
}
