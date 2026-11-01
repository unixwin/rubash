//! Dynamic (loadable) builtin support for `enable -f` / `enable -d`.
//!
//! GNU ownership:
//! - `builtins/enable.def:dyn_load_builtin` (bash 5.3, vendored tree
//!   `third_party/bash/builtins/enable.def:318`) — `dlopen`, `dlsym
//!   NAME_struct`, the optional `NAME_builtin_load` hook, and the
//!   `struct builtin` flag/handle updates.
//! - `builtins/enable.def:dyn_unload_builtin` (:536) — the optional
//!   `NAME_builtin_unload` hook, handle reference counting, `dlclose`,
//!   and removal from the builtin table.
//! - `builtins.h:53` — the ABI this module mirrors:
//!
//!   ```c
//!   struct builtin {
//!     char *name;
//!     sh_builtin_func_t *function;  /* int (*)(WORD_LIST *) */
//!     int flags;
//!     char * const *long_doc;
//!     const char *short_doc;
//!     char *handle;
//!   };
//!   ```
//!
//!   The task brief mentions a "reserved words" field; the vendored
//!   bash 5.3 header has no such member, so the six fields above are
//!   the exact layout. Adding a field would break every real loadable
//!   builtin and is deliberately not done.
//!
//! `WORD_LIST` / `WORD_DESC` mirror `command.h:131-140`.  The loaded
//! function receives only the ARGUMENTS: `execute_cmd.c:5097
//! execute_builtin` calls `(*builtin) (words->next)`, not `words`.
//! `builtins/common.c:345 make_builtin_argv` supplies argv[0] from
//! `this_command_name` itself, confirming the list does not carry the
//! command word.
//!
//! The registry is process-global, like GNU's `shell_builtins` table
//! (`builtins.h:65`); it is not part of `ShellState`.  dlopen handles
//! are process resources in both implementations.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// `builtins.h:43-50` flag bits used by the loader.
pub(crate) const BUILTIN_ENABLED: c_int = 0x01;
pub(crate) const BUILTIN_DELETED: c_int = 0x02;
pub(crate) const STATIC_BUILTIN: c_int = 0x04;
pub(crate) const SPECIAL_BUILTIN: c_int = 0x08;

/// `command.h:131-134 WORD_DESC`.
#[repr(C)]
pub(crate) struct WordDesc {
    pub word: *mut c_char,
    pub flags: c_int,
}

/// `command.h:137-140 WORD_LIST`.
#[repr(C)]
pub(crate) struct WordList {
    pub next: *mut WordList,
    pub word: *mut WordDesc,
}

/// `builtins.h:53-61 struct builtin`.
#[repr(C)]
pub(crate) struct Builtin {
    pub name: *mut c_char,
    pub function: Option<BuiltinFn>,
    pub flags: c_int,
    pub long_doc: *const *const c_char,
    pub short_doc: *const c_char,
    pub handle: *mut c_void,
}

/// `general.h:240 sh_builtin_func_t`: `int (*)(WORD_LIST *)`.
pub(crate) type BuiltinFn = unsafe extern "C" fn(*mut WordList) -> c_int;
type LoadFn = unsafe extern "C" fn(*mut c_char) -> c_int;
type UnloadFn = unsafe extern "C" fn(*mut c_char);

struct LoadedBuiltin {
    name: String,
    function: BuiltinFn,
    flags: c_int,
    struct_ptr: *mut Builtin,
    handle: *mut c_void,
}

// SAFETY: the pointers are opaque dlopen handles / plugin-owned structs
// whose lifetime is managed by this module under REGISTRY's mutex.  The
// shell executes builtins on one thread; the only cross-thread use is
// the registry lookup itself.
unsafe impl Send for LoadedBuiltin {}

static REGISTRY: OnceLock<Mutex<Vec<LoadedBuiltin>>> = OnceLock::new();
/// Fast zero check for the hot dispatch path: most shells never load a
/// builtin, so `is_loaded` must not take a mutex in that case.
static LOADED_COUNT: AtomicUsize = AtomicUsize::new(0);

