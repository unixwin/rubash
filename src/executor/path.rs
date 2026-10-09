//! path module.
//!
//! GNU Bash source ownership:
// - findcmd.c
// - findcmd.h

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

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
/// <name>` child process), so hits, misses, per-directory listings backing
/// a merged PATH scan, and dispatcher probe outcomes are all cached
/// (rubash#159).
struct CommandLookupCache {
    fingerprint: String,
    results: HashMap<String, Option<PathBuf>>,
    /// Lowercased file-name listing per physical PATH directory. Replaces
    /// the PATH x PATHEXT per-candidate stat grid with one read_dir per
    /// directory (rubash#159).
    listings: HashMap<PathBuf, Arc<HashSet<String>>>,
    /// `winuxcmd help NAME` dispatcher probe outcomes keyed by dispatch
    /// name (rubash#159). Each uncached probe is a child process; GNU has
    /// no analogue because findcmd.c:623 find_user_command_in_path is a
    /// pure stat walk.
    probes: HashMap<String, bool>,
    /// Per-fingerprint PATH scan scaffold (rubash#159): the split and
    /// translated PATH directories and the parsed PATHEXT order, built on
    /// the first uncached lookup after a fingerprint change. They are pure
    /// functions of the fingerprint's inputs, so recomputing them per
    /// lookup (split_shell_path + shell_path_to_windows per directory,
    /// which itself stats the pinned POSIX tools dir) is pure overhead.
    path_dirs: Option<Arc<Vec<PathBuf>>>,
    path_extensions: Option<Arc<Vec<String>>>,
}

impl CommandLookupCache {
    /// Drop everything keyed on the previous fingerprint. GNU's
    /// equivalent single flush point is phash_flush()
    /// (builtins/hash.def:150, `hash -r`); the fingerprint mismatch path
    /// mirrors GNU invalidating the hash table when PATH is assigned
    /// (findcmd.c:356-380 search_for_command consults the live $PATH each
    /// time, so a PATH change is observed immediately).
    fn reset(&mut self, fingerprint: String) {
        self.fingerprint = fingerprint;
        self.results.clear();
        self.listings.clear();
        self.probes.clear();
        self.path_dirs = None;
        self.path_extensions = None;
    }
}

static COMMAND_LOOKUP_CACHE: OnceLock<Mutex<CommandLookupCache>> = OnceLock::new();

fn command_lookup_cache() -> &'static Mutex<CommandLookupCache> {
    COMMAND_LOOKUP_CACHE.get_or_init(|| {
        Mutex::new(CommandLookupCache {
            fingerprint: String::new(),
            results: HashMap::new(),
            listings: HashMap::new(),
            probes: HashMap::new(),
            path_dirs: None,
            path_extensions: None,
        })
    })
}

