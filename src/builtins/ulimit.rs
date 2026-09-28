//! ulimit module.
//!
//! GNU Bash source ownership:
//! - `builtins/ulimit.def:239` `limits[]` — the option table (option char,
//!   resource, block factor, description, units), one `-a` line per entry.
//! - `builtins/ulimit.def:346-356` — optstring is built as `"aSH"` plus
//!   each table letter followed by `';'` (optional argument).
//! - `builtins/bashgetopt.c` `internal_getopt` — the option scan: `NOTOPT`
//!   (a word not starting with `-`, or exactly `-`) ends the scan;
//!   `--help` in option position is `CASE_HELPOPT` (common.h:31) which
//!   prints the builtin help and returns `EX_USAGE` (2); a `';'` option
//!   takes an attached argument (`-n2048`) or the following non-option
//!   word, otherwise none.
//! - `builtins/ulimit.def:382-384` — invalid option letter:
//!   `sh_invalidopt` + `builtin_usage` + `EX_USAGE` (2).
//! - `builtins/ulimit.def:414-421` — no table option given defaults to
//!   `-f` with the first operand as its argument.
//! - `builtins/ulimit.def:437-441` — POSIX compatibility: a leftover
//!   operand becomes the argument of the last option that has none.
//! - `builtins/ulimit.def:451-517` `ulimit_internal` — `hard`/`soft`/
//!   `unlimited` operand words; the `DIGIT` gate plus
//!   `string_to_rlimtype` full-consumption check routing to
//!   `sh_invalidnum` (rc 1); the block-factor multiply with the
//!   `real_limit / block_factor != limit` overflow check routing to
//!   `sh_erange` (rc 1); get/set failures report
//!   `"<desc>: cannot get/modify limit: <strerror>"` (rc 1).
//! - `builtins/ulimit.def:754-777` `printone` — `printf("%-20s %20s")`
//!   description/unit header when printing with a description, then
//!   `unlimited` or `curlim / factor`.
//! - `builtins/ulimit.def:208-212` — POSIXBLK: `-c`/`-f` count 512-byte
//!   blocks in posix mode, 1024-byte blocks otherwise.
//! - `builtins/common.c:214-226` `sh_invalidnum` — `0`+digit reports
//!   "invalid octal number", `0x` prefix reports "invalid hex number".
//!
//! Platform contract:
//! - **unix**: real `getrlimit`/`setrlimit` (plus the `pipesize()` special
//!   for `-p` and the non-root `unlimited` softening of ulimit.def:623-626).
//!   The table is cfg-gated per target exactly like the C `#ifdef`s.
//! - **windows**: there is no OS rlimit API, so limits are an *honest
//!   emulation*: a per-shell soft/hard table persisted in the shell
//!   environment under `__RUBASH_ULIMIT_<opt>` (`"<soft>:<hard>"` decimal,
//!   `u64::MAX` meaning unlimited). Queries read the emulated table and
//!   sets update it in place, mirroring fork inheritance through the
//!   env-var copy (a subshell/comsub sees the parent's values and its own
//!   sets do not propagate back). No OS resource limit is enforced: a
//!   hard-limit raise a non-root GNU would reject with EPERM succeeds here
//!   (the WSL oracle runs as root, where it succeeds too). The one errno
//!   the emulation keeps is the kernel's own setrlimit pair check — a set
//!   whose resulting soft exceeds its resulting hard fails with EINVAL,
//!   exactly like the oracle — plus `-p` (the pipe buffer is a kernel
//!   constant, not a limit — `set` fails with EINVAL on both platforms).
//!   The table shape mirrors the Linux glibc build of GNU bash 5.3.0 (the
//!   WSL oracle) so scripts ported from Linux see the same 17 `-a` rows.

use std::collections::HashMap;
use std::io::{self, Write};

use super::help;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

const USAGE: &str = "ulimit: usage: ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]";

/// RLIM_INFINITY (ulimit.def:193-195; Linux uses ~0).
const UNLIMITED: u64 = u64::MAX;

const LIMIT_HARD: u8 = 0x01;
const LIMIT_SOFT: u8 = 0x02;

// ---- limits[] (ulimit.def:239-298) ----------------------------------------

/// Block scaling (ulimit.def:208-212): POSIXBLK is 512 in posix mode and
/// 1024 otherwise; every other entry carries an explicit factor.
#[derive(Clone, Copy)]
enum Block {
    Posix,
    Units(u64),
}

