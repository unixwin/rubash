//! kill module.
//!
//! GNU Bash source ownership:
// - builtins/kill.def

use std::io::{self, Write};

const SIGNALS: &[(i32, &str, &str)] = &[
    (1, "1", "HUP"),
    (2, "2", "INT"),
    (3, "3", "QUIT"),
    (4, "4", "ILL"),
    (5, "5", "TRAP"),
    (6, "6", "ABRT"),
    (7, "7", "BUS"),
    (8, "8", "FPE"),
    (9, "9", "KILL"),
    (10, "10", "USR1"),
    (11, "11", "SEGV"),
    (12, "12", "USR2"),
    (13, "13", "PIPE"),
    (14, "14", "ALRM"),
    (15, "15", "TERM"),
    (16, "16", "STKFLT"),
    (17, "17", "CHLD"),
    (18, "18", "CONT"),
    (19, "19", "STOP"),
    (20, "20", "TSTP"),
    (21, "21", "TTIN"),
    (22, "22", "TTOU"),
    (23, "23", "URG"),
    (24, "24", "XCPU"),
    (25, "25", "XFSZ"),
    (26, "26", "VTALRM"),
    (27, "27", "PROF"),
    (28, "28", "WINCH"),
    (29, "29", "IO"),
    (30, "30", "PWR"),
    (31, "31", "SYS"),
    (34, "34", "RTMIN"),
    (35, "35", "RTMIN+1"),
    (36, "36", "RTMIN+2"),
    (37, "37", "RTMIN+3"),
    (38, "38", "RTMIN+4"),
    (39, "39", "RTMIN+5"),
    (40, "40", "RTMIN+6"),
    (41, "41", "RTMIN+7"),
    (42, "42", "RTMIN+8"),
    (43, "43", "RTMIN+9"),
    (44, "44", "RTMIN+10"),
    (45, "45", "RTMIN+11"),
    (46, "46", "RTMIN+12"),
    (47, "47", "RTMIN+13"),
    (48, "48", "RTMIN+14"),
    (49, "49", "RTMIN+15"),
    (50, "50", "RTMAX-14"),
    (51, "51", "RTMAX-13"),
    (52, "52", "RTMAX-12"),
    (53, "53", "RTMAX-11"),
    (54, "54", "RTMAX-10"),
    (55, "55", "RTMAX-9"),
    (56, "56", "RTMAX-8"),
    (57, "57", "RTMAX-7"),
    (58, "58", "RTMAX-6"),
    (59, "59", "RTMAX-5"),
    (60, "60", "RTMAX-4"),
    (61, "61", "RTMAX-3"),
    (62, "62", "RTMAX-2"),
    (63, "63", "RTMAX-1"),
    (64, "64", "RTMAX"),
];

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut stdout, &mut stderr)
}

pub fn execute_with_io<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let Some(first) = args.first().map(String::as_str) else {
        write_kill_usage(stderr)?;
        return Ok(2);
    };

    if matches!(first, "-l" | "-L") {
        if args.len() == 1 || args.get(1).is_some_and(|value| value == "-1") {
            write_signal_list(stdout)?;
            return Ok(0);
        }
        let mut status = 0;
        for value in &args[1..] {
            if let Some(translation) = translate_signal(value) {
                writeln!(stdout, "{translation}")?;
            } else {
                writeln!(
                    stderr,
                    "{}kill: {value}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                status = 1;
            }
        }
        return Ok(status);
    }

    let mut signal = 15;
    let mut index = 0;
    let mut operands_start = 0;
    while let Some(value) = args.get(index).map(String::as_str) {
        if value == "--" {
            operands_start = index + 1;
            break;
        }
        if value == "-s" || value == "-n" {
            let Some(sigspec) = args.get(index + 1).map(String::as_str) else {
                write_kill_usage(stderr)?;
                return Ok(2);
            };
            if translate_signal(sigspec).is_none() {
                writeln!(
                    stderr,
                    "{}kill: {sigspec}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                return Ok(1);
            }
            signal = signal_number_from_spec(sigspec).unwrap_or(15);
            index += 2;
            operands_start = index;
            // After an explicit signal option, the remaining words are
            // operands. In particular, `kill -s 0 -1` uses -1 as a process
            // group operand, not as another short signal specification.
            break;
        }
        if value.starts_with('-') && value != "-" {
            let sigspec = value.trim_start_matches('-');
            if translate_signal(sigspec).is_none() {
                writeln!(
                    stderr,
                    "{}kill: {sigspec}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                return Ok(1);
            }
            signal = signal_number_from_spec(sigspec).unwrap_or(15);
            index += 1;
            operands_start = index;
            continue;
        }
        operands_start = index;
        break;
    }

    if operands_start >= args.len() {
        write_kill_usage(stderr)?;
        return Ok(2);
    }

    let mut status = 0;
    for operand in &args[operands_start..] {
        if signal == 0 && matches!(operand.as_str(), "-1" | "0") {
            // Windows cannot address a Unix process group through
            // OpenProcess, but signal 0 is only an existence probe. These
            // group targets are valid Bash operands and do not require a
            // native termination call.
            continue;
        }

        let Some(pid) = parse_pid(operand) else {
            writeln!(
                stderr,
                "{}kill: {operand}: arguments must be process or job IDs",
                diagnostic_prefix()
            )?;
            status = 1;
            continue;
        };

        // Negative pid → Unix process group.  Windows has no such target,
        // so silently skip delivery (signal 0 already handled above for
        // existence probes of the current group).
        if pid < 0 {
            continue;
        }

        let pid = pid as u32;

        // Bash treats pid 0 as the current process group. Windows does not
        // expose that target through OpenProcess, but signal 0 is only an
        // existence probe, so the shell can answer it from its own group
        // context without attempting to terminate a process.
        if pid == 0 && signal == 0 {
            continue;
        }

        match deliver_rubash_signal(pid, signal) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                writeln!(stderr, "{}kill: ({pid}) - {error}", diagnostic_prefix())?;
                status = 1;
                continue;
            }
        }

        if let Err(message) = signal_process(pid, signal) {
            writeln!(stderr, "{}kill: ({pid}) - {message}", diagnostic_prefix())?;
            status = 1;
        }
    }

    Ok(status)
}

