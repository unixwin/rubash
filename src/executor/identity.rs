//! Engine identity persona (rubash#154).
//!
//! The engine presents a selectable platform identity. Two personas exist:
//!
//! * [`ShellIdentity::Msys`] (Windows default) — an MSYS2-compatible
//!   identity: `uname -s` reports `MSYS_NT-<ver>` (or the `MSYSTEM`-derived
//!   `MINGW64_NT-` / `UCRT64_NT-` / ... prefix), `uname -o` reports `Msys`,
//!   and `OSTYPE=msys` / `MACHTYPE=<arch>-pc-msys`. This is the default
//!   because the wider shell ecosystem gates on it: bash-completion and many
//!   installers branch on `uname -s` matching `MINGW*|MSYS*|CYGWIN*` or on
//!   `$OSTYPE` being `msys`/`cygwin`, and exit out on anything else.
//! * [`ShellIdentity::Native`] (opt-in via `RUBASH_IDENTITY=native`) — the
//!   honest-native identity: `OSTYPE=windows`,
//!   `MACHTYPE=<arch>-pc-windows`, `uname -s` reports `Windows_NT` (the
//!   native `%OS%` value). Tests and native-first users select this.
//!
//! GNU anchor: bash's `OSTYPE`/`MACHTYPE`/`HOSTTYPE` are configure-time
//! build constants bound with `set_if_not` at startup
//! (third_party/bash/variables.c:723-725) — an inherited value wins, which
//! is why the persona only chooses the *default* injected value. The uname
//! surface follows uname(1)/arch(1) semantics with the MSYS2 output forms as
//! the reference shape (Git Bash probe 2026-09-26: `-s` MINGW64_NT-10.0-
//! 19044, `-o` Msys, `-m` x86_64, `arch` == `uname -m`).

/// Version the engine-built uname/arch report in their `--version` banner.
pub const ENGINE_BUILTIN_VERSION: &str = "5.3.0";

/// Which platform identity the engine currently presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIdentity {
    /// MSYS2-compatible persona (Windows default).
    Msys,
    /// Honest-native persona (`RUBASH_IDENTITY=native`).
    Native,
}

/// Read the effective persona: `RUBASH_IDENTITY=native` (case-insensitive)
/// selects [`ShellIdentity::Native`]; every other value — and every other
/// platform — stays on the default. Non-Windows targets are always native:
/// the MSYS persona exists to keep Windows inside the MSYS/Cygwin script
/// ecosystem, which is meaningless elsewhere.
pub fn current_identity() -> ShellIdentity {
    if !cfg!(windows) {
        return ShellIdentity::Native;
    }
    match std::env::var("RUBASH_IDENTITY") {
        Ok(value) if value.trim().eq_ignore_ascii_case("native") => ShellIdentity::Native,
        _ => ShellIdentity::Msys,
    }
}

/// `$OSTYPE` default (variables.c:724 `set_if_not ("OSTYPE", OSTYPE)`):
/// `msys` under the MSYS persona, `windows` under the native persona.
pub fn ostype() -> String {
    if !cfg!(windows) {
        return std::env::consts::OS.to_string();
    }
    match current_identity() {
        ShellIdentity::Msys => "msys".to_string(),
        ShellIdentity::Native => "windows".to_string(),
    }
}

/// `$MACHTYPE` default (variables.c:725): configure host triple. MSYS
/// persona reports `<arch>-pc-msys` (the MSYS2 bash configure triple);
/// native reports `<arch>-pc-windows`.
pub fn machtype() -> String {
    if !cfg!(windows) {
        return native_machtype();
    }
    match current_identity() {
        ShellIdentity::Msys => format!("{}-pc-msys", machine()),
        ShellIdentity::Native => native_machtype(),
    }
}

fn native_machtype() -> String {
    if cfg!(windows) {
        format!("{}-pc-windows", std::env::consts::ARCH)
    } else if cfg!(target_os = "macos") {
        format!("{}-apple-darwin", std::env::consts::ARCH)
    } else if cfg!(target_env = "gnu") {
        format!("{}-pc-{}-gnu", std::env::consts::ARCH, std::env::consts::OS)
    } else {
        format!(
            "{}-unknown-{}",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    }
}

/// `uname -m` / `arch(1)`: machine hardware name. MSYS2 reports the
/// compiler target arch, so map `std::env::consts::ARCH` onto the GNU
/// spelling (`x86_64`, `aarch64`, `i686`).
pub fn machine() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i686",
        other => other,
    }
}

/// `uname -s` (kernel name): `MSYS_NT-<ver>` under the MSYS persona, with
/// the prefix chosen by the `MSYSTEM` environment variable the same way
/// MSYS2 runtime does (`MINGW64` → `MINGW64_NT-...`, `UCRT64` →
/// `UCRT64_NT-...`, `MSYS`/unset → `MSYS_NT-...`). Native persona reports
/// `Windows_NT` (the native `%OS%` value).
pub fn sysname() -> String {
    if !cfg!(windows) {
        return std::env::consts::OS.to_string();
    }
    match current_identity() {
        ShellIdentity::Msys => format!("{}-{}", msystem_prefix(), windows_version()),
        ShellIdentity::Native => "Windows_NT".to_string(),
    }
}

fn msystem_prefix() -> &'static str {
    let Ok(value) = std::env::var("MSYSTEM") else {
        return "MSYS_NT";
    };
    match value.trim().to_ascii_uppercase().as_str() {
        "MINGW64" => "MINGW64_NT",
        "MINGW32" => "MINGW32_NT",
        "UCRT64" => "UCRT64_NT",
        "CLANG64" => "CLANG64_NT",
        "CLANG32" => "CLANG32_NT",
        "CLANGARM64" => "CLANGARM64_NT",
        _ => "MSYS_NT",
    }
}

