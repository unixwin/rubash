//! /proc minimal emulation (TASKBOARD Q11, docs/proc-vfs-plan.md P1).
//!
//! Windows has no procfs; GNU bash on Windows has none either, so this is a
//! userland value-add layer OUTSIDE the GNU compatibility contract and NOT
//! part of the 83-suite ledger. Unix builds must never intercept /proc --
//! a real kernel procfs exists there and a synthesis layer would shadow it
//! (a semantic-correctness hazard, not a compile-hygiene one), so the
//! non-Windows stub answers None and paths fall through to the real fs.
//!
//! Field names and line formats follow Linux downstream grep habits so tools
//! like `free`/`grep MemTotal`/monitoring scripts keep working: the meminfo
//! line SET mirrors Linux (values derived from GlobalMemoryStatusEx), the
//! stat line set is cpu/per-cpu/intr/ctxt/btime/processes/procs_running/
//! procs_blocked/softirq, cpuinfo emits the standard per-CPU block, loadavg
//! is five fields, uptime is two floats.
//!
//! niubash#758 completeness wave: the per-pid face grew the entries
//! procps actually reads — `stat` (the 52-field line: comm/state/ppid
//! from the snapshot, utime/stime from GetProcessTimes, vsize/rss from
//! the VM counters, the thread count from the snapshot record),
//! `statm` (the same memory data in pages), and `environ` (the true
//! environment block: GetEnvironmentStringsW for self, the PEB
//! ProcessParameters.Environment pointer for other pids) — plus the
//! top-level `mounts` (one line per logical volume) and `net/dev`
//! (GetIfTable2 counters) faces. Fields with no Windows source read 0,
//! the module's standing rule: the SHAPE never changes, only a field's
//! fidelity.

/// Top-level synthetic file names (the /proc root's file face). The name
/// table, the content generator below, and the directory enumeration
/// ([`proc_dir_entries`]) are defined in this one module from this one
/// table, so a name can never be enumerable-but-unreadable or
/// readable-but-unenumerable (rubash#1917 A0: one table, three faces).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const TOP_LEVEL_FILES: [&str; 7] = [
    "cpuinfo", "loadavg", "meminfo", "mounts", "stat", "uptime", "version",
];

/// Top-level synthetic DIRECTORY names: the /proc root's subdirectories
/// whose children are again synthetic files (one level only — the
/// simulation has no deeper tree). Same one-table rule as the files.
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const TOP_LEVEL_DIRS: [&str; 1] = ["net"];

/// The synthetic files under `/proc/net` (the one top-level directory).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const NET_FILES: [&str; 1] = ["dev"];

/// Per-pid CONTENT entries (bytes generated on read).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const PID_CONTENT_FILES: [&str; 5] = ["cmdline", "environ", "stat", "statm", "status"];

/// Per-pid LINK entries (readlink-type: the entry resolves to a target
/// path elsewhere in the filesystem; see [`proc_link_target`]).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const PID_LINK_FILES: [&str; 2] = ["cwd", "exe"];

/// Every per-pid directory entry name, content and link faces together
/// (the `/proc/<pid>` and `/proc/self` enumeration face).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const PID_ENTRIES: [&str; 7] = [
    "cmdline", "cwd", "environ", "exe", "stat", "statm", "status",
];

/// The canonical entry name when `path` is exactly a top-level synthetic
/// file spelling (`/proc/<name>`, no further components), else None.
/// Pure table membership — the operand-materialization face
/// (rubash#1905) asks it on every platform, because the rewrite decision
/// must be an exact-spelling judgment: looser shapes the content
/// generator happens to answer (a trailing slash, a per-pid spelling)
/// are NOT operand-materializable, each for its own ruled reason
/// (see `executor::proc_operands`). The generation face stays
/// `proc_file_content`'s; this face only names the spelling, and the
/// caller's content fetch goes through the registry as usual, so a
/// table/generator drift would surface as a kept literal word (a loud
/// failure in the child), never as a wrong snapshot.
pub(crate) fn top_level_file_name(path: &str) -> Option<&'static str> {
    let name = path.strip_prefix("/proc/")?;
    if name.contains('/') {
        return None;
    }
    TOP_LEVEL_FILES
        .iter()
        .find(|entry| **entry == name)
        .copied()
}

// Count of ToolHelp process snapshots taken by this module on the
// calling thread, so the rubash#1917 §4.2 call-count budget (self
// face: zero snapshots; one enumeration pass: at most one) is
// assertable in tests instead of by timing. The budget is a
// per-command-execution property, and a command executes on one
// thread, so the counter is thread-local; only the snapshot SITES in
// this module are counted. (Plain comment: a doc comment cannot
// attach to a thread_local! invocation.)
#[cfg(windows)]
thread_local! {
    static TOOLHELP_SNAPSHOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(windows)]
pub(crate) fn toolhelp_snapshot_count() -> usize {
    TOOLHELP_SNAPSHOTS.with(|count| count.get())
}

#[cfg(windows)]
pub(crate) fn reset_toolhelp_snapshot_count() {
    TOOLHELP_SNAPSHOTS.with(|count| count.set(0));
}

#[cfg(not(windows))]
pub(crate) fn toolhelp_snapshot_count() -> usize {
    0
}

#[cfg(not(windows))]
pub(crate) fn reset_toolhelp_snapshot_count() {}

/// Take a ToolHelp process snapshot, counted (see TOOLHELP_SNAPSHOTS).
/// Returns the raw handle, or None when the snapshot could not be taken;
/// the caller owns the handle and must CloseHandle it.
#[cfg(windows)]
fn open_process_snapshot() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPPROCESS,
    };
    TOOLHELP_SNAPSHOTS.with(|count| count.set(count.get() + 1));
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    Some(snapshot)
}

/// Which pid a `/proc/<pid>/...` or `/proc/self/...` spelling names.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PidRef {
    SelfPid,
    Pid(u32),
}

/// Parse `/proc/<self|pid>/<tail>` into its pid reference and the tail
/// after it ("" for the directory itself). Shared by the link and
/// enumeration faces; the content face predates it and keeps its own
/// parse (its `/proc/self` bare-spelling asymmetry is load-bearing for
/// existing consumers — see the git history of proc_file_content).
#[cfg(windows)]
fn parse_pid_spelling(path: &str) -> Option<(PidRef, &str)> {
    let rest = path.strip_prefix("/proc/")?;
    if let Some(tail) = rest.strip_prefix("self") {
        let tail = if tail.is_empty() {
            ""
        } else {
            tail.strip_prefix('/')?
        };
        return Some((PidRef::SelfPid, tail));
    }
    let (head, tail) = match rest.split_once('/') {
        Some((h, t)) => (h, t),
        None => (rest, ""),
    };
    let pid: u32 = head.parse().ok()?;
    Some((PidRef::Pid(pid), tail))
}

/// Whether the pid a spelling names is a live process. The shell's own
/// pid is answered without a snapshot (rubash#1917 §4.2: the self face
/// never pays for the system-wide table).
#[cfg(windows)]
fn pid_ref_alive(pid_ref: PidRef) -> bool {
    match pid_ref {
        PidRef::SelfPid => true,
        PidRef::Pid(pid) => pid != 0 && snapshot_entry(pid).is_some(),
    }
}

/// Whether `path` names a synthetic LINK entry (`/proc/self/exe`,
/// `/proc/<pid>/cwd`, ...): the spelling is one of the link names AND
/// the named process is alive. Classification only — resolution is
/// [`proc_link_target`]. A dead pid's link entries do not exist, which
/// is the procfs directory-vanish rule applied to the link face.
#[cfg(windows)]
pub fn proc_is_link_path(path: &str) -> bool {
    let Some((pid_ref, tail)) = parse_pid_spelling(path) else {
        return false;
    };
    PID_LINK_FILES.contains(&tail) && pid_ref_alive(pid_ref)
}

#[cfg(not(windows))]
pub fn proc_is_link_path(_path: &str) -> bool {
    false
}

/// Directory entries for a synthetic /proc directory: the root (the
/// file table + `self` + every live pid, ascending) and per-pid
/// directories (the entry table, for live pids). None when `path` is
/// not a synthetic /proc directory — off Windows always None, where
/// the real procfs lists itself. This is the single enumeration source
/// the glob face and (later) the niu completion face ask (rubash#1917
/// A3: `list_pids` gains its first consumer here).
#[cfg(windows)]
pub fn proc_dir_entries(path: &str) -> Option<Vec<String>> {
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    if trimmed == "/proc" {
        let mut names: Vec<String> = TOP_LEVEL_FILES
            .iter()
            .map(|name| name.to_string())
            .collect();
        names.extend(TOP_LEVEL_DIRS.iter().map(|name| name.to_string()));
        names.push("self".to_string());
        names.extend(list_pids().into_iter().map(|pid| pid.to_string()));
        return Some(names);
    }
    if trimmed == "/proc/net" {
        return Some(NET_FILES.iter().map(|name| name.to_string()).collect());
    }
    let (pid_ref, tail) = parse_pid_spelling(trimmed)?;
    if !tail.is_empty() || !pid_ref_alive(pid_ref) {
        return None;
    }
    Some(PID_ENTRIES.iter().map(|name| name.to_string()).collect())
}

#[cfg(not(windows))]
pub fn proc_dir_entries(_path: &str) -> Option<Vec<String>> {
    None
}

