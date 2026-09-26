//! path module.
//!
//! GNU Bash source ownership:
// - findcmd.c
// - findcmd.h

use std::collections::HashMap;
use std::fs;
use std::io;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use super::support_names::split_shell_path;
use crate::executor::markers::DATA_DOLLAR;

pub(crate) const COMPATIBLE_SHELL_PATH_ENV: &str = "__RUBASH_COMPATIBLE_SHELL_PATH";

/// Process-wide command-resolution cache.
///
/// GNU bash remembers executed commands in its hash table (findcmd.c +
/// builtins/hash.def) so a repeated name never pays a full PATH scan twice.
/// Rubash's user-visible `hash` table is serialized into env_vars, which is
/// too slow to consult per lookup, so external resolution keeps a separate
/// in-memory map here. On Windows a single miss costs tens of milliseconds
/// (PATH entries x PATHEXT stat probes, plus a possible `winuxcmd help
/// <name>` child process), so both hits and misses are cached.
struct CommandLookupCache {
    fingerprint: String,
    results: HashMap<String, Option<PathBuf>>,
}

static COMMAND_LOOKUP_CACHE: OnceLock<Mutex<CommandLookupCache>> = OnceLock::new();

fn command_lookup_cache() -> &'static Mutex<CommandLookupCache> {
    COMMAND_LOOKUP_CACHE.get_or_init(|| {
        Mutex::new(CommandLookupCache {
            fingerprint: String::new(),
            results: HashMap::new(),
        })
    })
}

/// Forget all remembered command locations (`hash -r`, tests).
pub(crate) fn clear_command_lookup_cache() {
    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.results.clear();
    cache.fingerprint.clear();
}

/// Forget one remembered command location.
///
/// GNU `hash -d NAME` (builtins/hash.def) calls `phash_remove(w)` to drop a
/// single entry from the hash table. Rubash keeps the user-visible hash table
/// in `__RUBASH_HASH_TABLE` and the in-memory lookup cache here; both must be
/// invalidated together or the next `find_user_command(name)` returns the
/// stale cached path.
pub(crate) fn remove_command_lookup_cache(name: &str) {
    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.results.remove(name);
}

/// Insert or override one remembered command location.
///
/// GNU `hash -p PATH NAME` (builtins/hash.def) calls `phash_insert(w,
/// pathname)` so the next lookup of `name` returns `pathname` without a PATH
/// scan. `hash NAME` rehash also lands here after a fresh PATH search. The
/// internal cache must mirror the user-visible table or `find_user_command`
/// keeps returning the old result.
pub(crate) fn set_command_lookup_cache(name: &str, path: Option<PathBuf>) {
    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.results.len() >= 4096 {
        cache.results.clear();
    }
    cache.results.insert(name.to_string(), path);
}

/// Environment values that can change a lookup result. The cache resets
/// whenever any of them differ, which covers `PATH=`/`PATH` unset without
/// needing a hook in the assignment path.
fn command_lookup_fingerprint(env_vars: &HashMap<String, String>) -> String {
    const ENV_KEYS: &[&str] = &[
        "PATH",
        "PATHEXT",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "__RUBASH_SHELL_ROOT",
        "WINUXSH_ROOT",
        "RUBASH_ROOT",
        "COREUTILS_PATH",
        "SHELL_COREUTILS_DIR",
        "WINUXCMD",
        "WINUXCMD_PATH",
        "WINUXCMD_HOME",
        COMPATIBLE_SHELL_PATH_ENV,
        "RUBASH_COMPATIBLE_SHELL_PATH",
    ];
    // Fallbacks consulted via std::env::var when the env_vars key is absent
    // (executable_extensions, windows_real_home_path).
    const PROCESS_FALLBACK_KEYS: &[&str] = &["PATHEXT", "HOME", "USERPROFILE"];

    let mut fingerprint = String::new();
    for key in ENV_KEYS {
        fingerprint.push_str(key);
        fingerprint.push('=');
        if let Some(value) = env_vars.get(*key) {
            fingerprint.push_str(value);
        }
        fingerprint.push(DATA_DOLLAR);
    }
    for key in PROCESS_FALLBACK_KEYS {
        if let Ok(value) = std::env::var(key) {
            fingerprint.push_str(key);
            fingerprint.push('~');
            fingerprint.push_str(&value);
            fingerprint.push(crate::executor::markers::SUBSCRIPT_CARRIER);
        }
    }
    fingerprint
}

pub(crate) fn shell_path_entries(path: &str) -> Vec<String> {
    split_shell_path(path)
}

/// A directory entry in the shell namespace.
#[derive(Debug, Clone)]
pub(crate) struct ShellDirectoryEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
    pub(crate) is_file: bool,
}

/// Enumerate a shell-visible directory from its real Windows backing path.
pub(crate) fn shell_directory_entries(
    path: &str,
    env_vars: &HashMap<String, String>,
) -> io::Result<Vec<ShellDirectoryEntry>> {
    let physical_dir = shell_path_to_windows(path, env_vars);
    let mut entries = Vec::new();
    let mut physical_error = None;

    match fs::read_dir(&physical_dir) {
        Ok(directory) => {
            for entry in directory {
                let entry = entry?;
                let name = shell_path_display_from_windows(&entry.file_name().to_string_lossy());
                let file_type = entry.file_type()?;
                entries.push(ShellDirectoryEntry {
                    name,
                    path: entry.path(),
                    is_dir: file_type.is_dir(),
                    is_file: file_type.is_file(),
                });
            }
        }
        Err(error) => physical_error = Some(error),
    }

    if entries.is_empty() {
        if let Some(error) = physical_error {
            return Err(error);
        }
    }

    Ok(entries)
}

pub fn find_user_command(name: &str, env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    // GNU findcmd.c:356-365: `hashing_enabled` is the runtime mirror of
    // `set -h` / `set +h` (the `hashall` shell option). When it is off,
    // search_for_command skips phash_search AND phash_insert entirely, so
    // every lookup pays a full PATH scan and nothing is remembered.
    if !crate::builtins::set::shell_option_enabled(env_vars, "hashall") {
        return find_user_command_uncached(name, env_vars);
    }

    // GNU findcmd.c:356-359: if PATH is in the temporary command environment
    // (PATH=foo cmd), search_for_command skips phash_search AND phash_insert
    // entirely. Rubash tags temp-PATH state with __RUBASH_TEMP_PATH (set in
    // apply_temporary_assignments, cleared in restore_temporary_assignments).
    if env_vars.get("__RUBASH_TEMP_PATH").map(String::as_str) == Some("1") {
        return find_user_command_uncached(name, env_vars);
    }

    let fingerprint = command_lookup_fingerprint(env_vars);
    {
        let mut cache = command_lookup_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.fingerprint != fingerprint {
            cache.results.clear();
            cache.fingerprint = fingerprint.clone();
        } else if let Some(result) = cache.results.get(name) {
            // GNU findcmd.c:367-380: if check_hashed_filenames (the `checkhash`
            // shopt) is active, stat the cached path on every hit; a file
            // that no longer exists or is not executable is removed from the
            // hash table and PATH search resumes.
            if let Some(path) = result {
                if crate::builtins::shopt::checkhash_enabled() && !cached_path_still_valid(path) {
                    cache.results.remove(name);
                    // fall through to re-search PATH below
                } else {
                    return Some(path.clone());
                }
            } else {
                return result.clone();
            }
        }
    }

    let result = find_user_command_uncached(name, env_vars);

    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.fingerprint != fingerprint {
        cache.results.clear();
        cache.fingerprint = fingerprint;
    }
    if cache.results.len() >= 4096 {
        cache.results.clear();
    }
    cache.results.insert(name.to_string(), result.clone());
    result
}

/// GNU findcmd.c:373 `file_status(hashed_file)` checks FS_EXISTS|FS_EXECABLE.
/// On Windows executability is extension-driven, so a plain existence+file
/// check is the closest equivalent; the full PATHEXT probe would be too
/// expensive per cache hit.
fn cached_path_still_valid(path: &Path) -> bool {
    path.exists() && path.is_file()
}

