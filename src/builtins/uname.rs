//! `uname`/`arch` engine builtins (rubash#154).
//!
//! uname(1)/arch(1) semantics with the persona-valued fields from
//! `crate::executor::identity`: the fields no PATH binary can supply
//! honestly (`-s`, `-r`, `-o`) come from the identity persona (uname(2)
//! utsname fields on unix), `-n`/`-m` from the host, `-p`/`-i` are
//! platform-sourced per coreutils uname.c (`unknown` where the platform
//! has no source, which the MSYS2/Windows persona reports for both).
//! Field order follows coreutils uname: `s n r v m [p] [i] o`, and `-a`
//! prints `s n r v m [p] [i] o` omitting `unknown` p/i — exactly the Git
//! Bash shape on Windows (probe 2026-09-26: `MINGW64_NT-10.0-19044 X12-C
//! 3.6.3-7674c51e.x86_64 2025-07-01 09:13 UTC x86_64 Msys`) and the WSL
//! coreutils 9.4 shape on Linux (`Linux X12-C 5.10.16.3-... x86_64
//! x86_64 x86_64 GNU/Linux`).
//!
//! The engine answers `uname`/`arch` itself instead of spawning a PATH
//! binary: identity is strategic information and must not depend on what
//! happens to be on PATH (rubash#154), and the spawn cost disappears
//! (#130). Like `sleep`/`dirname`/`basename`, these stay out of
//! BUILTIN_NAMES so introspection (`type`, `enable`, `compgen -b`) keeps
//! reporting them the way GNU bash reports external commands.

use crate::executor::identity;

/// Buffered result shared by both builtins: the executor routes stdout /
/// stderr through `write_buffered_builtin_output` so redirections apply.
pub struct IdentityToolOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// uname(1). With no option, same as `-s` (coreutils uname.c).
pub fn execute_uname(args: &[String]) -> IdentityToolOutput {
    let mut fields = Fields::default();
    let mut saw_option = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "--all" => {
                fields.all = true;
                saw_option = true;
            }
            "--kernel-name" => {
                fields.sysname = true;
                saw_option = true;
            }
            "--nodename" => {
                fields.nodename = true;
                saw_option = true;
            }
            "--kernel-release" => {
                fields.release = true;
                saw_option = true;
            }
            "--kernel-version" => {
                fields.version = true;
                saw_option = true;
            }
            "--machine" => {
                fields.machine = true;
                saw_option = true;
            }
            "--processor" => {
                fields.processor = true;
                saw_option = true;
            }
            "--hardware-platform" => {
                fields.hardware_platform = true;
                saw_option = true;
            }
            "--operating-system" => {
                fields.operating_system = true;
                saw_option = true;
            }
            "--help" => {
                return IdentityToolOutput {
                    status: 0,
                    stdout: usage_text("uname").into_bytes(),
                    stderr: Vec::new(),
                }
            }
            "--version" => {
                return version_banner("uname");
            }
            _ if arg.starts_with("--") => {
                return invalid_option_long("uname", arg);
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for option in arg[1..].chars() {
                    match option {
                        'a' => fields.all = true,
                        's' => fields.sysname = true,
                        'n' => fields.nodename = true,
                        'r' => fields.release = true,
                        'v' => fields.version = true,
                        'm' => fields.machine = true,
                        'p' => fields.processor = true,
                        'i' => fields.hardware_platform = true,
                        'o' => fields.operating_system = true,
                        _ => {
                            return invalid_option_short("uname", option);
                        }
                    }
                    saw_option = true;
                }
            }
            _ => {
                // coreutils: uname takes no operands.
                return extra_operand("uname", arg);
            }
        }
        index += 1;
    }
    // Any operand after `--` (or trailing operands) is still an error for
    // uname — it accepts no operands at all.
    if let Some(operand) = args.get(index) {
        return extra_operand("uname", operand);
    }

    if !saw_option {
        fields.sysname = true;
    }

    let mut stdout = Vec::new();
    let mut push = |text: &str, first: &mut bool| {
        if !*first {
            stdout.push(b' ');
        }
        stdout.extend_from_slice(text.as_bytes());
        *first = false;
    };
    let mut first = true;
    if fields.all {
        // coreutils -a = -s -n -r -v -m -p -i -o, OMITTING the p/i slots
        // when they are `unknown` (uname.c: `if (! (toprint == UINT_MAX &&
        // element == unknown)) print_element (element)`; --help: "except
        // omit -p and -i if unknown"). One rule reproduces every shape:
        // glibc Linux keeps both (WSL probe: `... x86_64 x86_64 x86_64
        // GNU/Linux`), darwin keeps -p (`arm`/`i386`) and omits -i, and
        // the MSYS2/Windows persona skips both (the Git Bash probe shape
        // `... x86_64 Msys`).
        push(&identity::sysname(), &mut first);
        push(&identity::nodename(), &mut first);
        push(&identity::release(), &mut first);
        push(&identity::kernel_version(), &mut first);
        push(identity::machine(), &mut first);
        let processor = identity::processor();
        if processor != "unknown" {
            push(&processor, &mut first);
        }
        let hardware_platform = identity::hardware_platform();
        if hardware_platform != "unknown" {
            push(&hardware_platform, &mut first);
        }
        push(&identity::operating_system(), &mut first);
    } else {
        if fields.sysname {
            push(&identity::sysname(), &mut first);
        }
        if fields.nodename {
            push(&identity::nodename(), &mut first);
        }
        if fields.release {
            push(&identity::release(), &mut first);
        }
        if fields.version {
            push(&identity::kernel_version(), &mut first);
        }
        if fields.machine {
            push(identity::machine(), &mut first);
        }
        if fields.processor {
            push(&identity::processor(), &mut first);
        }
        if fields.hardware_platform {
            push(&identity::hardware_platform(), &mut first);
        }
        if fields.operating_system {
            push(&identity::operating_system(), &mut first);
        }
    }
    stdout.push(b'\n');

    IdentityToolOutput {
        status: 0,
        stdout,
        stderr: Vec::new(),
    }
}

