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
    CoprocStdout { pid: u32, fd: Rc<FileFd> },
}

#[derive(Debug, Clone)]
pub(crate) enum FdWriteEndpoint {
    Stdout,
    Stderr,
    File(Rc<FileFd>),
    CoprocStdin { pid: u32, fd: Rc<FileFd> },
    ProcessSubstitution { path: PathBuf, command: String },
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
    pub(crate) next_dynamic_fd: u32,
    /// builtins/read.def read_timeout: while `read -t N` runs, handle-backed
    /// reads wait at most until this deadline for input. `None` means an
    /// unbounded read. Only the blocking endpoints (File/CoprocStdout)
    /// consult it; buffered text endpoints answer instantly.
    pub(crate) read_deadline: Option<std::time::Instant>,
    /// Set when a bounded read expired mid-record — GNU still assigns the
    /// partial input and returns 128+SIGALRM (read.def:539-562).
    pub(crate) read_timed_out: bool,
}

impl FdTable {
    pub(crate) fn new() -> Self {
        let mut table = Self {
            entries: BTreeMap::new(),
            next_dynamic_fd: 10,
            read_deadline: None,
            read_timed_out: false,
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

    pub(crate) fn allocate_dynamic_with_limit(&mut self, limit: Option<u32>) -> Option<u32> {
        let upper = limit.map(|value| value as usize).unwrap_or(1024);
        // GNU fcntl F_DUPFD with SHELL_FD_BASE=10 fails with EINVAL when the
        // requested base is beyond RLIMIT_NOFILE (redir.c fcntl(...10) path).
        // Mirror that by treating no fd >=10 and <limit as allocation failure
        // (vredir6.sub: ulimit -n 6 then exec {v}</dev/null leaves v unset).
        let fd = (10..1024).find(|fd| {
            if limit.is_some_and(|lim| *fd >= lim) {
                return false;
            }
            if (*fd as usize) >= upper {
                return false;
            }
            self.entries
                .get(fd)
                .map_or(true, |entry| !Self::occupied(entry))
        })?;
        self.next_dynamic_fd = fd.saturating_add(1).max(10);
        Some(fd)
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

    pub(crate) fn read_text(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact: bool,
    ) -> Option<String> {
        self.read_bytes(fd, delimiter as u8, char_limit, exact)
            .map(|bytes| bytes_to_shell_text(&bytes))
    }

    pub(crate) fn read_bytes(
        &mut self,
        fd: u32,
        delimiter: u8,
        char_limit: Option<usize>,
        exact: bool,
    ) -> Option<Vec<u8>> {
        let endpoint = self.entries.get(&fd)?.read.clone()?;
        let file = match &endpoint {
            FdReadEndpoint::File(file) | FdReadEndpoint::CoprocStdout { fd: file, .. } => {
                Some(file.clone())
            }
            _ => None,
        };
        if let Some(file) = file {
            return self.read_file_bytes(file, delimiter, char_limit, exact);
        }
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let mut input = input.borrow_mut();
        if input.offset >= input.data.len() {
            return None;
        }
        if char_limit == Some(0) {
            return Some(Vec::new());
        }
        let slice = &input.data[input.offset..];
        let mut consumed = 0;
        let mut chars = 0;
        for (index, byte) in slice.iter().copied().enumerate() {
            if !exact && byte == delimiter {
                consumed = index + 1;
                break;
            }
            if byte & 0xc0 != 0x80 {
                chars += 1;
            }
            consumed = index + 1;
            if char_limit.is_some_and(|limit| chars >= limit) {
                break;
            }
        }
        if consumed == 0 {
            return None;
        }
        let result_len = if !exact && slice[consumed - 1] == delimiter {
            consumed - 1
        } else {
            consumed
        };
        let result = slice[..result_len].to_vec();
        input.offset += consumed;
        Some(result)
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
        delimiter: u8,
        char_limit: Option<usize>,
        exact: bool,
    ) -> Option<Vec<u8>> {
        if char_limit == Some(0) {
            return Some(Vec::new());
        }
        let mut result = Vec::new();
        let mut chars = 0usize;
        let mut consumed = false;
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
                Err(_) => break,
            };
            consumed = true;
            let mut keep = true;
            if !exact && byte == delimiter {
                keep = false; // delimiter is consumed but not returned
            }
            if byte & 0xc0 != 0x80 {
                chars += 1;
            }
            if keep {
                result.push(byte);
            }
            if !keep || char_limit.is_some_and(|limit| chars >= limit) {
                break;
            }
        }
        // EOF with zero bytes read must surface as None — `read` at EOF
        // exits 1, distinct from an empty line (delimiter consumed).
        if !consumed {
            return None;
        }
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

    pub(crate) fn input_snapshot_bytes(&self, fd: u32) -> Option<(Vec<u8>, usize)> {
        let endpoint = self.entries.get(&fd)?.read.as_ref()?;
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let input = input.borrow();
        Some((input.data.clone(), input.offset))
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
                });
                let write = entry.write.as_ref().map(|endpoint| match endpoint {
                    FdWriteEndpoint::Stdout => MaterializedWrite::Stdout,
                    FdWriteEndpoint::Stderr => MaterializedWrite::Stderr,
                    FdWriteEndpoint::File(file) => MaterializedWrite::File(file.path.clone()),
                    FdWriteEndpoint::CoprocStdin { pid, .. } => {
                        MaterializedWrite::CoprocStdin(*pid)
                    }
                    FdWriteEndpoint::ProcessSubstitution { path, command } => {
                        MaterializedWrite::ProcessSubstitution {
                            path: path.clone(),
                            command: command.clone(),
                        }
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
            (Self::File(a), Self::File(b)) => Rc::ptr_eq(a, b),
            (Self::CoprocStdin { pid: a, .. }, Self::CoprocStdin { pid: b, .. }) => a == b,
            (
                Self::ProcessSubstitution {
                    path: p1,
                    command: c1,
                },
                Self::ProcessSubstitution {
                    path: p2,
                    command: c2,
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
        })))
    }

    pub(crate) fn text(input: impl Into<String>) -> Self {
        Self::bytes(input.into().into_bytes())
    }

    pub(crate) fn process_substitution(input: impl Into<String>) -> Self {
        Self::ProcessSubstitution(Rc::new(RefCell::new(TextInput {
            data: input.into().into_bytes(),
            offset: 0,
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
            table.read_text(10, '\n', None, false).as_deref(),
            Some("one")
        );
        table.move_input(11, 10).unwrap();
        assert!(table.is_closed(10));
        assert_eq!(
            table.read_text(11, '\n', None, false).as_deref(),
            Some("two")
        );
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
