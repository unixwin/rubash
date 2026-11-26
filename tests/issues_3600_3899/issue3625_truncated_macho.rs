//! Regression pins for rubash#3625 — truncated Mach-O headers must not
//! be classified as directly exec-able native Mach-O images.
//!
//! XNU requires a full `mach_header` (28 bytes for 32-bit, 32 bytes for
//! 64-bit) or `fat_header` (8 bytes) + `nfat_arch * 20` (or `* 32` for
//! FAT_MAGIC_64 / FAT_CIGAM_64). Truncated files fall through to ENOEXEC
//! and must be refused with exit status 126 ("cannot execute binary file")
//! rather than bypassing the ENOEXEC gate.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rubash-3625-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        Self { dir }
    }

    fn write_exec(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).expect("write fixture file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fixture file");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_script(cwd: &Path, script: &str) -> (i32, String, String) {
    let out = Command::new(BIN)
        .args(["-c", script])
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn truncated_thin_macho_is_refused_as_enoexec_binary_fg_and_bg() {
    let fx = Fixture::new("thin");
    // 12-byte truncated thin 64-bit Mach-O (< 32 bytes) with NUL bytes.
    let mut thin64 = vec![0u8; 12];
    thin64[..8].copy_from_slice(&[0xCF, 0xFA, 0xED, 0xFE, 0x0C, 0x00, 0x00, 0x01]);
    fx.write_exec("trunc64", &thin64);

    // 12-byte truncated thin 32-bit Mach-O (< 28 bytes) with NUL bytes.
    let mut thin32 = vec![0u8; 12];
    thin32[..8].copy_from_slice(&[0xCE, 0xFA, 0xED, 0xFE, 0x07, 0x00, 0x00, 0x00]);
    fx.write_exec("trunc32", &thin32);

    for name in ["trunc64", "trunc32"] {
        let (code, _out, err) = run_script(&fx.dir, &format!("./{name}"));
        assert_eq!(code, 126, "fg {name}: stderr={err}");
        assert!(
            err.contains("cannot execute binary file"),
            "fg {name}: stderr={err}"
        );

        let (bg_code, _bg_out, bg_err) = run_script(&fx.dir, &format!("./{name} & wait $!"));
        assert_eq!(bg_code, 126, "bg {name}: stderr={bg_err}");
        assert!(
            bg_err.contains("cannot execute binary file"),
            "bg {name}: stderr={bg_err}"
        );
    }
}

#[test]
fn truncated_fat_and_fat64_macho_are_refused_as_enoexec_binary() {
    let fx = Fixture::new("fat");
    // Fat 32-bit claiming 2 slices (needs 48 bytes), truncated to 16 bytes.
    let mut fat32 = vec![0u8; 16];
    fat32[..8].copy_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x00, 0x00, 0x02]);
    fx.write_exec("trunc_fat32", &fat32);

    // Fat 64-bit claiming 2 slices (needs 72 bytes), truncated to 40 bytes.
    let mut fat64 = vec![0u8; 40];
    fat64[..8].copy_from_slice(&[0xCA, 0xFE, 0xBA, 0xBF, 0x00, 0x00, 0x00, 0x02]);
    fx.write_exec("trunc_fat64", &fat64);

    for name in ["trunc_fat32", "trunc_fat64"] {
        let (code, _out, err) = run_script(&fx.dir, &format!("./{name}"));
        assert_eq!(code, 126, "fg {name}: stderr={err}");
        assert!(
            err.contains("cannot execute binary file"),
            "fg {name}: stderr={err}"
        );

        let (bg_code, _bg_out, bg_err) = run_script(&fx.dir, &format!("./{name} & wait $!"));
        assert_eq!(bg_code, 126, "bg {name}: stderr={bg_err}");
        assert!(
            bg_err.contains("cannot execute binary file"),
            "bg {name}: stderr={bg_err}"
        );
    }
}