fn registry() -> &'static Mutex<Vec<LoadedBuiltin>> {
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Whether NAME is a currently registered loadable builtin.
pub(crate) fn is_loaded(name: &str) -> bool {
    if LOADED_COUNT.load(Ordering::Relaxed) == 0 {
        return false;
    }
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|builtin| builtin.name == name)
}

/// Names in load order (GNU appends new builtins to `shell_builtins`).
pub(crate) fn loaded_names() -> Vec<String> {
    if LOADED_COUNT.load(Ordering::Relaxed) == 0 {
        return Vec::new();
    }
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|builtin| builtin.name.clone())
        .collect()
}

/// GNU `SPECIAL_BUILTIN` on the loaded `struct builtin`.
pub(crate) fn is_special(name: &str) -> bool {
    if LOADED_COUNT.load(Ordering::Relaxed) == 0 {
        return false;
    }
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|builtin| builtin.name == name)
        .is_some_and(|builtin| builtin.flags & SPECIAL_BUILTIN != 0)
}

/// GNU `BUILTIN_ENABLED` on the loaded `struct builtin`.
pub(crate) fn is_enabled(name: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|builtin| builtin.name == name)
        .is_some_and(|builtin| builtin.flags & BUILTIN_ENABLED != 0)
}

/// Mirror `enable`'s enabled/disabled change into the loaded struct and
/// the plugin's own flags field, like GNU's `enable_shell_command`
/// (`enable.def:279-306`).
pub(crate) fn set_enabled(name: &str, enabled: bool) {
    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(builtin) = registry.iter_mut().find(|builtin| builtin.name == name) {
        if enabled {
            builtin.flags |= BUILTIN_ENABLED;
        } else {
            builtin.flags &= !BUILTIN_ENABLED;
        }
        // The loaded struct's flags field is observable by the plugin.
        unsafe {
            (*builtin.struct_ptr).flags = builtin.flags;
        }
    }
}

/// GNU `execute_cmd.c:4959 builtin_status`: loadable builtins may return
/// the shell-internal error codes from `shell.h:70-76`; the caller sees
/// the normalized public status (EX_USAGE/EX_BADSYNTAX -> 2, the other
/// `EX_SHERRBASE` values -> 1).
pub(crate) fn normalize_builtin_status(status: i32) -> i32 {
    const EX_SHERRBASE: i32 = 256;
    const EX_BADSYNTAX: i32 = 257;
    const EX_USAGE: i32 = 258;
    const EX_REDIRFAIL: i32 = 259;
    const EX_BADASSIGN: i32 = 260;
    const EX_EXPFAIL: i32 = 261;
    const EX_UTILERROR: i32 = 263;
    match status {
        EX_USAGE | EX_BADSYNTAX => 2,
        EX_REDIRFAIL | EX_BADASSIGN | EX_EXPFAIL | EX_UTILERROR => 1,
        status if status > EX_SHERRBASE => 1,
        status => status,
    }
}

/// Captured result of a loadable builtin invocation.
pub(crate) struct CallOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Invoke the loaded builtin with the shell's stdio captured, so its
/// C-level writes obey the command's redirects / `$()` capture exactly
/// like a builtin that returns bytes.  `args` are the command's
/// arguments only (GNU passes `words->next`); the plugin recovers the
/// command name from `this_command_name`.  Returns `None` when the name
/// is no longer loaded.
pub(crate) fn call_captured(name: &str, args: &[String]) -> Option<CallOutput> {
    let function = {
        let registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry
            .iter()
            .find(|builtin| builtin.name == name)
            .map(|builtin| builtin.function)?
    };

    let mut prepared = PreparedArgs::new(args)?;
    #[cfg(unix)]
    if let Some(capture) = StdioCapture::begin() {
        let status = unsafe { function(prepared.head()) };
        let (stdout, stderr) = capture.finish();
        return Some(CallOutput {
            status,
            stdout,
            stderr,
        });
    }
    let status = unsafe { function(prepared.head()) };
    Some(CallOutput {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    })
}

