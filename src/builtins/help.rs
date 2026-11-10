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

/// The help topic names, in table order — the single source the
/// completion `helptopic` action projects from (rubash#2036 S2; the
/// completion face previously carried its own hand copy, which had
/// drifted by thirteen `set -o` option names GNU does not treat as
/// help topics). The registry consistency tests pin this set against
/// the command registry's builtin/keyword projection.
pub(crate) fn help_topic_names() -> Vec<&'static str> {
    HELP_TOPICS_TABLE.iter().map(|topic| topic.name).collect()
}

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut stdout, &mut stderr)
}

/// The bare-listing layout inputs the rendering cannot see from the
/// argument list: the session's COLUMNS variable (GNU reads it through
/// get_string_value in default_columns, general.c:1461 — a shell
/// variable, not just the process environment) and whether the active
/// locale is multibyte (help.def selects wdispcolumn over dispcolumn
/// on MB_CUR_MAX > 1). The executor supplies both from its variable
/// table; the process-level entry points derive them from the
/// environment instead.
#[derive(Debug, Clone)]
pub(crate) struct HelpListingLayout {
    pub columns_var: Option<String>,
    pub multibyte: bool,
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
    let layout = HelpListingLayout {
        columns_var: std::env::var("COLUMNS").ok(),
        multibyte: crate::locale::is_multi_byte_active(),
    };
    execute_with_io_and_layout(args, stdout, stderr, &layout)
}