struct LimitSpec {
    option: char,
    block: Block,
    description: &'static str,
    units: Option<&'static str>,
}

const fn spec(
    option: char,
    block: Block,
    description: &'static str,
    units: Option<&'static str>,
) -> LimitSpec {
    LimitSpec {
        option,
        block,
        description,
        units,
    }
}

/// Linux table: what the vendored C compiles to with glibc's
/// sys/resource.h (RLIMIT_NPTS/PTHREAD/SBSIZE/KQUEUES/SWAP undefined).
/// Verified row-for-row against the WSL GNU Bash 5.3.0 `ulimit -a` oracle.
#[cfg(target_os = "linux")]
const LIMITS: &[LimitSpec] = &[
    spec(
        'R',
        Block::Units(1),
        "real-time non-blocking time",
        Some("microseconds"),
    ),
    spec('c', Block::Posix, "core file size", Some("blocks")),
    spec('d', Block::Units(1024), "data seg size", Some("kbytes")),
    spec('e', Block::Units(1), "scheduling priority", None),
    spec('f', Block::Posix, "file size", Some("blocks")),
    spec('i', Block::Units(1), "pending signals", None),
    spec('l', Block::Units(1024), "max locked memory", Some("kbytes")),
    spec('m', Block::Units(1024), "max memory size", Some("kbytes")),
    spec('n', Block::Units(1), "open files", None),
    spec('p', Block::Units(512), "pipe size", Some("512 bytes")),
    spec('q', Block::Units(1), "POSIX message queues", Some("bytes")),
    spec('r', Block::Units(1), "real-time priority", None),
    spec('s', Block::Units(1024), "stack size", Some("kbytes")),
    spec('t', Block::Units(1), "cpu time", Some("seconds")),
    spec('u', Block::Units(1), "max user processes", None),
    spec('v', Block::Units(1024), "virtual memory", Some("kbytes")),
    spec('x', Block::Units(1), "file locks", None),
];

/// Conservative table for non-Linux unix targets: the resources POSIX-ish
/// libc implementations agree on. Exotic letters are invalid options there,
/// exactly like a C build without the corresponding RLIMIT_* defines.
#[cfg(all(unix, not(target_os = "linux")))]
const LIMITS: &[LimitSpec] = &[
    spec('c', Block::Posix, "core file size", Some("blocks")),
    spec('d', Block::Units(1024), "data seg size", Some("kbytes")),
    spec('f', Block::Posix, "file size", Some("blocks")),
    spec('n', Block::Units(1), "open files", None),
    spec('p', Block::Units(512), "pipe size", Some("512 bytes")),
    spec('s', Block::Units(1024), "stack size", Some("kbytes")),
    spec('t', Block::Units(1), "cpu time", Some("seconds")),
    spec('u', Block::Units(1), "max user processes", None),
];

/// Windows emulation: the same shape as the Linux oracle table (see the
/// module platform-contract note). Values are the emulated defaults; the
/// soft/hard pair lives in the shell env as `__RUBASH_ULIMIT_<opt>`.
#[cfg(windows)]
const LIMITS: &[LimitSpec] = &[
    spec(
        'R',
        Block::Units(1),
        "real-time non-blocking time",
        Some("microseconds"),
    ),
    spec('c', Block::Posix, "core file size", Some("blocks")),
    spec('d', Block::Units(1024), "data seg size", Some("kbytes")),
    spec('e', Block::Units(1), "scheduling priority", None),
    spec('f', Block::Posix, "file size", Some("blocks")),
    spec('i', Block::Units(1), "pending signals", None),
    spec('l', Block::Units(1024), "max locked memory", Some("kbytes")),
    spec('m', Block::Units(1024), "max memory size", Some("kbytes")),
    spec('n', Block::Units(1), "open files", None),
    spec('p', Block::Units(512), "pipe size", Some("512 bytes")),
    spec('q', Block::Units(1), "POSIX message queues", Some("bytes")),
    spec('r', Block::Units(1), "real-time priority", None),
    spec('s', Block::Units(1024), "stack size", Some("kbytes")),
    spec('t', Block::Units(1), "cpu time", Some("seconds")),
    spec('u', Block::Units(1), "max user processes", None),
    spec('v', Block::Units(1024), "virtual memory", Some("kbytes")),
    spec('x', Block::Units(1), "file locks", None),
];

fn findlim(option: char) -> Option<&'static LimitSpec> {
    LIMITS.iter().find(|spec| spec.option == option)
}