/// Synthetic content for a /proc pseudo-file, or None when `path` is not one
/// of the emulated files. Each call generates a fresh snapshot.
///
/// This module owns *generation*; the consumer-facing entry is the
/// special-path registry ([`crate::special_paths::synthetic_proc_content`],
/// vfs audit §4.5 slice S1), which is where open points and probes ask.
#[cfg(windows)]
pub fn proc_file_content(path: &str) -> Option<Vec<u8>> {
    let rest = path.strip_prefix("/proc/")?;
    // "/proc/self/..." aliases the shell's own pid (Linux procfs convention).
    if let Some(self_tail) = rest.strip_prefix("self") {
        let tail = self_tail.strip_prefix('/')?;
        return proc_pid_file(std::process::id(), tail);
    }
    // The one subdirectory face: /proc/net/dev (niubash#758).
    if rest == "net/dev" {
        return Some(net_dev().into_bytes());
    }
    let (head, tail) = match rest.split_once('/') {
        Some((h, t)) => (h, t),
        None => (rest, ""),
    };
    if let Ok(pid) = head.parse::<u32>() {
        return proc_pid_file(pid, tail);
    }
    if !tail.is_empty() {
        return None;
    }
    let text = match head {
        "meminfo" => meminfo(),
        "cpuinfo" => cpuinfo(),
        "stat" => stat(),
        "loadavg" => loadavg(),
        "uptime" => uptime(),
        "version" => version(),
        "mounts" => mounts(),
        _ => return None,
    };
    Some(text.into_bytes())
}

/// `/proc/<pid>/{cmdline,status}` for a LIVE Windows process, plus the
/// `/proc/<pid>` directory probe itself (tail == "" merely checks liveness,
/// matching procfs where the directory vanishes when the process does).
#[cfg(windows)]
fn proc_pid_file(pid: u32, tail: &str) -> Option<Vec<u8>> {
    if pid == 0 {
        return None;
    }
    // rubash#1917 §4.2: the self face never pays for a system-wide
    // ToolHelp snapshot — every self entry is a direct self query.
    if pid == std::process::id() {
        return proc_self_file(tail);
    }
    // Unknown pid: the directory probe (tail == "") must report missing, and
    // so must any file under it -- procfs dirs vanish when the process does.
    let entry = snapshot_entry(pid)?;
    match tail {
        // NUL-separated argv, per procfs; the reader splits on '\0'.
        "cmdline" => Some(pid_cmdline(pid, &entry)),
        "status" => {
            let facts = pid_facts(pid);
            Some(
                pid_status(
                    pid,
                    &entry.image,
                    entry.parent_pid,
                    entry.threads,
                    facts.as_ref(),
                )
                .into_bytes(),
            )
        }
        // niubash#758: the procps faces. Facts degrade to zeros when
        // the process refuses the query — the line's shape is the
        // contract, never the file's existence.
        "stat" => {
            let facts = pid_facts(pid).unwrap_or_else(ProcessFacts::zeros);
            Some(pid_stat(pid, &entry.image, entry.parent_pid, entry.threads, &facts).into_bytes())
        }
        "statm" => {
            let facts = pid_facts(pid).unwrap_or_else(ProcessFacts::zeros);
            Some(pid_statm(&facts).into_bytes())
        }
        "environ" => Some(pid_environ(pid)),
        "" => Some(Vec::new()),
        _ => None,
    }
}

/// The /proc/self face: direct self queries only, zero ToolHelp
/// snapshots (rubash#1917 §4.2 budget, pinned by unit test).
#[cfg(windows)]
fn proc_self_file(tail: &str) -> Option<Vec<u8>> {
    match tail {
        "" => Some(Vec::new()),
        "status" => {
            let own = std::process::id();
            let facts = self_facts();
            // The thread count keeps the self face's standing value
            // when the image name answers directly: no snapshot-free
            // source for it exists, and the §4.2 budget forbids the
            // snapshot the true count would cost. The snapshot
            // fallback path (image name unavailable) has the true
            // count in hand and prints it.
            match self_image_name() {
                Some(image) => {
                    Some(pid_status(own, &image, own_parent_pid(), 1, facts.as_ref()).into_bytes())
                }
                None => snapshot_entry(own).map(|entry| {
                    pid_status(
                        own,
                        &entry.image,
                        entry.parent_pid,
                        entry.threads,
                        facts.as_ref(),
                    )
                    .into_bytes()
                }),
            }
        }
        // niubash#758: the self procps faces — direct self queries
        // only (GetCurrentProcess facts, the image name, the cached
        // parent), zero snapshots, per the §4.2 budget.
        "stat" => {
            let own = std::process::id();
            let facts = self_facts().unwrap_or_else(ProcessFacts::zeros);
            let image = self_image_name().unwrap_or_default();
            Some(pid_stat(own, &image, own_parent_pid(), 1, &facts).into_bytes())
        }
        "statm" => {
            let facts = self_facts().unwrap_or_else(ProcessFacts::zeros);
            Some(pid_statm(&facts).into_bytes())
        }
        "environ" => Some(self_environ()),
        "cmdline" => Some(self_cmdline()),
        _ => None,
    }
}

/// The shell process's own image (base) name, from the module path —
/// no snapshot needed.
#[cfg(windows)]
fn self_image_name() -> Option<String> {
    std::env::current_exe()
        .ok()?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// The shell process's own parent pid, asked once and cached: the
/// kernel records it at process creation, so it cannot change over
/// the process's life. The query is NtQueryInformationProcess on the
/// current process (the same technique the executor's $PPID binding
/// uses, support_names.rs) with the ToolHelp snapshot as fallback —
/// never a fabricated value (rubash#1918).
#[cfg(windows)]
fn own_parent_pid() -> u32 {
    static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        nt_own_parent_pid()
            .or_else(|| snapshot_entry(std::process::id()).map(|entry| entry.parent_pid))
            .unwrap_or(0)
    })
}

