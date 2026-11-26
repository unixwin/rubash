//! niubash#451 / #379 regression gate: a console-less `rubash -c` run must
//! never show a console window — not for the shell and not for any external
//! child it spawns. The bug class: a console-subsystem child created by a
//! parent with no console gets a *visible* console auto-allocated by
//! Windows. Children of a detached shell take the executor's
//! CREATE_NO_WINDOW path — a *hidden* console is fine, a visible window
//! fails this gate.

#![cfg(windows)]

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

type Hwnd = HWND;

fn process_entry() -> PROCESSENTRY32W {
    PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    }
}

fn visible_console_window_owners() -> Vec<u32> {
    use std::sync::Mutex;
    static OWNERS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    unsafe extern "system" fn collect(hwnd: Hwnd, _lparam: isize) -> i32 {
        let mut cls = [0u16; 64];
        unsafe {
            if GetClassNameW(hwnd, cls.as_mut_ptr(), cls.len() as i32) == 0 {
                return 1;
            }
            let name =
                String::from_utf16_lossy(&cls[..cls.iter().position(|&c| c == 0).unwrap_or(64)]);
            if name == "ConsoleWindowClass" && IsWindowVisible(hwnd) != 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, &mut pid);
                OWNERS.lock().unwrap().push(pid);
            }
        }
        1
    }
    OWNERS.lock().unwrap().clear();
    unsafe {
        EnumWindows(Some(collect), 0);
    }
    std::mem::take(&mut *OWNERS.lock().unwrap())
}

