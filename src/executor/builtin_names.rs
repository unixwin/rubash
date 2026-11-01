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
            | "[["
            | "]]"
            | "coproc"
    )
}

/// Names rubash treats as real shell builtins. Kept sorted; see
/// `docs/builtins.md` for the authoritative inventory (the doc-sync test
/// below fails when this table and the doc drift apart).
///
/// Not listed here on purpose: the fast-path names `sleep`, `dirname`,
/// `basename` and the hidden identity/console tools `uname`, `arch`,
/// `stty` — they are not builtins, so the builtin-table faces
/// (compgen -b, enable) exclude them exactly as GNU's table excludes
/// them; since #2036 S2 the describe faces (type / command -v)
/// disclose their in-process claims explicitly instead of reporting
/// them as plain files. `env` IS listed below: it is rubash's one
/// extension builtin (docs/builtins.md section 2).
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

/// The canonical, sorted list of shell builtin command names. Host completion
/// (and the `compgen -b` action) read builtins from here instead of
/// hardcoding their own copy, so the lists never drift when builtins change.
pub fn builtin_names() -> &'static [&'static str] {
    BUILTIN_NAMES
}

pub(in crate::executor) fn is_shell_builtin_name(name: &str) -> bool {
    // The compiled table plus process-global `enable -f` builtins.  GNU
    // keeps both in shell_builtins, so tempenv binding and command
    // resolution treat a loadable builtin exactly like a static one.
    BUILTIN_NAMES.binary_search(&name).is_ok() || crate::builtins::dynamic::is_loaded(name)
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
                 fast-path names must stay out of the whitelist because they are not builtins — \
                 the builtin-table faces (compgen -b, enable) must not list them (matching GNU's \
                 table); their in-process claims are disclosed by the describe faces instead \
                 (type / command -v, #2036 S2)"
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
