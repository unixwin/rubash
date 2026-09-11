//! `test` and `[` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/test.def (`test_builtin`)
//! - test.c
//! - test.h

#[cfg(test)]
#[path = "test_tests.rs"]
mod tests;
mod variable;

pub(crate) use variable::variable_is_set;

use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_BADUSAGE: i32 = 2;

/// Build the shell diagnostic prefix (`<script>: line N: `) from the
/// executor environment, falling back to `rubash: ` when no script context
/// is active. This mirrors GNU Bash's command-error reporting.
fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";

/// Execute `test` or `[` with arguments after the command name.
pub fn execute(
    args: &[String],
    bracket: bool,
    env_vars: &HashMap<String, String>,
) -> io::Result<i32> {
    let mut stderr = io::stderr().lock();
    execute_with_stderr(
        args.iter().map(String::as_str),
        bracket,
        env_vars,
        &mut stderr,
    )
}

fn execute_with_stderr<'a, I, W>(
    args: I,
    bracket: bool,
    env_vars: &HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let mut args: Vec<&str> = args.into_iter().collect();

    if bracket {
        match args.last() {
            Some(&"]") => {
                args.pop();
            }
            _ => {
                writeln!(stderr, "{}[: missing `]'", diagnostic_prefix(env_vars))?;
                return Ok(EX_BADUSAGE);
            }
        }
    }

    if args.is_empty() {
        return Ok(EXECUTION_FAILURE);
    }

    match eval_expr_with_bracket(&args, bracket, env_vars) {
        Ok(true) => Ok(EXECUTION_SUCCESS),
        Ok(false) => Ok(EXECUTION_FAILURE),
        Err(message) => {
            writeln!(
                stderr,
                "{}{}: {}",
                diagnostic_prefix(env_vars),
                if bracket { "[" } else { "test" },
                message
            )?;
            Ok(EX_BADUSAGE)
        }
    }
}

/// Faithful port of GNU bash-5.2 `test.c` (`posixtest`/`two_arguments`/
/// `three_arguments`/`term`/`expr`/`or`/`and` plus the leftover-argument
/// checks in `test_command`). The baseline is WSL GNU bash 5.2.21, whose
/// parser differs from bash-5.3 (5.2: term parens always re-enter expr();
/// posixtest 0/1/2/(..) cases discard leftovers via pos=argc; `-t` with a
/// non-numeric operand is FALSE; `!` in two_arguments does not advance
/// pos). Malformed expressions report GNU's specific diagnostics
/// (`unary operator expected`, ``syntax error: `-ne' unexpected``,
/// `too many arguments`, ```)' expected``, ...) instead of a generic
/// `syntax error`.
struct TestParser<'a> {
    args: &'a [&'a str],
    pos: usize,
    bracket: bool,
    env_vars: &'a HashMap<String, String>,
}

