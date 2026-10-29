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
//! - `icanon` / `-icanon` — LINE_INPUT; `isig` / `-isig` —
//!                PROCESSED_INPUT (the same carriers `raw`/`sane` use,
//!                matching the WinuxCmd applet's effective-setting set).
//! - `size`     — `rows cols` from the console screen buffer window.
//! - `cols` / `columns` / `rows` — the single geometry value (GNU query
//!   operands, served from the same screen-buffer query so the faces
//!   never disagree); followed by a number they are the window-size
//!   SETTING (`stty rows 40 cols 120`, GNU stty.c:1216-1234
//!   set_window_size): the console screen buffer is grown and the window
//!   resized so a following `stty size` reflects it.
//! - `speed`    — Windows: the nominal `38400` the bundled applet reports
//!   (the console has no baud rate; WinuxCmd #1430 family). Unix: the
//!   real cfgetospeed rate.
//! - a bare decimal operand is a baud setting (GNU stty.c:2178
//!   string_to_baud over the gnulib speed table): real
//!   cfsetispeed/cfsetospeed on Unix, accepted-and-ignored on Windows
//!   (the WinuxCmd model — validated for GNU compatibility so serial
//!   scripts do not break).
//! - `-a` / `--all`, and bare `stty` with no operands — the full
//!   human-readable dump in the WinuxCmd stty.cpp shape (owner-approved,
//!   real-console verified): GNU-default control characters (the console
//!   has no c_cc model), flag lines driven by the stdin console mode,
//!   real CONOUT$ geometry (the one substitution for the applet's
//!   hardcoded `rows 50; columns 120`). The Unix face fills the same
//!   line layout from real termios state.
//! - `-g` / `--save` — Unix: GNU gfmt (stty.c:2103 display_recoverable —
//!   `iflag:oflag:cflag:lflag` plus every c_cc word, lowercase hex).
//!   Windows, best-effort honest: one lowercase hex field holding the
//!   stdin console mode DWORD — the actual console state, NOT GNU's
//!   termios gfmt (there is no c_cc model to print), and there is no
//!   recover_mode reader on Windows, so `stty $(stty -g)` is not yet a
//!   restore face there.
//!
//! Parsing follows GNU main() (stty.c:1287): the option tokens `-a`/`-g`
//! (long forms `--all`/`--save`) are extracted from any position before
//! `--`, settings apply left to right, the two output styles are
//! mutually exclusive (stty.c:1379) and refuse setting operands
//! (stty.c:1385) with the GNU wording, and the operand list is validated
//! by a checking pass BEFORE the terminal is touched (stty.c:1392
//! apply_settings(checking=true)) — so `stty bogus < /dev/null` reports
//! the operand while `stty rows 40 < /dev/null` and `stty -a <
//! /dev/null` reach the ioctl diagnostic (probed byte-for-byte on WSL
//! GNU coreutils stty 9.11, LC_ALL=C).
//!
//! Documented divergences: the dump layout is the WinuxCmd subset (fixed
//! line breaks, no dynamic width wrapping, GNU-default c_cc values on
//! Windows, the `sane -icanon` summary line) rather than GNU
//! display_all's wrapped full tables — bare `stty` prints that dump too,
//! where GNU prints the changed-from-sane format; bare `rows`/`cols` are
//! queries here (GNU demands an argument); `-F`/`--file` has no face;
//! `ispeed`/`ospeed`/`min`/`time`/`line`/control-character operands and
//! the gfmt restore (`stty $(stty -g)`) are not admitted (invalid
//! argument) on either face; diagnostics keep the established one-error
//! wording, with GNU's `Try 'stty --help'` hint after invalid argument
//! and no hint after invalid integer (both probed). On macOS the baud
//! table stops at 38400: Darwin's libc provides no B57600+ constants,
//! so higher bare-decimal rates are invalid arguments there.
//!
//! The mode persists on the console handle after the builtin returns, so a
//! menu script's `stty raw -echo` stays in force for every later `read` —
//! and the read builtin's own mode guards save/restore AROUND it, so they
//! never clobber the script's chosen mode (niubash#280's per-read guards
//! capture the mode at read entry).
//!
//! On Unix the same operand set maps onto termios on stdin (`raw` clears
//! ICANON/ECHO/ISIG; `cooked`/`sane`/`-raw` restore them), `size`
//! reports `rows cols` via TIOCGWINSZ, and `rows N`/`cols N` store
//! through TIOCSWINSZ (GNU coreutils stty behavior).

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

/// GNU diagnostics, byte-for-byte as coreutils 9.11 emits them under
/// LC_ALL=C on a non-terminal stdin (probed on WSL):
/// - error(): `stty: 'standard input': Inappropriate ioctl for device`
/// - error()+usage(): `invalid argument 'X'` plus the `Try` hint
/// - xnumtoumax(): `invalid integer argument: 'X'` with no hint
/// - the two option-style refusals (stty.c:1379-1388), the first with
///   its embedded newline before `mutually exclusive`.
const ENOTTY_DIAGNOSTIC: &str = "stty: 'standard input': Inappropriate ioctl for device\n";
const TRY_HELP_HINT: &str = "Try 'stty --help' for more information.\n";

