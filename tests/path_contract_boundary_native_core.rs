//! Boundary contract — invariant A: the native core.
//!
//! Pins the core design decision that survived the 2026-10 merges: rubash is
//! a Windows-native binary with ZERO MSYS2/cygwin runtime dependency, and the
//! spawn path speaks Win32 directly (a POSIX-emulation layer would show up as
//! `msys-2.0.dll` / `cygwin1.dll` imports and as POSIX-form paths leaking to
//! native children).
//!
//! These are CONTRACT tests, not issue regressions: every assertion here is
//! expected green on current master. Red means the native-core invariant was
//! broken by a merge and the boundary doc
//! (`docs/windows-path-contract.md`) is now lying.
//!
//! The contract's verification target IS the Windows-native behavior: the
//! file is `#![cfg(windows)]`-gated as a whole. unix CI only verifies
//! compilation (the file cfg-strips to an empty test binary there), not the
//! cases — the cases need a Windows runner with the installed winuxcmd tree
//! for the toolchain leg (it skips itself when WINUXCMD_HOME is absent).

#![cfg(windows)]

use std::path::Path;
use std::process::Command;

/// The set of DLL families a native Rust/MSVC binary may import. Anything
/// outside this set on the import table is a design break worth an issue.
#[cfg(windows)]
fn is_windows_system_import(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("api-ms-win-")
        || lower.starts_with("ext-ms-")
        || matches!(
            lower.as_str(),
            "kernel32.dll"
                | "ntdll.dll"
                | "user32.dll"
                | "gdi32.dll"
                | "shell32.dll"
                | "advapi32.dll"
                | "ole32.dll"
                | "oleaut32.dll"
                | "shlwapi.dll"
                | "ws2_32.dll"
                | "bcrypt.dll"
                | "bcryptprimitives.dll"
                | "ncrypt.dll"
                | "userenv.dll"
                | "dbghelp.dll"
                | "vcruntime140.dll"
                | "vcruntime140_1.dll"
                | "msvcp140.dll"
                | "ucrtbase.dll"
                | "rpcrt4.dll"
                | "sechost.dll"
                | "comdlg32.dll"
                | "wintrust.dll"
        )
}

#[cfg(windows)]
struct PeImage<'a> {
    bytes: &'a [u8],
    sections: usize,
    section_count: usize,
    import_rva: u32,
}