/// A GNU `WORD_LIST` built from owned argv strings.
struct PreparedArgs {
    // The CStrings own the bytes `WORD_DESC.word` points at; the Vecs
    // must outlive the call and must not reallocate after linking.
    _cstrings: Vec<CString>,
    _descs: Vec<WordDesc>,
    nodes: Vec<WordList>,
}

impl PreparedArgs {
    fn new(argv: &[String]) -> Option<Self> {
        let cstrings: Vec<CString> = argv
            .iter()
            .map(|arg| CString::new(arg.as_bytes()).ok())
            .collect::<Option<_>>()?;
        let mut descs: Vec<WordDesc> = cstrings
            .iter()
            .map(|arg| WordDesc {
                // WORD_DESC.word is `char *`; the plugin only reads it.
                word: arg.as_ptr() as *mut c_char,
                flags: 0,
            })
            .collect();
        let mut nodes: Vec<WordList> = descs
            .iter_mut()
            .map(|desc| WordList {
                next: std::ptr::null_mut(),
                word: desc as *mut WordDesc,
            })
            .collect();

        if !nodes.is_empty() {
            // Link with raw pointers so each node's `next` can name a
            // sibling without borrowing the vector twice.
            let desc_base = descs.as_mut_ptr();
            let node_base = nodes.as_mut_ptr();
            for offset in 0..nodes.len() - 1 {
                unsafe {
                    (*node_base.add(offset)).word = desc_base.add(offset);
                    (*node_base.add(offset)).next = node_base.add(offset + 1);
                }
            }
            let last = nodes.len() - 1;
            unsafe {
                (*node_base.add(last)).word = desc_base.add(last);
            }
        }

        Some(Self {
            _cstrings: cstrings,
            _descs: descs,
            nodes,
        })
    }

    fn head(&mut self) -> *mut WordList {
        if self.nodes.is_empty() {
            std::ptr::null_mut()
        } else {
            self.nodes.as_mut_ptr()
        }
    }
}

