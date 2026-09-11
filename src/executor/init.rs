use super::*;
use crate::shell::VariableStore;

fn shell_pwd_display(path: &str) -> String {
    #[cfg(windows)]
    {
        if std::env::var_os("WINUXSH_SHELL_PATH_STYLE").is_some() {
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
        env_vars.remove("__RUBASH_CURRENT_FUNCTION");
        env_vars.remove("__RUBASH_IN_SOURCE");
        if env_vars.get("__RUBASH_COPROC_CHILD").map(String::as_str) != Some("1") {
            env_vars.remove("__RUBASH_SCRIPT_NAME");
        }
        env_vars.remove("__RUBASH_SHELL_NAME");
        env_vars.remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
        env::remove_var("__RUBASH_CURRENT_FUNCTION");
        env::remove_var("__RUBASH_IN_SOURCE");
        if env_vars.get("__RUBASH_COPROC_CHILD").map(String::as_str) != Some("1") {
            env::remove_var("__RUBASH_SCRIPT_NAME");
        }
        env::remove_var("__RUBASH_SHELL_NAME");
        env::remove_var(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
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
        env_vars
            .entry("TMPDIR".to_string())
            .or_insert_with(safe_temp_dir_string);
        crate::executor::path::ensure_var_tmp_dir(&env_vars);
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
        env_vars.insert(
            "THIS_SH".to_string(),
            std::env::current_exe()
                .map(|path| path.to_string_lossy().replace('\\', "/").to_string())
                .unwrap_or_else(|_| "rubash".to_string()),
        );
        env_vars.remove("OLDPWD");
        initialize_shell_level(&mut env_vars);
        mark_initial_exported_vars(&mut env_vars);
        mark_env_name(&mut env_vars, EXPORTED_VARS, "OLDPWD");
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
                    crate::builtins::set::set_shell_option(&mut env_vars, name, true);
                }
            }
        }
        if let Some(value) = env_vars.get("BASHOPTS").cloned() {
            for name in value.split(':').filter(|name| !name.is_empty()) {
                if crate::builtins::shopt::is_supported_option(name) {
                    crate::builtins::shopt::set_option(&mut env_vars, name, true);
                }
            }
        }
        env_vars.insert(
            "SHELLOPTS".to_string(),
            crate::builtins::set::shellopts_value(&env_vars),
        );
        mark_env_name(&mut env_vars, READONLY_VARS, "SHELLOPTS");
        env_vars.insert(
            "BASHOPTS".to_string(),
            crate::builtins::shopt::bashopts_value(&env_vars),
        );
        mark_env_name(&mut env_vars, READONLY_VARS, "BASHOPTS");
        mark_env_name(&mut env_vars, ARRAY_VARS, "PIPESTATUS");
        env_vars.insert("OPTIND".to_string(), "1".to_string());
        env_vars.remove("OPTARG");
        env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        env_vars
            .entry("BASH_VERSION".to_string())
            .or_insert_with(bash_version_value);
        env_vars
            .entry("BASH".to_string())
            .or_insert_with(bash_path_value);
        store_indexed_array(&mut env_vars, "BASH_VERSINFO", bash_versinfo_values());
        mark_env_name(&mut env_vars, READONLY_VARS, "BASH_VERSINFO");
        store_indexed_array(&mut env_vars, "BASH_ARGC", Vec::new());
        store_indexed_array(&mut env_vars, "BASH_ARGV", Vec::new());
        store_indexed_array(&mut env_vars, "BASH_LINENO", vec!["0".to_string()]);
        store_indexed_array(&mut env_vars, "BASH_SOURCE", Vec::new());
        env_vars.insert("BASH_CMDS".to_string(), "()".to_string());
        mark_env_name(&mut env_vars, ASSOC_VARS, "BASH_CMDS");
        env_vars.insert("BASH_ALIASES".to_string(), "()".to_string());
        mark_env_name(&mut env_vars, ASSOC_VARS, "BASH_ALIASES");
        // The stored DIRSTACK cell starts as an empty indexed array, not
        // scalar-empty: a bare `declare -a` listing prints the last
        // materialized cell (variables.c get_dirstack runs on named access
        // only), and GNU shows `declare -a DIRSTACK=()` there.
        store_indexed_array(&mut env_vars, "DIRSTACK", Vec::new());
        env_vars.insert("FUNCNAME".to_string(), String::new());
        mark_env_name(&mut env_vars, ARRAY_VARS, "FUNCNAME");
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
        mark_env_name(&mut env_vars, READONLY_VARS, "UID");
        mark_env_name(&mut env_vars, READONLY_VARS, "EUID");
        mark_env_name(&mut env_vars, READONLY_VARS, "PPID");
        let shell_pid = env_vars
            .get("__RUBASH_SHELL_PID")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_else(std::process::id);
        env_vars.remove("__RUBASH_SHELL_PID");
        let owns_signal_mailbox =
            if env_vars.get("__RUBASH_COPROC_CHILD").map(String::as_str) == Some("1") {
                // A blocked coprocess reader cannot consume a queued TERM.
                false
            } else {
                crate::builtins::kill::register_signal_mailbox(std::process::id()).is_ok()
            };

        Self {
            shell_state: ShellState {
                variables: VariableStore::from_environment(&env_vars),
                ..ShellState::default()
            },
            fd_table: FdTable::new(),
            job_table: JobTable::default(),
            exit_code: 0,
            parse_error_occurred: false,
            bash_logout_sourced: false,
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
            random_state: Cell::new(current_epoch_micros() as u32),
            shell_pid,
            subshell_depth: Cell::new(0),
            owns_signal_mailbox,
            last_background_pid: None,
            background_children: HashMap::new(),
            background_jobs: HashMap::new(),
            background_job_order: Vec::new(),
            coproc_stdin_writers: HashMap::new(),
            coproc_stdout_readers: HashMap::new(),
            coproc_stderr_forwarders: HashMap::new(),
            assignment_output_process_substitutions: HashMap::new(),
            pending_scalar_assignment: false,
            suppress_errexit: 0,
            debug_trap_running: false,
            return_trap_running: false,
            signal_trap_running: false,
            sigchld_notifications_pending: std::cell::Cell::new(0),
            source_debug_suppressed: false,
            debug_trap_command: std::cell::RefCell::new(None),
            debug_trap_function_line: None,
            arithmetic_expansion_error: Cell::new(false),
            arithmetic_nonfatal_error: Cell::new(false),
            arithmetic_fatal_error: Cell::new(false),
            arithmetic_nounset_error: Cell::new(false),
            arithmetic_last_error_category: Cell::new(None),
            inside_compound_condition: Cell::new(false),
            inside_assignment_rhs: Cell::new(false),
            last_command_substitution_status: Cell::new(None),
            current_shell_substitution_exit: Cell::new(None),
            last_command_substitution_parse_error: Cell::new(false),
            stdout_capture: None,
            stderr_capture: None,
            host_external_command_handler: None,
            #[cfg(windows)]
            elevation_handler: None,
            external_file_builtins_enabled: true,
            process_env_snapshot,
            history_provider: None,
            session_history: None,
            last_notified_job_ids: HashSet::new(),
            completion_specs: crate::builtins::complete::CompletionRegistry::new(),
        }
    }
}