fn blocksize(block: Block, posix_mode: bool) -> u64 {
    match block {
        Block::Posix => {
            if posix_mode {
                512
            } else {
                1024
            }
        }
        Block::Units(n) => n,
    }
}

// ---- platform get/set ------------------------------------------------------

#[cfg(unix)]
mod plat {
    use super::*;

    /// get_limit (ulimit.def:519-576): getrlimit for table resources, the
    /// pipesize() special for `-p` (ulimit.def:681-713: pathconf, falling
    /// back to PIPE_BUF).
    pub(super) fn get(option: char, _env_vars: &HashMap<String, String>) -> io::Result<(u64, u64)> {
        if option == 'p' {
            let configured = unsafe {
                libc::pathconf(b".\0".as_ptr() as *const libc::c_char, libc::_PC_PIPE_BUF)
            };
            let pipe_buf: u64 = if configured >= 0 {
                configured as u64
            } else {
                4096
            };
            return Ok((pipe_buf, pipe_buf));
        }
        rlimit_get(resource(option))
    }

    /// set_limit (ulimit.def:578-638): `-p` has no settable limit
    /// (EINVAL); everything else goes through setrlimit with the
    /// non-root `unlimited` softening of ulimit.def:623-626.
    pub(super) fn set(
        option: char,
        newlim: u64,
        mode: u8,
        _env_vars: &mut HashMap<String, String>,
    ) -> io::Result<()> {
        if option == 'p' {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        let res = resource(option);
        let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrlimit(res, &mut limit) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let val = if unsafe { libc::geteuid() } != 0
            && newlim == UNLIMITED
            && (mode & LIMIT_HARD) == 0
            && (limit.rlim_cur as u64) <= (limit.rlim_max as u64)
        {
            limit.rlim_max as u64
        } else {
            newlim
        };
        if mode & LIMIT_SOFT != 0 {
            limit.rlim_cur = val as libc::rlim_t;
        }
        if mode & LIMIT_HARD != 0 {
            limit.rlim_max = val as libc::rlim_t;
        }
        if unsafe { libc::setrlimit(res, &limit) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn resource(option: char) -> libc::__rlimit_resource_t {
        match option {
            'R' => libc::RLIMIT_RTTIME,
            'c' => libc::RLIMIT_CORE,
            'd' => libc::RLIMIT_DATA,
            'e' => libc::RLIMIT_NICE,
            'f' => libc::RLIMIT_FSIZE,
            'i' => libc::RLIMIT_SIGPENDING,
            'l' => libc::RLIMIT_MEMLOCK,
            'm' => libc::RLIMIT_RSS,
            'n' => libc::RLIMIT_NOFILE,
            'q' => libc::RLIMIT_MSGQUEUE,
            'r' => libc::RLIMIT_RTPRIO,
            's' => libc::RLIMIT_STACK,
            't' => libc::RLIMIT_CPU,
            'u' => libc::RLIMIT_NPROC,
            'v' => libc::RLIMIT_AS,
            'x' => libc::RLIMIT_LOCKS,
            _ => libc::RLIMIT_FSIZE, // unreachable: parser only admits table letters
        }
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn resource(option: char) -> libc::c_int {
        match option {
            'c' => libc::RLIMIT_CORE,
            'd' => libc::RLIMIT_DATA,
            'f' => libc::RLIMIT_FSIZE,
            'n' => libc::RLIMIT_NOFILE,
            's' => libc::RLIMIT_STACK,
            't' => libc::RLIMIT_CPU,
            'u' => libc::RLIMIT_NPROC,
            _ => libc::RLIMIT_FSIZE, // unreachable: parser only admits table letters
        }
    }

    #[cfg(target_os = "linux")]
    fn rlimit_get(res: libc::__rlimit_resource_t) -> io::Result<(u64, u64)> {
        let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrlimit(res, &mut limit) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((limit.rlim_cur as u64, limit.rlim_max as u64))
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn rlimit_get(res: libc::c_int) -> io::Result<(u64, u64)> {
        let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrlimit(res, &mut limit) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((limit.rlim_cur as u64, limit.rlim_max as u64))
    }
}

#[cfg(windows)]
mod plat {
    use super::*;

    const KEY_PREFIX: &str = "__RUBASH_ULIMIT_";

    fn key(option: char) -> String {
        format!("{KEY_PREFIX}{option}")
    }

    /// Emulated defaults (documented in the module platform-contract note):
    /// the old stub's visible values where it had an opinion (f/d/t/u/v
    /// unlimited, n soft 1024, p 8x512, s 8192 kbytes), core off like a
    /// stock Linux kernel, plus the remaining oracle-table rows.
    fn default_for(option: char) -> (u64, u64) {
        match option {
            'c' => (0, UNLIMITED), // Linux-typical: core dumps off by default
            'e' | 'r' => (0, 0),
            'n' => (1024, 4096),
            'p' => (4096, 4096),
            'q' => (819200, 819200),
            's' => (8_388_608, UNLIMITED),
            _ => (UNLIMITED, UNLIMITED),
        }
    }

    pub(super) fn get(option: char, env_vars: &HashMap<String, String>) -> io::Result<(u64, u64)> {
        if let Some(cell) = env_vars.get(&key(option)) {
            let (soft, hard) = cell.split_once(':').unwrap_or((cell, cell));
            let soft = soft.parse::<u64>().unwrap_or(UNLIMITED);
            let hard = hard.parse::<u64>().unwrap_or(UNLIMITED);
            return Ok((soft, hard));
        }
        Ok(default_for(option))
    }

    pub(super) fn set(
        option: char,
        newlim: u64,
        mode: u8,
        env_vars: &mut HashMap<String, String>,
    ) -> io::Result<()> {
        // The pipe buffer is a kernel constant, not a limit: GNU fails the
        // set with EINVAL on every platform, and so does the emulation.
        // The error is synthesized (a raw os error code would be read as a
        // Win32 code on Windows), carrying the strerror text itself —
        // posix_errors::message passes synthesized payloads through.
        if option == 'p' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid argument",
            ));
        }
        let (soft, hard) = get(option, env_vars)?;
        let soft = if mode & LIMIT_SOFT != 0 { newlim } else { soft };
        let hard = if mode & LIMIT_HARD != 0 { newlim } else { hard };
        // Linux kernel setrlimit invariant (verified against the WSL GNU
        // 5.3.0 root oracle, kernel sys/resource check): a submitted pair
        // with rlim_cur > rlim_max fails with EINVAL no matter who calls —
        // `ulimit -Sn 4096` over hard 2048, `ulimit -Hn 1024` under soft
        // 2048, and `-Sn unlimited` over a finite hard all fail. UNLIMITED
        // is RLIM_INFINITY (~0), numerically greater than any finite hard.
        // Raising a hard limit needs privilege the emulation does not
        // model (the oracle runs as root, where it always succeeds); the
        // soft>hard check is the one errno the emulation keeps.
        if soft > hard {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid argument",
            ));
        }
        env_vars.insert(key(option), format!("{soft}:{hard}"));
        Ok(())
    }
}

