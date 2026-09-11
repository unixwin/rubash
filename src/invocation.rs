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
    pub shell_flags: Vec<(String, bool)>,
    pub shopt_flags: Vec<(String, bool)>,
}

impl ShellInvocation {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut expanded = Vec::with_capacity(args.len());
        for arg in args {
            if let Some(flags) = arg.strip_prefix('-') {
                let flags = flags.strip_prefix('-').unwrap_or(flags);
                if flags.len() > 1
                    && !flags.contains('o')
                    && !flags.contains('O')
                    && flags.chars().all(|f| {
                        f == 'c' || f == 's' || f == 'i' || f == 'D' || cli_flag_name(f).is_some()
                    })
                {
                    for flag in flags.chars() {
                        expanded.push(if flag == 'c' {
                            "-c".to_string()
                        } else {
                            format!("-{flag}")
                        });
                    }
                    continue;
                }
            }
            expanded.push(arg.clone());
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
            shell_flags: Vec::new(),
            shopt_flags: Vec::new(),
        };
        let mut i = 0usize;
        while i < expanded.len() {
            let arg = expanded[i].as_str();
            match arg {
                "-c" => {
                    let command = expanded
                        .get(i + 1)
                        .ok_or_else(|| "-c: option requires an argument".to_string())?;
                    out.command = Some(command.clone());
                    if let Some(name) = expanded.get(i + 2) {
                        out.command_name = Some(name.clone());
                        out.positional_params = expanded[i + 3..].to_vec();
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
                    out.shell_flags.push(("restricted".to_string(), true));
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

    pub fn apply_to_executor(&self, executor: &mut Executor) -> Result<(), String> {
        if self.posix {
            executor.set_env("__RUBASH_POSIX_MODE", "1");
        }
        for (name, enabled) in &self.shell_flags {
            if !executor.is_shell_option(name) {
                return Err(format!("{name}: invalid shell option name"));
            }
            executor.set_shell_option(name, *enabled);
            // Keep the environment bridge in sync for -o posix, the same way
            // rubash's own binary entry does for -o posix -c ...
            if name == "posix" {
                executor.set_env("__RUBASH_POSIX_MODE", if *enabled { "1" } else { "0" });
            }
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
            executor.set_env("__RUBASH_SCRIPT_NAME", name);
        }
        executor.set_positional_params(self.positional_params.clone());
        Ok(())
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
        assert!(parsed.shell_flags.contains(&("restricted".into(), true)));
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
            "restricted",
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
    }
}