#[derive(Default)]
struct Fields {
    all: bool,
    sysname: bool,
    nodename: bool,
    release: bool,
    version: bool,
    machine: bool,
    processor: bool,
    hardware_platform: bool,
    operating_system: bool,
}

/// arch(1): print machine hardware name — `uname -m` (coreutils arch.c).
pub fn execute_arch(args: &[String]) -> IdentityToolOutput {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "--help" => {
                return IdentityToolOutput {
                    status: 0,
                    stdout: usage_text("arch").into_bytes(),
                    stderr: Vec::new(),
                }
            }
            "--version" => {
                return version_banner("arch");
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                let option = arg[1..].chars().next().unwrap_or(' ');
                return invalid_option_short("arch", option);
            }
            _ => {
                return extra_operand("arch", arg);
            }
        }
    }
    if let Some(operand) = args.get(index) {
        return extra_operand("arch", operand);
    }

    let mut stdout = identity::machine().as_bytes().to_vec();
    stdout.push(b'\n');
    IdentityToolOutput {
        status: 0,
        stdout,
        stderr: Vec::new(),
    }
}

/// coreutils error shape: `uname: invalid option -- 'z'` plus the
/// "Try 'uname --help'" hint (smart quotes as printed by the MSYS2
/// coreutils build; Git Bash probe 2026-09-26).
fn invalid_option_short(tool: &str, option: char) -> IdentityToolOutput {
    IdentityToolOutput {
        status: 1,
        stdout: Vec::new(),
        stderr: format!(
            "{tool}: invalid option -- '{option}'\nTry '{tool} --help' for more information.\n"
        )
        .into_bytes(),
    }
}

fn invalid_option_long(tool: &str, arg: &str) -> IdentityToolOutput {
    IdentityToolOutput {
        status: 1,
        stdout: Vec::new(),
        stderr: format!(
            "{tool}: unrecognized option '{arg}'\nTry '{tool} --help' for more information.\n"
        )
        .into_bytes(),
    }
}

