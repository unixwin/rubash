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

#[derive(Debug, Clone)]
pub(crate) enum FdReadEndpoint {
    Text(Rc<RefCell<TextInput>>),
    #[allow(dead_code)]
    File(PathBuf),
    InheritedProcessStdin,
    ProcessSubstitution(Rc<RefCell<TextInput>>),
    CoprocStdout(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FdWriteEndpoint {
    Stdout,
    Stderr,
    File(PathBuf),
    CoprocStdin(u32),
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
}

impl FdTable {
    pub(crate) fn new() -> Self {
        let mut table = Self {
            entries: BTreeMap::new(),
            next_dynamic_fd: 10,
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

    pub(crate) fn dup_input(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        let endpoint = self
            .entries
            .get(&source)
            .filter(|entry| !entry.closed)
            .and_then(|entry| entry.read.clone())
            .ok_or_else(|| {
                if self.entries.contains_key(&source) {
                    FdError::NotOpenForRead
                } else {
                    FdError::Closed
                }
            })?;
        let dynamic = self.is_dynamic(target);
        self.open_input(target, endpoint, dynamic);
        Ok(())
    }

    pub(crate) fn dup_output(&mut self, target: u32, source: u32) -> Result<(), FdError> {
        let endpoint = self
            .entries
            .get(&source)
            .filter(|entry| !entry.closed)
            .and_then(|entry| entry.write.clone())
            .ok_or_else(|| {
                if self.entries.contains_key(&source) {
                    FdError::NotOpenForWrite
                } else {
                    FdError::Closed
                }
            })?;
        let dynamic = self.is_dynamic(target);
        self.open_output(target, endpoint, dynamic);
        Ok(())
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

    pub(crate) fn read_all_bytes(&mut self, fd: u32) -> Option<Vec<u8>> {
        let endpoint = self.entries.get(&fd)?.read.clone()?;
        let input = match endpoint {
            FdReadEndpoint::Text(input) | FdReadEndpoint::ProcessSubstitution(input) => input,
            _ => return None,
        };
        let mut input = input.borrow_mut();
        let result = input.data.get(input.offset..)?.to_vec();
        input.offset = input.data.len();
        Some(result)
    }

    pub(crate) fn consume_all_text(&mut self, fd: u32) -> Option<usize> {
        let endpoint = self.entries.get(&fd)?.read.clone()?;
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
                    FdReadEndpoint::File(path) => MaterializedRead::File(path.clone()),
                    FdReadEndpoint::InheritedProcessStdin => {
                        MaterializedRead::InheritedProcessStdin
                    }
                    FdReadEndpoint::CoprocStdout(pid) => MaterializedRead::CoprocStdout(*pid),
                });
                let write = entry.write.as_ref().map(|endpoint| match endpoint {
                    FdWriteEndpoint::Stdout => MaterializedWrite::Stdout,
                    FdWriteEndpoint::Stderr => MaterializedWrite::Stderr,
                    FdWriteEndpoint::File(path) => MaterializedWrite::File(path.clone()),
                    FdWriteEndpoint::CoprocStdin(pid) => MaterializedWrite::CoprocStdin(*pid),
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
