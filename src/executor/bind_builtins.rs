//! `bind` builtin execution and the host-facing bind -x runner.
//!
//! GNU ownership: builtins/bind.def bind_builtin (option grammar and
//! dispatch order) + bashline.c:4593 bash_execute_unix_command (the
//! READLINE_LINE/READLINE_POINT/READLINE_MARK protocol around the
//! command execution) + bashline.c:4850 bind_keyseq_to_unix_command
//! (registration). The pure parsing/listing half lives in
//! [`crate::builtins::bind`]; this file holds the stateful half.
//!
//! reedline-upstream list (wt100 matrix rows that stay open because the
//! HOST line editor must implement the behavior — the engine side of
//! each is done here): the `-p`/`-P`/`-l` FULL GNU tables (reedline's
//! keymaps cover a subset of GNU's 176 function names — listing more
//! would claim bindings no host can deliver); multi-key sequences beyond
//! what a host's key parser accepts; and APPLYING the readline variable
//! table's behavior inside an editor (`editing-mode`/`keymap` are
//! consumed by hosts today; the rest of the 48-variable table is a data
//! plane hosts read, not an emulation).

use std::io::Write;

use crate::parser::CommandNode;
use crate::shell::bind_registry::{BindEntry, BindKind};

use super::{ExecuteError, Executor};

/// Result of running a `bind -x` command on a keypress
/// (bashline.c:4593-4707). `line`/`point`/`mark` are the READLINE_*
/// variables read back after the command; `None` means the command left
/// them unset, which GNU treats as "keep the editor state as it was"
/// (bashline.c:4692-4700 only consults the variables when present).
/// READLINE_POINT/READLINE_MARK are CHARACTER offsets
/// (bashline.c:4540 readline_get_char_offset uses MB_STRLEN), clamped to
/// [0, buffer length] on read-back (readline_set_char_offset).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindXRunOutcome {
    pub line: Option<String>,
    pub point: Option<usize>,
    pub mark: Option<usize>,
    /// The command's exit status. GNU redraws the edit line
    /// unconditionally when it is 124 (bashline.c:4701-4706 comment);
    /// hosts that always repaint after a widget can ignore it.
    pub status: i32,
}

#[derive(Debug, Default, Clone)]
struct BindFlags {
    list_functions: bool,
    dump_readable: bool,
    dump_plain: bool,
    vars_readable: bool,
    vars_plain: bool,
    macros_readable: bool,
    macros_plain: bool,
    register_execute: bool,
    list_execute: bool,
    file: Option<String>,
    query: Option<String>,
    unbind_function: Option<String>,
    keymap: Option<String>,
    remove: Option<String>,
    execute_spec: Option<String>,
}

