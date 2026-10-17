//! `stty` console-mode builtin (niubash#282).
//!
//! GNU bash runs stty as an external command; on Windows there is no
//! termios — the console mode API is the equivalent surface. This engine
//! builtin maps the mode operands TUI scripts actually use onto
//! GetConsoleMode/SetConsoleMode on stdin's console handle:
//!
//! - `raw`      — per-character unreflected input: LINE_INPUT, ECHO_INPUT
//!                and PROCESSED_INPUT cleared (GNU raw turns off icanon,
//!                echo and isig; PROCESSED_INPUT is the isig carrier).
//! - `-raw` / `cooked` / `sane` — the canonical set restored: PROCESSED,
//!                LINE and ECHO set, everything else untouched.
//! - `echo` / `-echo` — ECHO_INPUT bit. The console couples ECHO to LINE
//!                (SetConsoleMode rejects ECHO without LINE), so `echo` on
//!                a raw terminal also restores LINE — the closest honest
//!                mapping of `stty echo`.
//! - `size`     — `rows cols` from the console screen buffer window.
//!
//! Operands apply left to right, one SetConsoleMode per operand. A
//! non-console standard input reports the GNU diagnostic and fails;
//! unknown operands report `invalid argument` and fail (coreutils stty
//! wording). Documented divergences: `-a`/`-g` and the per-flag termios
//! operands (`cs8`, `min 1`, ...) have no console-mode equivalent and are
//! reported as invalid arguments.
//!
//! The mode persists on the console handle after the builtin returns, so a
//! menu script's `stty raw -echo` stays in force for every later `read` —
//! and the read builtin's own mode guards save/restore AROUND it, so they
//! never clobber the script's chosen mode (niubash#280's per-read guards
//! capture the mode at read entry).
//!
//! On Unix the same operand set maps onto termios on stdin (`raw` clears
//! ICANON/ECHO/ISIG; `cooked`/`sane`/`-raw` restore them) and `size`
//! reports `rows cols` via TIOCGWINSZ (GNU coreutils stty behavior).

use crate::builtins::uname::IdentityToolOutput;

#[cfg(windows)]
const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
#[cfg(windows)]
const ENABLE_LINE_INPUT: u32 = 0x0002;
#[cfg(windows)]
const ENABLE_ECHO_INPUT: u32 = 0x0004;
#[cfg(windows)]
const CONSOLE_RAW_MASK: u32 = ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT;
#[cfg(windows)]
const CONSOLE_CANONICAL_MASK: u32 = ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT;

/// stty(1). Operands apply left to right; the first failure (bad operand or
/// a non-terminal stdin) stops the walk with status 1.
pub fn execute_stty(args: &[String]) -> IdentityToolOutput {
    #[cfg(windows)]
    {
        stty_windows(args)
    }
    #[cfg(unix)]
    {
        stty_unix(args)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = args;
        IdentityToolOutput {
            status: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

#[cfg(unix)]
fn stty_unix(args: &[String]) -> IdentityToolOutput {
    // GNU coreutils stty behavior (not bash source): a non-terminal
    // standard input fails every operand form, including bare `stty` and
    // `stty size`, with this diagnostic and status 1. isatty(0) is the
    // probe; no ioctl is needed for the diagnostic itself.
    let mut out = IdentityToolOutput {
        status: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    if unsafe { libc::isatty(0) } != 1 {
        out.stderr
            .extend_from_slice(b"stty: 'standard input': Inappropriate ioctl for device\n");
        out.status = 1;
        return out;
    }
    for arg in args {
        let outcome = match arg.as_str() {
            "size" => stty_size_unix(&mut out.stdout),
            "raw" => set_termios_lflag_bits(!(libc::ICANON | libc::ECHO | libc::ISIG)),
            "-raw" | "cooked" | "sane" => {
                set_termios_lflag_bits_add(libc::ICANON | libc::ECHO | libc::ISIG)
            }
            "-echo" => set_termios_lflag_bits(!libc::ECHO),
            "echo" => set_termios_lflag_bits_add(libc::ECHO),
            other => {
                out.stderr
                    .extend_from_slice(format!("stty: invalid argument '{other}'\n").as_bytes());
                out.status = 1;
                return out;
            }
        };
        if outcome.is_err() {
            out.stderr
                .extend_from_slice(b"stty: 'standard input': Inappropriate ioctl for device\n");
            out.status = 1;
            return out;
        }
    }
    out
}

#[cfg(unix)]
fn set_termios_lflag_bits(keep_mask: libc::tcflag_t) -> Result<(), ()> {
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(0, &mut term) } != 0 {
        return Err(());
    }
    term.c_lflag &= keep_mask;
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) } != 0 {
        return Err(());
    }
    Ok(())
}

#[cfg(unix)]
fn set_termios_lflag_bits_add(set_mask: libc::tcflag_t) -> Result<(), ()> {
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(0, &mut term) } != 0 {
        return Err(());
    }
    term.c_lflag |= set_mask;
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) } != 0 {
        return Err(());
    }
    Ok(())
}