fn invalid_argument_diagnostic(token: &str) -> String {
    format!("stty: invalid argument '{token}'\n{TRY_HELP_HINT}")
}

fn invalid_integer_diagnostic(token: &str) -> String {
    format!("stty: invalid integer argument: '{token}'\n")
}

fn diagnostic_output(message: &str) -> IdentityToolOutput {
    IdentityToolOutput {
        status: 1,
        stdout: Vec::new(),
        stderr: message.as_bytes().to_vec(),
    }
}

fn enotty_output() -> IdentityToolOutput {
    diagnostic_output(ENOTTY_DIAGNOSTIC)
}

/// One recognized-or-rejected setting operand, after the option tokens
/// have been extracted (GNU stty.c:1101 apply_settings walks the same
/// grammar; the window-size setting keeps its value token unparsed until
/// the apply pass, exactly where GNU runs integer_arg — stty.c:1220
/// `if (checking) continue;` skips it, so a bad value still reaches the
/// ioctl diagnostic first on a non-terminal).
#[derive(Clone)]
enum Setting {
    Raw,
    Cooked,
    Sane,
    Echo(bool),
    Icanon(bool),
    Isig(bool),
    Size,
    Speed,
    /// `rows`/`cols`/`columns`. None = the bare query form rubash serves
    /// (a documented extension; GNU demands a value). Some(token) = the
    /// GNU window-size setting; the token is parsed at apply time.
    Rows(Option<String>),
    Cols(Option<String>),
    /// Bare decimal operand naming a gnulib speed-table baud.
    Baud(u32),
    /// No checking rule admits this token; reported verbatim by the
    /// checking pass (stty.c:1274).
    Invalid(String),
}

/// An invocation after option extraction: the two output-style flags plus
/// the settings, in order.
struct Invocation {
    all: bool,
    save: bool,
    settings: Vec<Setting>,
}

/// Parse `args` the way GNU main() does (stty.c:1287): a first getopt
/// sweep extracts `-a`/`-g` (long `--all`/`--save`) from ANY position,
/// clearing them out so the settings pass never sees them (stty.c:1374);
/// after `--` nothing is an option. The remaining tokens are settings,
/// left to right; a `rows`/`cols`/`columns` token consumes the next
/// token as its value unconditionally (stty.c:1105 check_argument).
fn parse_invocation(args: &[String]) -> Invocation {
    let mut all = false;
    let mut save = false;
    let mut end_of_options = false;
    let mut rest: Vec<&String> = Vec::new();
    for arg in args {
        if end_of_options {
            rest.push(arg);
            continue;
        }
        match arg.as_str() {
            "--" => end_of_options = true,
            "-a" | "--all" => all = true,
            "-g" | "--save" => save = true,
            _ => rest.push(arg),
        }
    }
    let mut settings = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        match arg {
            "raw" => settings.push(Setting::Raw),
            "-raw" | "cooked" => settings.push(Setting::Cooked),
            "sane" => settings.push(Setting::Sane),
            "echo" => settings.push(Setting::Echo(true)),
            "-echo" => settings.push(Setting::Echo(false)),
            "icanon" => settings.push(Setting::Icanon(true)),
            "-icanon" => settings.push(Setting::Icanon(false)),
            "isig" => settings.push(Setting::Isig(true)),
            "-isig" => settings.push(Setting::Isig(false)),
            "size" => settings.push(Setting::Size),
            "speed" => settings.push(Setting::Speed),
            "rows" | "cols" | "columns" => {
                let value = rest.get(i + 1).map(|s| (*s).clone());
                if value.is_some() {
                    i += 1;
                }
                if arg == "rows" {
                    settings.push(Setting::Rows(value));
                } else {
                    settings.push(Setting::Cols(value));
                }
            }
            other => match baud_value(other) {
                Some(baud) => settings.push(Setting::Baud(baud)),
                None => settings.push(Setting::Invalid(other.to_string())),
            },
        }
        i += 1;
    }
    Invocation {
        all,
        save,
        settings,
    }
}

/// GNU string_to_baud (stty.c:2178) over the gnulib speed table; `134.5`
/// maps to B134 exactly as the table does.
fn baud_value(token: &str) -> Option<u32> {
    Some(match token {
        "0" => 0,
        "50" => 50,
        "75" => 75,
        "110" => 110,
        "134" | "134.5" => 134,
        "150" => 150,
        "200" => 200,
        "300" => 300,
        "600" => 600,
        "1200" => 1200,
        "1800" => 1800,
        "2400" => 2400,
        "4800" => 4800,
        "9600" => 9600,
        "19200" => 19200,
        "38400" => 38400,
        "57600" => 57600,
        "115200" => 115200,
        "230400" => 230400,
        "460800" => 460800,
        "500000" => 500000,
        "576000" => 576000,
        "921600" => 921600,
        "1000000" => 1000000,
        "1152000" => 1152000,
        "1500000" => 1500000,
        "2000000" => 2000000,
        "2500000" => 2500000,
        "3000000" => 3000000,
        "3500000" => 3500000,
        "4000000" => 4000000,
        _ => return None,
    })
}