#[cfg(windows)]
fn nt_own_parent_pid() -> Option<u32> {
    use std::ffi::c_void;

    // Field layout mirrors the PROCESS_BASIC_INFORMATION the executor's
    // $PPID path already relies on (support_names.rs): the two-word
    // affinity slot also covers BasePriority, so UniqueProcessId and
    // InheritedFromUniqueProcessId land at their true offsets.
    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessBasicInformation {
        ExitStatus: *mut c_void,
        PebBaseAddress: *mut c_void,
        AffinityMask: [*mut c_void; 2],
        UniqueProcessId: usize,
        InheritedFromUniqueProcessId: usize,
    }

    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn NtQueryInformationProcess(
            process: *mut c_void,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    const PROCESS_BASIC_INFORMATION: u32 = 0;
    let mut info = unsafe { std::mem::zeroed::<ProcessBasicInformation>() };
    let mut returned = 0u32;
    let status = unsafe {
        NtQueryInformationProcess(
            GetCurrentProcess(),
            PROCESS_BASIC_INFORMATION,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            &mut returned,
        )
    };
    if status != 0 {
        return None;
    }
    u32::try_from(info.InheritedFromUniqueProcessId)
        .ok()
        .filter(|pid| *pid != 0)
}

/// The fallback cmdline shape: the image name, NUL-terminated (the
/// only argv the ToolHelp snapshot knows). Used wherever the true
/// command line cannot be read; the SHAPE never changes, only the
/// content's fidelity (rubash#1917 §3.1).
#[cfg(windows)]
fn image_cmdline_fallback(image: &str) -> Vec<u8> {
    let mut cmdline = image.as_bytes().to_vec();
    cmdline.push(0);
    cmdline
}

/// Another process's cmdline bytes: the TRUE argv, read from the
/// target's PEB (rubash#1917 §3.1 — the same technique WinuxCmd's
/// ps.cpp uses: NtQueryInformationProcess → PEB →
/// RTL_USER_PROCESS_PARAMETERS.CommandLine). The snapshot only knows
/// the image name, which is NOT argv; on any read failure (access,
/// bitness, the process exiting mid-read) the answer falls back to
/// the image-name shape — the shape never changes, only the content's
/// fidelity.
#[cfg(windows)]
fn pid_cmdline(pid: u32, entry: &SnapshotEntry) -> Vec<u8> {
    if let Some(command_line) = peb_strings(pid).and_then(|peb| peb.command_line) {
        let bytes = command_line_to_argv_bytes(&command_line);
        if !bytes.is_empty() {
            return bytes;
        }
    }
    image_cmdline_fallback(&entry.image)
}

/// The shell's own cmdline bytes: GetCommandLineW is the direct self
/// query (no snapshot, no PEB walk); the image-name shape is the
/// fallback if it somehow yields nothing.
#[cfg(windows)]
fn self_cmdline() -> Vec<u8> {
    if let Some(command_line) = own_command_line() {
        let bytes = command_line_to_argv_bytes(&command_line);
        if !bytes.is_empty() {
            return bytes;
        }
    }
    match self_image_name() {
        Some(image) => image_cmdline_fallback(&image),
        None => snapshot_entry(std::process::id())
            .map(|entry| image_cmdline_fallback(&entry.image))
            .unwrap_or_default(),
    }
}

/// The current process's raw command line (UTF-16 units, no NUL).
#[cfg(windows)]
fn own_command_line() -> Option<Vec<u16>> {
    extern "system" {
        fn GetCommandLineW() -> *const u16;
    }
    let ptr = unsafe { GetCommandLineW() };
    if ptr.is_null() {
        return None;
    }
    let mut units = Vec::new();
    // The string is NUL-terminated; the scan bound is a sanity rail
    // against a corrupt pointer, far past any real command line.
    for i in 0..(1 << 20) {
        let unit = unsafe { *ptr.add(i) };
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Some(units)
}

/// Render a raw UTF-16 environment block (entries NUL-separated, the
/// block closed by an empty entry) as procfs environ bytes: every
/// entry transcoded to UTF-8 and NUL-terminated, in block order.
/// Entries are never empty (each carries at least `NAME=`), so the
/// split's empty pieces are exactly the terminator.
#[cfg(windows)]
fn environ_block_to_bytes(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in units.split(|unit| *unit == 0) {
        if entry.is_empty() {
            continue;
        }
        bytes.extend_from_slice(String::from_utf16_lossy(entry).as_bytes());
        bytes.push(0);
    }
    bytes
}

/// The shell's own environ bytes: the process environment block
/// itself (GetEnvironmentStringsW — a direct self query, no snapshot,
/// per the §4.2 budget), transcoded to the procfs shape.
#[cfg(windows)]
fn self_environ() -> Vec<u8> {
    extern "system" {
        fn GetEnvironmentStringsW() -> *const u16;
        fn FreeEnvironmentStringsW(env: *const u16) -> i32;
    }
    let ptr = unsafe { GetEnvironmentStringsW() };
    if ptr.is_null() {
        return Vec::new();
    }
    // The block is double-NUL terminated; the scan bound is the same
    // sanity rail own_command_line uses.
    let mut units = Vec::new();
    let mut prev_zero = false;
    for i in 0..(1 << 20) {
        let unit = unsafe { *ptr.add(i) };
        if unit == 0 && prev_zero {
            break;
        }
        prev_zero = unit == 0;
        units.push(unit);
    }
    unsafe {
        FreeEnvironmentStringsW(ptr);
    }
    environ_block_to_bytes(&units)
}

/// Another process's environ bytes, from its PEB (the
/// ProcessParameters.Environment pointer). A protected or exited
/// process yields an EMPTY file, never a missing one: the pid is
/// live (the caller gated on the snapshot), so the entry exists —
/// only its content is beyond reach, which the empty read reports
/// the way a denied procfs environ read surfaces to a byte consumer
/// (niubash#758; the fidelity loss is recorded in the matrix).
#[cfg(windows)]
fn pid_environ(pid: u32) -> Vec<u8> {
    peb_strings(pid)
        .and_then(|peb| peb.environment)
        .unwrap_or_default()
}

/// The strings the PEB route can read for one process.
#[cfg(windows)]
struct PebStrings {
    /// RTL_USER_PROCESS_PARAMETERS.CommandLine, raw UTF-16 units.
    command_line: Option<Vec<u16>>,
    /// RTL_USER_PROCESS_PARAMETERS.CurrentDirectory.DosPath, rendered
    /// (the /proc/<pid>/cwd face consumes this in the #1900 slice).
    current_dir: Option<String>,
    /// RTL_USER_PROCESS_PARAMETERS.Environment, rendered to procfs
    /// environ bytes (the /proc/<pid>/environ face, niubash#758).
    environment: Option<Vec<u8>>,
}

/// The shell session's cwd in both spellings (rubash#1900): `native`
/// for handing to the OS, `display` for shell-namespace answers.
/// Built from the G-02 session cwd — the process-global cwd is never
/// consulted for /proc/self/cwd (the rubash#1887 anchoring rule).
pub struct ProcSelfCwd {
    pub native: String,
    pub display: String,
}

pub fn self_cwd_forms(session_cwd: &std::path::Path) -> ProcSelfCwd {
    let native = session_cwd.to_string_lossy().into_owned();
    let display = crate::executor::path::shell_pwd_display_path(&native);
    ProcSelfCwd { native, display }
}

/// A resolved synthetic-link target in both spellings.
pub struct ProcLinkTarget {
    pub native: String,
    pub display: String,
}

/// Resolve a SyntheticLink path to its target (rubash#1900).
/// `/proc/self/exe` is the running image; `/proc/<pid>/exe` is asked
/// of the OS per process (QueryFullProcessImageNameW — the image a
/// process was actually started from, not the snapshot's file NAME);
/// cwd comes from the session (self) or the target's PEB
/// CurrentDirectory.DosPath (other pids). A dead or unopenable pid
/// resolves to None — the link dangles exactly when its process does.
#[cfg(windows)]
pub fn proc_link_target(path: &str, self_cwd: Option<&ProcSelfCwd>) -> Option<ProcLinkTarget> {
    let (pid_ref, tail) = parse_pid_spelling(path)?;
    if !PID_LINK_FILES.contains(&tail) {
        return None;
    }
    match pid_ref {
        PidRef::SelfPid => match tail {
            "exe" => {
                let native = std::env::current_exe().ok()?.to_string_lossy().into_owned();
                let display = crate::executor::path::shell_pwd_display_path(&native);
                Some(ProcLinkTarget { native, display })
            }
            "cwd" => self_cwd.map(|cwd| ProcLinkTarget {
                native: cwd.native.clone(),
                display: cwd.display.clone(),
            }),
            _ => None,
        },
        PidRef::Pid(pid) => {
            // No liveness pre-walk: the OS queries below are the
            // liveness check — a dead or protected pid fails them and
            // the link dangles, which is the answer either way.
            match tail {
                "exe" => {
                    let native = process_image_path(pid)?;
                    let display = crate::executor::path::shell_pwd_display_path(&native);
                    Some(ProcLinkTarget { native, display })
                }
                "cwd" => {
                    let native = peb_strings(pid).and_then(|peb| peb.current_dir)?;
                    let display = crate::executor::path::shell_pwd_display_path(&native);
                    Some(ProcLinkTarget { native, display })
                }
                _ => None,
            }
        }
    }
}

#[cfg(not(windows))]
pub fn proc_link_target(_path: &str, _self_cwd: Option<&ProcSelfCwd>) -> Option<ProcLinkTarget> {
    None
}

/// A process's full image path, asked of the OS
/// (QueryFullProcessImageNameW). None for dead or protected pids.
#[cfg(windows)]
fn process_image_path(pid: u32) -> Option<String> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    extern "system" {
        fn QueryFullProcessImageNameW(
            process: *mut c_void,
            flags: u32,
            exe_name: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 || size == 0 {
        return None;
    }
    buffer.truncate(size as usize);
    Some(String::from_utf16_lossy(&buffer))
}

/// File-test facts for a synthetic /proc path (rubash#1900/#1902):
/// derived from the registry tables plus pid liveness — the test
/// face's previous implementation answered from a constant table that
/// never consulted either, which is why /proc/self/cmdline claimed
/// "not a file". `link_target_is_dir` states the link ENTRY's
/// definitional target kind (an exe image is a file; a cwd is a
/// directory), not a per-instance guess. `writable` / `executable`
/// are definitional in the same sense (rubash#1936): they state the
/// access the procfs entry's mode grants — the top-level generated
/// files are 0444 (read-only for every user, root included, because
/// the entries have no write handler), the per-pid content files are
/// 0644 owned by the process's user, the directories are 0555 (x for
/// traversal, no w), and a link's access is its target's (an exe
/// image is r-x at least and owned writable; a cwd directory is
/// rwx for its owner). GNU test.c:556/559 routes -w/-x through
/// sh_eaccess (lib/sh/eaccess.c:194), which is exactly this mode
/// question asked of the real entries on the oracle platform
/// (measured against GNU bash 5.3.20 on real procfs).
pub struct ProcUnaryFacts {
    pub is_dir: bool,
    pub is_file: bool,
    pub is_link: bool,
    pub link_target_is_dir: bool,
    pub writable: bool,
    pub executable: bool,
}

#[cfg(windows)]
pub fn proc_unary_facts(path: &str) -> Option<ProcUnaryFacts> {
    let dir_facts = ProcUnaryFacts {
        is_dir: true,
        is_file: false,
        is_link: false,
        link_target_is_dir: false,
        writable: false,
        executable: true,
    };
    let file_facts = ProcUnaryFacts {
        is_dir: false,
        is_file: true,
        is_link: false,
        link_target_is_dir: false,
        writable: false,
        executable: false,
    };
    let trimmed = path.trim_end_matches('/');
    if trimmed == "/proc" {
        return Some(dir_facts);
    }
    if trimmed == "/proc/net" {
        return Some(dir_facts);
    }
    if trimmed == "/proc/net/dev" {
        return Some(file_facts);
    }
    if let Some((pid_ref, tail)) = parse_pid_spelling(trimmed) {
        if !pid_ref_alive(pid_ref) {
            return None;
        }
        if tail.is_empty() {
            return Some(dir_facts);
        }
        if tail == "fd" {
            // niubash#758: the fd DIRECTORY exists for every live
            // pid (procfs mode dr-x------: traversal yes, writes no).
            // Only its EXISTENCE is answered here — listing its
            // entries names another process's descriptor table,
            // which this layer cannot enumerate; the self spellings'
            // children are served by the fd-alias faces and the
            // host's fd-snapshot completion instead.
            return Some(dir_facts);
        }
        if PID_CONTENT_FILES.contains(&tail) {
            return Some(ProcUnaryFacts {
                writable: true,
                ..file_facts
            });
        }
        if PID_LINK_FILES.contains(&tail) {
            return Some(ProcUnaryFacts {
                is_dir: false,
                is_file: false,
                is_link: true,
                link_target_is_dir: tail == "cwd",
                writable: true,
                executable: true,
            });
        }
        return None;
    }
    // Top-level content files (the pid spelling parse rejects their
    // non-numeric head, which is what routes them here).
    let leaf = trimmed.strip_prefix("/proc/")?;
    if !leaf.contains('/') && TOP_LEVEL_FILES.contains(&leaf) {
        return Some(file_facts);
    }
    None
}

#[cfg(not(windows))]
pub fn proc_unary_facts(_path: &str) -> Option<ProcUnaryFacts> {
    None
}

/// The write class of a synthetic /proc CONTENT entry (rubash#1935):
/// `Some(true)` when a write-direction open succeeds on the real
/// procfs entry, `Some(false)` when it fails at open, `None` when the
/// path is not synthetic content at all. The classes are the entries'
/// procfs modes: the top-level generated files are 0444 and have no
/// write handler, so even root's O_RDWR/O_WRONLY open fails EACCES;
/// the per-pid content files (status/cmdline) are 0644, so the open
/// succeeds and only the WRITE fails (EINVAL). Measured against GNU
/// bash 5.3.20 on real procfs (`exec 3<>/proc/version` → Permission
/// denied; `exec 3<>/proc/self/status` opens, reads serve, writes
/// fail EINVAL). Off Windows the generator is a stub and this answers
/// None — the real procfs serves the path there.
#[cfg(windows)]
pub fn proc_content_write_allowed(path: &str) -> Option<bool> {
    let trimmed = path.trim_end_matches('/');
    if trimmed == "/proc/net/dev" {
        // The 0444 class, like the top-level generated files.
        return Some(false);
    }
    if let Some((pid_ref, tail)) = parse_pid_spelling(trimmed) {
        if !pid_ref_alive(pid_ref) {
            return None;
        }
        if PID_CONTENT_FILES.contains(&tail) {
            return Some(true);
        }
        return None;
    }
    let leaf = trimmed.strip_prefix("/proc/")?;
    if !leaf.contains('/') && TOP_LEVEL_FILES.contains(&leaf) {
        return Some(false);
    }
    None
}

#[cfg(not(windows))]
pub fn proc_content_write_allowed(_path: &str) -> Option<bool> {
    None
}

// PEB / RTL_USER_PROCESS_PARAMETERS field offsets for the two strings
// this module reads. The layouts are pointer-width dependent; a
// cross-bitness read (64-bit shell, 32-bit target) addresses the
// wrong fields, fails the sanity checks below, and falls back — the
// design's accepted coverage edge (rubash#1917 §7.1).
#[cfg(windows)]
#[cfg(target_pointer_width = "64")]
const PEB_PARAMS_OFFSET: usize = 0x20;
#[cfg(windows)]
#[cfg(target_pointer_width = "32")]
const PEB_PARAMS_OFFSET: usize = 0x10;
#[cfg(windows)]
#[cfg(target_pointer_width = "64")]
const PARAMS_CURDIR_OFFSET: usize = 0x38;
#[cfg(windows)]
#[cfg(target_pointer_width = "32")]
const PARAMS_CURDIR_OFFSET: usize = 0x24;
#[cfg(windows)]
#[cfg(target_pointer_width = "64")]
const PARAMS_CMDLINE_OFFSET: usize = 0x70;
#[cfg(windows)]
#[cfg(target_pointer_width = "32")]
const PARAMS_CMDLINE_OFFSET: usize = 0x40;
// RTL_USER_PROCESS_PARAMETERS.Environment (a bare PVOID, not a
// UNICODE_STRING) directly follows the CommandLine string header.
#[cfg(windows)]
#[cfg(target_pointer_width = "64")]
const PARAMS_ENV_OFFSET: usize = 0x80;
#[cfg(windows)]
#[cfg(target_pointer_width = "32")]
const PARAMS_ENV_OFFSET: usize = 0x48;

/// Read a process's PEB strings. None when the process cannot be
/// opened for query+VM-read (protected, wrong bitness plumbing, or
/// already gone); individual strings are None when their field fails
/// its sanity checks.
#[cfg(windows)]
fn peb_strings(pid: u32) -> Option<PebStrings> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    extern "system" {
        fn NtQueryInformationProcess(
            process: *mut c_void,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
        fn ReadProcessMemory(
            process: *mut c_void,
            base_address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            bytes_read: *mut usize,
        ) -> i32;
    }

    // Same PROCESS_BASIC_INFORMATION layout as nt_own_parent_pid.
    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessBasicInformation {
        ExitStatus: *mut c_void,
        PebBaseAddress: *mut c_void,
        AffinityMask: [*mut c_void; 2],
        UniqueProcessId: usize,
        InheritedFromUniqueProcessId: usize,
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let result = (|| {
        let mut info = unsafe { std::mem::zeroed::<ProcessBasicInformation>() };
        let mut returned = 0u32;
        let status = unsafe {
            NtQueryInformationProcess(
                handle,
                0,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ProcessBasicInformation>() as u32,
                &mut returned,
            )
        };
        if status != 0 || info.PebBaseAddress.is_null() {
            return None;
        }
        let read = |address: usize, buffer: &mut [u8]| -> bool {
            let mut got = 0usize;
            unsafe {
                ReadProcessMemory(
                    handle,
                    address as *const c_void,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len(),
                    &mut got,
                ) != 0
                    && got == buffer.len()
            }
        };
        // PEB.ProcessParameters pointer.
        let mut params_ptr = [0u8; std::mem::size_of::<usize>()];
        if !read(
            info.PebBaseAddress as usize + PEB_PARAMS_OFFSET,
            &mut params_ptr,
        ) {
            return None;
        }
        let params_addr = usize::from_ne_bytes(params_ptr);
        if params_addr == 0 {
            return None;
        }
        // The parameter block's first 0x100 bytes cover both strings'
        // UNICODE_STRING headers on either width.
        let mut params = [0u8; 0x100];
        if !read(params_addr, &mut params) {
            return None;
        }
        let unicode_string = |offset: usize| -> Option<Vec<u16>> {
            let length = u16::from_ne_bytes(params[offset..offset + 2].try_into().ok()?) as usize;
            let buffer_offset = offset
                + if cfg!(target_pointer_width = "64") {
                    8
                } else {
                    4
                };
            let buffer_addr = usize::from_ne_bytes(
                params[buffer_offset..buffer_offset + std::mem::size_of::<usize>()]
                    .try_into()
                    .ok()?,
            );
            // Sanity rails: a genuine command line / DosPath is a
            // non-empty, even-length UTF-16 string of bounded size;
            // anything else means the offsets addressed the wrong
            // layout (cross-bitness) and the read is void.
            if length == 0 || length % 2 != 0 || length > 32766 || buffer_addr == 0 {
                return None;
            }
            let mut raw = vec![0u8; length];
            if !read(buffer_addr, &mut raw) {
                return None;
            }
            Some(
                raw.chunks_exact(2)
                    .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
                    .collect(),
            )
        };
        // Environment: a bare pointer to the double-NUL-terminated
        // UTF-16 block. Read in chunks until the terminator shows;
        // the unit cap is a sanity rail far past any real block (a
        // corrupt/cross-bitness read must end in the fallback, not a
        // runaway scan).
        let mut env_ptr_bytes = [0u8; std::mem::size_of::<usize>()];
        let environment = if !read(params_addr + PARAMS_ENV_OFFSET, &mut env_ptr_bytes) {
            None
        } else {
            let env_addr = usize::from_ne_bytes(env_ptr_bytes);
            if env_addr == 0 {
                None
            } else {
                let mut units: Vec<u16> = Vec::new();
                while units.len() < (1 << 18) {
                    let mut chunk = [0u8; 8192];
                    if !read(env_addr + units.len() * 2, &mut chunk) {
                        break;
                    }
                    units.extend(
                        chunk
                            .chunks_exact(2)
                            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]])),
                    );
                    if let Some(end) = units.windows(2).position(|pair| pair == [0, 0]) {
                        units.truncate(end);
                        break;
                    }
                }
                let bytes = environ_block_to_bytes(&units);
                if bytes.is_empty() {
                    None
                } else {
                    Some(bytes)
                }
            }
        };
        Some(PebStrings {
            command_line: unicode_string(PARAMS_CMDLINE_OFFSET),
            current_dir: unicode_string(PARAMS_CURDIR_OFFSET)
                .map(|units| String::from_utf16_lossy(&units)),
            environment,
        })
    })();
    unsafe {
        CloseHandle(handle);
    }
    result
}