fn find_user_command_uncached(name: &str, env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if has_path_separator(name) {
        // `/bin/bash` / `/usr/bin/bash` normally resolve to the literal
        // file (execve semantics). The substitute chain below (explicit
        // compatible-shell env, winuxsh, then a PATH search for `bash`)
        // is the Windows fixture: hosts without /bin need SOME bash to
        // satisfy suite shebangs. On unix the chain would invert lookup
        // priority — e.g. on macOS a homebrew bash at the front of PATH
        // would shadow the real /bin/bash that the word names — so unix
        // takes the literal probe directly (GNU findcmd.c:383-385:
        // absolute_program => savestring(pathname), no PATH walk; WSL
        // probe: PATH=<fakebash>:$PATH /bin/bash runs the real shell).
        #[cfg(windows)]
        if is_standard_unix_bash_path(name) {
            if let Some(found) = configured_compatible_shell(env_vars) {
                return Some(found);
            }
            if let Some(found) = configured_shell_root_winuxsh(env_vars) {
                return Some(found);
            }
            if let Some(found) = find_user_command("bash", env_vars) {
                return Some(found);
            }
        }
        let candidate = shell_path_to_windows(name, env_vars);
        if let Some(found) = executable_candidate(&candidate, env_vars) {
            return Some(found);
        }
        #[cfg(windows)]
        if let Some(found) = find_winuxcmd_absolute_command(name, env_vars) {
            return Some(found);
        }
        // `/bin/X` and `/usr/bin/X` name commands from the system tool
        // namespace, which has no Windows filesystem location when no
        // shell root is configured. Resolve the basename through PATH —
        // the Windows equivalent of that namespace (Git usr/bin, WinuxCmd
        // links, the suite's own fixture dir). Suites spawning `/bin/sh`,
        // `/bin/cat`, `/bin/echo` etc. then exercise real subprocess
        // semantics instead of "command not found". Names that must stay
        // unresolvable (zsh/ksh/csh, /bin/qux, /etc/...) simply miss on
        // PATH as well, preserving the 127 result.
        //
        // Unix gate (E6): GNU findcmd.c:383-385 makes a slash-bearing name
        // an absolute_program — `command = savestring (pathname)` with no
        // PATH walk — so a missing /bin/X must stay not-found (127), never
        // re-resolved by basename.
        #[cfg(windows)]
        if let Some(base) = unix_bin_basename(name) {
            if let Some(found) = find_user_command(base, env_vars) {
                return Some(found);
            }
        }
        return None;
    }

    // GNU findcmd.c:623 find_user_command_in_path walks with
    // FS_EXEC_PREFERRED|FS_NODIRS: an executable regular file returns
    // immediately (findcmd.c:580-586); an existing-but-not-executable one is
    // remembered as file_to_lose_on (findcmd.c:591) and returned after the
    // walk only if no executable ever matched (findcmd.c:695) -- the caller
    // then attempts the exec and fails with EACCES ("Permission denied",
    // 126), not not-found (127). Windows has no execute bit, so the
    // preference tier collapses to the first-existing-file behavior.
    #[cfg(unix)]
    let mut file_to_lose_on: Option<PathBuf> = None;
    for dir in split_shell_path(env_vars.get("PATH").map(String::as_str).unwrap_or_default()) {
        let candidate = shell_path_to_windows(&dir, env_vars).join(name);
        #[cfg(unix)]
        {
            if !candidate.is_file() {
                continue;
            }
            if file_is_executable(&candidate) {
                return Some(candidate);
            }
            if file_to_lose_on.is_none() {
                file_to_lose_on = Some(candidate);
            }
        }
        #[cfg(not(unix))]
        {
            if let Some(found) = executable_candidate(&candidate, env_vars) {
                return Some(found);
            }
        }
    }
    #[cfg(unix)]
    if let Some(fallback) = file_to_lose_on {
        return Some(fallback);
    }

    // A workspace may expose WinuxCmd as one dispatcher executable instead of
    // one wrapper per command.  Ask the dispatcher whether it owns the name
    // before returning it; unknown names must retain Bash's 127 behavior.
    #[cfg(windows)]
    if let Some(dispatcher) = find_winuxcmd_dispatcher(env_vars) {
        if winuxcmd_has_command(&dispatcher, name, env_vars) {
            return Some(dispatcher);
        }
    }

    None
}

pub fn standard_path(_env_vars: &HashMap<String, String>) -> String {
    if cfg!(windows) {
        if configured_shell_root(_env_vars).is_some() {
            return "/usr/local/bin:/usr/bin:/bin".to_string();
        }
        let mut dirs = vec![
            PathBuf::from(r"C:\Windows\System32"),
            PathBuf::from(r"C:\Windows"),
        ];
        // command.def: `command -p` must guarantee a PATH that finds the
        // standard utilities (confstr _CS_PATH). Windows has no system
        // POSIX bin directory — the standard utilities live wherever the
        // host keeps its toolset (Git usr/bin, WinuxCmd links), discovered
        // from the real PATH as the first directory holding a full set.
        #[cfg(windows)]
        if let Some(dir) = windows_posix_tools_dir(_env_vars) {
            dirs.push(dir);
        }
        return dirs
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(";");
    }

    // GNU general.c:1414 conf_standard_path(): try confstr(_CS_PATH) — the
    // POSIX.2 value "guaranteed to find all of the standard utilities"
    // (WSL glibc: /bin:/usr/bin, so `command -p -v ls` finds /bin/ls) —
    // and fall back to STANDARD_UTILS_PATH from config-top.h:70-73 when
    // confstr reports nothing. findcmd.c:391 feeds exactly this string to
    // find_user_command_in_path for CMDSRCH_STDPATH lookups.
    #[cfg(unix)]
    {
        if let Some(path) = confstr_cs_path() {
            return path;
        }
        "/bin:/usr/bin:/sbin:/usr/sbin".to_string()
    }

    // Non-windows, non-unix target of last resort (none today).
    #[cfg(not(unix))]
    "/usr/local/bin:/usr/bin:/bin".to_string()
}

/// confstr(_CS_PATH) ported from general.c:1419-1430 conf_standard_path():
/// first call sizes the buffer, second fills it (NUL included in the
/// returned length). len == 0 means "no value" and selects the
/// STANDARD_UTILS_PATH fallback, matching GNU's `len > 0` guard.
#[cfg(unix)]
fn confstr_cs_path() -> Option<String> {
    unsafe {
        let len = libc::confstr(libc::_CS_PATH, std::ptr::null_mut(), 0);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let written = libc::confstr(libc::_CS_PATH, buf.as_mut_ptr().cast(), buf.len());
        if written == 0 {
            return None;
        }
        // confstr NUL-terminates the buffer; take bytes up to the NUL.
        let value = buf
            .split(|&byte| byte == 0)
            .next()
            .unwrap_or_default();
        let value = String::from_utf8_lossy(value);
        (!value.is_empty()).then(|| value.into_owned())
    }
}

pub fn find_shell(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(shell) = configured_compatible_shell(env_vars) {
        return Some(shell);
    }

    if cfg!(windows) {
        return None;
    }

    ["sh", "bash"]
        .into_iter()
        .find_map(|name| find_user_command(name, env_vars))
        .or_else(find_standard_unix_shell)
}

fn configured_compatible_shell(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    let value = env_vars
        .get(COMPATIBLE_SHELL_PATH_ENV)
        .filter(|value| !value.is_empty())?;
    let candidate = shell_path_to_windows(value, env_vars);
    executable_candidate(&candidate, env_vars)
}

#[cfg(windows)]
fn configured_shell_root_winuxsh(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    let candidate = configured_shell_root(env_vars)?.join("winuxsh.exe");
    executable_candidate(&candidate, env_vars)
}

pub fn should_run_with_shell(path: &Path) -> bool {
    if cfg!(windows) {
        if matches!(
            path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase),
            Some(ext) if matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd")
        ) {
            return false;
        }
        // CreateProcess runs a PE image regardless of its file extension, so
        // an extension-less binary copy (posixexp.tests `cp ${THIS_SH}
        // $TMPDIR/sh`, then `$TMPDIR/sh -c ...`) must still exec directly —
        // mirroring GNU shell_execve, which only reaches the shell fallback
        // when execve itself refuses.
        if path.extension().is_none()
            && std::fs::read(path).is_ok_and(|sample| sample.starts_with(b"MZ"))
        {
            return false;
        }
        return true;
    }
    false
}

#[allow(dead_code)]
pub fn external_command_for_program(
    program: &Path,
    args: &[String],
    env_vars: &HashMap<String, String>,
) -> (Command, bool) {
    external_command_for_named_program(program, None, args, env_vars)
}

/// Append already-expanded arguments to a child command line.
///
/// GNU shell_execve (execute_cmd.c:6139+) hands execve the word list that
/// expand_word_internal produced, so a quoted wildcard reaches the child
/// literally and nothing ever expands it again. MSYS2- and WinuxCmd-hosted
/// children instead glob wildcard characters in *unquoted* command-line
/// arguments themselves (CRT wildcard expansion): `ls "*.txt"` reached the
/// child as a bare `*.txt` token and was expanded a second time
/// (niubash#119, rubash-side of #83). Rubash already ran pathname expansion
/// with quote suppression in expand_command_words, so every `*`/`?`/`[`
/// surviving in an argument is literal — emit those arguments quoted so the
/// child runtime does not re-expand them. Command processors that reparse
/// their own line (cmd.exe batch, PowerShell -File) keep plain .args().
fn push_external_args(command: &mut Command, args: &[String]) {
    for arg in args {
        #[cfg(windows)]
        {
            if arg.contains(['*', '?', '[']) {
                command.raw_arg(windows_quoted_wildcard_arg(arg));
                continue;
            }
        }
        command.arg(arg);
    }
}

/// Quote one argument for a Windows command line so the child's argv sees
/// the text literally (no CRT wildcard expansion). Follows the
/// CommandLineToArgvW rules: wrap in double quotes, escape embedded `"` as
/// `\"`, and double a backslash run that directly precedes the closing quote.
#[cfg(windows)]
fn windows_quoted_wildcard_arg(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        for _ in 0..backslashes {
            quoted.push('\\');
        }
        backslashes = 0;
        if ch == '"' {
            quoted.push_str("\\\"");
        } else {
            quoted.push(ch);
        }
    }
    for _ in 0..backslashes {
        quoted.push_str("\\\\");
    }
    quoted.push('"');
    quoted
}

pub fn external_command_for_named_program(
    program: &Path,
    command_name: Option<&str>,
    args: &[String],
    env_vars: &HashMap<String, String>,
) -> (Command, bool) {
    // Native command processors own slash-prefixed switches such as `/C`.
    // Do not reinterpret those switches as paths under Winuxsh's logical
    // shell root; doing so starts cmd.exe without its command string.
    let preserve_native_args = is_windows_command_processor(program);
    let native_args = args
        .iter()
        .map(|arg| {
            if preserve_native_args {
                arg.clone()
            } else {
                external_argument_path(arg, env_vars)
            }
        })
        .collect::<Vec<_>>();

    if is_windows_powershell_script(program) {
        let program = cmd_compatible_windows_path(program);
        let mut command = Command::new(windows_powershell_processor(env_vars));
        command
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(program);
        command.args(&native_args);
        return (command, false);
    }

    if is_windows_batch_file(program) {
        let program = cmd_compatible_windows_path(program);
        let mut command = Command::new(windows_command_processor());
        command.arg("/D").arg("/C").arg(program);
        command.args(&native_args);
        return (command, false);
    }

    if should_run_with_shell(program) {
        if let Some(shell) = find_shell(env_vars) {
            let mut command = Command::new(shell);
            command.arg(program);
            push_external_args(&mut command, &native_args);
            return (command, true);
        }
        if let Some(shell) = current_shell_processor() {
            let mut command = Command::new(shell);
            command.arg(program);
            push_external_args(&mut command, &native_args);
            return (command, true);
        }
    }

    let mut command = Command::new(program);
    if is_winuxcmd_dispatcher(program) {
        if let Some(command_name) = command_name {
            let dispatch_name = dispatcher_command_name(command_name);
            if !matches!(
                dispatch_name.to_ascii_lowercase().as_str(),
                "winuxcmd" | "winuxcmd.exe"
            ) {
                command.arg(dispatch_name);
            }
        }
    }
    push_external_args(&mut command, &native_args);
    // For `sh -c '...'` without an explicit $0, Bash sets $0 to the shell
    // name (e.g. "/bin/sh" for `/bin/sh -c 'echo $0'`). WinuxCmd's sh.exe
    // defaults to "niu" in that case, so inject the logical name as $0
    // (histexp.tests: `!2` expands to `/bin/sh -c 'echo this is $0'` and
    // expects "this is /bin/sh").
    if let Some(name) = command_name {
        let lower = name.replace('\\', "/").to_ascii_lowercase();
        let is_sh =
            lower == "sh" || lower == "bash" || lower.ends_with("/sh") || lower.ends_with("/bash");
        if is_sh && native_args.len() == 2 && native_args[0] == "-c" {
            command.arg(name);
        }
    }
    (command, false)
}

