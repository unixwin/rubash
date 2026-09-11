//! complete module.
//!
//! GNU Bash source ownership:
// - builtins/complete.def (complete/compgen/compopt, build_actions, print_*)
// - pcomplib.c (COMPSPEC, progcomp_* registry, COMPLETE_HASH_BUCKETS=512)
// - hashlib.c (FNV-1a hash_string, prepend-on-insert chains, hash_walk)

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};

use crate::executor::path::{shell_directory_entries, shell_path_entries};

use crate::builtins::alias::Alias;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const DISABLED_BUILTINS: &str = "__RUBASH_DISABLED_BUILTINS";
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const SERVICE_NAMES: &str = "__RUBASH_SERVICE_NAMES";
const SHELL_BUILTINS: &[&str] = &[
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
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "select", "while", "until", "do",
    "done", "in", "function", "time", "{", "}", "!", "[[", "]]", "coproc",
];
const READLINE_BINDINGS: &[&str] = &[
    "abort",
    "accept-line",
    "arrow-key-prefix",
    "backward-byte",
    "backward-char",
    "backward-delete-char",
    "backward-kill-line",
    "backward-kill-word",
    "backward-word",
    "beginning-of-history",
    "beginning-of-line",
    "bracketed-paste-begin",
    "call-last-kbd-macro",
    "capitalize-word",
    "character-search",
    "character-search-backward",
    "clear-display",
    "clear-screen",
    "complete",
    "copy-backward-word",
    "copy-forward-word",
    "copy-region-as-kill",
    "delete-char",
    "delete-char-or-list",
    "delete-horizontal-space",
    "digit-argument",
    "do-lowercase-version",
    "downcase-word",
    "dump-functions",
    "dump-macros",
    "dump-variables",
    "emacs-editing-mode",
    "end-kbd-macro",
    "end-of-history",
    "end-of-line",
    "exchange-point-and-mark",
    "execute-named-command",
    "export-completions",
    "fetch-history",
    "forward-backward-delete-char",
    "forward-byte",
    "forward-char",
    "forward-search-history",
    "forward-word",
    "history-search-backward",
    "history-search-forward",
    "history-substring-search-backward",
    "history-substring-search-forward",
    "insert-comment",
    "insert-completions",
    "kill-line",
    "kill-region",
    "kill-whole-line",
    "kill-word",
    "menu-complete",
    "menu-complete-backward",
    "next-history",
    "next-screen-line",
    "non-incremental-forward-search-history",
    "non-incremental-forward-search-history-again",
    "non-incremental-reverse-search-history",
    "non-incremental-reverse-search-history-again",
    "old-menu-complete",
    "operate-and-get-next",
    "overwrite-mode",
    "paste-from-clipboard",
    "possible-completions",
    "previous-history",
    "previous-screen-line",
    "print-last-kbd-macro",
    "quoted-insert",
    "re-read-init-file",
    "redraw-current-line",
    "reverse-search-history",
    "revert-line",
    "self-insert",
    "set-mark",
    "skip-csi-sequence",
    "start-kbd-macro",
    "tab-insert",
    "tilde-expand",
    "transpose-chars",
    "transpose-words",
    "tty-status",
    "undo",
    "universal-argument",
    "unix-filename-rubout",
    "unix-line-discard",
    "unix-word-rubout",
    "upcase-word",
    "vi-append-eol",
    "vi-append-mode",
    "vi-arg-digit",
    "vi-back-to-indent",
    "vi-backward-bigword",
    "vi-backward-word",
    "vi-bWord",
    "vi-change-case",
    "vi-change-char",
    "vi-change-to",
    "vi-char-search",
    "vi-column",
    "vi-complete",
    "vi-delete",
    "vi-delete-to",
    "vi-editing-mode",
    "vi-end-bigword",
    "vi-end-word",
    "vi-eof-maybe",
    "vi-eWord",
    "vi-fetch-history",
    "vi-first-print",
    "vi-forward-bigword",
    "vi-forward-word",
    "vi-fword",
    "vi-goto-mark",
    "vi-insert-beg",
    "vi-insertion-mode",
    "vi-match",
    "vi-movement-mode",
    "vi-next-word",
    "vi-overstrike",
    "vi-overstrike-delete",
    "vi-prev-word",
    "vi-put",
    "vi-redo",
    "vi-replace",
    "vi-rubout",
    "vi-search",
    "vi-search-again",
    "vi-set-mark",
    "vi-subst",
    "vi-tilde-expand",
    "vi-undo",
    "vi-unix-word-rubout",
    "vi-yank-arg",
    "vi-yank-pop",
    "vi-yank-to",
    "yank",
    "yank-last-arg",
    "yank-nth-arg",
    "yank-pop",
];

// ---------------------------------------------------------------------------
// Completion actions/options model (complete.def compacts[] / compopts[]).
// ---------------------------------------------------------------------------

const CA_ALIAS: u64 = 1 << 0;
const CA_ARRAYVAR: u64 = 1 << 1;
const CA_BINDING: u64 = 1 << 2;
const CA_BUILTIN: u64 = 1 << 3;
const CA_COMMAND: u64 = 1 << 4;
const CA_DIRECTORY: u64 = 1 << 5;
const CA_DISABLED: u64 = 1 << 6;
const CA_ENABLED: u64 = 1 << 7;
const CA_EXPORT: u64 = 1 << 8;
const CA_FILE: u64 = 1 << 9;
const CA_FUNCTION: u64 = 1 << 10;
const CA_HELPTOPIC: u64 = 1 << 11;
const CA_HOSTNAME: u64 = 1 << 12;
const CA_GROUP: u64 = 1 << 13;
const CA_JOB: u64 = 1 << 14;
const CA_KEYWORD: u64 = 1 << 15;
const CA_RUNNING: u64 = 1 << 16;
const CA_SERVICE: u64 = 1 << 17;
const CA_SETOPT: u64 = 1 << 18;
const CA_SHOPT: u64 = 1 << 19;
const CA_SIGNAL: u64 = 1 << 20;
const CA_STOPPED: u64 = 1 << 21;
const CA_USER: u64 = 1 << 22;
const CA_VARIABLE: u64 = 1 << 23;

