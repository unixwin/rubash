//! The invocation-aware `--help` / `--version` surface (issue #585,
//! rubash#596): GNU answers both off argv[0]'s basename (shell.c:477-482
//! show_shell_usage(stdout, extra=1), version.c:88-104), so the bash/sh
//! invocation faces print the GNU forms while product names keep the
//! product help. This module lives in the LIB so every binary that embeds
//! the engine answers identically — the niu host used to print its product
//! help under `bash --help` because the fork lived in this crate's bin
//! target only (owner report 2026-10-14).

/// The engine's accepted GNU long options (shell.c:264-274) — shared by the
/// argv parser and the usage table `show_shell_help` prints.
pub const LONG_OPTIONS: &[&str] = &[
    "debug",
    "debugger",
    "dump-po-strings",
    "dump-strings",
    "help",
    "init-file",
    "login",
    "noediting",
    "noprofile",
    "norc",
    "posix",
    "pretty-print",
    "rcfile",
    "restricted",
    "verbose",
    "version",
];

/// Long options that consume the following argv entry (type Charp in
/// shell.c:264,274).
pub const LONG_OPTIONS_WITH_VALUE: &[&str] = &["init-file", "rcfile"];

/// One GNU short invocation option (`shell.c` parse_shell_options):
/// the accepted letter plus the product-help copy. `value_name` is `Some`
/// for the letters that take the following argv word as their value
/// (`-c <command>`, `-o <option>`, `-O <shopt>` — shell.c:919-958), and
/// `long_alias` names an equivalent GNU long option when one exists
/// (`-l, --login`). The table lives in the lib so the product help is
/// generated from the same set the parser accepts instead of being
/// hand-maintained next to it (niubash#385).
pub struct ShellShortOption {
    pub letter: char,
    pub value_name: Option<&'static str>,
    pub long_alias: Option<&'static str>,
    pub description: &'static str,
}

/// The short options `parse_shell_options` accepts, in help order: the
/// invocation-only letters first (`-ilrsD or -c command or -O shopt_option`,
/// shell.c:2088), then the `set -o` single letters of the usage block
/// (`-abefhkmnptuvxBCEHPT or -o option`, shell.c:2095). This is the single
/// source of truth for `is_short_option_letter`, and therefore for the
/// accepted set in both the engine's argv parser and the niu launcher help.
pub const SHORT_OPTIONS: &[ShellShortOption] = &[
    ShellShortOption {
        letter: 'c',
        value_name: Some("command"),
        long_alias: None,
        description: "Execute the next non-option argument as a command",
    },
    ShellShortOption {
        letter: 'i',
        value_name: None,
        long_alias: None,
        description: "Force the interactive shell",
    },
    ShellShortOption {
        letter: 'l',
        value_name: None,
        long_alias: Some("--login"),
        description: "Act as a login shell",
    },
    ShellShortOption {
        letter: 'r',
        value_name: None,
        long_alias: Some("--restricted"),
        description: "Restricted shell (restricts cd and PATH/ENV changes)",
    },
    ShellShortOption {
        letter: 's',
        value_name: None,
        long_alias: None,
        description: "Read commands from standard input",
    },
    ShellShortOption {
        letter: 'D',
        value_name: None,
        long_alias: None,
        description: "Dump locale-translatable strings and exit",
    },
    ShellShortOption {
        letter: 'o',
        value_name: Some("option"),
        long_alias: None,
        description: "Set a `set -o` option by name",
    },
    ShellShortOption {
        letter: 'O',
        value_name: Some("shopt"),
        long_alias: None,
        description: "Set a `shopt` option by name",
    },
    ShellShortOption {
        letter: 'a',
        value_name: None,
        long_alias: None,
        description: "allexport: export all new and modified variables",
    },
    ShellShortOption {
        letter: 'b',
        value_name: None,
        long_alias: None,
        description: "notify: report the status of terminated background jobs",
    },
    ShellShortOption {
        letter: 'e',
        value_name: None,
        long_alias: None,
        description: "errexit: exit immediately if a command fails",
    },
    ShellShortOption {
        letter: 'f',
        value_name: None,
        long_alias: None,
        description: "noglob: disable pathname expansion",
    },
    ShellShortOption {
        letter: 'h',
        value_name: None,
        long_alias: None,
        description: "hashall: remember the location of executed commands",
    },
    ShellShortOption {
        letter: 'k',
        value_name: None,
        long_alias: None,
        description: "keyword: keep keyword=value arguments in the environment",
    },
    ShellShortOption {
        letter: 'm',
        value_name: None,
        long_alias: None,
        description: "monitor: enable job control",
    },
    ShellShortOption {
        letter: 'n',
        value_name: None,
        long_alias: None,
        description: "noexec: read commands without executing them",
    },
    ShellShortOption {
        letter: 'p',
        value_name: None,
        long_alias: None,
        description: "privileged: do not reset the effective uid",
    },
    ShellShortOption {
        letter: 't',
        value_name: None,
        long_alias: None,
        description: "onecmd: exit after reading and executing one command",
    },
    ShellShortOption {
        letter: 'u',
        value_name: None,
        long_alias: None,
        description: "nounset: error when expanding an unset variable",
    },
    ShellShortOption {
        letter: 'v',
        value_name: None,
        long_alias: Some("--verbose"),
        description: "verbose: echo input lines as they are read",
    },
    ShellShortOption {
        letter: 'x',
        value_name: None,
        long_alias: None,
        description: "xtrace: trace expanded commands",
    },
    ShellShortOption {
        letter: 'B',
        value_name: None,
        long_alias: None,
        description: "braceexpand: enable brace expansion",
    },
    ShellShortOption {
        letter: 'C',
        value_name: None,
        long_alias: None,
        description: "noclobber: refuse to overwrite existing files with >",
    },
    ShellShortOption {
        letter: 'E',
        value_name: None,
        long_alias: None,
        description: "errtrace: inherit the ERR trap in functions and subshells",
    },
    ShellShortOption {
        letter: 'H',
        value_name: None,
        long_alias: None,
        description: "histexpand: enable ! history expansion",
    },
    ShellShortOption {
        letter: 'P',
        value_name: None,
        long_alias: None,
        description: "physical: do not resolve symlinks in cd",
    },
    ShellShortOption {
        letter: 'T',
        value_name: None,
        long_alias: None,
        description: "functrace: inherit DEBUG and RETURN traps",
    },
];