/// GNU integer_arg (stty.c:2326 xnumtoumax, base 0, max INT_MAX): leading
/// whitespace and `+` allowed, `0x` hex / leading-`0` octal / decimal,
/// negatives and overflow rejected — `stty: invalid integer argument`.
fn integer_arg(token: &str) -> Option<u32> {
    let token = token.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let unsigned = match token.strip_prefix('-') {
        Some(_) => return None,
        None => token.strip_prefix('+').unwrap_or(token),
    };
    if unsigned.is_empty() {
        return None;
    }
    let (digits, radix) = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (hex, 16)
    } else if unsigned.len() > 1 && unsigned.starts_with('0') {
        (&unsigned[1..], 8)
    } else {
        (unsigned, 10)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let value = u64::from_str_radix(digits, radix).ok()?;
    if value > i32::MAX as u64 {
        return None;
    }
    Some(value as u32)
}

/// stty(1). Options and settings may appear in any order; the first
/// failure (bad operand, bad value, or a non-terminal stdin) stops with
/// status 1.
pub fn execute_stty(args: &[String]) -> IdentityToolOutput {
    let invocation = parse_invocation(args);
    // GNU stty.c:1379: the output styles are mutually exclusive.
    if invocation.all && invocation.save {
        return diagnostic_output(
            "stty: the options for verbose and stty-readable output styles are\nmutually exclusive\n",
        );
    }
    // GNU stty.c:1385: an output style refuses setting operands.
    if (invocation.all || invocation.save) && !invocation.settings.is_empty() {
        return diagnostic_output("stty: when specifying an output style, modes may not be set\n");
    }
    // GNU stty.c:1392: the checking pass validates every operand BEFORE
    // the terminal is touched, so an unknown operand is reported even
    // when stdin is a pipe while a well-formed one falls through to the
    // ioctl diagnostic.
    for setting in &invocation.settings {
        if let Setting::Invalid(token) = setting {
            return diagnostic_output(&invalid_argument_diagnostic(token));
        }
    }
    #[cfg(windows)]
    {
        stty_windows(&invocation)
    }
    #[cfg(unix)]
    {
        stty_unix(&invocation)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = invocation;
        IdentityToolOutput {
            status: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------
// Windows console-mode face
// ---------------------------------------------------------------------

#[cfg(windows)]
fn stty_windows(invocation: &Invocation) -> IdentityToolOutput {
    let mut out = IdentityToolOutput {
        status: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let handle = crate::fd::process_std_handle(0);
    if handle == -1 || handle == 0 || !crate::fd::is_console_handle(handle) {
        return enotty_output();
    }

    // Display faces: -g, and -a / bare `stty` (the WinuxCmd applet shows
    // the full dump for a bare invocation too).
    if invocation.save {
        if let Err(()) = save_console_mode(handle, &mut out.stdout) {
            return enotty_output();
        }
        return out;
    }
    if invocation.all || invocation.settings.is_empty() {
        if let Err(()) = display_all_windows(handle, &mut out.stdout) {
            return enotty_output();
        }
        return out;
    }

    for setting in &invocation.settings {
        let outcome = match setting {
            Setting::Raw => set_console_mode_bits(handle, CONSOLE_RAW_MASK, false),
            Setting::Cooked | Setting::Sane => {
                set_console_mode_bits(handle, CONSOLE_CANONICAL_MASK, true)
            }
            Setting::Echo(true) => {
                // The console couples ECHO to LINE: enabling ECHO on a raw
                // terminal restores canonical input with it.
                set_console_mode_bits(handle, ENABLE_LINE_INPUT, true).and(set_console_mode_bits(
                    handle,
                    ENABLE_ECHO_INPUT,
                    true,
                ))
            }
            Setting::Echo(false) => set_console_mode_bits(handle, ENABLE_ECHO_INPUT, false),
            Setting::Icanon(value) => set_console_mode_bits(handle, ENABLE_LINE_INPUT, *value),
            Setting::Isig(value) => set_console_mode_bits(handle, ENABLE_PROCESSED_INPUT, *value),
            Setting::Size => stty_size(&mut out.stdout),
            Setting::Speed => {
                // The console has no baud rate; report the nominal speed
                // the bundled applet reports (WinuxCmd #1430 family) as a
                // value rather than silence.
                out.stdout.extend_from_slice(b"38400\n");
                Ok(())
            }
            Setting::Rows(None) => stty_dimension(&mut out.stdout, true),
            Setting::Cols(None) => stty_dimension(&mut out.stdout, false),
            Setting::Rows(Some(token)) => match integer_arg(token) {
                Some(rows) => console_resize(Some(rows), None),
                None => return diagnostic_output(&invalid_integer_diagnostic(token)),
            },
            Setting::Cols(Some(token)) => match integer_arg(token) {
                Some(cols) => console_resize(None, Some(cols)),
                None => return diagnostic_output(&invalid_integer_diagnostic(token)),
            },
            // Validated by the checking pass (GNU string_to_baud table);
            // the console has no baud rate to set, so like the WinuxCmd
            // applet the value is accepted and ignored.
            Setting::Baud(_) => Ok(()),
            Setting::Invalid(_) => Ok(()), // unreachable: checking pass ran
        };
        if outcome.is_err() {
            return enotty_output();
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
fn get_console_mode(handle: isize) -> Result<u32, ()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleMode(h: isize, mode: *mut u32) -> i32;
    }
    let mut mode: u32 = 0;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return Err(());
    }
    Ok(mode)
}

/// `-g`, Windows face: the honest console state is the stdin console mode
/// DWORD, printed as one lowercase hex field. This is NOT GNU termios
/// gfmt (the console keeps no c_cc model) and nothing reads it back yet.
#[cfg(windows)]
fn save_console_mode(handle: isize, stdout: &mut Vec<u8>) -> Result<(), ()> {
    let mode = get_console_mode(handle)?;
    stdout.extend_from_slice(format!("{mode:x}\n").as_bytes());
    Ok(())
}

/// `-a` / bare `stty`, Windows face: the owner-approved WinuxCmd stty.cpp
/// dump shape — GNU-default control characters, the min line and the
/// icrnl/echo/echok/echoctl/echoke flags driven by the stdin console
/// mode, fixed output-flag line, and REAL CONOUT$ geometry on the speed
/// line (the one substitution for the applet's hardcoded 50x120).
#[cfg(windows)]
fn display_all_windows(handle: isize, stdout: &mut Vec<u8>) -> Result<(), ()> {
    let mode = get_console_mode(handle)?;
    let (rows, cols) = console_window_rows_cols()?;
    let processed = mode & ENABLE_PROCESSED_INPUT != 0;
    let echo = mode & ENABLE_ECHO_INPUT != 0;
    let line = mode & ENABLE_LINE_INPUT != 0;

    // GNU-default control characters (the console has no c_cc model).
    stdout.extend_from_slice(
        b"intr = ^C; quit = ^\\; erase = ^?; kill = ^U; eof = ^D; eol = <undef>; \
eol2 = <undef>; swtch = <undef>; start = ^Q; stop = ^S; susp = ^Z; rprnt = ^R; \
werase = ^W; lnext = ^V; discard = ^O;\n",
    );
    stdout.extend_from_slice(format!("min = {}; time = 0;\n", u32::from(processed)).as_bytes());
    stdout.extend_from_slice(
        format!("speed 38400 baud; rows {rows}; columns {cols}; line = 0;\n").as_bytes(),
    );
    // Input flags: PROCESSED_INPUT carries ignbrk/icrnl in the applet's
    // mapping; the rest of the line is the GNU-sane negatives.
    let input_line: &[u8] = if processed {
        b" -ignbrk -brkint -parmrk -istrip -inlcr -igncr icrnl -ixon -ixoff -iuclc -ixany -imaxbel -iutf8\n"
    } else {
        b" ignbrk -brkint -parmrk -istrip -inlcr -igncr -icrnl -ixon -ixoff -iuclc -ixany -imaxbel -iutf8\n"
    };
    stdout.extend_from_slice(input_line);
    stdout.extend_from_slice(
        b" -opost -olcuc -ocrnl -onlcr -onocr -onlret -ofill -ofdel nl0 cr0 tab0 bs0 vt0 ff0\n",
    );
    // Local flags: ECHO_INPUT carries echo/echoctl/echoke, LINE_INPUT
    // carries echok.
    let mut local = String::new();
    local.push_str(if echo { " echo" } else { " -echo" });
    local.push_str(" -echoe");
    local.push_str(if line { " echok" } else { " -echok" });
    local.push_str(" -echonl -noflsh -tostop");
    local.push_str(if echo { " echoctl" } else { " -echoctl" });
    local.push_str(" -echoprt");
    local.push_str(if echo { " echoke" } else { " -echoke" });
    local.push_str(" -flusho -extproc\n");
    stdout.extend_from_slice(local.as_bytes());
    // The applet's summary line (its authors treat the console as
    // non-canonical for dump purposes; owner-approved verbatim).
    stdout.extend_from_slice(b"sane -icanon\n");
    Ok(())
}

#[cfg(windows)]
#[repr(C)]
struct Coord {
    x: u16,
    y: u16,
}

#[cfg(windows)]
#[repr(C)]
struct SmallRect {
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

#[cfg(windows)]
#[repr(C)]
struct ConsoleScreenBufferInfo {
    size: Coord,
    #[allow(dead_code)]
    cursor_position: Coord,
    #[allow(dead_code)]
    attributes: u16,
    window: SmallRect,
    #[allow(dead_code)]
    maximum_window_size: Coord,
}

/// Open CONOUT$ and return the screen-buffer info, shared by the geometry
/// query and the window-size setting.
#[cfg(windows)]
fn console_screen_buffer_info() -> Result<ConsoleScreenBufferInfo, ()> {
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
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;
    let mut wide: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
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
    Ok(info)
}

/// (rows, cols) of the console window, queried on a fresh `CONOUT$`
/// handle — the reserved console-output device name and the documented
/// screen-buffer handle. niubash#535: this once opened `"CONOUT"`, a
/// plain relative file name rather than the reserved device, so the
/// open failed on every console and the size face reported the
/// non-terminal diagnostic even in a real terminal.
#[cfg(windows)]
fn console_window_rows_cols() -> Result<(u32, u32), ()> {
    let info = console_screen_buffer_info()?;
    let rows = info.window.bottom as u32 - info.window.top as u32 + 1;
    let cols = info.window.right as u32 - info.window.left as u32 + 1;
    Ok((rows, cols))
}

/// GNU set_window_size (stty.c:1751): patch only the dimension(s) given.
/// The console needs the screen buffer to grow around the window before
/// the window can grow into it, so the buffer is grown (never shrunk —
/// the cursor must stay inside it) first and the window is set second;
/// a later `stty size` reads the window and reflects the setting.
#[cfg(windows)]
fn console_resize(rows: Option<u32>, cols: Option<u32>) -> Result<(), ()> {
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
        fn SetConsoleScreenBufferSize(h: isize, size: Coord) -> i32;
        fn SetConsoleWindowInfo(h: isize, absolute: i32, window: *const SmallRect) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;
    let mut wide: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
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
    if unsafe { GetConsoleScreenBufferInfo(console_out, &mut info) } == 0 {
        unsafe {
            CloseHandle(console_out);
        }
        return Err(());
    }
    let cur_rows = info.window.bottom as u32 - info.window.top as u32 + 1;
    let cur_cols = info.window.right as u32 - info.window.left as u32 + 1;
    let new_rows = rows.unwrap_or(cur_rows).min(u16::MAX as u32);
    let new_cols = cols.unwrap_or(cur_cols).min(u16::MAX as u32);
    // Grow the buffer around the requested window; never shrink it (the
    // cursor must stay inside).
    let buf_x = info.size.x.max(new_cols as u16);
    let buf_y = info.size.y.max(new_rows as u16);
    if buf_x != info.size.x || buf_y != info.size.y {
        if unsafe { SetConsoleScreenBufferSize(console_out, Coord { x: buf_x, y: buf_y }) } == 0 {
            unsafe {
                CloseHandle(console_out);
            }
            return Err(());
        }
    }
    let window = SmallRect {
        left: 0,
        top: 0,
        right: new_cols.min(buf_x as u32).saturating_sub(1) as u16,
        bottom: new_rows.min(buf_y as u32).saturating_sub(1) as u16,
    };
    let ok = unsafe { SetConsoleWindowInfo(console_out, 1, &window) };
    unsafe {
        CloseHandle(console_out);
    }
    if ok == 0 {
        return Err(());
    }
    Ok(())
}

#[cfg(windows)]
fn stty_size(stdout: &mut Vec<u8>) -> Result<(), ()> {
    let (rows, cols) = console_window_rows_cols()?;
    stdout.extend_from_slice(format!("{rows} {cols}\n").as_bytes());
    Ok(())
}

#[cfg(windows)]
fn stty_dimension(stdout: &mut Vec<u8>, rows: bool) -> Result<(), ()> {
    let (row_count, col_count) = console_window_rows_cols()?;
    let value = if rows { row_count } else { col_count };
    stdout.extend_from_slice(format!("{value}\n").as_bytes());
    Ok(())
}

// ---------------------------------------------------------------------
// Unix termios face
// ---------------------------------------------------------------------

#[cfg(unix)]
fn stty_unix(invocation: &Invocation) -> IdentityToolOutput {
    let mut out = IdentityToolOutput {
        status: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    // GNU coreutils stty behavior (not bash source): a non-terminal
    // standard input fails every operand form, including bare `stty` and
    // `stty size`, with this diagnostic and status 1. isatty(0) is the
    // probe; no ioctl is needed for the diagnostic itself.
    if unsafe { libc::isatty(0) } != 1 {
        return enotty_output();
    }

    // Display faces (GNU main stty.c:1413: -a, -g and the bare
    // invocation all display; our bare face shows the -a dump, where GNU
    // shows the changed-from-sane format — documented above).
    if invocation.save {
        return match display_recoverable_unix(&mut out.stdout) {
            Ok(()) => out,
            Err(()) => enotty_output(),
        };
    }
    if invocation.all || invocation.settings.is_empty() {
        return match display_all_unix(&mut out.stdout) {
            Ok(()) => out,
            Err(()) => enotty_output(),
        };
    }

    for setting in &invocation.settings {
        let outcome = match setting {
            Setting::Raw => set_termios_lflag_bits(!(libc::ICANON | libc::ECHO | libc::ISIG)),
            Setting::Cooked | Setting::Sane => {
                set_termios_lflag_bits_add(libc::ICANON | libc::ECHO | libc::ISIG)
            }
            Setting::Echo(true) => set_termios_lflag_bits_add(libc::ECHO),
            Setting::Echo(false) => set_termios_lflag_bits(!libc::ECHO),
            Setting::Icanon(true) => set_termios_lflag_bits_add(libc::ICANON),
            Setting::Icanon(false) => set_termios_lflag_bits(!libc::ICANON),
            Setting::Isig(true) => set_termios_lflag_bits_add(libc::ISIG),
            Setting::Isig(false) => set_termios_lflag_bits(!libc::ISIG),
            Setting::Size => stty_size_unix(&mut out.stdout),
            Setting::Speed => stty_speed_unix(&mut out.stdout),
            Setting::Rows(None) => stty_dimension_unix(&mut out.stdout, true),
            Setting::Cols(None) => stty_dimension_unix(&mut out.stdout, false),
            Setting::Rows(Some(token)) => match integer_arg(token) {
                Some(rows) => set_window_size_unix(Some(rows), None),
                None => return diagnostic_output(&invalid_integer_diagnostic(token)),
            },
            Setting::Cols(Some(token)) => match integer_arg(token) {
                Some(cols) => set_window_size_unix(None, Some(cols)),
                None => return diagnostic_output(&invalid_integer_diagnostic(token)),
            },
            Setting::Baud(value) => set_baud_unix(*value),
            Setting::Invalid(_) => Ok(()), // unreachable: checking pass ran
        };
        if outcome.is_err() {
            return enotty_output();
        }
    }
    out
}

#[cfg(unix)]
fn current_termios() -> Result<libc::termios, ()> {
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(0, &mut term) } != 0 {
        return Err(());
    }
    Ok(term)
}

#[cfg(unix)]
fn set_termios_lflag_bits(keep_mask: libc::tcflag_t) -> Result<(), ()> {
    let mut term = current_termios()?;
    term.c_lflag &= keep_mask;
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) } != 0 {
        return Err(());
    }
    Ok(())
}

#[cfg(unix)]
fn set_termios_lflag_bits_add(set_mask: libc::tcflag_t) -> Result<(), ()> {
    let mut term = current_termios()?;
    term.c_lflag |= set_mask;
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) } != 0 {
        return Err(());
    }
    Ok(())
}

/// GNU set_speed (stty.c:1717) for the bare-decimal baud setting: real
/// cfsetispeed/cfsetospeed over the termios on stdin.
#[cfg(unix)]
fn set_baud_unix(value: u32) -> Result<(), ()> {
    let baud = baud_to_const(value)?;
    let mut term = current_termios()?;
    if unsafe { libc::cfsetispeed(&mut term, baud) } != 0
        || unsafe { libc::cfsetospeed(&mut term, baud) } != 0
    {
        return Err(());
    }
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) } != 0 {
        return Err(());
    }
    Ok(())
}