#[cfg(windows)]
impl<'a> PeImage<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let read_u16 = |offset: usize| -> Result<u16, String> {
            bytes
                .get(offset..offset + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .ok_or_else(|| format!("image truncated at {offset}"))
        };
        let read_u32 = |offset: usize| -> Result<u32, String> {
            bytes
                .get(offset..offset + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| format!("image truncated at {offset}"))
        };

        if bytes.get(0..2) != Some(b"MZ") {
            return Err("not an MZ image".into());
        }
        let pe_offset = read_u32(0x3C)? as usize;
        if bytes.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0") {
            return Err("PE signature missing".into());
        }
        let section_count = read_u16(pe_offset + 6)? as usize;
        let optional_header_size = read_u16(pe_offset + 20)? as usize;
        let optional_header = pe_offset + 24;
        let magic = read_u16(optional_header)?;
        // DataDirectory[1] = the import table (entry 0 is the export
        // directory, hence the +8). The array starts after the standard
        // fields: PE32+ at +112, PE32 at +96.
        let import_dir = match magic {
            0x20B => optional_header + 112 + 8,
            0x10B => optional_header + 96 + 8,
            other => return Err(format!("unknown optional-header magic {other:#x}")),
        };
        let import_rva = read_u32(import_dir)?;
        let sections = optional_header + optional_header_size;
        Ok(Self {
            bytes,
            sections,
            section_count,
            import_rva,
        })
    }

    fn rva_to_offset(&self, rva: u32) -> Result<usize, String> {
        let read_u32 = |offset: usize| -> Result<u32, String> {
            self.bytes
                .get(offset..offset + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| format!("image truncated at {offset}"))
        };
        for index in 0..self.section_count {
            let entry = self.sections + index * 40;
            let virtual_size = read_u32(entry + 8)? as u64;
            let virtual_address = read_u32(entry + 12)? as u64;
            let raw_size = read_u32(entry + 16)? as u64;
            let raw_pointer = read_u32(entry + 20)? as u64;
            if (virtual_address..virtual_address + virtual_size.max(raw_size))
                .contains(&(rva as u64))
            {
                return Ok((raw_pointer + (rva as u64 - virtual_address)) as usize);
            }
        }
        Err(format!("RVA {rva:#x} not in any section"))
    }

    fn cstring_at(&self, offset: usize) -> Result<String, String> {
        let start = self
            .bytes
            .get(offset..)
            .ok_or_else(|| format!("image truncated at {offset}"))?;
        let end = start
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "unterminated import name".to_string())?;
        Ok(String::from_utf8_lossy(&start[..end]).into_owned())
    }

    /// DLL names on the import table (empty when the image imports nothing).
    fn import_dll_names(&self) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        if self.import_rva == 0 {
            return Ok(names);
        }
        let mut descriptor = self.rva_to_offset(self.import_rva)?;
        loop {
            let read_u32 = |offset: usize| -> Result<u32, String> {
                self.bytes
                    .get(offset..offset + 4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .ok_or_else(|| format!("image truncated at {offset}"))
            };
            let name_rva = read_u32(descriptor + 12)?;
            if name_rva == 0 {
                break; // null descriptor terminates the array
            }
            names.push(self.cstring_at(self.rva_to_offset(name_rva)?)?);
            descriptor += 20;
        }
        Ok(names)
    }
}

#[cfg(windows)]
fn import_dll_names_of(path: &Path) -> Result<Vec<String>, String> {
    let image = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    PeImage::parse(&image)?.import_dll_names()
}

fn winuxcmd_home() -> Option<String> {
    if let Ok(home) = std::env::var("WINUXCMD_HOME") {
        if !home.is_empty() && Path::new(&home).is_dir() {
            return Some(home);
        }
    }
    None
}

/// POSITIVE: rubash.exe's import table names only Windows system DLLs —
/// the binary is self-contained native code speaking Win32 directly.
#[test]
fn rubash_links_only_windows_system_dlls() {
    let imports = import_dll_names_of(Path::new(env!("CARGO_BIN_EXE_rubash")))
        .expect("parse rubash.exe PE imports");
    assert!(
        !imports.is_empty(),
        "rubash.exe reports an empty import table — parser or image broken"
    );
    for name in &imports {
        assert!(
            is_windows_system_import(name),
            "rubash.exe imports {name:?}, which is outside the Windows system set \
             (native-core invariant A: no third-party runtime may be required)"
        );
    }
}

/// NEGATIVE: the POSIX runtimes must appear NOWHERE on the import table.
/// (The string literals "msys-2.0.dll"/"cygwin1.dll" legitimately exist in
/// .rdata — they are the per-child posix-runtime-hosted DETECTION probes in
/// executor/path.rs — so the assertion reads the import table, not raw
/// bytes.)
#[test]
fn rubash_does_not_import_msys_or_cygwin_runtime() {
    let imports = import_dll_names_of(Path::new(env!("CARGO_BIN_EXE_rubash")))
        .expect("parse rubash.exe PE imports");
    for name in &imports {
        let lower = name.to_ascii_lowercase();
        assert!(
            !lower.contains("msys") && !lower.contains("cygwin"),
            "rubash.exe imports POSIX runtime DLL {name:?} — the native-core \
             invariant (zero MSYS/cygwin dependency) is broken"
        );
    }
}