impl Executor {
    pub(in crate::executor) fn execute_bind(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let args: Vec<String> = cmd.words[1..].to_vec();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let prefix = self.diagnostic_prefix();
        let line_editing_enabled = self
            .get_env("__RUBASH_INTERACTIVE")
            .is_some_and(|value| value == "1");
        let status = self.bind_dispatch(
            &args,
            line_editing_enabled,
            &prefix,
            &mut stdout,
            &mut stderr,
        );
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    /// bind.def:114 bind_builtin. Dispatch order mirrors the C: listings
    /// (-l/-p/-P/-s/-S/-v/-V), then -f, then -q/-u/-r, then -x, then -X,
    /// then the remaining arguments as inputrc-form bindings
    /// (bind.def:229-330).
    fn bind_dispatch(
        &mut self,
        args: &[String],
        line_editing_enabled: bool,
        prefix: &str,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        // bind.def:127-131: the line-editing warning prints first, then
        // initialize_readline runs ONCE per process (bashline.c:278
        // bash_readline_initialized) — on the first `bind` invocation in
        // ANY shell mode, which is what makes $INPUTRC/~/.inputrc apply
        // to `bind -v` even under `-c` (GNU probe: INPUTRC +
        // `bash --norc -c 'bind -v'` shows the file's variables).
        if !line_editing_enabled {
            let _ = writeln!(stderr, "{prefix}bind: warning: line editing not enabled");
        }
        if !self.bind_registry().borrow().readline_initialized {
            self.initialize_readline();
        }

        let mut flags = BindFlags::default();
        let mut return_code: Option<i32> = None;
        match self.parse_bind_options(args, &mut flags, stderr) {
            Ok(()) => {}
            Err(status) => return status,
        }

        // -m: install the keymap for the duration of this command
        // (bind.def:246-257). An invalid name fails the whole command.
        if let Some(name) = &flags.keymap {
            if !crate::shell::bind_registry::KEYMAP_NAMES.contains(&name.as_str()) {
                let _ = writeln!(
                    stderr,
                    "{prefix}bind: {}",
                    crate::builtins::bind::format_invalid_keymap(name)
                );
                return 1;
            }
        }
        let keymap = flags.keymap.clone();

        if flags.list_functions {
            for name in crate::builtins::bind::READLINE_FUNCTIONS {
                let _ = writeln!(stdout, "{name}");
            }
        }

        if flags.dump_readable {
            // rl_function_dumper(1): readable bindings, defaults first
            // then user-registered function bindings for this keymap.
            let entries = self.bind_registry_snapshot();
            for (seq, name) in crate::builtins::bind::DEFAULT_FUNCTION_BINDINGS {
                let _ = writeln!(
                    stdout,
                    "{}",
                    crate::builtins::bind::format_function_readable(seq, name)
                );
            }
            for entry in entries.iter().filter(|e| {
                matches!(&e.kind, BindKind::Function { .. })
                    && same_keymap(e.keymap.as_deref(), keymap.as_deref())
            }) {
                if let BindKind::Function { name } = &entry.kind {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        crate::builtins::bind::format_function_readable(&entry.keyseq, name)
                    );
                }
            }
        }

        if flags.dump_plain {
            // rl_function_dumper(0): `name can be found on "seq", "seq".`
            let mut by_name: Vec<(String, Vec<String>)> = Vec::new();
            for (seq, name) in crate::builtins::bind::DEFAULT_FUNCTION_BINDINGS {
                push_keyseq(&mut by_name, name, seq.to_string());
            }
            for entry in self.bind_registry_snapshot() {
                if let BindKind::Function { name } = &entry.kind {
                    if same_keymap(entry.keymap.as_deref(), keymap.as_deref()) {
                        push_keyseq(&mut by_name, name, entry.keyseq.clone());
                    }
                }
            }
            for (name, seqs) in by_name {
                let _ = writeln!(
                    stdout,
                    "{}",
                    crate::builtins::bind::format_function_plain(&name, &seqs)
                );
            }
        }

        if flags.macros_readable || flags.macros_plain {
            let entries: Vec<_> = self
                .bind_registry_snapshot()
                .into_iter()
                .filter(|e| {
                    matches!(&e.kind, BindKind::Macro { .. })
                        && same_keymap(e.keymap.as_deref(), keymap.as_deref())
                })
                .collect();
            for entry in entries {
                if let BindKind::Macro { text } = &entry.kind {
                    let line = if flags.macros_readable {
                        crate::builtins::bind::format_macro_readable(&entry.keyseq, text)
                    } else {
                        crate::builtins::bind::format_macro_plain(&entry.keyseq, text)
                    };
                    let _ = writeln!(stdout, "{line}");
                }
            }
        }

        // -v/-V: readline variable dump. GNU prints its 48-line variable
        // table (probe: `bind -v | wc -l` = 48) — `-v` as readable
        // `set NAME VALUE` lines, `-V` as ``NAME is set to `VALUE'``
        // (rl_variable_dumper). The table lives in the registry: inputrc
        // `set` lines and `bind 'set …'` arguments update it, hosts read
        // it to mirror the variables whose behavior they implement
        // (editing-mode first among them).
        if flags.vars_readable || flags.vars_plain {
            let registry = self.bind_registry();
            for (name, value) in registry.borrow().variables.iter() {
                let line = if flags.vars_readable {
                    crate::builtins::bind::format_variable_readable(name, value)
                } else {
                    crate::builtins::bind::format_variable_plain(name, value)
                };
                let _ = writeln!(stdout, "{line}");
            }
        }

        if let Some(file) = &flags.file {
            // bind.def:268-282: read the file; failure is
            // `bind: FILE: cannot read: ERRNO` with rc 1.
            match std::fs::read_to_string(file) {
                Ok(text) => {
                    let term = self.get_env("TERM").map(str::to_owned);
                    let editing_mode = self.bind_editing_mode();
                    let env = crate::builtins::bind::InputrcEnv {
                        term: term.as_deref(),
                        editing_mode: editing_mode.as_deref(),
                    };
                    let parsed = crate::builtins::bind::parse_inputrc_with_env(
                        &text,
                        file,
                        &env,
                        &mut |name| std::fs::read_to_string(name).ok(),
                    );
                    for diagnostic in &parsed.diagnostics {
                        let _ = writeln!(
                            stderr,
                            "readline: {}: line {}: {}",
                            diagnostic.file, diagnostic.line, diagnostic.core
                        );
                    }
                    {
                        let registry_handle = self.bind_registry();
                        let mut registry = registry_handle.borrow_mut();
                        for (name, value) in &parsed.variables {
                            registry.set_readline_variable(name, value);
                        }
                        for (keyseq, spec) in &parsed.bindings {
                            let kind = match spec {
                                crate::builtins::bind::BindingSpec::Function { name } => {
                                    BindKind::Function { name: name.clone() }
                                }
                                crate::builtins::bind::BindingSpec::Macro { text } => {
                                    BindKind::Macro { text: text.clone() }
                                }
                            };
                            registry.register(BindEntry {
                                keyseq: keyseq.clone(),
                                kind,
                                keymap: keymap.clone(),
                            });
                        }
                    }
                }
                Err(err) => {
                    let _ = writeln!(
                        stderr,
                        "{prefix}bind: {}",
                        crate::builtins::bind::format_cannot_read(file, &err.to_string())
                    );
                    return 1;
                }
            }
        }

        if let Some(name) = &flags.query {
            // query_bindings (bind.def:344-367) — sets the return code
            // and processing continues (bind.def:285 keeps going).
            if !crate::builtins::bind::is_known_function(name) {
                let _ = writeln!(
                    stderr,
                    "{prefix}bind: {}",
                    crate::builtins::bind::format_unknown_function(name)
                );
                return_code = Some(1);
            } else {
                let mut seqs: Vec<String> = crate::builtins::bind::DEFAULT_FUNCTION_BINDINGS
                    .iter()
                    .filter(|(_, bound)| bound == name)
                    .map(|(seq, _)| seq.to_string())
                    .collect();
                if name == "self-insert" {
                    // self-insert is bound to every printable character in
                    // the default keymaps; query_bindings lists the first
                    // five and truncates (GNU probe: `self-insert can be
                    // invoked via " ", "!", "\"", "#", "$", ...`).
                    seqs = vec![
                        " ".to_string(),
                        "!".to_string(),
                        "\"".to_string(),
                        "#".to_string(),
                        "$".to_string(),
                        "%".to_string(),
                    ];
                }
                for entry in self.bind_registry_snapshot() {
                    if let BindKind::Function { name: bound } = &entry.kind {
                        if bound == name && same_keymap(entry.keymap.as_deref(), keymap.as_deref())
                        {
                            seqs.push(entry.keyseq.clone());
                        }
                    }
                }
                if seqs.is_empty() {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        crate::builtins::bind::format_query_unbound(name)
                    );
                    return_code = Some(1);
                } else {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        crate::builtins::bind::format_query(name, &seqs)
                    );
                }
            }
        }

        if let Some(name) = &flags.unbind_function {
            // unbind_command (bind.def:375-393) — unknown names fail;
            // processing continues.
            if !crate::builtins::bind::is_known_function(name) {
                let _ = writeln!(
                    stderr,
                    "{prefix}bind: {}",
                    crate::builtins::bind::format_unknown_function(name)
                );
                return_code = Some(1);
            } else {
                self.unbind_function_for_keymap(name, keymap.as_deref());
            }
        }

        if let Some(keyseq) = &flags.remove {
            // unbind_keyseq (bind.def:395-421) BIND_RETURNs: silent
            // success whether or not anything was bound, then done.
            self.unbind_keyseq_in_keymap(keyseq, keymap.as_deref());
            return 0;
        }

        if let Some(spec) = &flags.execute_spec {
            // bind_keyseq_to_unix_command (bashline.c:4850). A parse
            // error sets the failure code but processing continues
            // (bind.def:296-298).
            match crate::builtins::bind::parse_unix_command_spec(spec) {
                Ok((keyseq, command)) => {
                    self.register_bind_entry(BindEntry {
                        keyseq,
                        kind: BindKind::Execute { command },
                        keymap: keymap.clone(),
                    });
                }
                Err(err) => {
                    let _ = writeln!(stderr, "{prefix}bind: {}", err.message);
                    return_code = Some(1);
                }
            }
        }

        if flags.list_execute {
            // print_unix_command_map (bashline.c:4729).
            for entry in self.bind_registry_snapshot().into_iter().filter(|e| {
                matches!(&e.kind, BindKind::Execute { .. })
                    && same_keymap(e.keymap.as_deref(), keymap.as_deref())
            }) {
                if let BindKind::Execute { command } = &entry.kind {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        crate::builtins::bind::format_unix_command_readable(&entry.keyseq, command)
                    );
                }
            }
        }

        // Remaining arguments are inputrc-form binding specifications
        // (bind.def:304) handed to rl_parse_and_bind one by one. GNU
        // semantics probed on WSL bash 5.3.0:
        // - `set NAME VALUE` arguments update the readline variable table
        //   (`bind 'set completion-ignore-case on'` → visible in
        //   `bind -v`); unknown/invalid values print
        //   ``readline: NAME: unknown variable name`` /
        //   ```readline: NAME: could not set value to `V'``` and keep
        //   going.
        // - non-quote, non-set text is silently ignored (`bind 'ls -l'`
        //   → rc 0, no output).
        // - parse failures print `readline: …` and do NOT fail the
        //   command (bind.def:304-333 ignores rl_parse_and_bind's return;
        //   probe: `bind '"\C-t'` → rc 0).
        let mut remaining = Vec::new();
        collect_positional(args, &mut remaining);
        for arg in remaining {
            if let Some(rest) = crate::builtins::bind::strip_set_keyword(&arg) {
                let mut parts = rest.trim().splitn(2, char::is_whitespace);
                let name = parts.next().unwrap_or_default();
                let value = parts.next().unwrap_or_default().trim();
                if name.is_empty() {
                    continue;
                }
                let outcome = self
                    .bind_registry()
                    .borrow_mut()
                    .set_readline_variable(name, value);
                match outcome {
                    crate::builtins::bind::SetVariableOutcome::Applied => {}
                    crate::builtins::bind::SetVariableOutcome::UnknownVariable => {
                        let _ = writeln!(stderr, "readline: {name}: unknown variable name");
                    }
                    crate::builtins::bind::SetVariableOutcome::CouldNotSetValue => {
                        let _ =
                            writeln!(stderr, "readline: {name}: could not set value to `{value}'");
                    }
                }
                continue;
            }
            if !arg.trim_start().starts_with('"') {
                continue;
            }
            match crate::builtins::bind::parse_positional_binding(&arg) {
                Ok(parsed) => {
                    let kind = match parsed.spec {
                        crate::builtins::bind::BindingSpec::Function { name } => {
                            BindKind::Function { name }
                        }
                        crate::builtins::bind::BindingSpec::Macro { text } => {
                            BindKind::Macro { text }
                        }
                    };
                    self.register_bind_entry(BindEntry {
                        keyseq: parsed.keyseq,
                        kind,
                        keymap: keymap.clone(),
                    });
                }
                Err(err) => {
                    if let Some(readline_message) = err.readline_message {
                        let _ = writeln!(stderr, "readline: {readline_message}");
                    } else {
                        let _ = writeln!(stderr, "{prefix}bind: {}", err.message);
                        return_code = Some(1);
                    }
                }
            }
        }

        return_code.unwrap_or(0)
    }

    /// internal_getopt port for bind's option string
    /// `"lvpVPsSXf:q:u:m:r:x:"` (bind.def:164). Bundled short options are
    /// allowed; option arguments may be attached (`-x'...'`) or separate.
    fn parse_bind_options(
        &self,
        args: &[String],
        flags: &mut BindFlags,
        stderr: &mut Vec<u8>,
    ) -> Result<(), i32> {
        let prefix = self.diagnostic_prefix();
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if arg == "--" {
                return Ok(());
            }
            if !arg.starts_with('-') || arg == "-" {
                return Ok(());
            }
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut offset = 0;
            while offset < chars.len() {
                let option = chars[offset];
                let takes_value = matches!(option, 'f' | 'q' | 'u' | 'm' | 'r' | 'x');
                match option {
                    'l' => flags.list_functions = true,
                    'p' => flags.dump_readable = true,
                    'P' => flags.dump_plain = true,
                    'v' => flags.vars_readable = true,
                    'V' => flags.vars_plain = true,
                    's' => flags.macros_readable = true,
                    'S' => flags.macros_plain = true,
                    'X' => flags.list_execute = true,
                    'f' | 'q' | 'u' | 'm' | 'r' => {}
                    // -x both sets the flag and takes a value; the value
                    // consumption below handles the argument.
                    'x' => flags.register_execute = true,
                    other => {
                        let _ = writeln!(stderr, "{prefix}bind: -{other}: invalid option");
                        let _ = crate::builtins::bind::write_usage(stderr);
                        return Err(2);
                    }
                }
                if takes_value {
                    let rest: String = chars[offset + 1..].iter().collect();
                    let value = if !rest.is_empty() {
                        rest
                    } else {
                        index += 1;
                        match args.get(index) {
                            Some(value) => value.clone(),
                            None => {
                                let _ = writeln!(
                                    stderr,
                                    "{prefix}bind: -{option}: option requires an argument"
                                );
                                let _ = crate::builtins::bind::write_usage(stderr);
                                return Err(2);
                            }
                        }
                    };
                    match option {
                        'f' => flags.file = Some(value),
                        'q' => flags.query = Some(value),
                        'u' => flags.unbind_function = Some(value),
                        'm' => flags.keymap = Some(value),
                        'r' => flags.remove = Some(value),
                        'x' => flags.execute_spec = Some(value),
                        _ => unreachable!("takes_value set covers these"),
                    }
                    break;
                }
                offset += 1;
            }
            index += 1;
        }
        Ok(())
    }

    /// Run a `bind -x` command on behalf of the host line editor
    /// (bashline.c:4593 bash_execute_unix_command).
    ///
    /// Protocol, straight from the C:
    /// 1. bind READLINE_LINE = current line, READLINE_POINT /
    ///    READLINE_MARK = CHARACTER offsets, all EXPORTED so the command
    ///    and its children see them (bashline.c:4653-4673
    ///    `VSETATTR (v, att_exported)`).
    /// 2. parse_and_execute the command (bashline.c:4683; SEVAL_NOHIST —
    ///    a widget keystroke never enters the history list).
    /// 3. read the variables back: READLINE_LINE replaces the editor
    ///    buffer when different (maybe_make_readline_line,
    ///    bashline.c:2808); READLINE_POINT/READLINE_MARK are clamped to
    ///    [0, buffer length] (readline_set_char_offset, bashline.c:4559).
    /// 4. UNBIND all READLINE_* variables — they exist only for the
    ///    duration of the command (unbind_readline_variables,
    ///    bashline.c:4573).
    ///
    /// `point` and `mark` are character offsets into `line`; read-back
    /// offsets are character offsets into the (possibly new) line.
    pub fn run_bind_x_command(
        &mut self,
        command: &str,
        line: &str,
        point: usize,
        mark: usize,
    ) -> BindXRunOutcome {
        self.set_env("READLINE_LINE", line);
        self.set_env("READLINE_POINT", &point.to_string());
        self.set_env("READLINE_MARK", &mark.to_string());

        let status = crate::script_driver::run_source(self, command, false);

        let length = read_back_line_length(self);
        let out_line = self.get_env("READLINE_LINE").map(str::to_owned);
        let clamp = |value: Option<&str>| -> Option<usize> {
            value
                .and_then(|value| value.trim().parse::<usize>().ok())
                .map(|offset| offset.min(length))
        };
        let outcome = BindXRunOutcome {
            line: out_line,
            point: clamp(self.get_env("READLINE_POINT")),
            mark: clamp(self.get_env("READLINE_MARK")),
            status,
        };

        // unbind_readline_variables (bashline.c:4573-4581): the variables
        // are removed entirely, from the shell-variable view and the
        // process environment.
        for name in ["READLINE_LINE", "READLINE_POINT", "READLINE_MARK"] {
            let _ = self.shell_state.variables.unset(name);
            self.unset_env(name);
        }
        outcome
    }

    /// Registry snapshot for hosts (niubash's reedline bridge).
    pub fn bind_registry_snapshot(&self) -> Vec<BindEntry> {
        self.shell_state.bind_registry.borrow().snapshot()
    }

    /// Current registry generation (bumped on every mutation). Hosts
    /// compare it between prompts to notice runtime `bind` calls and
    /// rebuild their mirrored keymaps.
    pub fn bind_registry_generation(&self) -> u64 {
        self.shell_state.bind_registry.borrow().generation
    }

    /// `set editing-mode` seen through inputrc files, if any.
    pub fn bind_editing_mode(&self) -> Option<String> {
        self.shell_state.bind_registry.borrow().editing_mode.clone()
    }

    /// initialize_readline's inputrc leg (bashline.c:451: rl_readline_name,
    /// the rl_add_defun table, then rl_initialize → rl_read_init_file):
    /// readline reads `$INPUTRC`, else `~/.inputrc`, once per process —
    /// on the first `bind` invocation (bind.def:131) or the first prompt
    /// read (y.tab.c:4024), whichever comes first, i.e. AFTER the startup
    /// files, so rcfile `bind` commands run first and the inputrc
    /// overrides them. Missing/unreadable files are silent (GNU probe:
    /// no ~/.inputrc, no message). Diagnostics carry the
    /// `readline: FILE: line N: …` shape GNU prints (probe:
    /// ``readline: bogus.rc: line 1: bogus-var: unknown variable name``).
    pub fn initialize_readline(&mut self) {
        if self.bind_registry().borrow().readline_initialized {
            return;
        }
        if let Ok(mut registry) = self.bind_registry().try_borrow_mut() {
            registry.readline_initialized = true;
        }
        let path = match self.get_env("INPUTRC") {
            Some(value) if !value.is_empty() => value.to_string(),
            _ => {
                let home = self.home_value();
                if home.is_empty() {
                    return;
                }
                let separator = if home.ends_with('/') || home.ends_with('\\') {
                    ""
                } else {
                    "/"
                };
                format!("{home}{separator}.inputrc")
            }
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let term = self.get_env("TERM").map(str::to_owned);
        let editing_mode = self.bind_editing_mode();
        let env = crate::builtins::bind::InputrcEnv {
            term: term.as_deref(),
            editing_mode: editing_mode.as_deref(),
        };
        let parsed =
            crate::builtins::bind::parse_inputrc_with_env(&text, &path, &env, &mut |name| {
                std::fs::read_to_string(name).ok()
            });
        for diagnostic in &parsed.diagnostics {
            eprintln!(
                "readline: {}: line {}: {}",
                diagnostic.file, diagnostic.line, diagnostic.core
            );
        }
        {
            let registry = self.bind_registry();
            let mut registry = registry.borrow_mut();
            for (name, value) in &parsed.variables {
                registry.set_readline_variable(name, value);
            }
            for (keyseq, spec) in &parsed.bindings {
                let kind = match spec {
                    crate::builtins::bind::BindingSpec::Function { name } => {
                        BindKind::Function { name: name.clone() }
                    }
                    crate::builtins::bind::BindingSpec::Macro { text } => {
                        BindKind::Macro { text: text.clone() }
                    }
                };
                registry.register(BindEntry {
                    keyseq: keyseq.clone(),
                    kind,
                    keymap: None,
                });
            }
        }
    }

    /// Shared registry handle (process-global; see the field docs).
    pub fn bind_registry(&self) -> crate::shell::bind_registry::SharedBindRegistry {
        self.shell_state.bind_registry.clone()
    }

    pub fn register_bind_entry(&mut self, entry: BindEntry) {
        self.shell_state.bind_registry.borrow_mut().register(entry);
    }

    fn unbind_keyseq_in_keymap(&mut self, keyseq: &str, keymap: Option<&str>) {
        self.shell_state
            .bind_registry
            .borrow_mut()
            .unbind_keyseq_for_keymap(keyseq, keymap);
    }

    fn unbind_function_for_keymap(&mut self, name: &str, keymap: Option<&str>) {
        // unbind_command removes the bindings in the CURRENT keymap only
        // (bind.def:375-393 rl_unbind_function_in_map).
        let registry = self.shell_state.bind_registry.clone();
        let mut registry = registry.borrow_mut();
        let doomed: Vec<(String, Option<String>)> = registry
            .entries
            .iter()
            .filter(|entry| {
                matches!(&entry.kind, BindKind::Function { name: bound } if bound == name)
                    && same_keymap(entry.keymap.as_deref(), keymap)
            })
            .map(|entry| (entry.keyseq.clone(), entry.keymap.clone()))
            .collect();
        for (keyseq, bound_keymap) in doomed {
            registry.unbind_keyseq_for_keymap(&keyseq, bound_keymap.as_deref());
        }
    }
}

