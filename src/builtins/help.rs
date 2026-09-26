//! help module.
//!
//! GNU Bash source ownership:
// - builtins/help.def (help_builtin 91-190, show_longdoc 224-243,
//   show_desc 245-283, show_manpage 287-368, dispcolumn 371-405)
// - builtins/bashgetopt.c (internal_getopt 60-130 with NOTOPT at :37)
// - builtins/common.h (ISHELP at :27)
// - lib/glob/glob_loop.c (internal_glob_pattern_p 24-77 via glob_pattern_p)
// - builtins/mkbuiltins.c (extraction of the doc table; see help_texts.rs)

use std::io::{self, Write};

#[path = "help_texts.rs"]
mod help_texts;

use help_texts::{HelpTopic, HELP_TOPICS_TABLE};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

// Consumed previously by the completion helptopic action, which now carries
// its own GNU-aligned table (see builtins/complete.rs).
#[allow(dead_code)]
pub(crate) const HELP_TOPICS: &[&str] = &[
    "!",
    ".",
    ":",
    "[",
    "[[",
    "]]",
    "{",
    "}",
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
    "do",
    "done",
    "echo",
    "elif",
    "else",
    "enable",
    "esac",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "fi",
    "for",
    "function",
    "getopts",
    "hash",
    "help",
    "history",
    "if",
    "in",
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
    #[cfg(windows)]
    "sudo",
    "suspend",
    "test",
    "then",
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
];

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // builtins/bashgetopt.c internal_getopt(list, "dms"): option processing
    // stops at the first non-option word (NOTOPT at bashgetopt.c:37 also
    // treats a bare `-` as a non-option, so `help -` searches for the topic
    // `-`); `--` ends options; a `--help` word in option position (ISHELP,
    // common.h:27) makes CASE_HELPOPT print this builtin's own help and
    // return EX_USAGE (help.def:114).  Invalid option letters are reported
    // one at a time by sh_invalidopt (bashgetopt.c:107).
    let mut dflag = false;
    let mut sflag = false;
    let mut mflag = false;
    let mut patterns: Vec<&str> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let word = args[index].as_str();
        if !word.starts_with('-') || word == "-" {
            // Non-option word: it and every remaining word are the pattern
            // list (loptend); no further option parsing happens.
            patterns = args[index..].iter().map(String::as_str).collect();
            break;
        }
        if word == "--help" {
            if let Some(topic) = find_topic("help") {
                print_topic_long(topic, stdout)?;
            }
            return Ok(EX_USAGE);
        }
        if word == "--" {
            patterns = args[index + 1..].iter().map(String::as_str).collect();
            break;
        }
        for option in word[1..].chars() {
            match option {
                'd' => dflag = true,
                'm' => mflag = true,
                's' => sflag = true,
                _ => {
                    writeln!(
                        stderr,
                        "{}help: -{option}: invalid option",
                        diagnostic_prefix()
                    )?;
                    writeln!(stderr, "help: usage: help [-dms] [pattern ...]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    if patterns.is_empty() {
        print_help_list(stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    // help.def:131-136: when the first pattern word is a glob pattern,
    // announce the search (ngettext picks keyword/keywords on whether the
    // whole word list has more than one member).
    if glob_pattern_p(patterns[0]) {
        let keyword = if patterns.len() > 1 {
            "keywords"
        } else {
            "keyword"
        };
        writeln!(
            stdout,
            "Shell commands matching {keyword} `{}'",
            patterns.join(", ")
        )?;
        writeln!(stdout)?;
    }

    // help.def:138-181: for each pattern, pass 1 takes exact or pattern
    // (strmatch, FNMATCH_EXTFLAG) matches over shell_builtins order; pass 2
    // runs only when pass 1 found nothing for that pattern and takes prefix
    // matches (strncmp of plen bytes).
    let mut match_found = 0usize;
    let last_pattern = patterns[patterns.len() - 1];
    for &pattern in &patterns {
        let mut this_found = false;
        for pass in 1..=2u8 {
            for topic in HELP_TOPICS_TABLE.iter() {
                let matched = if pass == 1 {
                    topic.name == pattern
                        || crate::executor::conditional::shell_pattern_matches(pattern, topic.name)
                } else {
                    topic.name.starts_with(pattern)
                };
                if !matched {
                    continue;
                }
                this_found = true;
                match_found += 1;
                // help.def:161-176: -d wins over -m; without them print
                // `name: short_doc', and the long doc unless -s.
                if dflag {
                    show_desc(topic, stdout)?;
                } else if mflag {
                    show_manpage(topic, stdout)?;
                } else {
                    print_topic_synopsis(topic, stdout)?;
                    if !sflag {
                        print_topic_longdoc(topic, stdout)?;
                    }
                }
            }
            if pass == 1 && this_found {
                break;
            }
        }
    }

    // help.def:183-187: no topic matched any pattern; the diagnostic names
    // the last pattern searched.  (`help bash' deliberately does nothing
    // special, per the comment at help.def:129.)
    if match_found == 0 {
        if let Some(status) = try_windows_help_extension(&patterns, dflag, mflag, sflag, stdout)? {
            return Ok(status);
        }
        writeln!(
            stderr,
            "{}help: no help topics match `{last_pattern}'.  Try `help help' or `man -k {last_pattern}' or `info {last_pattern}'.",
            diagnostic_prefix()
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    Ok(EXECUTION_SUCCESS)
}

// rubash windows extension preserved from the previous implementation: the
// windows-only `sudo` builtin keeps its hand-written help page for the exact
// long-form invocation `help sudo`.  GNU has no such table entry.
#[cfg(windows)]
fn try_windows_help_extension<W>(
    patterns: &[&str],
    dflag: bool,
    mflag: bool,
    sflag: bool,
    stdout: &mut W,
) -> io::Result<Option<i32>>
where
    W: Write,
{
    if patterns == ["sudo"] && !dflag && !mflag && !sflag {
        crate::builtins::sudo::print_help_with_io(stdout)?;
        return Ok(Some(EXECUTION_SUCCESS));
    }
    Ok(None)
}

#[cfg(not(windows))]
fn try_windows_help_extension<W>(
    _patterns: &[&str],
    _dflag: bool,
    _mflag: bool,
    _sflag: bool,
    _stdout: &mut W,
) -> io::Result<Option<i32>>
where
    W: Write,
{
    Ok(None)
}

fn find_topic(name: &str) -> Option<&'static HelpTopic> {
    HELP_TOPICS_TABLE.iter().find(|topic| topic.name == name)
}

// help.def:172: `name: short_doc'.
fn print_topic_synopsis<W>(topic: &HelpTopic, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(stdout, "{}: {}", topic.name, topic.short_doc)
}

// help.def:224-243 show_longdoc: every long-documentation line is printed
// with a BASE_INDENT (builtins.h:50) four-space prefix; blank documentation
// lines render as four spaces.  A doc string that mkbuiltins left ending in
// `\n` (a `#`-guard line after the last text line) renders as one final
// EMPTY line instead (see help_texts.rs).
fn print_topic_longdoc<W>(topic: &HelpTopic, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    for line in topic.doc {
        writeln!(stdout, "    {line}")?;
    }
    if topic.trailing_newline {
        writeln!(stdout)?;
    }
    Ok(())
}

// builtin_help (help.def:192-204), also used by `help --help`.
fn print_topic_long<W>(topic: &HelpTopic, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    print_topic_synopsis(topic, stdout)?;
    print_topic_longdoc(topic, stdout)
}

// help.def:245-283 show_desc: `name - <first doc line>'.
fn show_desc<W>(topic: &HelpTopic, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        stdout,
        "{} - {}",
        topic.name,
        topic.doc.first().copied().unwrap_or("")
    )
}

// help.def:287-368 show_manpage: pseudo-manpage rendering.  The version,
// copyright, and license lines come from version.c:90-94 show_shell_version
// and version.c:50-51 bash_copyright/bash_license (the license string itself
// ends in a newline, so the page ends with a blank line).
fn show_manpage<W>(topic: &HelpTopic, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(stdout, "NAME")?;
    writeln!(
        stdout,
        "    {} - {}",
        topic.name,
        topic.doc.first().copied().unwrap_or("")
    )?;
    writeln!(stdout)?;
    writeln!(stdout, "SYNOPSIS")?;
    writeln!(stdout, "    {}", topic.short_doc)?;
    writeln!(stdout)?;
    writeln!(stdout, "DESCRIPTION")?;
    print_topic_longdoc(topic, stdout)?;
    writeln!(stdout)?;
    writeln!(stdout, "SEE ALSO")?;
    writeln!(stdout, "    bash(1)")?;
    writeln!(stdout)?;
    writeln!(stdout, "IMPLEMENTATION")?;
    writeln!(
        stdout,
        "    GNU bash, version 5.3.0(1)-release ({})",
        crate::executor::machtype_value()
    )?;
    writeln!(
        stdout,
        "    Copyright (C) 2025 Free Software Foundation, Inc."
    )?;
    writeln!(
        stdout,
        "    License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>"
    )?;
    writeln!(stdout)
}

// lib/glob/glob_loop.c internal_glob_pattern_p (24-77), reached through
// glob_pattern_p (lib/glob/glob.c:155): `*' and `?' anywhere, a `]' that
// closes an earlier `[' (bracket expressions must be complete), and the
// extended-glob openers `+(' `@(' `!(' make the word a pattern; a backslash
// hides the following character, and a trailing backslash is not a pattern.
fn glob_pattern_p(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut bracket_open = false;
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '?' | '*' => return true,
            '[' => bracket_open = true,
            ']' => {
                if bracket_open {
                    return true;
                }
            }
            '+' | '@' | '!' => {
                if chars.get(index + 1) == Some(&'(') {
                    return true;
                }
            }
            '\\' => {
                if index + 1 < chars.len() {
                    index += 1;
                } else {
                    return false;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

pub(crate) fn print_shift_help_with_io<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if let Some(topic) = find_topic("shift") {
        return print_topic_long(topic, stdout);
    }
    Ok(())
}

fn print_help_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    // GNU help.def prints the shell version banner as the first line of the
    // bare help listing (builtins10.sub pipes through "sed 1d" to strip it).
    // version.c:90 show_shell_version(): one "-release" (it is part of the
    // version string) followed by the configure-time MACHTYPE in parens.
    writeln!(
        stdout,
        "GNU bash, version 5.3.0(1)-release ({})",
        crate::executor::machtype_value()
    )?;
    // TODO(builtins/help.def/builtins/gen-helpfiles.c): Generate this from the
    // builtin table. The current list matches the upstream builtins10.sub
    // expected output after its pipeline removes the Bash version line.
    writeln!(
        stdout,
        "These shell commands are defined internally.  Type `help' to see this list."
    )?;
    writeln!(
        stdout,
        "Type `help name' to find out more about the function `name'."
    )?;
    writeln!(
        stdout,
        "Use `info bash' to find out more about the shell in general."
    )?;
    writeln!(
        stdout,
        "Use `man -k' or `info' to find out more about commands not in this list."
    )?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "A star (*) next to a name means that the command is disabled."
    )?;
    writeln!(stdout)?;
    render_builtin_listing(stdout)?;
    Ok(())
}

// GNU bash 5.3 help.def show_builtin_command_help / dispcolumn: the short
// docs of the builtin table are shown in two columns at default_columns()/2
// (40 under the 80-column default), pairing entry i with entry i + height,
// height = (num + 1) / 2.  The last row carries only the left entry when
// (i << 1) >= num.  The harness runs with a UTF-8 locale, so the multibyte
// display path applies: a cell truncates when its display width (doc + 1
// marker column) reaches width - 2, keeping min(len, 38) - 1 characters in
// the left column and min(len, 38) - 2 in the right one, with a trailing >.
fn render_builtin_listing<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    const WIDTH: usize = 40;
    let num = HELP_ENTRIES.len();
    let height = (num + 1) / 2;
    for i in 0..height {
        let doc = HELP_ENTRIES[i];
        let mut line = truncate_cell(doc, WIDTH - 2, 1);
        while line.chars().count() < WIDTH {
            line.push(' ');
        }
        if (i << 1) >= num || i + height >= num {
            let rendered = line.trim_end().to_string();
            writeln!(stdout, "{rendered}")?;
            continue;
        }
        line.push_str(&truncate_cell(HELP_ENTRIES[i + height], WIDTH - 2, 2));
        let rendered = line.trim_end().to_string();
        writeln!(stdout, "{rendered}")?;
    }
    Ok(())
}

// One column cell: a space marker (a disabled builtin would print *; the
// default table never has one) plus the short doc, truncated with a trailing
// > when the doc plus marker reaches the column limit.  limit is the display
// columns available for marker + doc + truncation marker; column selects the
// GNU wdispcolumn keep count (dispchars - column).
fn truncate_cell(doc: &str, limit: usize, column: usize) -> String {
    let chars: Vec<char> = doc.chars().collect();
    let dispchars = chars.len().min(limit);
    if chars.len() + 1 >= limit {
        let kept = dispchars - column;
        let mut cell: String = chars[..kept].iter().collect();
        cell.push('>');
        return format!(" {cell}");
    }
    format!(" {doc}")
}

// GNU bash 5.3 builtin-table short docs in shell_builtins[] display order
// (builtins/*.def $SHORT_DOC strings; reserved.def supplies the leading `!`
// entry, new in 5.3: `. [-p path]`, `read [-Eers]`, `trap [-Plp]`,
// `compgen [-V varname]`, and the reworded `cd` usage).
const HELP_ENTRIES: &[&str] = &[
    "! PIPELINE",
    "job_spec [&]",
    "(( expression ))",
    ". [-p path] filename [arguments]",
    ":",
    "[ arg... ]",
    "[[ expression ]]",
    "alias [-p] [name[=value] ... ]",
    "bg [job_spec ...]",
    "bind [-lpsvPSVX] [-m keymap] [-f filename] [-q name] [-u name] [-r keyseq] [-x keyseq:shell-command] [keyseq:readline-function or readline-command]",
    "break [n]",
    "builtin [shell-builtin [arg ...]]",
    "caller [expr]",
    "case WORD in [PATTERN [| PATTERN]...) COMMANDS ;;]... esac",
    "cd [-L|[-P [-e]]] [-@] [dir]",
    "command [-pVv] command [arg ...]",
    "compgen [-V varname] [-abcdefgjksuv] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [word]",
    "complete [-abcdefgjksuv] [-pr] [-DEI] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [name ...]",
    "compopt [-o|+o option] [-DEI] [name ...]",
    "continue [n]",
    "coproc [NAME] command [redirections]",
    "declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]",
    "dirs [-clpv] [+N] [-N]",
    "disown [-h] [-ar] [jobspec ... | pid ...]",
    "echo [-neE] [arg ...]",
    "enable [-a] [-dnps] [-f filename] [name ...]",
    "eval [arg ...]",
    "exec [-cl] [-a name] [command [argument ...]] [redirection ...]",
    "exit [n]",
    "export [-fn] [name[=value] ...] or export -p [-f]",
    "false",
    "fc [-e ename] [-lnr] [first] [last] or fc -s [pat=rep] [command]",
    "fg [job_spec]",
    "for NAME [in WORDS ... ] ; do COMMANDS; done",
    "for (( exp1; exp2; exp3 )); do COMMANDS; done",
    "function name { COMMANDS ; } or name () { COMMANDS ; }",
    "getopts optstring name [arg ...]",
    "hash [-lr] [-p pathname] [-dt] [name ...]",
    "help [-dms] [pattern ...]",
    "history [-c] [-d offset] [n] or history -anrw [filename] or history -ps arg [arg...]",
    "if COMMANDS; then COMMANDS; [ elif COMMANDS; then COMMANDS; ]... [ else COMMANDS; ] fi",
    "jobs [-lnprs] [jobspec ...] or jobs -x command [args]",
    "kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]",
    "let arg [arg ...]",
    "local [option] name[=value] ...",
    "logout [n]",
    "mapfile [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]",
    "popd [-n] [+N | -N]",
    "printf [-v var] format [arguments]",
    "pushd [-n] [+N | -N | dir]",
    "pwd [-LP]",
    "read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]",
    "readarray [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]",
    "readonly [-aAf] [name[=value] ...] or readonly -p",
    "return [n]",
    "select NAME [in WORDS ... ;] do COMMANDS; done",
    "set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]",
    "shift [n]",
    "shopt [-pqsu] [-o] [optname ...]",
    "source [-p path] filename [arguments]",
    "suspend [-f]",
    "test [expr]",
    "time [-p] pipeline",
    "times",
    "trap [-Plp] [[action] signal_spec ...]",
    "true",
    "type [-afptP] name [name ...]",
    "typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p [-aAfFilnrtux] [name ...]",
    "ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]",
    "umask [-p] [-S] [mode]",
    "unalias [-a] name [name ...]",
    "unset [-f] [-v] [-n] [name ...]",
    "until COMMANDS; do COMMANDS-2; done",
    "variables - Names and meanings of some shell variables",
    "wait [-fn] [-p var] [id ...]",
    "while COMMANDS; do COMMANDS-2; done",
    "{ COMMANDS ; }",
];

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}