impl TestParser<'_> {
    /// 5.2 test.c counts `argc` including the command word (`test`/`[`), and
    /// `pos` starts at 1 (past it), so argc = args.len() + 1 and argv[argc]
    /// is one past the end (NULL in C).
    fn argc(&self) -> usize {
        self.args.len() + 1
    }

    /// argv[i] in 5.2 terms: argv[0] is the command word, argv[i] = args[i-1].
    fn argv(&self, i: usize) -> Option<&str> {
        if i == 0 {
            Some("test")
        } else if i - 1 < self.args.len() {
            Some(self.args[i - 1])
        } else {
            None
        }
    }

    /// The current token (argv[pos]); only valid while pos < argc.
    fn cur(&self) -> &str {
        self.args[self.pos - 1]
    }

    /// GNU `advance(f)`: `++pos`; if `f` and we moved past the end, report
    /// `argument expected`.
    fn advance(&mut self, check: bool) -> Result<(), String> {
        self.pos += 1;
        if check && self.pos >= self.argc() {
            return Err("argument expected".to_string());
        }
        Ok(())
    }

    /// 5.2 `posixtest()`: dispatch on the number of real arguments
    /// (`argc - 1`); the 0/1/2 and `(...)` cases discard any leftovers by
    /// setting pos = argc.
    fn posixtest(&mut self) -> Result<bool, String> {
        match self.argc() - 1 {
            0 => {
                self.pos = self.argc();
                Ok(false)
            }
            1 => {
                let value = !self.argv(1).unwrap_or("").is_empty();
                self.pos = self.argc();
                Ok(value)
            }
            2 => {
                let value = self.two_arguments()?;
                self.pos = self.argc();
                Ok(value)
            }
            3 => self.three_arguments(),
            4 => {
                if self.cur() == "!" {
                    self.advance(true)?;
                    let value = self.three_arguments()?;
                    Ok(!value)
                } else if self.cur() == "(" && self.argv(self.argc() - 1) == Some(")") {
                    self.advance(true)?;
                    let value = self.two_arguments()?;
                    self.pos = self.argc();
                    Ok(value)
                } else {
                    self.expr()
                }
            }
            _ => self.expr(),
        }
    }

    /// 5.2 `two_arguments()`: the `!` form reads its operand but does not
    /// advance pos (the callers set pos = argc to discard leftovers).
    fn two_arguments(&mut self) -> Result<bool, String> {
        if self.cur() == "!" {
            return Ok(self.argv(self.pos + 1).unwrap_or("").is_empty());
        }
        let first = self.cur();
        // Single-letter `-X` form: a valid unary runs it; anything else
        // (including `-A` and long options like `-eq`) is `unary operator
        // expected` on that token.
        if first.starts_with('-') && first.len() == 2 {
            if is_unary_operator(first) {
                return self.unary_operator();
            }
            return Err(format!("{}: unary operator expected", first));
        }
        Err(format!("{}: unary operator expected", first))
    }

    /// 5.2 `three_arguments()`.
    fn three_arguments(&mut self) -> Result<bool, String> {
        let middle = self.argv(self.pos + 1).unwrap_or("");
        if is_binary_operator(middle) {
            let value = self.binary_operator()?;
            self.pos = self.argc();
            return Ok(value);
        }
        if middle == "-a" || middle == "-o" {
            let left = !self.cur().is_empty();
            let right = !self.argv(self.pos + 2).unwrap_or("").is_empty();
            let value = if middle == "-a" {
                left && right
            } else {
                left || right
            };
            self.pos = self.argc();
            return Ok(value);
        }
        if self.cur() == "!" {
            self.advance(true)?;
            let value = self.two_arguments()?;
            self.pos = self.argc();
            return Ok(!value);
        }
        if self.cur() == "(" && self.argv(self.pos + 2) == Some(")") {
            let value = !self.argv(self.pos + 1).unwrap_or("").is_empty();
            self.pos = self.argc();
            return Ok(value);
        }
        Err(format!("{}: binary operator expected", middle))
    }

    /// 5.2 `unary_operator()`: `-t` with a non-numeric operand is FALSE
    /// (not an error); every other unary needs exactly one operand.
    fn unary_operator(&mut self) -> Result<bool, String> {
        let op = self.cur();
        if op == "-t" {
            self.advance(false)?;
            if self.pos < self.argc() {
                if self.cur().parse::<i64>().is_ok() {
                    let operand = self.cur().to_string();
                    self.advance(false)?;
                    return eval_unary("-t", &operand, self.env_vars);
                }
                return Ok(false);
            }
            return eval_unary("-t", "1", self.env_vars);
        }
        if self.pos + 1 >= self.argc() {
            return Err("argument expected".to_string());
        }
        let operand = self.argv(self.pos + 1).unwrap_or("");
        let value = eval_unary(op, operand, self.env_vars)?;
        self.pos += 2;
        Ok(value)
    }

    /// 5.2 `binary_operator()`.
    fn binary_operator(&mut self) -> Result<bool, String> {
        let left = self.cur();
        let op = self.argv(self.pos + 1).unwrap_or("");
        let right = self.argv(self.pos + 2).unwrap_or("");
        let value = eval_binary(left, op, right, self.env_vars)?;
        self.pos += 3;
        Ok(value)
    }

    /// 5.2 `expr()` -> `or()` -> `and()` -> `term()`.
    fn expr(&mut self) -> Result<bool, String> {
        if self.pos >= self.argc() {
            return Err("argument expected".to_string());
        }
        self.or()
    }

    fn or(&mut self) -> Result<bool, String> {
        let mut value = self.and()?;
        while self.pos < self.argc() && self.cur() == "-o" {
            self.advance(false)?;
            // 5.2 always parses the right operand (errors propagate) even
            // when the left value would short-circuit the boolean result.
            let v2 = self.or()?;
            value = value || v2;
        }
        Ok(value)
    }

    fn and(&mut self) -> Result<bool, String> {
        let mut value = self.term()?;
        while self.pos < self.argc() && self.cur() == "-a" {
            self.advance(false)?;
            let v2 = self.and()?;
            value = value && v2;
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<bool, String> {
        if self.pos >= self.argc() {
            return Err("argument expected".to_string());
        }
        // Leading `!`s toggle the result of the following term.
        if self.cur() == "!" {
            let mut negate = false;
            while self.pos < self.argc() && self.cur() == "!" {
                self.advance(true)?;
                negate = !negate;
            }
            let inner = self.term()?;
            return Ok(if negate { !inner } else { inner });
        }
        // Parenthesized expression: 5.2 always re-enters expr() (no short
        // arity fast-path).
        if self.cur() == "(" {
            self.advance(true)?;
            let value = self.expr()?;
            // After the sub-expression the closing token must be a `)`. For
            // `[ ... ]` the already-consumed `]` is reported as the offender.
            if self.pos >= self.argc() {
                if self.bracket {
                    return Err("`)' expected, found ]".to_string());
                }
                return Err("`)' expected".to_string());
            }
            if self.cur() != ")" {
                return Err(format!("`)' expected, found {}", self.cur()));
            }
            self.advance(false)?;
            return Ok(value);
        }
        // Binary, then unary, then a plain string term.
        if self.pos + 3 <= self.argc() && is_binary_operator(self.argv(self.pos + 1).unwrap_or(""))
        {
            return self.binary_operator();
        }
        if self.pos + 2 <= self.argc() && is_unary_operator(self.cur()) {
            return self.unary_operator();
        }
        let value = !self.cur().is_empty();
        self.advance(false)?;
        Ok(value)
    }
}

fn eval_expr_with_bracket(
    args: &[&str],
    bracket: bool,
    env_vars: &HashMap<String, String>,
) -> Result<bool, String> {
    let mut parser = TestParser {
        args,
        pos: 1,
        bracket,
        env_vars,
    };
    let value = parser.posixtest()?;
    // 5.2 test_command: any arguments not consumed by the parse are
    // reported as `syntax error: `X' unexpected` (option-like) or
    // `too many arguments`.
    if parser.pos != parser.argc() {
        if parser.pos < parser.argc() && parser.cur().starts_with('-') {
            return Err(format!("syntax error: `{}' unexpected", parser.cur()));
        }
        return Err("too many arguments".to_string());
    }
    Ok(value)
}

fn is_unary_operator(op: &str) -> bool {
    matches!(
        op,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-h"
            | "-L"
            | "-k"
            | "-p"
            | "-r"
            | "-s"
            | "-S"
            | "-t"
            | "-u"
            | "-w"
            | "-x"
            | "-z"
            | "-n"
            | "-o"
            | "-v"
            | "-R"
            | "-O"
            | "-G"
            | "-N"
    )
}

fn eval_unary(op: &str, operand: &str, env_vars: &HashMap<String, String>) -> Result<bool, String> {
    if let Some(result) = virtual_device_test(op, operand) {
        return Ok(result);
    }

    match op {
        "-z" => Ok(operand.is_empty()),
        "-n" => Ok(!operand.is_empty()),
        "-o" => Ok(crate::builtins::set::is_shell_option(operand)
            && crate::builtins::set::shell_option_enabled(env_vars, operand)),
        "-v" => Ok(variable_is_set(operand, env_vars)),
        "-R" => {
            let namerefs = marked_vars(env_vars, NAMEREF_VARS);
            Ok(namerefs.iter().any(|name| name == operand))
        }
        "-a" | "-e" => Ok(test_path(operand, env_vars).exists()),
        "-d" => Ok(test_path(operand, env_vars).is_dir()),
        "-f" => Ok(test_path(operand, env_vars).is_file()),
        "-h" | "-L" => Ok(fs::symlink_metadata(test_path(operand, env_vars))
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)),
        "-s" => Ok(fs::metadata(test_path(operand, env_vars))
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)),
        "-r" | "-w" | "-x" => {
            // chmod through the emulated POSIX mode layer (Windows has no
            // mode bits) takes precedence, like GNU stat'ing the real mode.
            let bit = match op {
                "-r" => 0o400,
                "-w" => 0o200,
                _ => 0o100,
            };
            match emulated_file_mode(operand, env_vars) {
                Some(mode) => Ok(mode & bit != 0),
                None => Ok(test_path(operand, env_vars).exists()),
            }
        }
        "-O" => Ok(file_owned_by_effective_user(operand, env_vars)),
        "-G" => Ok(file_owned_by_effective_group(operand, env_vars)),
        "-N" => Ok(modified_since_last_read(operand, env_vars)),
        "-b" => Ok(file_type_matches(
            operand,
            env_vars,
            UnixFileKind::BlockDevice,
        )),
        "-c" => Ok(file_type_matches(
            operand,
            env_vars,
            UnixFileKind::CharDevice,
        )),
        "-p" => Ok(file_type_matches(operand, env_vars, UnixFileKind::Fifo)),
        "-S" => Ok(file_type_matches(operand, env_vars, UnixFileKind::Socket)),
        "-u" => Ok(file_mode_has_bit(operand, env_vars, 0o4000)),
        "-g" => Ok(file_mode_has_bit(operand, env_vars, 0o2000)),
        "-k" => Ok(file_mode_has_bit(operand, env_vars, 0o1000)),
        "-t" => Ok(fd_is_terminal(operand)),
        _ => Err(format!("{}: unary operator expected", op)),
    }
}