/// Derive procfs cmdline bytes from a raw Windows command line: split
/// with the OS's own CommandLineToArgvW rules (the derivation every
/// Windows process's argv undergoes), then emit each argument as
/// UTF-8, NUL-terminated — the procfs byte shape. An empty derivation
/// yields an empty Vec and the caller falls back.
#[cfg(windows)]
fn command_line_to_argv_bytes(command_line: &[u16]) -> Vec<u8> {
    use std::ffi::c_void;

    // CommandLineToArgvW lives in shell32, outside the default link
    // set — the #[link] attribute follows the tree's kernel32
    // precedent (src/builtins/stty.rs).
    #[link(name = "shell32")]
    extern "system" {
        fn CommandLineToArgvW(cmd_line: *const u16, num_args: *mut i32) -> *mut *mut u16;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    if command_line.is_empty() {
        return Vec::new();
    }
    // CommandLineToArgvW needs a NUL-terminated string.
    let mut terminated = command_line.to_vec();
    terminated.push(0);
    let mut argc = 0i32;
    let argv = unsafe { CommandLineToArgvW(terminated.as_ptr(), &mut argc) };
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    unsafe {
        for i in 0..argc as isize {
            let arg_ptr = *argv.offset(i);
            if arg_ptr.is_null() {
                continue;
            }
            let mut len = 0usize;
            while *arg_ptr.add(len) != 0 {
                len += 1;
            }
            let arg = String::from_utf16_lossy(std::slice::from_raw_parts(arg_ptr, len));
            bytes.extend_from_slice(arg.as_bytes());
            bytes.push(0);
        }
        LocalFree(argv as *mut c_void);
    }
    bytes
}

/// One process's ToolHelp snapshot record: image name, the REAL
/// parent pid the kernel recorded at its creation
/// (PROCESSENTRY32W.th32ParentProcessID — rubash#1918: this field was
/// previously read by the walk and thrown away, while pid_status
/// fabricated a PPid), and the thread count
/// (PROCESSENTRY32W.cntThreads — niubash#758: the status/stat faces
/// print the real count instead of a fabricated 1 for other pids).
#[cfg(windows)]
struct SnapshotEntry {
    image: String,
    parent_pid: u32,
    threads: u32,
}

/// ToolHelp snapshot walk for one pid, or None when the pid does not
/// exist.
#[cfg(windows)]
fn snapshot_entry(pid: u32) -> Option<SnapshotEntry> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        Process32FirstW, Process32NextW, PROCESSENTRY32W,
    };
    let snapshot = open_process_snapshot()?;

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut found = None;
    unsafe {
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == pid {
                    let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(0);
                    found = Some(SnapshotEntry {
                        image: String::from_utf16_lossy(&entry.szExeFile[..len]),
                        parent_pid: entry.th32ParentProcessID,
                        threads: entry.cntThreads,
                    });
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    found
}

/// The per-process facts the stat/statm/status faces render, asked of
/// the OS per process (niubash#758). Every field degrades to 0 when its
/// query fails (a protected process, a handle the OS refuses): the
/// FACES keep their shape and only the value's fidelity drops — the
/// cmdline fallback's rule (rubash#1917 §3.1) applied to numbers.
#[cfg(windows)]
struct ProcessFacts {
    /// GetProcessTimes kernel+user time, in USER_HZ (100/s) ticks —
    /// the /proc/<pid>/stat utime/stime fields' unit.
    utime_ticks: u64,
    stime_ticks: u64,
    /// Process creation time in the same ticks, measured from system
    /// start — the stat starttime field's unit and epoch.
    start_ticks: u64,
    /// VM_COUNTERS sizes, in bytes.
    vsize_bytes: u64,
    rss_bytes: u64,
    pagefile_bytes: u64,
}

#[cfg(windows)]
impl ProcessFacts {
    fn zeros() -> Self {
        ProcessFacts {
            utime_ticks: 0,
            stime_ticks: 0,
            start_ticks: 0,
            vsize_bytes: 0,
            rss_bytes: 0,
            pagefile_bytes: 0,
        }
    }
}

/// System start as a FILETIME-scale value (100 ns since 1601):
/// now minus the tick-count uptime. The stat starttime derivation's
/// epoch anchor.
#[cfg(windows)]
fn system_start_100ns() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    let now_100ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_nanos() / 100) as u64)
        .unwrap_or(0);
    // FILETIME's 1601 epoch and UNIX_EPOCH differ by a constant; the
    // subtraction below only ever differences two values on the SAME
    // scale (a process creation FILETIME and this anchor), so the
    // constant cancels and the anchor just needs the same scale.
    const EPOCH_DELTA_100NS: u64 = 116_444_736_000_000_000;
    now_100ns
        .saturating_add(EPOCH_DELTA_100NS)
        .saturating_sub(unsafe { GetTickCount64() } * 10_000)
}