/// (action name, action flag, short option letter) in complete.def compacts[]
/// order. print_compactions prints the short forms first, then -A names, each
/// pass in this order.
const COMPACTS: &[(&str, u64, Option<char>)] = &[
    ("alias", CA_ALIAS, Some('a')),
    ("arrayvar", CA_ARRAYVAR, None),
    ("binding", CA_BINDING, None),
    ("builtin", CA_BUILTIN, Some('b')),
    ("command", CA_COMMAND, Some('c')),
    ("directory", CA_DIRECTORY, Some('d')),
    ("disabled", CA_DISABLED, None),
    ("enabled", CA_ENABLED, None),
    ("export", CA_EXPORT, Some('e')),
    ("file", CA_FILE, Some('f')),
    ("function", CA_FUNCTION, None),
    ("helptopic", CA_HELPTOPIC, None),
    ("hostname", CA_HOSTNAME, None),
    ("group", CA_GROUP, Some('g')),
    ("job", CA_JOB, Some('j')),
    ("keyword", CA_KEYWORD, Some('k')),
    ("running", CA_RUNNING, None),
    ("service", CA_SERVICE, Some('s')),
    ("setopt", CA_SETOPT, None),
    ("shopt", CA_SHOPT, None),
    ("signal", CA_SIGNAL, None),
    ("stopped", CA_STOPPED, None),
    ("user", CA_USER, Some('u')),
    ("variable", CA_VARIABLE, Some('v')),
];

const COPT_BASHDEFAULT: u64 = 1 << 0;
const COPT_DEFAULT: u64 = 1 << 1;
const COPT_DIRNAMES: u64 = 1 << 2;
const COPT_FILENAMES: u64 = 1 << 3;
const COPT_FULLQUOTE: u64 = 1 << 4;
const COPT_NOQUOTE: u64 = 1 << 5;
const COPT_NOSORT: u64 = 1 << 6;
const COPT_NOSPACE: u64 = 1 << 7;
const COPT_PLUSDIRS: u64 = 1 << 8;

/// (option name, flag) in complete.def compopts[] order.
const COMPOPTS: &[(&str, u64)] = &[
    ("bashdefault", COPT_BASHDEFAULT),
    ("default", COPT_DEFAULT),
    ("dirnames", COPT_DIRNAMES),
    ("filenames", COPT_FILENAMES),
    ("fullquote", COPT_FULLQUOTE),
    ("noquote", COPT_NOQUOTE),
    ("nosort", COPT_NOSORT),
    ("nospace", COPT_NOSPACE),
    ("plusdirs", COPT_PLUSDIRS),
];

// Candidate lists for the actions whose GNU sources are fixed tables.
// Helptopics are the live 5.3.0 help-topic table (includes the pseudo topics
// %, (( ... )), [[ ... ]], for ((, { ... }) and "variables"; excludes the bare
// reserved words do/done/elif/else/esac/fi/in/then). Setopt is the flags.c
// set -o table; shopt is the 5.3.0 shopt_vars table.
const HELP_TOPIC_COMPLETIONS: &[&str] = &[
    "!",
    "%",
    "(( ... ))",
    ".",
    ":",
    "[",
    "[[ ... ]]",
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "case",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "coproc",
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
    "for",
    "for ((",
    "function",
    "getopts",
    "hash",
    "help",
    "history",
    "if",
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
    "select",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "time",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "until",
    "variables",
    "wait",
    "while",
    "{ ... }",
];

const SETOPT_COMPLETIONS: &[&str] = &[
    "allexport",
    "braceexpand",
    "emacs",
    "errexit",
    "errtrace",
    "functrace",
    "hashall",
    "histexpand",
    "history",
    "ignoreeof",
    "interactive-comments",
    "keyword",
    "monitor",
    "noclobber",
    "noexec",
    "noglob",
    "nolog",
    "notify",
    "nounset",
    "onecmd",
    "physical",
    "pipefail",
    "posix",
    "privileged",
    "verbose",
    "vi",
    "xtrace",
];

const SHOPT_COMPLETIONS: &[&str] = &[
    "array_expand_once",
    "assoc_expand_once",
    "autocd",
    "bash_source_fullpath",
    "cdable_vars",
    "cdspell",
    "checkhash",
    "checkjobs",
    "checkwinsize",
    "cmdhist",
    "compat31",
    "compat32",
    "compat40",
    "compat41",
    "compat42",
    "compat43",
    "compat44",
    "complete_fullquote",
    "direxpand",
    "dirspell",
    "dotglob",
    "execfail",
    "expand_aliases",
    "extdebug",
    "extglob",
    "extquote",
    "failglob",
    "force_fignore",
    "globasciiranges",
    "globskipdots",
    "globstar",
    "gnu_errfmt",
    "histappend",
    "histreedit",
    "histverify",
    "hostcomplete",
    "huponexit",
    "inherit_errexit",
    "interactive_comments",
    "lastpipe",
    "lithist",
    "localvar_inherit",
    "localvar_unset",
    "login_shell",
    "mailwarn",
    "no_empty_cmd_completion",
    "nocaseglob",
    "nocasematch",
    "noexpand_translation",
    "nullglob",
    "patsub_replacement",
    "progcomp",
    "progcomp_alias",
    "promptvars",
    "restricted_shell",
    "shift_verbose",
    "sourcepath",
    "varredir_close",
    "xpg_echo",
];

