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

/// Top-level synthetic file names (the /proc root's file face). The name
/// table, the content generator below, and the directory enumeration
/// ([`proc_dir_entries`]) are defined in this one module from this one
/// table, so a name can never be enumerable-but-unreadable or
/// readable-but-unenumerable (rubash#1917 A0: one table, three faces).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const TOP_LEVEL_FILES: [&str; 6] = ["cpuinfo", "loadavg", "meminfo", "stat", "uptime", "version"];

/// Per-pid CONTENT entries (bytes generated on read).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const PID_CONTENT_FILES: [&str; 2] = ["cmdline", "status"];

/// Per-pid LINK entries (readlink-type: the entry resolves to a target
/// path elsewhere in the filesystem; see [`proc_link_target`]).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const PID_LINK_FILES: [&str; 2] = ["cwd", "exe"];

/// Every per-pid directory entry name, content and link faces together
/// (the `/proc/<pid>` and `/proc/self` enumeration face).
#[allow(dead_code)] // table face consumed by the Windows generators and the cross-platform tests
const PID_ENTRIES: [&str; 4] = ["cmdline", "cwd", "exe", "status"];

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
        names.push("self".to_string());
        names.extend(list_pids().into_iter().map(|pid| pid.to_string()));
        return Some(names);
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
        "status" => Some(pid_status(pid, &entry.image, entry.parent_pid).into_bytes()),
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
            match self_image_name() {
                Some(image) => Some(pid_status(own, &image, own_parent_pid()).into_bytes()),
                None => snapshot_entry(own)
                    .map(|entry| pid_status(own, &entry.image, entry.parent_pid).into_bytes()),
            }
        }
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

/// The strings the PEB route can read for one process.
#[cfg(windows)]
struct PebStrings {
    /// RTL_USER_PROCESS_PARAMETERS.CommandLine, raw UTF-16 units.
    command_line: Option<Vec<u16>>,
    /// RTL_USER_PROCESS_PARAMETERS.CurrentDirectory.DosPath, rendered
    /// (the /proc/<pid>/cwd face consumes this in the #1900 slice).
    current_dir: Option<String>,
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
    if let Some((pid_ref, tail)) = parse_pid_spelling(trimmed) {
        if !pid_ref_alive(pid_ref) {
            return None;
        }
        if tail.is_empty() {
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
        Some(PebStrings {
            command_line: unicode_string(PARAMS_CMDLINE_OFFSET),
            current_dir: unicode_string(PARAMS_CURDIR_OFFSET)
                .map(|units| String::from_utf16_lossy(&units)),
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

/// One process's ToolHelp snapshot record: image name and the REAL
/// parent pid the kernel recorded at its creation
/// (PROCESSENTRY32W.th32ParentProcessID — rubash#1918: this field was
/// previously read by the walk and thrown away, while pid_status
/// fabricated a PPid).
#[cfg(windows)]
struct SnapshotEntry {
    image: String,
    parent_pid: u32,
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

/// Linux /proc/<pid>/status field set (name/state/pid/ppid/threads...).
/// The PPid is the snapshot's recorded creator pid, passed in by the
/// caller — this function invents nothing (rubash#1918: the previous
/// body answered 0 for self and the shell's own pid for everyone else,
/// while the true value sat unused in the same snapshot record).
#[cfg(windows)]
fn pid_status(pid: u32, image: &str, ppid: u32) -> String {
    format!(
        "Name:\t{image}\n\
         State:\tS (sleeping)\n\
         Tgid:\t{pid}\n\
         Pid:\t{pid}\n\
         PPid:\t{ppid}\n\
         Threads:\t1\n"
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