fn write_kill_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(
        stderr,
        "{}kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]",
        diagnostic_prefix()
    )
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub fn signal_number_for_spec(value: &str) -> Option<i32> {
    signal_number_from_spec(value)
}

pub fn translate_signal(value: &str) -> Option<&'static str> {
    if value == "0" {
        return Some("EXIT");
    }

    if value == "EXIT" || value == "SIGEXIT" {
        return Some("0");
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return signal_name(number);
    }

    let name = value.strip_prefix("SIG").unwrap_or(value);
    signal_number(name)
}

fn signal_number_from_spec(value: &str) -> Option<i32> {
    if value == "0" {
        return Some(0);
    }

    let name = value.strip_prefix("SIG").unwrap_or(value);
    if name == "EXIT" {
        return Some(0);
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return (number == 0 || signal_name(number).is_some()).then_some(number);
    }

    signal_number(name)?.parse::<i32>().ok()
}

fn parse_pid(value: &str) -> Option<i32> {
    value.parse::<i32>().ok()
}

pub fn process_exists(pid: u32) -> bool {
    pid == std::process::id() || signal_process(pid, 0).is_ok_or_permission_denied()
}

pub fn register_signal_mailbox(pid: u32) -> io::Result<()> {
    let dir = signal_mailbox_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(signal_marker_path(pid), std::process::id().to_string())
}

pub fn unregister_signal_mailbox(pid: u32) {
    let _ = std::fs::remove_file(signal_marker_path(pid));
    let _ = std::fs::remove_file(signal_queue_path(pid));
    if let Ok(entries) = std::fs::read_dir(signal_mailbox_dir()) {
        let prefix = format!("{pid}.q.");
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn parse_signal_lines(content: &str) -> Vec<i32> {
    content
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .collect()
}

pub fn take_pending_signals(pid: u32) -> io::Result<Vec<i32>> {
    let dir = signal_mailbox_dir();
    let mut signals = Vec::new();
    // Legacy single-file queue: take it atomically via rename so a signal
    // appended after the read is not dropped by the remove below.
    let legacy_taken = dir.join(format!("{pid}.queue.taking"));
    if std::fs::rename(signal_queue_path(pid), &legacy_taken).is_ok() {
        if let Ok(content) = std::fs::read_to_string(&legacy_taken) {
            signals.extend(parse_signal_lines(&content));
        }
        let _ = std::fs::remove_file(&legacy_taken);
    }
    // Per-delivery entries: senders write a unique .part file and rename it
    // into place, so the reader only ever sees fully written entries and a
    // concurrent delivery can never be lost. The old read-then-remove
    // window dropped a signal whenever a delivery landed between the two
    // steps, which made the trap9 kill-to-self from a busy loop
    // unreliable.
    let prefix = format!("{pid}.q.");
    let mut entries: Vec<std::path::PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with(&prefix) && !name.ends_with(".part"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(signals),
        Err(error) => return Err(error),
    };
    entries.sort();
    for path in entries {
        if let Ok(content) = std::fs::read_to_string(&path) {
            signals.extend(parse_signal_lines(&content));
        }
        let _ = std::fs::remove_file(&path);
    }
    Ok(signals)
}

fn signal_name(number: i32) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(signal_number, _, name)| (*signal_number == number).then_some(*name))
}

fn signal_number(name: &str) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(_, number, signal_name)| (*signal_name == name).then_some(*number))
}

fn write_signal_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    // GNU kill.def list_signals: 5 entries per line, tab-separated; the
    // final partial line still carries the tab separator before the closing
    // newline (captured from GNU 5.3.0 `kill -l` on the WSL baseline).
    let total = SIGNALS.len();
    for (position, (number, _, name)) in SIGNALS.iter().enumerate() {
        let position = position + 1;
        if position > 1 && (position - 1) % 5 == 0 {
            writeln!(stdout)?;
        } else if position > 1 {
            write!(stdout, "\t")?;
        }
        write!(stdout, "{number:>2}) SIG{name}")?;
        if position == total && position % 5 != 0 {
            write!(stdout, "\t")?;
        }
    }
    writeln!(stdout)?;
    Ok(())
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