/// Parsed completion specification (pcomplib.c COMPSPEC; complete.def
/// complete_builtin builds it at lines 468-478).
#[derive(Clone, Debug, Default)]
pub(crate) struct Compspec {
    pub(crate) actions: u64,
    pub(crate) options: u64,
    pub(crate) globpat: Option<String>,
    pub(crate) words: Option<String>,
    pub(crate) prefix: Option<String>,
    pub(crate) suffix: Option<String>,
    pub(crate) filterpat: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) funcname: Option<String>,
}

impl Compspec {
    pub(crate) fn from_parsed(p: &ParsedCompletionOptions) -> Self {
        Compspec {
            actions: p.actions,
            options: p.options,
            globpat: p.globpat.clone(),
            words: p.words.clone(),
            prefix: p.prefix.clone(),
            suffix: p.suffix.clone(),
            filterpat: p.filterpat.clone(),
            command: p.command.clone(),
            funcname: p.funcname.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Completion registry (pcomplib.c prog_completes over hashlib.c).
// ---------------------------------------------------------------------------

/// pcomplib.c: COMPLETE_HASH_BUCKETS 512 (must be power of two).
const COMPLETE_HASH_BUCKETS: usize = 512;
/// hashlib.c HASH_REHASH_MULTIPLIER / HASH_REHASH_FACTOR.
const HASH_REHASH_MULTIPLIER: usize = 4;
const HASH_REHASH_FACTOR: usize = 2;

/// Registration table reproducing bash's hash table so iteration order
/// (complete -p / bare complete) matches GNU exactly: buckets ascend
/// 0..nbuckets, each chain runs head (most recently inserted) to tail, and
/// re-registering an existing name replaces the value in place without moving
/// it (hashlib.c hash_insert). The created flag models progcomp_remove
/// succeeding while prog_completes is still NULL (before the first insertion,
/// surviving progcomp_flush).
#[derive(Debug)]
pub(crate) struct CompletionRegistry {
    buckets: Vec<Vec<(String, Compspec)>>,
    nentries: usize,
    created: bool,
}

impl CompletionRegistry {
    pub(crate) fn new() -> Self {
        CompletionRegistry {
            buckets: vec![Vec::new(); COMPLETE_HASH_BUCKETS],
            nentries: 0,
            created: false,
        }
    }

    /// hashlib.c hash_string: FNV-1a over the bytes; char is signed on the
    /// reference platform so bytes >= 0x80 sign-extend.
    fn hash_string(s: &str) -> u32 {
        const FNV_OFFSET: u32 = 2166136261;
        let mut i: u32 = FNV_OFFSET;
        for b in s.bytes() {
            i = i
                .wrapping_add(i << 1)
                .wrapping_add(i << 4)
                .wrapping_add(i << 7)
                .wrapping_add(i << 8)
                .wrapping_add(i << 24);
            i ^= (b as i8) as i32 as u32;
        }
        i
    }

    fn bucket_of(name: &str, nbuckets: usize) -> usize {
        (Self::hash_string(name) as usize) & (nbuckets - 1)
    }

    /// hashlib.c hash_rehash: walk old buckets ascending, chain head to tail,
    /// prepending each item into its new bucket.
    fn grow(&mut self) {
        let new_len = self.buckets.len() * HASH_REHASH_MULTIPLIER;
        let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); new_len]);
        for chain in old {
            for (name, spec) in chain {
                let b = Self::bucket_of(&name, self.buckets.len());
                self.buckets[b].insert(0, (name, spec));
            }
        }
    }

    pub(crate) fn insert(&mut self, name: &str, spec: Compspec) {
        self.created = true;
        if self.nentries >= self.buckets.len() * HASH_REHASH_FACTOR {
            self.grow();
        }
        let b = Self::bucket_of(name, self.buckets.len());
        let bucket = &mut self.buckets[b];
        if let Some(slot) = bucket.iter_mut().find(|(k, _)| k == name) {
            slot.1 = spec;
            return;
        }
        bucket.insert(0, (name.to_string(), spec));
        self.nentries += 1;
    }

    /// True when the name was removed (or the table was never created);
    /// false means "no completion specification" for this name.
    pub(crate) fn remove(&mut self, name: &str) -> bool {
        if !self.created {
            return true;
        }
        let b = Self::bucket_of(name, self.buckets.len());
        let before = self.buckets[b].len();
        self.buckets[b].retain(|(k, _)| k != name);
        if self.buckets[b].len() != before {
            self.nentries -= 1;
            true
        } else {
            false
        }
    }

