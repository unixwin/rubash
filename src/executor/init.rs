use super::*;

impl Executor {
    pub fn new() -> Self {
        let mut env_vars: HashMap<String, String> = std::env::vars().collect();
        let imported_functions = import_exported_functions_from_env(&env_vars);
        env_vars.remove("__RUBASH_CURRENT_FUNCTION");
        env_vars.remove("__RUBASH_IN_SOURCE");
        env_vars.remove("__RUBASH_SCRIPT_NAME");
        env::remove_var("__RUBASH_CURRENT_FUNCTION");
        env::remove_var("__RUBASH_IN_SOURCE");
        env::remove_var("__RUBASH_SCRIPT_NAME");
        env_vars.remove("BASH_ARGV0");
        env_vars.remove("BASH_EXECUTION_STRING");
        env_vars.entry("PWD".to_string()).or_insert_with(|| {
            std::env::current_dir()
                .map(|path| shell_display_path(&path.to_string_lossy().replace('\\', "/")))
                .unwrap_or_else(|_| "/".to_string())
        });
        env_vars
            .entry("TMPDIR".to_string())
            .or_insert_with(safe_temp_dir_string);
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
        store_indexed_array(&mut env_vars, "PIPESTATUS", vec!["0".to_string()]);
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
        env_vars.insert("DIRSTACK".to_string(), String::new());
        mark_env_name(&mut env_vars, ARRAY_VARS, "DIRSTACK");
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

        Self {
            exit_code: 0,
            env_vars,
            aliases: HashMap::new(),
            functions: imported_functions,
            function_definition_redirects: HashMap::new(),
            positional_params: Vec::new(),
            local_var_scopes: Vec::new(),
            local_attr_scopes: Vec::new(),
            expanding_aliases: Vec::new(),
            loop_depth: 0,
            function_depth: 0,
            random_state: Cell::new(current_epoch_micros() as u32),
            subshell_depth: Cell::new(0),
            last_background_pid: None,
            suppress_errexit: 0,
            last_command_substitution_status: Cell::new(None),
            stdout_capture: None,
            stderr_capture: None,
        }
    }
}
