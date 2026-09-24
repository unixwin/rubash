use super::*;
use crate::shell::VariableStore;

fn shell_pwd_display(path: &str) -> String {
    #[cfg(windows)]
    {
        if crate::executor::path::shell_path_style_enabled() {
            return path.to_string();
        }
        let bytes = path.as_bytes();
        if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' {
            let drive = (bytes[0] as char).to_ascii_lowercase();
            return format!("/{drive}/{}", &path[3..]);
        }
    }
    path.to_string()
}

impl Executor {
    pub fn new() -> Self {
        let process_env_snapshot: HashMap<String, String> = std::env::vars().collect();
        let mut env_vars = process_env_snapshot.clone();
        // On Windows, std::env::vars() returns PATH as "Path" (capital P).
        // Every rubash command-lookup site reads env_vars.get("PATH") (all caps),
        // which is a case-sensitive HashMap lookup — it misses on Windows.
        // Mirror the value into the all-caps key so external command lookup works.
        #[cfg(windows)]
        if let Some(path_val) = env_vars.remove("Path") {
            env_vars.entry("PATH".to_string()).or_insert(path_val);
        }

        // Pin the host's POSIX toolset directory before scripts can
        // overwrite PATH: `PATH=/bin:/usr/bin` (invocation.tests) and
        // `command -p` (command.def _CS_PATH) both need the logical bin
        // namespace to keep resolving. env_var writes sync into the process
        // environment, so a runtime probe of std::env PATH is polluted.
        #[cfg(windows)]
        if !env_vars.contains_key("__RUBASH_POSIX_TOOLS_DIR") {
            if let Some(dir) = crate::executor::path::windows_posix_tools_dir(&env_vars) {
                env_vars.insert(
                    "__RUBASH_POSIX_TOOLS_DIR".to_string(),
                    dir.to_string_lossy().to_string(),
                );
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

        let (imported_functions, imported_function_def_infos) =
            import_exported_functions_from_env(&env_vars);
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
        let internal_respawn = env_vars.get("__RUBASH_COPROC_CHILD").map(String::as_str) == Some("1")
            || env_vars.contains_key("__RUBASH_SHELL_PID");
        if !internal_respawn {
            env_vars.remove("__RUBASH_SCRIPT_NAME");
        }
        env_vars.remove("__RUBASH_SHELL_NAME");
        env_vars.remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
        env::remove_var("__RUBASH_CURRENT_FUNCTION");
        env::remove_var("__RUBASH_IN_SOURCE");
        if !internal_respawn {
            env::remove_var("__RUBASH_SCRIPT_NAME");
        }
        env::remove_var("__RUBASH_SHELL_NAME");
        env::remove_var(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
        Self::initialize_fresh_shell_env_vars(&mut env_vars);

        let shell_pid = env_vars
            .get("__RUBASH_SHELL_PID")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_else(std::process::id);
        env_vars.remove("__RUBASH_SHELL_PID");

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
                std::env::remove_var(key);
                return false;
            } else if let Some(num) = key.strip_prefix("__RUBASH_FD_WPATH_") {
                std::env::remove_var(key);
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
            std::env::remove_var(key);
            false
        });
        let owns_signal_mailbox =
            if env_vars.get("__RUBASH_COPROC_CHILD").map(String::as_str) == Some("1") {
                // A blocked coprocess reader cannot consume a queued TERM.
                false
            } else {
                crate::builtins::kill::register_signal_mailbox(std::process::id()).is_ok()
            };

        let mut executor = Self {
            shell_state: ShellState {
                variables: VariableStore::from_environment(&env_vars),
                env_vars,
                aliases: HashMap::new(),
                functions: imported_functions,
                function_definition_redirects: HashMap::new(),
                function_def_infos: imported_function_def_infos,
                function_definition_locations: HashMap::new(),
                positional_params: Vec::new(),
                pipestatus: vec![0],
                function_name_stack: Vec::new(),
                bash_argc_stack: Vec::new(),
                bash_argv_stack: Vec::new(),
                bash_lineno_stack: Vec::new(),
                bash_source_stack: Vec::new(),
                local_var_scopes: Vec::new(),
                local_attr_scopes: Vec::new(),
                local_typed_scopes: Vec::new(),
                expanding_aliases: Vec::new(),
                loop_depth: 0,
                function_depth: 0,
                dollar_vars_changed_by_set: false,
                random_state: RandomGen::seeded(),
                subshell_depth: Cell::new(0),
                in_command_substitution: Cell::new(false),
                stdin_redir: Cell::new(false),
                job_table: crate::jobs::table::JobTable::default(),
                last_background_pid: None,
                coproc_names: HashMap::new(),
                completion_specs: crate::builtins::complete::CompletionRegistry::new(),
                session_history: None,
                arithmetic_expansion_error: Cell::new(false),
                arithmetic_nonfatal_error: Cell::new(false),
                arithmetic_fatal_error: Cell::new(false),
                arithmetic_nounset_error: Cell::new(false),
                arithmetic_last_error_category: Cell::new(None),
                parameter_bad_substitution: Cell::new(false),
                debug_trap_command: std::cell::RefCell::new(None),
                xtrace_fd: Cell::new(-1),
                xtrace_fd_source: std::cell::RefCell::new(String::new()),
                procsub_streams: std::cell::RefCell::new(HashMap::new()),
            },
            fd_table: FdTable::new(),
            exit_code: 0,
            parse_error_occurred: false,
            bash_logout_sourced: false,
            shell_pid,
            owns_signal_mailbox,
            background_children: HashMap::new(),
            coproc_stderr_forwarders: HashMap::new(),
            assignment_output_process_substitutions: HashMap::new(),
            pending_scalar_assignment: false,
            suppress_errexit: 0,
            debug_trap_running: false,
            return_trap_running: false,
            signal_trap_running: false,
            error_trap_running: false,
            sigchld_notifications_pending: std::cell::Cell::new(0),
            source_debug_suppressed: false,
            host_internal_depth: std::cell::Cell::new(0),
            debug_trap_function_line: None,
            arithmetic_last_error_expression: std::cell::RefCell::new(String::new()),
            arithmetic_last_eval_input: std::cell::RefCell::new(String::new()),
            assignment_command_name: None,
            buffer_assignment_diagnostics: false,
            pending_assignment_diagnostics: Vec::new(),
            parameter_assignment_failure: Cell::new(false),
            tempenv_names: Vec::new(),
            tempenv_marks: Vec::new(),
            tempenv_promoted_names: Vec::new(),
            tempenv_previous: HashMap::new(),
            tempenv_propagated_names: Vec::new(),
            function_tempenv_names: Vec::new(),
            evalerror_pending: Cell::new(false),
            evalerror_line: Cell::new(None),
            evalerror_exec_depth: Cell::new(0),
            reader_command_line: Cell::new(None),
            ambient_line: Cell::new(None),
            inside_compound_condition: Cell::new(false),
            conditional_invert_pending: Cell::new(false),
            inside_assignment_rhs: Cell::new(false),
            last_command_substitution_status: Cell::new(None),
            comsub_stdin_writeback: Cell::new(None),
            pipeline_stdin_consumed: Cell::new(None),
            last_heredoc_warning_source: RefCell::new(None),
            comsub_leading_newlines: Cell::new(0),
            current_shell_substitution_exit: Cell::new(None),
            last_command_substitution_parse_error: Cell::new(false),
            last_command_inverted: Cell::new(false),
            exit_jump_pending: Cell::new(false),
            upstream_script_consumed: Cell::new(false),
            special_builtin_failed: Cell::new(false),
            last_builtin_write_failed: Cell::new(false),
            redirect_target_memo: RefCell::new(HashMap::new()),
            fd_var_external_undo: Vec::new(),
            read_deadline: None,
            read_timed_out: false,
            stdout_capture: None,
            stderr_capture: None,
            host_external_command_handler: None,
            #[cfg(windows)]
            elevation_handler: None,
            external_file_builtins_enabled: true,
            process_env_snapshot,
            history_provider: None,
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
        env_vars: &mut HashMap<String, String>,
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
        let pwd = match std::env::current_dir() {
            Ok(path) => shell_pwd_display(&path.to_string_lossy().replace('\\', "/")),
            Err(_) => env_vars
                .get("PWD")
                .cloned()
                .map(|value| shell_pwd_display(&value))
                .unwrap_or_else(|| "/".to_string()),
        };
        env_vars.insert("PWD".to_string(), pwd);
        // Suites write $TMPDIR into generated scripts unquoted
        // (posix2.tests conftest2: `$TMPDIR/conftest2 "$@"`); an inherited
        // Windows backslash path is escape syntax to the shell reader and
        // corrupts to `D:repo...`. $HOME has the same problem in pattern
        // position (exp.tests: `${x#$HOME}` — `\U`/`\A` become pattern
        // escapes so the prefix never strips). Windows filesystem APIs
        // accept forward slashes, so normalize inherited values to the
        // shell-safe form.
        for name in ["TMPDIR", "HOME"] {
            if let Some(value) = env_vars.get_mut(name) {
                if value.contains('\\') {
                    *value = value.replace('\\', "/");
                }
            }
        }
        env_vars
            .entry("TMPDIR".to_string())
            .or_insert_with(safe_temp_dir_string);
        crate::executor::path::ensure_var_tmp_dir(env_vars);
        env_vars
            .entry("SHELL".to_string())
            .or_insert_with(shell_path_value);
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
            let basename = value
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(value.as_str());
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
            env::remove_var("OLDPWD");
        }
        initialize_shell_level(env_vars);
        mark_initial_exported_vars(env_vars);
        mark_env_name(env_vars, EXPORTED_VARS, "OLDPWD");
        env_vars
            .entry("IFS".to_string())
            .or_insert_with(|| " \t\n".to_string());
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
        if let Some(value) = env_vars.get("SHELLOPTS").cloned() {
            for name in value.split(':').filter(|name| !name.is_empty()) {
                if crate::builtins::set::is_shell_option(name) {
                    crate::builtins::set::set_shell_option(env_vars, name, true);
                }
            }
        }
        if let Some(value) = env_vars.get("BASHOPTS").cloned() {
            for name in value.split(':').filter(|name| !name.is_empty()) {
                if crate::builtins::shopt::is_supported_option(name) {
                    crate::builtins::shopt::set_option(env_vars, name, true);
                }
            }
        }
        env_vars.insert(
            "SHELLOPTS".to_string(),
            crate::builtins::set::shellopts_value(env_vars),
        );
        mark_env_name(env_vars, READONLY_VARS, "SHELLOPTS");
        env_vars.insert(
            "BASHOPTS".to_string(),
            crate::builtins::shopt::bashopts_value(env_vars),
        );
        mark_env_name(env_vars, READONLY_VARS, "BASHOPTS");
        mark_env_name(env_vars, ARRAY_VARS, "PIPESTATUS");
        env_vars.insert("OPTIND".to_string(), "1".to_string());
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
    }
}