fn virtual_device_test(op: &str, operand: &str) -> Option<bool> {
    if crate::executor::path::is_shell_null_device(operand) {
        return Some(match op {
            "-a" | "-e" | "-r" | "-w" | "-c" => true,
            "-x" | "-s" | "-b" | "-p" | "-S" | "-u" | "-g" | "-k" | "-h" | "-L" | "-O" | "-G"
            | "-N" => false,
            _ => return None,
        });
    }

    if matches!(operand, "/dev/stdin" | "/proc/self/fd/0" | "/dev/fd/0") {
        return Some(match op {
            "-a" | "-e" | "-r" | "-c" => true,
            "-w" | "-x" | "-s" | "-b" | "-p" | "-S" | "-u" | "-g" | "-k" | "-h" | "-L" | "-O"
            | "-G" | "-N" => false,
            _ => return None,
        });
    }

    None
}

fn test_path(operand: &str, env_vars: &HashMap<String, String>) -> std::path::PathBuf {
    crate::executor::path::shell_path_to_windows_for_lookup(operand, env_vars)
}

fn marked_vars(env_vars: &HashMap<String, String>, key: &str) -> Vec<String> {
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

fn is_binary_operator(op: &str) -> bool {
    matches!(
        op,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-nt"
            | "-ot"
            | "-ef"
    )
}

fn eval_binary(
    left: &str,
    op: &str,
    right: &str,
    env_vars: &HashMap<String, String>,
) -> Result<bool, String> {
    match op {
        "=" | "==" => Ok(left == right),
        "!=" => Ok(left != right),
        "<" => Ok(left < right),
        ">" => Ok(left > right),
        "-eq" => Ok(parse_int(left)? == parse_int(right)?),
        "-ne" => Ok(parse_int(left)? != parse_int(right)?),
        "-lt" => Ok(parse_int(left)? < parse_int(right)?),
        "-le" => Ok(parse_int(left)? <= parse_int(right)?),
        "-gt" => Ok(parse_int(left)? > parse_int(right)?),
        "-ge" => Ok(parse_int(left)? >= parse_int(right)?),
        "-nt" => Ok(modified(left, env_vars) > modified(right, env_vars)),
        "-ot" => Ok(modified(left, env_vars) < modified(right, env_vars)),
        "-ef" => Ok(same_file(left, right, env_vars)),
        _ => Err(format!("{}: binary operator expected", op)),
    }
}

fn parse_int(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| format!("{}: integer expression expected", value))
}

