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

/// Synthetic content for a /proc pseudo-file, or None when `path` is not one
/// of the emulated files. Each call generates a fresh snapshot.
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
    // Unknown pid: the directory probe (tail == "") must report missing, and
    // so must any file under it -- procfs dirs vanish when the process does.
    let Some((image, mut cmdline)) = pid_identity(pid) else {
        return None;
    };
    match tail {
        // NUL-separated argv, per procfs; the reader splits on '\0'.
        "cmdline" => {
            if cmdline.is_empty() {
                cmdline = image.clone().into_bytes();
                cmdline.push(0);
            }
            Some(cmdline)
        }
        "status" => Some(pid_status(pid, &image).into_bytes()),
        "" => Some(Vec::new()),
        _ => None,
    }
}

/// Image name + command line for `pid` via the ToolHelp snapshot. The
/// snapshot only knows the image name, so cmdline is the image NUL-terminated.
#[cfg(windows)]
fn pid_identity(pid: u32) -> Option<(String, Vec<u8>)> {
    snapshot_identity(pid)
}

/// ToolHelp snapshot: the process's image name as argv[0] (NUL-terminated),
/// or None when the pid does not exist.
#[cfg(windows)]
fn snapshot_identity(pid: u32) -> Option<(String, Vec<u8>)> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
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
                    found = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    found.map(|image| {
        let mut cmdline = image.clone().into_bytes();
        cmdline.push(0);
        (image, cmdline)
    })
}

/// Linux /proc/<pid>/status field set (name/state/pid/ppid/threads...).
#[cfg(windows)]
fn pid_status(pid: u32, image: &str) -> String {
    let ppid = if pid == std::process::id() {
        0
    } else {
        std::process::id()
    };
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
                let mut flags = String::from("fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht");
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
}

/// Pids to list for `/proc` enumeration (ToolHelp snapshot), ascending.
/// Empty on non-Windows (the real procfs lists itself there).
#[cfg(windows)]
pub fn list_pids() -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
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