fn extra_operand(tool: &str, operand: &str) -> IdentityToolOutput {
    IdentityToolOutput {
        status: 1,
        stdout: Vec::new(),
        stderr: format!(
            "{tool}: extra operand \u{2018}{operand}\u{2019}\nTry '{tool} --help' for more information.\n"
        )
        .into_bytes(),
    }
}

/// `--version`: the exact GNU coreutils 9.4 banner (rubash#240). The
/// compatibility contract names the baseline build — coreutils 9.4 on the
/// WSL reference — the same stance `bash --version` line 1 already takes;
/// an engine-naming banner leaks a nonexistent coreutils version to
/// scripts scraping tool output.
fn version_banner(tool: &str) -> IdentityToolOutput {
    // Tool-specific author line (coreutils 9.4: uname is MacKenzie alone,
    // arch credits Karel Zak as well).
    let authors = if tool == "arch" {
        "David MacKenzie and Karel Zak"
    } else {
        "David MacKenzie"
    };
    IdentityToolOutput {
        status: 0,
        stdout: format!(
            "{tool} (GNU coreutils) 9.4\n\
             Copyright (C) 2023 Free Software Foundation, Inc.\n\
             License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.\n\
             This is free software: you are free to change and redistribute it.\n\
             There is NO WARRANTY, to the extent permitted by law.\n\
             \n\
             Written by {authors}.\n"
        )
        .into_bytes(),
        stderr: Vec::new(),
    }
}

/// `--help`: the coreutils usage headline on stdout with exit 0.
fn usage_text(tool: &str) -> String {
    let body = if tool == "arch" {
        "Print machine hardware name (same as uname -m).\n"
    } else {
        "Print certain system information.  With no OPTION, same as -s.\n"
    };
    format!("Usage: {tool} [OPTION]...\n{body}")
}

#[cfg(test)]
mod uname_tests {
    use super::*;