fn process_tree() -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == INVALID_HANDLE_VALUE {
            return map;
        }
        let mut pe = process_entry();
        if Process32FirstW(snap, &mut pe) != 0 {
            loop {
                map.insert(pe.th32ProcessID, pe.th32ParentProcessID);
                if Process32NextW(snap, &mut pe) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    map
}

fn in_tree(pid: u32, root: u32, tree: &HashMap<u32, u32>) -> bool {
    let mut cur = pid;
    for _ in 0..64 {
        if cur == root {
            return true;
        }
        match tree.get(&cur) {
            Some(&next) if next != cur && next != 0 => cur = next,
            _ => return false,
        }
    }
    false
}

/// Snapshot `(pid, ppid, exe_name_lowercase)` for every process on the host.
fn process_entries() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut pe = process_entry();
        if Process32FirstW(snap, &mut pe) != 0 {
            loop {
                let len = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let name = String::from_utf16_lossy(&pe.szExeFile[..len]).to_ascii_lowercase();
                out.push((pe.th32ProcessID, pe.th32ParentProcessID, name));
                if Process32NextW(snap, &mut pe) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

fn collect_offenders(child_pid: u32, mut alive: impl FnMut() -> bool) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut offenders = Vec::new();
    while alive() && Instant::now() < deadline {
        let owners = visible_console_window_owners();
        if !owners.is_empty() {
            let tree = process_tree();
            for owner in owners {
                if in_tree(owner, child_pid, &tree) && !offenders.contains(&owner) {
                    offenders.push(owner);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    offenders
}

fn spawn_detached(exe: &str, args: &[&str]) -> std::process::Child {
    let mut command = Command::new(exe);
    command
        .args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().expect("spawn child")
}

#[test]
fn detached_rubash_c_spawns_no_visible_console_window() {
    let rubash = env!("CARGO_BIN_EXE_rubash");
    // ~2s of external-command work — long enough for a regression to flash.
    let mut child = spawn_detached(
        rubash,
        &["-c", "C:/Windows/System32/ping.exe -n 3 127.0.0.1 >NUL"],
    );
    let pid = child.id();
    let offenders = collect_offenders(pid, || child.try_wait().ok().flatten().is_none());
    let status = child.wait().expect("wait rubash");
    assert!(status.success(), "rubash -c ping exited {status}");
    assert!(
        offenders.is_empty(),
        "visible console windows appeared in the rubash tree (owner pids {offenders:?})"
    );
}

#[test]
fn shim_forwarding_spawns_no_visible_console_window() {
    // The bash.exe forwarder needs a host shell; skip when no niu.exe is
    // configured for this run (NIU_SHELL is the shim's own resolver key).
    let Some(niu) = std::env::var_os("NIU_SHELL") else {
        eprintln!("skip: NIU_SHELL unset — no niu.exe for the shim to forward to");
        return;
    };
    let bash = env!("CARGO_BIN_EXE_bash");
    let mut command = Command::new(bash);
    command
        .args(["-c", "C:/Windows/System32/ping.exe -n 3 127.0.0.1 >NUL"])
        .env("NIU_SHELL", niu)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn shim");
    let pid = child.id();
    let offenders = collect_offenders(pid, || child.try_wait().ok().flatten().is_none());
    let status = child.wait().expect("wait shim");
    assert!(status.success(), "bash -c ping exited {status}");
    assert!(
        offenders.is_empty(),
        "visible console windows appeared in the shim tree (owner pids {offenders:?})"
    );
}

#[test]
fn detached_multi_command_session_allocates_at_most_one_hidden_console() {
    // niubash#462: a detached `-c` session running N sequential external
    // commands must claim the first child's hidden console and let children
    // 2..N inherit it, allocating at most 1 conhost.exe for the entire
    // session while preserving piped stdout/stderr across all N.
    let script = concat!(
        "C:/Windows/System32/hostname.exe; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "C:/Windows/System32/hostname.exe; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "C:/Windows/System32/hostname.exe"
    );
    let (output, offenders, conhost_pids) =
        run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(
        output.status.success(),
        "rubash -c exited {}",
        output.status
    );
    assert!(
        offenders.is_empty(),
        "visible console windows appeared (owner pids {offenders:?})"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        3,
        "piped stdout must stay intact across AttachConsole + subsequent children; got: {stdout:?}"
    );
    assert!(
        conhost_pids.len() <= 1,
        "expected at most 1 conhost.exe across 5 external spawns, saw {}: {conhost_pids:?}",
        conhost_pids.len()
    );
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn run_and_observe_rubash(
    args: &[&str],
    flags: u32,
) -> (
    std::process::Output,
    Vec<u32>,
    std::collections::HashSet<u32>,
) {
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let mut command = Command::new(rubash);
    command
        .args(args)
        .creation_flags(flags | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn rubash");
    let root_pid = child.id();
    let mut conhost_pids = std::collections::HashSet::new();
    let mut offenders = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        let entries = process_entries();
        let tree: HashMap<u32, u32> = entries.iter().map(|&(p, pp, _)| (p, pp)).collect();
        for &(pid, _ppid, ref name) in &entries {
            if name == "conhost.exe" && in_tree(pid, root_pid, &tree) {
                conhost_pids.insert(pid);
            }
        }
        for owner in visible_console_window_owners() {
            if in_tree(owner, root_pid, &tree) && !offenders.contains(&owner) {
                offenders.push(owner);
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let output = child.wait_with_output().expect("wait rubash");
    (output, offenders, conhost_pids)
}

fn non_empty_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn detached_builtin_only_session_allocates_zero_conhosts() {
    let script = concat!(
        "acc=0; ",
        "for i in 1 2 3 4 5; do acc=$((acc + i)); done; ",
        "printf 'SUM=%d\\n' \"$acc\"; ",
        "test \"$acc\" -eq 15 && echo BUILTIN-OK; ",
        "pwd >/dev/null"
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.is_empty(),
        "builtin-only session must allocate 0 conhost.exe, saw: {conhosts:?}"
    );
    assert_eq!(
        non_empty_lines(&output.stdout),
        vec!["SUM=15", "BUILTIN-OK"]
    );
}

#[test]
fn detached_command_substitutions_share_single_hidden_console() {
    let script = concat!(
        "h1=$(C:/Windows/System32/hostname.exe); ",
        "h2=`C:/Windows/System32/hostname.exe`; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "h3=$(C:/Windows/System32/hostname.exe); ",
        "echo \"$h1|$h2|$h3\""
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "command substitutions must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    let lines = non_empty_lines(&output.stdout);
    assert_eq!(lines.len(), 1);
    let parts: Vec<&str> = lines[0].split('|').collect();
    assert_eq!(parts.len(), 3);
    assert!(!parts[0].is_empty() && parts[0] == parts[1] && parts[1] == parts[2]);
}

#[test]
fn detached_pipelines_share_single_hidden_console() {
    let script = concat!(
        "C:/Windows/System32/hostname.exe | C:/Windows/System32/findstr.exe . ; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 | C:/Windows/System32/findstr.exe . >NUL ; ",
        "C:/Windows/System32/hostname.exe | C:/Windows/System32/findstr.exe ."
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "pipelines must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    assert_eq!(non_empty_lines(&output.stdout).len(), 2);
}

#[test]
fn detached_background_jobs_and_wait_share_single_hidden_console() {
    // Covers BOTH direct external `&` jobs (`compound_exec.rs:467`) AND
    // redirected/compound `&` jobs that route through `fd::spawn_whitelisted`
    // (`compound_exec.rs:732` / `windows_impl.rs:1385`) as the very first
    // command of a console-less session.
    let script = concat!(
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL & p1=$!; ",
        "( C:/Windows/System32/hostname.exe >/dev/null ) & p2=$!; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 & p3=$!; ",
        "wait $p1 $p2 $p3; ",
        "C:/Windows/System32/hostname.exe"
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "background jobs (including spawn_whitelisted wrapper jobs) must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    assert!(!non_empty_lines(&output.stdout).is_empty());
}

#[test]
fn detached_subshells_functions_and_loops_share_single_hidden_console() {
    let script = concat!(
        "emit_host() { C:/Windows/System32/hostname.exe; }; ",
        "( emit_host; C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL ); ",
        "for n in 1 2 3; do emit_host; done"
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "subshells/functions/loops must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    assert_eq!(non_empty_lines(&output.stdout).len(), 4);
}

#[test]
fn detached_nested_rubash_and_exec_builtin_share_single_hidden_console() {
    let script = concat!(
        "\"$THIS_SH\" -c \"C:/Windows/System32/hostname.exe; C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL\"; ",
        "\"$THIS_SH\" -c \"exec C:/Windows/System32/hostname.exe\""
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "nested rubash + exec must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    assert_eq!(non_empty_lines(&output.stdout).len(), 2);
}

#[test]
fn create_no_window_parent_reuses_caller_hidden_console() {
    let script = concat!(
        "C:/Windows/System32/hostname.exe; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "C:/Windows/System32/hostname.exe; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "C:/Windows/System32/hostname.exe"
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], CREATE_NO_WINDOW);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "CREATE_NO_WINDOW parent must have <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    assert_eq!(non_empty_lines(&output.stdout).len(), 3);
}

#[test]
fn detached_scaling_timing_gate_avoids_per_spawn_console_tax() {
    let cmd = "C:/Windows/System32/hostname.exe >NUL";
    let script_1 = cmd.to_string();
    let script_10 = std::iter::repeat(cmd)
        .take(10)
        .collect::<Vec<_>>()
        .join("; ");

    let _ = run_and_observe_rubash(&["-c", &script_1], DETACHED_PROCESS);

    let mut samples_1 = Vec::new();
    let mut samples_10 = Vec::new();
    for _ in 0..3 {
        let t0 = Instant::now();
        let (out1, off1, _) = run_and_observe_rubash(&["-c", &script_1], DETACHED_PROCESS);
        let d1 = t0.elapsed();
        assert!(out1.status.success());
        assert!(off1.is_empty(), "visible windows in N=1: {off1:?}");
        samples_1.push(d1);

        let t0 = Instant::now();
        let (out10, off10, con10) = run_and_observe_rubash(&["-c", &script_10], DETACHED_PROCESS);
        let d10 = t0.elapsed();
        assert!(out10.status.success());
        assert!(off10.is_empty(), "visible windows in N=10: {off10:?}");
        assert!(
            con10.len() <= 1,
            "N=10 must allocate <=1 conhost.exe, saw {}: {con10:?}",
            con10.len()
        );
        samples_10.push(d10);
    }
    samples_1.sort();
    samples_10.sort();
    let med_1 = samples_1[1];
    let med_10 = samples_10[1];
    let per_extra_ms = med_10.saturating_sub(med_1).as_secs_f64() * 1000.0 / 9.0;
    eprintln!(
        "[#462 rubash timing gate] N=1 median={med_1:?}, N=10 median={med_10:?}, per_extra_spawn={per_extra_ms:.1} ms"
    );
    assert!(
        per_extra_ms < 35.0,
        "per-spawn overhead too high ({per_extra_ms:.1} ms/spawn; N=1 {med_1:?}, N=10 {med_10:?})"
    );
}

#[test]
fn detached_coproc_and_process_substitution_share_single_hidden_console() {
    let script = concat!(
        "coproc CP { C:/Windows/System32/hostname.exe; }; ",
        "read -r line <&\"${CP[0]}\"; ",
        "wait \"$CP_PID\" 2>/dev/null || true; ",
        "C:/Windows/System32/findstr.exe . <(C:/Windows/System32/hostname.exe); ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "echo \"COPROC=$line\""
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", script], DETACHED_PROCESS);
    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "coproc/procsub must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    let lines = non_empty_lines(&output.stdout);
    assert_eq!(lines.len(), 2);
    assert!(lines[1].starts_with("COPROC=") && lines[1].len() > "COPROC=".len());
}

#[test]
fn detached_shebang_and_cmd_batch_scripts_share_single_hidden_console() {
    let dir = std::env::temp_dir().join(format!("rubash-gate-shebang-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp shebang dir");

    let shebang = dir.join("run_shebang");
    std::fs::write(
        &shebang,
        "#!/usr/bin/env sh\nC:/Windows/System32/hostname.exe\n",
    )
    .expect("write shebang");

    let batch = dir.join("run_batch.cmd");
    std::fs::write(
        &batch,
        "@echo off\r\nC:\\Windows\\System32\\hostname.exe\r\n",
    )
    .expect("write batch");

    let dir_posix = dir.to_string_lossy().replace('\\', "/");
    let script = format!(
        "\"{dir_posix}/run_shebang\"; \"{dir_posix}/run_batch.cmd\"; C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; \"{dir_posix}/run_shebang\""
    );
    let (output, offenders, conhosts) = run_and_observe_rubash(&["-c", &script], DETACHED_PROCESS);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success());
    assert!(offenders.is_empty(), "visible windows: {offenders:?}");
    assert!(
        conhosts.len() <= 1,
        "shebang/batch must share <=1 conhost.exe, saw {}: {conhosts:?}",
        conhosts.len()
    );
    assert_eq!(non_empty_lines(&output.stdout).len(), 3);
}

#[test]
fn shim_forwarding_multi_command_reuses_single_hidden_console() {
    // Verify that `bash.exe` forwarder + forwarded shell + 5 external children
    // allocate at most 1 conhost.exe under both DETACHED_PROCESS and
    // CREATE_NO_WINDOW parents.
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let bash = env!("CARGO_BIN_EXE_bash");
    let script = concat!(
        "C:/Windows/System32/hostname.exe; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "C:/Windows/System32/hostname.exe; ",
        "C:/Windows/System32/ping.exe -n 2 127.0.0.1 >NUL; ",
        "C:/Windows/System32/hostname.exe"
    );
    for flags in [DETACHED_PROCESS, CREATE_NO_WINDOW] {
        let mut command = Command::new(bash);
        command
            .args(["-c", script])
            .env("NIU_SHELL", rubash)
            .creation_flags(flags | CREATE_NEW_PROCESS_GROUP)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn shim");
        let root_pid = child.id();
        let mut conhosts = std::collections::HashSet::new();
        let mut offenders = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            let entries = process_entries();
            let tree: HashMap<u32, u32> = entries.iter().map(|&(p, pp, _)| (p, pp)).collect();
            for &(pid, _ppid, ref name) in &entries {
                if name == "conhost.exe" && in_tree(pid, root_pid, &tree) {
                    conhosts.insert(pid);
                }
            }
            for owner in visible_console_window_owners() {
                if in_tree(owner, root_pid, &tree) && !offenders.contains(&owner) {
                    offenders.push(owner);
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let out = child.wait_with_output().expect("wait shim");
        assert!(out.status.success());
        assert!(
            offenders.is_empty(),
            "visible windows under shim (flags={flags:#x}): {offenders:?}"
        );
        assert!(
            conhosts.len() <= 1,
            "shim tree (flags={flags:#x}) must have <=1 conhost.exe, saw {}: {conhosts:?}",
            conhosts.len()
        );
        assert_eq!(non_empty_lines(&out.stdout).len(), 3);
    }
}
