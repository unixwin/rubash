//! help module.
//!
//! GNU Bash source ownership:
// - builtins/help.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
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
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    if args.first() == Some(&"--") {
        args.remove(0);
    }

    if args
        .iter()
        .any(|arg| arg.starts_with('-') && !matches!(*arg, "-s" | "-d" | "-m"))
    {
        writeln!(stderr, "{}help: -x: invalid option", diagnostic_prefix())?;
        writeln!(stderr, "help: usage: help [-dms] [pattern ...]")?;
        return Ok(EX_USAGE);
    }

    let short = args.contains(&"-s");
    let desc = args.contains(&"-d");
    let manpage = args.contains(&"-m");
    let patterns: Vec<&str> = args
        .into_iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();

    if patterns.is_empty() {
        print_help_list(stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    if short {
        print_short_help(&patterns, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    if desc {
        print_desc_help(&patterns, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    if manpage {
        print_manpage_help(&patterns, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    print_long_help(&patterns, stdout, stderr)?;
    Ok(EXECUTION_SUCCESS)
}

fn print_short_help<W>(patterns: &[&str], stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    match patterns {
        ["help"] => writeln!(stdout, "help: help [-dms] [pattern ...]")?,
        ["builtin", "shift"] => {
            writeln!(stdout, "builtin: builtin [shell-builtin [arg ...]]")?;
            writeln!(stdout, "shift: shift [n]")?;
        }
        ["read*"] => {
            writeln!(stdout, "Shell commands matching keyword `read*'")?;
            writeln!(stdout)?;
            print_read_synopses(stdout)?;
        }
        ["rea"] => print_read_synopses(stdout)?,
        _ => {}
    }
    Ok(())
}

fn print_read_synopses<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(stdout, "read: read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]")?;
    writeln!(stdout, "readarray: readarray [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]")?;
    writeln!(
        stdout,
        "readonly: readonly [-aAf] [name[=value] ...] or readonly -p"
    )
}

fn print_desc_help<W>(patterns: &[&str], stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if patterns == ["shift"] {
        writeln!(stdout, "shift - Shift positional parameters.")?;
    }
    Ok(())
}

fn print_long_help<W, E>(patterns: &[&str], stdout: &mut W, stderr: &mut E) -> io::Result<()>
where
    W: Write,
    E: Write,
{
    match patterns {
        [":"] => {
            writeln!(stdout, ":: :")?;
            writeln!(stdout, "    Null command.")?;
            writeln!(stdout, "    ")?;
            writeln!(stdout, "    No effect; the command does nothing.")?;
            writeln!(stdout, "    ")?;
            writeln!(stdout, "    Exit Status:")?;
            writeln!(stdout, "    Always succeeds.")?;
        }
        ["bash"] => {
            writeln!(
                stderr,
                "{}help: no help topics match `bash'.  Try `help help' or `man -k bash' or `info bash'.",
                diagnostic_prefix()
            )?;
        }
        #[cfg(windows)]
        ["sudo"] => {
            crate::builtins::sudo::print_help_with_io(stdout)?;
        }
        _ => {}
    }
    Ok(())
}

fn print_manpage_help<W>(patterns: &[&str], stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if patterns == [":"] {
        writeln!(stdout, "NAME")?;
        writeln!(stdout, "    : - Null command.")?;
        writeln!(stdout)?;
        writeln!(stdout, "SYNOPSIS")?;
        writeln!(stdout, "    :")?;
        writeln!(stdout)?;
        writeln!(stdout, "DESCRIPTION")?;
        writeln!(stdout, "    Null command.")?;
        writeln!(stdout, "    ")?;
        writeln!(stdout, "    No effect; the command does nothing.")?;
        writeln!(stdout, "    ")?;
        writeln!(stdout, "    Exit Status:")?;
        writeln!(stdout, "    Always succeeds.")?;
        writeln!(stdout)?;
        writeln!(stdout, "SEE ALSO")?;
        writeln!(stdout, "    bash(1)")?;
        writeln!(stdout)?;
        writeln!(stdout, "IMPLEMENTATION")?;
        writeln!(
            stdout,
            "    Copyright (C) 2025 Free Software Foundation, Inc."
        )?;
        writeln!(stdout)?;
    }
    Ok(())
}

pub(crate) fn print_shift_help_with_io<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(stdout, "shift: shift [n]")?;
    writeln!(stdout, "    Shift positional parameters.")?;
    writeln!(stdout, "    ")?;
    writeln!(
        stdout,
        "    Rename the positional parameters $N+1,$N+2 ... to $1,$2 ...  If N is"
    )?;
    writeln!(stdout, "    not given, it is assumed to be 1.")?;
    writeln!(stdout, "    ")?;
    writeln!(stdout, "    Exit Status:")?;
    writeln!(
        stdout,
        "    Returns success unless N is negative or greater than $#."
    )
}

fn print_help_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    // GNU help.def prints the shell version banner as the first line of the
    // bare help listing (builtins10.sub pipes through "sed 1d" to strip it).
    writeln!(
        stdout,
        "GNU bash, version 5.3.0(1)-release-(x86_64-pc-msys)"
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