/// Whether `flag` is a short invocation option GNU `parse_shell_options`
/// accepts. Derived from `SHORT_OPTIONS`, so the parser's accepted set and
/// the product help cannot drift apart.
pub fn is_short_option_letter(flag: char) -> bool {
    SHORT_OPTIONS.iter().any(|option| option.letter == flag)
}

/// The usage-contract shell name for this invocation, when `--help` must
/// print the GNU usage form (issue #585). GNU keys the fork off argv[0]'s
/// basename (shell.c set_shell_name): the `.exe` spelling is a Windows
/// artifact and never reaches the usage lines, the `-` login marker is
/// KEPT in the display name (empirical 5.3.0 oracle: `exec -a -bash bash
/// --help` prints `Usage:\t-bash ...`), and the bash-vs-sh decision uses
/// the dash-stripped lowercased stem so `-BASH.EXE` still forks. Any other
/// name (rubash, niu, ...) returns None and keeps the product help.
pub fn invocation_help_name(argv0: Option<&str>) -> Option<String> {
    let basename = argv0?.rsplit(['/', '\\']).next()?;
    let stem = if basename.len() >= 4 && basename.to_ascii_lowercase().ends_with(".exe") {
        &basename[..basename.len() - 4]
    } else {
        basename
    };
    let decided = stem.strip_prefix('-').unwrap_or(stem);
    match decided.to_ascii_lowercase().as_str() {
        "bash" | "sh" => Some(stem.to_string()),
        _ => None,
    }
}