/// Ask the OS for one process's facts. `handle` is an already-open
/// process handle (query rights); the VM counters go through
/// NtQueryInformationProcess class 3 (PROCESS_VM_COUNTERS, the
/// documented-in-practice layout psapi's GetProcessMemoryInfo wraps),
/// following this module's existing NtQueryInformationProcess extern
/// precedent. None only when the TIMES query fails outright; the VM
/// counters degrade independently to zeros.
#[cfg(windows)]
fn facts_for_handle(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<ProcessFacts> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    extern "system" {
        fn NtQueryInformationProcess(
            process: *mut c_void,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    #[repr(C)]
    #[derive(Default)]
    struct VmCounters {
        peak_virtual_size: usize,
        virtual_size: usize,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    const PROCESS_VM_COUNTERS: u32 = 3;
    let zero_ft = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut creation, mut exit, mut kernel, mut user) = (zero_ft, zero_ft, zero_ft, zero_ft);
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == 0 {
        return None;
    }
    let ft = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    // 100 ns units → USER_HZ ticks (10 ms) is a division by 100_000.
    let mut facts = ProcessFacts {
        utime_ticks: ft(user) / 100_000,
        stime_ticks: ft(kernel) / 100_000,
        start_ticks: ft(creation).saturating_sub(system_start_100ns()) / 100_000,
        ..ProcessFacts::zeros()
    };
    let mut vm = VmCounters::default();
    let mut returned = 0u32;
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            PROCESS_VM_COUNTERS,
            &mut vm as *mut _ as *mut c_void,
            std::mem::size_of::<VmCounters>() as u32,
            &mut returned,
        )
    };
    if status == 0 {
        facts.vsize_bytes = vm.virtual_size as u64;
        facts.rss_bytes = vm.working_set_size as u64;
        facts.pagefile_bytes = vm.pagefile_usage as u64;
    }
    Some(facts)
}

/// Facts for another process: open it with the least rights the
/// queries accept, ask, close. None when the process cannot be opened
/// at all (protected) or has already exited.
#[cfg(windows)]
fn pid_facts(pid: u32) -> Option<ProcessFacts> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let facts = facts_for_handle(handle);
    unsafe {
        CloseHandle(handle);
    }
    facts
}

/// Facts for the shell process itself: the current-process
/// pseudo-handle needs no OpenProcess and no snapshot (the §4.2 self
/// budget covers every self face).
#[cfg(windows)]
fn self_facts() -> Option<ProcessFacts> {
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    facts_for_handle(unsafe { GetCurrentProcess() })
}

/// Linux /proc/<pid>/status field set (name/state/pid/ppid/threads...).
/// The PPid is the snapshot's recorded creator pid, passed in by the
/// caller — this function invents nothing (rubash#1918: the previous
/// body answered 0 for self and the shell's own pid for everyone else,
/// while the true value sat unused in the same snapshot record).
/// The thread count is the snapshot's recorded count (niubash#758);
/// the VmSize/VmRSS lines appear exactly when the caller's facts
/// query answered, at their true values.
#[cfg(windows)]
fn pid_status(
    pid: u32,
    image: &str,
    ppid: u32,
    threads: u32,
    facts: Option<&ProcessFacts>,
) -> String {
    let mut out = format!(
        "Name:\t{image}\n\
         State:\tS (sleeping)\n\
         Tgid:\t{pid}\n\
         Pid:\t{pid}\n\
         PPid:\t{ppid}\n"
    );
    if let Some(facts) = facts {
        out.push_str(&format!(
            "VmSize:\t{} kB\nVmRSS:\t{} kB\n",
            facts.vsize_bytes / 1024,
            facts.rss_bytes / 1024
        ));
    }
    out.push_str(&format!("Threads:\t{threads}\n"));
    out
}

/// The Linux /proc/<pid>/stat line: 52 space-separated fields, comm
/// in parentheses (field 2). procps parses this file positionally and
/// splits comm on the LAST ')', so the image name is sanitized of
/// parentheses (and truncated to the kernel's 15-byte TASK_COMM_LEN
/// budget) before it is embedded. Fields with no Windows source read
/// 0 — the shape is the contract (niubash#758).
#[cfg(windows)]
fn pid_stat(pid: u32, image: &str, ppid: u32, threads: u32, facts: &ProcessFacts) -> String {
    let mut comm: String = image
        .chars()
        .filter(|c| *c != '(' && *c != ')')
        .take(15)
        .collect();
    if comm.is_empty() {
        comm.push_str("?");
    }
    const PAGE: u64 = 4096;
    let mut fields: Vec<String> = vec![
        pid.to_string(),                      // 1 pid
        format!("({comm})"),                  // 2 comm
        "S".to_string(),                      // 3 state
        ppid.to_string(),                     // 4 ppid
        "0".to_string(),                      // 5 pgrp (no Windows source)
        "0".to_string(),                      // 6 session (no Windows source)
        "0".to_string(),                      // 7 tty_nr
        "0".to_string(),                      // 8 tpgid
        "0".to_string(),                      // 9 flags
        "0".to_string(),                      // 10 minflt
        "0".to_string(),                      // 11 cminflt
        "0".to_string(),                      // 12 majflt
        "0".to_string(),                      // 13 cmajflt
        facts.utime_ticks.to_string(),        // 14 utime
        facts.stime_ticks.to_string(),        // 15 stime
        "0".to_string(),                      // 16 cutime
        "0".to_string(),                      // 17 cstime
        "20".to_string(),                     // 18 priority (the normal class)
        "0".to_string(),                      // 19 nice
        threads.to_string(),                  // 20 num_threads
        "0".to_string(),                      // 21 itrealvalue
        facts.start_ticks.to_string(),        // 22 starttime
        facts.vsize_bytes.to_string(),        // 23 vsize
        (facts.rss_bytes / PAGE).to_string(), // 24 rss (pages)
        "18446744073709551615".to_string(),   // 25 rsslim (the kernel's unlimited)
    ];
    // 26..=52: address/exit/signal bookkeeping with no Windows source.
    fields.extend((26..=52).map(|_| "0".to_string()));
    debug_assert_eq!(fields.len(), 52);
    let mut line = fields.join(" ");
    line.push('\n');
    line
}

/// The Linux /proc/<pid>/statm line: seven page counts (size,
/// resident, shared, text, lib, data, dt). Shared/text/lib have no
/// cheap Windows source and read 0; data is the private pagefile
/// commitment (niubash#758).
#[cfg(windows)]
fn pid_statm(facts: &ProcessFacts) -> String {
    const PAGE: u64 = 4096;
    format!(
        "{} {} 0 0 0 {} 0\n",
        facts.vsize_bytes / PAGE,
        facts.rss_bytes / PAGE,
        facts.pagefile_bytes / PAGE
    )
}

#[cfg(not(windows))]
pub fn proc_file_content(_path: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(not(windows))]
fn cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(windows)]
fn cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(windows)]
struct MemSnapshot {
    total_phys_kb: u64,
    free_phys_kb: u64,
    swap_total_kb: u64,
    swap_free_kb: u64,
}

#[cfg(windows)]
fn mem_snapshot() -> MemSnapshot {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    // The global status query does not fail in practice; on failure the
    // zeroed struct yields 0 kB fields rather than a panic mid-redirection.
    unsafe {
        GlobalMemoryStatusEx(&mut status);
    }
    MemSnapshot {
        total_phys_kb: status.ullTotalPhys / 1024,
        free_phys_kb: status.ullAvailPhys / 1024,
        swap_total_kb: status.ullTotalPageFile.saturating_sub(status.ullTotalPhys) / 1024,
        swap_free_kb: status.ullAvailPageFile.saturating_sub(status.ullAvailPhys) / 1024,
    }
}