pub(crate) fn execute_with_io_and_layout<W, E>(
    args: &[String],
    stdout: &mut W,
    stderr: &mut E,
    layout: &HelpListingLayout,
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
        print_help_list(stdout, layout)?;
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
    // special, per the comment at help.def:129.) GNU has no `sudo` help
    // topic either — rubash#759 item 4: with the sudo builtin hidden behind
    // the embedding-host elevation handler, `help sudo` takes this same
    // no-topics form instead of a hand-written page.
    if match_found == 0 {
        writeln!(
            stderr,
            "{}help: no help topics match `{last_pattern}'.  Try `help help' or `man -k {last_pattern}' or `info {last_pattern}'.",
            diagnostic_prefix()
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    Ok(EXECUTION_SUCCESS)
}

fn find_topic(name: &str) -> Option<&'static HelpTopic> {
    HELP_TOPICS_TABLE.iter().find(|topic| topic.name == name)
}

/// The short documentation line for a builtin's help topic, for the
/// declaration table's `DocId` resolution (Stage-2 ④ Phase 0).
/// `None` when the name has no topic (rubash's own `env`).
pub(crate) fn builtin_short_doc(name: &str) -> Option<&'static str> {
    find_topic(name).map(|topic| topic.short_doc)
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

/// CASE_HELPOPT (builtins/common.h:31-36): a `--help` word in option
/// position makes the builtin print its own help. Shared by builtins that
/// run internal_getopt (the caller returns EX_USAGE, as the macro does).
pub(crate) fn print_builtin_help<W>(name: &str, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if let Some(topic) = find_topic(name) {
        print_topic_long(topic, stdout)?;
    }
    Ok(())
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
        "    GNU bash, version {} ({})",
        crate::executor::bash_version_value(),
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

pub(crate) fn print_caller_help_with_io<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if let Some(topic) = find_topic("caller") {
        return print_topic_long(topic, stdout);
    }
    Ok(())
}

fn print_help_list<W>(stdout: &mut W, layout: &HelpListingLayout) -> io::Result<()>
where
    W: Write,
{
    // GNU help.def prints the shell version banner as the first line of the
    // bare help listing (builtins10.sub pipes through "sed 1d" to strip it).
    // version.c:90 show_shell_version(): one "-release" (it is part of the
    // version string) followed by the configure-time MACHTYPE in parens.
    writeln!(
        stdout,
        "GNU bash, version {} ({})",
        crate::executor::bash_version_value(),
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
    render_builtin_listing(stdout, layout)?;
    Ok(())
}

// general.c:1461 default_columns: the COLUMNS variable when atoi parses
// it to a positive value, else the window size, else 80. (The select
// menu's select_default_columns is the executor's other port of this
// function; this one keeps atoi semantics — a leading digit run wins,
// trailing junk is ignored — because that is what general.c does.)
fn default_columns(columns_var: Option<&str>) -> usize {
    if let Some(value) = columns_var {
        if let Some(columns) = atoi_positive(value) {
            return columns;
        }
    }
    if let Some((_, cols)) = crate::script_driver::process_console_size() {
        if cols > 0 {
            return cols as usize;
        }
    }
    80
}

/// C atoi: skips leading whitespace and an optional sign, then reads a
/// digit run; no digits (or a non-positive result) yields None here.
fn atoi_positive(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && (bytes[index] == b'-' || bytes[index] == b'+') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let mut parsed: i64 = 0;
    let mut any_digit = false;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        any_digit = true;
        parsed = parsed
            .saturating_mul(10)
            .saturating_add(i64::from(bytes[index] - b'0'));
        index += 1;
    }
    if any_digit && !negative && parsed > 0 {
        usize::try_from(parsed).ok()
    } else {
        None
    }
}

// GNU bash 5.3 help.def show_builtin_command_help: the short docs of the
// builtin table are shown in two columns at default_columns()/2, capped
// at the 128-byte blurb size and floored back to 40 when that yields 3
// or less; height = (num + 1) / 2 rows pair entry i (left column) with
// entry i + height (right column) — a column-major fill.  The last row
// carries only the left entry when (i << 1) >= num or i + height >= num
// (dispcolumn's early newline).  The left cell is padded out to the
// column width before the right cell is printed.
fn render_builtin_listing<W>(stdout: &mut W, layout: &HelpListingLayout) -> io::Result<()>
where
    W: Write,
{
    let columns = default_columns(layout.columns_var.as_deref());
    let mut width = columns / 2;
    if width > 128 {
        width = 128;
    }
    if width <= 3 {
        width = 40;
    }
    let num = HELP_ENTRIES.len();
    let height = (num + 1) / 2;
    for i in 0..height {
        let first = listing_cell(HELP_ENTRIES[i], width, true, layout.multibyte);
        if (i << 1) >= num || i + height >= num {
            writeln!(stdout, "{first}")?;
            continue;
        }
        // The table is pure ASCII, so a cell's character count is its
        // display width — the quantity both GNU cells pad against
        // (strlen in dispcolumn, dispcols in wdispcolumn).
        let mut line = first;
        while line.chars().count() < width {
            line.push(' ');
        }
        line.push_str(&listing_cell(
            HELP_ENTRIES[i + height],
            width,
            false,
            layout.multibyte,
        ));
        writeln!(stdout, "{line}")?;
    }
    Ok(())
}

// One column cell: a space marker (a disabled builtin would print *; the
// default table never has one) plus the short doc, truncated with a
// trailing > at the column boundary.  The two GNU renderers differ by
// one kept character at the boundary, and the multibyte one is selected
// purely by the locale (MB_CUR_MAX > 1), so both are kept:
// - wdispcolumn (multibyte): dispchars = min(len, width - 2) display
//   characters fit the cell; truncation fires when dispchars + 1 >=
//   width - 2 and keeps dispchars - 1 characters on the left,
//   dispchars - 2 on the right (the right column's dispchars-1 index
//   is the bash-4.3 terminal guard).
// - dispcolumn (single-byte): the doc is copied into a width-1 buffer
//   whose last two (left) / last-but-two (right) slots the '>' and the
//   NUL overwrite, so truncation fires at len >= width - 3 (left) /
//   len >= width - 4 (right) and keeps width - 3 / width - 4
//   characters; a doc of exactly that length still gains the '>'.
fn listing_cell(doc: &str, width: usize, first_column: bool, multibyte: bool) -> String {
    let len = doc.chars().count();
    if multibyte {
        let mut slen = len;
        if slen >= width {
            slen = width.saturating_sub(2);
        }
        let dispchars = slen.min(width.saturating_sub(2));
        if dispchars + 1 >= width.saturating_sub(2) {
            let kept = if first_column {
                dispchars.saturating_sub(1)
            } else {
                dispchars.saturating_sub(2)
            };
            let truncated: String = doc.chars().take(kept).collect();
            return format!(" {truncated}>");
        }
        return format!(" {doc}");
    }
    let (threshold, keep) = if first_column {
        (width.saturating_sub(3), width.saturating_sub(3))
    } else {
        (width.saturating_sub(4), width.saturating_sub(4))
    };
    if len >= threshold {
        let truncated: String = doc.chars().take(keep).collect();
        return format!(" {truncated}>");
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
    "env [-i] [-u name] [name=value ...] [command [argument ...]]",
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