    /// pcomplib.c progcomp_flush (complete -r with no names): the entries go
    /// away, the table itself stays allocated.
    pub(crate) fn flush(&mut self) {
        for chain in &mut self.buckets {
            chain.clear();
        }
        self.nentries = 0;
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Compspec> {
        if !self.created {
            return None;
        }
        let b = Self::bucket_of(name, self.buckets.len());
        self.buckets[b]
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }

    /// hashlib.c hash_walk order: bucket index ascending, chain head first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Compspec)> {
        self.buckets.iter().flatten().map(|(k, v)| (k, v))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionBuiltin {
    Complete,
    Compgen,
    Compopt,
}

pub fn execute_with_io<E>(
    builtin: CompletionBuiltin,
    args: &[String],
    env_vars: &HashMap<String, String>,
    aliases: &HashMap<String, Alias>,
    function_names: &[String],
    job_names: &[String],
    diagnostic_prefix: &str,
    stdout: &mut E,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    match builtin {
        // complete is handled at the executor level (registration registry,
        // -p print and -r remove need Executor state); this entry point only
        // serves compgen/compopt.
        CompletionBuiltin::Complete => Ok(EXECUTION_SUCCESS),
        CompletionBuiltin::Compgen => execute_compgen(
            args,
            env_vars,
            aliases,
            function_names,
            job_names,
            diagnostic_prefix,
            stdout,
            stderr,
        ),
        CompletionBuiltin::Compopt => execute_compopt(args, diagnostic_prefix, stderr),
    }
}

/// compgen_builtin (complete.def:669): with no arguments, success and no
/// Generate candidates for the action bitmask `actions` (the compgen
/// `-a`/`-b`/`-c`/`-d`/`-f`/... flags packed into a `u64`). Shared by
/// `execute_compgen` and the host `complete_line` hook so both behave
/// identically for the static action set.
pub(crate) fn apply_completion_actions(
    actions: u64,
    word: &str,
    env_vars: &HashMap<String, String>,
    aliases: &HashMap<String, Alias>,
    function_names: &[String],
    job_names: &[String],
) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for &(actname, actbit, _) in COMPACTS {
        if actions & actbit == 0 {
            continue;
        }
        match actname {
            "alias" => candidates.extend(alias_completion_candidates(aliases)),
            "arrayvar" => candidates.extend(array_variable_completion_candidates(env_vars)),
            "binding" => candidates.extend(READLINE_BINDINGS.iter().map(|s| s.to_string())),
            "builtin" => candidates.extend(SHELL_BUILTINS.iter().map(|s| s.to_string())),
            "command" => candidates.extend(command_completion_candidates(
                env_vars,
                aliases,
                function_names,
            )),
            "directory" => {
                candidates.extend(path_completion_candidates(
                    word,
                    PathCompletionKind::Directory,
                    env_vars,
                ));
            }
            "file" => {
                candidates.extend(path_completion_candidates(
                    word,
                    PathCompletionKind::File,
                    env_vars,
                ));
            }
            "disabled" => candidates.extend(disabled_builtin_completion_candidates(env_vars)),
            "enabled" => candidates.extend(enabled_builtin_completion_candidates(env_vars)),
            "export" => candidates.extend(exported_variable_completion_candidates(env_vars)),
            "helptopic" => candidates.extend(HELP_TOPIC_COMPLETIONS.iter().map(|s| s.to_string())),
            "hostname" => candidates.extend(hostname_completion_candidates(env_vars)),
            "function" => candidates.extend(function_completion_candidates(function_names)),
            "group" => candidates.extend(group_completion_candidates(env_vars)),
            "job" | "running" => candidates.extend(job_completion_candidates(job_names)),
            "keyword" => candidates.extend(SHELL_KEYWORDS.iter().map(|s| s.to_string())),
            "service" => candidates.extend(service_completion_candidates(env_vars)),
            "setopt" => candidates.extend(SETOPT_COMPLETIONS.iter().map(|s| s.to_string())),
            "shopt" => candidates.extend(SHOPT_COMPLETIONS.iter().map(|s| s.to_string())),
            "signal" => {
                candidates.extend(crate::builtins::trap::SIGNALS.iter().map(|s| s.to_string()))
            }
            "stopped" => {}
            "user" => candidates.extend(user_completion_candidates(env_vars)),
            "variable" => candidates.extend(variable_completion_candidates(env_vars)),
            _ => {}
        }
    }
    candidates
}

/// Split `line[..cursor]` into words for completion purposes, returning the
/// word list, the index of the word currently being completed, and that word's
/// partial text. A trailing separator means a fresh (empty) word is being
/// completed; an interior cursor completes the partial last word.
fn split_for_completion(line: &str, cursor: usize) -> (Vec<String>, usize, String) {
    let cursor = cursor.min(line.len());
    let prefix = &line[..cursor];
    let ends_with_ws = prefix
        .chars()
        .last()
        .map(|c| c.is_whitespace())
        .unwrap_or(true);
    let words: Vec<String> = prefix.split_whitespace().map(str::to_string).collect();
    let n = words.len();
    if ends_with_ws {
        (words, n, String::new())
    } else {
        let idx = words.len().saturating_sub(1);
        let cur = words.last().cloned().unwrap_or_default();
        (words, idx, cur)
    }
}

/// Host completion hook. Given the in-progress command `line` and the `cursor`
/// position, return completion candidates for the word under the cursor,
/// honoring the compspec registered for the command (if any) and falling back
/// to command completion (first word) or file completion (later words).
///
/// This is the entry point niubash's interactive completer delegates to, so the
/// GNU programmable-completion engine (compspec + compgen) lives in rubash
/// while the UI stays in the host (mirroring how `HistoryProvider` keeps the
/// storage contract on the host).
///
/// Dynamic compspec actions `-C` (external command) and `-F` (shell function
/// filling `COMPREPLY`) are resolved by the executor; see
/// `Executor::complete_line`, which calls this with the registry and merges any
/// dynamic candidates.
pub(crate) fn complete_line_candidates(
    line: &str,
    cursor: usize,
    specs: &CompletionRegistry,
    env_vars: &HashMap<String, String>,
    aliases: &HashMap<String, Alias>,
    function_names: &[String],
    job_names: &[String],
) -> Vec<String> {
    let (words, cur_idx, cur_word) = split_for_completion(line, cursor);
    let command = words.first().cloned().unwrap_or_default();

    let mut candidates: Vec<String> = if let Some(cs) = specs.get(&command) {
        let mut c = Vec::new();
        if cs.actions != 0 {
            c.extend(apply_completion_actions(
                cs.actions,
                &cur_word,
                env_vars,
                aliases,
                function_names,
                job_names,
            ));
        }
        if let Some(globpat) = cs.globpat.as_deref() {
            if let crate::executor::glob::PathnameExpansion::Matches(matches) =
                crate::executor::glob::pathname_expand_word(globpat, env_vars)
            {
                c.extend(matches);
            }
        }
        if let Some(wordlist) = cs.words.as_deref() {
            c.extend(wordlist.split_whitespace().map(str::to_string));
        }
        // -C command / -F function candidates are contributed by the executor.
        c
    } else if cur_idx == 0 {
        command_completion_candidates(env_vars, aliases, function_names)
    } else {
        path_completion_candidates(&cur_word, PathCompletionKind::File, env_vars)
    };

    // Apply the compspec prefix/suffix and the -X filter, then keep only
    // candidates that extend the partial word (same rule compgen uses).
    let (prefix, suffix, filterpat) = if let Some(cs) = specs.get(&command) {
        (cs.prefix.clone(), cs.suffix.clone(), cs.filterpat.clone())
    } else {
        (None, None, None)
    };
    if let Some(filter) = filterpat.as_deref() {
        let keep = |candidate: &str| -> bool {
            if let Some(pattern) = filter.strip_prefix('!') {
                !crate::executor::conditional::shell_pattern_matches(pattern, candidate)
            } else {
                crate::executor::conditional::shell_pattern_matches(filter, candidate)
            }
        };
        candidates.retain(|c| !keep(c));
    }
    candidates.retain(|c| c.starts_with(&cur_word));
    if let (Some(p), Some(s)) = (prefix, suffix) {
        candidates = candidates
            .into_iter()
            .map(|c| format!("{p}{c}{s}"))
            .collect();
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

/// output; otherwise generate candidates from every set action (plus the -G
/// globpat and -W wordlist), filter by the word prefix and the -X filter
/// pattern, then print them (or store them into the -V array at the executor
/// level, which reads them from the stdout buffer).
fn execute_compgen<E>(
    args: &[String],
    env_vars: &HashMap<String, String>,
    aliases: &HashMap<String, Alias>,
    function_names: &[String],
    job_names: &[String],
    diagnostic_prefix: &str,
    stdout: &mut E,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    if args.is_empty() {
        return Ok(EXECUTION_SUCCESS);
    }

    let parsed = match parse_completion_options(
        CompletionBuiltin::Compgen,
        args,
        diagnostic_prefix,
        stderr,
    )? {
        Err(status) => return Ok(status),
        Ok(parsed) => parsed,
    };

    let mut candidates = apply_completion_actions(
        parsed.actions,
        parsed.word(),
        env_vars,
        aliases,
        function_names,
        job_names,
    );

    if let Some(glob_pattern) = parsed.globpat.as_deref() {
        if let crate::executor::glob::PathnameExpansion::Matches(matches) =
            crate::executor::glob::pathname_expand_word(glob_pattern, env_vars)
        {
            candidates.extend(matches);
        }
    }

    if let Some(wordlist) = parsed.words.as_deref() {
        candidates.extend(wordlist.split_whitespace().map(str::to_string));
    }

    write_compgen_matches(candidates.iter().map(String::as_str), &parsed, stdout)
}

/// The -V varname of a compgen invocation, derived with the same option scan
/// as parse_completion_options (the identifier itself is validated there).
pub(crate) fn compgen_varname(args: &[String]) -> Option<String> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            return None;
        }
        if !arg.starts_with('-') || arg == "-" {
            return None;
        }
        let mut chars = arg[1..].chars().peekable();
        while let Some(option) = chars.next() {
            if CompletionBuiltin::Compgen.flag_options().contains(option) {
                continue;
            }
            if CompletionBuiltin::Compgen.arg_options().contains(option) {
                let inline = chars.peek().is_some();
                let value = if inline {
                    chars.collect::<String>()
                } else {
                    index += 1;
                    args.get(index)?.clone()
                };
                if option == 'V' {
                    return Some(value);
                }
                break;
            }
            return None;
        }
        index += 1;
    }
    None
}

