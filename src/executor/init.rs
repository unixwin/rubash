use super::*;
use crate::shell::VariableStore;

fn shell_pwd_display(path: &str) -> String {
    // rubash#537: unconditional POSIX persona. The startup PWD (and the
    // self-injected TMPDIR default below) previously kept the native
    // `D:/…` spelling whenever a host exported WINUXSH_SHELL_PATH_STYLE —
    // which niu/winuxsh do for their whole lifetime — so every flagship
    // session ran with a drive-form $PWD and nvm.sh's nvm_find_up
    // death-looped on it (GNU bash on MSYS never reports `D:/…`).
    shell_pwd_display_impl(path)
}

#[cfg(windows)]
fn shell_pwd_display_impl(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        return format!("/{drive}/{}", &path[3..]);
    }
    path.to_string()
}

#[cfg(not(windows))]
fn shell_pwd_display_impl(path: &str) -> String {
    path.to_string()
}
/// Process-global flag: this process is a respawned internal `rubash -c`
/// child (the __RUBASH_SHELL_PID/__RUBASH_COPROC_CHILD marker was seen at
/// Executor::new AND its provenance validated — see
/// [`respawn_provenance_validated`]). Read by main.rs
/// apply_startup_job_control.
pub static RESPAWNED_CHILD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Transport key stamping the DIRECT spawner's identity on every
/// fork-shaped respawn (async/subshell wrapper, coproc, both
/// process-substitution forms): its process id, plus its kernel start
/// time where the platform exposes one (see [`spawner_stamp_value`]).
/// rubash#2136: the provenance claims
/// (`__RUBASH_SHELL_PID`, `__RUBASH_COPROC_CHILD`) and the state they
/// unlock (`$$` adoption, BASH_SUBSHELL/BASHPID seeds, the positional
/// transport) are only honored when this stamp names the process that
/// actually spawned us — the same getppid cross-check the PDEATHSIG
/// arming uses (rubash#867). Before the stamp existed, the claim keys
/// alone opened the gate, so any parent could forge a child's `$$`,
/// seed its positional parameters, or rewrite its subshell depth by
/// exporting two strings.
pub const RESPAWN_SPAWNER_PID_KEY: &str = "__RUBASH_SPAWNER_PID";

/// The parent process id of this process, where the platform exposes it
/// cheaply. Windows enumerates a Toolhelp snapshot, so callers invoke
/// this only when a provenance claim is actually present.
pub(crate) fn parent_process_id() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: getppid has no failure mode; it returns 1 (or the
        // subreaper) for an orphaned process.
        let pid = unsafe { libc::getppid() };
        if pid > 0 {
            return Some(pid as u32);
        }
        return None;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let own = std::process::id();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        let mut result = None;
        unsafe {
            if Process32FirstW(snapshot, &mut entry) != 0 {
                loop {
                    if entry.th32ProcessID == own {
                        result = Some(entry.th32ParentProcessID);
                        break;
                    }
                    if Process32NextW(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snapshot);
        }
        return result;
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// The value a fork-shaped spawn site stamps under
/// [`RESPAWN_SPAWNER_PID_KEY`]: the spawner's own pid, plus — on Linux —
/// its kernel start time (`/proc/self/stat` field 22, clock ticks since
/// boot) so a validated child can tell the spawner apart from an
/// unrelated process that later reused the pid.
pub(crate) fn spawner_stamp_value() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Some((_, Some(start), _)) = linux_proc_stat(std::process::id()) {
            return format!("{}:{start}", std::process::id());
        }
    }
    std::process::id().to_string()
}

/// Parse a spawner stamp into (pid, optional kernel start time).
fn parse_spawner_stamp(value: &str) -> Option<(u32, Option<u64>)> {
    match value.split_once(':') {
        Some((pid, start)) => Some((pid.parse().ok()?, start.parse().ok())),
        None => Some((value.parse().ok()?, None)),
    }
}

/// Linux `/proc/<pid>/stat`: the process state letter, kernel start
/// time, and virtual memory size. The comm in parentheses may contain
/// spaces or parentheses, so fields are counted from the LAST ')':
/// state is the first field after it, starttime the 20th (stat field
/// 22 overall), vsize the 21st (field 23).
#[cfg(target_os = "linux")]
fn linux_proc_stat(pid: u32) -> Option<(char, Option<u64>, Option<u64>)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start = fields.nth(18).and_then(|token| token.parse::<u64>().ok());
    let vsize = fields.next().and_then(|token| token.parse::<u64>().ok());
    Some((state, start, vsize))
}

/// Whether the stamped spawner is GONE — the orphan clause of
/// [`respawn_provenance_validated`]. A zombie does not count as alive:
/// an exited-but-unreaped spawner answers `kill(pid, 0)` and
/// `OpenProcess` exactly like a live one, but it will never run again.
/// Neither does an unrelated process that REUSED the dead spawner's
/// pid: where the stamp carries the spawner's kernel start time, a
/// live process whose start time differs is a reuser, not the spawner
/// (pid reuse inside the orphan window is routine under parallel test
/// load, and mistaking the reuser for a live spawner would strand a
/// genuine orphan's provenance).
fn stamped_spawner_gone(pid: u32, stamp_start: Option<u64>) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        match linux_proc_stat(pid) {
            None => true,
            Some((state, start, vsize)) => {
                // Z/X: zombie or dead. vsize 0: the process has already
                // released its address space in do_exit but its state
                // letter has not flipped to Z yet — the window in which
                // a just-orphaned child (already reparented to init,
                // which happens before the Z flip becomes visible) sees
                // its spawner as still "running". No live user process
                // has a zero address space, so this cannot misfire.
                matches!(state, 'Z' | 'X')
                    || vsize == Some(0)
                    || matches!((stamp_start, start), (Some(want), Some(got)) if want != got)
            }
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let _ = stamp_start;
        // SAFETY: signal 0 performs the existence/permission check
        // without delivering anything. (A zombie still answers here, so
        // on these platforms the orphan clause only fires once the
        // spawner has also been reaped.)
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return false;
        }
        return std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM);
    }
    #[cfg(windows)]
    {
        let _ = stamp_start;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return true;
        }
        let mut exit_code = 0u32;
        // STILL_ACTIVE: the spawner has not exited yet.
        let live = unsafe { GetExitCodeProcess(handle, &mut exit_code) } != 0 && exit_code == 259;
        unsafe { CloseHandle(handle) };
        return !live;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = stamp_start;
        false
    }
}

/// Decide whether a respawn provenance claim is established by this
/// process's actual parentage (rubash#2136). The claim keys and their
/// transported state ride the environment, which any parent can write,
/// so the strings alone prove nothing; the spawner stamp must agree
/// with a fact only the real spawner and the kernel share:
///
/// * the stamp names our live parent (`getppid()`), or
/// * we were orphaned to the namespace init (ppid 1) and the stamped
///   spawner is gone — the async child whose shell exited while it
///   started. A live unrelated parent fails both arms, which is what
///   makes a leaked or planted claim in a normal session inert.
///
/// A parent that deliberately stamps its own pid is in the same trust
/// class GNU extends to BASH_ENV/SHELLOPTS: it already controls the
/// child's whole environment and command line.
pub(crate) fn respawn_provenance_validated(
    claim_present: bool,
    spawner_stamp: Option<(u32, Option<u64>)>,
) -> bool {
    if !claim_present {
        return false;
    }
    let Some((stamp_pid, stamp_start)) = spawner_stamp else {
        return false;
    };
    match parent_process_id() {
        Some(ppid) if ppid == stamp_pid => true,
        Some(1) => stamped_spawner_gone(stamp_pid, stamp_start),
        _ => false,
    }
}

/// [`respawn_provenance_validated`] against the live process
/// environment — the form main.rs needs before `Executor::new` imports
/// it (the async-child entry face, rubash#2134).
pub fn respawn_provenance_validated_from_process_env() -> bool {
    let claim = std::env::var_os("__RUBASH_SHELL_PID").is_some()
        || std::env::var("__RUBASH_COPROC_CHILD").as_deref() == Ok("1");
    let stamp = std::env::var(RESPAWN_SPAWNER_PID_KEY)
        .ok()
        .and_then(|value| parse_spawner_stamp(&value));
    respawn_provenance_validated(claim, stamp)
}

impl Executor {
    pub fn new() -> Self {
        Self::new_inner(false)
    }

    /// startup21: the process's outermost executor, dropped immediately
    /// before process exit (main.rs run_main). Identical to
    /// [`Executor::new`] except the process-env snapshot is not built:
    /// its only reader is the Drop-time environment restore, which the
    /// process-exit executor skips (nothing can observe a dying process's
    /// environment), so the extra full-map clone per spawn was pure cost.
    pub fn new_process_exit() -> Self {
        Self::new_inner(true)
    }