/// The Linux meminfo line set. Windows cannot observe page-cache/Slab-style
/// accounting, so derived-from-nothing fields read 0 -- the field NAMES and
/// order match Linux so downstream `grep '^Field:'` consumers find them.
#[cfg(windows)]
fn meminfo() -> String {
    let m = mem_snapshot();
    let cached = 0u64;
    let avail = m.free_phys_kb.max(cached);
    format!(
        "MemTotal:       {total} kB\n\
         MemFree:        {free} kB\n\
         MemAvailable:   {avail} kB\n\
         Buffers:        0 kB\n\
         Cached:         {cached} kB\n\
         SwapCached:     0 kB\n\
         Active:         0 kB\n\
         Inactive:       0 kB\n\
         Active(anon):   0 kB\n\
         Inactive(anon): 0 kB\n\
         Active(file):   0 kB\n\
         Inactive(file): 0 kB\n\
         Unevictable:    0 kB\n\
         Mlocked:        0 kB\n\
         SwapTotal:      {swap_total} kB\n\
         SwapFree:       {swap_free} kB\n\
         Dirty:          0 kB\n\
         Writeback:      0 kB\n\
         AnonPages:      0 kB\n\
         Mapped:         0 kB\n\
         Shmem:          0 kB\n\
         KReclaimable:   0 kB\n\
         Slab:           0 kB\n\
         SReclaimable:   0 kB\n\
         SUnreclaim:     0 kB\n\
         KernelStack:    0 kB\n\
         PageTables:     0 kB\n\
         NFS_Unstable:   0 kB\n\
         Bounce:         0 kB\n\
         WritebackTmp:   0 kB\n\
         CommitLimit:    {commit_limit} kB\n\
         Committed_AS:   {committed} kB\n\
         VmallocTotal:   0 kB\n\
         VmallocUsed:    0 kB\n\
         VmallocChunk:   0 kB\n\
         Percpu:         0 kB\n\
         AnonHugePages:  0 kB\n\
         ShmemHugePages: 0 kB\n\
         ShmemPmdMapped: 0 kB\n\
         FileHugePages:  0 kB\n\
         FilePmdMapped:  0 kB\n\
         HugePages_Total:       0\n\
         HugePages_Free:        0\n\
         HugePages_Rsvd:        0\n\
         HugePages_Surp:        0\n\
         Hugetlb:        0 kB\n\
         Hugepagesize:       2048 kB\n\
         DirectMap4k:    0 kB\n\
         DirectMap2M:    0 kB\n\
         DirectMap1G:    0 kB\n",
        total = m.total_phys_kb,
        free = m.free_phys_kb,
        swap_total = m.swap_total_kb,
        swap_free = m.swap_free_kb,
        commit_limit = (m.total_phys_kb + m.swap_total_kb) / 2,
        committed = m.total_phys_kb.saturating_sub(m.free_phys_kb),
    )
}

/// USER_HZ tick counts (user, nice, system, idle, iowait, irq, softirq,
/// steal) for /proc/stat cpu lines. Linux uses USER_HZ=100; Win32 times are
/// 100ns units. Windows has no per-CPU GetSystemTimes, so per-CPU lines
/// repeat the aggregate split evenly (GNU consumers read totals anyway).
#[cfg(windows)]
fn cpu_times() -> [u64; 8] {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut idle = zero;
    let mut kernel = zero;
    let mut user = zero;
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return [0; 8];
    }
    let ticks =
        |t: FILETIME| (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) / 100_000;
    let (idle_t, user_t, kern_t) = (ticks(idle), ticks(user), ticks(kernel));
    let sys = kern_t.saturating_sub(idle_t);
    [user_t, 0, sys, idle_t, 0, 0, 0, 0]
}

#[cfg(windows)]
fn uptime_secs() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    (unsafe { GetTickCount64() }) / 1000
}

#[cfg(windows)]
fn stat() -> String {
    let t = cpu_times();
    let line = |name: &str| {
        format!(
            "{name} {} {} {} {} {} {} {} {} 0 0\n",
            t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7]
        )
    };
    let mut out = line("cpu ");
    for i in 0..cpu_count() {
        out.push_str(&line(&format!("cpu{i}")));
    }
    let btime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(uptime_secs()))
        .unwrap_or(0);
    out.push_str(&format!(
        "intr 0\nctxt 0\nbtime {btime}\nprocesses 0\nprocs_running 1\nprocs_blocked 0\n\
         softirq 0 0 0 0 0 0 0 0 0 0 0\n"
    ));
    out
}

#[cfg(windows)]
fn uptime() -> String {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    let ms = unsafe { GetTickCount64() };
    // GNU uptime prints "<up> <idle>" as fractional seconds; Windows idle
    // time is not exposed without PDH, so both share the uptime value.
    format!("{:.2} {:.2}\n", ms as f64 / 1000.0, ms as f64 / 1000.0)
}

#[cfg(windows)]
fn loadavg() -> String {
    // No per-run-queue counter without PDH; report the shell itself as the
    // one runnable task so the five-field shape consumers expect holds.
    format!("0.00 0.00 0.00 1/1 {}\n", std::process::id())
}

#[cfg(windows)]
fn version() -> String {
    "Linux version 6.1-rubash-proc (rubash minimal /proc emulation) (windows) #1 SMP\n".to_string()
}

/// /proc/mounts (niubash#758): one line per logical volume, in the
/// procfs six-field shape — device spelling, mountpoint in the
/// shell namespace (the drive-root spelling the path mapper serves),
/// the volume's real filesystem name (GetVolumeInformationW), and
/// rw/ro from the volume's read-only flag. The payload root's own
/// line is deliberately absent: the root's native path is session
/// state the generator does not hold (the same caller-fed channel
/// the /proc/<pid>/root link waits on), and a mounts line with a
/// fabricated source would be worse than its absence — volume
/// consumers key on the drive mountpoints, which are exact.
#[cfg(windows)]
fn mounts() -> String {
    use windows_sys::Win32::Storage::FileSystem::{GetLogicalDriveStringsW, GetVolumeInformationW};
    // FILE_READ_ONLY_VOLUME (Win32 file-system flag).
    const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;
    let mut buffer = vec![0u16; 1024];
    let written = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) };
    if written == 0 {
        return String::new();
    }
    buffer.truncate(written as usize);
    let mut out = String::new();
    for root in buffer.split(|unit| *unit == 0) {
        if root.is_empty() {
            continue;
        }
        let root_str = String::from_utf16_lossy(root);
        let Some(drive) = root_str.chars().next() else {
            continue;
        };
        let mut root_z = root.to_vec();
        root_z.push(0);
        let mut fs_name = vec![0u16; 64];
        let mut flags = 0u32;
        let ok = unsafe {
            GetVolumeInformationW(
                root_z.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                fs_name.as_mut_ptr(),
                fs_name.len() as u32,
            )
        };
        let fstype = if ok != 0 {
            let end = fs_name
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(fs_name.len());
            let raw = String::from_utf16_lossy(&fs_name[..end]).to_lowercase();
            if raw == "fat32" {
                "vfat".to_string()
            } else if raw.is_empty() {
                "unknown".to_string()
            } else {
                raw
            }
        } else {
            "unknown".to_string()
        };
        let opts = if ok != 0 && flags & FILE_READ_ONLY_VOLUME != 0 {
            "ro"
        } else {
            "rw"
        };
        let lower = drive.to_ascii_lowercase();
        let upper = drive.to_ascii_uppercase();
        out.push_str(&format!("{upper}: /{lower} {fstype} {opts} 0 0\n"));
    }
    out
}

/// One interface's display name for /proc/net/dev: the friendly
/// alias (what Windows users recognize), sanitized to the
/// whitespace-free shape net/dev consumers split on.
#[cfg(windows)]
fn iface_display_name(
    row: &windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2,
) -> String {
    let take = |units: &[u16]| -> String {
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
            .chars()
            .map(|c| {
                if c.is_whitespace() || c == ':' {
                    '_'
                } else {
                    c
                }
            })
            .collect()
    };
    let alias = take(&row.Alias);
    if !alias.is_empty() {
        return alias;
    }
    let description = take(&row.Description);
    if !description.is_empty() {
        return description;
    }
    format!("if{}", row.InterfaceIndex)
}

/// /proc/net/dev (niubash#758): the two Linux header lines, then one
/// line per interface with GetIfTable2's real counters in the
/// 16-column receive/transmit layout. Packet counts fold unicast +
/// non-unicast (the row's only total); the fifo/frame/compressed
/// columns have no Windows source and read 0.
#[cfg(windows)]
fn net_dev() -> String {
    use std::ffi::c_void;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    if unsafe { GetIfTable2(&mut table) } != 0 || table.is_null() {
        return String::new();
    }
    let count = unsafe { (*table).NumEntries } as usize;
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), count) };
    let mut out = String::from(
        "Inter-|   Receive                                                |  Transmit\n\
         face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n",
    );
    for row in rows {
        let name = iface_display_name(row);
        out.push_str(&format!(
            "{name}: {} {} {} {} 0 0 0 {} {} {} {} {} 0 0 0 0\n",
            row.InOctets,
            row.InUcastPkts + row.InNUcastPkts,
            row.InErrors,
            row.InDiscards,
            row.InNUcastPkts,
            row.OutOctets,
            row.OutUcastPkts + row.OutNUcastPkts,
            row.OutErrors,
            row.OutDiscards,
        ));
    }
    unsafe {
        FreeMibTable(table as *const c_void);
    }
    out
}

/// Standard per-CPU block. Win32 exposes CPUID, so vendor/family/flags come
/// from the real CPU rather than placeholders where cheap to fetch.
#[cfg(windows)]
fn cpuinfo() -> String {
    let cores = cpu_count();
    let (vendor, family, model, stepping, flags, mhz) = cpuid_identity();
    let mut out = String::new();
    for i in 0..cores {
        out.push_str(&format!(
            "processor\t: {i}\n\
             vendor_id\t: {vendor}\n\
             cpu family\t: {family}\n\
             model\t\t: {model}\n\
             model name\t: {mhz_name}\n\
             stepping\t: {stepping}\n\
             microcode\t: 0xffffffff\n\
             cpu MHz\t\t: {mhz}\n\
             cache size\t: 0 KB\n\
             physical id\t: 0\n\
             siblings\t: {cores}\n\
             core id\t\t: 0\n\
             cpu cores\t: {cores}\n\
             apicid\t\t: {i}\n\
             initial apicid\t: {i}\n\
             fpu\t\t: yes\n\
             fpu_exception\t: yes\n\
             cpuid level\t: 5\n\
             wp\t\t: yes\n\
             flags\t\t: {flags}\n\
             bugs\t\t:\n\
             bogomips\t: {bogo}\n\
             clflush size\t: 64\n\
             cache_alignment\t: 64\n\
             address sizes\t: 39 bits physical, 48 bits virtual\n\
             power management:\n\n",
            mhz_name = "Windows Virtual CPU",
            vendor = vendor,
            family = family,
            model = model,
            stepping = stepping,
            flags = flags,
            mhz = mhz,
            bogo = mhz * 2,
            cores = cores,
        ));
    }
    out
}