fn modified(path: &str, env_vars: &HashMap<String, String>) -> Option<std::time::SystemTime> {
    fs::metadata(test_path(path, env_vars))
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn modified_since_last_read(path: &str, env_vars: &HashMap<String, String>) -> bool {
    let Ok(metadata) = fs::metadata(test_path(path, env_vars)) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return true;
    };
    let Ok(accessed) = metadata.accessed() else {
        return true;
    };
    modified >= accessed
}

fn fd_is_terminal(operand: &str) -> bool {
    let Ok(fd) = operand.parse::<i32>() else {
        return false;
    };
    match fd {
        0 => io::stdin().is_terminal(),
        1 => io::stdout().is_terminal(),
        2 => io::stderr().is_terminal(),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum UnixFileKind {
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

#[cfg(unix)]
fn file_type_matches(path: &str, env_vars: &HashMap<String, String>, kind: UnixFileKind) -> bool {
    use std::os::unix::fs::FileTypeExt;

    let Ok(metadata) = fs::metadata(test_path(path, env_vars)) else {
        return false;
    };
    let file_type = metadata.file_type();
    match kind {
        UnixFileKind::BlockDevice => file_type.is_block_device(),
        UnixFileKind::CharDevice => file_type.is_char_device(),
        UnixFileKind::Fifo => file_type.is_fifo(),
        UnixFileKind::Socket => file_type.is_socket(),
    }
}

#[cfg(not(unix))]
fn file_type_matches(
    _path: &str,
    _env_vars: &HashMap<String, String>,
    _kind: UnixFileKind,
) -> bool {
    false
}

#[cfg(unix)]
fn file_mode_has_bit(path: &str, env_vars: &HashMap<String, String>, bit: u32) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = fs::metadata(test_path(path, env_vars)) else {
        return false;
    };
    metadata.permissions().mode() & bit != 0
}

#[cfg(not(unix))]
fn file_mode_has_bit(_path: &str, _env_vars: &HashMap<String, String>, _bit: u32) -> bool {
    false
}

#[cfg(unix)]
fn file_owned_by_effective_user(path: &str, env_vars: &HashMap<String, String>) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(metadata) = fs::metadata(test_path(path, env_vars)) else {
        return false;
    };
    metadata.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn file_owned_by_effective_user(path: &str, env_vars: &HashMap<String, String>) -> bool {
    test_path(path, env_vars).exists()
}

#[cfg(unix)]
fn file_owned_by_effective_group(path: &str, env_vars: &HashMap<String, String>) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(metadata) = fs::metadata(test_path(path, env_vars)) else {
        return false;
    };
    metadata.gid() == unsafe { libc::getegid() }
}

#[cfg(not(unix))]
fn file_owned_by_effective_group(path: &str, env_vars: &HashMap<String, String>) -> bool {
    test_path(path, env_vars).exists()
}

fn same_file(left: &str, right: &str, env_vars: &HashMap<String, String>) -> bool {
    let Ok(left) = fs::canonicalize(test_path(left, env_vars)) else {
        return false;
    };
    let Ok(right) = fs::canonicalize(test_path(right, env_vars)) else {
        return false;
    };
    left == right
}

pub(crate) const EMULATED_FILE_MODES: &str = "__RUBASH_FILE_MODES";

/// The mode recorded by the emulated chmod for this path, if any. Entries are
/// stored as windows-path=octal pairs separated by the unit separator.
pub(crate) fn emulated_file_mode(operand: &str, env_vars: &HashMap<String, String>) -> Option<u32> {
    let windows = test_path(operand, env_vars).to_string_lossy().to_string();
    let entries = env_vars.get(EMULATED_FILE_MODES)?;
    for entry in entries.split('\x1f') {
        let (path, mode) = entry.rsplit_once('=')?;
        if path == windows {
            return u32::from_str_radix(mode, 8).ok();
        }
    }
    None
}
