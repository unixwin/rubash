use crate::executor::Executor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInvocation {
    pub posix: bool,
    pub login: bool,
    pub no_profile: bool,
    pub no_rc: bool,
    /// --rcfile <file> / --init-file <file>: use this startup file instead of
    /// the default interactive startup file.
    pub rc_file: Option<String>,
    /// --noediting: do not use readline-style line editing in the REPL.
    pub no_editing: bool,
    /// -i: force an interactive shell even when stdin is not a terminal.
    pub interactive: bool,
    /// -D / --dump-strings / --dump-po-strings: print locale strings ($"...")
    /// and exit instead of executing.
    pub dump_strings: bool,
    /// --dump-po-strings selects GNU gettext PO output format.
    pub dump_po: bool,
    /// --pretty-print: parse input and print it back in normalized form.
    pub pretty_print: bool,
    /// --debug / --debugger: accepted; --debugger enables extdebug so a
    /// debugger can install its hooks.
    pub debugger: bool,
    pub command: Option<String>,
    pub command_name: Option<String>,
    pub positional_params: Vec<String>,
    pub script: Option<String>,
    pub read_stdin: bool,
    pub restricted: bool,
    pub shell_flags: Vec<(String, bool)>,
    pub shopt_flags: Vec<(String, bool)>,
}

impl ShellInvocation {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        // GNU shell.c parse_shell_options walks the leading option words and
        // treats each `-`/`+` word as a cluster of single-letter flags. Most
        // letters are boolean flags, but `o` and `O` consume the NEXT argv
        // word as their value — never the rest of the cluster (shell.c:931-
        // 958; empirical GNU 5.3.0: `-eo pipefail` == `-e -o pipefail`,
        // while `-eopipefail` takes `-c` as the option name and fails).
        // Normalize each cluster into the token shapes the loop below
        // consumes: one `-<letter>` / `+<letter>` per flag, with `-o`/`-O`
        // immediately followed by the argv word they take. Long options
        // (`--rcfile`) and the first non-option word are copied verbatim.
        let mut expanded = Vec::with_capacity(args.len());
        let mut i = 0usize;
        while i < args.len() {
            let arg = args[i].as_str();
            if arg == "-" || arg == "--" {
                // End of options (shell.c:905-911): the rest is the operand.
                expanded.extend(args[i..].iter().cloned());
                break;
            }
            if arg.starts_with("--") {
                expanded.push(arg.to_string());
                i += 1;
                continue;
            }
            // A word that does not begin with `-`/`+` ends the option run
            // (shell.c:899): copy it and everything after it untouched.
            let Some(sign) = arg.chars().next().filter(|c| *c == '-' || *c == '+') else {
                expanded.extend(args[i..].iter().cloned());
                break;
            };
            let letters: Vec<char> = arg[1..].chars().collect();
            if letters.is_empty() {
                expanded.push(arg.to_string());
                i += 1;
                continue;
            }
            // A lone `-o`/`-O` takes the next argv word directly; a cluster
            // that includes `o`/`O` takes the following words in turn. Only
            // clusters made entirely of accepted letters are expanded — a
            // cluster with an unknown letter is left for the loop below to
            // report as GNU's invalid option.
            let lone_value_option = letters.len() == 1 && (letters[0] == 'o' || letters[0] == 'O');
            let known_cluster = letters.len() > 1
                && letters
                    .iter()
                    .all(|flag| crate::usage::is_short_option_letter(*flag));
            if lone_value_option || known_cluster {
                let mut next_value = i + 1;
                for &flag in &letters {
                    if flag == 'o' || flag == 'O' {
                        expanded.push(format!("{sign}{flag}"));
                        if let Some(value) = args.get(next_value) {
                            expanded.push(value.clone());
                            next_value += 1;
                        }
                    } else if flag == 'c' {
                        expanded.push("-c".to_string());
                    } else if flag == 's' {
                        expanded.push("-s".to_string());
                    } else if flag == 'l' {
                        expanded.push("-l".to_string());
                    } else if flag == 'i' {
                        expanded.push("-i".to_string());
                    } else if flag == 'D' {
                        expanded.push("-D".to_string());
                    } else {
                        expanded.push(format!("{sign}{flag}"));
                    }
                }
                i = next_value;
                continue;
            }
            expanded.push(arg.to_string());
            i += 1;
        }