/// Whether the host requested shell-native (Windows-style) PWD display.
///
/// Reads the neutral `__RUBASH_PATH_STYLE` first; `WINUXSH_SHELL_PATH_STYLE`
/// is kept as a compatibility fallback for hosts that still export the
/// pre-rename name (remove the fallback once niu stops setting it).
pub fn shell_path_style_enabled() -> bool {
    cfg!(windows)
        && ["__RUBASH_PATH_STYLE", "WINUXSH_SHELL_PATH_STYLE"]
            .into_iter()
            .any(|name| std::env::var_os(name).is_some())
}

fn is_winuxcmd_dispatcher(path: &Path) -> bool {
    cfg!(windows)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.eq_ignore_ascii_case("winuxcmd"))
}

#[cfg(windows)]
fn find_winuxcmd_dispatcher(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    // Neutral names first; WINUXCMD/WINUXCMD_PATH are pre-rename
    // compatibility fallbacks (remove once hosts stop exporting them).
    for name in [
        "COREUTILS_PATH",
        "SHELL_COREUTILS_DIR",
        "WINUXCMD",
        "WINUXCMD_PATH",
    ] {
        if let Some(value) = env_vars.get(name) {
            let candidate = shell_path_to_windows(value, env_vars);
            if let Some(found) = executable_candidate(&candidate, env_vars) {
                return Some(found);
            }
        }
    }

    // Winuxsh owns WinuxCmd selection. Rubash must not guess a dispatcher from
    // PATH because a process can contain command links from a different
    // WinuxCmd installation. Embedders may still provide an explicit path via
    // WINUXCMD_PATH/WINUXCMD or `Executor::set_winuxcmd_path`.
    None
}

#[cfg(windows)]
fn winuxcmd_has_command(dispatcher: &Path, name: &str, env_vars: &HashMap<String, String>) -> bool {
    let mut command = Command::new(dispatcher);
    command.arg("help").arg(name);
    for key in ["SystemRoot", "WINDIR", "ComSpec"] {
        if let Some(value) = env_vars.get(key) {
            command.env(key, value);
        }
    }
    command.output().is_ok_and(|output| output.status.success())
}

#[cfg(windows)]
fn find_winuxcmd_absolute_command(
    name: &str,
    env_vars: &HashMap<String, String>,
) -> Option<PathBuf> {
    let command_name = logical_bin_command_name(name)?;
    let dispatcher = find_winuxcmd_dispatcher(env_vars)?;
    winuxcmd_has_command(&dispatcher, &command_name, env_vars).then_some(dispatcher)
}

/// Return the native directory a Windows child should receive for one shell
/// PATH entry. Logical command directories are real directories below the
/// configured shell root, so no provider directory needs to be appended.
pub(crate) fn shell_path_process_entries(
    path: &str,
    env_vars: &HashMap<String, String>,
) -> Vec<PathBuf> {
    let physical = shell_path_to_windows(path, env_vars);
    vec![physical]
}

/// Materialize a shell PATH for a native child process.
///
/// Logical shell PATH entries are converted to their real Windows directories
/// before a native child process is started.
pub(crate) fn shell_path_to_process(path: &str, env_vars: &HashMap<String, String>) -> String {
    let separator = if cfg!(windows) { ';' } else { ':' };
    shell_path_entries(path)
        .into_iter()
        .flat_map(|entry| shell_path_process_entries(&entry, env_vars))
        // The `/`-to-`\` rewrite is the Windows native-child spelling; a
        // unix child must receive PATH entries verbatim.
        .map(|entry| {
            if cfg!(windows) {
                entry.to_string_lossy().replace('/', "\\")
            } else {
                entry.to_string_lossy().into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(&separator.to_string())
}

#[cfg(windows)]
fn logical_bin_command_name(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    let rest = normalized
        .strip_prefix("/bin/")
        .or_else(|| normalized.strip_prefix("/usr/bin/"))
        .or_else(|| normalized.strip_prefix("/usr/local/bin/"))?;
    if rest.is_empty() || rest.contains('/') || rest.contains('\\') {
        return None;
    }
    Some(rest.to_string())
}

fn repair_windows_drive_slash_argument(arg: &str) -> Option<String> {
    let bytes = arg.as_bytes();
    if bytes.len() < 4
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'/'
        || bytes[3] != b'/'
    {
        return None;
    }

    // Winuxsh's host boundary can turn an unquoted `C:\\...` into
    // `C://...`. Repair only that unambiguous drive-shaped artifact; ordinary
    // slash paths and native arguments remain untouched.
    let mut repaired = String::with_capacity(arg.len());
    repaired.push(bytes[0] as char);
    repaired.push(':');
    let mut previous_was_slash = false;
    for ch in arg[2..].chars() {
        if ch == '/' {
            if !previous_was_slash {
                repaired.push('\\');
            }
            previous_was_slash = true;
        } else {
            repaired.push(ch);
            previous_was_slash = false;
        }
    }
    Some(repaired)
}

fn external_argument_path(arg: &str, env_vars: &HashMap<String, String>) -> String {
    if cfg!(windows) {
        if let Some(repaired) = repair_windows_drive_slash_argument(arg) {
            return repaired;
        }

        // A leading backslash is a valid native argument spelling (and is
        // commonly used by utilities for escape sequences such as `\\n`).
        // Only slash-prefixed shell paths belong to the logical root; treating
        // `\\n` as `/n` changes data arguments into paths under WINUXSH_ROOT.
        if arg.starts_with('\\') {
            return arg.to_string();
        }
        let normalized = arg.replace('\\', "/");
        if !normalized.starts_with('/') {
            return arg.to_string();
        }

        let drive_path = normalized.len() >= 3
            && normalized.as_bytes()[0] == b'/'
            && normalized.as_bytes()[2] == b'/'
            && normalized.as_bytes()[1].is_ascii_alphabetic();
        if drive_path {
            let drive = normalized.as_bytes()[1].to_ascii_lowercase();
            let translated = shell_path_to_windows(arg, env_vars);
            // `/c/...` is Rubash's explicit POSIX display-path spelling and
            // must be translated even before the target is created. Other
            // `/X/...` arguments are ambiguous (regexes, git pathspecs,
            // sed/awk fragments); convert those only when they resolve to a
            // real filesystem path.
            if drive != b'c' && normalized.len() <= 3 {
                return arg.to_string();
            }
            if drive != b'c' && !translated.exists() {
                return arg.to_string();
            }
            return translated.to_string_lossy().into_owned();
        }
        if windows_external_absolute_argument_needs_translation(&normalized, env_vars) {
            return shell_path_to_windows(arg, env_vars)
                .to_string_lossy()
                .into_owned();
        }

        arg.to_string()
    } else {
        arg.to_string()
    }
}

fn windows_external_absolute_argument_needs_translation(
    normalized: &str,
    env_vars: &HashMap<String, String>,
) -> bool {
    if normalized == "/" {
        return configured_shell_root(env_vars).is_some();
    }

    // /dev/* pseudo-device operands pass through literally: POSIX-aware
    // children resolve them against their own descriptor table (winuxcmd's
    // native_path layer maps /dev/std* and /dev/fd/N to the real fds,
    // MSYS2/Cygwin tools via their emulation, and /dev/null -> NUL). Shell-
    // side translation to CONOUT$/CONIN$/NUL either dangles (a shell-root
    // `dev/stdout` path) or pins the child to the console device instead
    // of its actual fd — `echo x | tee /dev/stdout | wc -l` must write the
    // pipe, not CONOUT$ (unixwin/rubash#120). Redirect targets still go
    // through shell_path_to_windows, which owns the fd/console mapping.
    if normalized == "/dev" || normalized.starts_with("/dev/") {
        return false;
    }

    if normalized == "/tmp" || normalized.starts_with("/tmp/") {
        return true;
    }

    if normalized == "/var/tmp" || normalized.starts_with("/var/tmp/") {
        return true;
    }

    if normalized == "/home" || normalized.starts_with("/home/") {
        return windows_real_home_path(env_vars).is_some()
            || configured_shell_root(env_vars).is_some();
    }

    // /mnt/X drive paths need translation for all drive letters.
    // Must be exactly /mnt/X or /mnt/X/... to avoid false matches like /mnt/cfoo.
    if normalized.starts_with("/mnt/") && normalized.len() >= 6 {
        let bytes = normalized.as_bytes();
        if bytes[5].is_ascii_alphabetic() && (normalized.len() == 6 || bytes[6] == b'/') {
            return true;
        }
    }

    if configured_shell_root(env_vars).is_some()
        && matches!(
            normalized.split('/').nth(1),
            Some("bin" | "etc" | "lib" | "lib64" | "opt" | "sbin" | "usr" | "var")
        )
    {
        return shell_path_to_windows(normalized, env_vars).exists();
    }

    false
}

fn dispatcher_command_name(command_name: &str) -> String {
    if let Some(name) = logical_bin_command_name_any_platform(command_name) {
        return name;
    }
    command_name
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(command_name)
        .to_string()
}

fn logical_bin_command_name_any_platform(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    let rest = normalized
        .strip_prefix("/bin/")
        .or_else(|| normalized.strip_prefix("/usr/bin/"))
        .or_else(|| normalized.strip_prefix("/usr/local/bin/"))?;
    if rest.is_empty() || rest.contains('/') || rest.contains('\\') {
        return None;
    }
    Some(rest.to_string())
}

fn current_shell_processor() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn is_windows_powershell_script(path: &Path) -> bool {
    cfg!(windows)
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ps1"))
}

fn is_windows_batch_file(path: &Path) -> bool {
    cfg!(windows)
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "bat" | "cmd"))
}

fn windows_powershell_processor(env_vars: &HashMap<String, String>) -> PathBuf {
    find_user_command("pwsh", env_vars)
        .or_else(|| find_user_command("powershell", env_vars))
        .or_else(|| {
            let system_root = env_vars
                .get("SystemRoot")
                .or_else(|| env_vars.get("WINDIR"))
                .cloned()
                .or_else(|| std::env::var("SystemRoot").ok())
                .or_else(|| std::env::var("WINDIR").ok())?;
            let path = PathBuf::from(system_root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe");
            path.is_file().then_some(path)
        })
        .unwrap_or_else(|| PathBuf::from("pwsh"))
}

fn windows_command_processor() -> PathBuf {
    std::env::var_os("ComSpec")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"))
}

fn is_windows_command_processor(path: &Path) -> bool {
    cfg!(windows)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
}

fn cmd_compatible_windows_path(path: &Path) -> PathBuf {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !cfg!(windows) {
        return path;
    }

    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(windows)]
pub fn apply_required_windows_child_environment(
    process: &mut Command,
    env_vars: &HashMap<String, String>,
) {
    for name in ["SystemRoot", "WINDIR", "ComSpec"] {
        let value = env_vars
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok());
        if let Some(value) = value {
            if !value.contains('\0') {
                process.env(name, value);
            }
        }
    }

    let home = env_vars
        .get("USERPROFILE")
        .cloned()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .or_else(|| {
            env_vars.get("HOME").map(|value| {
                shell_path_to_windows(value, env_vars)
                    .to_string_lossy()
                    .into_owned()
            })
        });

    if let Some(home) = home.filter(|value| !value.trim().is_empty() && !value.contains('\0')) {
        let native_home = home.replace('/', "\\");
        process.env("USERPROFILE", &native_home);
        process.env("HOME", &native_home);
        if let Some((drive, path)) = windows_drive_and_home_path(&native_home) {
            process.env("HOMEDRIVE", drive);
            process.env("HOMEPATH", path);
        }
        let base = native_home.trim_end_matches('\\');
        process.env("APPDATA", format!("{base}\\AppData\\Roaming"));
        process.env("LOCALAPPDATA", format!("{base}\\AppData\\Local"));
    }
}

#[cfg(not(windows))]
pub fn apply_required_windows_child_environment(
    _process: &mut Command,
    _env_vars: &HashMap<String, String>,
) {
}

#[cfg(windows)]
fn windows_drive_and_home_path(path: &str) -> Option<(String, String)> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let drive = path[..2].to_string();
    let rest = path[2..].trim_start_matches(['\\', '/']);
    Some((drive, format!("\\{}", rest.replace('/', "\\"))))
}

