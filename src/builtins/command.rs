//! `command` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/command.def (`command_builtin`)

use crate::executor::builtin_diag::Diag;
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Complete(i32),
    Execute {
        words: Vec<String>,
        use_standard_path: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescribeMode {
    Reusable,
    Verbose,
}

/// Execute `command` with arguments after the command name.
///
/// Stage-2 ④ Phase 2: the option scan is the (table row + `BuiltinOpt` +
/// `Diag`) triple (`internal_getopt (list, "pvV")`,
/// builtins/command.def:74-75); every diagnostic speaks through `diag`.
/// The describe loop here is the FALLBACK face: the executor's own
/// describe (`execute_command_describe_with_io`, fed by
/// `parse_command_describe_args`) handles the primary `-v`/`-V` path
/// first — this loop runs only for the shapes that parser rejects —
/// so it keeps its process-env lookup and any-found rule as-is.
pub(crate) fn execute_with_io<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut W,
) -> io::Result<CommandAction>
where
    W: Write,
{
    let (use_standard_path, describe_mode, help, operands) = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut use_standard_path = false;
        let mut describe_mode = None;
        let mut help = false;
        loop {
            match opt.next() {
                OptEvent::Opt(b'p') => use_standard_path = true,
                // GNU command.def:74-75: `-v`/`-V` are plain flags and
                // the LAST one wins (assignment, not accumulation).
                OptEvent::Opt(b'v') => describe_mode = Some(DescribeMode::Reusable),
                OptEvent::Opt(b'V') => describe_mode = Some(DescribeMode::Verbose),
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // The row's spec is `"pvV"`: no other option character
                // (and no argument-taking option) can ever arrive.
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("command spec is \"pvV\"")
                }
            }
        }
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        (use_standard_path, describe_mode, help, operands)
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(CommandAction::Complete(EX_USAGE));
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok(CommandAction::Complete(diag.status()));
    }

    if operands.is_empty() {
        return Ok(CommandAction::Complete(EXECUTION_SUCCESS));
    }

    if let Some(mode) = describe_mode {
        let mut any_found = false;
        for name in operands {
            if describe_command(name, mode, use_standard_path, stdout)? {
                any_found = true;
            } else if mode == DescribeMode::Verbose {
                diag.free(format_args!("{name}: not found"));
            }
        }

        return Ok(CommandAction::Complete(if any_found {
            EXECUTION_SUCCESS
        } else {
            EXECUTION_FAILURE
        }));
    }

    Ok(CommandAction::Execute {
        words: operands.iter().map(|word| (*word).to_string()).collect(),
        use_standard_path,
    })
}

fn describe_command<W>(
    name: &str,
    mode: DescribeMode,
    use_standard_path: bool,
    stdout: &mut W,
) -> io::Result<bool>
where
    W: Write,
{
    if is_shell_builtin(name) {
        match mode {
            DescribeMode::Reusable => writeln!(stdout, "{name}")?,
            DescribeMode::Verbose => writeln!(stdout, "{name} is a shell builtin")?,
        }
        return Ok(true);
    }

    // rubash#2717: a slash-bearing name is qualified by file_status
    // alone (FS_EXECABLE; see slash_name_is_describable in
    // executor::path) and reported verbatim — the find_user_command
    // lose-on report below serves slash-free names only, whose
    // existing non-executable PATH hits GNU does report.
    if crate::executor::path::has_path_separator(name) {
        let env_vars = describe_env(use_standard_path);
        if !crate::executor::path::slash_name_is_describable(name, &env_vars) {
            return Ok(false);
        }
        match mode {
            DescribeMode::Reusable => writeln!(stdout, "{name}")?,
            DescribeMode::Verbose => writeln!(stdout, "{name} is {name}")?,
        }
        return Ok(true);
    }

    let Some(path) = find_in_path(name, use_standard_path) else {
        return Ok(false);
    };

    match mode {
        DescribeMode::Reusable => writeln!(stdout, "{}", path.display())?,
        DescribeMode::Verbose => writeln!(stdout, "{name} is {}", path.display())?,
    }

    Ok(true)
}

fn is_shell_builtin(name: &str) -> bool {
    // Stage-2 ④ Phase 2: the introspection face reads the DERIVED
    // table (D1) instead of the registry projection.
    crate::executor::builtin_table::is_table_builtin_name(name)
}

fn describe_env(use_standard_path: bool) -> HashMap<String, String> {
    let mut env_vars: HashMap<String, String> = crate::executor::env_helpers::process_env_pairs()
        .into_iter()
        .collect();
    if use_standard_path {
        env_vars.insert(
            "PATH".to_string(),
            crate::executor::path::standard_path(&env_vars),
        );
    }
    env_vars
}

fn find_in_path(name: &str, use_standard_path: bool) -> Option<PathBuf> {
    // GNU builtins/type.def:371-379 describe_command(): `command -p`
    // (CDESC_STDPATH) walks conf_standard_path(); every other mode calls
    // find_user_command (findcmd.c:247) — the same FS_EXEC_PREFERRED +
    // file_to_lose_on walk the executor uses to run commands. Delegating
    // to crate::executor::path keeps describe and execute on one lookup.
    let env_vars = describe_env(use_standard_path);
    crate::executor::path::find_user_command(name, &env_vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::builtin_diag::{Diag, Prolog};

    fn run(args: &[&str]) -> (CommandAction, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let row = crate::executor::builtin_table::builtin_table_row("command").unwrap();
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let action = {
            let prolog = Prolog {
                interactive: false,
                shell_basename: "rubash".to_string(),
                error_name: "rubash".to_string(),
                script_file: None,
                line: None,
            };
            let mut diag = Diag::new(row, &mut stderr);
            let action = execute_with_io(row, &args, &mut diag, &mut stdout).unwrap();
            diag.flush_with(|| prolog);
            action
        };

        (
            action,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn no_operands_succeeds() {
        assert_eq!(run(&[]).0, CommandAction::Complete(EXECUTION_SUCCESS));
    }

    #[test]
    fn reusable_description_reports_builtin_name() {
        let (action, stdout, stderr) = run(&["-v", "echo"]);

        assert_eq!(action, CommandAction::Complete(EXECUTION_SUCCESS));
        assert_eq!(stdout, "echo\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn reusable_description_reports_extended_builtin_names() {
        let (action, stdout, stderr) = run(&["-v", "read", "mapfile", "declare", "alias"]);

        assert_eq!(action, CommandAction::Complete(EXECUTION_SUCCESS));
        assert_eq!(stdout, "read\nmapfile\ndeclare\nalias\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_action_preserves_operands() {
        assert_eq!(
            run(&["--", "echo", "hello"]).0,
            CommandAction::Execute {
                words: vec!["echo".to_string(), "hello".to_string()],
                use_standard_path: false,
            }
        );
    }
}