#[derive(Clone, Copy)]
enum PathCompletionKind {
    Directory,
    File,
}

fn path_completion_candidates(
    word: &str,
    kind: PathCompletionKind,
    env_vars: &HashMap<String, String>,
) -> Vec<String> {
    let (search_dir, display_prefix) = path_completion_base(word);
    let Ok(entries) = shell_directory_entries(search_dir, env_vars) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries {
        if matches!(kind, PathCompletionKind::Directory) && !entry.is_dir {
            continue;
        }
        candidates.push(format!("{display_prefix}{}", entry.name));
    }
    candidates.sort();
    candidates
}

fn path_completion_base(word: &str) -> (&str, &str) {
    let Some(separator_index) = word.rfind(['/', '\\']) else {
        return (".", "");
    };
    if separator_index == 0 {
        return (&word[..1], &word[..1]);
    }
    (&word[..separator_index], &word[..=separator_index])
}

fn enabled_builtin_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let disabled = disabled_builtin_completion_candidates(env_vars);
    SHELL_BUILTINS
        .iter()
        .copied()
        .filter(|name| !disabled.iter().any(|disabled_name| disabled_name == name))
        .map(str::to_string)
        .collect()
}

fn disabled_builtin_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = env_vars
        .get(DISABLED_BUILTINS)
        .map(|value| {
            value
                .split(':')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn hostname_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .filter_map(|name| env_vars.get(*name))
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn user_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .filter_map(|name| env_vars.get(*name))
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn group_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = ["GROUP", "GROUPNAME", "USERDOMAIN"]
        .iter()
        .filter_map(|name| env_vars.get(*name))
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn service_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = ["SERVICE", "SERVICENAME"]
        .iter()
        .filter_map(|name| env_vars.get(*name))
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    candidates.extend(marked_completion_names(env_vars, SERVICE_NAMES));
    candidates.sort();
    candidates.dedup();
    candidates
}

fn variable_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates: Vec<String> = env_vars.keys().cloned().collect();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn array_variable_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    candidates.extend(marked_completion_names(env_vars, ARRAY_VARS));
    candidates.extend(marked_completion_names(env_vars, ASSOC_VARS));
    candidates.into_iter().collect()
}