/// Result of one `enable -f` call.  `errors` and `warnings` are GNU
/// diagnostic bodies (the caller adds the `enable:` prolog).
pub(crate) struct LoadOutcome {
    pub loaded: Vec<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Load NAMES from FILENAME.  Returns `Err(body)` only for a failure
/// that prevents opening the shared object at all.
///
/// GNU `builtins/enable.def:335-348`: an absolute_program (contains a
/// slash) goes straight to `dlopen`; otherwise BASH_LOADABLES_PATH is
/// searched, and only when the variable is unset does the current
/// directory become the fallback.
pub(crate) fn load_builtin(
    filename: &str,
    names: &[String],
    loadables_path: Option<&str>,
    cwd: &Path,
    special: bool,
    disabled: bool,
) -> Result<LoadOutcome, String> {
    let open_path = resolve_shared_object(filename, loadables_path, cwd);

    let handle = dlopen(&open_path);
    if handle.is_null() {
        return Err(format!(
            "cannot open shared object {filename}: {}",
            dl_error()
        ));
    }

    let mut outcome = LoadOutcome {
        loaded: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let mut pending: Vec<LoadedBuiltin> = Vec::new();
    let mut pending_names: HashSet<String> = HashSet::new();

    for name in names {
        // GNU enable.def:374-378: builtin names may not contain slashes.
        if name.contains('/') || (cfg!(windows) && name.contains('\\')) {
            outcome
                .errors
                .push(format!("{name}: builtin names may not contain slashes"));
            continue;
        }
        // The brief's duplicate-name failure path.  GNU replaces an
        // existing table slot; rubash keeps the loaded object and its
        // handle explicit, so a second load of the same live name is a
        // diagnosed failure rather than a silent handle leak.  The
        // wording reuses GNU's `dynamic builtin already loaded`
        // (enable.def:405).
        if is_loaded(name) || pending_names.contains(name) {
            outcome
                .errors
                .push(format!("{name}: dynamic builtin already loaded"));
            continue;
        }

        let struct_name = format!("{name}_struct");
        let struct_ptr = dlsym(handle, &struct_name);
        if struct_ptr.is_null() {
            outcome.errors.push(format!(
                "cannot find {struct_name} in shared object {filename}: {}",
                dl_error()
            ));
            continue;
        }
        let builtin = struct_ptr as *mut Builtin;

        // GNU enable.def:395-418: optional NAME_builtin_load hook runs
        // before the struct is installed; a zero return vetoes the load.
        let load_name = format!("{name}_builtin_load");
        let load_func = dlsym(handle, &load_name);
        if !load_func.is_null() {
            let load_func: LoadFn = unsafe { std::mem::transmute(load_func) };
            let Ok(c_name) = CString::new(name.as_bytes()) else {
                outcome.errors.push(format!("{name}: invalid builtin name"));
                continue;
            };
            let status = unsafe { load_func(c_name.as_ptr() as *mut c_char) };
            if status == 0 {
                outcome.errors.push(format!(
                    "load function for {name} returns failure ({status}): not loaded"
                ));
                continue;
            }
        }

        // GNU enable.def:421-436: clear STATIC_BUILTIN, apply the
        // requested special/disabled flags, and remember the handle.
        unsafe {
            (*builtin).flags &= !STATIC_BUILTIN;
            if special {
                (*builtin).flags |= SPECIAL_BUILTIN;
            }
            if disabled {
                (*builtin).flags &= !BUILTIN_ENABLED;
            }
            (*builtin).handle = handle;
        }

        let Some(function) = (unsafe { (*builtin).function }) else {
            outcome.errors.push(format!(
                "cannot find a function in {struct_name} in shared object {filename}"
            ));
            continue;
        };

        pending_names.insert(name.clone());
        outcome.loaded.push(name.clone());
        pending.push(LoadedBuiltin {
            name: name.clone(),
            function,
            flags: unsafe { (*builtin).flags },
            struct_ptr: builtin,
            handle,
        });
    }

    if pending.is_empty() {
        // GNU enable.def:441-446: nothing in this object was usable.
        let _ = dlclose(handle);
        return Ok(outcome);
    }

    {
        let mut registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.extend(pending);
        LOADED_COUNT.store(registry.len(), Ordering::Relaxed);
    }
    Ok(outcome)
}

/// GNU `dyn_unload_builtin` (`enable.def:536`): run the optional
/// unload hook, close the handle when it has no other users, then
/// remove the entry.  A failed `dlclose` leaves the entry registered.
pub(crate) fn unload_builtin(name: &str) -> Result<(), String> {
    let (handle, found) = {
        let registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match registry.iter().find(|builtin| builtin.name == name) {
            Some(builtin) => (builtin.handle, true),
            None => (std::ptr::null_mut(), false),
        }
    };
    if !found {
        return Err(format!("{name}: not dynamically loaded"));
    }

    // enable.def:562-573: NAME_builtin_unload is void and optional.
    let unload_name = format!("{name}_builtin_unload");
    let unload_func = dlsym(handle, &unload_name);
    if !unload_func.is_null() {
        let unload_func: UnloadFn = unsafe { std::mem::transmute(unload_func) };
        if let Ok(c_name) = CString::new(name.as_bytes()) {
            unsafe { unload_func(c_name.as_ptr() as *mut c_char) };
        }
    }

    // enable.def:575-583: close only when the reference count reaches
    // one (this builtin is the last user of the handle).
    let refs = {
        let registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry
            .iter()
            .filter(|builtin| builtin.handle == handle)
            .count()
    };
    if refs == 1 {
        if let Err(message) = dlclose(handle) {
            return Err(format!("{name}: cannot delete: {message}"));
        }
    }

    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|builtin| builtin.name != name);
    LOADED_COUNT.store(registry.len(), Ordering::Relaxed);
    Ok(())
}

/// GNU `find_in_path`/`make_absolute` reduction for BASH_LOADABLES_PATH
/// (`enable.def:338-352`).
fn resolve_shared_object(filename: &str, loadables_path: Option<&str>, cwd: &Path) -> PathBuf {
    let candidate = Path::new(filename);
    let has_slash = filename.contains('/') || (cfg!(windows) && filename.contains('\\'));
    if has_slash || candidate.is_absolute() {
        return if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
    }

    if let Some(search) = loadables_path {
        for entry in search.split(':') {
            let directory = if entry.is_empty() { "." } else { entry };
            let path = if Path::new(directory).is_absolute() {
                Path::new(directory).join(filename)
            } else {
                cwd.join(directory).join(filename)
            };
            if path.is_file() {
                return path;
            }
        }
        // GNU: when BASH_LOADABLES_PATH is set, do not fall back to `.`.
        return cwd.join(filename);
    }

    cwd.join(filename)
}

// ---------------------------------------------------- C stdio capture

/// Redirect the process's fd 1/fd 2 into temporary files around the
/// plugin call.  GNU binds the builtin's redirections with dup2 before
/// calling its function (execute_cmd.c:5606 execute_builtin_or_function
/// -> redir.c do_redirections); a plugin writes through libc, so the
/// bytes must be captured here and replayed through rubash's buffered
/// builtin-output router to keep `>file`, pipelines and `$()` working.
#[cfg(unix)]
struct StdioCapture {
    stdout_file: *mut libc::FILE,
    stderr_file: *mut libc::FILE,
    saved_stdout: c_int,
    saved_stderr: c_int,
    restored: bool,
}

#[cfg(unix)]
impl StdioCapture {
    fn begin() -> Option<Self> {
        unsafe {
            let stdout_file = libc::tmpfile();
            if stdout_file.is_null() {
                return None;
            }
            let stderr_file = libc::tmpfile();
            if stderr_file.is_null() {
                libc::fclose(stdout_file);
                return None;
            }
            let saved_stdout = libc::dup(libc::STDOUT_FILENO);
            let saved_stderr = libc::dup(libc::STDERR_FILENO);
            if saved_stdout < 0 || saved_stderr < 0 {
                if saved_stdout >= 0 {
                    libc::close(saved_stdout);
                }
                if saved_stderr >= 0 {
                    libc::close(saved_stderr);
                }
                libc::fclose(stdout_file);
                libc::fclose(stderr_file);
                return None;
            }
            if libc::dup2(libc::fileno(stdout_file), libc::STDOUT_FILENO) < 0
                || libc::dup2(libc::fileno(stderr_file), libc::STDERR_FILENO) < 0
            {
                libc::dup2(saved_stdout, libc::STDOUT_FILENO);
                libc::dup2(saved_stderr, libc::STDERR_FILENO);
                libc::close(saved_stdout);
                libc::close(saved_stderr);
                libc::fclose(stdout_file);
                libc::fclose(stderr_file);
                return None;
            }
            Some(Self {
                stdout_file,
                stderr_file,
                saved_stdout,
                saved_stderr,
                restored: false,
            })
        }
    }