// ---- option scan (bashgetopt.c internal_getopt for this optstring) --------

struct Scan {
    all_limits: bool,
    mode: u8,
    cmdlist: Vec<(char, Option<String>)>,
    operands: Vec<String>,
}

enum ScanOutcome {
    Parsed(Scan),
    /// sh_invalidopt + builtin_usage + EX_USAGE (ulimit.def:382-384).
    InvalidOption(char),
    /// CASE_HELPOPT (common.h:31): builtin_help then EX_USAGE.
    Help,
}

/// A word is a non-option (bashgetopt.c:37 NOTOPT, plus==false here) when
/// it does not start with `-` or is exactly `-`.
fn notopt(word: &str) -> bool {
    !word.starts_with('-') || word == "-"
}

fn scan(args: &[String]) -> ScanOutcome {
    let mut all_limits = false;
    let mut mode: u8 = 0;
    let mut cmdlist: Vec<(char, Option<String>)> = Vec::new();
    let mut index = 0usize;
    // sp: byte offset within the current word, starting after the leading
    // '-' (bashgetopt.c keeps sp = 1 for exactly this).
    let mut sp = 1usize;

    while index < args.len() {
        let word = args[index].as_str();
        if sp == 1 {
            if notopt(word) {
                break;
            }
            if word == "--help" {
                return ScanOutcome::Help;
            }
            if word == "--" {
                index += 1;
                break;
            }
        }
        let Some(&c) = word.as_bytes().get(sp) else {
            index += 1;
            sp = 1;
            continue;
        };
        match c {
            b'a' => {
                all_limits = true;
                sp += 1;
                if sp >= word.len() {
                    index += 1;
                    sp = 1;
                }
            }
            b'S' => {
                mode |= LIMIT_SOFT;
                sp += 1;
                if sp >= word.len() {
                    index += 1;
                    sp = 1;
                }
            }
            b'H' => {
                mode |= LIMIT_HARD;
                sp += 1;
                if sp >= word.len() {
                    index += 1;
                    sp = 1;
                }
            }
            _ if findlim(char::from(c)).is_some() => {
                // `';'` option (bashgetopt.c:104-136): an attached rest
                // (`-n2048`) or the following non-option word is the
                // argument; otherwise none.
                let attached = &word[sp + 1..];
                if !attached.is_empty() {
                    cmdlist.push((char::from(c), Some(attached.to_string())));
                    index += 1;
                    sp = 1;
                } else if index + 1 < args.len() && notopt(&args[index + 1]) {
                    cmdlist.push((char::from(c), Some(args[index + 1].clone())));
                    index += 2;
                    sp = 1;
                } else {
                    cmdlist.push((char::from(c), None));
                    index += 1;
                    sp = 1;
                }
            }
            _ => return ScanOutcome::InvalidOption(char::from(c)),
        }
    }

    ScanOutcome::Parsed(Scan {
        all_limits,
        mode,
        cmdlist,
        operands: args[index.min(args.len())..].to_vec(),
    })
}

