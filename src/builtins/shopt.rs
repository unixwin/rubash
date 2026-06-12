//! shopt module.
//!
//! GNU Bash source ownership:
// - builtins/shopt.def

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

static XPG_ECHO: AtomicBool = AtomicBool::new(false);
static SOURCEPATH: AtomicBool = AtomicBool::new(true);
static CHECKHASH: AtomicBool = AtomicBool::new(false);

pub(crate) fn xpg_echo_enabled() -> bool {
    XPG_ECHO.load(Ordering::Relaxed)
}

pub(crate) fn sourcepath_enabled() -> bool {
    SOURCEPATH.load(Ordering::Relaxed)
}

pub(crate) fn checkhash_enabled() -> bool {
    CHECKHASH.load(Ordering::Relaxed)
}

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, &mut stdout, &mut stderr)
}

fn execute_with_io<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut print = args.is_empty();
    let mut status = EXECUTION_SUCCESS;
    let mut mode = ShoptMode::List;
    let mut names = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-s" => mode = ShoptMode::Set,
            "-u" => mode = ShoptMode::Unset,
            "-q" => mode = ShoptMode::Query,
            "-p" => print = true,
            option if option.starts_with('-') => {
                writeln!(stderr, "rubash: shopt: {option}: invalid option")?;
                status = EXECUTION_FAILURE;
            }
            name => names.push(name),
        }
    }

    if !names.is_empty() {
        for name in names {
            if !is_supported_option(name) {
                writeln!(stderr, "rubash: shopt: {name}: invalid shell option name")?;
                status = EXECUTION_FAILURE;
                continue;
            }

            match mode {
                ShoptMode::Set if name == "xpg_echo" => XPG_ECHO.store(true, Ordering::Relaxed),
                ShoptMode::Unset if name == "xpg_echo" => XPG_ECHO.store(false, Ordering::Relaxed),
                ShoptMode::Set if name == "sourcepath" => SOURCEPATH.store(true, Ordering::Relaxed),
                ShoptMode::Unset if name == "sourcepath" => {
                    SOURCEPATH.store(false, Ordering::Relaxed)
                }
                ShoptMode::Set if name == "checkhash" => {
                    CHECKHASH.store(true, Ordering::Relaxed);
                    std::env::set_var("__RUBASH_SHOPT_CHECKHASH", "1");
                }
                ShoptMode::Unset if name == "checkhash" => {
                    CHECKHASH.store(false, Ordering::Relaxed);
                    std::env::remove_var("__RUBASH_SHOPT_CHECKHASH");
                }
                ShoptMode::Query if option_enabled(name) => {}
                ShoptMode::Query => status = EXECUTION_FAILURE,
                ShoptMode::List => {
                    if print {
                        print_shopt(name, stdout)?;
                    }
                }
                _ => {}
            }
        }
    }

    if print {
        if args.is_empty() {
            writeln!(stdout, "expand_aliases\toff")?;
            writeln!(
                stdout,
                "sourcepath\t{}",
                if sourcepath_enabled() { "on" } else { "off" }
            )?;
            writeln!(
                stdout,
                "checkhash\t{}",
                if checkhash_enabled() { "on" } else { "off" }
            )?;
            writeln!(
                stdout,
                "xpg_echo\t{}",
                if xpg_echo_enabled() { "on" } else { "off" }
            )?;
        }
    }

    Ok(status)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShoptMode {
    List,
    Set,
    Unset,
    Query,
}

fn option_enabled(name: &str) -> bool {
    match name {
        "xpg_echo" => xpg_echo_enabled(),
        "checkhash" => checkhash_enabled(),
        "sourcepath" => sourcepath_enabled(),
        "expand_aliases" => false,
        _ => false,
    }
}

fn is_supported_option(name: &str) -> bool {
    matches!(name, "checkhash" | "expand_aliases" | "sourcepath" | "xpg_echo")
}

fn print_shopt<W>(name: &str, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        stdout,
        "{name}\t{}",
        if option_enabled(name) { "on" } else { "off" }
    )
}