    fn finish(mut self) -> (Vec<u8>, Vec<u8>) {
        unsafe {
            // Flush every C stream (the plugin's printf buffers) before
            // restoring the real descriptors.
            libc::fflush(std::ptr::null_mut());
            libc::dup2(self.saved_stdout, libc::STDOUT_FILENO);
            libc::dup2(self.saved_stderr, libc::STDERR_FILENO);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
            self.restored = true;

            let stdout = read_capture(self.stdout_file);
            let stderr = read_capture(self.stderr_file);
            libc::fclose(self.stdout_file);
            libc::fclose(self.stderr_file);
            (stdout, stderr)
        }
    }
}

#[cfg(unix)]
impl Drop for StdioCapture {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        unsafe {
            libc::fflush(std::ptr::null_mut());
            libc::dup2(self.saved_stdout, libc::STDOUT_FILENO);
            libc::dup2(self.saved_stderr, libc::STDERR_FILENO);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
            libc::fclose(self.stdout_file);
            libc::fclose(self.stderr_file);
        }
    }
}

#[cfg(unix)]
unsafe fn read_capture(file: *mut libc::FILE) -> Vec<u8> {
    let fd = libc::fileno(file);
    if fd < 0 || libc::lseek(fd, 0, libc::SEEK_SET) < 0 {
        return Vec::new();
    }
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = libc::read(fd, buffer.as_mut_ptr() as *mut c_void, buffer.len());
        if read < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read as usize]);
    }
    data
}

