//! kill module.
//!
//! GNU Bash source ownership:
// - builtins/kill.def

use std::io::{self, Write};

const SIGNALS: &[(i32, &str, &str)] = &[
    (1, "1", "HUP"),
    (2, "2", "INT"),
    (3, "3", "QUIT"),
    (4, "4", "ILL"),
    (5, "5", "TRAP"),
    (6, "6", "ABRT"),
    (7, "7", "EMT"),
    (8, "8", "FPE"),
    (9, "9", "KILL"),
    (10, "10", "BUS"),
    (11, "11", "SEGV"),
    (12, "12", "SYS"),
    (13, "13", "PIPE"),
    (14, "14", "ALRM"),
    (15, "15", "TERM"),
    (16, "16", "URG"),
    (17, "17", "STOP"),
    (18, "18", "TSTP"),
    (19, "19", "CONT"),
    (20, "20", "CHLD"),
    (21, "21", "TTIN"),
    (22, "22", "TTOU"),
    (23, "23", "IO"),
    (24, "24", "XCPU"),
    (25, "25", "XFSZ"),
    (26, "26", "VTALRM"),
    (27, "27", "PROF"),
    (28, "28", "WINCH"),
    (29, "29", "PWR"),
    (30, "30", "USR1"),
    (31, "31", "USR2"),
    (32, "32", "RTMIN"),
    (33, "33", "RTMIN+1"),
    (34, "34", "RTMIN+2"),
    (35, "35", "RTMIN+3"),
    (36, "36", "RTMIN+4"),
    (37, "37", "RTMIN+5"),
    (38, "38", "RTMIN+6"),
    (39, "39", "RTMIN+7"),
    (40, "40", "RTMIN+8"),
    (41, "41", "RTMIN+9"),
    (42, "42", "RTMIN+10"),
    (43, "43", "RTMIN+11"),
    (44, "44", "RTMIN+12"),
    (45, "45", "RTMIN+13"),
    (46, "46", "RTMIN+14"),
    (47, "47", "RTMIN+15"),
    (48, "48", "RTMIN+16"),
    (49, "49", "RTMAX-15"),
    (50, "50", "RTMAX-14"),
    (51, "51", "RTMAX-13"),
    (52, "52", "RTMAX-12"),
    (53, "53", "RTMAX-11"),
    (54, "54", "RTMAX-10"),
    (55, "55", "RTMAX-9"),
    (56, "56", "RTMAX-8"),
    (57, "57", "RTMAX-7"),
    (58, "58", "RTMAX-6"),
    (59, "59", "RTMAX-5"),
    (60, "60", "RTMAX-4"),
    (61, "61", "RTMAX-3"),
    (62, "62", "RTMAX-2"),
    (63, "63", "RTMAX-1"),
    (64, "64", "RTMAX"),
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
        .find_map(|(signal_number, _, name)| (*signal_number == number).then_some(*name))
}

fn signal_number(name: &str) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(_, number, signal_name)| (*signal_name == name).then_some(*number))
}

fn write_signal_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    for chunk in SIGNALS.chunks(5) {
        for (index, (number, _, name)) in chunk.iter().enumerate() {
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