fn deliver_rubash_signal(pid: u32, signal: i32) -> io::Result<bool> {
    if !signal_marker_path(pid).is_file() {
        return Ok(false);
    }
    // A background Rubash process owns its own mailbox even though it
    // inherits the parent shell identity for expansion semantics. Do not
    // reject that child merely because the marker names the parent.
    // SIGKILL is not trappable. STOP/TSTP/CONT must use the native Windows
    // process-state backend rather than a mailbox event.
    if matches!(signal, 9 | 17 | 18 | 19) {
        return Ok(false);
    }
    if !process_exists(pid) {
        unregister_signal_mailbox(pid);
        return Ok(false);
    }
    if signal == 0 {
        return Ok(true);
    }

    std::fs::create_dir_all(signal_mailbox_dir())?;
    // Deliver as a unique per-signal entry: write to a .part file and
    // rename it into place. The rename is atomic, so the reading shell
    // either misses the entry entirely or observes it fully written; a
    // shared append-queue could lose the write to the reader's
    // read-then-remove window.
    static SIGNAL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SIGNAL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = format!("{pid}.q.{}.{}", std::process::id(), seq);
    let part = signal_mailbox_dir().join(format!("{base}.part"));
    let full = signal_mailbox_dir().join(&base);
    std::fs::write(&part, format!("{signal}
"))?;
    std::fs::rename(&part, &full)?;
    Ok(true)
}

fn signal_mailbox_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("rubash-signals")
}

fn signal_marker_path(pid: u32) -> std::path::PathBuf {
    signal_mailbox_dir().join(format!("{pid}.alive"))
}

fn signal_queue_path(pid: u32) -> std::path::PathBuf {
    signal_mailbox_dir().join(format!("{pid}.queue"))
}

#[cfg(windows)]
fn signal_process(pid: u32, signal: i32) -> Result<(), &'static str> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, OpenThread, ResumeThread, SuspendThread, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
    };

    // Linux numbering: CONT=18 resumes, STOP=19/TSTP=20 suspend. SIGCHLD
    // (17) is delivered-but-ignored by default, so kill -CHLD succeeds as a
    // no-op instead of terminating (GNU kill.def sends it through kill(2)
    // where the default disposition is SIG_DFL-ignored).
    if signal == 17 {
        return Ok(());
    }
    if signal == 18 || signal == 19 || signal == 20 {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == -1isize as _ {
            return Err("Cannot enumerate process threads");
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        let mut found = false;
        let mut result = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while result {
            if entry.th32OwnerProcessID == pid {
                found = true;
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    unsafe { CloseHandle(snapshot) };
                    return Err("Cannot open process thread");
                }
                let failed = if signal == 18 {
                    unsafe { ResumeThread(thread) == u32::MAX }
                } else {
                    unsafe { SuspendThread(thread) == u32::MAX }
                };
                unsafe { CloseHandle(thread) };
                if failed {
                    unsafe { CloseHandle(snapshot) };
                    return Err("Failed to change process state");
                }
            }
            result = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        return if found {
            Ok(())
        } else {
            Err("No such process")
        };
    }

    use windows_sys::Win32::System::Threading::PROCESS_TERMINATE;
    let access = if signal == 0 {
        PROCESS_QUERY_LIMITED_INFORMATION
    } else {
        PROCESS_TERMINATE
    };

    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        return Err(match error {
            ERROR_ACCESS_DENIED => "Permission denied",
            ERROR_INVALID_PARAMETER => "No such process",
            _ => "Cannot open process",
        });
    }

    if signal == 0 {
        let mut exit_code = 0;
        let process_is_active = unsafe {
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32
        };
        if !process_is_active {
            unsafe { CloseHandle(handle) };
            return Err("No such process");
        }
    } else if unsafe { TerminateProcess(handle, (128 + signal) as u32) == 0 } {
        unsafe { CloseHandle(handle) };
        return Err("Failed to terminate process");
    }

    unsafe { CloseHandle(handle) };
    Ok(())
}

#[cfg(unix)]
fn signal_process(pid: u32, signal: i32) -> Result<(), &'static str> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }

    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Err("No such process"),
        Some(libc::EPERM) => Err("Permission denied"),
        _ => Err("Failed to signal process"),
    }
}

#[cfg(not(any(unix, windows)))]
fn signal_process(_pid: u32, signal: i32) -> Result<(), &'static str> {
    if signal == 0 {
        Err("No such process")
    } else {
        Err("Failed to signal process")
    }
}

trait KillResultExt {
    fn is_ok_or_permission_denied(&self) -> bool;
}

impl KillResultExt for Result<(), &'static str> {
    fn is_ok_or_permission_denied(&self) -> bool {
        self.is_ok() || matches!(self, Err(message) if *message == "Permission denied")
    }
}