    fn new_inner(process_exit: bool) -> Self {
        // rubash#1557: snapshot the process-inherited descriptor set
        // (fd >= 3) before anything in this constructor can open or
        // close a descriptor of our own (the signal-mailbox thread, env
        // import, ...). Registration happens at the end of construction,
        // after the __RUBASH_FD_* respawn restore, and re-validates each
        // descriptor live — a transient fd captured here but closed by
        // then is skipped. GNU has no such snapshot because it has no
        // table: redir.c checks the kernel per redirect (redir.c:1160).
        let inherited_fds_snapshot = crate::fd::inherited_open_fds();
        // G-01: the birth snapshot is a process-state read — take it
        // under the lock so a sibling's mirror write cannot tear it.
        let process_env_snapshot: HashMap<String, String> = if process_exit {
            HashMap::new()
        } else {
            crate::executor::local_helpers::with_process_state_lock(|| {
                crate::executor::env_helpers::process_env_pairs()
                    .into_iter()
                    .collect()
            })
        };
        // G-03: this executor's process-env write journal, pushed for the
        // rest of construction so the init-time __RUBASH_* clears below
        // are journaled as this lineage's writes (its Drop restores them
        // against the snapshot above, conditionally — see Drop).
        let env_write_journal: EnvWriteJournal =
            std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));
        let _env_journal_guard = push_env_journal(&env_write_journal);
        // VarTable::from_values also imports any __RUBASH_*_VARS attribute
        // lists a parent rubash left in the environment — the structured
        // form of the cross-process attribute transport.
        let mut env_vars = if process_exit {
            crate::shell::var_table::VarTable::from_values(
                crate::executor::env_helpers::process_env_pairs()
                    .into_iter()
                    .collect(),
            )
        } else {
            crate::shell::var_table::VarTable::from_values(process_env_snapshot.clone())
        };
        // On Windows, std::env::vars() returns PATH as "Path" (capital P).
        // Every rubash command-lookup site reads env_vars.get("PATH") (all caps),
        // which is a case-sensitive HashMap lookup — it misses on Windows.
        // Mirror the value into the all-caps key so external command lookup works.
        #[cfg(windows)]
        if let Some(path_val) = env_vars.remove("Path") {
            env_vars.entry("PATH".to_string()).or_insert(path_val);
        }
        // rubash#175: a script's $PATH must be the `:`-separated shell
        // semantic form (drive entries as /c/...), the way GNU shell.c
        // imports environ verbatim into a POSIX variable space. The Windows
        // process PATH is `;`-separated drive-letter text; importing it
        // verbatim leaves `${PATH%%:*}` sliced at the `C:` colon and
        // `PATH=/usr/bin:$PATH` glued to a `;`-list (ltmain func_path_progs,
        // rbenv-commands). Normalize once at this import boundary; native
        // children get the reverse conversion at materialization
        // (child_env_value -> shell_path_to_process). The process
        // environment itself keeps its Windows spelling for OS spawns that
        // inherit it unchanged.
        #[cfg(windows)]
        if let Some(path_val) = env_vars.remove("PATH") {
            env_vars.insert(
                "PATH".to_string(),
                crate::executor::path::process_path_to_shell(&path_val),
            );
        }

        // rubash#2078: name this executor's command-lookup cache slice. The
        // process-global cache in executor::path is a map keyed on this
        // token instead of a single fingerprint slot, so two Sessions in one
        // process (the embed API's supported shape, contract Q5) with
        // different PATH or hash states stop serving each other's parse
        // results. Fresh stamp per constructor entry; an env that already
        // carries the token keeps it — a lineage re-entering this
        // constructor on a cloned env (the fork-copy shape
        // command_substitution_executor uses) stays on the parent's slice,
        // which is exactly the sharing the hashed_cache_fork_guard brackets
        // assume. In-memory only: `__RUBASH_*` names are excluded from the
        // initial export set (env_helpers is_initial_export_candidate) and
        // from the spawn env (the rubash_spawn_inherited_state filter), so
        // the token never crosses a process boundary and each process
        // slices its own static map.
        if !env_vars.contains_key(crate::executor::path::SESSION_SCOPE_KEY) {
            static NEXT_SESSION_SCOPE: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let scope = NEXT_SESSION_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            env_vars.insert(
                crate::executor::path::SESSION_SCOPE_KEY.to_string(),
                format!("s{scope}"),
            );
        }

        // rubash#588: record the imported (normalized) PATH once at the
        // startup boundary. GNU command search walks the PATH variable
        // verbatim — search_for_command feeds value_cell(path) straight to
        // find_user_command_in_path (findcmd.c:383-386), and that walk
        // (findcmd.c:623-697 find_user_command_in_path) consults NOTHING
        // beyond the variable's elements; the only default a PATH-less
        // shell ever gets is a real startup assignment
        // (variables.c:550 set_if_not("PATH", DEFAULT_PATH_VALUE)). Rubash's
        // Windows fixture adds out-of-band namespaces (the winuxcmd
        // dispatcher, the /bin display paths) that stand in for that
        // default — legitimate only while PATH still holds its startup
        // value. path::path_is_user_owned compares the live PATH against
        // this snapshot: an explicit assignment — including a starving one
        // like `PATH=/nonexistent; hash -r; ls`, which GNU 5.3.0 answers
        // with 127 and `command -v ls` with rc 1 (WSL probe 2026-10-13) —
        // owns resolution from then on and the fixture stands down
        // (findcmd.c:623-697: a PATH element that resolves nothing yields
        // no command). The contains_key guard keeps subshell and
        // command-substitution executors, which re-enter this constructor
        // on an inherited env_vars map, on the process's original snapshot.
        if !env_vars.contains_key(crate::executor::path::INITIAL_PATH_KEY) {
            let startup_path = env_vars.get("PATH").cloned().unwrap_or_default();
            env_vars.insert(
                crate::executor::path::INITIAL_PATH_KEY.to_string(),
                startup_path,
            );
        }

        // Pin the host's POSIX toolset directory before scripts can
        // overwrite PATH: `PATH=/bin:/usr/bin` (invocation.tests) and
        // `command -p` (command.def _CS_PATH) both need the logical bin
        // namespace to keep resolving. env_var writes sync into the process
        // environment, so a runtime probe of std::env PATH is polluted.
        // startup21: the filesystem probe (3 is_file stats per PATH entry,
        // ~0.28ms per spawn) is deferred — record the startup PATH as the
        // probe source and pin it into __RUBASH_POSIX_TOOLS_DIR; the first
        // consumer (path.rs windows_posix_tools_dir) resolves it lazily,
        // memoized per PATH string, to the same directory the eager probe
        // found.
        #[cfg(windows)]
        if !env_vars.contains_key("__RUBASH_POSIX_TOOLS_DIR") {
            if let Some(startup_path) = env_vars.get("PATH").cloned() {
                crate::executor::path::pin_startup_tools_dir_source(startup_path.clone());
                env_vars.insert("__RUBASH_POSIX_TOOLS_DIR".to_string(), startup_path);
            }
        }

        // Seed the trap table a shell inherits from its environment: traps
        // ignored at startup become hard-ignores (trap.c), and WSL's init
        // leaves SIGRTMIN ignored for every child, which the GNU 5.3.0
        // baseline lists as "trap -- '' SIGRTMIN" in fresh shells.
        crate::builtins::trap::seed_startup_traps(&mut env_vars);

        // MSYS Bash exposes HOME even when the native Windows environment
        // only provides USERPROFILE.  Keep `$HOME` usable for scripts that
        // pass it to cd and other builtins, while preserving an explicitly
        // supplied (including empty) HOME value.
        #[cfg(windows)]
        if !env_vars.contains_key("HOME") {
            let fallback_home = env_vars.get("USERPROFILE").cloned().or_else(|| {
                env_vars
                    .get("HOMEDRIVE")
                    .zip(env_vars.get("HOMEPATH"))
                    .map(|(drive, path)| format!("{drive}{path}"))
            });
            if let Some(home) = fallback_home {
                env_vars.insert("HOME".to_string(), home);
            }
        }

        let (mut imported_functions, imported_function_def_infos, failed_function_imports) =
            import_exported_functions_from_env(&env_vars);
        // GNU variables.c: an imported function keeps att_exported — it
        // shows as `declare -fx` and re-exports to grandchildren
        // (rubash#1275). The import stripped the BASH_FUNC_* env entry
        // but never recorded the attribute.
        for name in imported_functions.keys() {
            mark_env_name(&mut env_vars, EXPORTED_FUNCTIONS, name);
        }
        // GNU variables.c import refusal (the ShellShock guard) reports
        // both lines under the shell's invocation name (rubash#1276).
        if !failed_function_imports.is_empty() {
            let shell_name = std::env::args()
                .next()
                .unwrap_or_else(|| "rubash".to_string());
            for name in &failed_function_imports {
                eprintln!("{shell_name}: warning: {name}: ignoring function definition attempt");
                eprintln!("{shell_name}: error importing function definition for `{name}'");
            }
        }
        // GNU variables.c:397-446 (initialize_shell_variables,
        // FUNCTION_IMPORT): a successfully imported exported function
        // becomes a shell function, NOT a variable — the BASH_FUNC_name%%
        // entry vanishes from the variable table and is re-exported to
        // children through the function export path
        // (make_func_export_array, variables.c:5124). Leaving it in
        // env_vars made `export -p` print `declare -x BASH_FUNC_x%%=() ...`
        // lines whose source is a syntax error (niubash issue #102).
        // A FAILED import keeps the raw entry: its name still fails the
        // valid-identifier filters, so listings stay clean while children
        // still receive the original value (GNU bind_invalid_envvar,
        // variables.c:3307).
        let imported_fn_env_names: Vec<String> = env_vars
            .keys()
            .filter(|env_name| {
                imported_function_name(env_name)
                    .is_some_and(|name| imported_functions.contains_key(name))
            })
            .cloned()
            .collect();
        for env_name in imported_fn_env_names {
            env_vars.remove(&env_name);
        }
        env_vars.remove("__RUBASH_CURRENT_FUNCTION");
        env_vars.remove("__RUBASH_IN_SOURCE");
        // rubash#2135: the function-stdin staging pair is execution
        // state, not import state. Every legitimate write happens
        // in-process long after import (the function-call stdin scope
        // in command_input_scope.rs / ast_exec.rs, the source builtin's
        // explicit-path staging) and every read consults this live
        // table, so an ambient copy can only be a plant: pre-fix, a
        // parent exporting __RUBASH_FUNCTION_STDIN=<file> made the
        // child's first function-call `read` — and external commands in
        // the staged scope — consume the named file instead of the
        // process's real stdin. Strip both keys (and their process-env
        // mirrors, so respawn builders seeded from the process env
        // cannot re-import them downstream).
        env_vars.remove(super::types::FUNCTION_STDIN);
        env_vars.remove(super::types::FUNCTION_STDIN_OFFSET);
        // rubash#2137 (F8): the command-substitution body marker is
        // bracketed execution state — set inside this process around an
        // embedded comsub run (command_substitution.rs) and around the
        // mutation fold (embedded_mutations.rs), then restored. An
        // ambient copy at import rewrote the fatal-expansion exit
        // status of a top-level shell (127 -> 1,
        // parameter_errors.rs expansion_fatal_status). Strip it; the
        // in-process writers re-establish it per run.
        env_vars.remove("__RUBASH_COMSUB_BODY");
        // rubash#2137 (F7): debugger mode is invocation state. Its only
        // legitimate writers are this process's own argv handling
        // (invocation.rs --debugger, main.rs option arms), which run
        // after import and write the live table; an ambient copy made
        // every -c child attempt a debugger startup (including the
        // ATTEMPTED latch, which then suppressed the real thing). Strip
        // both keys; own-argv handling re-establishes them.
        env_vars.remove("__RUBASH_DEBUGGER_MODE");
        env_vars.remove("__RUBASH_DEBUGGER_ATTEMPTED");
        // rubash#2259: the rest of the invocation-classification family
        // (local_helpers::INVOCATION_STATE_ENV_KEYS) gets the F7
        // adjudication above. Interactivity, login shape and the `-c`
        // shape are derived by THIS process's own argv/tty handling
        // after import (main.rs / invocation.rs — GNU shell.c:540-549
        // derives interactive_shell from the process's own shape,
        // never from the environment); an ambient copy is a parent's
        // classification leaking into this process, and pre-fix a
        // nested shell classified itself by it (the discard
        // classification rubash#2236 observed). Strip the whole family
        // (and its process-env mirrors below, so respawn builders
        // seeded from the process env cannot re-import it either);
        // own-argv handling re-establishes whatever applies.
        env_vars.remove("__RUBASH_INTERACTIVE");
        env_vars.remove("__RUBASH_INTERACTIVE_FLAG_OFF");
        env_vars.remove("__RUBASH_IS_C");
        env_vars.remove("__RUBASH_LOGIN_SHELL");
        // rubash#2136: a respawn provenance claim is honored only when
        // the spawner stamp agrees with this process's actual parent
        // (respawn_provenance_validated). An unvalidated claim is
        // indistinguishable from a planted one, so every piece of state
        // the claim unlocks below — `$$` adoption, the subshell-depth
        // and BASHPID seeds, the positional transport, the startup
        // door — falls back to the plain top-level defaults, while the
        // keys themselves are still consumed so they never surface as
        // shell variables or ride the process-env mirror into children.
        let coproc_child_claimed =
            env_vars.get("__RUBASH_COPROC_CHILD").map(String::as_str) == Some("1");
        let provenance_claim = coproc_child_claimed || env_vars.contains_key("__RUBASH_SHELL_PID");
        let spawner_stamp = env_vars
            .get(RESPAWN_SPAWNER_PID_KEY)
            .and_then(|value| parse_spawner_stamp(value));
        let internal_respawn = respawn_provenance_validated(provenance_claim, spawner_stamp);
        env_vars.remove(RESPAWN_SPAWNER_PID_KEY);
        env_vars.remove("__RUBASH_COPROC_CHILD");
        // rubash#2137 (F9): POSIX mode gets ONE source of truth — the
        // option state. Pre-fix the state was split: posix_mode_enabled
        // and ~30 dialect read sites consult the __RUBASH_POSIX_MODE
        // marker while `set -o` reports the __RUBASH_SETOPT_posix key,
        // and both were ambient-writable, so the two halves could
        // disagree in both directions (dialect on / option off, and
        // option on with inconsistent fatality). The only legitimate
        // in-tree writer keeps them in sync
        // (apply_posix_mode_transition), and the GNU entry channels —
        // POSIXLY_CORRECT and a SHELLOPTS/BASHOPTS naming posix — write
        // through it in initialize_fresh_shell_env_vars below. At
        // import, therefore:
        //   * a validated respawn is a fork-shaped child whose option
        //     state is transported state — re-project the marker from
        //     the transported option truth so the pair cannot split;
        //   * anything else is a top-level entry: drop the marker and
        //     the whole transported SETOPT family, and let the GNU
        //     channels re-derive the pair in sync below.
        if internal_respawn {
            if env_vars.get("__RUBASH_SETOPT_posix").map(String::as_str) == Some("1") {
                env_vars.insert("__RUBASH_POSIX_MODE".to_string(), "1".to_string());
            } else {
                env_vars.remove("__RUBASH_POSIX_MODE");
            }
        } else {
            env_vars.remove("__RUBASH_POSIX_MODE");
            env_vars.retain(|key, _| !key.starts_with("__RUBASH_SETOPT_"));
        }
        // rubash#1074: a respawned `&`/coproc child is GNU's forked subshell
        // (execute_cmd.c:1576 execute_in_subshell: subshell_level++ in the
        // child), so BASH_SUBSHELL inside it is parent_depth + 1, not 0.
        // The spawn sites carry the target depth in this marker; consume it
        // here so it never surfaces as a shell variable.
        let respawned_subshell_depth = if internal_respawn {
            env_vars
                .get("__RUBASH_SUBSHELL_DEPTH")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0)
        } else {
            0
        };
        env_vars.remove("__RUBASH_SUBSHELL_DEPTH");
        // G-03/G-01: the init-time process-env clears route through
        // mirror_env_remove — journaled to this executor's lineage
        // (the journal guard above is still held) and serialized
        // against sibling sessions' mirror writes.
        mirror_env_remove("__RUBASH_SUBSHELL_DEPTH");
        // Rubash#234: a respawned `rubash -c` child is GNU's forked child —
        // it inherits the parent's job_control state and never re-runs
        // initialize_job_control. The env marker is stripped below, so
        // record it in a process global for main.rs's startup job-control
        // gate (which must not print the terminal-grab diagnostics here).
        if internal_respawn {
            RESPAWNED_CHILD.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if !internal_respawn {
            env_vars.remove("__RUBASH_SCRIPT_NAME");
            // rubash#2259: the script's current-line frame is the same
            // diagnostic transport as the script name — a validated
            // respawn child keeps the frame its spawner stamped, any
            // other entry must not execute under a foreign parent's
            // line number.
            env_vars.remove("__RUBASH_CURRENT_LINE");
            // dollar_vars[0] slot reset at script entry (shell.c:1613) —
            // see main.rs; the snapshot rides the process env otherwise.
            env_vars.remove("__RUBASH_ARGV0_AFTER_UNSET");
        }
        env_vars.remove("__RUBASH_SHELL_NAME");
        env_vars.remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
        mirror_env_remove("__RUBASH_CURRENT_FUNCTION");
        mirror_env_remove("__RUBASH_IN_SOURCE");
        mirror_env_remove(super::types::FUNCTION_STDIN);
        mirror_env_remove(super::types::FUNCTION_STDIN_OFFSET);
        mirror_env_remove("__RUBASH_COMSUB_BODY");
        mirror_env_remove("__RUBASH_DEBUGGER_MODE");
        mirror_env_remove("__RUBASH_DEBUGGER_ATTEMPTED");
        // rubash#2259: the invocation-classification family's mirrors
        // leave with the table copies stripped above.
        mirror_env_remove("__RUBASH_INTERACTIVE");
        mirror_env_remove("__RUBASH_INTERACTIVE_FLAG_OFF");
        mirror_env_remove("__RUBASH_IS_C");
        mirror_env_remove("__RUBASH_LOGIN_SHELL");
        // Only the posix pair leaves the process-env mirror (rubash#2137
        // F9): the non-posix SETOPT keys stay as the diagnostic surface
        // they were, and the niu host's pretty-print read of
        // __RUBASH_POSIX_MODE consults a live shell's own mirror.
        mirror_env_remove("__RUBASH_POSIX_MODE");
        mirror_env_remove("__RUBASH_SETOPT_posix");
        mirror_env_remove(RESPAWN_SPAWNER_PID_KEY);
        mirror_env_remove("__RUBASH_COPROC_CHILD");
        if !internal_respawn {
            mirror_env_remove("__RUBASH_SCRIPT_NAME");
            mirror_env_remove("__RUBASH_CURRENT_LINE");
            mirror_env_remove("__RUBASH_ARGV0_AFTER_UNSET");
        }
        mirror_env_remove("__RUBASH_SHELL_NAME");
        mirror_env_remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
        // Birth context: the session cwd field (set in the ShellState
        // literal below) starts at the process cwd.
        Self::initialize_fresh_shell_env_vars(
            &mut env_vars,
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            internal_respawn,
            process_exit,
        );

        // rubash#2136: the stamped pid is adopted as `$$` only for a
        // validated respawn (GNU fork semantics: a forked child's `$$`
        // stays the main shell's pid, variables.c). An ambient copy at a
        // top-level entry named a process that may not even exist; the
        // shell's own pid is the only honest default there.
        let shell_pid = if internal_respawn {
            env_vars
                .get("__RUBASH_SHELL_PID")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or_else(std::process::id)
        } else {
            std::process::id()
        };
        env_vars.remove("__RUBASH_SHELL_PID");
        mirror_env_remove("__RUBASH_SHELL_PID");

        // rubash#1095: an async-wrapper spawn (compound_exec.rs) already is
        // GNU's single async/subshell fork (execute_cmd.c:701), so the
        // child seeds its subshell depth from the parent and records which
        // level its own pid answers for $BASHPID (variables.c:1511
        // get_bashpid = getpid). Ordinary shells see neither key and keep
        // depth 0 / origin 0. Stripped like __RUBASH_SHELL_PID so they
        // never surface as shell variables.
        let bg_depth = if internal_respawn {
            env_vars
                .get("__RUBASH_BG_DEPTH")
                .and_then(|value| value.parse::<usize>().ok())
        } else {
            None
        };
        env_vars.remove("__RUBASH_BG_DEPTH");
        mirror_env_remove("__RUBASH_BG_DEPTH");
        let bg_bashpid_base = if internal_respawn {
            env_vars
                .get("__RUBASH_BG_BASHPID_BASE")
                .and_then(|value| value.parse::<usize>().ok())
        } else {
            None
        };
        env_vars.remove("__RUBASH_BG_BASHPID_BASE");
        mirror_env_remove("__RUBASH_BG_BASHPID_BASE");

        // rubash#2032: every fork-shaped respawn (async/subshell wrapper,
        // coproc, both process-substitution forms) stamps the parent's
        // live positional window under POSITIONAL_PARAMS_TRANSPORT — a
        // GNU fork carries dollar_vars verbatim (variables.c; the fork
        // sites are execute_cmd.c:1576 execute_in_subshell and
        // subst.c:6362 process_substitute), but a respawned `rubash -c`
        // child re-imports only its process environment, so without the
        // stamp the body saw $# = 0. Consume the stamp only for internal
        // respawns; strip it either way so it never surfaces as a shell
        // variable. The seed lands in the ShellState literal below, and
        // ShellInvocation::apply_to_executor_before_startup keeps it
        // when the invocation carries no parameters of its own (the
        // respawn argv is just `-c source`).
        let respawn_positional_params: Vec<String> =
            match env_vars.get(super::compound_exec::POSITIONAL_PARAMS_TRANSPORT) {
                Some(value) if internal_respawn => array_values(value),
                _ => Vec::new(),
            };
        env_vars.remove(super::compound_exec::POSITIONAL_PARAMS_TRANSPORT);
        mirror_env_remove(super::compound_exec::POSITIONAL_PARAMS_TRANSPORT);

        // rubash#2058: the fork-context stamp (see
        // Executor::fork_context_transport_entries) — the parent's
        // effective call-stack views, its PIPESTATUS cell, and the
        // BASH_ARGV bottom-frame state. A GNU fork copies all of it
        // verbatim (variables.c); the respawn child re-imports only
        // its environment, so the stamp is consumed here for internal
        // respawns and stripped either way, exactly like
        // POSITIONAL_PARAMS_TRANSPORT above. The frames land in
        // ShellState::inherited_fork_frames (the dynamic views read
        // the child's own frames above them), the bottom frame seeds
        // bash_arg_bottom when the parent's arrays had materialized,
        // and the PIPESTATUS cell seeds the literal's pipestatus
        // fields.
        let mut respawn_inherited_frames: Option<crate::shell::state::InheritedForkFrames> = None;
        let mut respawn_function_line_spans: std::collections::HashMap<
            String,
            Vec<crate::parser::CommandLineSpan>,
        > = std::collections::HashMap::new();
        let mut respawn_argv_bottom: Option<Vec<String>> = None;
        let mut respawn_pipestatus: Option<(
            crate::shell::state::PipestatusShape,
            Vec<(i64, i32)>,
        )> = None;
        if internal_respawn && env_vars.contains_key(super::compound_exec::FORK_FUNCNAME_TRANSPORT)
        {
            let stamped_list = |key: &str| -> Vec<String> {
                env_vars
                    .get(key)
                    .map(|value| array_values(value))
                    .unwrap_or_default()
            };
            respawn_inherited_frames = Some(crate::shell::state::InheritedForkFrames {
                function_names: stamped_list(super::compound_exec::FORK_FUNCNAME_TRANSPORT),
                sources: stamped_list(super::compound_exec::FORK_SOURCE_TRANSPORT),
                linenos: stamped_list(super::compound_exec::FORK_LINENO_TRANSPORT),
                argv_frames: stamped_list(super::compound_exec::FORK_ARGV_TRANSPORT),
                argc_frames: stamped_list(super::compound_exec::FORK_ARGC_TRANSPORT),
                in_function: env_vars
                    .contains_key(super::compound_exec::FORK_IN_FUNCTION_TRANSPORT),
                has_main_frame: env_vars
                    .contains_key(super::compound_exec::FORK_MAIN_FRAME_TRANSPORT),
            });
            // Key presence is the materialization bit (see the
            // FORK_ARGV_BOTTOM_TRANSPORT doc): present-but-empty is a
            // materialized empty frame, absent is unmaterialized.
            if let Some(value) = env_vars.get(super::compound_exec::FORK_ARGV_BOTTOM_TRANSPORT) {
                respawn_argv_bottom = Some(array_values(value));
            }
            if let Some(value) = env_vars.get(super::compound_exec::FORK_PIPESTATUS_TRANSPORT) {
                respawn_pipestatus = crate::shell::state::decode_pipestatus_state(value);
            }
            // rubash#2202: per-function source-line spans (see
            // FORK_FN_SPANS_TRANSPORT); define_function re-seeds the
            // prelude's re-parsed bodies from these at definition
            // time and consumes each entry.
            if let Some(value) = env_vars.get(super::compound_exec::FORK_FN_SPANS_TRANSPORT) {
                respawn_function_line_spans =
                    super::compound_exec::decode_function_line_spans(value);
            }
        }
        // rubash#2202: function bodies that crossed as BASH_FUNC
        // entries — the coproc and process-substitution producer
        // spawns transport the function table that way, not as
        // prelude text — get the same span re-seed at birth. The
        // entry is consumed here (not left for define_function): the
        // imported body is the definition this shell holds, and a
        // definition executed later must keep its own lines.
        if !respawn_function_line_spans.is_empty() {
            let names: Vec<String> = imported_functions.keys().cloned().collect();
            for name in names {
                if let Some(spans) = respawn_function_line_spans.remove(&name) {
                    if let Some(body) = imported_functions.get_mut(&name) {
                        let mut commands = body.commands.clone();
                        if crate::parser::apply_command_line_spans(&mut commands, &spans) {
                            *body = std::rc::Rc::new(crate::parser::Ast { commands });
                        }
                    }
                }
            }
        }
        for key in super::compound_exec::FORK_TRANSPORT_KEYS {
            env_vars.remove(key);
            mirror_env_remove(key);
        }

        // Forked-child fd table restore: a `rubash -c` background/subshell
        // child inherits the parent's fd>=3 File endpoints through the
        // PROC_THREAD_ATTRIBUTE_HANDLE_LIST whitelist; the inherited handle
        // values arrive in __RUBASH_FD_* env keys (see
        // execute_background_ast_command). GNU execute_cmd.c
        // execute_in_subshell: fork copies the whole fd table. Collect and
        // strip the keys here so they never surface as shell variables.
        let mut inherited_fd_handles: BTreeMap<
            u32,
            (
                Option<crate::fd::HANDLE>,
                Option<crate::fd::HANDLE>,
                Option<String>,
            ),
        > = BTreeMap::new();
        env_vars.retain(|key, value| {
            let (is_write, num) = if let Some(num) = key.strip_prefix("__RUBASH_FD_HANDLE_") {
                (false, num)
            } else if let Some(num) = key.strip_prefix("__RUBASH_FD_WHANDLE_") {
                (true, num)
            } else if let Some(num) = key.strip_prefix("__RUBASH_FD_PATH_") {
                if let Ok(fd) = num.parse::<u32>() {
                    inherited_fd_handles.entry(fd).or_default().2 = Some(value.clone());
                }
                mirror_env_remove(key);
                return false;
            } else if let Some(num) = key.strip_prefix("__RUBASH_FD_WPATH_") {
                mirror_env_remove(key);
                return false;
            } else {
                return true;
            };
            if let Ok(fd) = num.parse::<u32>() {
                let handle =
                    crate::fd::HANDLE::from_str_radix(value.trim_start_matches("0x"), 16).ok();
                let entry = inherited_fd_handles.entry(fd).or_default();
                if is_write {
                    entry.1 = handle;
                } else {
                    entry.0 = handle;
                }
            }
            mirror_env_remove(key);
            false
        });
        let owns_signal_mailbox = if coproc_child_claimed && internal_respawn {
            // A blocked coprocess reader cannot consume a queued TERM,
            // so the child never owns the mailbox registration. The
            // startup signal-policy capture is a separate step and
            // must still run: on Unix the registration call is the
            // only other trigger for capture_entry_and_policy, and
            // skipping it left the exec image's runtime SIG_IGN for
            // SIGPIPE in place — the child's writes to a dead coproc
            // pipe then surfaced as reported-and-continued io errors
            // where GNU's forked child dies silently on the signal
            // (rubash#1882; the #1333 disposition reset is part of
            // the capture).
            crate::builtins::kill::capture_entry_policy_without_mailbox();
            false
        } else {
            // startup21: registration moved off the critical path —
            // create_dir_all + marker write cost ~0.5ms per spawn and
            // only cross-process kill consults the marker. Drop joins
            // the registration thread before unregistering, so the
            // marker never outlives a normally exiting shell.
            crate::builtins::kill::register_signal_mailbox_async(std::process::id());
            true
        };

        let mut executor = Self {
            shell_state: ShellState {
                variables: VariableStore::from_environment(&env_vars),
                env_vars,
                // G-02: a root session is born in the process cwd (the
                // host's directory at Executor::new, exactly what GNU's
                // startup getcwd reports); from here on the field, not
                // the process, carries the session's cwd.
                cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                array_compact_budget: HashMap::new(),
                aliases: HashMap::new(),
                functions: imported_functions,
                function_definition_redirects: HashMap::new(),
                function_def_infos: imported_function_def_infos,
                function_definition_locations: HashMap::new(),
                // rubash#2032: a fork-shaped respawn's transported
                // positional window (consumed above); ordinary shells
                // start empty and get their parameters from the
                // invocation (ShellInvocation::apply_to_executor_before_startup).
                positional_params: respawn_positional_params,
                positional_offset: 0,
                // GNU: PIPESTATUS starts unset (empty); it materializes on
                // the first pipeline/simple-command execution (rubash#1364).
                // A fork-shaped respawn instead inherits the parent's
                // fork-time cell verbatim (rubash#2058).
                pipestatus: respawn_pipestatus
                    .as_ref()
                    .map(|(_, vector)| vector.clone())
                    .unwrap_or_default(),
                pipestatus_shape: respawn_pipestatus
                    .as_ref()
                    .map(|(shape, _)| *shape)
                    .unwrap_or(crate::shell::state::PipestatusShape::Absent),
                function_name_stack: Vec::new(),
                bash_argc_stack: Vec::new(),
                bash_argv_stack: Vec::new(),
                // rubash#2058: a materialized bottom frame crosses the
                // respawn verbatim; None keeps the arrays unmaterialized
                // exactly as the parent's fork left them.
                bash_arg_bottom: std::cell::RefCell::new(respawn_argv_bottom),
                bash_lineno_stack: Vec::new(),
                bash_source_stack: Vec::new(),
                inherited_fork_frames: respawn_inherited_frames,
                respawn_function_line_spans,
                local_var_scopes: Vec::new(),
                local_attr_scopes: Vec::new(),
                local_typed_scopes: Vec::new(),
                expanding_aliases: Vec::new(),
                loop_depth: 0,
                inherited_loop_depth: 0,
                read_generation: 0,
                read_generation_ceiling: 0,
                command_number: 1,
                function_depth: 0,
                dollar_vars_changed_by_set: false,
                last_dispatch_was_eval_or_source: false,
                random_state: RandomGen::seeded(),
                // rubash#1095's __RUBASH_BG_DEPTH is the authoritative seed
                // for async `&` children (it deliberately under-seeds the
                // level when the serialized command still carries a paren
                // node that bumps in-process); __RUBASH_SUBSHELL_DEPTH
                // (rubash#1074) covers coproc/other respawns where BG_DEPTH
                // is absent.
                subshell_depth: Cell::new(bg_depth.unwrap_or(respawned_subshell_depth)),
                subshell_source_text: None,
                in_command_substitution: Cell::new(false),
                stdin_redir: Cell::new(false),
                xtrace_indirection_level: Cell::new(0),
                xtrace_suspended: Cell::new(0),
                job_table: crate::jobs::table::JobTable::default(),
                last_background_pid: None,
                coproc_names: HashMap::new(),
                completion_specs: crate::builtins::complete::CompletionRegistry::new(),
                bind_registry: std::rc::Rc::new(std::cell::RefCell::new(
                    crate::shell::bind_registry::BindRegistry::default(),
                )),
                session_history: None,
                interactive_hist_engine: crate::history_expand::HistEngineState::default(),
                arithmetic_expansion_error: Cell::new(false),
                arithmetic_nonfatal_error: Cell::new(false),
                arithmetic_fatal_error: Cell::new(false),
                arithmetic_nounset_error: Cell::new(false),
                arithmetic_readonly_error: Cell::new(false),
                conditional_nounset_error: Cell::new(false),
                arithmetic_last_error_category: Cell::new(None),
                parameter_bad_substitution: Cell::new(false),
                redirect_failglob_error: Cell::new(false),
                redirect_failglob_seen: Cell::new(false),
                debug_trap_command: std::cell::RefCell::new(None),
                xtrace_fd: Cell::new(-1),
                xtrace_fd_source: std::cell::RefCell::new(String::new()),
                procsub_streams: std::cell::RefCell::new(HashMap::new()),
                procsub_adhoc_carriers: std::cell::RefCell::new(Vec::new()),
                fork_saved_process_umask: None,
            },
            fd_table: FdTable::new(),
            exit_code: 0,
            parse_error_occurred: false,
            signal_discard_pending: false,
            termsig_death_pending: None,
            bash_logout_sourced: false,
            shell_pid,
            bashpid_origin_depth: bg_bashpid_base.unwrap_or(0),
            owns_signal_mailbox,
            is_process_exit_executor: process_exit,
            background_children: HashMap::new(),
            procsub_feed_handles: Vec::new(),
            procsub_stream_children: HashMap::new(),
            pending_job_completions: Vec::new(),
            pending_coproc_retirements: Vec::new(),
            pending_timed_coproc_reports: Vec::new(),
            timed_coproc_death_instants: std::collections::HashMap::new(),
            reap_prev_mark: Cell::new((0, None, 0)),
            execute_generation: Cell::new(0),
            foreground_wait_count: Cell::new(0),
            coproc_stderr_forwarders: HashMap::new(),
            coproc_displaced_stdout_writers: HashMap::new(),
            assignment_output_process_substitutions: Vec::new(),
            assignment_input_process_substitution_files: Vec::new(),
            defer_locale_assignment_warnings: Cell::new(false),
            deferred_locale_assignment_binds: Vec::new(),
            external_dispatch_ran: Cell::new(false),
            dispatch_net_bound_fds: Vec::new(),
            pipestatus_write_absorbed: Cell::new(false),
            tempenv_pipestatus_state: None,
            command_scoped_procsub_input_fds: Vec::new(),
            conditional_output_procsubs: Vec::new(),
            current_conditional_redirects: None,
            pending_scalar_assignment: false,
            suppress_errexit: 0,
            errexit_ignore_frame: 0,
            parse_and_execute_frames: Vec::new(),
            command_builtin_depth: 0,
            builtin_wrapper_depth: 0,
            standard_path_scope: None,
            debug_trap_running: false,
            debug_trap_action_cache: std::cell::RefCell::new(None),
            debug_driver: None,
            debug_syntax_unit_file: None,
            line_env_os_value: std::cell::RefCell::new(None),
            pre_stamped_line: std::cell::Cell::new(false),
            expansion_probe_active: std::cell::Cell::new(false),
            return_trap_running: false,
            signal_trap_running: false,
            error_trap_running: false,
            subshell_fork_collapse: std::cell::Cell::new(false),
            trap_saved_exit_value: None,
            trap_saved_exit_context: 0,
            exec_replaced: false,
            subshell_signal_death: None,
            fg_sigint_unwind: false,
            // rubash#1175: a process root IS its process's shell; the
            // image-replacement gates live at the exec dispatcher.
            owns_process_image: true,
            source_nest_depth: 0,
            sigchld_notifications_pending: std::cell::Cell::new(0),
            pending_signal_notices: std::cell::RefCell::new(Vec::new()),
            host_internal_depth: std::cell::Cell::new(0),
            debug_trap_function_line: None,
            arithmetic_last_error_expression: std::cell::RefCell::new(String::new()),
            arithmetic_last_eval_input: std::cell::RefCell::new(String::new()),
            assignment_command_name: None,
            buffer_assignment_diagnostics: false,
            getopts_optind_arith_fatal: false,
            defer_stage_time_report: false,
            pending_stage_time_report: None,
            defer_list_time_report: false,
            pending_list_time_reports: Vec::new(),
            pending_assignment_diagnostics: Vec::new(),
            parameter_assignment_failure: Cell::new(false),
            parameter_assignment_failure_prev_status: Cell::new(None),
            no_closing_brace_failure: Cell::new(false),
            tempenv_names: Vec::new(),
            tempenv_marks: Vec::new(),
            tempenv_promoted_names: Vec::new(),
            tempenv_previous: HashMap::new(),
            tempenv_propagated_names: Vec::new(),
            function_tempenv_names: Vec::new(),
            evalerror_pending: Cell::new(false),
            nounset_force_eof: Cell::new(false),
            conditional_expansion_fatal: Cell::new(false),
            conditional_fatal_status: Cell::new(None),
            conditional_expansion_error_status: Cell::new(None),
            conditional_procsub_span_targets: Vec::new(),
            evalerror_subscript_pending: Cell::new(false),
            evalerror_failglob_pending: Cell::new(false),
            evalerror_arith_pending: Cell::new(false),
            evalerror_exit_toomany_pending: Cell::new(false),
            eval_list_depth: Cell::new(None),
            evalerror_line: Cell::new(None),
            command_expansion_failed: Cell::new(false),
            err_trap_fired_at_dispatch: Cell::new(false),
            err_trap_armed_sample: Cell::new(None),
            evalerror_exec_depth: Cell::new(0),
            reader_command_line: Cell::new(None),
            break_failure_status: Cell::new(None),
            procsub_carrier_memo: std::cell::RefCell::new(HashMap::new()),
            reader_last_list_line: Cell::new(None),
            command_string_mode: Cell::new(false),
            command_string_final_pending: Cell::new(false),
            exec_replace_final_frame: Cell::new(false),
            exec_replace_dispatch_stamp: Cell::new(0),
            exec_replace_stamp_counter: Cell::new(0),
            exec_replace_stamp_hold: Cell::new(0),
            exec_replace_shlvl_snapshot: Cell::new(None),
            line_lex_locales: std::cell::RefCell::new(HashMap::new()),
            unit_lex_locale: std::cell::RefCell::new(None),
            ambient_line: Cell::new(None),
            inside_compound_condition: Cell::new(false),
            conditional_invert_pending: Cell::new(false),
            inside_assignment_rhs: Cell::new(false),
            last_command_substitution_status: Cell::new(None),
            word_expansion_comsub_exit: Cell::new(None),
            comsub_stdin_writeback: Cell::new(None),
            pipeline_stdin_consumed: Cell::new(None),
            pipeline_stage_fds_pre_wired: std::cell::Cell::new(false),
            stage_dup_channel_payloads: std::cell::RefCell::new(Vec::new()),
            pipeline_final_stage: std::cell::Cell::new(false),
            pipeline_stage_pre_traced: std::cell::Cell::new(false),
            pipeline_stage_restricted_refused: std::cell::Cell::new(false),
            last_heredoc_warning_source: RefCell::new(None),
            heredoc_admission_declined_source: RefCell::new(None),
            comsub_leading_newlines: Cell::new(0),
            current_shell_substitution_exit: Cell::new(None),
            last_command_substitution_parse_error: Cell::new(false),
            last_command_substitution_parse_error_backtick: Cell::new(false),
            backtick_body_diagnostic: Cell::new(false),
            parser_error_errexited: Cell::new(false),
            last_command_inverted: Cell::new(false),
            compound_redirect_failed: Cell::new(false),
            noclobber_prepared_targets: std::cell::RefCell::new(std::collections::HashSet::new()),
            exit_jump_pending: Cell::new(false),
            errexit_status_converted: Cell::new(false),
            posix_special_status_converted: Cell::new(false),
            exec_do_redirections_failed: Cell::new(false),
            timed_dispatch_depth: Cell::new(0),
            special_builtin_failed: Cell::new(false),
            last_builtin_write_failed: Cell::new(false),
            alias_introduced_words: Cell::new(0),
            redirect_target_memo: RefCell::new(HashMap::new()),
            redirect_target_single_glob_match: RefCell::new(std::collections::HashSet::new()),
            assignment_expansion_memo: RefCell::new(HashMap::new()),
            alias_select_scan_memo: RefCell::new(None),
            fd_var_external_undo: Vec::new(),
            fd_var_deferred_closes: Vec::new(),
            fd_var_pending_closes: Vec::new(),
            fd_var_close_generation: 0,
            fd_var_pending_close_generation: 0,
            fd_var_close_restore: Vec::new(),
            deferred_varredir_closes: Vec::new(),
            read_deadline: None,
            read_timed_out: false,
            read_interrupted_signal: None,
            read_sigint_guard_armed: false,
            read_eof_no_delimiter: false,
            read_process_stdin_eof: false,
            timed_stdin: None,
            stdout_capture: None,
            stderr_capture: None,
            pipeline_stdout_stream: None,
            pipeline_stdin_stream: None,
            external_stdio_outcome: None,
            external_stdio_piped_fallback: false,
            host_external_command_handler: None,
            external_route_provider: None,
            external_stdio_slots: None,
            own_input_bindings: std::collections::HashMap::new(),
            pending_stdin_fold: None,
            #[cfg(windows)]
            elevation_handler: None,
            external_file_builtins_enabled: true,
            process_env_snapshot,
            env_write_journal,
            history_provider: None,
            provider_stamp_ledger: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        };
        for (fd, (read_h, write_h, path)) in inherited_fd_handles {
            let path = PathBuf::from(path.unwrap_or_default());
            let read_rc = read_h.map(|handle| {
                Rc::new(FileFd {
                    handle,
                    path: path.clone(),
                })
            });
            let write_rc = match write_h {
                Some(handle) => match &read_rc {
                    // Same handle value: parent's read/write endpoints
                    // shared one open file description — keep it shared.
                    Some(rc) if rc.handle == handle => Some(rc.clone()),
                    _ => Some(Rc::new(FileFd {
                        handle,
                        path: path.clone(),
                    })),
                },
                None => None,
            };
            if let Some(rc) = read_rc {
                executor
                    .fd_table
                    .open_input(fd, FdReadEndpoint::File(rc), false);
            }
            if let Some(rc) = write_rc {
                executor
                    .fd_table
                    .open_output(fd, FdWriteEndpoint::File(rc), false);
            }
        }
        // rubash#1557: adopt whatever descriptors the process itself
        // inherited open (CI log channels, socket activation, a parent
        // shell's `3>file`) that the respawn channel did not claim.
        executor
            .fd_table
            .register_inherited_process_fds(&inherited_fds_snapshot);
        executor
    }

    /// Finalizes an env_vars map into a fresh shell's variable environment:
    /// managed variables, attribute marks, option replay, and id/host
    /// defaults. GNU reference: variables.c:511-526
    /// (initialize_shell_variables) plus shell.c:1974-1986
    /// (initialize_shell_options SHELLOPTS/BASHOPTS replay).
    ///
    /// Shared by `Executor::new` (real process start) and the in-process
    /// `${THIS_SH} ./x.sub` child path (executor/external_finish.rs
    /// execute_direct_shell_script), so in-process children get the same
    /// managed variables and attribute marks a spawned rubash.exe would.
    /// Callers must provide env_vars that already contains only the env a
    /// fresh shell inherits (exported names, BASH_FUNC_* payloads, and the
    /// required Windows host vars).
    pub(in crate::executor) fn initialize_fresh_shell_env_vars(
        env_vars: &mut crate::shell::var_table::VarTable,
        cwd: &std::path::Path,
        // rubash#2058: true for a validated internal respawn — a fork
        // is not a new shell, so SHLVL keeps the parent's value
        // instead of taking the startup increment.
        keep_shlvl: bool,
        // rubash#2212: true only for the standalone shell process's
        // own birth (Executor::new_process_exit via main) — its
        // environment import follows GNU wholesale (see
        // env_helpers::is_process_startup_export_candidate).
        // In-process children and embedded executors keep the
        // anti-leak heuristic: their "environment" is a parent
        // session's serialization or a shared host process env.
        process_startup: bool,
    ) {
        env_vars.remove("BASH_ARGV0");
        env_vars.remove("BASH_EXECUTION_STRING");
        // GNU Bash derives PWD from the real working directory at startup
        // (variables.c initialize_shell_variables): it only trusts an
        // inherited PWD that still matches the current directory, and
        // otherwise falls back to getcwd(). WSL interop forwards the Windows
        // session's PWD (e.g. D:/repo/rubash) rather than the WSL shell's
        // actual CWD; trusting it desynchronized cd/OLDPWD, so `cd $OLDPWD`
        // (assoc.tests) landed in the wrong directory and subsequent
        // `./assocN.sub` lookups failed. Always align PWD with the real CWD.
        // G-02: the "real CWD" is the caller's session cwd field, passed in
        // (at process birth it is the process cwd; for an in-process fresh
        // shell it is the parent session's cwd).
        let pwd = shell_pwd_display(&cwd.to_string_lossy().replace('\\', "/"));
        env_vars.insert("PWD".to_string(), pwd);
        // Windows-only: suites write $TMPDIR/$HOME into generated scripts
        // unquoted (posix2.tests conftest2: `$TMPDIR/conftest2 "$@"`; exp.tests
        // `${x#$HOME}`), and an inherited Windows backslash path is escape
        // syntax to the shell reader (`D:\repo` corrupts to `D:repo`). Windows
        // filesystem APIs accept forward slashes, so normalize to the
        // shell-safe form. Unix never rewrites inherited values: `\` is a
        // legal filename byte there and GNU bash imports the environment
        // verbatim (variables.c initialize_shell_variables) — a HOME of
        // /home/a\b must survive as-is.
        #[cfg(windows)]
        for name in ["TMPDIR", "HOME"] {
            if let Some(value) = env_vars.get_mut(name) {
                if value.contains('\\') {
                    *value = value.replace('\\', "/");
                }
            }
        }
        // Whether SHELL/PATH arrived with the environment: the fabricated
        // fallbacks below are bound WITHOUT att_exported in GNU (a
        // passwd-derived SHELL and the set_if_not PATH default are plain
        // shell variables; `env -i` export -p lists neither), so any name
        // fabricated here must be unmarked again after
        // mark_initial_exported_vars sweeps the table (rubash#1325).
        let shell_inherited = env_vars.contains_key("SHELL");
        env_vars
            .entry("SHELL".to_string())
            .or_insert_with(login_shell_value);
        // GNU variables.c:550 initialize_shell_variables:
        // set_if_not("PATH", DEFAULT_PATH_VALUE) with config-top.h:60-61
        // DEFAULT_PATH_VALUE "/usr/local/bin:/usr/local/sbin:/usr/bin:
        // /usr/sbin:/bin:/sbin:." — a shell started with no PATH in its
        // environment (env -i) still resolves the standard utilities; a
        // PATH that is present but empty is imported verbatim (rubash#1129).
        #[cfg(unix)]
        let path_inherited = env_vars.contains_key("PATH");
        #[cfg(unix)]
        env_vars.entry("PATH".to_string()).or_insert_with(|| {
            "/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin:.".to_string()
        });
        // Whether THIS_SH arrived with the environment: like the
        // SHELL/PATH fallbacks above, a value fabricated here is a
        // shell-internal convenience (the shell-under-test pointer for
        // ${THIS_SH} recursion), not a GNU shell variable — GNU has no
        // THIS_SH at all — so a fabricated one must NOT carry the export
        // mark into child environments (rubash#1806). An inherited
        // THIS_SH keeps its imported mark, as any imported variable does.
        let this_sh_inherited = env_vars.contains_key("THIS_SH");
        // GNU test scripts use ${THIS_SH} to invoke the shell under test
        // (e.g. `${THIS_SH} -c '...'`). Rubash is always the shell under
        // test, so the auto-detected executable path must OVERRIDE any
        // inherited value: the winuxsh wrapper exports THIS_SH pointing at
        // its own shim in the Windows environment, and WSL interop feeds
        // that Windows value to every rubash.exe spawned from the test
        // harness - keeping it would silently run test children under the
        // old shim instead of rubash (found via func.tests: func5 children
        // executed niu.exe semantics and truncated the family output).
        // Exception: a suite that copies THIS_SH to an `sh`-named file
        // (`cp ${THIS_SH} $TMPDIR/sh`; posixexp.tests) relies on the argv[0]
        // basename to select posix mode for child invocations. Keeping the
        // inherited name preserves that signal — the copy is still the
        // rubash binary, and a nonexistent target keeps the auto-detected
        // path so a stale winuxsh export cannot hijack children. The same
        // applies to a `bash`-named wrapper a harness installs so test
        // scripts see the GNU-conventional shell name in ${THIS_SH##*/}
        // (type.tests expects `bash`, not the product binary name).
        let inherited_sh = env_vars.get("THIS_SH").is_some_and(|value| {
            let basename = value.rsplit(['/', '\\']).next().unwrap_or(value.as_str());
            let stem = basename.strip_suffix(".exe").unwrap_or(basename);
            (stem.eq_ignore_ascii_case("sh") || stem.eq_ignore_ascii_case("bash"))
                && crate::executor::path::shell_path_to_windows(value, env_vars).is_file()
        });
        if !inherited_sh {
            env_vars.insert(
                "THIS_SH".to_string(),
                std::env::current_exe()
                    .map(|path| path.to_string_lossy().replace('\\', "/").to_string())
                    .unwrap_or_else(|_| "rubash".to_string()),
            );
        }
        // GNU variables.c:952-963 initialize_shell_variables: an imported
        // OLDPWD is kept only when it names a directory
        // (OLDPWD_CHECK_DIRECTORY, config-top.h:183); otherwise Bash binds a
        // dummy invisible variable, which is why `cd -` then reports
        // "OLDPWD not set". The directory check runs in the shell path
        // namespace, so a WSL-style /mnt/d/... value counts when its
        // Windows translation exists.
        let keep_oldpwd = env_vars.get("OLDPWD").is_some_and(|value| {
            !value.is_empty()
                && crate::executor::path::shell_path_to_windows(value, env_vars).is_dir()
        });
        if !keep_oldpwd {
            env_vars.remove("OLDPWD");
            // Word expansion falls back to the live process environment
            // when a name is absent from env_vars, so clear it there too or
            // `$OLDPWD` would still expand to the rejected value.
            mirror_env_remove("OLDPWD");
        }
        // rubash#2058: see the keep_shlvl parameter.
        if !keep_shlvl {
            initialize_shell_level(env_vars);
        }
        mark_initial_exported_vars(env_vars, process_startup);
        mark_env_name(env_vars, EXPORTED_VARS, "OLDPWD");
        // Undo the export mark for the SHELL/PATH fallbacks fabricated
        // above (see the note at the SHELL bind): GNU binds both without
        // att_exported, so `env -i` shells must not export them
        // (rubash#1325). Inherited values keep their imported mark.
        if !shell_inherited {
            unmark_env_name(env_vars, EXPORTED_VARS, "SHELL");
        }
        #[cfg(unix)]
        if !path_inherited {
            unmark_env_name(env_vars, EXPORTED_VARS, "PATH");
        }
        if !this_sh_inherited {
            unmark_env_name(env_vars, EXPORTED_VARS, "THIS_SH");
        }
        // GNU variables.c initialize_shell_variables only imports TMPDIR from
        // the environment — it never invents a value (WSL probe: `env -i
        // bash -c 'echo ${TMPDIR-UNSET}'` prints UNSET), and a value it never
        // had is never exported either. The injection below is the Windows
        // fixture: suites write unquoted `$TMPDIR/...` paths and /tmp
        // resolution needs a backing directory, so keep the SHELL variable —
        // but it MUST be inserted after mark_initial_exported_vars so the
        // injected default never enters the exported set (unixwin/niubash#162):
        // an exported POSIX-form `/c/...` TMPDIR reaches every foreign child
        // (Bun-compiled opencode resolves TMPDIR ahead of TEMP/TMP, treats the
        // slash-drive value as a drive-relative path, and its bunfs `B:`
        // virtual-drive extraction fails -> dlopen falls back to
        // `B:/~BUN/root/...` -> ERROR_MOD_NOT_FOUND 126). An inherited TMPDIR
        // still is: the or_insert no-ops and the import-time export marking
        // stands.
        // The /var/tmp backing directory is NOT created here: GNU touches
        // $TMPDIR only when a scratch file is actually needed (rubash#328 —
        // an eager `var/tmp` subtree below every TMPDIR, including the
        // caller's cwd, was visible to `ls`/globs of untouched directories;
        // the mapping in path.rs now materializes it lazily on first
        // /var/tmp use).
        #[cfg(windows)]
        {
            env_vars.entry("TMPDIR".to_string()).or_insert_with(|| {
                // rubash#331: the injected TMPDIR default is a value WE
                // produce, so it renders in POSIX form (/d/...) like $PWD
                // — the doc's FFmpeg-TMPDIR corollary and the form clash
                // with $PWD both die here. path.rs resolves /d/ paths for
                // /tmp mapping, so the internal consumers are unaffected.
                shell_pwd_display(&safe_temp_dir_string().replace('\\', "/"))
            });
        }
        // GNU variables.c initialize_shell_variables: IFS is never
        // imported from the environment — a fresh shell always binds it
        // to the default " \t\n", discarding any inherited value (even an
        // empty one, which would disable field splitting entirely). This
        // initializer runs exactly at fresh-shell starts (process start
        // and the in-process child path), never for state-cloned
        // subshells, so an unconditional bind is the GNU shape
        // (rubash#1334).
        env_vars.insert("IFS".to_string(), " \t\n".to_string());
        // GNU variables.c initialize_shell_variables binds these defaults
        // for every fresh shell (an inherited value wins): OPTERR=1
        // (getopts diagnostic default), PS4="+ " (xtrace prefix), and the
        // compiled-in BASH_LOADABLES_PATH search list (rubash#1075,
        // rubash#1076). They are ordinary (non-exported) variables, so
        // introspection (`${OPTERR-unset}`, `declare -p PS4`) sees them
        // exactly like GNU.
        env_vars
            .entry("OPTERR".to_string())
            .or_insert_with(|| "1".to_string());
        env_vars
            .entry("PS4".to_string())
            .or_insert_with(|| "+ ".to_string());
        env_vars
            .entry("BASH_LOADABLES_PATH".to_string())
            .or_insert_with(|| {
                "/usr/local/lib/bash:/usr/lib/bash:/opt/local/lib/bash:/usr/pkg/lib/bash:/opt/pkg/lib/bash:."
                    .to_string()
            });
        // GNU shell.c/variables.c: at startup $_ falls back to the
        // absolute pathname of the shell itself when the environment did
        // not carry one (the invoking parent normally supplies it, and
        // env -i script runs showed the binary path) — rubash#1076.
        env_vars
            .entry("_".to_string())
            .or_insert_with(bash_path_value);
        // GNU bashline.c initializes COMP_WORDBREAKS at startup from
        // rl_basic_word_break_characters, so the variable is always set
        // (declare -p COMP_WORDBREAKS succeeds in a fresh shell) even
        // non-interactively (rubash#1086).
        env_vars
            .entry("COMP_WORDBREAKS".to_string())
            .or_insert_with(|| " \t\n\"'@><=;|&(:".to_string());
        // GNU binds COMP_WORDBREAKS with att_invisible: the variable
        // exists (`declare -p COMP_WORDBREAKS` succeeds) but print_var_list
        // (the `set` listing) skips invisible variables (rubash#1325).
        mark_env_name(env_vars, INVISIBLE_VARS, "COMP_WORDBREAKS");
        // GNU shell.c: a shell whose environment carries no TERM binds
        // TERM="dumb" (plain, unexported) instead of leaving it unset
        // (rubash#1325).
        env_vars
            .entry("TERM".to_string())
            .or_insert_with(|| "dumb".to_string());
        env_vars.insert(
            SHELL_START_EPOCH.to_string(),
            current_epoch_seconds().to_string(),
        );
        // GNU shell.c:1974-1986 (shell_initialize -> initialize_shell_options
        // set.def:607-630 via parse_shellopts set.def:600-604, and
        // initialize_bashopts shopt.def): a non-privileged shell applies the
        // inherited $SHELLOPTS / $BASHOPTS ADDITIVELY - every colon-separated
        // name is turned on, while options absent from the value keep their
        // defaults. The rebuilds below overwrite the imported values, so the
        // inherited names must be applied first (invocation1.sub/2.sub child
        // inheritance).
        // GNU variables.c:5820 + 6257 (sv_strict_posix in the special-var
        // table, run by initialize_shell_variables): an IMPORTED
        // POSIXLY_CORRECT — present with ANY value, even empty — turns
        // posix mode on at startup (rubash#1054). This must run before
        // the SHELLOPTS/BASHOPTS snapshots below so both list posix and
        // the posix_vars walk results.
        if env_vars.contains_key("POSIXLY_CORRECT") {
            crate::builtins::set::set_shell_option(env_vars, "posix", true);
        }
        //
        // GNU variables.c:5760 { "BASH_COMPAT", sv_shcompat }: an inherited
        // BASH_COMPAT already synced the compatNN flags at import time, but
        // VarTable imports the process environment verbatim — derive the
        // flags here instead (before the BASHOPTS loop, which may itself
        // carry compatNN names additively like initialize_bashopts'
        // parse_bashopts). Silent: the range diagnostic belongs to bind
        // time, and this constructor is re-entered for subshells where a
        // repeat would spam; the level/flags are still exact.
        let _ = crate::builtins::shopt::sync_compat_flags_from_bash_compat(env_vars);
        if let Some(value) = env_vars.get("SHELLOPTS").cloned() {
            for name in value.split(':').filter(|name| !name.is_empty()) {
                if crate::builtins::set::is_shell_option(name) {
                    crate::builtins::set::set_shell_option(env_vars, name, true);
                } else {
                    // GNU shell.c initialize_shell_options -> set.def
                    // parse_shellopts: an unknown inherited name is
                    // reported once at startup (`<shell>: line 0: <name>:
                    // invalid option name`) and startup continues
                    // (rubash#1246). The value is rebuilt sanitized below,
                    // so re-entered constructors (subshells) never repeat
                    // the report.
                    let shell_name = std::env::args()
                        .next()
                        .filter(|arg| !arg.is_empty())
                        .unwrap_or_else(|| "bash".to_string());
                    eprintln!("{shell_name}: line 0: {name}: invalid option name");
                }
            }
        }
        if let Some(value) = env_vars.get("BASHOPTS").cloned() {
            for name in value.split(':').filter(|name| !name.is_empty()) {
                if crate::builtins::shopt::is_supported_option(name) {
                    let _ = crate::builtins::shopt::set_option(env_vars, name, true);
                }
            }
        }
        let shelopts = crate::builtins::set::shellopts_value(env_vars);
        env_vars.insert("SHELLOPTS".to_string(), shelopts);
        mark_env_name(env_vars, READONLY_VARS, "SHELLOPTS");
        let bashopts = crate::builtins::shopt::bashopts_value(env_vars);
        env_vars.insert("BASHOPTS".to_string(), bashopts);
        mark_env_name(env_vars, READONLY_VARS, "BASHOPTS");
        mark_env_name(env_vars, ARRAY_VARS, "PIPESTATUS");
        env_vars.insert("OPTIND".to_string(), "1".to_string());
        // GNU variables.c creates OPTIND with att_integer (it prints as
        // `declare -i OPTIND="1"`), which is also why a getopts name bind
        // into OPTIND arith-evaluates (rubash#1270).
        mark_env_name(env_vars, INTEGER_VARS, "OPTIND");
        env_vars.remove("OPTARG");
        env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        env_vars
            .entry("BASH_VERSION".to_string())
            .or_insert_with(bash_version_value);
        env_vars
            .entry("BASH".to_string())
            .or_insert_with(bash_path_value);
        store_indexed_array(env_vars, "BASH_VERSINFO", bash_versinfo_values());
        mark_env_name(env_vars, READONLY_VARS, "BASH_VERSINFO");
        store_indexed_array(env_vars, "BASH_ARGC", Vec::new());
        store_indexed_array(env_vars, "BASH_ARGV", Vec::new());
        store_indexed_array(env_vars, "BASH_LINENO", vec!["0".to_string()]);
        store_indexed_array(env_vars, "BASH_SOURCE", Vec::new());
        env_vars.insert("BASH_CMDS".to_string(), "()".to_string());
        mark_env_name(env_vars, ASSOC_VARS, "BASH_CMDS");
        env_vars.insert("BASH_ALIASES".to_string(), "()".to_string());
        mark_env_name(env_vars, ASSOC_VARS, "BASH_ALIASES");
        // The stored DIRSTACK cell starts as an empty indexed array, not
        // scalar-empty: a bare `declare -a` listing prints the last
        // materialized cell (variables.c get_dirstack runs on named access
        // only), and GNU shows `declare -a DIRSTACK=()` there.
        store_indexed_array(env_vars, "DIRSTACK", Vec::new());
        env_vars.insert("FUNCNAME".to_string(), String::new());
        mark_env_name(env_vars, ARRAY_VARS, "FUNCNAME");
        env_vars
            .entry("HOSTTYPE".to_string())
            .or_insert_with(hosttype_value);
        env_vars
            .entry("HOSTNAME".to_string())
            .or_insert_with(hostname_value);
        env_vars
            .entry("OSTYPE".to_string())
            .or_insert_with(ostype_value);
        env_vars
            .entry("MACHTYPE".to_string())
            .or_insert_with(machtype_value);
        env_vars.insert("UID".to_string(), uid_value());
        env_vars.insert("EUID".to_string(), euid_value());
        env_vars.insert("PPID".to_string(), ppid_value());
        mark_env_name(env_vars, READONLY_VARS, "UID");
        mark_env_name(env_vars, READONLY_VARS, "EUID");
        mark_env_name(env_vars, READONLY_VARS, "PPID");
        // GNU variables.c create_variable_tables/initialize_shell_variables:
        // UID/EUID/PPID are created with V_INTEGER|V_READONLY, so
        // `declare -p UID` reports -ir, not just -r (rubash#1326, same root
        // as #1184 finding 3 — both clusters landed this fix; kept once).
        mark_env_name(env_vars, INTEGER_VARS, "UID");
        mark_env_name(env_vars, INTEGER_VARS, "EUID");
        mark_env_name(env_vars, INTEGER_VARS, "PPID");
    }
}