/// Forget all remembered command locations (`hash -r`, tests).
pub(crate) fn clear_command_lookup_cache() {
    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.reset(String::new());
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

    // GNU findcmd.c:348-427 search_for_command(): when hashing is disabled
    // (`set +h`, findcmd.c:356 hashing_enabled) or PATH is in the command's
    // temporary environment (findcmd.c:359 temp_path), phash_search AND
    // phash_insert are both skipped and every lookup re-walks $PATH. GNU's
    // walk costs microseconds so it needs no cache; the Windows walk costs
    // milliseconds, so on these bypass paths a miss is memoized in the same
    // process-internal, fingerprint-keyed store the hashed path uses
    // (rubash#159). Only misses are memoized here: a file that appears
    // mid-session without a PATH change must stay findable, which keeps
    // GNU's re-scan semantics for positives (`hash -r` /
    // builtins/hash.def:150 phash_flush and any fingerprint change still
    // drop the memo).
    if !crate::builtins::set::shell_option_enabled(env_vars, "hashall")
        || env_vars.get("__RUBASH_TEMP_PATH").map(String::as_str) == Some("1")
    {
        let fingerprint = command_lookup_fingerprint(env_vars);
        {
            let mut cache = command_lookup_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if cache.fingerprint != fingerprint {
                cache.reset(fingerprint.clone());
            } else if cache.results.get(name) == Some(&None) {
                return None;
            }
        }
        let result = find_user_command_uncached(name, env_vars, &fingerprint);
        if result.is_none() {
            let mut cache = command_lookup_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if cache.fingerprint != fingerprint {
                cache.reset(fingerprint);
            } else {
                cache.results.insert(name.to_string(), None);
            }
        }
        return result;
    }

    let fingerprint = command_lookup_fingerprint(env_vars);
    {
        let mut cache = command_lookup_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.fingerprint != fingerprint {
            cache.reset(fingerprint.clone());
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

    let result = find_user_command_uncached(name, env_vars, &fingerprint);

    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.fingerprint != fingerprint {
        cache.reset(fingerprint);
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

/// Cached lowercased-name listing of one directory (rubash#159).
///
/// GNU bash stats each PATH-element/name candidate directly
/// (findcmd.c:623 find_user_command_in_path -> find_in_path_element ->
/// file_status, findcmd.c:113); on Linux that is a microsecond syscall, but
/// on Windows the equivalent walk is one CreateFile per candidate per
/// directory, which costs milliseconds across a realistic PATH x PATHEXT
/// grid. One read_dir per directory replaces the grid and is remembered
/// for the life of the fingerprint.
///
/// This is an in-process negative cache by design: bash has none (a miss
/// re-walks and can observe a file that just appeared), so the divergence
/// is bounded to "a file created inside an already-listed PATH directory
/// after its listing was taken is not found until the fingerprint changes
/// or `hash -r` (builtins/hash.def:150 phash_flush) clears the cache". An
/// unreadable directory (missing PATH entry, permission denial) yields an
/// empty listing -- exactly the set of names the per-candidate stat walk
/// would have found there: none. `cache` must already be verified against
/// the current fingerprint by the caller.
#[cfg(not(unix))]
fn cached_dir_listing(cache: &mut CommandLookupCache, directory: &Path) -> Arc<HashSet<String>> {
    if let Some(listing) = cache.listings.get(directory) {
        return Arc::clone(listing);
    }
    let names = fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let listing = Arc::new(names);
    if cache.listings.len() >= 512 {
        cache.listings.clear();
    }
    cache
        .listings
        .insert(directory.to_path_buf(), Arc::clone(&listing));
    listing
}

/// One probe candidate for the listing-assisted PATH walk: the lowercased
/// file name the directory listing is asked about, plus the spelled file
/// name to join onto a directory when the listing claims it. Built ONCE per
/// lookup, not per directory: GNU findcmd.c:623 find_user_command_in_path
/// passes the same NAME to find_in_path_element for every directory, and
/// the candidate set is a pure function of NAME and PATHEXT order. The
/// candidate order (bare first for extension-carrying names, extensions
/// first otherwise, bare last) and the `with_extension` spellings are
/// byte-identical to the per-directory `executable_candidate_listed` this
/// replaces — only the per-directory `dir.join(name)` /
/// `with_extension` / `to_string_lossy().to_lowercase()` churn moved out
/// of the loop (envfix3: a warm unique-name miss over a 69-entry PATH
/// spent ~310 us re-deriving the same ~8 strings 69 times).
#[cfg(not(unix))]
struct ListedCandidate {
    listing_key: String,
    file: PathBuf,
}

#[cfg(not(unix))]
fn listed_candidates(name: &str, extensions: &[String]) -> Vec<ListedCandidate> {
    let base = Path::new(name);
    let dotted = base.extension().is_some();
    let mut probes = Vec::with_capacity(extensions.len() + 1);
    let mut push = |candidate: &Path| {
        // file_name() is None for `.`/`..`-shaped candidates; the stat walk
        // could never match those against a listing either.
        if let Some(file_name) = candidate.file_name() {
            probes.push(ListedCandidate {
                listing_key: file_name.to_string_lossy().to_lowercase(),
                file: candidate.to_path_buf(),
            });
        }
    };
    if dotted {
        push(base);
    }
    for ext in extensions {
        push(&base.with_extension(ext));
    }
    if !dotted {
        push(base);
    }
    probes
}

/// Merged Windows PATH scan (rubash#159). Mirrors the walk shape of GNU
/// findcmd.c:623 find_user_command_in_path (first match wins, PATH order
/// preserved, per-directory extension order from executable_candidate),
/// but consults the per-fingerprint scan scaffold and the cached
/// directory listings under one lock, so a warm miss performs no
/// filesystem syscalls at all.
#[cfg(not(unix))]
fn find_in_path_via_listings(
    name: &str,
    env_vars: &HashMap<String, String>,
    fingerprint: &str,
) -> Option<PathBuf> {
    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.fingerprint != fingerprint {
        cache.reset(fingerprint.to_string());
    }
    if cache.path_dirs.is_none() {
        let dirs = split_shell_path(env_vars.get("PATH").map(String::as_str).unwrap_or_default())
            .into_iter()
            .map(|dir| shell_path_to_windows(&dir, env_vars))
            .collect::<Vec<_>>();
        cache.path_dirs = Some(Arc::new(dirs));
    }
    if cache.path_extensions.is_none() {
        cache.path_extensions = Some(Arc::new(executable_extensions(env_vars)));
    }
    let dirs = Arc::clone(cache.path_dirs.as_ref().unwrap());
    let extensions = Arc::clone(cache.path_extensions.as_ref().unwrap());
    // Candidate spellings are per-NAME, not per-directory (GNU passes the
    // same NAME through the whole walk); derive them once so a miss costs
    // one contains() per PATH entry per PATHEXT candidate and no
    // allocation at all.
    let candidates = listed_candidates(name, &extensions);
    for dir in dirs.iter() {
        let listing = cached_dir_listing(&mut cache, dir);
        for candidate in &candidates {
            // HashSet<String>::contains(&str): no lookup-key allocation.
            if !listing.contains(candidate.listing_key.as_str()) {
                continue;
            }
            // The listing only proves the NAME exists; confirm the stat
            // walk's exact semantics for directories and dangling symlinks.
            let path = dir.join(&candidate.file);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg_attr(unix, allow(unused_variables))]
fn find_user_command_uncached(
    name: &str,
    env_vars: &HashMap<String, String>,
    fingerprint: &str,
) -> Option<PathBuf> {
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
    {
        let mut file_to_lose_on: Option<PathBuf> = None;
        for dir in split_shell_path(env_vars.get("PATH").map(String::as_str).unwrap_or_default()) {
            let candidate = shell_path_to_windows(&dir, env_vars).join(name);
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
        if let Some(fallback) = file_to_lose_on {
            return Some(fallback);
        }
    }
    // rubash#159: the Windows branch replaces GNU's per-element
    // find_in_path_element -> file_status (findcmd.c:113) stat walk with a
    // merged per-directory listing scan: one read_dir per PATH directory,
    // cached per fingerprint, instead of one stat per PATH directory x
    // PATHEXT extension.
    #[cfg(not(unix))]
    if let Some(found) = find_in_path_via_listings(name, env_vars, fingerprint) {
        return Some(found);
    }

    // A workspace may expose WinuxCmd as one dispatcher executable instead of
    // one wrapper per command.  Ask the dispatcher whether it owns the name
    // before returning it; unknown names must retain Bash's 127 behavior.
    #[cfg(windows)]
    if let Some(dispatcher) = find_winuxcmd_dispatcher(env_vars) {
        if winuxcmd_has_command_cached(&dispatcher, name, env_vars) {
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
    // bionic (android) has no confstr(), so it skips straight to the
    // STANDARD_UTILS_PATH fallback.
    #[cfg(all(unix, not(target_os = "android")))]
    {
        if let Some(path) = confstr_cs_path() {
            return path;
        }
        "/bin:/usr/bin:/sbin:/usr/sbin".to_string()
    }

    #[cfg(all(unix, target_os = "android"))]
    {
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
// bionic (android) has no confstr(); the caller's STANDARD_UTILS_PATH
// fallback covers it.
#[cfg(all(unix, not(target_os = "android")))]
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
        let value = buf.split(|&byte| byte == 0).next().unwrap_or_default();
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
/// literally and nothing ever expands it again. Two Windows child families
/// instead re-expand *unquoted* command-line arguments at their runtime
/// boundary:
///
/// - CRT wildcard expansion (any child compiled with setargv): `ls "*.txt"`
///   reached the child as a bare `*.txt` token and was globbed a second time
///   (niubash#119, rubash-side of #83) — wildcard-bearing args are quoted for
///   every child.
/// - MSYS2/cygwin-hosted children (Git-for-Windows `usr/bin` tools et al.)
///   rebuild argv from the command line and run shell-like expansion on
///   unquoted tokens: probe 2026-10-02 against Git GNU sed 4.9 spawned
///   natively — `s/n/${S}/` reached sed as `s/n/$S/`, `s/n/{S}/` as
///   `s/n/S/`, `s/n/\n/` as `s/n/<newline>/`, `s/n/\t/` as `s/n/<tab>/`.
///   A single-quoted sed program inside `$( )` therefore lost its
///   backslashes and `${...}` braces before exec (rubash#417 — hawaii50's
///   `sed -e :a -e '$!N;s/\n/${SEP}/;ta'` joined lines with `$SEP`), so for
///   those children any argument carrying a byte of the MSYS expansion
///   grammar (`\` escape, `$`, braces, `~`, backtick, quote) is quoted too.
///
/// A double-quoted argument decodes to the same string under
/// CommandLineToArgvW, and the quoting is gated on the child actually being
/// MSYS-hosted (its program directory ships the POSIX runtime DLL), so
/// native children keep the exact unquoted command line the argv-dialect
/// contracts pin (niubash#124(b): path parameters never arrive quoted).
fn push_external_args(command: &mut Command, args: &[String]) {
    #[cfg(windows)]
    let posix_runtime_hosted = windows_program_is_posix_runtime_hosted(command);
    for arg in args {
        #[cfg(windows)]
        {
            if arg.contains(['*', '?', '[']) {
                command.raw_arg(windows_quoted_wildcard_arg(arg));
                continue;
            }
            if posix_runtime_hosted && arg.contains(['\\', '$', '{', '}', '~', '`', '\'']) {
                command.raw_arg(windows_quoted_wildcard_arg(arg));
                continue;
            }
        }
        command.arg(arg);
    }
}

/// Whether the child program is hosted by an MSYS2/cygwin POSIX runtime —
/// i.e. it will rebuild its argv from the Windows command line and re-expand
/// unquoted tokens. Detected by the runtime DLL shipping beside the program
/// (msys-2.0.dll for MSYS2/Git-for-Windows `usr/bin`, cygwin1.dll for
/// Cygwin); a failed lookup classifies the child as native, which keeps the
/// historical unquoted command line.
#[cfg(windows)]
fn windows_program_is_posix_runtime_hosted(command: &Command) -> bool {
    let program = command.get_program();
    let Some(directory) = std::path::Path::new(program).parent() else {
        return false;
    };
    ["msys-2.0.dll", "cygwin1.dll"].iter().any(|dll| {
        directory
            .join(dll)
            .metadata()
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    })
}

/// Quote one argument for a Windows command line so the child's argv sees
/// the text literally (no CRT wildcard expansion). Follows the
/// CommandLineToArgvW rules: wrap in double quotes, emit `2n + 1` backslashes
/// before an embedded `"`, and double trailing backslashes before the closing
/// quote. Backslash runs not followed by a quote stay literal.
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
        let backslashes_to_emit = if ch == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        for _ in 0..backslashes_to_emit {
            quoted.push('\\');
        }
        backslashes = 0;
        quoted.push(ch);
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
    // Child-argv byte contract (rubash#141): GNU hands execve the raw word
    // bytes (execute_cmd.c:6139 shell_execve); Windows argv is UTF-16, so an
    // invalid-UTF-8 byte travels as its own code point (WTF-8 style, byte
    // value == code point) instead of the internal PUA marker pair, which
    // ANSI children re-encoded as GBK mojibake.
    let decode_bytes = |arg: String| {
        crate::executor::substitution_metadata::decode_raw_byte_markers_to_byte_chars(&arg)
    };
    // A shell-wrapped child (a `#!/...` script run through another bash)
    // lives in the SAME shell path domain as the caller: GNU's ENOEXEC
    // re-entry (execute_cmd.c:6252) hands the child shell the argv words
    // verbatim, and the child understands `/d/...` shell paths natively.
    // Converting slash-paths to `D:\...` there mixes domains and breaks
    // path arithmetic in the child (`${BATS_TEST_FILENAME##*/}` keeps the
    // whole backslash path, rubash#259 bats-gather-tests). Native .exe
    // children keep the existing Windows-form conversion — they need real
    // Windows operands (sed.exe et al).
    let shell_wrapped = cfg!(windows)
        && !is_windows_powershell_script(program)
        && !is_windows_batch_file(program)
        && should_run_with_shell(program);
    // Option B (niubash#124(b)): a POSIX-aware child (WinuxCmd dispatcher
    // route or a program under a WinuxCmd installation root) resolves the
    // shell's POSIX namespace itself, so it joins the verbatim-argv set:
    // GNU hands execve the raw word bytes (execute_cmd.c:6119-6127
    // shell_execve) and never rewrites arguments, and the child's own
    // POSIX layer owns `/d/...` resolution — exactly the MSYS model, where
    // the runtime inside the child converts paths at its Win32 boundary.
    // Translating for such a child both loses dialect purity and splits
    // one argv into mixed forms when some operands exist and others do not
    // (the pre-Option-B existence-gated half-translation). The
    // `__RUBASH_ARGV_DIALECT=legacy` escape hatch restores the old
    // behavior for field rollback.
    #[cfg(windows)]
    let posix_aware = !argv_dialect_legacy(env_vars) && posix_aware_child(program, env_vars);
    #[cfg(not(windows))]
    let posix_aware = false;
    let native_args = args
        .iter()
        .map(|arg| {
            // unixwin/niubash#164: a virtual-system-root operand (/usr/bin,
            // /etc/..., /tmp, /home/...) exists only in the shell's root
            // map and NO non-shell child resolves it, so it translates
            // through the same map for every child class BEFORE the
            // per-child dialect branches below. Real drive forms (/d/x,
            // /mnt/d/x, /cygdrive/d/x) keep the Option B dialect: verbatim
            // for POSIX-aware children (their own native layer converts
            // those), converted for natives.
            #[cfg(windows)]
            if let Some(translated) =
                translated_virtual_system_root_argument(arg, shell_wrapped, env_vars)
            {
                return translated;
            }
            if preserve_native_args || shell_wrapped || posix_aware {
                arg.clone()
            } else {
                external_argument_path(arg, env_vars)
            }
        })
        .map(decode_bytes)
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
        // GNU findcmd.c:395-397 (search_for_command) + execute_cmd.c:
        // 5927-5931 (execute_disk_command): the word handed to
        // shell_execve keeps "the same format that the user used to type
        // it in" when it carries a slash, and a PATH-resolved word is the
        // joined path entry. execute_cmd.c:6252 then passes that word to
        // the ENOEXEC re-entry as the new script's name, so it becomes
        // the child shell's $0 (shell.c:1613 dollar_vars[0]). Recover it
        // from the typed word; the resolved Windows PathBuf would leak a
        // backslash path as $0 instead.
        let zero_word = script_zero_word(program, command_name);
        if let Some((command, used_shell)) =
            shell_wrapped_command(program, &zero_word, &native_args, env_vars)
        {
            return (command, used_shell);
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

/// The script argument a shell-wrapped child reports as `$0`.
///
/// GNU search_for_command (findcmd.c:395-397) returns a slash-bearing word
/// verbatim (`savestring (pathname)`), and execute_disk_command
/// (execute_cmd.c:5927-5931) leaves args[0] "in the same format that the
/// user used to type it in"; a PATH-resolved word is the joined path entry
/// (findcmd.c find_user_file_in_path). The ENOEXEC re-entry passes that
/// word as the new script name (execute_cmd.c:6252 `args[1] = command`).
pub fn script_zero_word(program: &Path, command_name: Option<&str>) -> String {
    match command_name {
        Some(word) if word.contains('/') || word.contains('\\') => word.to_string(),
        // PATH join: GNU concatenates path entry + "/" + word; the resolved
        // PathBuf carries the same join in Windows form, so display it with
        // forward slashes to keep the $0 shape GNU-identical.
        _ => program.to_string_lossy().replace('\\', "/"),
    }
}

/// Build the shell-wrapper invocation for a file the OS cannot exec
/// natively (`should_run_with_shell`). ZERO_WORD is the argument the child
/// shell takes as its script name ($0). Returns `None` when no shell
/// processor is available and the caller must fall through to a direct
/// spawn.
pub fn shell_wrapped_command(
    program: &Path,
    zero_word: &str,
    args: &[String],
    env_vars: &HashMap<String, String>,
) -> Option<(Command, bool)> {
    if !should_run_with_shell(program) {
        return None;
    }
    if let Some(shell) = find_shell(env_vars) {
        let mut command = Command::new(shell);
        command.arg(zero_word);
        push_external_args(&mut command, args);
        return Some((command, true));
    }
    if let Some(shell) = current_shell_processor() {
        let mut command = Command::new(shell);
        command.arg(zero_word);
        push_external_args(&mut command, args);
        return Some((command, true));
    }
    None
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

/// Option B argv-dialect escape hatch (niubash#124(b), MSYS per-child
/// dialect model). Unset or any value other than `legacy` runs the Option B
/// contract; `legacy` restores the pre-Option-B behavior (POSIX-aware
/// children translated like natives + existence-gated half-translation) so
/// a field regression can be reverted from the environment without a
/// rebuild. Read from the shell env so `export` toggles it mid-session.
fn argv_dialect_legacy(env_vars: &HashMap<String, String>) -> bool {
    env_vars
        .get("__RUBASH_ARGV_DIALECT")
        .is_some_and(|value| value.eq_ignore_ascii_case("legacy"))
}

/// Case-insensitive component prefix test for Windows paths. `starts_with`
/// on PathBuf is byte-sensitive, and WINUXCMD_HOME / dispatcher roots are
/// host-provided strings whose case need not match the resolved program
/// path (e.g. `c:\tools\...` vs `C:\Tools\...`).
#[cfg(windows)]
fn path_starts_with_ignore_case(path: &Path, prefix: &Path) -> bool {
    let path_parts: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let prefix_parts: Vec<String> = prefix
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    if prefix_parts.is_empty() {
        return false;
    }
    path_parts.len() >= prefix_parts.len() && path_parts[..prefix_parts.len()] == prefix_parts[..]
}

/// Memoized WinuxCmd-tree marker probe. For an already-resolved program
/// path, derive the candidate installation root
/// (`winuxcmd_installation_root_from_path`: `usr/bin` and flat layouts) and
/// confirm it with the `winuxcmd.exe` marker. This classifies a program's
/// dialect from its OWN tree, so a WinuxCmd installation reached through
/// PATH without env configuration (multi-install hosts) is still detected.
/// It does not select a dispatcher — that stays env-configured only (see
/// `find_winuxcmd_dispatcher`).
#[cfg(windows)]
fn under_winuxcmd_tree(program: &Path) -> bool {
    use std::sync::Mutex;
    static CACHE: std::sync::OnceLock<Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    // Normalize separators and case into a flat cache key; the probe result
    // depends only on the path (the marker files are installation layout,
    // not configuration).
    let key = program.to_string_lossy().replace('/', "\\").to_lowercase();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(&key) {
            return *cached;
        }
    }
    let root = winuxcmd_installation_root_from_path(program);
    let hit = root.join("usr").join("bin").join("winuxcmd.exe").is_file()
        || root.join("winuxcmd.exe").is_file();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, hit);
    }
    hit
}

/// Option B (niubash#124(b), the MSYS per-child dialect model): whether a
/// resolved program is POSIX-aware — its own runtime resolves verbatim
/// POSIX argv (`/d/...`, `/tmp/...`, `/dev/...`). GNU never rewrites argv
/// (execute_cmd.c:6119-6127 shell_execve: `execve (command, args, env)`),
/// so such children must receive the shell word bytes verbatim and own
/// path resolution themselves. Determinable by the engine in three ways:
/// the winuxcmd dispatcher route, the env-configured WinuxCmd
/// installation root (WINUXCMD_HOME / the dispatcher's tree), or the
/// per-program winuxcmd.exe marker probe.
#[cfg(windows)]
fn posix_aware_child(program: &Path, env_vars: &HashMap<String, String>) -> bool {
    if is_winuxcmd_dispatcher(program) {
        return true;
    }
    if let Some(home) = env_vars
        .get("WINUXCMD_HOME")
        .filter(|value| !value.is_empty())
    {
        if path_starts_with_ignore_case(program, Path::new(home)) {
            return true;
        }
    }
    if let Some(dispatcher) = find_winuxcmd_dispatcher(env_vars) {
        let root = winuxcmd_installation_root_from_path(&dispatcher);
        if path_starts_with_ignore_case(program, &root) {
            return true;
        }
    }
    under_winuxcmd_tree(program)
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

/// rubash#159: `winuxcmd help NAME` probe memo. Each uncached probe is a
/// child process (tens of milliseconds) and every PATH miss reaches it
/// once per unique name. GNU has no analogue (findcmd.c:623
/// find_user_command_in_path is a pure stat walk), so this is purely a
/// Windows dispatcher concern: outcomes are remembered per dispatch name
/// for the life of the env fingerprint, which covers the WINUXCMD*/
/// COREUTILS_PATH variables that select the dispatcher itself.
#[cfg(windows)]
fn winuxcmd_has_command_cached(
    dispatcher: &Path,
    name: &str,
    env_vars: &HashMap<String, String>,
) -> bool {
    let fingerprint = command_lookup_fingerprint(env_vars);
    {
        let mut cache = command_lookup_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.fingerprint != fingerprint {
            cache.reset(fingerprint.clone());
        } else if let Some(cached) = cache.probes.get(name) {
            return *cached;
        }
    }
    let probed = winuxcmd_has_command(dispatcher, name, env_vars);
    let mut cache = command_lookup_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.fingerprint != fingerprint {
        cache.reset(fingerprint);
    }
    if cache.probes.len() >= 4096 {
        cache.probes.clear();
    }
    cache.probes.insert(name.to_string(), probed);
    probed
}

#[cfg(windows)]
fn find_winuxcmd_absolute_command(
    name: &str,
    env_vars: &HashMap<String, String>,
) -> Option<PathBuf> {
    let command_name = logical_bin_command_name(name)?;
    let dispatcher = find_winuxcmd_dispatcher(env_vars)?;
    winuxcmd_has_command_cached(&dispatcher, &command_name, env_vars).then_some(dispatcher)
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

/// Normalize an inherited process PATH into the shell's `:`-separated
/// semantic form (rubash#175).
///
/// GNU shell.c/variables.c have no Windows concept: every environ entry is
/// imported verbatim, so a script's `$PATH` is always one `:`-separated
/// string and `${PATH%%:*}`, `PATH=/usr/bin:$PATH`, and ltmain's
/// func_path_progs slicing all work. Rubash's Windows process environment
/// instead carries a `;`-separated drive-letter PATH; importing that
/// verbatim breaks every POSIX colon operation on the first `C:` colon.
/// This is the import-boundary normalization only: entries are split with
/// the same dual semantics the PATH lookup uses (`split_shell_path`, which
/// accepts both `;`-separated drive entries and `:`-separated shell
/// entries), each drive entry is rewritten to its `/c/...` shell spelling
/// (`C:\x` and `C:/x` both become `/c/x`), and the list is joined with
/// `:`. The reverse direction for native children is the pre-existing
/// `shell_path_to_process` below, which turns `/c/x` entries back into
/// `C:\x` drive directories. The function is idempotent: a PATH already in
/// shell form splits and rejoins unchanged, so the in-process
/// `${THIS_SH} ./x.sub` child path that re-imports a normalized PATH keeps
/// it stable.
pub(crate) fn process_path_to_shell(path: &str) -> String {
    if !cfg!(windows) {
        return path.to_string();
    }
    split_shell_path(path)
        .into_iter()
        .map(|entry| windows_drive_entry_to_shell(&entry))
        .collect::<Vec<_>>()
        .join(":")
}

/// Rewrite one PATH entry from Windows drive spelling to shell spelling:
/// `C:\Windows` / `C:/Windows` -> `/c/Windows`. Non-drive entries (shell
/// paths, UNC paths, relative entries) pass through unchanged.
fn windows_drive_entry_to_shell(entry: &str) -> String {
    let bytes = entry.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        // entry[2..] is "/..." or "\...": normalize separators and keep the
        // lowercase-drive /c/ spelling that shell_path_to_windows maps back
        // to `C:\` (path.rs drive branch).
        let rest = entry[2..].replace('\\', "/");
        return format!("/{}{}", (bytes[0] as char).to_ascii_lowercase(), rest);
    }
    entry.to_string()
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
            // must be translated even before the target is created.
            // Option B (niubash#124(b)): other `/X/...` operands translate
            // UNCONDITIONALLY for native children — existence-gating here
            // was the source of one argv arriving as `D:\a` + `/d/b`
            // mixed dialect (target exists -> translated, target missing
            // -> verbatim). GNU hands argv verbatim to execve
            // (execute_cmd.c:6119-6127 shell_execve); the only reason a
            // native child gets a translated operand at all is that it
            // has no POSIX layer of its own, and such a child needs the
            // Windows spelling for MISSING targets too (an output file
            // being created). The POSIX-aware children that made the
            // exists() gate look safe (regexes, git pathspecs, sed/awk
            // fragments) now receive verbatim argv (posix_aware_child in
            // external_command_for_named_program), so the ambiguity class
            // no longer flows through here. The legacy escape hatch
            // (__RUBASH_ARGV_DIALECT=legacy) restores the gated behavior.
            if drive != b'c' && normalized.len() <= 3 {
                return arg.to_string();
            }
            if drive != b'c' && argv_dialect_legacy(env_vars) && !translated.exists() {
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
    // A bare "/" is an operand character under POSIX, not a path only the
    // shell can decide: `expr 10 / 3` uses it as the division operator and
    // `tr / X` as a SET1 member (unixwin/niubash#153). GNU hands argv
    // verbatim to execve (execute_cmd.c:6126 shell_execve) and never
    // rewrites arguments, so the Windows adaptation may only translate
    // unambiguously path-shaped operands. Path-consuming children (`ls /`)
    // resolve "/" through their own POSIX layer, which lands on their
    // installation root — the same tree the shell-root translation targeted.
    if normalized == "/" {
        return false;
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

    // Virtual system roots (unixwin/niubash#164) resolve through the root
    // map for native children exactly as they already did before the
    // extraction; see windows_virtual_system_root_argument for the class.
    if windows_virtual_system_root_argument(normalized, env_vars) {
        return true;
    }

    // /mnt/X drive paths need translation for all drive letters.
    // Must be exactly /mnt/X or /mnt/X/... to avoid false matches like /mnt/cfoo.
    if windows_mnt_drive_argument(normalized) {
        return true;
    }

    false
}

/// unixwin/niubash#164: whether a POSIX-shaped argument names a VIRTUAL
/// SYSTEM ROOT — /usr, /etc, /tmp, /home, /bin, ... — a location that
/// exists only in the shell's root map (`shell_path_to_windows`, the same
/// funnel the `cd` builtin resolves through: `cd /usr/bin` reaches the
/// install tree, `pwd` echoes /usr/bin), not on the host filesystem. This
/// is a third POSIX argument shape beside the single-character `/` operand
/// (niubash#153) and the `/X/` drive forms (niubash#62): a drive form
/// names a real host location the child's own POSIX layer resolves
/// (WinuxCmd native_path.cppm normalize_api_operand converts /d/repo/file
/// in-child; MSYS2 runtimes do the same), but no non-shell child maps
/// virtual roots — WinuxCmd falls back to the CURRENT DRIVE root (probed
/// 2026-10-02 against build-dev WinuxCmd 1.0.3: `winuxcmd ls /usr/bin` ->
/// "cannot access '/usr/bin'" while `winuxcmd ls /c/Windows` succeeds),
/// and native exes / cmd.exe have no POSIX layer at all. MSYS confirms
/// the split at its own Win32 boundary: for native children the runtime
/// converts /usr/bin -> <install>/usr/bin and /tmp -> the user temp dir
/// (Git Bash 2026-10-02 probe: `cmd //c echo /usr/bin` prints
/// D:/Git/usr/bin, `/tmp` prints the %TEMP% spelling).
///
/// The targets are exactly the existing root map's entries — this
/// function invents nothing: /tmp and /var/tmp resolve to the per-user
/// temp base (niubash#94), /home to the real home's parent
/// (HOME/USERPROFILE), and the install-tree components {bin,etc,lib,
/// lib64,opt,sbin,usr,var} join below the configured shell root
/// (WINUXSH_ROOT et al.) with the same Option B gating native children
/// already use (niubash#124(b)). `normalized` must already be
/// backslash-folded; only `/`-prefixed spellings name the namespace.
fn windows_virtual_system_root_argument(
    normalized: &str,
    env_vars: &HashMap<String, String>,
) -> bool {
    if !normalized.starts_with('/') {
        return false;
    }

    // /tmp is a per-user temporary namespace, not part of the install
    // tree (niubash#94): it maps through the temp base even when no shell
    // root is configured, so it is root-map-shaped for every child class.
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

    if configured_shell_root(env_vars).is_some()
        && matches!(
            normalized.split('/').nth(1),
            Some("bin" | "etc" | "lib" | "lib64" | "opt" | "sbin" | "usr" | "var")
        )
    {
        // Option B (niubash#124(b)): translate shell-root-prefixed operands
        // unconditionally — the previous exists() gate split one argv into
        // mixed dialects (an existing /etc/config became root\etc\config
        // while a missing sibling stayed /etc/...). Only the legacy escape
        // hatch keeps the existence probe.
        return !argv_dialect_legacy(env_vars)
            || shell_path_to_windows(normalized, env_vars).exists();
    }

    false
}

/// WSL drive form `/mnt/X` (and `/mnt/X/...`), for the argv funnel.
///
/// This is the shape `shell_path_to_windows` already maps to `X:\` (see its
/// `/mnt/` branch), so the engine-side translation exists and is tested —
/// what was missing was routing the *external argument* path through it.
/// WinuxCmd cannot resolve the form itself: `normalize_api_operand_w` folds
/// `/cygdrive/d/...` into `/d/...` and then maps a bare `/X/...` drive
/// letter, but its drive-letter test requires the letter to be followed by
/// a separator or end-of-string, which `/mnt/c` (letter `m` followed by
/// `n`) never satisfies. Keeping the operand verbatim for POSIX-aware
/// children therefore handed every applet an unresolvable path
/// (unixwin/WinuxCmd#1145, the `/d/...` sibling).
///
/// Must be exactly `/mnt/X` or `/mnt/X/...`: `/mnt/cfoo` and `/mnt/123` are
/// ordinary POSIX-shaped words, not drive forms.
fn windows_mnt_drive_argument(normalized: &str) -> bool {
    if !normalized.starts_with("/mnt/") || normalized.len() < 6 {
        return false;
    }
    let bytes = normalized.as_bytes();
    bytes[5].is_ascii_alphabetic() && (normalized.len() == 6 || bytes[6] == b'/')
}

/// unixwin/niubash#164: the root-map translation itself for one argv word,
/// applied at the single argv funnel (external_command_for_named_program)
/// so that EVERY non-shell child class receives the same resolved form:
/// the winuxcmd dispatcher and applets under the tree (posix_aware), the
/// cmd.exe command processor (whose slash switches must stay verbatim —
/// only the multi-component root names translate), and plain native exes.
/// GNU hands argv verbatim to execve (execute_cmd.c:6119-6127
/// shell_execve); the only Windows-side rewrites are the ones the child
/// cannot perform itself, and resolving the shell's own virtual namespace
/// is one of them (the applet's native layer covers drive forms and
/// /dev/*, not the roots — see windows_virtual_system_root_argument).
///
/// Returns None when the word keeps its per-child dialect: MSYS/Cygwin
/// drive forms the child's own native layer resolves (`/d/x`,
/// `/cygdrive/d/x` — WinuxCmd `native_path.cppm normalize_api_operand_w`
/// owns both), the bare `/` operand (niubash#153), /dev/* (rubash#120),
/// switches and data words, a leading-backslash spelling (escape data,
/// external_argument_path's rule), a legacy-dialect session
/// (__RUBASH_ARGV_DIALECT=legacy reverts to the pre-fix behavior), or a
/// shell-wrapped child — a child SHELL (ENOEXEC re-entry model,
/// execute_cmd.c:6252) resolves /usr/... through its own identical root
/// map and must see the words verbatim.
///
/// `/mnt/X` WSL drive forms do NOT belong to that verbatim set.
///
/// Why the verbatim set exists at all (niubash#62, the design that produced
/// it): `ls /d/` and `grep -F "/h/"` are textually identical, so the shell
/// cannot separate "drive path" from "pattern" by shape alone. niubash#164
/// resolved that by making the split PER-CHILD rather than per-name-list —
/// one argv arrives in exactly one form, and the child that owns a POSIX
/// layer decodes it. Two constraints bound this fix:
///
/// - niubash#62: "WinuxCmd (coreutils) must NOT change — the rule stays in
///   the shell layer." So this is fixed here, not by teaching WinuxCmd
///   `/mnt`.
/// - niubash#164: "要么都翻、要么都不翻，不应按命令名单切分" — the same path
///   must reach every child class in the same form.
///
/// `/mnt/X` satisfies neither half of the verbatim premise. WinuxCmd's
/// `native_path.cppm normalize_api_operand_w` folds `/cygdrive/d/...` into
/// `/d/...` and then maps a bare `/X/...` drive letter, but its letter test
/// requires a separator or end-of-string after the letter — `mnt` does not
/// satisfy it, and a repo-wide code search for `mnt` under unixwin/WinuxCmd
/// returns only docs and scripts, no parsing arm. No native exe has a POSIX
/// layer at all. So unlike `/d/x` and `/cygdrive/d/x`, `/mnt/X` is resolved
/// by NOBODY on the child side and must be translated engine-side for every
/// child class, exactly like the virtual system roots.
///
/// `cd` never exposed the gap: the builtin resolves through
/// `shell_path_to_windows`, which does own a `/mnt` branch, so the split
/// only surfaces once the same path is handed to an external child
/// (`cd /mnt/c` worked while `ls /mnt/c` and every other applet failed).
/// unixwin/WinuxCmd#1145 is the `/d/...` sibling of this gap.
#[cfg(windows)]
fn translated_virtual_system_root_argument(
    arg: &str,
    shell_wrapped: bool,
    env_vars: &HashMap<String, String>,
) -> Option<String> {
    if shell_wrapped || argv_dialect_legacy(env_vars) || !arg.starts_with('/') {
        return None;
    }
    let normalized = arg.replace('\\', "/");
    (windows_virtual_system_root_argument(&normalized, env_vars)
        || windows_mnt_drive_argument(&normalized))
    .then(|| {
        // niubash#177: the translated spelling goes through the operand
        // resolution so a WinuxCmd-tree file operand (`/usr/bin/seq`, only
        // `seq.exe` on disk) reaches the child in the spelling it can
        // open. Existence-checked: existing files, directories and
        // neither-spelling misses keep the plain translated form.
        windows_operand_file_path(shell_path_to_windows(arg, env_vars), env_vars)
            .to_string_lossy()
            .into_owned()
    })
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
        // GNU variables.c make_env_array_from_var_list copies the value cell
        // byte-for-byte: an explicitly exported HOME (any form) wins over
        // any derived default. Fill-if-missing mirrors the env-builtin
        // path's materialize_required_windows_env or_insert semantics — the
        // previous unconditional clobber replaced a user's POSIX-form HOME
        // with the USERPROFILE-derived native form on every spawn (the
        // rubash#329 source-(a) family, niubash#149), and the drive colon
        // then broke child scripts splicing $HOME into s:...: patterns.
        if !env_vars.contains_key("USERPROFILE") {
            process.env("USERPROFILE", &native_home);
        }
        if !env_vars.contains_key("HOME") {
            process.env("HOME", &native_home);
        }
        if let Some((drive, path)) = windows_drive_and_home_path(&native_home) {
            if !env_vars.contains_key("HOMEDRIVE") {
                process.env("HOMEDRIVE", drive);
            }
            if !env_vars.contains_key("HOMEPATH") {
                process.env("HOMEPATH", path);
            }
        }
        let base = native_home.trim_end_matches('\\');
        if !env_vars.contains_key("APPDATA") {
            process.env("APPDATA", format!("{base}\\AppData\\Roaming"));
        }
        if !env_vars.contains_key("LOCALAPPDATA") {
            process.env("LOCALAPPDATA", format!("{base}\\AppData\\Local"));
        }
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

/// unixwin/niubash#177: the DATA-OPERAND form of the Win32 executable
/// extension resolution — the read-side counterpart of
/// `executable_candidate` above, which already covers command NAMES (PATH
/// lookup, `test -x`) and `shell_path_to_windows_for_lookup`.
///
/// GNU opens what it is handed verbatim: execve gets the word bytes
/// (execute_cmd.c:6128 `execve (command, args, env)` inside shell_execve)
/// and redir_open opens the expanded filename without rewriting
/// (redir.c:675, redir.c:702). On POSIX a `/usr/bin/seq` operand IS the
/// file; on Windows the Win32/NTFS namespace stores it as `seq.exe`, and
/// the POSIX layer must bridge that at its Win32 boundary — which is what
/// MSYS does: its runtime resolves an absent as-spelled name to the
/// existing `<name>.exe` (live probe on this machine, 2026-10-02: Git
/// Bash's MSYS head.exe, handed `D:\...\usr\bin\seq` with only
/// `seq.exe` on disk, opens it and reads the exe bytes — the fallback is
/// in the MSYS runtime, not in coreutils). WinuxCmd applets have no such
/// runtime layer, so the translation layer must hand them the resolved
/// spelling itself: `head -1 /usr/bin/seq` under the root map used to
/// open `...\usr\bin\seq` and fail with "cannot open" (niubash#177) while
/// `test -f /usr/bin/seq` answered YES through the lookup form above —
/// one namespace, two contradictory views.
///
/// Order is the MSYS open/exec order, existence-checked both ways:
/// 1. as-spelled exists (file, directory, symlink): hand exactly that
///    spelling. A directory `seq` is the operand — it is never swapped
///    for a coincidental `seq.exe` (directory operands `ls /usr/bin`,
///    `cp x /etc` keep working with NO suffix appended);
/// 2. else the shell's own candidate set (`executable_candidate` — the
///    same resolution `test`/hash/PATH lookup apply, so children see the
///    namespace the shell's internal lookups see) picks the first
///    existing PATHEXT spelling;
/// 3. else the AS-SPELLED form stands — never a blind append: a name
///    missing under both spellings errors in the child exactly as it did
///    before this resolution existed.
pub(crate) fn windows_operand_file_path(
    path: PathBuf,
    env_vars: &HashMap<String, String>,
) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    if path.exists() {
        return path;
    }
    executable_candidate(&path, env_vars).unwrap_or(path)
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
    let mapped = shell_path_to_windows_inner(path, env_vars);
    if cfg!(windows) {
        map_ntfs_colon_final_component(mapped)
    } else {
        mapped
    }
}

/// rubash#359: on Windows the Win32/NTFS layer reads `:` in the LAST path
/// component as the `name:stream` alternate-data-stream separator. A legal
/// POSIX filename like `2026-09-30 14:48:26 UTC.log` therefore fails open()
/// with EINVAL ("Invalid argument") — a single-colon name "succeeds" but
/// silently targets an ADS on a different host file. GNU bash has no such
/// rule (redir.c:706 redir_open passes the expanded filename to open(2)
/// verbatim; `:` is an ordinary filename byte), and MSYS2/cygwin — the
/// reference Windows POSIX layer — stores such names by encoding `:` as
/// U+F03A from the Unicode private-use area (verified on this volume:
/// Git Bash's `a:b.log` occupies the on-disk name `61 F03A 62 2E 6C 6F 67`).
/// Do the same in this, the single shell-name -> NT-name funnel, so every
/// consumer (redirect open, test/cd/stat, glob directory walks) agrees on
/// one backing file. `shell_path_display_from_windows` reverses the
/// encoding for shell-visible names. Only the final component is mapped:
/// drive (`D:`) and server (`\\srv`) colons live in earlier components.
fn map_ntfs_colon_final_component(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if !text.contains(':') {
        return path;
    }
    let tail_start = text
        .rfind(['\\', '/'])
        .map(|separator| separator + 1)
        .unwrap_or(0);
    let (head, tail) = text.split_at(tail_start);
    if !tail.contains(':') {
        return path;
    }
    PathBuf::from(format!("{head}{}", tail.replace(':', "\u{F03A}")))
}

fn shell_path_to_windows_inner(path: &str, env_vars: &HashMap<String, String>) -> PathBuf {
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
    if cfg!(windows) && (normalized == "/var/tmp" || normalized.starts_with("/var/tmp/")) {
        if let Some(var_tmp) = windows_var_tmp_dir() {
            // Lazy materialization (rubash#328): the backing directory is
            // created only when a /var/tmp path is actually resolved — GNU
            // creates nothing at startup and an eager create_dir_all below
            // every TMPDIR (including a caller-owned cwd) leaked a visible
            // `var' entry into `ls'/glob output of untouched directories.
            // create_dir_all on the existing directory is a no-op, so
            // repeated /var/tmp resolution costs one metadata check.
            let _ = std::fs::create_dir_all(&var_tmp);
            return if normalized == "/var/tmp" {
                var_tmp
            } else {
                var_tmp.join(normalized.strip_prefix("/var/tmp/").unwrap_or_default())
            };
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
            // rubash#359: reverse map_ntfs_colon_final_component's U+F03A
            // encoding so shell-visible names (glob results, directory
            // listings) show the POSIX filename byte.
            .replace('\u{F03A}', ":")
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
                // A drive designator is only valid as a path PREFIX: `X:/…`
                // or `/X:/…` (optional leading slashes). An `X:/` pattern
                // that follows real path components — bashdb's `_Dbg_dir`
                // join produces `D:/repo/wt/D:/repo/wt/target/x.sh` — is an
                // invalid mid-path component, not a drive: Win32 only
                // accepts a colon at position 1. Mapping the tail as a
                // drive silently resolves the joined text to a REAL file,
                // so `[[ -f <cdir>/<absolute-drive-path> ]]` returned true
                // where GNU's stat() fails on the doubled path.
                if bytes[..index].iter().all(|byte| *byte == b'/') {
                    return Some(index);
                }
                continue;
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
///
/// startup21 lazy probe: the filesystem probe (up to 3 is_file stats per
/// PATH entry) cost ~0.28ms of every spawn although most shells never run
/// `command -p` or touch `/bin`. Executor::new now records the startup
/// PATH (pin_startup_tools_dir_source) and inserts the pin variable with
/// that PATH string as its value instead of eagerly probing; the first
/// consumer resolves it here, memoized per PATH string. The probe stays a
/// pure function of the startup PATH string, so the result is byte-equal
/// to the eager version's.
#[cfg(windows)]
static STARTUP_TOOLS_DIR_SOURCE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

#[cfg(windows)]
static TOOLS_DIR_PROBE_CACHE: std::sync::OnceLock<
    std::sync::Mutex<HashMap<String, Option<PathBuf>>>,
> = std::sync::OnceLock::new();

#[cfg(windows)]
fn tools_dir_cache() -> &'static std::sync::Mutex<HashMap<String, Option<PathBuf>>> {
    TOOLS_DIR_PROBE_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// startup21: record this process's startup shell-form PATH as the source
/// for the lazy POSIX-tools-dir probe. Called once from Executor::new
/// (the in-process child executors constructed later probe their own env
/// PATH exactly as they did before, because their env_vars map does not
/// carry the pin variable).
#[cfg(windows)]
pub(crate) fn pin_startup_tools_dir_source(startup_path: String) {
    let _ = STARTUP_TOOLS_DIR_SOURCE.set(Some(startup_path));
}

#[cfg(windows)]
fn memoized_tools_dir(path_value: &str) -> Option<PathBuf> {
    let mut cache = tools_dir_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(hit) = cache.get(path_value) {
        return hit.clone();
    }
    // The startup PATH arrives in Windows `;` form while an assigned PATH
    // (and the imported shell-form PATH, rubash#175) is `:`-separated
    // `/c/...` entries. Split with the dual-semantics splitter and map each
    // entry to its Windows directory so both spellings find the toolset.
    let found = split_shell_path(path_value)
        .into_iter()
        .map(|entry| shell_drive_entry_to_windows(&entry))
        .find(|dir| {
            ["sh.exe", "cat.exe", "rm.exe"]
                .iter()
                .all(|name| dir.join(name).is_file())
        });
    cache.insert(path_value.to_string(), found.clone());
    found
}

#[cfg(windows)]
pub(crate) fn windows_posix_tools_dir(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    // The toolset location is a host property: a script that overwrites
    // PATH (`PATH=/bin:/usr/bin`, invocation.tests) must not lose it, so
    // Executor::new pins the startup PATH into __RUBASH_POSIX_TOOLS_DIR
    // (startup21: as the lazy source string; the value equals the recorded
    // startup PATH, which no real directory value can collide with — and
    // the equality check below runs before the is_dir fallback anyway).
    // Probing the live process PATH is useless — env_var writes sync into
    // it before the lookup runs.
    if let Some(pinned) = env_vars.get("__RUBASH_POSIX_TOOLS_DIR") {
        if let Some(Some(startup_path)) = STARTUP_TOOLS_DIR_SOURCE.get() {
            if pinned == startup_path {
                return memoized_tools_dir(startup_path);
            }
        }
        let candidate = PathBuf::from(pinned);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    env_vars
        .get("PATH")
        .and_then(|path| memoized_tools_dir(path))
}

/// `/c/x` -> `C:\x` for one PATH entry; every other spelling (native
/// `C:\x`, `C:/x`, relative) passes through, since PathBuf accepts those
/// natively. Deliberately NOT routed through `shell_path_to_windows`,
/// whose logical `/bin` mapping consults `windows_posix_tools_dir` and
/// would recurse into this probe.
fn shell_drive_entry_to_windows(entry: &str) -> PathBuf {
    let bytes = entry.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        let rest = entry[3..].replace('/', "\\");
        return PathBuf::from(format!(
            "{}:\\{}",
            (bytes[1] as char).to_ascii_uppercase(),
            rest
        ));
    }
    PathBuf::from(entry)
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

    #[cfg(windows)]
    #[test]
    fn windows_native_argv_probe() {
        if std::env::var_os("RUBASH_WINDOWS_ARGV_PROBE").is_none() {
            return;
        }
        println!();
        for arg in std::env::args().skip(1) {
            let hex: String = arg.bytes().map(|byte| format!("{byte:02x}")).collect();
            println!("ARGV_HEX:{hex}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_forced_quoted_args_round_trip_through_native_child() {
        // Run this test executable as a native argv observer. Libtest accepts
        // arbitrary nonempty --skip values, so no external tools or fixtures
        // are needed. The child's actual argv parser is the round-trip oracle.
        let mut args = vec![
            "--exact".to_string(),
            "executor::path::tests::windows_native_argv_probe".to_string(),
            "--nocapture".to_string(),
        ];
        let mut cases = vec![
            r#"console.log(["a b", "", "c\"d", "e\\f", "---"])"#.to_string(),
            r#"plain*"#.to_string(),
            "trailing?\\".to_string(),
            "trailing[\\\\".to_string(),
        ];
        for wildcard in ['*', '?', '['] {
            for count in 0..=4 {
                cases.push(format!(
                    "before{wildcard}{}\"after with spaces",
                    "\\".repeat(count)
                ));
            }
        }
        cases.push("Unicode[中文]\\\" with spaces\\".to_string());
        for case in cases {
            args.push("--skip".to_string());
            args.push(case);
        }
        // A final argument detects a broken closing quote spilling into argv.
        args.extend(["--skip".to_string(), "__argv_tail__".to_string()]);

        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command.env("RUBASH_WINDOWS_ARGV_PROBE", "1");
        push_external_args(&mut command, &args);
        let output = command.output().expect("run native argv observer");
        assert!(output.status.success(), "{:?}", output);
        let stdout = String::from_utf8(output.stdout).expect("UTF-8 observer output");
        let observed: Vec<_> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("ARGV_HEX:"))
            .collect();
        let expected: Vec<String> = args
            .iter()
            .map(|arg| arg.bytes().map(|byte| format!("{byte:02x}")).collect())
            .collect();
        assert_eq!(observed, expected);
        assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    }

    #[cfg(windows)]
    #[test]
    fn process_path_to_shell_rewrites_windows_process_form() {
        assert_eq!(
            process_path_to_shell("C:\\Windows;C:\\WINDOWS\\System32;D:\\Git\\bin"),
            "/c/Windows:/c/WINDOWS/System32:/d/Git/bin"
        );
        // Forward-slash drive entries normalize the same way.
        assert_eq!(
            process_path_to_shell("C:/Program Files/Git/usr/bin"),
            "/c/Program Files/Git/usr/bin"
        );
    }

    #[cfg(windows)]
    #[test]
    fn process_path_to_shell_is_idempotent_on_shell_form() {
        let shell_form = "/c/Windows:/d/Git/bin:/usr/bin";
        assert_eq!(process_path_to_shell(shell_form), shell_form);
        // Hybrid lists keep both spellings working.
        assert_eq!(
            process_path_to_shell("/c/tools/cloc:/c/tools/winuxcmd;C:/Windows"),
            "/c/tools/cloc:/c/tools/winuxcmd:/c/Windows"
        );
    }

    #[cfg(windows)]
    #[test]
    fn process_path_to_shell_round_trips_through_shell_path_to_process() {
        let env_vars = HashMap::new();
        let windows_form = "C:\\Windows;C:\\WINDOWS\\System32;D:\\Git\\bin";
        let shell_form = process_path_to_shell(windows_form);
        assert_eq!(shell_path_to_process(&shell_form, &env_vars), windows_form);
    }

    #[cfg(not(windows))]
    #[test]
    fn process_path_to_shell_passes_unix_form_through() {
        assert_eq!(process_path_to_shell("/usr/bin:/bin"), "/usr/bin:/bin");
    }

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
            find_user_command_uncached("rubash-execwalk", &env, ""),
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
            find_user_command_uncached("rubash-execwalk", &env, ""),
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
        // A drive pattern after real path components is an invalid mid-path
        // component (Win32: a colon is only valid at position 1; GNU stat
        // fails on the analogous doubled /mnt path), not a drive designator
        // — the joined text stays relative to the Z: root so `[[ -f ]]`
        // fails like bashdb's `_Dbg_dir` join does on GNU.
        assert_eq!(
            shell_path_to_windows(
                "Z:/nope/D:/repo/rubash/target/bashdb-probe-target.sh",
                &env_vars
            ),
            PathBuf::from(r"Z:\nope\D:\repo\rubash\target\bashdb-probe-target.sh")
        );
        // A drive pattern directly after the leading slash IS a prefix
        // drive designator (the `/D:/…` spelling).
        assert_eq!(
            shell_path_to_windows("/D:/repo/rubash/target/bashdb-probe-target.sh", &env_vars),
            PathBuf::from(r"D:\repo\rubash\target\bashdb-probe-target.sh")
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
        // The wrapper's script argument is the child shell's $0
        // (findcmd.c:395 + execute_cmd.c:6252), carried in forward-slash
        // form — not the resolved Windows backslash path.
        assert_eq!(
            args,
            vec![script.to_string_lossy().replace('\\', "/"), ".".into()]
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
    fn windows_external_arguments_translate_drive_shaped_uniformly() {
        // Option B (niubash#124(b)): native children translate drive-shaped
        // `/X/...` operands UNCONDITIONALLY — the pre-Option-B existence
        // gate turned one argv into `D:\a` + `/d/b` mixed dialect and left
        // a native child unable to address a not-yet-existing target (an
        // output file being created). Ambiguous operands (git pathspecs,
        // regexes) belong to POSIX-aware children, which now receive
        // verbatim argv (posix_aware_child); a native git.exe under the
        // MSYS model gets the converted spelling, exactly as Git Bash
        // hands native children converted paths (MSYS_NO_PATHCONV is the
        // documented user-side opt-out there; __RUBASH_ARGV_DIALECT=legacy
        // is ours).
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
        assert_eq!(args, vec!["H:\\not-a-real-pathspec".to_string()]);

        // The legacy escape hatch restores the existence-gated behavior
        // (nonexistent drive-shaped operand stays verbatim).
        let mut legacy_env = HashMap::new();
        legacy_env.insert("__RUBASH_ARGV_DIALECT".to_string(), "legacy".to_string());
        let (command, _) = external_command_for_program(
            &PathBuf::from("git.exe"),
            &["/h/not-a-real-pathspec".to_string()],
            &legacy_env,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(args, vec!["/h/not-a-real-pathspec".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_argv_never_mixed_dialect() {
        // Option B class invariant (niubash#124(b)): one invocation, one
        // dialect. A native child receiving a drive-shaped POSIX operand
        // pair (one existing, one missing) must see BOTH in Windows form —
        // never `D:\existing` + `/d/missing` mixed. This is the exact
        // reproducer class from the reopened report (mktemp -p / mv / cp
        // operands), pinned at the funnel boundary.
        let env_vars = HashMap::new();
        let base = std::env::temp_dir().join("rubash-optb-mixed");
        std::fs::create_dir_all(base.join("m")).unwrap();
        let drive = base.to_string_lossy().to_string();
        let drive = drive.trim_end_matches('\\').to_string();
        let drive_letter = drive.chars().next().unwrap().to_ascii_lowercase();
        let posix_existing = format!("/{}{}/m", drive_letter, &drive[2..]).replace('\\', "/");
        let posix_missing =
            format!("/{}{}/missing/x", drive_letter, &drive[2..]).replace('\\', "/");

        let (command, _) = external_command_for_program(
            &PathBuf::from("git.exe"),
            &[posix_existing.clone(), posix_missing.clone()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(args.len(), 2, "both operands reach the child: {args:?}");
        for arg in &args {
            let bytes = arg.as_bytes();
            assert!(
                bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && (bytes[2] == b'\\' || bytes[2] == b'/'),
                "native child argv must be uniform Windows form, got {arg:?}"
            );
        }

        // POSIX-aware children keep the same pair verbatim (stage 1): the
        // winuxcmd dispatcher route.
        let (command, _) = external_command_for_program(
            &PathBuf::from(r"C:\t\winuxcmd\usr\bin\winuxcmd.exe"),
            &[posix_existing.clone(), posix_missing.clone()],
            &env_vars,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(
            args.contains(&posix_existing) && args.contains(&posix_missing),
            "POSIX-aware child must receive verbatim POSIX argv, got {args:?}"
        );
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
    fn windows_bare_root_slash_argument_never_translates() {
        // unixwin/niubash#153: a single "/" is an operand character (expr's
        // division operator, tr's SET1), not a path operand. GNU passes
        // argv verbatim (execute_cmd.c:6126 shell_execve); translating "/"
        // to the configured shell root corrupted `expr 10 / 3` (rc=2 with
        // the root path as the "unexpected argument") and `tr / X` (SET1
        // silently replaced by the root path string). Must hold both with
        // and without a configured root.
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            std::env::temp_dir().to_string_lossy().to_string(),
        );
        assert!(!windows_external_absolute_argument_needs_translation(
            "/", &env_vars
        ));
        let empty = HashMap::new();
        assert!(!windows_external_absolute_argument_needs_translation(
            "/", &empty
        ));
        // Root-mapped logical dirs keep their (existence-checked)
        // translation so the gnu-compat fixture root still works.
        let root = std::env::temp_dir().join("rubash-logical-root-paths");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let mut rooted = HashMap::new();
        rooted.insert(
            "WINUXSH_ROOT".to_string(),
            root.to_string_lossy().into_owned(),
        );
        assert!(windows_external_absolute_argument_needs_translation(
            "/bin", &rooted
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_parameter_arguments_never_empty_or_quoted() {
        // unixwin/niubash#124 (reopened): the reported symptom was `mktemp -p
        // "$d"` failing with template `""/x.XXXXXX`, which is exactly what a
        // child sees when the -p value arrives as two literal `"` characters
        // (byte-verified against GNU coreutils mktemp 8.32: `mktemp -p '""'
        // x.XXXXXX` prints that very message). GNU hands argv verbatim to
        // execve (execute_cmd.c:6126 shell_execve); the Windows adaptation
        // may translate a path-shaped operand to drive form, but it must
        // never empty it or let quoting leak into the value. Pin the whole
        // path-parameter family (mktemp -p/--tmpdir=, tar -C/-o, cp/mv
        // operands) across existing/missing targets, Windows-form input,
        // relative operands, and comsub-captured round-trips: every output
        // keeps the full value with no `"` byte.
        let env = {
            let mut env_vars = HashMap::new();
            env_vars.insert(
                "WINUXSH_ROOT".to_string(),
                std::env::temp_dir().to_string_lossy().to_string(),
            );
            env_vars
        };
        let base = std::env::temp_dir().join("rubash-p124-path-params");
        std::fs::create_dir_all(base.join("m")).unwrap();
        let drive = base.to_string_lossy().to_string();
        let drive = drive.trim_end_matches('\\').to_string();
        let drive_letter = drive.chars().next().unwrap().to_ascii_lowercase();
        let posix = format!("/{}{}", drive_letter, &drive[2..]).replace('\\', "/");
        let posix_existing = format!("{}/m", posix);
        let posix_missing = format!("{}/missing", posix);
        let win_existing = format!("{}/m", drive);
        let win_missing = format!("{}/missing", drive);

        let attached_tmpdir = format!("--tmpdir={posix_existing}");
        let attached_p = format!("-p{posix_existing}");
        let shapes = [
            // (label, argument)
            ("-p existing POSIX", posix_existing.as_str()),
            ("-p existing Windows", win_existing.as_str()),
            ("-p missing POSIX", posix_missing.as_str()),
            ("-p missing Windows", win_missing.as_str()),
            ("--tmpdir= attached", attached_tmpdir.as_str()),
            ("-p attached", attached_p.as_str()),
            ("relative operand", "m"),
            ("comsub-captured Windows form", win_existing.as_str()),
        ];
        for (label, arg) in shapes {
            let translated = external_argument_path(arg, &env);
            assert!(!translated.is_empty(), "{label}: emptied {arg:?}");
            assert!(
                !translated.contains('"'),
                "{label}: literal quote leaked into {translated:?}"
            );
            // The value must survive whole: it either stays verbatim or maps
            // onto the same drive with the same tail components.
            if translated != arg {
                let normalized = translated.replace('\\', "/");
                assert!(
                    normalized.starts_with(&win_existing.replace('\\', "/"))
                        || normalized.starts_with(&win_missing.replace('\\', "/")),
                    "{label}: value changed meaning: {arg:?} -> {translated:?}"
                );
            }
        }
        // Empty-string operand (host-boundary quote collapse can reduce
        // `d="..."` to empty): stays empty, never gains `""` payload bytes.
        assert_eq!(external_argument_path("", &env), "");
    }

    // ---- Option B (niubash#124(b)): per-child argv dialect -------------------

    #[test]
    fn argv_dialect_legacy_escape_hatch() {
        let mut env = HashMap::new();
        assert!(!argv_dialect_legacy(&env), "unset must mean Option B");
        env.insert("__RUBASH_ARGV_DIALECT".to_string(), "legacy".to_string());
        assert!(argv_dialect_legacy(&env));
        env.insert("__RUBASH_ARGV_DIALECT".to_string(), "LEGACY".to_string());
        assert!(argv_dialect_legacy(&env), "match is case-insensitive");
        env.insert("__RUBASH_ARGV_DIALECT".to_string(), "optionb".to_string());
        assert!(!argv_dialect_legacy(&env));
    }

    #[cfg(windows)]
    #[test]
    fn posix_aware_child_detection_routes() {
        // Route 1: the winuxcmd dispatcher itself.
        assert!(posix_aware_child(
            Path::new(r"C:\tools\winuxcmd\usr\bin\winuxcmd.exe"),
            &HashMap::new()
        ));
        assert!(posix_aware_child(
            Path::new(r"C:\t\WINUXCMD.exe"),
            &HashMap::new()
        ));

        // Route 2: under the env-configured WinuxCmd root (WINUXCMD_HOME is
        // set by Executor::set_winuxcmd_path); case and separators must not
        // matter (host-provided strings).
        let mut env = HashMap::new();
        env.insert(
            "WINUXCMD_HOME".to_string(),
            r"c:\Tools\WinuxCmd".to_string(),
        );
        assert!(posix_aware_child(
            Path::new(r"C:\tools\winuxcmd\usr\bin\ls.exe"),
            &env
        ));
        assert!(
            !posix_aware_child(Path::new(r"C:\tools\winuxcmd-other\x.exe"), &env),
            "component-boundary: a longer sibling directory must not match"
        );
        assert!(!posix_aware_child(
            Path::new(r"D:\elsewhere\node.exe"),
            &env
        ));

        // Route 3: the per-tree winuxcmd.exe marker probe — a WinuxCmd
        // installation reached through PATH without any env configuration.
        // The probe derives the root from the resolved program's own path
        // (winuxcmd_installation_root_from_path), so the program file must
        // exist exactly as a resolved spawn target would.
        let root = std::env::temp_dir().join("rubash-optb-wcmd-tree");
        let usr_bin = root.join("usr").join("bin");
        std::fs::create_dir_all(&usr_bin).unwrap();
        std::fs::write(usr_bin.join("winuxcmd.exe"), b"MZ").unwrap();
        std::fs::write(usr_bin.join("printf.exe"), b"MZ").unwrap();
        assert!(posix_aware_child(
            &usr_bin.join("printf.exe"),
            &HashMap::new()
        ));
        // Flat (legacy) installation layout: marker directly in the root.
        let flat = std::env::temp_dir().join("rubash-optb-wcmd-flat");
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::write(flat.join("winuxcmd.exe"), b"MZ").unwrap();
        std::fs::write(flat.join("printf.exe"), b"MZ").unwrap();
        assert!(posix_aware_child(&flat.join("printf.exe"), &HashMap::new()));
        // A plain directory without the marker stays native.
        let plain = std::env::temp_dir().join("rubash-optb-plain-tree");
        let plain_bin = plain.join("usr").join("bin");
        std::fs::create_dir_all(&plain_bin).unwrap();
        std::fs::write(plain_bin.join("node.exe"), b"MZ").unwrap();
        assert!(!posix_aware_child(
            &plain_bin.join("node.exe"),
            &HashMap::new()
        ));
    }

    #[cfg(windows)]
    #[test]
    fn path_prefix_match_ignores_case_and_components() {
        assert!(path_starts_with_ignore_case(
            Path::new(r"C:\Tools\WinuxCmd\usr\bin\ls.exe"),
            Path::new(r"c:\tools\winuxcmd")
        ));
        assert!(path_starts_with_ignore_case(
            Path::new(r"C:/Tools/WinuxCmd/ls.exe"),
            Path::new(r"c:\tools\winuxcmd")
        ));
        assert!(!path_starts_with_ignore_case(
            Path::new(r"C:\Tools\WinuxCmd2\ls.exe"),
            Path::new(r"c:\tools\winuxcmd")
        ));
        assert!(!path_starts_with_ignore_case(
            Path::new(r"C:\tools"),
            Path::new(r"c:\tools\winuxcmd")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn under_winuxcmd_tree_marker_probe() {
        // NOTE: the probe is memoized per process — use distinct trees so
        // this test cannot read another test's cache entry (and vice
        // versa). The program file must exist: the root derivation treats
        // an existing file's parent as the bin directory.
        let root = std::env::temp_dir().join("rubash-optb-marker-probe");
        let usr_bin = root.join("usr").join("bin");
        std::fs::create_dir_all(&usr_bin).unwrap();
        std::fs::write(usr_bin.join("cat.exe"), b"MZ").unwrap();
        // No marker yet: native.
        assert!(!under_winuxcmd_tree(&usr_bin.join("cat.exe")));
        let marked = std::env::temp_dir().join("rubash-optb-marker-probe-marked");
        let marked_bin = marked.join("usr").join("bin");
        std::fs::create_dir_all(&marked_bin).unwrap();
        std::fs::write(marked_bin.join("winuxcmd.exe"), b"MZ").unwrap();
        std::fs::write(marked_bin.join("cat.exe"), b"MZ").unwrap();
        assert!(under_winuxcmd_tree(&marked_bin.join("cat.exe")));
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

    // ---- unixwin/niubash#164: virtual system roots for every child class ----

    /// Fixture install tree: <root>/usr/bin/{winuxcmd.exe marker,ls.exe},
    /// <root>/etc/i164.conf. WINUXSH_ROOT points at <root> so the root map
    /// (/usr -> <root>\usr etc.) is active, mirroring the niu host.
    #[cfg(windows)]
    fn i164_fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join("rubash-i164-root");
        let _ = fs::remove_dir_all(&root);
        let usr_bin = root.join("usr").join("bin");
        fs::create_dir_all(&usr_bin).unwrap();
        fs::create_dir_all(root.join("etc")).unwrap();
        fs::write(usr_bin.join("winuxcmd.exe"), b"MZ").unwrap();
        fs::write(usr_bin.join("ls.exe"), b"MZ").unwrap();
        fs::write(root.join("etc").join("i164.conf"), b"i164\n").unwrap();
        root
    }

    #[cfg(windows)]
    fn i164_rooted_env(root: &Path) -> HashMap<String, String> {
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "WINUXSH_ROOT".to_string(),
            root.to_string_lossy().into_owned(),
        );
        env_vars
    }

    #[cfg(windows)]
    fn child_argv(
        program: &Path,
        args: &[&str],
        env_vars: &HashMap<String, String>,
    ) -> Vec<String> {
        let (command, _) = external_command_for_program(
            program,
            &args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(),
            env_vars,
        );
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    }

    #[cfg(windows)]
    #[test]
    fn windows_virtual_system_root_arguments_translate_for_every_child_class() {
        // niubash#164 matrix {/usr, /usr/bin, /etc/..., /tmp, /home} x
        // {dispatcher applet, under-tree applet exe, native exe, cmd.exe}:
        // every NON-SHELL child class resolves virtual roots through the
        // same root map (shell_path_to_windows — the funnel `cd` uses), so
        // `ls /usr/bin | wc -l` lists the bundled tree instead of failing
        // with rc=2 on the verbatim POSIX spelling. MSYS model evidence:
        // Git Bash hands native children `D:/Git/usr/bin` for /usr/bin and
        // the %TEMP% spelling for /tmp.
        let root = i164_fixture_root();
        let env_vars = i164_rooted_env(&root);
        let mut env_with_home = env_vars.clone();
        env_with_home.insert("HOME".to_string(), r"C:\Users\i164home".to_string());

        let dispatcher = root.join("usr").join("bin").join("winuxcmd.exe");
        let applet = root.join("usr").join("bin").join("ls.exe");
        let native = PathBuf::from("git.exe");
        let cmd = PathBuf::from(r"C:\Windows\System32\cmd.exe");

        let usr_bin = root.join("usr").join("bin").to_string_lossy().into_owned();
        let usr = root.join("usr").to_string_lossy().into_owned();
        let etc_conf = root
            .join("etc")
            .join("i164.conf")
            .to_string_lossy()
            .into_owned();
        let tmp_base = std::env::temp_dir().to_string_lossy().into_owned();

        for (operand, expected) in [
            ("/usr", usr.as_str()),
            ("/usr/bin", usr_bin.as_str()),
            ("/etc/i164.conf", etc_conf.as_str()),
        ] {
            // Applet class 1: the winuxcmd dispatcher route (prepends the
            // dispatch name, then the translated operand).
            let (command, _) = external_command_for_named_program(
                &dispatcher,
                Some("ls"),
                &[operand.to_string()],
                &env_vars,
            );
            let argv = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(argv, vec!["ls".to_string(), expected.to_string()]);

            // Applet class 2: an applet executable under the WinuxCmd tree.
            assert_eq!(child_argv(&applet, &[operand], &env_vars), vec![expected]);

            // Native exe: same form (this was already the Option B native
            // behavior — pin it as the class invariant).
            assert_eq!(child_argv(&native, &[operand], &env_vars), vec![expected]);

            // cmd.exe: the root operand translates while /C switches stay
            // verbatim (windows_cmd_switches_are_not_mapped_into_shell_root
            // pins the switch half).
            assert_eq!(
                child_argv(&cmd, &["/C", "echo", operand], &env_vars),
                vec!["/C".to_string(), "echo".to_string(), expected.to_string()]
            );
        }

        // /tmp maps to the per-user temp base (niubash#94), not the install
        // tree — exactly the target MSYS uses for native children.
        assert_eq!(
            child_argv(&dispatcher, &["/tmp"], &env_vars),
            vec![tmp_base]
        );
        assert_eq!(
            child_argv(&applet, &["/tmp/x"], &env_vars),
            vec![std::env::temp_dir()
                .join("x")
                .to_string_lossy()
                .into_owned()]
        );

        // /home maps to the real home's parent (HOME/USERPROFILE map).
        assert_eq!(
            child_argv(&dispatcher, &["/home"], &env_with_home),
            vec![r"C:\Users".to_string()]
        );
        assert_eq!(
            child_argv(&native, &["/home/u/x"], &env_with_home),
            vec![r"C:\Users\u\x".to_string()]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_virtual_system_root_arguments_keep_option_b_shapes_verbatim() {
        // niubash#164 does NOT widen translation past the root class: the
        // regression guards for the sibling shapes must hold for the applet
        // child. /c/Windows and /cygdrive/d/x are drive forms the applet's
        // own native layer resolves (Option B, niubash#124(b)/#62); "/" is
        // an operand character (niubash#153); /dev/* belongs to the child's
        // descriptor map (rubash#120); switches, relative words and
        // leading-backslash data never enter the root class.
        //
        // /mnt/c is deliberately NOT in this list: WinuxCmd's
        // normalize_api_operand_w has no /mnt arm (it folds /cygdrive/d/...
        // to /d/... and then maps a bare /X/... drive letter, a test
        // /mnt/c cannot satisfy because 'm' is followed by 'n'), so a
        // verbatim /mnt/c operand reaches the applet unresolvable. It is
        // translated engine-side for every child class now; the pin lives
        // in windows_mnt_drive_argument_translates_for_every_child.
        let root = i164_fixture_root();
        let env_vars = i164_rooted_env(&root);
        let dispatcher = root.join("usr").join("bin").join("winuxcmd.exe");

        let verbatim = [
            "/c/Windows",
            "/cygdrive/d/x",
            "/",
            "/dev/null",
            "/dev/stdout",
            "/nologo",
            "/CN=test",
            "relative/file",
            r"\n",
            "--flag=/usr/bin",
        ];
        for operand in verbatim {
            assert_eq!(
                child_argv(&dispatcher, &[operand], &env_vars),
                vec![operand.to_string()],
                "operand {operand:?} must stay verbatim for the applet"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_mnt_drive_argument_translates_for_every_child() {
        // WSL drive forms have no arm in WinuxCmd's normalize_api_operand_w
        // (unlike /cygdrive/d/... which it folds, and /d/... which it maps),
        // so the engine must translate them itself. Before this, `cd /mnt/c`
        // worked (the builtin resolves through shell_path_to_windows, which
        // owns a /mnt branch) while `ls /mnt/c` and every other external
        // command failed — the form only reached a child verbatim.
        let root = i164_fixture_root();
        let env_vars = i164_rooted_env(&root);
        let dispatcher = root.join("usr").join("bin").join("winuxcmd.exe");

        // Shape class: only /mnt/X and /mnt/X/... are drive forms.
        for drive_form in ["/mnt/c", "/mnt/c/Users", "/mnt/D", "/mnt/d/repo/x", "/mnt/z"] {
            assert!(
                windows_mnt_drive_argument(drive_form),
                "{drive_form:?} is a /mnt drive form"
            );
        }
        for ordinary in ["/mnt", "/mnt/", "/mnt/cfoo", "/mnt/123", "/mntx/c", "/usr/bin"] {
            assert!(
                !windows_mnt_drive_argument(ordinary),
                "{ordinary:?} is not a /mnt drive form"
            );
        }
        // Only ONE letter is a drive designator: /mnt/CD is not a drive form,
        // and neither is a bare /mnt/<letter> with a second letter behind it.
        assert!(!windows_mnt_drive_argument("/mnt/CD"));
        // An uppercase letter is a valid drive designator (the resolver
        // upper-cases it; the shape test is case-insensitive by construction).
        assert!(windows_mnt_drive_argument("/mnt/D"));

        // The applet receives the Windows spelling, not the verbatim word.
        let argv = child_argv(&dispatcher, &["/mnt/c"], &env_vars);
        assert_eq!(
            argv,
            vec![r"C:\".to_string()],
            "/mnt/c must reach the applet as C:\\"
        );
        let argv = child_argv(&dispatcher, &["/mnt/d/repo/x"], &env_vars);
        assert_eq!(
            argv,
            vec![r"D:\repo\x".to_string()],
            "/mnt/d/repo/x must reach the applet as D:\\repo\\x"
        );

        // The bare MSYS shapes stay verbatim: /d/x and /cygdrive/d/x are
        // resolved by the applet's own native layer and must not be
        // double-translated by the engine.
        let argv = child_argv(&dispatcher, &["/d/repo/x"], &env_vars);
        assert_eq!(argv, vec!["/d/repo/x".to_string()]);
        let argv = child_argv(&dispatcher, &["/cygdrive/d/x"], &env_vars);
        assert_eq!(argv, vec!["/cygdrive/d/x".to_string()]);

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_virtual_system_root_arguments_shell_wrapped_children_verbatim() {
        // A child SHELL (ENOEXEC re-entry, execute_cmd.c:6252) resolves
        // /usr/... through its own identical root map and must see the
        // words verbatim — GNU hands the child shell argv raw. argv[0] is
        // the child shell's script word ($0), not an operand.
        let root = i164_fixture_root();
        let env_vars = i164_rooted_env(&root);
        let script = root.join("i164script.sh");
        fs::write(&script, b"#!/bin/sh\n").unwrap();

        let argv = child_argv(&script, &["/usr/bin", "/etc/i164.conf"], &env_vars);
        assert_eq!(argv.len(), 3, "shell-wrapped child: $0 + two operands");
        assert_eq!(argv[0], script.to_string_lossy().replace('\\', "/"));
        assert_eq!(argv[1], "/usr/bin");
        assert_eq!(argv[2], "/etc/i164.conf");

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_virtual_system_root_arguments_legacy_dialect_reverts() {
        // __RUBASH_ARGV_DIALECT=legacy is the field rollback hatch for the
        // argv dialect: it must also revert the #164 translation, landing
        // on the pre-fix behavior (existence-gated translation through the
        // native funnel: an EXISTING root target still translates, a
        // missing one stays verbatim).
        let root = i164_fixture_root();
        let mut env_vars = i164_rooted_env(&root);
        env_vars.insert("__RUBASH_ARGV_DIALECT".to_string(), "legacy".to_string());
        let dispatcher = root.join("usr").join("bin").join("winuxcmd.exe");

        // Missing root target under legacy: verbatim (pre-Option-B gate).
        assert_eq!(
            child_argv(&dispatcher, &["/usr/definitely/missing"], &env_vars),
            vec!["/usr/definitely/missing".to_string()]
        );
        // Existing root target under legacy: still translated.
        assert_eq!(
            child_argv(&dispatcher, &["/usr/bin"], &env_vars),
            vec![root.join("usr").join("bin").to_string_lossy().into_owned()]
        );

        let _ = fs::remove_dir_all(&root);
    }

    // ---- unixwin/niubash#177: operand spelling resolution ----------------

    /// Fixture install tree for the operand-resolution family:
    /// <root>/usr/bin/{winuxcmd.exe marker,seq.exe} (NO plain `seq`),
    /// <root>/usr/bin/{dirboth/ directory + dirboth.exe file} (collision),
    /// <root>/etc/i177.conf (existing as-spelled text operand).
    #[cfg(windows)]
    fn i177_fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join("rubash-i177-root");
        let _ = fs::remove_dir_all(&root);
        let usr_bin = root.join("usr").join("bin");
        fs::create_dir_all(usr_bin.join("dirboth")).unwrap();
        fs::create_dir_all(root.join("etc")).unwrap();
        fs::write(usr_bin.join("winuxcmd.exe"), b"MZ").unwrap();
        fs::write(usr_bin.join("seq.exe"), b"MZ").unwrap();
        fs::write(usr_bin.join("dirboth.exe"), b"MZ").unwrap();
        fs::write(root.join("etc").join("i177.conf"), b"one\n").unwrap();
        root
    }

    #[cfg(windows)]
    #[test]
    fn windows_operand_file_path_prefers_existing_spelling_then_exe() {
        // The MSYS open/exec order, pinned directly: (1) an existing
        // as-spelled path (file OR directory) is handed out verbatim — a
        // directory `dirboth` is never swapped for a coincidental
        // `dirboth.exe`; (2) a name absent as-spelled resolves to the
        // existing `seq.exe` spelling (niubash#177: `head -1 /usr/bin/seq`
        // must open what `test -f /usr/bin/seq` says exists); (3) a name
        // missing under BOTH spellings keeps the as-spelled form — never a
        // blind append (the child reports the original operand, as before).
        let root = i177_fixture_root();
        let env_vars = i164_rooted_env(&root);

        let seq = root.join("usr").join("bin").join("seq");
        assert_eq!(
            windows_operand_file_path(seq.clone(), &env_vars),
            root.join("usr").join("bin").join("seq.exe"),
            "missing as-spelled + seq.exe on disk resolves to the exe"
        );

        let conf = root.join("etc").join("i177.conf");
        assert_eq!(
            windows_operand_file_path(conf.clone(), &env_vars),
            conf,
            "existing as-spelled file is handed out verbatim"
        );

        let dir = root.join("usr").join("bin").join("dirboth");
        assert_eq!(
            windows_operand_file_path(dir.clone(), &env_vars),
            dir,
            "existing directory wins over the .exe sibling"
        );

        let missing = root.join("usr").join("bin").join("nope");
        assert_eq!(
            windows_operand_file_path(missing.clone(), &env_vars),
            missing,
            "neither spelling exists: as-spelled stands, no blind append"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn virtual_system_root_file_operands_resolve_executable_spelling() {
        // The argv funnel hands every non-shell child class the RESOLVED
        // spelling for a file operand that exists only as `<name>.exe`
        // (`head -1 /usr/bin/seq` used to fail with "cannot open
        // ...\usr\bin\seq", niubash#177), while directory operands keep the
        // plain translated form and missing operands keep the original
        // spelling for the child's own error.
        let root = i177_fixture_root();
        let env_vars = i164_rooted_env(&root);
        let dispatcher = root.join("usr").join("bin").join("winuxcmd.exe");
        let applet = root.join("usr").join("bin").join("seq.exe");
        let native = PathBuf::from("git.exe");

        let seq_exe = root.join("usr").join("bin").join("seq.exe");
        let usr_bin = root.join("usr").join("bin");
        let conf = root.join("etc").join("i177.conf");
        let missing = root.join("usr").join("bin").join("nope");

        for program in [&dispatcher, &applet, &native] {
            assert_eq!(
                child_argv(program, &["/usr/bin/seq"], &env_vars),
                vec![seq_exe.to_string_lossy().into_owned()],
                "{program:?} must receive the resolved operand spelling"
            );
            assert_eq!(
                child_argv(program, &["/usr/bin"], &env_vars),
                vec![usr_bin.to_string_lossy().into_owned()],
                "{program:?}: directory operand keeps the plain form"
            );
            assert_eq!(
                child_argv(program, &["/etc/i177.conf"], &env_vars),
                vec![conf.to_string_lossy().into_owned()],
                "{program:?}: existing file operand keeps the plain form"
            );
            assert_eq!(
                child_argv(program, &["/usr/bin/nope"], &env_vars),
                vec![missing.to_string_lossy().into_owned()],
                "{program:?}: missing under both spellings keeps the original"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn virtual_system_root_operand_resolution_keeps_option_b_and_legacy_verbatim() {
        // The resolution rides ONLY on the root-map translation (#164):
        // drive forms, /dev/*, "/" and switches never enter the class, and
        // the legacy dialect hatch reverts the whole translation unchanged.
        let root = i177_fixture_root();
        let mut legacy = i164_rooted_env(&root);
        legacy.insert("__RUBASH_ARGV_DIALECT".to_string(), "legacy".to_string());
        let dispatcher = root.join("usr").join("bin").join("winuxcmd.exe");

        for verbatim in [
            "/c/Windows",
            "/mnt/c",
            "/cygdrive/d/x",
            "/",
            "/dev/null",
            "/nologo",
            "relative/file",
        ] {
            assert_eq!(
                child_argv(&dispatcher, &[verbatim], &i164_rooted_env(&root)),
                vec![verbatim.to_string()],
                "{verbatim:?} must stay verbatim for the applet"
            );
        }

        // Legacy dialect: the #164 translation never applies, so neither
        // does the operand resolution (pre-fix behavior byte-for-byte).
        assert_eq!(
            child_argv(&dispatcher, &["/usr/bin/seq"], &legacy),
            vec!["/usr/bin/seq".to_string()]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_virtual_system_root_argument_shape_class() {
        // The admission predicate owns the class boundary: install-tree
        // components under a configured root, the temp namespaces, and
        // /home. Drive forms, /dev, switches, relative words and the empty
        // string are not in the class.
        let root = i164_fixture_root();
        let env_vars = i164_rooted_env(&root);

        for shape in [
            "/usr",
            "/usr/bin",
            "/bin",
            "/etc",
            "/etc/passwd",
            "/lib",
            "/lib64",
            "/opt",
            "/sbin",
            "/var/log",
            "/var/tmp",
            "/tmp",
            "/tmp/x",
            "/home",
            "/home/u",
        ] {
            assert!(
                windows_virtual_system_root_argument(shape, &env_vars),
                "{shape:?} is a virtual system root"
            );
        }
        for shape in [
            "/c/Windows",
            "/d",
            "/mnt/c",
            "/cygdrive/d/x",
            "/dev/null",
            "/",
            "/nologo",
            "/CN=test",
            "usr/bin",
            "",
        ] {
            assert!(
                !windows_virtual_system_root_argument(shape, &env_vars),
                "{shape:?} is not a virtual system root"
            );
        }

        // Without a configured shell root the install-tree components have
        // no root-map entry (the tools-dir fallback belongs to command
        // lookup, not argv), while the temp namespaces still do.
        let unrooted = HashMap::new();
        assert!(!windows_virtual_system_root_argument("/usr/bin", &unrooted));
        assert!(!windows_virtual_system_root_argument("/etc", &unrooted));
        assert!(windows_virtual_system_root_argument("/tmp", &unrooted));
        assert!(windows_virtual_system_root_argument(
            "/var/tmp/x",
            &unrooted
        ));

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_merged_listing_scan_preserves_path_and_extension_order() {
        // rubash#159 merged scan must match the per-candidate stat walk it
        // replaced (GNU findcmd.c:623 find_user_command_in_path: first
        // match wins in PATH order; executable_candidate order within a
        // directory: PATHEXT extensions before the bare extensionless
        // file). Case-insensitive listing membership mirrors the
        // case-insensitive Windows stat the old walk relied on.
        let dir_a = std::env::temp_dir().join("rubash-merged-scan-a");
        let dir_b = std::env::temp_dir().join("rubash-merged-scan-b");
        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();
        fs::write(dir_a.join("toolA.exe"), b"").unwrap();
        fs::write(dir_b.join("toolA.exe"), b"").unwrap(); // later PATH dir loses
        fs::write(dir_b.join("TOOLB.exe"), b"").unwrap(); // case-insensitive hit
        let wrapper = dir_b.join("code.cmd");
        let script = dir_b.join("code");
        fs::write(&wrapper, b"").unwrap();
        fs::write(&script, b"").unwrap(); // extension probe wins over bare file

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "PATH".to_string(),
            format!(
                "{};{};C:/definitely/missing-dir",
                dir_a.display(),
                dir_b.display()
            ),
        );
        env_vars.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        assert_eq!(
            find_user_command("toolA", &env_vars),
            Some(dir_a.join("toolA.exe"))
        );
        // The candidate path is built from the lookup name; the
        // case-insensitive hit resolves to that spelling, exactly as the
        // per-candidate stat walk returned it (with_extension candidate).
        assert_eq!(
            find_user_command("toolb", &env_vars),
            Some(dir_b.join("toolb.exe"))
        );
        assert_eq!(find_user_command("code", &env_vars), Some(wrapper));
        // A miss in every listed directory stays a miss.
        assert_eq!(find_user_command("no_such_tool_xyz", &env_vars), None);

        let _ = fs::remove_dir_all(dir_a);
        let _ = fs::remove_dir_all(dir_b);
    }

    #[cfg(windows)]
    #[test]
    fn windows_listing_negative_cache_boundary_hash_r_and_path_change() {
        // rubash#159 documented divergence boundary: the per-directory
        // listing and the per-name negative result are process-internal
        // caches keyed by the env fingerprint. A file created inside an
        // already-listed PATH directory after the miss is NOT found --
        // not for the original name (per-name negative result, pre-#159
        // behavior) and not for a never-seen name either (listing
        // staleness). `hash -r` (phash_flush, builtins/hash.def:150) and
        // any PATH change must make it findable again.
        let dir = std::env::temp_dir().join("rubash-listing-negative-boundary");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PATH".to_string(), dir.to_string_lossy().to_string());

        assert_eq!(find_user_command("late_tool", &env_vars), None);
        assert_eq!(find_user_command("late_tool2", &env_vars), None);

        fs::write(dir.join("late_tool.exe"), b"").unwrap();
        fs::write(dir.join("late_tool2.exe"), b"").unwrap();

        // Sanctioned staleness: both lookups consult cached negatives and
        // a cached listing taken before the files existed.
        assert_eq!(find_user_command("late_tool", &env_vars), None);
        assert_eq!(find_user_command("late_tool3", &env_vars), None);

        // `hash -r` drops everything; the fresh listing sees the files.
        clear_command_lookup_cache();
        assert_eq!(
            find_user_command("late_tool", &env_vars),
            Some(dir.join("late_tool.exe"))
        );

        // Re-arm the stale state, then change PATH: fingerprint mismatch
        // resets listings along with results.
        clear_command_lookup_cache();
        assert_eq!(find_user_command("late_tool4", &env_vars), None);
        fs::write(dir.join("late_tool4.exe"), b"").unwrap();
        env_vars.insert(
            "PATH".to_string(),
            format!("{};{}", std::env::temp_dir().display(), dir.display()),
        );
        assert_eq!(
            find_user_command("late_tool4", &env_vars),
            Some(dir.join("late_tool4.exe"))
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_nohash_mode_memorizes_misses_but_not_hits() {
        // GNU findcmd.c:348-427 search_for_command with hashing disabled
        // (`set +h`): no phash_search / phash_insert, every lookup
        // re-walks PATH. rubash#159 memoizes only the misses on that path
        // (in-process, fingerprint-keyed); positives must keep GNU's
        // re-scan semantics: a command that appears in an EARLIER PATH
        // directory mid-session is found on the next lookup.
        let dir0 = std::env::temp_dir().join("rubash-nohash-memo-0");
        let dir1 = std::env::temp_dir().join("rubash-nohash-memo-1");
        let _ = fs::remove_dir_all(&dir0);
        let _ = fs::remove_dir_all(&dir1);
        fs::create_dir_all(&dir0).unwrap();
        fs::create_dir_all(&dir1).unwrap();
        let later = dir1.join("memo_tool.exe");
        fs::write(&later, b"").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "PATH".to_string(),
            format!("{};{}", dir0.display(), dir1.display()),
        );
        env_vars.insert("__RUBASH_SETOPT_hashall".to_string(), "0".to_string());

        // Positive: NOT memoized in the results map. Deletion is observed
        // on the next lookup because every listing hit is confirmed with
        // a live is_file() -- with hashing on, the stale cached path
        // would be returned instead (GNU phash without checkhash).
        assert_eq!(
            find_user_command("memo_tool", &env_vars),
            Some(later.clone())
        );
        fs::remove_file(&later).unwrap();
        assert_eq!(find_user_command("memo_tool", &env_vars), None);

        // Positives track a PATH reorder (fingerprint change resets the
        // scan scaffold): the now-first directory wins. A file created
        // inside an already-listed directory WITHOUT a fingerprint change
        // stays invisible -- the sanctioned rubash#159 staleness below.
        fs::write(dir1.join("memo_hit.exe"), b"").unwrap();
        fs::write(dir0.join("memo_hit.exe"), b"").unwrap();
        env_vars.insert(
            "PATH".to_string(),
            format!("{};{}", dir1.display(), dir0.display()),
        );
        assert_eq!(
            find_user_command("memo_hit", &env_vars),
            Some(dir1.join("memo_hit.exe"))
        );
        env_vars.insert(
            "PATH".to_string(),
            format!("{};{}", dir0.display(), dir1.display()),
        );
        assert_eq!(
            find_user_command("memo_hit", &env_vars),
            Some(dir0.join("memo_hit.exe"))
        );

        // Miss: memoized process-internally; the file created after the
        // miss is only visible once the fingerprint changes (documented
        // divergence; GNU would find it immediately).
        assert_eq!(find_user_command("memo_missing", &env_vars), None);
        fs::write(dir1.join("memo_missing.exe"), b"").unwrap();
        assert_eq!(find_user_command("memo_missing", &env_vars), None);
        env_vars.insert(
            "PATH".to_string(),
            format!("{};{}", dir1.display(), dir0.display()),
        );
        assert_eq!(
            find_user_command("memo_missing", &env_vars),
            Some(dir1.join("memo_missing.exe"))
        );

        let _ = fs::remove_dir_all(dir0);
        let _ = fs::remove_dir_all(dir1);
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

/// The canonical PWD display form: `D:/…` when the host enabled the
/// shell-native path style (`shell_path_style_enabled`, e.g. WINUXSH
/// exporting WINUXSH_SHELL_PATH_STYLE), otherwise the POSIX `/<drive>/…`
/// spelling that matches the startup PWD and GNU's single canonical form
/// (builtins/cd.def:136-175 bindpwd stores one value; pwd.def echoes it).
/// A `D:/…/D:/…` doubled-drive artifact is collapsed in native mode.
pub fn shell_pwd_display_path(path: &str) -> String {
    let value = path.replace('\\', "/");
    if shell_path_style_enabled() {
        if value.len() >= 5
            && value.as_bytes()[1] == b':'
            && value.as_bytes()[2] == b'/'
            && value.as_bytes()[3].to_ascii_lowercase() == value.as_bytes()[0].to_ascii_lowercase()
            && value.as_bytes()[4] == b'/'
        {
            return format!("{}{}", &value[..3], &value[5..]);
        }
        return if value.is_empty() {
            "/".to_string()
        } else {
            value
        };
    }
    if cfg!(windows)
        && value.len() >= 3
        && value.as_bytes()[1] == b':'
        && value.as_bytes()[2] == b'/'
    {
        let drive = (value.as_bytes()[0] as char).to_ascii_lowercase();
        return format!("/{drive}/{}", &value[3..]);
    }
    if value.is_empty() {
        "/".to_string()
    } else {
        value
    }
}