/// GNU findcmd.c:113 file_status: FS_EXECABLE requires eaccess(name, X_OK)
/// == 0 (the access() fallback branch at findcmd.c:156-157). Real-uid
/// access matches that fallback; the effective-uid variant only differs for
/// setuid shells.
#[cfg(unix)]
fn file_is_executable(path: &Path) -> bool {
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    unsafe { libc::access(cpath.as_ptr(), libc::X_OK) == 0 }
}

fn executable_candidate(path: &Path, env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    // Extensionless names probe every PATHEXT candidate before the bare file
    // (native wrappers like `code.cmd` win over an extensionless `code`).
    // The post-is_file pass below must not repeat that same scan on a miss.
    let probed_extensions = cfg!(windows) && path.extension().is_none();
    if probed_extensions {
        if let Some(candidate) = executable_extension_candidate(path, env_vars) {
            return Some(candidate);
        }
    }

    if path.is_file() {
        return Some(path.to_path_buf());
    }

    if cfg!(windows) && !probed_extensions {
        return executable_extension_candidate(path, env_vars);
    }

    None
}

fn executable_extension_candidate(
    path: &Path,
    env_vars: &HashMap<String, String>,
) -> Option<PathBuf> {
    for ext in executable_extensions(env_vars) {
        let candidate = path.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn executable_extensions(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut exts = env_vars
        .get("PATHEXT")
        .cloned()
        .or_else(|| std::env::var("PATHEXT").ok())
        .map(|value| {
            value
                .split(';')
                .filter_map(|ext| ext.trim().trim_start_matches('.').split_whitespace().next())
                .filter(|ext| !ext.is_empty())
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_else(|| vec!["exe".into(), "com".into(), "bat".into(), "cmd".into()]);

    for ext in ["exe", "com", "bat", "cmd", "ps1"] {
        if !exts
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(ext))
        {
            exts.push(ext.into());
        }
    }
    exts
}

fn find_standard_unix_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }

    ["/bin/sh", "/usr/bin/sh", "/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn has_path_separator(name: &str) -> bool {
    // GNU general.c:843 absolute_program(): only `/` makes a name absolute,
    // except under __MSYS__ where `\\` counts too — exactly the Windows /
    // unix split below. On unix `foo\bar` is an ordinary filename looked up
    // in PATH, not a path-bearing name.
    if cfg!(windows) {
        name.contains('/') || name.contains('\\')
    } else {
        name.contains('/')
    }
}

fn is_standard_unix_bash_path(name: &str) -> bool {
    matches!(
        name.replace('\\', "/").as_str(),
        "/bin/bash" | "/usr/bin/bash"
    )
}

/// Basename of a single-component `/bin/X` or `/usr/bin/X` absolute path.
/// Deeper paths (`/bin/foo/bar`) return None: they are real filesystem
/// locations, not entries in the system tool namespace.
#[cfg(windows)]
fn unix_bin_basename(name: &str) -> Option<&str> {
    let normalized = name.replace('\\', "/");
    let base = normalized
        .strip_prefix("/bin/")
        .or_else(|| normalized.strip_prefix("/usr/bin/"))?;
    if base.is_empty() || base.contains('/') {
        return None;
    }
    // `normalized` is a local String; return the basename taken from
    // `name` itself so the borrowed tail outlives the function.
    name.rsplit(['/', '\\'])
        .next()
        .filter(|base| !base.is_empty())
}

pub(crate) fn shell_path_to_windows(path: &str, env_vars: &HashMap<String, String>) -> PathBuf {
    // On Windows, a single leading backslash indicates a UNC path whose
    // prefix was consumed by shell backslash escaping.  For example, the
    // user types `cd \\DFDB-A1`, bash escaping reduces `\\` to `\`, and
    // the cd builtin receives `\DFDB-A1`.  Restore the UNC prefix so the
    // path is not misinterpreted as a logical-shell-relative path under
    // the shell root.  Paths like `\C:\...` (root-relative with drive
    // letter) are left to the normal normalization path.
    if cfg!(windows) && path.starts_with('\\') && !path.starts_with("\\\\") {
        let rest = &path[1..];
        if !(rest.len() >= 2
            && rest.as_bytes()[0].is_ascii_alphabetic()
            && rest.as_bytes()[1] == b':')
        {
            return PathBuf::from(format!("\\\\{}", rest));
        }
    }

    let normalized = path.replace('\\', "/");
    let shell_root = configured_shell_root(env_vars);

    if cfg!(windows) && (normalized == "/dev/null" || normalized.eq_ignore_ascii_case("NUL")) {
        return PathBuf::from("NUL");
    }

    // Map standard I/O pseudo-devices to Windows console devices.
    // CON is the Windows console device that can be used for both input and output.
    // This provides better compatibility with tools expecting POSIX /dev/stdin etc.
    if cfg!(windows) {
        match normalized.as_str() {
            "/dev/stdin" => return PathBuf::from("CONIN$"),
            "/dev/stdout" | "/dev/stderr" => return PathBuf::from("CONOUT$"),
            // GNU open("/dev/tty") binds the controlling terminal; the
            // Windows console device CON resolves to the input buffer under
            // GENERIC_READ and the screen buffer under GENERIC_WRITE, so a
            // single name covers `< /dev/tty` and `> /dev/tty` alike.
            "/dev/tty" => return PathBuf::from("CON"),
            _ => {}
        }
    }

    // `/dev` is a capability namespace. Only specific /dev paths are mapped
    // on Windows; do not let unsupported fd/tty spellings become ordinary
    // files below the logical root.
    if cfg!(windows) && (normalized == "/dev" || normalized.starts_with("/dev/")) {
        return PathBuf::from(r"\\.\WINUXSH_UNSUPPORTED_DEVICE");
    }

    if cfg!(windows) {
        if let Some(index) = windows_drive_absolute_tail_index(&normalized) {
            return PathBuf::from(normalized[index..].replace('/', "\\"));
        }
    }

    if cfg!(windows)
        && normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b'/'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        let drive = normalized.as_bytes()[1] as char;
        return PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &normalized[3..]).replace('/', "\\"),
        );
    }

    // Map /mnt/X drive paths to Windows drive letters (WSL-style convention).
    // This is a logical mapping, not a real directory - similar to /c/ -> C:\
    // Supports both /mnt/c and /mnt/c/some/path forms for all drive letters.
    // Must be exactly /mnt/X or /mnt/X/... to avoid false matches like /mnt/cfoo.
    if cfg!(windows) && normalized.starts_with("/mnt/") && normalized.len() >= 6 {
        let bytes = normalized.as_bytes();
        if bytes[5].is_ascii_alphabetic() && (normalized.len() == 6 || bytes[6] == b'/') {
            let drive = bytes[5] as char;
            let rest = if normalized.len() > 6 {
                normalized[7..].trim_start_matches('/')
            } else {
                ""
            };
            if rest.is_empty() {
                return PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase()));
            } else {
                return PathBuf::from(
                    format!("{}:\\{}", drive.to_ascii_uppercase(), rest).replace('/', "\\"),
                );
            }
        }
    }

    // /tmp is a per-user temporary namespace, not part of the simulated
    // install tree. Resolve it through TMPDIR (or the process temp dir)
    // even when a shell root is configured: a root below a non-user-writable
    // install dir (e.g. C:\Program Files\Niubash) would otherwise leave
    // /tmp read-only (unixwin/niubash#94).
    if cfg!(windows) && normalized == "/tmp" {
        return windows_tmp_base(env_vars);
    }

    if cfg!(windows) {
        if let Some(rest) = normalized.strip_prefix("/tmp/") {
            return windows_tmp_base(env_vars).join(rest);
        }
    }

    // /var/tmp plays the same temporary-files role as /tmp in GNU tests
    // (vredir.tests and friends default TMPDIR:=/var/tmp). It lives beside
    // the per-user temp base rather than under the shell root for the same
    // writability reason: /var/tmp/<x> resolves under
    // <safe-temp>/var/tmp/<x>, which keeps it distinct from /tmp
    // (=<safe-temp> itself) and away from the generic file names other
    // Windows processes create directly in %TEMP%.
    if cfg!(windows) {
        if let Some(var_tmp) = windows_var_tmp_dir() {
            if normalized == "/var/tmp" {
                return var_tmp;
            }
            if let Some(rest) = normalized.strip_prefix("/var/tmp/") {
                return var_tmp.join(rest);
            }
        }
    }

    if cfg!(windows) {
        if let Some(mapped) = map_windows_home_path(&normalized, env_vars) {
            return mapped;
        }
    }

    // Logical POSIX bin dirs name the system toolset namespace. With no
    // configured shell root, `PATH=/bin:/usr/bin` (invocation.tests) or
    // `/bin/ls` must still reach the host's POSIX utilities — map them to
    // the toolset directory discovered from the real PATH, the same
    // provider standard_path uses for `command -p`.
    if cfg!(windows) && shell_root.is_none() {
        #[cfg(windows)]
        if let Some(dir) = windows_posix_tools_dir(env_vars) {
            const POSIX_BIN_DIRS: &[&str] = &[
                "/bin",
                "/usr/bin",
                "/usr/local/bin",
                "/sbin",
                "/usr/sbin",
                "/usr/local/sbin",
            ];
            for base in POSIX_BIN_DIRS {
                if normalized == *base {
                    return dir;
                }
                if let Some(rest) = normalized
                    .strip_prefix(base)
                    .filter(|rest| rest.starts_with('/'))
                {
                    let candidate = dir.join(rest.trim_start_matches('/').replace('/', "\\"));
                    // The toolset holds `X.exe`; the logical name is bare.
                    // Probe extensions so `/bin/sh` resolves to the real
                    // file — GNU open(2) then reports ENOTDIR for `cd`,
                    // not ENOENT (errors.tests:225).
                    if let Some(found) = executable_candidate(&candidate, env_vars) {
                        return found;
                    }
                    return candidate;
                }
            }
        }
    }

    if let Some(root) = shell_root {
        if let Some(mapped) = map_logical_path(&normalized, &root) {
            return mapped;
        }
    }

    PathBuf::from(shell_path_from_shell_name(path))
}