#[cfg(unix)]
fn stty_size_unix(stdout: &mut Vec<u8>) -> Result<(), ()> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(0, libc::TIOCGWINSZ as _, &mut ws) } != 0 {
        return Err(());
    }
    stdout.extend_from_slice(format!("{} {}\n", ws.ws_row, ws.ws_col).as_bytes());
    Ok(())
}

#[cfg(windows)]
fn stty_windows(args: &[String]) -> IdentityToolOutput {
    let mut out = IdentityToolOutput {
        status: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let handle = crate::fd::process_std_handle(0);
    if handle == -1 || handle == 0 || !crate::fd::is_console_handle(handle) {
        out.stderr
            .extend_from_slice(b"stty: 'standard input': Inappropriate ioctl for device\n");
        out.status = 1;
        return out;
    }

    for arg in args {
        let outcome =
            match arg.as_str() {
                "size" => stty_size(&mut out.stdout),
                "raw" => set_console_mode_bits(handle, CONSOLE_RAW_MASK, false),
                "-raw" | "cooked" | "sane" => {
                    set_console_mode_bits(handle, CONSOLE_CANONICAL_MASK, true)
                }
                "-echo" => set_console_mode_bits(handle, ENABLE_ECHO_INPUT, false),
                "echo" => {
                    // The console couples ECHO to LINE: enable ECHO on a raw
                    // terminal restores canonical input with it.
                    set_console_mode_bits(handle, ENABLE_LINE_INPUT, true)
                        .and(set_console_mode_bits(handle, ENABLE_ECHO_INPUT, true))
                }
                other => {
                    out.stderr.extend_from_slice(
                        format!("stty: invalid argument '{other}'\n").as_bytes(),
                    );
                    out.status = 1;
                    return out;
                }
            };
        if outcome.is_err() {
            out.stderr
                .extend_from_slice(b"stty: 'standard input': Inappropriate ioctl for device\n");
            out.status = 1;
            return out;
        }
    }
    out
}

#[cfg(windows)]
fn set_console_mode_bits(handle: isize, mask: u32, set: bool) -> Result<(), ()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleMode(h: isize, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: isize, mode: u32) -> i32;
    }
    let mut mode: u32 = 0;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return Err(());
    }
    let updated = if set { mode | mask } else { mode & !mask };
    if unsafe { SetConsoleMode(handle, updated) } == 0 {
        return Err(());
    }
    Ok(())
}

#[cfg(windows)]
fn stty_size(stdout: &mut Vec<u8>) -> Result<(), ()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sa: *const core::ffi::c_void,
            disposition: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        fn GetConsoleScreenBufferInfo(h: isize, info: *mut ConsoleScreenBufferInfo) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    #[repr(C)]
    struct Coord {
        x: u16,
        y: u16,
    }
    #[repr(C)]
    struct SmallRect {
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    }
    #[repr(C)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;
    let mut wide: Vec<u16> = "CONOUT\0".encode_utf16().collect();
    let console_out = unsafe {
        CreateFileW(
            wide.as_mut_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if console_out == -1 || console_out == 0 {
        return Err(());
    }
    let mut info = unsafe { std::mem::zeroed::<ConsoleScreenBufferInfo>() };
    let ok = unsafe { GetConsoleScreenBufferInfo(console_out, &mut info) };
    unsafe {
        CloseHandle(console_out);
    }
    if ok == 0 {
        return Err(());
    }
    let rows = info.window.bottom as u32 - info.window.top as u32 + 1;
    let cols = info.window.right as u32 - info.window.left as u32 + 1;
    stdout.extend_from_slice(format!("{rows} {cols}\n").as_bytes());
    Ok(())
}