fn exported_variable_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut candidates = marked_completion_names(env_vars, EXPORTED_VARS);
    candidates.sort();
    candidates.dedup();
    candidates
}

fn marked_completion_names(env_vars: &HashMap<String, String>, key: &str) -> Vec<String> {
    env_vars
        .get(key)
        .map(|value| {
            value
                .split('\x1f')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn alias_completion_candidates(aliases: &HashMap<String, Alias>) -> Vec<String> {
    let mut candidates: Vec<String> = aliases.keys().cloned().collect();
    candidates.sort();
    candidates
}

fn function_completion_candidates(function_names: &[String]) -> Vec<String> {
    let mut candidates = function_names.to_vec();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn job_completion_candidates(job_names: &[String]) -> Vec<String> {
    let mut candidates = job_names.to_vec();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn command_completion_candidates(
    env_vars: &HashMap<String, String>,
    aliases: &HashMap<String, Alias>,
    function_names: &[String],
) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    candidates.extend(SHELL_BUILTINS.iter().map(|name| (*name).to_string()));
    candidates.extend(SHELL_KEYWORDS.iter().map(|name| (*name).to_string()));
    candidates.extend(aliases.keys().cloned());
    candidates.extend(function_names.iter().cloned());
    candidates.extend(path_command_completion_candidates(env_vars));
    candidates.into_iter().collect()
}

fn path_command_completion_candidates(env_vars: &HashMap<String, String>) -> Vec<String> {
    let Some(path_value) = env_vars.get("PATH") else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for dir in shell_path_entries(path_value) {
        let Ok(entries) = shell_directory_entries(&dir, env_vars) else {
            continue;
        };
        for entry in entries {
            if entry.is_file {
                candidates.push(entry.name);
            }
        }
    }
    candidates
}

fn write_compgen_matches<'a, I, E>(
    candidates: I,
    parsed: &ParsedCompletionOptions,
    stdout: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    E: Write,
{
    let word = parsed.word();
    let mut kept = 0usize;
    for candidate in candidates {
        if candidate.starts_with(word) {
            if parsed.filter_excludes(candidate) {
                continue;
            }
            kept += 1;
            writeln!(
                stdout,
                "{}{}{}",
                parsed.prefix.as_deref().unwrap_or_default(),
                candidate,
                parsed.suffix.as_deref().unwrap_or_default()
            )?;
        }
    }
    // compgen_builtin: rval is EXECUTION_SUCCESS only when the final list is
    // non-empty (complete.def:762-777).
    Ok(if kept > 0 {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    })
}

fn execute_compopt<E>(args: &[String], diagnostic_prefix: &str, stderr: &mut E) -> io::Result<i32>
where
    E: Write,
{
    let status = parse_compopt_options(args, diagnostic_prefix, stderr)?;
    if status != EXECUTION_SUCCESS {
        return Ok(status);
    }

    writeln!(
        stderr,
        "{diagnostic_prefix}compopt: not currently executing completion function"
    )?;
    Ok(EXECUTION_FAILURE)
}

/// Parse complete/compgen arguments the way complete.def build_actions does
/// (internal_getopt over "abcdefgjko:prsuvA:G:W:P:S:X:F:C:V:DEI" with the
/// builtin-specific validity rules). Ok(parsed) on success; Err(status) with
/// diagnostics already written to stderr on the EX_USAGE paths.
pub(crate) fn parse_completion_options<E>(
    builtin: CompletionBuiltin,
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<Result<ParsedCompletionOptions, i32>>
where
    E: Write,
{
    let name = builtin.name();
    let flag_options = builtin.flag_options();
    let arg_options = builtin.arg_options();
    let mut index = 0;
    let mut parsed = ParsedCompletionOptions::default();
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        let mut chars = arg[1..].chars().peekable();
        while let Some(option) = chars.next() {
            if flag_options.contains(option) {
                parsed.opt_given = true;
                match option {
                    'p' => parsed.pflag = true,
                    'r' => parsed.rflag = true,
                    'D' => parsed.dflag = true,
                    'E' => parsed.eflag = true,
                    'I' => parsed.iflag = true,
                    // Short action letter: OR the action bit in (build_actions
                    // cases a/b/c/d/e/f/g/j/k/s/u/v).
                    _ => {
                        if let Some(bit) =
                            COMPACTS.iter().find(|c| c.2 == Some(option)).map(|c| c.1)
                        {
                            parsed.actions |= bit;
                        }
                    }
                }
                continue;
            }
            if arg_options.contains(option) {
                parsed.opt_given = true;
                let inline_arg = chars.peek().is_some();
                let value = if inline_arg {
                    chars.collect::<String>()
                } else {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}{name}: -{option}: option requires an argument"
                        )?;
                        write_usage(builtin, stderr)?;
                        return Ok(Err(EX_USAGE));
                    };
                    value.clone()
                };
                match option {
                    'o' => match find_compopt(&value) {
                        Some(bit) => parsed.options |= bit,
                        // complete.def:276-278: sh_invalidoptname, no usage.
                        None => {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}{name}: {value}: invalid option name"
                            )?;
                            return Ok(Err(EX_USAGE));
                        }
                    },
                    'A' => match find_compact(&value) {
                        Some(bit) => parsed.actions |= bit,
                        // complete.def:285: builtin_error, no usage line.
                        None => {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}{name}: {value}: invalid action name"
                            )?;
                            return Ok(Err(EX_USAGE));
                        }
                    },
                    'G' => parsed.globpat = Some(value),
                    'W' => parsed.words = Some(value),
                    'P' => parsed.prefix = Some(value),
                    'S' => parsed.suffix = Some(value),
                    'X' => parsed.filterpat = Some(value),
                    'C' => parsed.command = Some(value),
                    // complete.def:329-337: -F argument must be an identifier.
                    'F' => {
                        if !valid_identifier(&value) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}{name}: `{value}': not a valid identifier"
                            )?;
                            return Ok(Err(EX_USAGE));
                        }
                        parsed.funcname = Some(value);
                    }
                    // complete.def:347-365: -V only when a varname out-param
                    // exists (compgen); the argument must be an identifier.
                    'V' => {
                        if !valid_identifier(&value) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}{name}: `{value}': not a valid identifier"
                            )?;
                            return Ok(Err(EX_USAGE));
                        }
                        parsed.varname = Some(value);
                    }
                    _ => {}
                }
                break;
            }

            writeln!(
                stderr,
                "{diagnostic_prefix}{name}: -{option}: invalid option"
            )?;
            write_usage(builtin, stderr)?;
            return Ok(Err(EX_USAGE));
        }
        index += 1;
    }

    parsed.operands = args[index..].to_vec();
    Ok(Ok(parsed))
}