const WINDOWS_LITERAL_STAR: &str = "%RUBASH_STAR%";
const WINDOWS_LITERAL_QUESTION: &str = "%RUBASH_QMARK%";

fn shell_path_from_shell_name(path: &str) -> String {
    if cfg!(windows) {
        // Path conversion must not rewrite wildcard characters in ordinary data.
        // Globbing is handled before this boundary, while native programs need
        // literal * and ? values to survive unchanged.
        path.replace('/', "\\")
    } else {
        path.to_string()
    }
}

pub(crate) fn shell_path_display_from_windows(name: &str) -> String {
    if cfg!(windows) {
        name.replace(WINDOWS_LITERAL_STAR, "*")
            .replace(WINDOWS_LITERAL_QUESTION, "?")
    } else {
        name.to_string()
    }
}

fn windows_drive_absolute_tail_index(path: &str) -> Option<usize> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 {
        return None;
    }

    let mut leading_drive = None;
    for index in 0..=bytes.len() - 3 {
        if bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && bytes[index + 2] == b'/'
            && (index == 0 || bytes[index - 1] == b'/')
        {
            if index > 0 {
                return Some(index);
            }
            leading_drive = Some(index);
        }
    }

    leading_drive
}

pub(crate) fn shell_path_to_windows_for_lookup(
    path: &str,
    env_vars: &HashMap<String, String>,
) -> PathBuf {
    let mapped = shell_path_to_windows(path, env_vars);
    if let Some(found) = executable_candidate(&mapped, env_vars) {
        return found;
    }

    mapped
}

pub(crate) fn resolve_shell_path_from_env(
    path: &str,
    env_vars: &HashMap<String, String>,
) -> PathBuf {
    shell_path_to_windows(path, env_vars)
}

pub(crate) fn is_shell_null_device(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized == "/dev/null" || (cfg!(windows) && normalized.eq_ignore_ascii_case("NUL"))
}

/// Base directory backing the shell-visible `/tmp` on Windows. Prefers the
/// executor's TMPDIR value; a TMPDIR that is empty or points back into the
/// virtual `/tmp` tree falls back to the process temp dir so resolution can
/// never recurse into itself. Only called under `cfg!(windows)`.
fn windows_tmp_base(env_vars: &HashMap<String, String>) -> PathBuf {
    if let Some(tmpdir) = env_vars.get("TMPDIR") {
        let normalized = tmpdir.replace('\\', "/");
        let normalized = normalized.trim_end_matches('/');
        if !normalized.is_empty() && normalized != "/tmp" && !normalized.starts_with("/tmp/") {
            return shell_path_to_windows(tmpdir, env_vars);
        }
    }
    std::env::temp_dir()
}

/// Windows has no real /var tree. Derive the var-tmp base from the same
/// source rubash uses for its TMPDIR default so /var/tmp stays
/// deterministic, and keep it in a var/tmp subdirectory so it cannot
/// collide with /tmp or with unrelated %TEMP% contents.
pub(in crate::executor) fn windows_var_tmp_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let base = crate::executor::local_helpers::safe_temp_dir_string();
    Some(PathBuf::from(base).join("var").join("tmp"))
}

/// Best-effort creation of the /var/tmp backing directory at shell startup.
/// Open() callers never mkdir, so without this every /var/tmp open fails.
/// The backing directory lives below the per-user temp base even when a
/// shell root is configured, so this also works for read-only install roots.
pub(in crate::executor) fn ensure_var_tmp_dir(_env_vars: &HashMap<String, String>) {
    if let Some(dir) = windows_var_tmp_dir() {
        let _ = std::fs::create_dir_all(dir);
    }
}

fn configured_shell_root(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    // Walk the chain and take the first NON-EMPTY value: an empty entry must
    // not shadow later names (a host exporting an empty __RUBASH_SHELL_ROOT
    // alongside WINUXSH_ROOT/RUBASH_ROOT used to disable root resolution).
    let value = ["__RUBASH_SHELL_ROOT", "WINUXSH_ROOT", "RUBASH_ROOT"]
        .into_iter()
        .find_map(|name| env_vars.get(name).filter(|value| !value.is_empty()))?;

    let normalized = value.replace('\\', "/");
    if cfg!(windows)
        && normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b'/'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        let drive = normalized.as_bytes()[1] as char;
        return Some(PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &normalized[3..]).replace('/', "\\"),
        ));
    }

    Some(PathBuf::from(value))
}

pub(crate) fn shell_root_configured(env_vars: &HashMap<String, String>) -> bool {
    configured_shell_root(env_vars).is_some()
}

/// The host directory holding the POSIX standard utilities — the first
/// real-PATH entry containing a full toolset (sh/cat/rm). Used for
/// `command -p`'s guaranteed-utility PATH (command.def) and for mapping
/// the logical `/bin`/`/usr/bin` namespace when no shell root is
/// configured.
#[cfg(windows)]
pub(crate) fn windows_posix_tools_dir(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    let tools_dir = |path_value: &str| {
        std::env::split_paths(path_value).find(|dir| {
            ["sh.exe", "cat.exe", "rm.exe"]
                .iter()
                .all(|name| dir.join(name).is_file())
        })
    };
    // The toolset location is a host property: a script that overwrites
    // PATH (`PATH=/bin:/usr/bin`, invocation.tests) must not lose it, so
    // Executor::new pins the directory found on the startup PATH into
    // __RUBASH_POSIX_TOOLS_DIR. Probing the live process PATH is useless —
    // env_var writes sync into it before the lookup runs.
    if let Some(pinned) = env_vars
        .get("__RUBASH_POSIX_TOOLS_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
    {
        return Some(pinned);
    }
    env_vars.get("PATH").and_then(|path| tools_dir(path))
}

fn map_windows_home_path(normalized: &str, env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    if normalized != "/home" && !normalized.starts_with("/home/") {
        return None;
    }

    let user_home = windows_real_home_path(env_vars)?;
    let mut mapped = user_home.parent()?.to_path_buf();
    let rest = normalized.strip_prefix("/home")?.trim_start_matches('/');
    for component in rest.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                mapped.pop();
            }
            component => mapped.push(component),
        }
    }
    Some(mapped)
}

fn windows_real_home_path(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(|name| {
            env_vars
                .get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
        })
        .filter(|value| !value.is_empty())
        .find_map(|value| windows_real_home_candidate(&value))
}

fn windows_real_home_candidate(value: &str) -> Option<PathBuf> {
    let normalized = value.replace('\\', "/");
    if normalized == "/home" || normalized.starts_with("/home/") {
        return None;
    }

    if normalized.starts_with("//") {
        return Some(PathBuf::from(value));
    }

    if normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b'/'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        let drive = normalized.as_bytes()[1] as char;
        return Some(PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &normalized[3..]).replace('/', "\\"),
        ));
    }

    if normalized.starts_with('/') {
        return None;
    }

    Some(PathBuf::from(value))
}