// ---- builtin body (ulimit_builtin, ulimit.def:336-448) ---------------------

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let prefix = diagnostic_prefix(env_vars);
    let posix_mode = env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1");

    let scan = match scan(args) {
        ScanOutcome::Parsed(scan) => scan,
        ScanOutcome::InvalidOption(c) => {
            writeln!(stderr, "{prefix}ulimit: -{c}: invalid option")?;
            writeln!(stderr, "{USAGE}")?;
            return Ok(EX_USAGE);
        }
        ScanOutcome::Help => {
            help::print_builtin_help("ulimit", stdout)?;
            return Ok(EX_USAGE);
        }
    };

    let Scan {
        all_limits,
        mut mode,
        mut cmdlist,
        operands,
    } = scan;

    if all_limits {
        // ulimit.def:396-411: -a ignores operands (set_all_limits is
        // compiled out) and prints soft limits unless -H/-S say otherwise.
        if mode == 0 {
            mode = LIMIT_SOFT;
        }
        for spec in LIMITS {
            match plat::get(spec.option, env_vars) {
                Ok((soft, hard)) => {
                    let value = if mode & LIMIT_SOFT != 0 { soft } else { hard };
                    printone(spec, value, true, posix_mode, stdout)?;
                }
                Err(err) if err.raw_os_error() != Some(22) => {
                    // print_all_limits (ulimit.def:746-749) skips EINVAL rows.
                    writeln!(
                        stderr,
                        "{prefix}ulimit: {}: cannot get limit: {}",
                        spec.description,
                        crate::posix_errors::message(&err)
                    )?;
                }
                Err(_) => {}
            }
        }
        return Ok(EXECUTION_SUCCESS);
    }

    // ulimit.def:414-421: no table option -> `-f` with the first operand.
    if cmdlist.is_empty() {
        let arg = operands.first().cloned();
        cmdlist.push(('f', arg));
    }

    // ulimit.def:437-441: a leftover operand becomes the argument of the
    // last option that has none.
    if !operands.is_empty() && cmdlist.last().is_some_and(|(_, arg)| arg.is_none()) {
        if let Some((_, slot)) = cmdlist.last_mut() {
            *slot = Some(operands[0].clone());
        }
    }

    for (cmd, arg) in &cmdlist {
        if ulimit_internal(
            *cmd,
            arg.as_deref(),
            mode,
            cmdlist.len() > 1,
            env_vars,
            stdout,
            stderr,
            &prefix,
            posix_mode,
        )? == EXECUTION_FAILURE
        {
            return Ok(EXECUTION_FAILURE);
        }
    }

    Ok(EXECUTION_SUCCESS)
}