/// `uname -r` (kernel release): the Windows version string (`10.0-19044`),
/// the same string that finishes `uname -s` — the kernel the engine runs on.
pub fn release() -> String {
    if cfg!(windows) {
        windows_version()
    } else {
        "unknown".to_string()
    }
}

/// `uname -v` (kernel version): MSYS2 prints an msys-runtime build
/// timestamp (`2025-07-01 09:13 UTC` shape). The engine has no msys
/// runtime, so it prints its own build timestamp — the executable's
/// modification time — in the same shape.
pub fn kernel_version() -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(modified) = exe.metadata().and_then(|meta| meta.modified()) {
            if let Some(text) = format_utc_timestamp(modified) {
                return text;
            }
        }
    }
    "unknown".to_string()
}

/// `uname -n` (nodename): the machine name.
pub fn nodename() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "localhost".to_string())
}

/// `uname -p` (processor): MSYS2 reports `unknown`; keep the form.
pub fn processor() -> &'static str {
    "unknown"
}

/// `uname -i` (hardware platform): MSYS2 reports `unknown`; keep the form.
pub fn hardware_platform() -> &'static str {
    "unknown"
}

/// `uname -o` (operating system): `Msys` under the MSYS persona (the MSYS2
/// uname value ecosystem scripts branch on), `Windows` under the native
/// persona.
pub fn operating_system() -> String {
    if !cfg!(windows) {
        return std::env::consts::OS.to_string();
    }
    match current_identity() {
        ShellIdentity::Msys => "Msys".to_string(),
        ShellIdentity::Native => "Windows".to_string(),
    }
}

/// Windows version as `MAJOR.MINOR-BUILD` (`10.0-19044`). RtlGetVersion is
/// used instead of GetVersionEx because the latter lies to unmanifested
/// processes (returns the compatibility manifest version).
#[cfg(windows)]
fn windows_version() -> String {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct OSVERSIONINFOW {
        dwOSVersionInfoSize: u32,
        dwMajorVersion: u32,
        dwMinorVersion: u32,
        dwBuildNumber: u32,
        dwPlatformId: u32,
        szCSDVersion: [u16; 128],
    }

    extern "system" {
        fn RtlGetVersion(version_information: *mut OSVERSIONINFOW) -> i32;
    }

    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    let status = unsafe { RtlGetVersion(&mut info) };
    if status == 0 {
        format!(
            "{}.{}-{}",
            info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
        )
    } else {
        "unknown".to_string()
    }
}

#[cfg(not(windows))]
fn windows_version() -> String {
    "unknown".to_string()
}

/// Civil-date formatting (no chrono dependency): days-from-civil inverse
/// per Howard Hinnant's algorithm.
fn format_utc_timestamp(time: std::time::SystemTime) -> Option<String> {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let days = (seconds / 86_400) as i64;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        time_of_day / 3600,
        (time_of_day % 3600) / 60
    ))
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// Human-readable persona name for `--identity` / `--help` disclosure.
pub fn persona_name() -> &'static str {
    match current_identity() {
        ShellIdentity::Msys => "msys",
        ShellIdentity::Native => "native",
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn windows_version_matches_major_minor_build_shape() {
        if cfg!(windows) {
            assert!(
                windows_version()
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '-'),
                "unexpected windows version shape: {}",
                windows_version()
            );
        }
    }

    #[test]
    fn msystem_prefix_maps_known_environments() {
        // The only test allowed to touch MSYSTEM (env mutation races
        // between parallel tests); restore on the way out.
        std::env::set_var("MSYSTEM", "MINGW64");
        assert_eq!(msystem_prefix(), "MINGW64_NT");
        std::env::set_var("MSYSTEM", "ucrt64");
        assert_eq!(msystem_prefix(), "UCRT64_NT");
        std::env::set_var("MSYSTEM", "totally-unknown");
        assert_eq!(msystem_prefix(), "MSYS_NT");
        std::env::remove_var("MSYSTEM");
        assert_eq!(msystem_prefix(), "MSYS_NT");
    }

    #[test]
    fn persona_values_are_self_consistent() {
        // Whatever the persona is, OSTYPE/MACHTYPE/sysname agree with it.
        if cfg!(windows) {
            match current_identity() {
                ShellIdentity::Msys => {
                    assert_eq!(ostype(), "msys");
                    assert_eq!(machtype(), format!("{}-pc-msys", machine()));
                    let sys = sysname();
                    assert!(
                        sys.starts_with("MSYS_NT-")
                            || sys.starts_with("MINGW64_NT-")
                            || sys.starts_with("MINGW32_NT-")
                            || sys.starts_with("UCRT64_NT-")
                            || sys.starts_with("CLANG"),
                        "unexpected msys sysname: {sys}"
                    );
                    assert_eq!(operating_system(), "Msys");
                }
                ShellIdentity::Native => {
                    assert_eq!(ostype(), "windows");
                    assert_eq!(machtype(), format!("{}-pc-windows", machine()));
                    assert_eq!(sysname(), "Windows_NT");
                    assert_eq!(operating_system(), "Windows");
                }
            }
            assert_eq!(machine(), "x86_64");
        }
    }

    #[test]
    fn utc_timestamp_shape_matches_msys_form() {
        let text = format_utc_timestamp(std::time::UNIX_EPOCH).unwrap();
        assert_eq!(text, "1970-01-01 00:00 UTC");
    }
}