    fn stdout(out: &IdentityToolOutput) -> String {
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn default_reports_sysname_only() {
        let out = execute_uname(&[]);
        assert_eq!(out.status, 0);
        assert_eq!(stdout(&out), format!("{}\n", identity::sysname()));
    }

    #[test]
    fn machine_and_arch_agree() {
        let out = execute_uname(&["-m".to_string()]);
        assert_eq!(stdout(&out), format!("{}\n", identity::machine()));
        let arch = execute_arch(&[]);
        assert_eq!(stdout(&arch), stdout(&out));
    }

    #[test]
    fn all_shape_is_sysname_nodename_release_version_machine_os() {
        let out = execute_uname(&["-a".to_string()]);
        let text = stdout(&out);
        // The kernel-version slot itself contains spaces (MSYS2 prints a
        // build timestamp, Linux prints `#1 SMP ...`, darwin prints
        // `Darwin Kernel Version ...`), so anchor the shape at both ends
        // instead of word-counting.
        assert!(
            text.starts_with(&format!("{} {}", identity::sysname(), identity::nodename())),
            "-a head: {text}"
        );
        // -a tail per platform. coreutils 9.4 uname.c prints
        // s n r v m [p] [i] o and OMITS the p/i slots when they are
        // `unknown` (`if (! (toprint == UINT_MAX && element == unknown))
        // print_element (element)`; --help: "except omit -p and -i if
        // unknown"), so the machine/os anchors differ per platform:
        //
        // - Windows (MSYS persona): p/i are `unknown` -> tail `m o`
        //   (the Git Bash probe shape, see the module doc).
        // - Linux (glibc): p/i print the machine field (WSL coreutils 9.4
        //   probe 2026-09-27: `... x86_64 x86_64 x86_64 GNU/Linux`) ->
        //   tail `m m m o`; asserted with identity::machine() three times
        //   because on the glibc targets rubash builds for (x86_64,
        //   aarch64) utsname.machine equals the Rust arch spelling.
        // - macOS: coreutils uname.c fills -p from its compile-time
        //   `__APPLE__` arm (`__arm__`/`__arm64__` -> "arm",
        //   `__i386__`/`__x86_64__` -> "i386"; the sysctl path is guarded
        //   `! defined __APPLE__`) and has no -i source on darwin (no
        //   Solaris sysinfo; UNAME_HARDWARE_PLATFORM is FreeBSD/NetBSD/
        //   OpenBSD only) -> tail `m arm o` on an aarch64-apple-darwin
        //   build (`m i386 o` on x86_64), with -i omitted as `unknown`.
        //   BSD /usr/bin/uname on macOS agrees on -p ("arm") and does not
        //   offer -i. (`-m` itself still reports the Rust target arch
        //   spelling "aarch64", a persona simplification; coreutils prints
        //   utsname.machine "arm64" — darwin -m is not asserted here.)
        #[cfg(target_os = "linux")]
        let expected_tail = format!(
            "{} {} {} {}",
            identity::machine(),
            identity::machine(),
            identity::machine(),
            identity::operating_system()
        );
        #[cfg(target_os = "macos")]
        let expected_tail = format!(
            "{} {} {}",
            identity::machine(),
            if cfg!(target_arch = "aarch64") {
                "arm"
            } else {
                "i386"
            },
            identity::operating_system()
        );
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let expected_tail = format!("{} {}", identity::machine(), identity::operating_system());
        assert!(
            text.trim_end().ends_with(&format!(" {expected_tail}")),
            "-a tail: {text} (expected ... {expected_tail})"
        );
        // An `unknown` p/i value must never survive into the -a tail.
        assert!(
            !text.trim_end().ends_with("unknown"),
            "-a tail must be machine(+p/i)+os, not unknown: {text}"
        );
    }

    #[test]
    fn combined_flags_print_in_field_order_not_flag_order() {
        let out = execute_uname(&["-mo".to_string()]);
        assert_eq!(
            stdout(&out),
            format!("{} {}\n", identity::machine(), identity::operating_system())
        );
    }

    #[test]
    fn processor_and_hardware_platform_report_unknown() {
        let out = execute_uname(&["-p".to_string(), "-i".to_string()]);
        // Explicit -p/-i print the literal `unknown` when the platform has
        // no source for the slot (the -a omission rule does not apply to
        // explicit flags — coreutils 9.4 uname.c prints `unknown` there).
        // Per platform (sources cited in identity::processor /
        // identity::hardware_platform):
        // - Windows (MSYS persona): no source for either ->
        //   `unknown unknown` (the MSYS2 coreutils port shape).
        // - Linux (glibc): sysinfo(SI_ARCHITECTURE)/SI_PLATFORM print the
        //   machine field (WSL coreutils 9.4 probe 2026-09-27: x86_64
        //   x86_64; utsname.machine equals the Rust arch spelling).
        // - macOS: -p comes from the compile-time `__APPLE__` arm
        //   (`__arm__`/`__arm64__` -> "arm", `__i386__`/`__x86_64__` ->
        //   "i386"; the sysctl path is guarded `! defined __APPLE__`;
        //   BSD /usr/bin/uname's native_arch() agrees: "arm"/"i386"), and
        //   -i has no darwin source -> `arm unknown` on an
        //   aarch64-apple-darwin build, `i386 unknown` on x86_64.
        // - Other unix: unported sysctl sources -> both `unknown`.
        #[cfg(target_os = "linux")]
        assert_eq!(
            stdout(&out),
            format!("{} {}\n", identity::machine(), identity::machine())
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            stdout(&out),
            format!(
                "{} unknown\n",
                if cfg!(target_arch = "aarch64") {
                    "arm"
                } else {
                    "i386"
                }
            )
        );
        #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
        assert_eq!(stdout(&out), "unknown unknown\n");
        #[cfg(not(unix))]
        assert_eq!(stdout(&out), "unknown unknown\n");
    }

    #[test]
    fn invalid_option_is_usage_error() {
        let out = execute_uname(&["-z".to_string()]);
        assert_eq!(out.status, 1);
        assert!(out.stdout.is_empty());
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(
            text.starts_with("uname: invalid option -- 'z'"),
            "stderr: {text}"
        );
    }

    #[test]
    fn extra_operand_is_rejected_like_coreutils() {
        let out = execute_arch(&["extra".to_string()]);
        assert_eq!(out.status, 1);
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(
            text.starts_with("arch: extra operand \u{2018}extra\u{2019}"),
            "stderr: {text}"
        );
    }
}