/// Real CPUID identity via __cpuid. Falls back to placeholders if the
/// intrinsic is unavailable.
#[cfg(windows)]
#[allow(unused_unsafe)] // __cpuid became a safe intrinsic on newer toolchains
fn cpuid_identity() -> (&'static str, u32, u32, u32, String, u64) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        unsafe {
            let max = __cpuid(0).eax;
            if max >= 1 {
                let leaf1 = __cpuid(1);
                let vendor_words = [__cpuid(0).ebx, __cpuid(0).ecx, __cpuid(0).edx];
                let vendor = match &vendor_words {
                    [0x756e6547, 0x6c65746e, 0x49656e69] => "GenuineIntel",
                    [0x68747541, 0x444d4163, 0x69746e65] => "AuthenticAMD",
                    _ => "GenuineIntel",
                };
                let family = (leaf1.eax >> 8) & 0xf;
                let family = if family == 0xf {
                    family + ((leaf1.eax >> 20) & 0xff)
                } else {
                    family
                };
                let model_base = (leaf1.eax >> 4) & 0xf;
                let model_ext = (leaf1.eax >> 16) & 0xf;
                let model = if (leaf1.eax >> 8) & 0xf == 0xf || (leaf1.eax >> 8) & 0xf >= 0x6 {
                    (model_ext << 4) | model_base
                } else {
                    model_base
                };
                let stepping = leaf1.eax & 0xf;
                // Leaf 1 EDX+ECX flag bits, rendered as Linux flag names.
                let ecx = leaf1.ecx;
                let edx = leaf1.edx;
                let mut flags = String::from(
                    "fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht",
                );
                let extra: &[(u32, u32, &str)] = &[
                    (ecx, 0, "sse3"),
                    (ecx, 9, "ssse3"),
                    (ecx, 19, "sse4_1"),
                    (ecx, 20, "sse4_2"),
                    (ecx, 23, "popcnt"),
                    (ecx, 25, "aes"),
                    (ecx, 28, "avx"),
                    (edx, 19, "clflush"),
                    (edx, 27, "pni"),
                    (edx, 22, "acpi"),
                    (edx, 29, "lm"),
                ];
                for (reg, bit, name) in extra {
                    if reg & (1 << bit) != 0 {
                        flags.push(' ');
                        flags.push_str(name);
                    }
                }
                let mhz = 2_000_000; // kHz-ish scale for "cpu MHz" display
                return (vendor, family, model, stepping, flags, mhz);
            }
        }
    }
    (
        "GenuineIntel",
        6,
        0,
        0,
        String::from("fpu tsc msr"),
        2_000_000,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &str) -> String {
        String::from_utf8(proc_file_content(path).unwrap_or_default()).unwrap()
    }

    #[test]
    fn meminfo_field_set_matches_linux() {
        if cfg!(windows) {
            let t = text("/proc/meminfo");
            for field in [
                "MemTotal:",
                "MemFree:",
                "MemAvailable:",
                "Buffers:",
                "Cached:",
                "SwapTotal:",
                "SwapFree:",
                "Slab:",
                "Committed_AS:",
                "HugePages_Total:",
                "Hugepagesize:",
                "DirectMap4k:",
            ] {
                assert!(t.contains(field), "missing {field}");
            }
            assert!(t.contains("Active(anon):"));
            assert!(t.ends_with('\n'));
        }
    }

    #[test]
    fn stat_line_set_matches_linux() {
        if cfg!(windows) {
            let t = text("/proc/stat");
            let lines: Vec<&str> = t.lines().collect();
            assert!(lines[0].starts_with("cpu  "));
            assert!(lines[1].starts_with("cpu0 "));
            for field in [
                "intr ",
                "ctxt ",
                "btime ",
                "processes ",
                "procs_running ",
                "procs_blocked ",
                "softirq ",
            ] {
                assert!(t.contains(field), "missing {field}");
            }
            // Ten values after each cpu label.
            assert_eq!(lines[0].split_whitespace().count(), 11);
        }
    }

    #[test]
    fn cpuinfo_block_shape_matches_linux() {
        if cfg!(windows) {
            let t = text("/proc/cpuinfo");
            for field in [
                "processor\t:",
                "vendor_id\t:",
                "cpu family\t:",
                "model name\t:",
                "cpu MHz\t\t:",
                "flags\t\t:",
                "bogomips\t:",
                "cache_alignment\t:",
            ] {
                assert!(t.contains(field), "missing {field}");
            }
            assert_eq!(t.matches("processor\t:").count(), cpu_count());
            assert_eq!(t.matches("flags\t\t:").count(), cpu_count());
        }
    }

    #[test]
    fn uptime_and_loadavg_shapes() {
        if cfg!(windows) {
            let up = text("/proc/uptime");
            let up_fields: Vec<&str> = up.split_whitespace().collect();
            assert_eq!(up_fields.len(), 2);
            assert!(up_fields[0].contains('.'));
            let la = text("/proc/loadavg");
            assert_eq!(la.split_whitespace().count(), 5);
        }
    }

    #[test]
    fn unknown_and_nested_paths_are_not_virtual() {
        assert!(proc_file_content("/proc/self/fd/0").is_none());
        assert!(proc_file_content("/proc/doesnotexist").is_none());
        assert!(proc_file_content("/etc/passwd").is_none());
    }

    /// rubash#1917 A0: the name tables, the content generator, and the
    /// link classification are three faces of one table — every table
    /// name must be served by exactly its own face, and the faces must
    /// not bleed into each other.
    #[test]
    fn entry_tables_agree_with_generators() {
        if cfg!(windows) {
            for name in TOP_LEVEL_FILES {
                assert!(
                    proc_file_content(&format!("/proc/{name}")).is_some(),
                    "top-level table name {name} generates no content"
                );
            }
            let own = std::process::id();
            for name in PID_CONTENT_FILES {
                assert!(
                    proc_file_content(&format!("/proc/self/{name}")).is_some(),
                    "content table name {name} generates no content"
                );
                assert!(
                    !proc_is_link_path(&format!("/proc/self/{name}")),
                    "content table name {name} classified as a link"
                );
            }
            for name in PID_LINK_FILES {
                assert!(
                    proc_file_content(&format!("/proc/self/{name}")).is_none(),
                    "link table name {name} generates content bytes"
                );
                assert!(
                    proc_is_link_path(&format!("/proc/self/{name}")),
                    "link table name {name} not classified as a link"
                );
                assert!(
                    proc_is_link_path(&format!("/proc/{own}/{name}")),
                    "link table name {name} not a link for the live self pid"
                );
            }
            // The enumeration face lists exactly the union of the tables.
            let self_entries = proc_dir_entries("/proc/self").expect("self dir enumerates");
            let mut expected: Vec<&str> = Vec::new();
            expected.extend(PID_CONTENT_FILES);
            expected.extend(PID_LINK_FILES);
            assert_eq!(self_entries.len(), expected.len());
            for name in expected {
                assert!(self_entries.iter().any(|e| e == name), "missing {name}");
            }
        }
    }

    /// rubash#1917 A3: root enumeration = file table + self + live pids;
    /// a dead pid's directory does not enumerate (directory-vanish).
    #[test]
    fn dir_entries_root_and_dead_pid() {
        if cfg!(windows) {
            let root = proc_dir_entries("/proc").expect("root enumerates");
            for name in TOP_LEVEL_FILES {
                assert!(root.iter().any(|e| e == name), "root missing {name}");
            }
            assert!(root.iter().any(|e| e == "self"));
            let own = std::process::id().to_string();
            assert!(root.iter().any(|e| *e == own), "root missing own pid");
            // 4294967294 cannot be a live pid (outside the pid space the
            // allocator hands out), so its directory must not enumerate,
            // its files must not generate, and its links must not classify.
            assert!(proc_dir_entries("/proc/4294967294").is_none());
            assert!(proc_file_content("/proc/4294967294/status").is_none());
            assert!(!proc_is_link_path("/proc/4294967294/exe"));
            assert!(proc_dir_entries("/proc/self/status").is_none());
            assert!(proc_dir_entries("/etc").is_none());
        }
    }

    /// niubash#758: the per-pid procps faces — stat is the 52-field
    /// line (comm parenthesized, state, the snapshot's ppid in field
    /// 4), statm is seven page counts, environ is the true block (the
    /// harness's own PATH entry must appear in the self face), and a
    /// live foreign pid serves the same three faces.
    /// (Attribute-gated rather than `if cfg!`: it calls the
    /// Windows-only snapshot walk directly.)
    #[cfg(windows)]
    #[test]
    fn stat_statm_environ_shapes() {
        {
            let stat = text("/proc/self/stat");
            let fields: Vec<&str> = stat.split_whitespace().collect();
            assert_eq!(fields.len(), 52, "stat field count: {stat}");
            assert!(
                fields[1].starts_with('(') && fields[1].ends_with(')'),
                "comm is parenthesized: {}",
                fields[1]
            );
            assert_eq!(fields[2], "S");
            assert_eq!(fields[0], std::process::id().to_string());
            assert_eq!(fields[3], own_parent_pid().to_string(), "ppid field");
            let statm = text("/proc/self/statm");
            assert_eq!(statm.split_whitespace().count(), 7, "statm: {statm}");
            let environ = proc_file_content("/proc/self/environ").expect("self environ");
            assert!(
                environ.windows(5).any(|w| w == b"PATH="),
                "self environ carries the true environment block"
            );
            assert!(
                environ.ends_with(&[0]),
                "environ entries are NUL-terminated"
            );
            // The status face now prints the memory lines and, for a
            // foreign pid, its true thread count.
            let status = text("/proc/self/status");
            assert!(status.contains("VmSize:"), "status VmSize: {status}");
            assert!(status.contains("VmRSS:"), "status VmRSS: {status}");
            let parent = snapshot_entry(std::process::id())
                .expect("own snapshot entry")
                .parent_pid;
            if parent != 0 {
                let pstat = text(&format!("/proc/{parent}/stat"));
                assert_eq!(
                    pstat.split_whitespace().count(),
                    52,
                    "foreign stat: {pstat}"
                );
                let pentry = snapshot_entry(parent).expect("parent snapshot entry");
                let pstatus = text(&format!("/proc/{parent}/status"));
                assert!(
                    pstatus.contains(&format!("Threads:\t{}", pentry.threads)),
                    "foreign status thread count is the snapshot's: {pstatus}"
                );
                assert!(proc_file_content(&format!("/proc/{parent}/environ")).is_some());
                assert!(proc_file_content(&format!("/proc/{parent}/statm")).is_some());
            }
        }
    }

    /// niubash#758: mounts lines are the six-field procfs shape with
    /// at least the system volume present; net/dev carries the two
    /// Linux header lines and 16-counter rows; the directory faces
    /// agree (root lists net, net lists dev, the fd directory exists
    /// as a directory), and the write classes follow the module's
    /// two-class rule (net/dev 0444, per-pid content 0644).
    #[test]
    fn mounts_net_dev_and_fd_dir_faces() {
        if cfg!(windows) {
            let m = text("/proc/mounts");
            assert!(!m.is_empty(), "at least one volume is mounted");
            for line in m.lines() {
                assert_eq!(line.split_whitespace().count(), 6, "mounts line: {line}");
            }
            let nd = text("/proc/net/dev");
            let mut lines = nd.lines();
            assert!(
                lines.next().unwrap_or_default().starts_with("Inter-|"),
                "net/dev header 1"
            );
            assert!(
                lines.next().unwrap_or_default().contains("face |"),
                "net/dev header 2"
            );
            for line in lines {
                let (_, rest) = line.split_once(':').expect("iface line carries a colon");
                assert_eq!(rest.split_whitespace().count(), 16, "net/dev row: {line}");
            }
            let root = proc_dir_entries("/proc").expect("root enumerates");
            assert!(root.iter().any(|e| e == "net"), "root lists net");
            assert_eq!(
                proc_dir_entries("/proc/net").expect("net enumerates"),
                vec!["dev".to_string()]
            );
            let facts = proc_unary_facts("/proc/self/fd").expect("fd dir facts");
            assert!(facts.is_dir && !facts.is_file && !facts.is_link);
            assert!(proc_unary_facts("/proc/net").expect("net facts").is_dir);
            assert!(
                proc_unary_facts("/proc/net/dev")
                    .expect("net/dev facts")
                    .is_file
            );
            assert_eq!(proc_content_write_allowed("/proc/net/dev"), Some(false));
            assert_eq!(proc_content_write_allowed("/proc/self/stat"), Some(true));
            assert_eq!(proc_content_write_allowed("/proc/self/environ"), Some(true));
            assert_eq!(proc_content_write_allowed("/proc/mounts"), Some(false));
        }
    }

    /// rubash#1917 §4.2 budget, enumeration leg: one root enumeration
    /// pass costs exactly one ToolHelp snapshot (the list_pids walk),
    /// and enumerating the self directory costs none.
    #[test]
    fn enumeration_snapshot_budget() {
        if cfg!(windows) {
            reset_toolhelp_snapshot_count();
            let _ = proc_dir_entries("/proc/self");
            assert_eq!(toolhelp_snapshot_count(), 0, "self dir enumeration");
            let _ = proc_dir_entries("/proc");
            assert_eq!(toolhelp_snapshot_count(), 1, "root enumeration");
        }
    }

    /// rubash#1917 §4.2 budget, self leg: after the one-time own-parent
    /// cache is warm, every /proc/self face is served with zero
    /// ToolHelp snapshots.
    #[test]
    fn self_face_uses_no_snapshot() {
        if cfg!(windows) {
            let _ = proc_file_content("/proc/self/status"); // warm the parent cache
            reset_toolhelp_snapshot_count();
            let _ = proc_file_content("/proc/self/status");
            let _ = proc_file_content("/proc/self/cmdline");
            let _ = proc_file_content("/proc/self");
            assert_eq!(toolhelp_snapshot_count(), 0);
        }
    }

    #[cfg(windows)]
    fn ppid_field(status_text: &str) -> u32 {
        status_text
            .lines()
            .find(|line| line.starts_with("PPid:"))
            .and_then(|line| line["PPid:".len()..].trim().parse().ok())
            .expect("status carries a numeric PPid")
    }

    /// rubash#1918: the status PPid is the snapshot's recorded creator
    /// pid — for self AND for another live pid (pre-fix, self answered
    /// the fabricated 0 and every other pid answered the shell's own
    /// pid; both shapes fail the exact-equality checks below).
    /// (Attribute-gated rather than `if cfg!`: it calls the
    /// Windows-only snapshot walk directly.)
    #[cfg(windows)]
    #[test]
    fn status_ppid_is_snapshot_truth() {
        {
            let own = std::process::id();
            let own_entry = snapshot_entry(own).expect("own snapshot entry");
            assert_eq!(ppid_field(&text("/proc/self/status")), own_entry.parent_pid);
            assert_ne!(own_entry.parent_pid, 0);
            let parent = own_entry.parent_pid;
            let parent_entry = snapshot_entry(parent).expect("parent snapshot entry");
            assert_eq!(
                ppid_field(&text(&format!("/proc/{parent}/status"))),
                parent_entry.parent_pid,
                "another pid's PPid must be ITS parent, not the shell's pid"
            );
        }
    }

    /// rubash#1917 A1: /proc/self/cmdline is the TRUE argv (the PEB /
    /// GetCommandLineW command line), not the snapshot's image name —
    /// argv[0] as invoked is a path ending in the image name, and the
    /// PEB route and the self fast path derive identical bytes.
    #[cfg(windows)]
    #[test]
    fn cmdline_is_true_argv_for_self() {
        let bytes = proc_file_content("/proc/self/cmdline").expect("self cmdline");
        assert!(bytes.ends_with(&[0]), "cmdline is NUL-terminated");
        let fields: Vec<&[u8]> = bytes[..bytes.len() - 1].split(|b| *b == 0).collect();
        assert!(!fields.is_empty());
        let image = self_image_name().expect("own image name");
        let argv0 = String::from_utf8_lossy(fields[0]).into_owned();
        assert!(
            argv0.ends_with(&image),
            "argv[0] {argv0:?} should end with the image name {image:?}"
        );
        // The fallback shape is the BARE image name as the only
        // field; the true read yields argv[0] as invoked — a full
        // path. (The harness may invoke the test binary with no
        // arguments at all, so a field-count premise would be wrong;
        // path-vs-bare-name is the discriminator that always holds.)
        assert!(
            argv0 != image && argv0.contains('\\'),
            "true argv[0] is the invocation path, not the bare-name fallback: {argv0:?}"
        );
        let entry = snapshot_entry(std::process::id()).expect("own snapshot entry");
        assert_eq!(pid_cmdline(std::process::id(), &entry), self_cmdline());
    }

    /// rubash#1917 §7.1: the fallback shape is exactly the image name
    /// plus its NUL — failure of the true read degrades content, never
    /// shape. (Attribute-gated: it calls the Windows-only fallback
    /// constructor directly.)
    #[cfg(windows)]
    #[test]
    fn cmdline_fallback_shape_is_image_plus_nul() {
        assert_eq!(image_cmdline_fallback("prog.exe"), b"prog.exe\0".to_vec());
    }

    /// rubash#1900: link resolution and unary facts — self/cwd is
    /// exactly the session cwd the caller passed (both forms pass
    /// through; the process-global cwd is never asked), self/exe
    /// resolves to the running image, content entries are not links,
    /// a dead pid dangles, and the facts face agrees.
    #[test]
    #[cfg(windows)]
    fn link_targets_and_facts() {
        {
            let cwd = self_cwd_forms(std::path::Path::new("D:\\some\\session dir"));
            let target = proc_link_target("/proc/self/cwd", Some(&cwd)).expect("self cwd link");
            assert_eq!(target.native, "D:\\some\\session dir");
            assert_eq!(target.display, cwd.display);
            assert!(proc_link_target("/proc/self/cwd", None).is_none());
            let exe = proc_link_target("/proc/self/exe", None).expect("self exe link");
            let image = self_image_name().expect("own image name");
            assert!(exe.native.ends_with(&image), "exe target {}", exe.native);
            assert!(exe.display.starts_with('/'));
            assert!(proc_link_target("/proc/self/status", Some(&cwd)).is_none());
            assert!(proc_link_target("/proc/4294967294/exe", None).is_none());
            let facts = proc_unary_facts("/proc/self/exe").expect("exe facts");
            assert!(facts.is_link && !facts.link_target_is_dir);
            let facts = proc_unary_facts("/proc/self/cwd").expect("cwd facts");
            assert!(facts.is_link && facts.link_target_is_dir);
            assert!(proc_unary_facts("/proc/self").expect("dir facts").is_dir);
            assert!(
                proc_unary_facts("/proc/self/status")
                    .expect("file facts")
                    .is_file
            );
            assert!(proc_unary_facts("/proc").expect("root facts").is_dir);
            assert!(proc_unary_facts("/proc/4294967294/status").is_none());
        }
    }
}

/// Pids to list for `/proc` enumeration (ToolHelp snapshot), ascending.
/// Empty on non-Windows (the real procfs lists itself there).
#[cfg(windows)]
pub fn list_pids() -> Vec<u32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        Process32FirstW, Process32NextW, PROCESSENTRY32W,
    };
    let Some(snapshot) = open_process_snapshot() else {
        return Vec::new();
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut pids = Vec::new();
    unsafe {
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                pids.push(entry.th32ProcessID);
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    pids.sort_unstable();
    pids
}

#[cfg(not(windows))]
pub fn list_pids() -> Vec<u32> {
    Vec::new()
}