        let mut out = Self {
            posix: false,
            login: false,
            no_profile: false,
            no_rc: false,
            rc_file: None,
            no_editing: false,
            interactive: false,
            dump_strings: false,
            dump_po: false,
            pretty_print: false,
            debugger: false,
            command: None,
            command_name: None,
            positional_params: Vec::new(),
            script: None,
            read_stdin: false,
            restricted: false,
            shell_flags: Vec::new(),
            shopt_flags: Vec::new(),
        };
        let mut i = 0usize;
        while i < expanded.len() {
            let arg = expanded[i].as_str();
            match arg {
                "-c" => {
                    i += 1;
                    while let Some(next) = parse_post_c_option(&expanded, i, &mut out)? {
                        i = next;
                    }
                    let command = expanded
                        .get(i)
                        .ok_or_else(|| "-c: option requires an argument".to_string())?;
                    out.command = Some(command.clone());
                    if let Some(name) = expanded.get(i + 1) {
                        out.command_name = Some(name.clone());
                        out.positional_params = expanded[i + 2..].to_vec();
                    }
                    return Ok(out);
                }
                "-s" => {
                    out.read_stdin = true;
                    out.positional_params = expanded[i + 1..].to_vec();
                    return Ok(out);
                }
                "-i" => {
                    out.interactive = true;
                    i += 1;
                }
                "-D" => {
                    out.dump_strings = true;
                    i += 1;
                }
                "--posix" => {
                    out.posix = true;
                    i += 1;
                }
                "--login" | "-l" => {
                    out.login = true;
                    i += 1;
                }
                "--noprofile" => {
                    out.no_profile = true;
                    i += 1;
                }
                "--norc" => {
                    out.no_rc = true;
                    i += 1;
                }
                "--rcfile" | "--init-file" => {
                    let file = expanded
                        .get(i + 1)
                        .ok_or_else(|| format!("{arg}: option requires an argument"))?;
                    out.rc_file = Some(file.clone());
                    i += 2;
                }
                "--noediting" => {
                    out.no_editing = true;
                    i += 1;
                }
                "--dump-strings" => {
                    out.dump_strings = true;
                    i += 1;
                }
                "--dump-po-strings" => {
                    out.dump_strings = true;
                    out.dump_po = true;
                    i += 1;
                }
                "--pretty-print" => {
                    out.pretty_print = true;
                    i += 1;
                }
                "--debug" | "--debugger" => {
                    out.debugger = true;
                    i += 1;
                }
                "--restricted" => {
                    out.restricted = true;
                    i += 1;
                }
                "--verbose" => {
                    out.shell_flags.push(("verbose".to_string(), true));
                    i += 1;
                }
                "-o" | "+o" => {
                    let name = expanded
                        .get(i + 1)
                        .ok_or_else(|| format!("{arg}: option requires an argument"))?;
                    out.shell_flags.push((name.clone(), arg == "-o"));
                    i += 2;
                }
                "-O" | "+O" => {
                    let name = expanded
                        .get(i + 1)
                        .ok_or_else(|| format!("{arg}: option requires an argument"))?;
                    out.shopt_flags.push((name.clone(), arg == "-O"));
                    i += 2;
                }
                "--" => {
                    i += 1;
                    let script = expanded
                        .get(i)
                        .ok_or_else(|| "--: option requires a script".to_string())?;
                    out.script = Some(script.clone());
                    out.positional_params = expanded[i + 1..].to_vec();
                    return Ok(out);
                }
                option if option.starts_with('-') || option.starts_with('+') => {
                    let enabled = option.starts_with('-');
                    let flags = &option[1..];
                    if flags.is_empty() || flags.contains('o') {
                        return Err(format!("{option}: invalid option"));
                    }
                    for flag in flags.chars() {
                        match flag {
                            'i' => {
                                out.interactive = enabled;
                                continue;
                            }
                            'D' => {
                                out.dump_strings = enabled;
                                continue;
                            }
                            'r' => {
                                if out.restricted && !enabled {
                                    return Err(format!("{option}: invalid option"));
                                }
                                out.restricted = enabled;
                                continue;
                            }
                            _ => {}
                        }
                        let name = cli_flag_name(flag)
                            .ok_or_else(|| format!("{option}: invalid option"))?;
                        if name == "posix" {
                            out.posix = enabled;
                        }
                        out.shell_flags.push((name.to_string(), enabled));
                    }
                    i += 1;
                }
                script => {
                    out.script = Some(script.to_string());
                    out.positional_params = expanded[i + 1..].to_vec();
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    /// Apply invocation options when startup files have already run, or when
    /// the host does not run startup files. Hosts with startup files must use
    /// `apply_to_executor_before_startup`, run startup, then `finish_startup`.
    pub fn apply_to_executor(&self, executor: &mut Executor) -> Result<(), String> {
        self.apply_to_executor_before_startup(executor)?;
        self.finish_startup(executor);
        Ok(())
    }

    /// Apply options without activating invocation restrictions during startup.
    /// GNU shell.c:679-744 main saves -r across run_startup_files and activates
    /// it afterward through maybe_make_restricted (shell.c:1278).
    pub fn apply_to_executor_before_startup(&self, executor: &mut Executor) -> Result<(), String> {
        if self.interactive {
            executor.set_shopt_option("expand_aliases", true);
        }
        // shell.c:497-503: --login/-l flips LOGIN_SHELL so the `logout`
        // builtin and exit path see a login shell.
        if self.login {
            executor.set_env("__RUBASH_LOGIN_SHELL", "1");
        }
        for (name, enabled) in &self.shell_flags {
            if !executor.is_shell_option(name) {
                return Err(format!("{name}: invalid shell option name"));
            }
            // posix flips (with the set_posix_mode walk and the
            // __RUBASH_POSIX_MODE sync) live in set::set_shell_option.
            executor.set_shell_option(name, *enabled);
        }
        for (name, enabled) in &self.shopt_flags {
            if !executor.set_shopt_option(name, *enabled) {
                return Err(format!("{name}: invalid shell option name"));
            }
        }
        if self.debugger {
            // GNU bash --debugger enables extdebug so a debugger can install
            // its hooks; the host shell decides the rest.
            executor.set_shopt_option("extdebug", true);
        }
        if self.posix {
            executor.set_shell_option("posix", true);
        }
        if let Some(name) = &self.command_name {
            // GNU shell.c:523-528: the operand after the -c command string
            // becomes dollar_vars[0]; BASH_ARGV0 mirrors it (variables.c).
            // Hosts like niu pre-seed BASH_ARGV0 with their own name, so the
            // explicit operand must overwrite it or `$0` keeps the host name.
            executor.set_env("__RUBASH_SCRIPT_NAME", name);
            executor.set_env("BASH_ARGV0", name);
        } else if self.command.is_some() && executor.get_env("__RUBASH_SCRIPT_NAME").is_none() {
            // GNU `bash -c` without explicit $0 reports as "bash: -c: ...".
            // Internal respawns (coproc / async `&` children) carry the
            // parent's script name through init.rs — GNU's forked subshell
            // keeps $0, so do not clobber it here.
            executor.set_env("__RUBASH_SCRIPT_NAME", "bash");
        }
        if self.command.is_some() {
            executor.set_env("__RUBASH_IS_C", "1");
        }
        if self.read_stdin {
            // rubash#591: flags.c:294-306 which_set_flags shows `s` in `$-`
            // while GNU's read_from_stdin (shell.c:301) is set, and
            // shell.c:928 records the explicit -s at option-parse time. The
            // engine's own drivers arm the marker inside run_stdin_script /
            // run_interactive_stdin; a library host (niu) drives the stdin
            // script through its own reader after this shared invocation
            // surface, so the marker must be armed here too — an explicit
            // -s keeps the `s` in `$-` whatever driver serves the input.
            // Like the C global it never reaches child processes (the
            // `__RUBASH_` env filter).
            executor.set_env(crate::script_driver::READ_STDIN_MARKER, "1");
        }
        executor.set_positional_params(self.positional_params.clone());
        Ok(())
    }

    /// Activate invocation restrictions after the host finishes startup files.
    pub fn finish_startup(&self, executor: &mut Executor) {
        if self.restricted {
            executor.set_shell_option("restricted", true);
        }
    }
}

/// GNU shell.c:722 run_startup_files -> run_login_profile_files
/// (shell.c:1836-1880): a login shell sources /etc/profile and then the
/// FIRST EXISTING of ~/.bash_profile, ~/.bash_login, ~/.profile. A missing
/// candidate is skipped silently (open_startup_file's failed fopen is not
/// an error). `exists` is injected so the fallback order is unit-testable
/// on every host.
pub fn login_profile_files(home: &str, exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut files = Vec::new();
    let system_profile = "/etc/profile";
    if exists(system_profile) {
        files.push(system_profile.to_string());
    }
    for name in [".bash_profile", ".bash_login", ".profile"] {
        let candidate = format!("{home}/{name}");
        if exists(&candidate) {
            files.push(candidate);
            break;
        }
    }
    files
}

/// GNU shell.c:1863-1880 (the non-login interactive branch of
/// run_startup_files): sources /etc/bash.bashrc (SYS_BASHRC, the Debian
/// system rc) and then ~/.bashrc — or `rc_override` (--rcfile/--init-file)
/// in place of the personal rc. `exists` is injected for host-independent
/// tests. Login shells never take this branch: the profile chain replaces
/// it entirely (shell.c:1833 `if (login_shell) ... else if (interactive)`).
pub fn nonlogin_interactive_files(
    home: &str,
    rc_override: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut files = Vec::new();
    let system_rc = "/etc/bash.bashrc";
    if exists(system_rc) {
        files.push(system_rc.to_string());
    }
    match rc_override {
        Some(rc) => files.push(rc.to_string()),
        None => {
            let personal = format!("{home}/.bashrc");
            if exists(&personal) {
                files.push(personal);
            }
        }
    }
    files
}

fn parse_post_c_option(
    args: &[String],
    index: usize,
    out: &mut ShellInvocation,
) -> Result<Option<usize>, String> {
    let Some(arg) = args.get(index).map(String::as_str) else {
        return Ok(None);
    };
    match arg {
        "--login" | "-l" => {
            out.login = true;
            Ok(Some(index + 1))
        }
        "--posix" => {
            out.posix = true;
            Ok(Some(index + 1))
        }
        "--noprofile" => {
            out.no_profile = true;
            Ok(Some(index + 1))
        }
        "--norc" => {
            out.no_rc = true;
            Ok(Some(index + 1))
        }
        "--noediting" => {
            out.no_editing = true;
            Ok(Some(index + 1))
        }
        "--debug" | "--debugger" => {
            out.debugger = true;
            Ok(Some(index + 1))
        }
        "-o" | "+o" => {
            let name = args
                .get(index + 1)
                .ok_or_else(|| format!("{arg}: option requires an argument"))?;
            out.shell_flags.push((name.clone(), arg == "-o"));
            Ok(Some(index + 2))
        }
        "-O" | "+O" => {
            let name = args
                .get(index + 1)
                .ok_or_else(|| format!("{arg}: option requires an argument"))?;
            out.shopt_flags.push((name.clone(), arg == "-O"));
            Ok(Some(index + 2))
        }
        option if option.starts_with('-') || option.starts_with('+') => {
            let enabled = option.starts_with('-');
            let flags = &option[1..];
            if flags.is_empty() || flags.contains('o') || flags.contains('O') {
                return Ok(None);
            }
            for flag in flags.chars() {
                match flag {
                    'i' => {
                        out.interactive = enabled;
                        continue;
                    }
                    'D' => {
                        out.dump_strings = enabled;
                        continue;
                    }
                    'r' => {
                        if out.restricted && !enabled {
                            return Err(format!("{option}: invalid option"));
                        }
                        out.restricted = enabled;
                        continue;
                    }
                    _ => {}
                }
                let name = match cli_flag_name(flag) {
                    Some(name) => name,
                    None => return Ok(None),
                };
                if name == "posix" {
                    out.posix = enabled;
                }
                out.shell_flags.push((name.to_string(), enabled));
            }
            Ok(Some(index + 1))
        }
        _ => Ok(None),
    }
}

fn cli_flag_name(flag: char) -> Option<&'static str> {
    match flag {
        'a' => Some("allexport"),
        'b' => Some("notify"),
        'e' => Some("errexit"),
        'f' => Some("noglob"),
        'h' => Some("hashall"),
        'k' => Some("keyword"),
        'm' => Some("monitor"),
        'n' => Some("noexec"),
        'p' => Some("privileged"),
        'r' => Some("restricted"),
        't' => Some("onecmd"),
        'u' => Some("nounset"),
        'v' => Some("verbose"),
        'x' => Some("xtrace"),
        'B' => Some("braceexpand"),
        'C' => Some("noclobber"),
        'E' => Some("errtrace"),
        'H' => Some("histexpand"),
        'P' => Some("physical"),
        'T' => Some("functrace"),
        _ => None,
    }
}

/// niubash#362: out-of-band transport for an over-long `-c`
/// command string.
///
/// A respawned async `&` child is re-exec'd with the WHOLE serialized script
/// (every function definition carried by the parent + the command) as its
/// `-c` argument. On Windows that string becomes part of `CreateProcessW`'s
/// lpCommandLine, which is capped at 32767 UTF-16 units, so a function-heavy
/// shell (`echo hello &` after oh-my-niu sourced a few hundred functions)
/// aborts the spawn with ERROR_FILENAME_EXCED_RANGE, surfaced as
/// "File name too long". Unix has the analogous per-argument execve cap
/// MAX_ARG_STRLEN (128 KiB). `compound_exec` (the spawn side) therefore
/// writes such a script to a temp file and names it in the child
/// environment as `__RUBASH_CMD_FILE`.
///
/// This runs at process entry, BEFORE argument parsing and before the
/// executor imports the process environment, so it splices the file's
/// contents back in as the operand following the first `-c` token — exactly
/// as if the parent had passed `-c <script>` on the command line — and
/// removes both the environment key and the temp file. Every downstream
/// semantic (`BASH_EXECUTION_STRING`, command-string mode, diagnostics,
/// `__RUBASH_IS_C`) therefore stays byte-identical to the argv path. A
/// no-op when the key is absent.
pub fn materialize_command_file(args: &mut Vec<String>) {
    let Some(path) = std::env::var_os("__RUBASH_CMD_FILE") else {
        return;
    };
    // Consume the key unconditionally so it can never leak to a grandchild
    // or into the shell's variable table, even if the read below fails.
    std::env::remove_var("__RUBASH_CMD_FILE");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path);
    if let Some(index) = args.iter().position(|arg| arg == "-c") {
        if index + 1 < args.len() {
            args[index + 1] = contents;
        } else {
            args.push(contents);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combined_flags_and_command() {
        let args = vec![
            "-ne".into(),
            "-c".into(),
            "printf ok".into(),
            "demo".into(),
            "x".into(),
        ];
        let parsed = ShellInvocation::parse(&args).unwrap();
        assert_eq!(parsed.command.as_deref(), Some("printf ok"));
        assert!(parsed.shell_flags.contains(&("noexec".into(), true)));
        assert!(parsed.shell_flags.contains(&("errexit".into(), true)));
        assert_eq!(parsed.positional_params, vec!["x"]);
    }

    #[test]
    fn parses_combined_login_command_flags() {
        // Harnesses (ZCode snapshot probe, tmux default-shell) spawn
        // `<shell> -lc '<script>'`; GNU bash accepts the combined form.
        let args = vec!["-lc".into(), "env -0".into()];
        let parsed = ShellInvocation::parse(&args).unwrap();
        assert_eq!(parsed.command.as_deref(), Some("env -0"));
        assert!(parsed.login);

        let interactive = ShellInvocation::parse(&["-ilc".into(), "env -0".into()]).unwrap();
        assert_eq!(interactive.command.as_deref(), Some("env -0"));
        assert!(interactive.login && interactive.interactive);
    }

    // rubash#382: GNU shell.c parse_shell_options (shell.c:931-958) lets
    // `o`/`O` in a combined option take the NEXT argv word as their value.
    // `-eo pipefail -c ...` is a common CI/Makefile invocation, so the
    // clustered form must match the separated one byte for byte.

    #[test]
    fn clustered_value_option_consumes_the_next_argv() {
        let clustered =
            ShellInvocation::parse(&["-eo".into(), "pipefail".into(), "-c".into(), "true".into()])
                .unwrap();
        let separated = ShellInvocation::parse(&[
            "-e".into(),
            "-o".into(),
            "pipefail".into(),
            "-c".into(),
            "true".into(),
        ])
        .unwrap();
        assert_eq!(clustered.shell_flags, separated.shell_flags);
        assert!(clustered.shell_flags.contains(&("errexit".into(), true)));
        assert!(clustered.shell_flags.contains(&("pipefail".into(), true)));
        assert_eq!(clustered.command.as_deref(), Some("true"));
    }

    #[test]
    fn clustered_value_option_after_other_flags_still_takes_next_argv() {
        // `-euo pipefail`: `e` and `u` are boolean, `o` takes `pipefail`.
        let parsed =
            ShellInvocation::parse(&["-euo".into(), "pipefail".into(), "-c".into(), "true".into()])
                .unwrap();
        assert!(parsed.shell_flags.contains(&("errexit".into(), true)));
        assert!(parsed.shell_flags.contains(&("nounset".into(), true)));
        assert!(parsed.shell_flags.contains(&("pipefail".into(), true)));
        assert_eq!(parsed.command.as_deref(), Some("true"));
    }

    #[test]
    fn clustered_value_option_does_not_take_the_rest_of_the_cluster() {
        // GNU takes the next ARGV word, not the cluster tail: the letters
        // after `o` stay flags, so `-eopipefail` binds the following `-c`
        // as the option name (and later fails) rather than setting
        // pipefail. Pin the flag scanning shape.
        let parsed =
            ShellInvocation::parse(&["-eop".into(), "pipefail".into(), "-c".into(), "true".into()])
                .unwrap();
        // `p` is `privileged` (a flag), and the option name is `pipefail`
        // from the next argv — the letters after `o` are not swallowed as
        // the value.
        assert!(parsed.shell_flags.contains(&("privileged".into(), true)));
        assert_eq!(
            parsed
                .shell_flags
                .iter()
                .filter(|(name, _)| name == "pipefail")
                .count(),
            1
        );
    }

    #[test]
    fn clustered_shopt_option_consumes_the_next_argv() {
        let parsed =
            ShellInvocation::parse(&["-eO".into(), "extglob".into(), "-c".into(), "true".into()])
                .unwrap();
        assert!(parsed.shell_flags.contains(&("errexit".into(), true)));
        assert_eq!(parsed.shopt_flags, vec![("extglob".into(), true)]);
        assert_eq!(parsed.command.as_deref(), Some("true"));
    }

    #[test]
    fn clustered_plus_value_option_carries_the_sign() {
        let parsed =
            ShellInvocation::parse(&["+eo".into(), "pipefail".into(), "-c".into(), "true".into()])
                .unwrap();
        assert_eq!(
            parsed.shell_flags,
            vec![("errexit".into(), false), ("pipefail".into(), false),]
        );
        assert_eq!(parsed.command.as_deref(), Some("true"));
    }

    #[test]
    fn cluster_after_the_script_is_a_positional_parameter() {
        // GNU stops option parsing at the first non-option word: a
        // `-`-looking word after the script name is an ordinary positional,
        // not a flag cluster to expand.
        let parsed =
            ShellInvocation::parse(&["script.sh".into(), "-eo".into(), "pipefail".into()]).unwrap();
        assert_eq!(parsed.script.as_deref(), Some("script.sh"));
        assert_eq!(parsed.positional_params, vec!["-eo", "pipefail"]);
    }

    #[test]
    fn parses_compat_options_after_c_before_command_string() {
        let args = vec![
            "-c".into(),
            "-l".into(),
            "--norc".into(),
            "-e".into(),
            "printf ok".into(),
            "demo".into(),
            "x".into(),
        ];
        let parsed = ShellInvocation::parse(&args).unwrap();
        assert!(parsed.login);
        assert!(parsed.no_rc);
        assert!(parsed.shell_flags.contains(&("errexit".into(), true)));
        assert_eq!(parsed.command.as_deref(), Some("printf ok"));
        assert_eq!(parsed.command_name.as_deref(), Some("demo"));
        assert_eq!(parsed.positional_params, vec!["x"]);
    }

    #[test]
    fn parses_full_bash_invocation_surface() {
        let args = vec![
            "--posix".into(),
            "--norc".into(),
            "--noprofile".into(),
            "--noediting".into(),
            "-i".into(),
            "-D".into(),
            "--rcfile".into(),
            "custom.rc".into(),
            "-c".into(),
            "true".into(),
        ];
        let parsed = ShellInvocation::parse(&args).unwrap();
        assert!(parsed.posix);
        assert!(parsed.no_rc);
        assert!(parsed.no_profile);
        assert!(parsed.no_editing);
        assert!(parsed.interactive);
        assert!(parsed.dump_strings);
        assert_eq!(parsed.rc_file.as_deref(), Some("custom.rc"));
        assert_eq!(parsed.command.as_deref(), Some("true"));
    }

    #[test]
    fn parses_dump_po_pretty_print_debugger_and_long_flags() {
        let args = vec![
            "--dump-po-strings".into(),
            "--pretty-print".into(),
            "--debugger".into(),
            "--restricted".into(),
            "--verbose".into(),
            "--init-file".into(),
            "alt.rc".into(),
        ];
        let parsed = ShellInvocation::parse(&args).unwrap();
        assert!(parsed.dump_strings);
        assert!(parsed.dump_po);
        assert!(parsed.pretty_print);
        assert!(parsed.debugger);
        assert!(parsed.restricted);
        assert!(parsed.shell_flags.contains(&("verbose".into(), true)));
        assert_eq!(parsed.rc_file.as_deref(), Some("alt.rc"));
    }

    #[test]
    fn parses_combined_flags_with_interactive_and_dump() {
        let args = vec!["-iDc".into(), "printf hi".into()];
        let parsed = ShellInvocation::parse(&args).unwrap();
        assert!(parsed.interactive);
        assert!(parsed.dump_strings);
        assert_eq!(parsed.command.as_deref(), Some("printf hi"));
    }

    #[test]
    fn maps_all_short_set_flags_to_option_names() {
        let args = vec!["-abefhkmnprtuvxBCEHPT".into(), "script.sh".into()];
        let parsed = ShellInvocation::parse(&args).unwrap();
        for name in [
            "allexport",
            "notify",
            "errexit",
            "noglob",
            "hashall",
            "keyword",
            "monitor",
            "noexec",
            "privileged",
            "onecmd",
            "nounset",
            "verbose",
            "xtrace",
            "braceexpand",
            "noclobber",
            "errtrace",
            "histexpand",
            "physical",
            "functrace",
        ] {
            assert!(
                parsed.shell_flags.contains(&(name.to_string(), true)),
                "missing set flag {name}"
            );
        }
        assert_eq!(parsed.script.as_deref(), Some("script.sh"));
        assert!(parsed.restricted);
    }

    #[test]
    fn login_profile_chain_prefers_bash_profile_then_falls_back() {
        // /etc/profile always leads (when present); the personal file is the
        // FIRST existing of bash_profile > bash_login > profile, never all.
        let set = |present: &[&str]| {
            let owned: Vec<String> = present.iter().map(|p| p.to_string()).collect();
            move |p: &str| owned.iter().any(|candidate| candidate == p)
        };
        let files = crate::invocation::login_profile_files(
            "/home/u",
            set(&[
                "/etc/profile",
                "/home/u/.bash_profile",
                "/home/u/.bash_login",
                "/home/u/.profile",
            ]),
        );
        assert_eq!(files, vec!["/etc/profile", "/home/u/.bash_profile"]);

        // Fallback order: bash_login when bash_profile is missing.
        let files = crate::invocation::login_profile_files(
            "/home/u",
            set(&["/etc/profile", "/home/u/.bash_login", "/home/u/.profile"]),
        );
        assert_eq!(files, vec!["/etc/profile", "/home/u/.bash_login"]);

        // Final fallback .profile; missing /etc/profile is silently skipped.
        let files = crate::invocation::login_profile_files("/home/u", set(&["/home/u/.profile"]));
        assert_eq!(files, vec!["/home/u/.profile"]);

        // Nothing exists: the chain is empty, not an error.
        assert!(crate::invocation::login_profile_files("/home/u", set(&[])).is_empty());
    }

    #[test]
    fn nonlogin_interactive_chain_is_system_rc_then_personal_rc() {
        let set = |present: &[&str]| {
            let owned: Vec<String> = present.iter().map(|p| p.to_string()).collect();
            move |p: &str| owned.iter().any(|candidate| candidate == p)
        };
        let files = crate::invocation::nonlogin_interactive_files(
            "/home/u",
            None,
            set(&["/etc/bash.bashrc", "/home/u/.bashrc"]),
        );
        assert_eq!(files, vec!["/etc/bash.bashrc", "/home/u/.bashrc"]);

        // A missing system rc is skipped; the personal rc still runs.
        let files = crate::invocation::nonlogin_interactive_files(
            "/home/u",
            None,
            set(&["/home/u/.bashrc"]),
        );
        assert_eq!(files, vec!["/home/u/.bashrc"]);

        // --rcfile replaces ONLY the personal rc, not the system one.
        let files = crate::invocation::nonlogin_interactive_files(
            "/home/u",
            Some("custom.rc"),
            set(&["/etc/bash.bashrc"]),
        );
        assert_eq!(files, vec!["/etc/bash.bashrc", "custom.rc"]);
    }
}

#[cfg(test)]
mod command_file_tests {
    use super::*;
    use std::sync::Mutex;

    // These tests mutate the process environment, so they serialize among
    // themselves (the key is only ever read by materialize_command_file, so
    // no other test can observe it).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rubash-cmdfile-{}-{tag}.sh", std::process::id()))
    }

    #[test]
    fn materialize_command_file_splices_operand_and_consumes_keys() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = scratch("splice");
        std::fs::write(&path, "echo from-file\n").unwrap();
        std::env::set_var("__RUBASH_CMD_FILE", path.to_string_lossy().as_ref());

        let mut args = vec!["niu".to_string(), "-c".to_string(), String::new()];
        materialize_command_file(&mut args);

        assert_eq!(args, vec!["niu", "-c", "echo from-file\n"]);
        assert!(std::env::var_os("__RUBASH_CMD_FILE").is_none());
        assert!(
            !path.exists(),
            "the temp file must be removed after the read"
        );
    }

    #[test]
    fn materialize_command_file_is_noop_without_the_key() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("__RUBASH_CMD_FILE");
        let mut args = vec!["niu".to_string(), "-c".to_string(), "echo hi".to_string()];
        materialize_command_file(&mut args);
        assert_eq!(args, vec!["niu", "-c", "echo hi"]);
    }

    #[test]
    fn materialize_command_file_consumes_key_when_file_is_missing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // A stale key must never survive, and the argv operand stays as-is.
        std::env::set_var("__RUBASH_CMD_FILE", "/nonexistent/rubash-cmd-file");
        let mut args = vec!["niu".to_string(), "-c".to_string(), "echo hi".to_string()];
        materialize_command_file(&mut args);
        assert_eq!(args, vec!["niu", "-c", "echo hi"]);
        assert!(std::env::var_os("__RUBASH_CMD_FILE").is_none());
    }
}