fn find_compact(name: &str) -> Option<u64> {
    COMPACTS.iter().find(|c| c.0 == name).map(|c| c.1)
}

fn find_compopt(name: &str) -> Option<u64> {
    COMPOPTS.iter().find(|c| c.0 == name).map(|c| c.1)
}

/// bash valid_identifier: [A-Za-z_][A-Za-z0-9_]*.
fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Result of parsing complete/compgen words (complete.def build_actions).
#[derive(Default)]
pub(crate) struct ParsedCompletionOptions {
    pub(crate) actions: u64,
    pub(crate) options: u64,
    pub(crate) globpat: Option<String>,
    pub(crate) words: Option<String>,
    pub(crate) prefix: Option<String>,
    pub(crate) suffix: Option<String>,
    pub(crate) filterpat: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) funcname: Option<String>,
    pub(crate) varname: Option<String>,
    pub(crate) pflag: bool,
    pub(crate) rflag: bool,
    pub(crate) dflag: bool,
    pub(crate) eflag: bool,
    pub(crate) iflag: bool,
    pub(crate) opt_given: bool,
    pub(crate) operands: Vec<String>,
}

impl ParsedCompletionOptions {
    fn word(&self) -> &str {
        self.operands
            .first()
            .map(String::as_str)
            .unwrap_or_default()
    }

    fn filter_excludes(&self, candidate: &str) -> bool {
        let Some(filter_pattern) = self.filterpat.as_deref() else {
            return false;
        };
        if let Some(pattern) = filter_pattern.strip_prefix('!') {
            !crate::executor::conditional::shell_pattern_matches(pattern, candidate)
        } else {
            crate::executor::conditional::shell_pattern_matches(filter_pattern, candidate)
        }
    }
}

fn parse_compopt_options<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if !arg.starts_with('-') && !arg.starts_with("+o") || arg == "-" {
            break;
        }

        if let Some(rest) = arg.strip_prefix("+o") {
            if rest.is_empty() {
                index += 1;
                if args.get(index).is_none() {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}compopt: +o: option requires an argument"
                    )?;
                    write_usage(CompletionBuiltin::Compopt, stderr)?;
                    return Ok(EX_USAGE);
                }
            }
            index += 1;
            continue;
        }

        let mut chars = arg[1..].chars().peekable();
        while let Some(option) = chars.next() {
            match option {
                'D' | 'E' | 'I' => {}
                'o' => {
                    if chars.peek().is_none() {
                        index += 1;
                        let Some(option_name) = args.get(index) else {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}compopt: -o: option requires an argument"
                            )?;
                            write_usage(CompletionBuiltin::Compopt, stderr)?;
                            return Ok(EX_USAGE);
                        };
                        // Validate option name (GNU complete.def compopts[])
                        if !matches!(
                            option_name.as_str(),
                            "bashdefault"
                                | "default"
                                | "dirnames"
                                | "filenames"
                                | "fullquote"
                                | "noquote"
                                | "nosort"
                                | "nospace"
                                | "plusdirs"
                        ) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}compopt: {option_name}: invalid option name"
                            )?;
                            return Ok(EX_USAGE);
                        }
                    }
                    break;
                }
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}compopt: -{other}: invalid option"
                    )?;
                    write_usage(CompletionBuiltin::Compopt, stderr)?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

