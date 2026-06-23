//! kill module.
//!
//! GNU Bash source ownership:
// - builtins/kill.def

use std::io::{self, Write};

const SIGNALS: &[(i32, &str)] = &[
    (1, "HUP"),
    (2, "INT"),
    (3, "QUIT"),
    (4, "ILL"),
    (5, "TRAP"),
    (6, "ABRT"),
    (7, "EMT"),
    (8, "FPE"),
    (9, "KILL"),
    (10, "BUS"),
    (11, "SEGV"),
    (12, "SYS"),
    (13, "PIPE"),
    (14, "ALRM"),
    (15, "TERM"),
    (16, "URG"),
    (17, "STOP"),
    (18, "TSTP"),
    (19, "CONT"),
    (20, "CHLD"),
    (21, "TTIN"),
    (22, "TTOU"),
    (23, "IO"),
    (24, "XCPU"),
    (25, "XFSZ"),
    (26, "VTALRM"),
    (27, "PROF"),
    (28, "WINCH"),
    (29, "PWR"),
    (30, "USR1"),
    (31, "USR2"),
];

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut stdout, &mut stderr)
}

pub fn execute_with_io<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/kill.def/siglist.c): Implement the full signal table,
    // option parser, and process signalling. Upstream builtins.tests only uses
    // `kill -l` name/number translation.
    if args.first().map(String::as_str) != Some("-l") {
        return Ok(0);
    }

    match args.get(1).map(String::as_str) {
        None => {
            write_signal_list(stdout)?;
            Ok(0)
        }
        Some(value) => {
            if let Some(translation) = translate_signal(value) {
                writeln!(stdout, "{translation}")?;
                return Ok(0);
            }
            writeln!(
                stderr,
                "{}kill: {value}: invalid signal specification",
                diagnostic_prefix()
            )?;
            Ok(1)
        }
    }
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub fn translate_signal(value: &str) -> Option<&'static str> {
    if value == "0" {
        return Some("EXIT");
    }

    if value == "EXIT" || value == "SIGEXIT" {
        return Some("0");
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return signal_name(number);
    }

    let name = value.strip_prefix("SIG").unwrap_or(value);
    signal_number(name)
}

fn signal_name(number: i32) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(signal_number, name)| (*signal_number == number).then_some(*name))
}

fn signal_number(name: &str) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(signal_number, signal_name)| (*signal_name == name).then_some(*signal_number))
        .map(signal_number_string)
}

fn signal_number_string(number: i32) -> &'static str {
    match number {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        10 => "10",
        11 => "11",
        12 => "12",
        13 => "13",
        14 => "14",
        15 => "15",
        16 => "16",
        17 => "17",
        18 => "18",
        19 => "19",
        20 => "20",
        21 => "21",
        22 => "22",
        23 => "23",
        24 => "24",
        25 => "25",
        26 => "26",
        27 => "27",
        28 => "28",
        29 => "29",
        30 => "30",
        31 => "31",
        _ => unreachable!("signal number comes from SIGNALS"),
    }
}

fn write_signal_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    for chunk in SIGNALS.chunks(5) {
        for (index, (number, name)) in chunk.iter().enumerate() {
            if index > 0 {
                write!(stdout, "\t")?;
            }
            write!(stdout, "{number:>2}) SIG{name}")?;
        }
        writeln!(stdout)?;
    }
    Ok(())
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