// --------------------------------------------------------------- dlopen

#[cfg(unix)]
fn dlopen(path: &Path) -> *mut c_void {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return std::ptr::null_mut();
    };
    // Task requirement and GNU's non-AIX DLFLAGS (enable.def:326-331):
    // RTLD_LAZY.  RTLD_GLOBAL is added so a plugin can resolve symbols
    // (readline/history and any host exports) from the main executable
    // rather than pulling in a second copy of a library.
    unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) }
}

#[cfg(unix)]
fn dlsym(handle: *mut c_void, symbol: &str) -> *mut c_void {
    let Ok(symbol) = CString::new(symbol) else {
        return std::ptr::null_mut();
    };
    unsafe { libc::dlsym(handle, symbol.as_ptr()) }
}

#[cfg(unix)]
fn dlclose(handle: *mut c_void) -> Result<(), String> {
    let status = unsafe { libc::dlclose(handle) };
    if status == 0 {
        Ok(())
    } else {
        Err(dl_error())
    }
}

#[cfg(unix)]
fn dl_error() -> String {
    unsafe {
        let message = libc::dlerror();
        if message.is_null() {
            "unknown error".to_string()
        } else {
            CStr::from_ptr(message).to_string_lossy().into_owned()
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
    fn GetLastError() -> u32;
}

#[cfg(windows)]
fn dlopen(path: &Path) -> *mut c_void {
    let display = path.to_string_lossy().replace('\\', "/");
    let Ok(display) = CString::new(display) else {
        return std::ptr::null_mut();
    };
    unsafe { LoadLibraryA(display.as_ptr() as *const u8) }
}

#[cfg(windows)]
fn dlsym(handle: *mut c_void, symbol: &str) -> *mut c_void {
    let Ok(symbol) = CString::new(symbol) else {
        return std::ptr::null_mut();
    };
    unsafe { GetProcAddress(handle, symbol.as_ptr() as *const u8) }
}

#[cfg(windows)]
fn dlclose(handle: *mut c_void) -> Result<(), String> {
    let status = unsafe { FreeLibrary(handle) };
    if status != 0 {
        Ok(())
    } else {
        Err(dl_error())
    }
}

#[cfg(windows)]
fn dl_error() -> String {
    format!("Windows error {}", unsafe { GetLastError() })
}

#[cfg(not(any(unix, windows)))]
fn dlopen(_path: &Path) -> *mut c_void {
    std::ptr::null_mut()
}

#[cfg(not(any(unix, windows)))]
fn dlsym(_handle: *mut c_void, _symbol: &str) -> *mut c_void {
    std::ptr::null_mut()
}

#[cfg(not(any(unix, windows)))]
fn dlclose(_handle: *mut c_void) -> Result<(), String> {
    Err("dynamic loading is not supported on this platform".to_string())
}

#[cfg(not(any(unix, windows)))]
fn dl_error() -> String {
    "dynamic loading is not supported on this platform".to_string()
}

// Keep the unused-but-ABI-documented constants visible to the compiler
// on all platforms (BUILTIN_DELETED is part of the public GNU layout).
const _: c_int = BUILTIN_DELETED;