impl CompletionBuiltin {
    fn name(self) -> &'static str {
        match self {
            CompletionBuiltin::Complete => "complete",
            CompletionBuiltin::Compgen => "compgen",
            CompletionBuiltin::Compopt => "compopt",
        }
    }

    /// build_actions internal_getopt option letters. -p/-r/-D/-E/-I exist
    /// for complete only (complete.def:205 "abcdefgjko:prsuvA:G:W:P:S:X:F:C:
    /// V:DEI" with p/r/D/E/I rejected when no optflags out-param exists, i.e.
    /// for compgen); -V takes an argument and is only valid for compgen.
    fn flag_options(self) -> &'static str {
        match self {
            CompletionBuiltin::Complete => "abcdefgjksuvprDEI",
            CompletionBuiltin::Compgen => "abcdefgjksuv",
            CompletionBuiltin::Compopt => "",
        }
    }

    fn arg_options(self) -> &'static str {
        match self {
            CompletionBuiltin::Complete => "oAGWFCXPS",
            CompletionBuiltin::Compgen => "oAGWFCXPSV",
            CompletionBuiltin::Compopt => "",
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical spec printing (complete.def print_one_completion:537-599).
// ---------------------------------------------------------------------------

/// print_compoptions: -o names in compopts[] order. print_compactions: short
/// flags first, then -A names, in compacts[] order. Then the quoted args
/// -G -W -P -S -X, -C, -F (quoted only when it contains shell metas), then
/// the command name.
pub(crate) fn print_compspec_line<E>(name: &str, cs: &Compspec, out: &mut E) -> io::Result<()>
where
    E: Write,
{
    write!(out, "complete ")?;
    for &(optname, bit) in COMPOPTS {
        if cs.options & bit != 0 {
            write!(out, "-o {optname} ")?;
        }
    }
    for &(_, bit, short) in COMPACTS {
        if let Some(c) = short {
            if cs.actions & bit != 0 {
                write!(out, "-{c} ")?;
            }
        }
    }
    for &(actname, bit, short) in COMPACTS {
        if short.is_none() && cs.actions & bit != 0 {
            write!(out, "-A {actname} ")?;
        }
    }
    print_arg(cs.globpat.as_deref(), "-G", true, out)?;
    print_arg(cs.words.as_deref(), "-W", true, out)?;
    print_arg(cs.prefix.as_deref(), "-P", true, out)?;
    print_arg(cs.suffix.as_deref(), "-S", true, out)?;
    print_arg(cs.filterpat.as_deref(), "-X", true, out)?;
    print_arg(cs.command.as_deref(), "-C", true, out)?;
    if let Some(funcname) = cs.funcname.as_deref() {
        print_arg(Some(funcname), "-F", sh_contains_shell_metas(funcname), out)?;
    }
    write!(out, "{}", print_cmd_name(name))?;
    writeln!(out)
}

/// complete.def print_arg: print "FLAG ARG " with sh_single_quote when quote.
fn print_arg<E>(arg: Option<&str>, flag: &str, quote: bool, out: &mut E) -> io::Result<()>
where
    E: Write,
{
    if let Some(arg) = arg {
        let rendered = if quote {
            sh_single_quote(arg)
        } else {
            arg.to_string()
        };
        write!(out, "{flag} {rendered} ")?;
    }
    Ok(())
}

/// complete.def print_cmd_name: -D/-E/-I pseudo names pass through, the empty
/// name prints as '', other names single-quote when they contain metas.
fn print_cmd_name(cmd: &str) -> String {
    match cmd {
        "-D" | "-E" | "-I" => cmd.to_string(),
        "" => "''".to_string(),
        _ => {
            if sh_contains_shell_metas(cmd) {
                sh_single_quote(cmd)
            } else {
                cmd.to_string()
            }
        }
    }
}

/// lib/sh/shquote.c sh_single_quote: wrap in single quotes, rendering each
/// embedded quote as '\''.
fn sh_single_quote(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// lib/sh/shquote.c sh_contains_shell_metas.
fn sh_contains_shell_metas(s: &str) -> bool {
    const METAS: &str = " \t\n'\"\\|&;()<>!{}*[]?]^$`";
    for (index, c) in s.char_indices() {
        if METAS.contains(c) {
            return true;
        }
        if c == '~' {
            if index == 0 {
                return true;
            }
            let prev = s[..index].chars().last();
            if prev == Some('=') || prev == Some(':') {
                return true;
            }
        }
        if c == '#' && index == 0 {
            return true;
        }
    }
    false
}

pub(crate) fn write_usage<E>(builtin: CompletionBuiltin, stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    let usage = match builtin {
        CompletionBuiltin::Complete => {
            "complete: usage: complete [-abcdefgjksuv] [-pr] [-DEI] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [name ...]"
        }
        CompletionBuiltin::Compgen => {
            "compgen: usage: compgen [-V varname] [-abcdefgjksuv] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [word]"
        }
        CompletionBuiltin::Compopt => {
            "compopt: usage: compopt [-o|+o option] [-DEI] [name ...]"
        }
    };
    writeln!(stderr, "{usage}")
}

#[cfg(test)]
mod completion_hook_tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_context() -> (
        HashMap<String, String>,
        HashMap<String, crate::builtins::alias::Alias>,
        Vec<String>,
        Vec<String>,
    ) {
        (HashMap::new(), HashMap::new(), Vec::new(), Vec::new())
    }

    #[test]
    fn default_command_completion_finds_echo() {
        let (env, aliases, fns, jobs) = empty_context();
        let specs = CompletionRegistry::new();
        let out = complete_line_candidates("ec", 2, &specs, &env, &aliases, &fns, &jobs);
        assert!(out.iter().any(|c| c == "echo"), "expected echo in {out:?}");
    }

    #[test]
    fn compspec_wordlist_filters_by_prefix() {
        let (env, aliases, fns, jobs) = empty_context();
        let mut specs = CompletionRegistry::new();
        specs.insert(
            "y",
            Compspec {
                actions: 0,
                options: 0,
                globpat: None,
                words: Some("alpha beta gamma".to_string()),
                prefix: None,
                suffix: None,
                filterpat: None,
                command: None,
                funcname: None,
            },
        );
        let out = complete_line_candidates("y g", 4, &specs, &env, &aliases, &fns, &jobs);
        assert!(
            out.iter().any(|c| c == "gamma"),
            "expected gamma in {out:?}"
        );
        assert!(
            !out.iter().any(|c| c == "alpha"),
            "alpha should be filtered out by prefix g: {out:?}"
        );
    }

    #[test]
    fn compspec_file_action_lists_root() {
        let (env, aliases, fns, jobs) = empty_context();
        let mut specs = CompletionRegistry::new();
        let file_bit = COMPACTS
            .iter()
            .find(|(n, _, _)| *n == "file")
            .map(|(_, b, _)| *b)
            .expect("file action bit");
        specs.insert(
            "z",
            Compspec {
                actions: file_bit,
                options: 0,
                globpat: None,
                words: None,
                prefix: None,
                suffix: None,
                filterpat: None,
                command: None,
                funcname: None,
            },
        );
        let out = complete_line_candidates("z /", 3, &specs, &env, &aliases, &fns, &jobs);
        assert!(!out.is_empty(), "expected directory entries under /");
        assert!(
            out.iter().all(|c| c.starts_with('/')),
            "entries should be absolute paths: {out:?}"
        );
    }
}