/// GNU show_shell_usage (shell.c:2057-2103) with extra=1 — the `--help`
/// form, issue #585: printed to STDOUT and exited EXECUTION_SUCCESS
/// (shell.c:477-482), unlike the usage-error form above (stderr, no
/// version/extra block). The version line uses the engine's own version
/// string and machtype in GNU's `version %s-(%s)` shape
/// (shell.c:2071-2072 `_("GNU bash, version %s-(%s)\n")`); the long-option
/// table is the engine's accepted LONG_OPTIONS set, and the Shell options
/// block matches the usage-error contract byte for byte. The bash/sh names
/// in the usage and `Type ...' lines follow the invocation (shell_name,
/// shell.c:2084-2088, 2094-2095); the home-page lines keep GNU's literal
/// `bash home page` text. No Windows-only resources are referenced: the
/// same strings render in the unix cross-compile lanes.
///
/// `version` is the caller's runtime `$BASH_VERSION` when it has an
/// executor; `None` (embedders without one, e.g. the niu launcher help
/// path) falls back to the engine's advertised version value — the same
/// literal `init.rs` seeds the environment with.
pub fn show_shell_help(shell_name: &str, version: Option<&str>) {
    let version = version
        .map(str::to_string)
        .unwrap_or_else(crate::executor::bash_version_value);
    println!(
        "GNU bash, version {version}-({})",
        crate::executor::machtype_value()
    );
    println!("Usage:\t{shell_name} [GNU long option] [option] ...");
    println!("\t{shell_name} [GNU long option] [option] script-file ...");
    println!("GNU long options:");
    for name in LONG_OPTIONS {
        println!("	--{name}");
    }
    println!("Shell options:");
    println!("	-ilrsD or -c command or -O shopt_option		(invocation only)");
    println!("	-abefhkmnptuvxBCEHPT or -o option");
    println!("Type `{shell_name} -c \"help set\"' for more information about shell options.");
    println!("Type `{shell_name} -c help' for more information about shell builtin commands.");
    println!("Use the `bashbug' command to report bugs.");
    println!();
    println!("bash home page: <http://www.gnu.org/software/bash>");
    println!("General help using GNU software: <http://www.gnu.org/gethelp/>");
}

/// GNU show_shell_version (version.c:88-104): the version banner, the FSF
/// copyright/license block, then rubash#488's build fingerprint appendix —
/// everything before the appendix stays byte-identical to the bash 5.3
/// contract the upstream .right assets gate on. `version` is the caller's
/// runtime `$BASH_VERSION`; `None` falls back to the advertised value.
pub fn show_shell_version(version: Option<&str>) {
    let version = version
        .map(str::to_string)
        .unwrap_or_else(crate::executor::bash_version_value);
    println!(
        "GNU bash, version {version} ({})",
        crate::executor::machtype_value()
    );
    println!("Copyright (C) 2025 Free Software Foundation, Inc.");
    println!("License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>");
    println!();
    println!("This is free software; you are free to change and redistribute it.");
    println!("There is NO WARRANTY, to the extent permitted by law.");
    println!("{}", crate::build_info::version_appendix());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_option_table_covers_exactly_the_parser_letters() {
        // The accepted set of shell.c parse_shell_options: the invocation-only
        // letters `-ilrsD or -c command or -O shopt_option` plus the
        // single-letter `set -o` flags `-abefhkmnptuvxBCEHPT`. `-e`, `-o`,
        // `-u`, `-x` (rubash#382 / niubash#385) must be in it.
        let mut expected: Vec<char> = "cilrsDoOabefhkmnptuvxBCEHPT".chars().collect();
        let mut actual: Vec<char> = SHORT_OPTIONS.iter().map(|option| option.letter).collect();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(
            actual, expected,
            "short-option table drifted from the parser set"
        );
        for letter in expected {
            assert!(is_short_option_letter(letter), "missing {letter}");
        }
        assert!(!is_short_option_letter('Z'));
        assert!(!is_short_option_letter('d'));
        assert!(!is_short_option_letter('q'));
    }

    #[test]
    fn value_taking_short_options_name_their_argument() {
        for (letter, name) in [('c', "command"), ('o', "option"), ('O', "shopt")] {
            let option = SHORT_OPTIONS
                .iter()
                .find(|option| option.letter == letter)
                .expect("value-taking option present");
            assert_eq!(option.value_name, Some(name));
        }
        let login = SHORT_OPTIONS
            .iter()
            .find(|option| option.letter == 'l')
            .expect("login option present");
        assert_eq!(login.long_alias, Some("--login"));
    }
}
