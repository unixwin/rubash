use super::*;

pub(in crate::executor) fn format_redirect(operator: &str, redirect: &Redirect) -> String {
    // fd-dup/close targets are stored as `&N`/`&-`/`&N-` (redirections.rs
    // redirect_target); GNU print_cmd fuses them: `2>&1`, `>&-`, `<&5-`.
    if let Some(fd_target) = redirect.target.strip_prefix('&') {
        let op = operator
            .trim_start_matches(|ch: char| ch.is_ascii_digit())
            .trim_end_matches('&');
        return match redirect.fd {
            Some(fd) => format!("{fd}{op}&{fd_target}"),
            None => format!("{op}&{fd_target}"),
        };
    }
    match redirect.fd {
        Some(_) if operator.starts_with(char::is_numeric) => {
            format!("{operator} {}", redirect.target)
        }
        Some(fd) => format!("{fd}{operator} {}", redirect.target),
        None => format!("{operator} {}", redirect.target),
    }
}

pub(in crate::executor) fn is_shell_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "case"
            | "esac"
            | "for"
            | "select"
            | "while"
            | "until"
            | "do"
            | "done"
            | "in"
            | "function"
            | "time"
            | "{"
            | "}"
            | "!"
    )
}

/// Names rubash treats as real shell builtins. Kept sorted; see
/// `docs/builtins.md` for the authoritative inventory (the doc-sync test
/// below fails when this table and the doc drift apart).
///
/// Not listed here on purpose (hidden fast-path builtins, which GNU Bash
/// also reports as external commands): `env`, `sleep`, `dirname`,
/// `basename` — see docs/builtins.md section 3.
const BUILTIN_NAMES: &[&str] = &[
    ".",
    ":",
    "[",
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "env",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "setopt",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "unsetopt",
    "wait",
];

/// The canonical, sorted list of shell builtin command names. Host completion
/// (and the `compgen -b` action) read builtins from here instead of
/// hardcoding their own copy, so the lists never drift when builtins change.
pub fn builtin_names() -> &'static [&'static str] {
    BUILTIN_NAMES
}

pub(in crate::executor) fn is_shell_builtin_name(name: &str) -> bool {
    BUILTIN_NAMES.binary_search(&name).is_ok() || (cfg!(windows) && name == "sudo")
}

#[cfg(test)]
mod builtin_doc_sync {
    use super::*;

    const DOC: &str = include_str!("../../docs/builtins.md");

    /// GNU Bash 5.2 builtin set (`enable` output). Every one of these must
    /// stay recognized by rubash — this is the coverage floor.
    const BASH_61: &[&str] = &[
        ".",
        ":",
        "[",
        "alias",
        "bg",
        "bind",
        "break",
        "builtin",
        "caller",
        "cd",
        "command",
        "compgen",
        "complete",
        "compopt",
        "continue",
        "declare",
        "dirs",
        "disown",
        "echo",
        "enable",
        "eval",
        "exec",
        "exit",
        "export",
        "false",
        "fc",
        "fg",
        "getopts",
        "hash",
        "help",
        "history",
        "jobs",
        "kill",
        "let",
        "local",
        "logout",
        "mapfile",
        "popd",
        "printf",
        "pushd",
        "pwd",
        "read",
        "readarray",
        "readonly",
        "return",
        "set",
        "shift",
        "shopt",
        "source",
        "suspend",
        "test",
        "times",
        "trap",
        "true",
        "type",
        "typeset",
        "ulimit",
        "umask",
        "unalias",
        "unset",
        "wait",
    ];

    fn machine_list(marker: &str) -> Vec<String> {
        let start = DOC
            .find(&format!("<!-- {marker}\n"))
            .unwrap_or_else(|| panic!("docs/builtins.md: missing <!-- {marker} block"));
        let body = &DOC[start..];
        let end = body
            .find("-->")
            .unwrap_or_else(|| panic!("docs/builtins.md: unterminated {marker} block"));
        // "<!-- " (5) + marker + "\n" (1) = content starts at marker.len() + 6.
        body[marker.len() + 6..end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn builtin_names_table_is_sorted() {
        // binary_search correctness depends on sortedness.
        assert!(BUILTIN_NAMES.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn doc_builtin_list_matches_whitelist() {
        let mut doc_names = machine_list("builtins-list");
        doc_names.sort();
        let mut table: Vec<String> = BUILTIN_NAMES.iter().map(|s| s.to_string()).collect();
        table.sort();
        assert_eq!(
            table, doc_names,
            "docs/builtins.md builtins-list is out of sync with BUILTIN_NAMES in builtin_names.rs"
        );
    }

    #[test]
    fn all_gnu_bash_builtins_are_recognized() {
        for name in BASH_61 {
            assert!(
                BUILTIN_NAMES.contains(name),
                "GNU bash builtin `{name}` is missing from BUILTIN_NAMES"
            );
        }
    }

    #[test]
    fn doc_fastpath_list_matches_hidden_builtins() {
        for name in machine_list("fastpath-list") {
            assert!(
                !BUILTIN_NAMES.contains(&name.as_str()),
                "`{name}` is listed as fast-path in docs/builtins.md but also present in BUILTIN_NAMES; \
                 fast-path builtins must stay out of the whitelist so introspection keeps \
                 reporting them as external (matching GNU bash)"
            );
        }
    }

    /// Pipeline-stage arms matched by `execute_pipeline_stage_inner`
    /// (`pipeline_exec.rs`, `match name`). Frozen set: these arms are inline
    /// match branches, not table entries the test can derive, so the doc list
    /// is compared against this constant and the test fails on any arm
    /// addition, removal or rename until `docs/builtins.md` §3.1 is updated.
    #[test]
    fn doc_pipeline_fastpath_list_matches_pipeline_arms() {
        let mut expected: Vec<String> = [
            "true", ":", "false", "echo", "printf", "trap", "head", "cat", "sed", "grep", "wc",
            "tr",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        expected.sort();

        let mut doc_names = machine_list("pipeline-fastpath-list");
        doc_names.sort();
        assert_eq!(
            expected, doc_names,
            "docs/builtins.md pipeline-fastpath-list is out of sync with the \
             pipeline-stage arms in execute_pipeline_stage_inner (pipeline_exec.rs, `match name`)"
        );
    }

    /// Command-substitution filter arms matched by
    /// `command_substitution_pipeline_filter` (`command_substitution_values.rs`).
    /// Same frozen-set rationale as the pipeline list. Note `sort`/`tail`/`uniq`
    /// exist only here while `sed`/`tr`/`head`/`grep`/`wc` also have pipeline
    /// arms — the overlap is expected and asserted only against this list.
    #[test]
    fn doc_substitution_fastpath_list_matches_substitution_filter_arms() {
        let mut expected: Vec<String> = ["sort", "sed", "tr", "head", "grep", "wc", "tail", "uniq"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        expected.sort();

        let mut doc_names = machine_list("substitution-fastpath-list");
        doc_names.sort();
        assert_eq!(
            expected, doc_names,
            "docs/builtins.md substitution-fastpath-list is out of sync with the \
             filter arms in command_substitution_pipeline_filter \
             (command_substitution_values.rs)"
        );
    }

    #[test]
    fn whitelist_never_leaks_fast_path_names() {
        for name in ["dirname", "basename", "sleep"] {
            assert!(
                !is_shell_builtin_name(name),
                "`{name}` must not be a whitelist builtin; it is a fast-path hidden builtin"
            );
        }
    }
}