/// READLINE_LINE value after the command — used to clamp
/// READLINE_POINT/READLINE_MARK (readline_set_char_offset clamps to
/// rl_end of the NEW line, bashline.c:4559-4569 + 4697-4702).
fn read_back_line_length(executor: &Executor) -> usize {
    executor
        .get_env("READLINE_LINE")
        .map(|line| line.chars().count())
        .unwrap_or(0)
}

fn same_keymap(entry: Option<&str>, current: Option<&str>) -> bool {
    fn norm(name: Option<&str>) -> &str {
        crate::shell::bind_registry::normalize_keymap_name(name.unwrap_or("emacs-standard"))
    }
    norm(entry) == norm(current)
}

fn push_keyseq(by_name: &mut Vec<(String, Vec<String>)>, name: &str, seq: String) {
    match by_name.iter_mut().find(|(bound, _)| bound == name) {
        Some((_, seqs)) => seqs.push(seq),
        None => by_name.push((name.to_string(), vec![seq])),
    }
}

/// Collect the positional (non-option) arguments — the inputrc-form
/// binding specs after the option list (bind.def:150 `list = loptend`).
fn collect_positional(args: &[String], out: &mut Vec<String>) {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if arg.starts_with('-') && arg != "-" {
            // Skip this option cluster plus its attached/separate value
            // the same way parse_bind_options consumed them.
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut offset = 0;
            let mut consumed_next = false;
            while offset < chars.len() {
                let option = chars[offset];
                let takes_value = matches!(option, 'f' | 'q' | 'u' | 'm' | 'r' | 'x');
                if takes_value {
                    if offset + 1 >= chars.len() {
                        consumed_next = true;
                    }
                    break;
                }
                offset += 1;
            }
            if consumed_next {
                index += 1;
            }
            index += 1;
            continue;
        }
        out.push(arg.clone());
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executor_without_host_inputrc() -> Executor {
        let mut executor = Executor::new();
        #[cfg(unix)]
        const EMPTY_INPUTRC: &str = "/dev/null";
        #[cfg(windows)]
        const EMPTY_INPUTRC: &str = "NUL";
        executor.set_env("INPUTRC", EMPTY_INPUTRC);
        executor
    }

    #[test]
    fn run_bind_x_sets_reads_back_and_unbinds() {
        let mut executor = Executor::new();
        // The command sees READLINE_LINE; on return the variables are
        // unbound (unbind_readline_variables, bashline.c:4573).
        let outcome = executor.run_bind_x_command("COPIED=$READLINE_LINE", "hello", 2, 0);
        assert_eq!(outcome.status, 0);
        assert_eq!(outcome.line.as_deref(), Some("hello"));
        // Unchanged READLINE_LINE reads back identical; the editor keeps
        // its own buffer (maybe_make_readline_line is a no-op then).
        assert_eq!(outcome.point, Some(2));
        assert_eq!(executor.get_env("READLINE_LINE"), None);
        assert_eq!(executor.get_env("READLINE_POINT"), None);
        assert_eq!(executor.get_env("READLINE_MARK"), None);
        assert_eq!(executor.shell_state.variables.get("READLINE_LINE"), None);
    }

    #[test]
    fn run_bind_x_write_back_replaces_line_and_honors_point() {
        let mut executor = Executor::new();
        let outcome = executor.run_bind_x_command(
            r#"READLINE_LINE="replaced-$READLINE_LINE"; READLINE_POINT=${#READLINE_LINE}"#,
            "AB",
            2,
            0,
        );
        assert_eq!(outcome.line.as_deref(), Some("replaced-AB"));
        assert_eq!(outcome.point, Some("replaced-AB".chars().count()));
    }

    #[test]
    fn run_bind_x_point_read_back_is_char_offset_and_clamped() {
        let mut executor = Executor::new();
        // 999 is beyond the new line's end: readline_set_char_offset
        // clamps to rl_end (bashline.c:4559-4569). Multibyte content
        // counts CHARACTERS (readline_get_char_offset MB_STRLEN).
        let outcome = executor.run_bind_x_command(
            r#"READLINE_LINE="中文"; READLINE_POINT=999"#,
            "start",
            0,
            0,
        );
        assert_eq!(outcome.line.as_deref(), Some("中文"));
        assert_eq!(outcome.point, Some(2));
    }

    #[test]
    fn bind_x_registration_is_visible_and_process_global_across_clones() {
        let mut executor = executor_without_host_inputrc();
        let code = crate::script_driver::run_source(
            &mut executor,
            r#"bind -x '"\C-g": echo bound-c-g'"#,
            false,
        );
        assert_eq!(code, 0);
        let entries = executor.bind_registry_snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].keyseq, r#"\C-g"#);
        match &entries[0].kind {
            BindKind::Execute { command } => assert_eq!(command, "echo bound-c-g"),
            other => panic!("unexpected kind: {other:?}"),
        }
        // ShellState clones SHARE the registry (readline keymaps are
        // process-global in GNU): a bind inside a subshell reaches the
        // parent.
        let clone = executor.shell_state().clone();
        crate::script_driver::run_source(&mut executor, r#"bind -r '\C-g'"#, false);
        assert!(clone.bind_registry.borrow().entries.is_empty());
        assert!(executor.bind_registry_snapshot().is_empty());
    }

    #[test]
    fn bind_x_listing_and_removal_round_trip() {
        let mut executor = executor_without_host_inputrc();
        crate::script_driver::run_source(
            &mut executor,
            r#"bind -m emacs-standard -x '"\C-h": echo bound-c-h'"#,
            false,
        );
        let entries = executor.bind_registry_snapshot();
        assert_eq!(entries[0].keymap.as_deref(), Some("emacs-standard"));
        // -r in the default (emacs-standard) keymap removes it.
        crate::script_driver::run_source(&mut executor, r#"bind -r '\C-h'"#, false);
        assert!(executor.bind_registry_snapshot().is_empty());
    }

    #[test]
    fn macro_and_function_bindings_register_and_unbind_by_function() {
        let mut executor = executor_without_host_inputrc();
        crate::script_driver::run_source(
            &mut executor,
            r#"bind '"\C-t": "ins-macro"'; bind '"\C-o": accept-line'"#,
            false,
        );
        let entries = executor.bind_registry_snapshot();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| matches!(
            &entry.kind,
            BindKind::Macro { text } if text == "ins-macro"
        )));
        assert!(entries.iter().any(|entry| matches!(
            &entry.kind,
            BindKind::Function { name } if name == "accept-line"
        )));
        // bind -u accept-line removes every binding of that function.
        crate::script_driver::run_source(&mut executor, "bind -u accept-line", false);
        let entries = executor.bind_registry_snapshot();
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0].kind, BindKind::Macro { .. }));
    }

    #[test]
    fn generation_changes_on_mutation_only() {
        let mut executor = executor_without_host_inputrc();
        let before = executor.bind_registry_generation();
        crate::script_driver::run_source(&mut executor, "bind -p >/dev/null", false);
        assert_eq!(executor.bind_registry_generation(), before);
        crate::script_driver::run_source(&mut executor, r#"bind -x '"\C-g": true'"#, false);
        assert!(executor.bind_registry_generation() > before);
    }

    #[test]
    fn bind_x_exit_status_flows_through() {
        let mut executor = Executor::new();
        let outcome = executor.run_bind_x_command("exit 3", "line", 0, 0);
        assert_eq!(outcome.status, 3);
    }
}