/// ulimit_internal (ulimit.def:451-517).
#[allow(clippy::too_many_arguments)]
fn ulimit_internal<W, E>(
    cmd: char,
    cmdarg: Option<&str>,
    mode_in: u8,
    multiple: bool,
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
    prefix: &str,
    posix_mode: bool,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let Some(spec) = findlim(cmd) else {
        // ulimit.def:426-431 bad-command arm; unreachable through scan()
        // because the option parser only admits table letters.
        writeln!(stderr, "{prefix}ulimit: `{cmd}': bad command")?;
        return Ok(EX_USAGE);
    };

    let setting = cmdarg.is_some();
    let mut mode = mode_in;
    if mode == 0 {
        mode = if setting {
            LIMIT_HARD | LIMIT_SOFT
        } else {
            LIMIT_SOFT
        };
    }

    let (soft_limit, hard_limit) = match plat::get(cmd, env_vars) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(
                stderr,
                "{prefix}ulimit: {}: cannot get limit: {}",
                spec.description,
                crate::posix_errors::message(&err)
            )?;
            return Ok(EXECUTION_FAILURE);
        }
    };

    if !setting {
        let value = if mode & LIMIT_SOFT != 0 {
            soft_limit
        } else {
            hard_limit
        };
        printone(spec, value, multiple, posix_mode, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    let cmdarg = cmdarg.unwrap_or("");
    let real_limit: u64 = if cmdarg == "hard" {
        hard_limit
    } else if cmdarg == "soft" {
        soft_limit
    } else if cmdarg == "unlimited" {
        UNLIMITED
    } else if digit_gate(cmdarg) {
        let Some(limit) = parse_rlimtype(cmdarg) else {
            sh_invalidnum(cmdarg, prefix, stderr)?;
            return Ok(EXECUTION_FAILURE);
        };
        let factor = blocksize(spec.block, posix_mode);
        // Unsigned multiply wraps, then the round-trip check of
        // ulimit.def:496-500 catches the overflow as "out of range".
        let real = limit.wrapping_mul(factor);
        if real / factor != limit {
            writeln!(stderr, "{prefix}ulimit: {cmdarg}: limit out of range")?;
            return Ok(EXECUTION_FAILURE);
        }
        real
    } else {
        sh_invalidnum(cmdarg, prefix, stderr)?;
        return Ok(EXECUTION_FAILURE);
    };

    if let Err(err) = plat::set(cmd, real_limit, mode, env_vars) {
        writeln!(
            stderr,
            "{prefix}ulimit: {}: cannot modify limit: {}",
            spec.description,
            crate::posix_errors::message(&err)
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    Ok(EXECUTION_SUCCESS)
}

/// The ulimit.def:483 gate: first byte a digit, and the second byte a digit
/// or end-of-string (deeper validation happens in the full parse).
fn digit_gate(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.first().is_some_and(|b| b.is_ascii_digit())
        && (bytes.len() == 1 || bytes.get(1).is_some_and(|b| b.is_ascii_digit()))
}

/// string_to_rlimtype (ulimit.def:138): full-string decimal conversion.
/// Returns None when any trailing character is left (the caller reports
/// sh_invalidnum); saturates at u64::MAX like strtoull.
fn parse_rlimtype(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(s.parse::<u64>().unwrap_or(u64::MAX))
}

/// sh_invalidnum (builtins/common.c:214-226): "0"+digit reports an octal
/// number, a "0x" prefix a hex number, everything else a plain number.
fn sh_invalidnum<E>(s: &str, prefix: &str, stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    let bytes = s.as_bytes();
    let msg = if bytes.first() == Some(&b'0') && bytes.get(1).is_some_and(|b| b.is_ascii_digit()) {
        "invalid octal number"
    } else if s.starts_with("0x") {
        "invalid hex number"
    } else {
        "invalid number"
    };
    writeln!(stderr, "{prefix}ulimit: {s}: {msg}")
}

/// printone (ulimit.def:754-777).
fn printone<W>(
    spec: &LimitSpec,
    curlim: u64,
    pdesc: bool,
    posix_mode: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    if pdesc {
        let unitstr = match spec.units {
            Some(units) => format!("({units}, -{}) ", spec.option),
            None => format!("(-{}) ", spec.option),
        };
        write!(stdout, "{:<20} {:>20}", spec.description, unitstr)?;
    }
    if curlim == UNLIMITED {
        writeln!(stdout, "unlimited")
    } else {
        let factor = blocksize(spec.block, posix_mode);
        writeln!(stdout, "{}", curlim / factor)
    }
}

/// Build the shell diagnostic prefix (`<script>: line N: `) from the shell
/// environment, falling back to `rubash: ` without script context — GNU
/// builtin_error prefixes `./script: line N:` when running a script file
/// (baseline: `./builtins11.sub: line 37: ulimit: -g: invalid option`).
fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}