/// POSITIVE + NEGATIVE: rubash runs its builtin surface with NO POSIX
/// toolset on PATH at all (no winuxcmd tree, no usr/bin): `echo`, the
/// command substitution, and the hidden fast-path `uname` builtin
/// (docs/platform-identity.md: works "even when PATH carries no uname.exe")
/// all come from the engine itself. A simulation layer that shelled out to
/// MSYS tools would fail this.
#[test]
fn rubash_builtin_surface_runs_with_empty_posix_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "echo pathless-$(uname -m)"])
        .env("PATH", "C:\\Windows\\System32")
        .env_remove("WINUXSH_ROOT")
        .env_remove("NIU_SHELL_ROOT")
        .env_remove("RUBASH_ROOT")
        .env_remove("WINUXCMD_HOME")
        .output()
        .expect("spawn rubash");
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("pathless-") && (stdout.contains("x86") || stdout.contains("aarch")),
        "builtin surface without POSIX tools printed {stdout:?} — the engine \
         must not depend on an external POSIX toolset"
    );
    // NEGATIVE half: no MSYS-style toolset path may leak into engine output.
    assert!(
        !stdout.contains("/usr/bin/"),
        "engine output references an external POSIX toolset: {stdout:?}"
    );
}

/// POSITIVE + NEGATIVE: the spawn path speaks Win32 for native children —
/// after `cd /c/...`, a native `cmd /c cd` reports the BACKSLASH drive
/// form, not the shell's POSIX persona. This pins the per-child dialect
/// contract (docs/platform-identity.md): POSIX form inside the shell, real
/// Win32 spellings for path-shaped state handed to native children.
#[test]
fn native_children_receive_win32_cwd_not_posix_persona() {
    let scratch = std::env::temp_dir().join(format!("rubash-bc-native-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let scratch_win = scratch.to_string_lossy().replace('\\', "/");
    let scratch_posix = format!(
        "/{}/{}",
        (scratch_win.as_bytes()[0] as char).to_ascii_lowercase(),
        &scratch_win[3..]
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", &format!("cd '{scratch_posix}'; cmd /c cd")])
        .output()
        .expect("spawn rubash");
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let drive = scratch_win[..2].to_ascii_uppercase(); // e.g. "C:"
    assert!(
        stdout.contains(&drive) && stdout.contains('\\'),
        "native child cwd {stdout:?} is not the Win32 form {drive}\\... — \
         the spawn boundary stopped translating for native children"
    );
    assert!(
        !stdout.trim().starts_with('/'),
        "native child cwd {stdout:?} leaked the POSIX persona form — \
         per-child dialect broken (natives must get Win32 spellings)"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// Installed-toolchain leg (skips cleanly when winuxcmd is not installed,
/// e.g. CI): every audited tool in the winuxcmd usr/bin toolset — the
/// dispatcher plus the wrappers the boundary doc names (touch/ls/find/
/// timeout/xargs/env/rm) — must import no POSIX runtime DLL, same rule as
/// rubash itself.
#[test]
fn winuxcmd_toolchain_imports_are_windows_native() {
    let Some(home) = winuxcmd_home() else {
        eprintln!(
            "skipping: winuxcmd installation not found (set WINUXCMD_HOME to audit \
             the installed toolchain)"
        );
        return;
    };
    let bin = Path::new(&home).join("usr").join("bin");
    let tools = [
        "winuxcmd.exe",
        "touch.exe",
        "ls.exe",
        "find.exe",
        "timeout.exe",
        "xargs.exe",
        "env.exe",
        "rm.exe",
    ];
    for tool in tools {
        let path = bin.join(tool);
        if !path.is_file() {
            continue; // partial install: the tools that exist must still hold the contract
        }
        let imports = import_dll_names_of(&path)
            .unwrap_or_else(|error| panic!("parse {} imports: {error}", path.display()));
        for name in &imports {
            let lower = name.to_ascii_lowercase();
            assert!(
                !lower.contains("msys") && !lower.contains("cygwin"),
                "{} imports POSIX runtime DLL {name:?} — the winuxcmd toolchain \
                 must stay zero-MSYS/cygwin (invariant A)",
                path.display()
            );
        }
    }
}