/// Return the real installation root for a WinuxCmd executable or bin
/// directory. New installations place the executable in `usr/bin`; legacy
/// flat installations continue to use the executable's parent directory.
pub(crate) fn winuxcmd_installation_root_from_path(path: &Path) -> PathBuf {
    let is_executable = path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("winuxcmd.exe") || name.eq_ignore_ascii_case("winuxcmd")
    });
    let mut directory = if is_executable || path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };

    let is_usr_bin = directory
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("bin"))
        && directory
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("usr"));
    if is_usr_bin {
        if let Some(root) = directory.parent().and_then(Path::parent) {
            directory = root.to_path_buf();
        }
    }

    directory
}

fn map_logical_path(normalized: &str, root: &Path) -> Option<PathBuf> {
    if !normalized.starts_with('/') || normalized.starts_with("//") {
        return None;
    }
    if normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b'/'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        return None;
    }

    let mut components = Vec::new();
    for component in normalized[1..].split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            component => components.push(component),
        }
    }

    let mut mapped = root.to_path_buf();
    for component in components {
        mapped.push(component);
    }
    Some(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::collections::HashSet;
    #[cfg(windows)]
    use std::fs;

    #[cfg(unix)]
    #[test]
    fn path_walk_prefers_executable_and_falls_back_to_existing() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join("rubash-execbit-walk");
        let dir_a = base.join("a");
        let dir_b = base.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let non_exec = dir_a.join("rubash-execwalk");
        std::fs::write(&non_exec, b"#!/bin/sh\n").unwrap();
        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            format!("{}:{}", dir_a.display(), dir_b.display()),
        );
        // file_to_lose_on (findcmd.c:591/695): no executable anywhere -> the
        // first existing non-executable regular file is the command; the
        // exec then fails with EACCES (126), not 127.
        assert_eq!(
            find_user_command_uncached("rubash-execwalk", &env),
            Some(non_exec)
        );
        // FS_EXEC_PREFERRED (findcmd.c:580): an executable in a LATER PATH
        // dir beats the earlier non-executable candidate.
        let exec = dir_b.join("rubash-execwalk");
        std::fs::write(&exec, b"#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&exec).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exec, perms).unwrap();
        assert_eq!(
            find_user_command_uncached("rubash-execwalk", &env),
            Some(exec)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_shell_lookup_falls_back_to_standard_paths() {
        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), "target/rubash-isolated-bin".to_string());

        assert!(find_shell(&env_vars).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn windows_display_path_decodes_literal_glob_markers() {
        assert_eq!(
            shell_path_display_from_windows("C:/tmp/%RUBASH_QMARK%/%RUBASH_STAR%"),
            "C:/tmp/?/*"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_lookup_ignores_compatible_shell_env() {
        let native_exe = std::env::current_exe().unwrap();
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "RUBASH_COMPATIBLE_SHELL_PATH".to_string(),
            native_exe.to_string_lossy().to_string(),
        );
        env_vars.insert("PATH".to_string(), String::new());

        assert_eq!(find_shell(&env_vars), None);
        assert_eq!(find_user_command("sh", &env_vars), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_lookup_does_not_probe_path() {
        let bin_dir = std::env::temp_dir().join("rubash-path-only-shell-bin");
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&bin_dir).unwrap();
        let shell = bin_dir.join("sh.exe");
        fs::write(&shell, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), bin_dir.to_string_lossy().to_string());

        assert_eq!(find_shell(&env_vars), None);
        assert_eq!(find_user_command("sh", &env_vars), Some(shell));
        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_lookup_uses_explicit_internal_shell() {
        let shell = std::env::current_exe().unwrap();
        let mut env_vars = HashMap::new();
        env_vars.insert(
            COMPATIBLE_SHELL_PATH_ENV.to_string(),
            shell.to_string_lossy().to_string(),
        );
        env_vars.insert("PATH".to_string(), String::new());

        assert_eq!(find_shell(&env_vars), Some(shell));
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_works_with_mixed_case_path() {
        // On Windows, std::env::vars() returns PATH as "Path" (capital P).
        // find_user_command reads env_vars.get("PATH") (all caps), so we should
        // fail to find the command when only "Path" is set. This test documents
        // the upstream behavior and motivates the init.rs fix that mirrors
        // the value into the all-caps key.
        let target_dir = std::env::temp_dir().join("rubash-mixed-case-path");
        let _ = fs::remove_dir_all(&target_dir);
        fs::create_dir_all(&target_dir).unwrap();
        let marker = target_dir.join("cmd.exe");
        fs::write(&marker, "").unwrap();

        // This lookup attempts to find "cmd" using only the all-caps PATH key,
        // which is what Executor::new() will hold after the init.rs fix runs.
        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), target_dir.to_string_lossy().to_string());
        assert_eq!(
            find_user_command("cmd", &env_vars).map(|p| p.to_string_lossy().to_string()),
            Some(marker.to_string_lossy().to_string()),
        );

        let _ = fs::remove_dir_all(&target_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_cache_invalidates_on_path_change() {
        // The resolution cache keys results on a fingerprint of the env vars
        // that affect lookup. Assigning PATH must reset it so a command that
        // missed under the old PATH resolves under the new one.
        let bin_dir = std::env::temp_dir().join("rubash-lookup-cache-path-change");
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&bin_dir).unwrap();
        let marker = bin_dir.join("newtool.exe");
        fs::write(&marker, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), String::new());
        assert_eq!(find_user_command("newtool", &env_vars), None);

        env_vars.insert("PATH".to_string(), bin_dir.to_string_lossy().to_string());
        assert_eq!(find_user_command("newtool", &env_vars), Some(marker));

        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_fails_when_only_path_lower_is_set() {
        // Direct counter-test for the casing bug: setting the OS-side
        // "Path" (capital P) without the all-caps "PATH" key causes
        // find_user_command to miss the command. Bug surfaces in shells
        // embedding rubash on Windows until Executor::new() normalizes.
        let target_dir = std::env::temp_dir().join("rubash-only-path-lower");
        let _ = fs::remove_dir_all(&target_dir);
        std::fs::create_dir_all(&target_dir).unwrap();
        let marker = target_dir.join("cmd.exe");
        std::fs::write(&marker, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("Path".to_string(), target_dir.to_string_lossy().to_string());

        // find_user_command has no normalization itself; the init.rs workaround
        // upstream performs the casing mirror. Without that workaround, this
        // lookup returns None.
        assert_eq!(
            find_user_command("cmd", &env_vars),
            None,
            "find_user_command should not see the lowercase Path key until init.rs normalizes"
        );

        let _ = std::fs::remove_dir_all(&target_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_splits_bash_style_prefix_before_native_path() {
        let empty_dir = std::env::temp_dir().join("rubash-bash-style-path-prefix-empty");
        let bin_dir = std::env::temp_dir().join("rubash-bash-style-path-prefix-bin");
        let _ = fs::remove_dir_all(&empty_dir);
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&empty_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        let marker = bin_dir.join("clear.exe");
        fs::write(&marker, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "PATH".to_string(),
            format!(
                "{}:{};C:/definitely/missing",
                windows_shell_path(&empty_dir),
                windows_shell_path(&bin_dir)
            ),
        );

        assert_eq!(find_user_command("clear", &env_vars), Some(marker));

        let _ = fs::remove_dir_all(empty_dir);
        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_maps_logical_usr_bin_from_shell_root() {
        let root = std::env::temp_dir().join("rubash-logical-root-absolute-command");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("usr").join("bin")).unwrap();
        let command = root.join("usr").join("bin").join("tool.exe");
        fs::write(&command, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "RUBASH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );

        assert_eq!(find_user_command("/usr/bin/tool", &env_vars), Some(command));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_bin_commands_fall_back_to_path_lookup() {
        // `/bin/X` and `/usr/bin/X` name the system tool namespace, which
        // has no Windows location without a shell root; resolve the
        // basename through PATH so suites spawn real subprocesses instead
        // of hitting "command not found".
        let dir = std::env::temp_dir().join("rubash-bin-path-fallback");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sh = dir.join("sh.exe");
        let cat = dir.join("cat.exe");
        fs::write(&sh, "").unwrap();
        fs::write(&cat, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), dir.to_string_lossy().to_string());

        assert_eq!(find_user_command("/bin/sh", &env_vars), Some(sh.clone()));
        assert_eq!(find_user_command("/usr/bin/sh", &env_vars), Some(sh));
        assert_eq!(find_user_command("/bin/cat", &env_vars), Some(cat.clone()));
        assert_eq!(find_user_command("/usr/bin/cat", &env_vars), Some(cat));
        // Basenames absent from PATH keep the 127 result; non-/bin
        // absolute paths and nested paths never fall back.
        assert_eq!(find_user_command("/bin/nonexistent-tool", &env_vars), None);
        assert_eq!(find_user_command("/sbin/cat", &env_vars), None);
        assert_eq!(find_user_command("/bin/sub/cat", &env_vars), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_neutral_shell_root_env_drives_root_resolution() {
        // The neutral __RUBASH_SHELL_ROOT name must be sufficient on its own;
        // WINUXSH_ROOT/RUBASH_ROOT are compatibility fallbacks only.
        let root = std::env::temp_dir().join("rubash-neutral-shell-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("etc")).unwrap();
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "__RUBASH_SHELL_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );
        assert_eq!(
            shell_path_to_windows("/etc/config", &env_vars),
            root.join("etc").join("config")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_root_maps_root_and_clamps_parent_components() {
        let root = std::env::temp_dir().join("rubash-logical-root-paths");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "RUBASH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );
        env_vars.insert("TMPDIR".to_string(), r"C:\Windows\Temp".to_string());

        assert_eq!(shell_path_to_windows("/", &env_vars), root);
        assert_eq!(
            shell_path_to_windows("/bin/../etc/config", &env_vars),
            root.join("etc").join("config")
        );
        // /tmp resolves through TMPDIR even when a shell root is configured
        // (unixwin/niubash#94), never below a possibly read-only root.
        assert_eq!(
            shell_path_to_windows("/tmp/cache", &env_vars),
            PathBuf::from(r"C:\Windows\Temp").join("cache")
        );
        assert_eq!(
            shell_path_to_windows("/c/Users/example", &env_vars),
            PathBuf::from(r"C:\Users\example")
        );
        assert_eq!(
            shell_path_to_windows("C:/Users/example", &env_vars),
            PathBuf::from(r"C:/Users/example")
        );
        assert_eq!(
            shell_path_to_windows(
                "Z:/nope/D:/repo/rubash/target/bashdb-probe-target.sh",
                &env_vars
            ),
            PathBuf::from(r"D:/repo/rubash/target/bashdb-probe-target.sh")
        );
        assert_eq!(
            shell_path_to_windows("/dev/null", &env_vars),
            PathBuf::from("NUL")
        );
        assert_eq!(
            shell_path_to_windows("/dev/fd/1", &env_vars),
            PathBuf::from(r"\\.\WINUXSH_UNSUPPORTED_DEVICE")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_tmp_prefers_tmpdir_even_with_shell_root() {
        // unixwin/niubash#94: a shell root under a non-user-writable install
        // dir must not leave /tmp read-only; TMPDIR wins over <root>/tmp.
        let root = std::env::temp_dir().join("rubash-tmp-shell-root");
        let tmp = std::env::temp_dir().join("rubash-tmpdir-base");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );
        env_vars.insert("TMPDIR".to_string(), tmp.to_string_lossy().to_string());

        assert_eq!(shell_path_to_windows("/tmp", &env_vars), tmp);
        assert_eq!(
            shell_path_to_windows("/tmp/cache", &env_vars),
            tmp.join("cache")
        );

        // A TMPDIR that spells the virtual /tmp itself must not recurse;
        // it falls back to the real process temp dir.
        env_vars.insert("TMPDIR".to_string(), "/tmp".to_string());
        assert_eq!(
            shell_path_to_windows("/tmp/x", &env_vars),
            std::env::temp_dir().join("x")
        );
        env_vars.insert("TMPDIR".to_string(), "/tmp/".to_string());
        assert_eq!(
            shell_path_to_windows("/tmp/y", &env_vars),
            std::env::temp_dir().join("y")
        );

        // With no TMPDIR at all /tmp lands in the process temp dir too.
        env_vars.remove("TMPDIR");
        assert_eq!(
            shell_path_to_windows("/tmp", &env_vars),
            std::env::temp_dir()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_var_tmp_stays_in_user_temp_even_with_shell_root() {
        // unixwin/niubash#94: /var/tmp follows /tmp into the per-user temp
        // namespace so a read-only install root cannot break it either.
        let root = std::env::temp_dir().join("rubash-var-tmp-shell-root");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );

        let var_tmp = windows_var_tmp_dir().unwrap();
        assert_eq!(shell_path_to_windows("/var/tmp", &env_vars), var_tmp);
        assert_eq!(
            shell_path_to_windows("/var/tmp/f", &env_vars),
            var_tmp.join("f")
        );
        // Other /var children still live below the shell root.
        assert_eq!(
            shell_path_to_windows("/var/log/x", &env_vars),
            root.join("var").join("log").join("x")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_home_path_maps_to_real_user_profiles_parent() {
        let real_home = std::env::temp_dir()
            .join("rubash-real-home-paths")
            .join("alice");
        let mut env_vars = HashMap::new();
        env_vars.insert("HOME".to_string(), real_home.to_string_lossy().to_string());

        assert_eq!(
            shell_path_to_windows("/home", &env_vars),
            real_home.parent().unwrap()
        );
        assert_eq!(
            shell_path_to_windows("/home/alice/docs", &env_vars),
            real_home.join("docs")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_home_path_uses_real_profile_before_shell_root() {
        let shell_root = std::env::temp_dir().join("rubash-home-shell-root");
        let user_profile = std::env::temp_dir()
            .join("rubash-real-userprofile")
            .join("bob");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            shell_root.to_string_lossy().to_string(),
        );
        env_vars.insert("HOME".to_string(), "/home/bob".to_string());
        env_vars.insert(
            "USERPROFILE".to_string(),
            user_profile.to_string_lossy().to_string(),
        );

        assert_eq!(
            shell_path_to_windows("/home", &env_vars),
            user_profile.parent().unwrap()
        );
        assert_eq!(
            shell_path_to_windows("/home/bob/project", &env_vars),
            user_profile.join("project")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_root_selects_logical_standard_path() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        assert_eq!(
            standard_path(&env_vars),
            "/usr/local/bin:/usr/bin:/bin".to_string()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_dispatcher_requires_explicit_session_path() {
        let dir = std::env::temp_dir().join("rubash-explicit-winuxcmd-dispatcher");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dispatcher = dir.join("winuxcmd.exe");
        fs::write(&dispatcher, b"").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), dir.to_string_lossy().to_string());
        assert_eq!(find_winuxcmd_dispatcher(&env_vars), None);

        env_vars.insert(
            "WINUXCMD_PATH".to_string(),
            dispatcher.to_string_lossy().to_string(),
        );
        assert_eq!(find_winuxcmd_dispatcher(&env_vars), Some(dispatcher));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_real_installation_tree_resolves_commands_without_provider_overlay() {
        let root = std::env::temp_dir().join("rubash-real-winuxcmd-tree");
        let _ = fs::remove_dir_all(&root);
        let shell_root = root.join("winuxcmd");
        fs::create_dir_all(shell_root.join("bin")).unwrap();
        fs::create_dir_all(shell_root.join("usr").join("bin")).unwrap();

        let dispatcher = shell_root.join("usr").join("bin").join("winuxcmd.exe");
        let bin_command = shell_root.join("bin").join("ls.exe");
        let usr_command = shell_root.join("usr").join("bin").join("awk.exe");
        fs::write(&dispatcher, b"").unwrap();
        fs::write(&bin_command, b"").unwrap();
        fs::write(&usr_command, b"").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            shell_root.to_string_lossy().to_string(),
        );
        env_vars.insert(
            "WINUXCMD_PATH".to_string(),
            dispatcher.to_string_lossy().to_string(),
        );
        env_vars.insert(
            "WINUXCMD_HOME".to_string(),
            shell_root.to_string_lossy().to_string(),
        );

        assert_eq!(find_user_command("/usr/bin/ls", &env_vars), None);
        assert_eq!(
            find_user_command("/bin/ls", &env_vars),
            Some(bin_command.clone())
        );
        assert_eq!(
            shell_path_to_windows_for_lookup("/usr/bin/awk", &env_vars),
            usr_command
        );
        assert_eq!(
            find_user_command("/usr/bin/awk", &env_vars),
            Some(shell_root.join("usr").join("bin").join("awk.exe"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_real_bin_directory_view_excludes_wpm_state() {
        let root = std::env::temp_dir().join("rubash-real-bin-directory-view");
        let _ = fs::remove_dir_all(&root);
        let shell_root = root.join("winuxcmd");
        fs::create_dir_all(shell_root.join("usr").join("bin")).unwrap();
        fs::create_dir_all(shell_root.join(".wpm").join("cache")).unwrap();
        fs::write(shell_root.join("usr").join("bin").join("local.exe"), b"").unwrap();
        fs::write(shell_root.join("usr").join("bin").join("ls.exe"), b"").unwrap();
        fs::write(shell_root.join("usr").join("bin").join("wpm.exe"), b"").unwrap();
        fs::write(shell_root.join(".wpm").join("cache").join("jq.exe"), b"").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            shell_root.to_string_lossy().to_string(),
        );
        env_vars.insert(
            "WINUXCMD_HOME".to_string(),
            shell_root.to_string_lossy().to_string(),
        );

        let entries = shell_directory_entries("/usr/bin", &env_vars).unwrap();
        let names = entries
            .into_iter()
            .map(|entry| entry.name)
            .collect::<HashSet<_>>();
        assert!(names.contains("local.exe"));
        assert!(names.contains("ls.exe"));
        assert!(names.contains("wpm.exe"));
        assert!(!names.contains(".wpm"));
        assert!(!names.contains("jq.exe"));

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_real_path_process_entries_use_only_backing_directory() {
        let root = std::env::temp_dir().join("rubash-real-path-process-entries");
        let _ = fs::remove_dir_all(&root);
        let shell_root = root.join("winuxcmd");
        fs::create_dir_all(shell_root.join("usr").join("bin")).unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            shell_root.to_string_lossy().to_string(),
        );
        let entries = shell_path_process_entries("/usr/bin", &env_vars);
        assert_eq!(entries, vec![shell_root.join("usr").join("bin")]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn winuxcmd_installation_root_is_derived_from_canonical_and_legacy_paths() {
        let root = std::env::temp_dir().join("rubash-winuxcmd-installation-root");
        assert_eq!(
            winuxcmd_installation_root_from_path(&root.join("usr/bin/winuxcmd.exe")),
            root
        );
        assert_eq!(
            winuxcmd_installation_root_from_path(&root.join("usr/bin")),
            root
        );
        assert_eq!(
            winuxcmd_installation_root_from_path(&root.join("winuxcmd.exe")),
            root
        );
    }

    #[test]
    fn dispatcher_command_name_strips_logical_bin_prefixes() {
        assert_eq!(dispatcher_command_name("head"), "head");
        assert_eq!(dispatcher_command_name("/bin/cat"), "cat");
        assert_eq!(dispatcher_command_name("/usr/bin/cat"), "cat");
        assert_eq!(dispatcher_command_name("/usr/local/bin/cat"), "cat");
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_switches_are_not_mapped_into_shell_root() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );
        let program = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        let args = vec!["/C".to_string(), "echo child".to_string()];
        let (command, _) =
            external_command_for_named_program(&program, Some("cmd.exe"), &args, &env_vars);
        let actual = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(actual, args);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_prefers_native_wrapper_before_extensionless_script() {
        let bin_dir = std::env::temp_dir().join("rubash-native-wrapper-before-script");
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&bin_dir).unwrap();
        let script = bin_dir.join("code");
        let wrapper = bin_dir.join("code.cmd");
        fs::write(&script, "#!/usr/bin/env sh\n").unwrap();
        fs::write(&wrapper, "@echo off\r\n").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), bin_dir.to_string_lossy().to_string());
        env_vars.insert("PATHEXT".to_string(), ".EXE;.PS1".to_string());

        assert_eq!(find_user_command("code", &env_vars), Some(wrapper));

        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_find_user_command_prefers_ps1_before_extensionless_script() {
        let bin_dir = std::env::temp_dir().join("rubash-ps1-before-script");
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&bin_dir).unwrap();
        let script = bin_dir.join("tool");
        let wrapper = bin_dir.join("tool.ps1");
        fs::write(&script, "#!/usr/bin/env sh\n").unwrap();
        fs::write(&wrapper, "Write-Output tool\r\n").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), bin_dir.to_string_lossy().to_string());
        env_vars.insert("PATHEXT".to_string(), ".EXE".to_string());

        assert_eq!(find_user_command("tool", &env_vars), Some(wrapper));

        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_extensionless_script_without_sh_falls_back_to_current_shell() {
        let bin_dir = std::env::temp_dir().join("rubash-extensionless-self-shell");
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&bin_dir).unwrap();
        let script = bin_dir.join("code");
        fs::write(&script, "#!/usr/bin/env sh\n").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), String::new());

        let (command, used_shell) = external_command_for_program(&script, &[".".into()], &env_vars);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(used_shell);
        assert_eq!(
            PathBuf::from(command.get_program()),
            std::env::current_exe().unwrap()
        );
        assert_eq!(
            args,
            vec![script.to_string_lossy().to_string(), ".".to_string()]
        );

        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_ps1_commands_run_through_powershell_file() {
        let bin_dir = std::env::temp_dir().join("rubash-powershell-bin");
        let _ = fs::remove_dir_all(&bin_dir);
        fs::create_dir_all(&bin_dir).unwrap();
        let pwsh = bin_dir.join("pwsh");
        fs::write(&pwsh, "").unwrap();
        let script = bin_dir.join("probe.ps1");
        fs::write(&script, "").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), bin_dir.to_string_lossy().to_string());

        let (command, used_shell) =
            external_command_for_program(&script, &["one".into(), "two".into()], &env_vars);
        let expected_script = cmd_compatible_windows_path(&script)
            .to_string_lossy()
            .to_string();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(PathBuf::from(command.get_program()), pwsh);
        assert_eq!(
            args,
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                expected_script,
                "one".to_string(),
                "two".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(bin_dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_arguments_translate_shell_display_paths() {
        let env_vars = HashMap::new();
        let (command, used_shell) = external_command_for_program(
            &PathBuf::from("head.exe"),
            &["/c/Users/example/file.txt".to_string()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(args, vec![r"C:\Users\example\file.txt"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_arguments_preserve_native_literals_and_options() {
        let root = std::env::temp_dir().join("rubash-native-argv-literals");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );

        let (command, used_shell) = external_command_for_program(
            &PathBuf::from("pwsh.exe"),
            &[
                "-Command".to_string(),
                r"Copy-Item full\bin\* smoke -Force".to_string(),
                "repos/nmap/nmap/contents/configure.ac?ref=v7.991".to_string(),
                "--send-only".to_string(),
                "/CN=test".to_string(),
            ],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        // push_external_args wraps wildcard-bearing argv in double quotes so
        // MSYS2/WinuxCmd-hosted children do not re-glob them (niubash#119):
        // `*` and `?` reach the child literally through CommandLineToArgvW.
        assert_eq!(
            args,
            vec![
                "-Command".to_string(),
                r#""Copy-Item full\bin\* smoke -Force""#.to_string(),
                r#""repos/nmap/nmap/contents/configure.ac?ref=v7.991""#.to_string(),
                "--send-only".to_string(),
                "/CN=test".to_string(),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_arguments_preserve_bare_drive_shaped_patterns() {
        let env_vars = HashMap::new();
        let (command, used_shell) = external_command_for_program(
            &PathBuf::from("git.exe"),
            &["/h/".to_string(), "--literal".to_string()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(args, vec!["/h/".to_string(), "--literal".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_arguments_preserve_nonexistent_drive_shaped_patterns() {
        let env_vars = HashMap::new();
        let (command, used_shell) = external_command_for_program(
            &PathBuf::from("git.exe"),
            &["/h/not-a-real-pathspec".to_string()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(args, vec!["/h/not-a-real-pathspec".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_arguments_repair_host_rewritten_drive_backslashes() {
        let env_vars = HashMap::new();
        let (command, used_shell) = external_command_for_program(
            &PathBuf::from("where.exe"),
            &["C://Windows//System32".to_string()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(args, vec!["C:\\Windows\\System32".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_arguments_preserve_leading_backslash_data() {
        let root = std::env::temp_dir().join("rubash-preserve-backslash-argument");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );

        let (command, used_shell) = external_command_for_program(
            &PathBuf::from("tr.exe"),
            &[" ".to_string(), r"\n".to_string()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(args, vec![" ".to_string(), r"\n".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_dispatcher_receives_original_command_name_before_arguments() {
        let env_vars = HashMap::new();
        let (command, used_shell) = external_command_for_named_program(
            Path::new(r"C:\tools\winuxcmd.exe"),
            Some("head"),
            &["-3000".to_string(), "input.txt".to_string()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(!used_shell);
        assert_eq!(args, vec!["head", "-3000", "input.txt"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_dispatcher_does_not_re_dispatch_itself() {
        let env_vars = HashMap::new();
        for command_name in ["winuxcmd", "winuxcmd.exe", r"C:\tools\winuxcmd.exe"] {
            let (command, used_shell) = external_command_for_named_program(
                Path::new(r"C:\tools\winuxcmd.exe"),
                Some(command_name),
                &["--version".to_string()],
                &env_vars,
            );
            let args = command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect::<Vec<_>>();

            assert!(!used_shell);
            assert_eq!(args, vec!["--version"], "command name: {command_name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_unc_path_with_single_leading_backslash_is_restored() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // After shell backslash escaping, `\\DFDB-A1` becomes `\DFDB-A1`.
        // shell_path_to_windows should restore it to `\\DFDB-A1` (UNC).
        assert_eq!(
            shell_path_to_windows(r"\DFDB-A1", &env_vars),
            PathBuf::from(r"\\DFDB-A1")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_unc_path_with_share_is_restored() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // `\DFDB-A1\share` after shell escaping should become UNC.
        assert_eq!(
            shell_path_to_windows(r"\DFDB-A1\share", &env_vars),
            PathBuf::from(r"\\DFDB-A1\share")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_root_relative_drive_path_not_treated_as_unc() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // A path like `\C:\Users` with a drive letter after the first
        // backslash should NOT be treated as UNC.
        assert_eq!(
            shell_path_to_windows(r"\C:\Users", &env_vars),
            PathBuf::from(r"C:\Users")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_bare_leading_backslash_becomes_unc_root() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // A bare single backslash should become UNC root.
        assert_eq!(shell_path_to_windows(r"\", &env_vars), PathBuf::from(r"\\"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_mnt_drive_paths_map_to_windows_drives() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // /mnt/c -> C:\
        assert_eq!(
            shell_path_to_windows("/mnt/c", &env_vars),
            PathBuf::from(r"C:\")
        );
        // /mnt/c/some/path -> C:\some\path
        assert_eq!(
            shell_path_to_windows("/mnt/c/some/path", &env_vars),
            PathBuf::from(r"C:\some\path")
        );
        // /mnt/d -> D:\
        assert_eq!(
            shell_path_to_windows("/mnt/d", &env_vars),
            PathBuf::from(r"D:\")
        );
        // /mnt/e/Users -> E:\Users
        assert_eq!(
            shell_path_to_windows("/mnt/e/Users", &env_vars),
            PathBuf::from(r"E:\Users")
        );
        // /mnt/z -> Z:\
        assert_eq!(
            shell_path_to_windows("/mnt/z", &env_vars),
            PathBuf::from(r"Z:\")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_mnt_drive_paths_are_case_insensitive() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // /mnt/C -> C:\ (uppercase drive letter)
        assert_eq!(
            shell_path_to_windows("/mnt/C", &env_vars),
            PathBuf::from(r"C:\")
        );
        // /mnt/D/path -> D:\path
        assert_eq!(
            shell_path_to_windows("/mnt/D/path", &env_vars),
            PathBuf::from(r"D:\path")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_mnt_drive_paths_need_translation() {
        let env_vars = HashMap::new();

        // All /mnt/X paths should need translation
        assert!(windows_external_absolute_argument_needs_translation(
            "/mnt/c", &env_vars
        ));
        assert!(windows_external_absolute_argument_needs_translation(
            "/mnt/d/some/path",
            &env_vars
        ));
        assert!(windows_external_absolute_argument_needs_translation(
            "/mnt/z", &env_vars
        ));

        // Invalid /mnt paths should not need translation
        assert!(!windows_external_absolute_argument_needs_translation(
            "/mnt", &env_vars
        ));
        assert!(!windows_external_absolute_argument_needs_translation(
            "/mnt/", &env_vars
        ));
        assert!(!windows_external_absolute_argument_needs_translation(
            "/mnt/123", &env_vars
        ));
        // /mnt/cfoo should NOT match (letter not followed by / or end-of-string)
        assert!(!windows_external_absolute_argument_needs_translation(
            "/mnt/cfoo",
            &env_vars
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_mnt_drive_paths_do_not_false_positive() {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );

        // /mnt/cfoo should NOT map to C:\foo — it should fall through to shell root
        let result = shell_path_to_windows("/mnt/cfoo", &env_vars);
        let root_str = std::env::temp_dir().to_string_lossy().to_string();
        assert!(result.starts_with(&root_str));
        assert!(result.ends_with("mnt\\cfoo"));

        // /mnt/c without trailing slash maps correctly
        assert_eq!(
            shell_path_to_windows("/mnt/c", &env_vars),
            PathBuf::from(r"C:\")
        );
        // /mnt/c/ with trailing slash also maps correctly
        assert_eq!(
            shell_path_to_windows("/mnt/c/", &env_vars),
            PathBuf::from(r"C:\")
        );
    }

    fn windows_shell_path(path: &Path) -> String {
        let value = path.to_string_lossy().replace('\\', "/");
        let bytes = value.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/'
        {
            format!(
                "/{}/{}",
                (bytes[0] as char).to_ascii_lowercase(),
                &value[3..]
            )
        } else {
            value
        }
    }
}
