//! help module.
//!
//! GNU Bash source ownership:
// - builtins/help.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

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
    for line in HELP_LIST {
        writeln!(stdout, "{line}")?;
    }
    Ok(())
}

const HELP_LIST: &[&str] = &[
    " ! PIPELINE                              history [-c] [-d offset] [n] or hist>",
    " job_spec [&]                            if COMMANDS; then COMMANDS; [ elif C>",
    " (( expression ))                        jobs [-lnprs] [jobspec ...] or jobs >",
    " . [-p path] filename [arguments]        kill [-s sigspec | -n signum | -sigs>",
    " :                                       let arg [arg ...]",
    " [ arg... ]                              local [option] name[=value] ...",
    " [[ expression ]]                        logout [n]",
    " alias [-p] [name[=value] ... ]          mapfile [-d delim] [-n count] [-O or>",
    " bg [job_spec ...]                       popd [-n] [+N | -N]",
    " bind [-lpsvPSVX] [-m keymap] [-f file>  printf [-v var] format [arguments]",
    " break [n]                               pushd [-n] [+N | -N | dir]",
    " builtin [shell-builtin [arg ...]]       pwd [-LP]",
    " caller [expr]                           read [-Eers] [-a array] [-d delim] [>",
    " case WORD in [PATTERN [| PATTERN]...)>  readarray [-d delim] [-n count] [-O >",
    " cd [-L|[-P [-e]]] [-@] [dir]            readonly [-aAf] [name[=value] ...] o>",
    " command [-pVv] command [arg ...]        return [n]",
    " compgen [-V varname] [-abcdefgjksuv] >  select NAME [in WORDS ... ;] do COMM>",
    " complete [-abcdefgjksuv] [-pr] [-DEI]>  set [-abefhkmnptuvxBCEHPT] [-o optio>",
    " compopt [-o|+o option] [-DEI] [name .>  shift [n]",
    " continue [n]                            shopt [-pqsu] [-o] [optname ...]",
    " coproc [NAME] command [redirections]    source [-p path] filename [argument>",
    " declare [-aAfFgiIlnrtux] [name[=value>  suspend [-f]",
    " dirs [-clpv] [+N] [-N]                  test [expr]",
    " disown [-h] [-ar] [jobspec ... | pid >  time [-p] pipeline",
    " echo [-neE] [arg ...]                   times",
    " enable [-a] [-dnps] [-f filename] [na>  trap [-Plp] [[action] signal_spec ..>",
    " eval [arg ...]                          true",
    " exec [-cl] [-a name] [command [argume>  type [-afptP] name [name ...]",
    " exit [n]                                typeset [-aAfFgiIlnrtux] name[=value>",
    " export [-fn] [name[=value] ...] or ex>  ulimit [-SHabcdefiklmnpqrstuvxPRT] [>",
    " false                                   umask [-p] [-S] [mode]",
    " fc [-e ename] [-lnr] [first] [last] o>  unalias [-a] name [name ...]",
    " fg [job_spec]                           unset [-f] [-v] [-n] [name ...]",
    " for NAME [in WORDS ... ] ; do COMMAND>  until COMMANDS; do COMMANDS-2; done",
    " for (( exp1; exp2; exp3 )); do COMMAN>  variables - Names and meanings of so>",
    " function name { COMMANDS ; } or name >  wait [-fn] [-p var] [id ...]",
    " getopts optstring name [arg ...]        while COMMANDS; do COMMANDS-2; done",
    " hash [-lr] [-p pathname] [-dt] [name >  { COMMANDS ; }",
    " help [-dms] [pattern ...]",
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