/// Map a numeric baud to the Bxxx speed_t constant (the gnulib speed
/// table's right-hand column).
#[cfg(unix)]
fn baud_to_const(value: u32) -> Result<libc::speed_t, ()> {
    match value {
        0 => Ok(libc::B0),
        50 => Ok(libc::B50),
        75 => Ok(libc::B75),
        110 => Ok(libc::B110),
        134 => Ok(libc::B134),
        150 => Ok(libc::B150),
        200 => Ok(libc::B200),
        300 => Ok(libc::B300),
        600 => Ok(libc::B600),
        1200 => Ok(libc::B1200),
        1800 => Ok(libc::B1800),
        2400 => Ok(libc::B2400),
        4800 => Ok(libc::B4800),
        9600 => Ok(libc::B9600),
        19200 => Ok(libc::B19200),
        38400 => Ok(libc::B38400),
        // The extended Bxxx constants (B57600 and up) exist in the
        // Linux/Android libc only — Darwin's libc stops at B38400, so
        // higher bare-decimal rates are invalid arguments there
        // (documented divergence, module header).
        #[cfg(any(target_os = "linux", target_os = "android"))]
        57600 => Ok(libc::B57600),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        115200 => Ok(libc::B115200),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        230400 => Ok(libc::B230400),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        460800 => Ok(libc::B460800),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        500000 => Ok(libc::B500000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        576000 => Ok(libc::B576000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        921600 => Ok(libc::B921600),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        1000000 => Ok(libc::B1000000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        1152000 => Ok(libc::B1152000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        1500000 => Ok(libc::B1500000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        2000000 => Ok(libc::B2000000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        2500000 => Ok(libc::B2500000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        3000000 => Ok(libc::B3000000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        3500000 => Ok(libc::B3500000),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        4000000 => Ok(libc::B4000000),
        _ => Err(()),
    }
}

/// Decode a cfget[io]speed constant back to the numeric baud
/// (GNU baud_to_value): on Linux the Bxxx constants are the encoded
/// speed_t values, so the reverse table is the same list.
#[cfg(unix)]
fn baud_from_const(baud: libc::speed_t) -> Option<u32> {
    Some(match baud {
        libc::B0 => 0,
        libc::B50 => 50,
        libc::B75 => 75,
        libc::B110 => 110,
        libc::B134 => 134,
        libc::B150 => 150,
        libc::B200 => 200,
        libc::B300 => 300,
        libc::B600 => 600,
        libc::B1200 => 1200,
        libc::B1800 => 1800,
        libc::B2400 => 2400,
        libc::B4800 => 4800,
        libc::B9600 => 9600,
        libc::B19200 => 19200,
        libc::B38400 => 38400,
        // Extended constants: Linux/Android libc only (see baud_to_const).
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B57600 => 57600,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B115200 => 115200,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B230400 => 230400,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B460800 => 460800,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B500000 => 500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B576000 => 576000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B921600 => 921600,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B1000000 => 1000000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B1152000 => 1152000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B1500000 => 1500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B2000000 => 2000000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B2500000 => 2500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B3000000 => 3000000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B3500000 => 3500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B4000000 => 4000000,
        _ => return None,
    })
}

#[cfg(unix)]
fn terminal_winsize() -> Result<libc::winsize, ()> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(0, libc::TIOCGWINSZ as _, &mut ws) } != 0 {
        return Err(());
    }
    Ok(ws)
}

/// GNU set_window_size (stty.c:1751): TIOCSWINSZ patching only the
/// dimension(s) given.
#[cfg(unix)]
fn set_window_size_unix(rows: Option<u32>, cols: Option<u32>) -> Result<(), ()> {
    let mut ws = terminal_winsize()?;
    if let Some(rows) = rows {
        ws.ws_row = rows as u16;
    }
    if let Some(cols) = cols {
        ws.ws_col = cols as u16;
    }
    if unsafe { libc::ioctl(0, libc::TIOCSWINSZ as _, &ws) } != 0 {
        return Err(());
    }
    Ok(())
}

#[cfg(unix)]
fn stty_size_unix(stdout: &mut Vec<u8>) -> Result<(), ()> {
    let ws = terminal_winsize()?;
    stdout.extend_from_slice(format!("{} {}\n", ws.ws_row, ws.ws_col).as_bytes());
    Ok(())
}

/// GNU display_speed non-fancy (stty.c:2088): the decoded cfgetospeed
/// rate, one number.
#[cfg(unix)]
fn stty_speed_unix(stdout: &mut Vec<u8>) -> Result<(), ()> {
    let term = current_termios()?;
    let speed = unsafe { libc::cfgetospeed(&term) };
    let value = baud_from_const(speed).ok_or(())?;
    stdout.extend_from_slice(format!("{value}\n").as_bytes());
    Ok(())
}

#[cfg(unix)]
fn stty_dimension_unix(stdout: &mut Vec<u8>, rows: bool) -> Result<(), ()> {
    let ws = terminal_winsize()?;
    let value = if rows { ws.ws_row } else { ws.ws_col };
    stdout.extend_from_slice(format!("{value}\n").as_bytes());
    Ok(())
}

/// GNU visible() (stty.c): `<undef>` for the disable value, `^X` below
/// 0x20, `^?` for 0x7f, `M-` prefix above 0x7f, the character itself
/// otherwise.
#[cfg(unix)]
fn visible_cc(cc: u8) -> String {
    if cc == libc::_POSIX_VDISABLE as u8 {
        return "<undef>".to_string();
    }
    let mut out = String::new();
    let mut ch = cc;
    if ch >= 0x80 {
        out.push_str("M-");
        ch -= 0x80;
    }
    if ch < 0x20 {
        out.push('^');
        // ch < 0x20 here, so ch + 0x40 lands in 0x40..=0x5F: ASCII by
        // construction, never a UTF-8 continuation byte.
        out.push(char::from(ch + 0x40));
    } else if ch == 0x7f {
        out.push_str("^?");
    } else {
        // The >= 0x80 band was stripped above and the branches exclude
        // < 0x20 and 0x7F, so ch is in 0x20..=0x7E: ASCII printable.
        out.push(char::from(ch));
    }
    out
}

#[cfg(unix)]
fn cc_value(term: &libc::termios, offset: u32) -> String {
    visible_cc(term.c_cc[offset as usize])
}

/// Render ` name` / ` -name` entries for one flag word, WinuxCmd line
/// style (leading space per entry).
#[cfg(unix)]
fn flag_line(entries: &[(&str, libc::tcflag_t, libc::tcflag_t)], word: libc::tcflag_t) -> String {
    let mut line = String::new();
    for (name, mask, bits) in entries {
        let on = (word & mask) == *bits;
        line.push(' ');
        if !on {
            line.push('-');
        }
        line.push_str(name);
    }
    line
}

/// `-a` / bare `stty`, Unix face: the WinuxCmd line layout (the same
/// shape the Windows face prints) filled from real termios state — real
/// c_cc control characters, cfgetospeed, TIOCGWINSZ geometry and the
/// iflag/oflag/lflag bits.
#[cfg(unix)]
fn display_all_unix(stdout: &mut Vec<u8>) -> Result<(), ()> {
    let term = current_termios()?;
    let ws = terminal_winsize()?;
    let speed = unsafe { libc::cfgetospeed(&term) };
    let ispeed = unsafe { libc::cfgetispeed(&term) };

    // Control characters (GNU control_info order).
    let controls: [(&str, u32); 15] = [
        ("intr", libc::VINTR as u32),
        ("quit", libc::VQUIT as u32),
        ("erase", libc::VERASE as u32),
        ("kill", libc::VKILL as u32),
        ("eof", libc::VEOF as u32),
        ("eol", libc::VEOL as u32),
        ("eol2", libc::VEOL2 as u32),
        ("swtch", libc::VSWTC as u32),
        ("start", libc::VSTART as u32),
        ("stop", libc::VSTOP as u32),
        ("susp", libc::VSUSP as u32),
        ("rprnt", libc::VREPRINT as u32),
        ("werase", libc::VWERASE as u32),
        ("lnext", libc::VLNEXT as u32),
        ("discard", libc::VDISCARD as u32),
    ];
    let control_line: String = controls
        .iter()
        .map(|(name, offset)| format!("{name} = {};", cc_value(&term, *offset)))
        .collect::<Vec<_>>()
        .join(" ");
    stdout.extend_from_slice(control_line.as_bytes());
    stdout.push(b'\n');
    stdout.extend_from_slice(
        format!(
            "min = {}; time = {};\n",
            term.c_cc[libc::VMIN as usize],
            term.c_cc[libc::VTIME as usize]
        )
        .as_bytes(),
    );
    // Speed line: GNU display_speed fancy wording (equal or zero input
    // speed collapses to one `speed`), real geometry and line discipline.
    let speed_text = if ispeed == 0 || ispeed == speed {
        format!("speed {} baud;", baud_from_const(speed).unwrap_or(0))
    } else {
        format!(
            "ispeed {} baud; ospeed {} baud;",
            baud_from_const(ispeed).unwrap_or(0),
            baud_from_const(speed).unwrap_or(0)
        )
    };
    stdout.extend_from_slice(
        format!(
            "{speed_text} rows {}; columns {}; line = {};\n",
            ws.ws_row, ws.ws_col, term.c_line
        )
        .as_bytes(),
    );

    // Input flags, WinuxCmd line order.
    let input: [(&str, libc::tcflag_t, libc::tcflag_t); 12] = [
        ("ignbrk", libc::IGNBRK, libc::IGNBRK),
        ("brkint", libc::BRKINT, libc::BRKINT),
        ("parmrk", libc::PARMRK, libc::PARMRK),
        ("istrip", libc::ISTRIP, libc::ISTRIP),
        ("inlcr", libc::INLCR, libc::INLCR),
        ("igncr", libc::IGNCR, libc::IGNCR),
        ("icrnl", libc::ICRNL, libc::ICRNL),
        ("ixon", libc::IXON, libc::IXON),
        ("ixoff", libc::IXOFF, libc::IXOFF),
        ("iuclc", libc::IUCLC, libc::IUCLC),
        ("ixany", libc::IXANY, libc::IXANY),
        ("imaxbel", libc::IMAXBEL, libc::IMAXBEL),
    ];
    let mut input_line = flag_line(&input, term.c_iflag);
    input_line.push_str(if term.c_iflag & libc::IUTF8 != 0 {
        " iutf8\n"
    } else {
        " -iutf8\n"
    });
    stdout.extend_from_slice(input_line.as_bytes());
    // Output flags; the delay masks are fixed at 0 like the applet.
    let output: [(&str, libc::tcflag_t, libc::tcflag_t); 8] = [
        ("opost", libc::OPOST, libc::OPOST),
        ("olcuc", libc::OLCUC, libc::OLCUC),
        ("ocrnl", libc::OCRNL, libc::OCRNL),
        ("onlcr", libc::ONLCR, libc::ONLCR),
        ("onocr", libc::ONOCR, libc::ONOCR),
        ("onlret", libc::ONLRET, libc::ONLRET),
        ("ofill", libc::OFILL, libc::OFILL),
        ("ofdel", libc::OFDEL, libc::OFDEL),
    ];
    let mut output_line = flag_line(&output, term.c_oflag);
    output_line.push_str(" nl0 cr0 tab0 bs0 vt0 ff0\n");
    stdout.extend_from_slice(output_line.as_bytes());
    // Local flags (icanon is reported on the summary line, like the
    // applet's `sane -icanon`).
    let local: [(&str, libc::tcflag_t, libc::tcflag_t); 11] = [
        ("echo", libc::ECHO, libc::ECHO),
        ("echoe", libc::ECHOE, libc::ECHOE),
        ("echok", libc::ECHOK, libc::ECHOK),
        ("echonl", libc::ECHONL, libc::ECHONL),
        ("noflsh", libc::NOFLSH, libc::NOFLSH),
        ("tostop", libc::TOSTOP, libc::TOSTOP),
        ("echoctl", libc::ECHOCTL, libc::ECHOCTL),
        ("echoprt", libc::ECHOPRT, libc::ECHOPRT),
        ("echoke", libc::ECHOKE, libc::ECHOKE),
        ("flusho", libc::FLUSHO, libc::FLUSHO),
        ("extproc", libc::EXTPROC, libc::EXTPROC),
    ];
    let mut local_line = flag_line(&local, term.c_lflag);
    local_line.push('\n');
    stdout.extend_from_slice(local_line.as_bytes());
    let summary: &[u8] = if term.c_lflag & libc::ICANON != 0 {
        b"sane icanon\n"
    } else {
        b"sane -icanon\n"
    };
    stdout.extend_from_slice(summary);
    Ok(())
}

/// `-g`, Unix face: GNU display_recoverable (stty.c:2103) — lowercase
/// hex `iflag:oflag:cflag:lflag` plus every c_cc word.
#[cfg(unix)]
fn display_recoverable_unix(stdout: &mut Vec<u8>) -> Result<(), ()> {
    let term = current_termios()?;
    let mut line = format!(
        "{:x}:{:x}:{:x}:{:x}",
        term.c_iflag, term.c_oflag, term.c_cflag, term.c_lflag
    );
    for cc in term.c_cc {
        line.push_str(&format!(":{cc:x}"));
    }
    line.push('\n');
    stdout.extend_from_slice(line.as_bytes());
    Ok(())
}
