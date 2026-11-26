use crate::executor::*;

/// GNU background/coproc children are forks (execute_cmd.c
/// execute_in_subshell / make_child): the subshell inherits every variable
/// attribute (array/assoc/readonly/integer/nameref/trace/case transforms,
/// declared-unset), the shell option state (set flags + shopt), the hash
/// table, the directory stack, ignored-signal dispositions, ulimits, umask
/// and the SECONDS epoch. Spawned `rubash -c` children re-import their
/// process environment as `env_vars`, so forwarding these `__RUBASH_*`
/// state keys restores exactly that fork inheritance. Keys that are
/// per-process runtime state (FD tables, coproc pipes, command-substitution
/// and heredoc payloads, trap table entries, line/pid bookkeeping,
/// evaluation-context flags, test-harness sentinels) stay dropped.
pub(in crate::executor) fn rubash_spawn_inherited_state(key: &str) -> bool {
    const KEYS: &[&str] = &[
        "__RUBASH_ARRAY_VARS",
        "__RUBASH_ASSOC_VARS",
        "__RUBASH_ASSOC_128_VARS",
        "__RUBASH_DECLARED_UNSET_VARS",
        "__RUBASH_EXPORTED_VARS",
        "__RUBASH_EXPORTED_FUNCTIONS",
        "__RUBASH_READONLY_VARS",
        "__RUBASH_READONLY_FUNCTIONS",
        "__RUBASH_FUNC_TRACE_FUNCTIONS",
        "__RUBASH_UNSTREAMED_FUNCTION_BODIES",
        "__RUBASH_TRACE_VARS",
        "__RUBASH_INTEGER_VARS",
        "__RUBASH_UPPERCASE_VARS",
        "__RUBASH_LOWERCASE_VARS",
        "__RUBASH_CAPCASE_VARS",
        "__RUBASH_NAMEREF_VARS",
        "__RUBASH_HASH_TABLE",
        "__RUBASH_DIR_STACK",
        "__RUBASH_DISABLED_BUILTINS",
        "__RUBASH_GETOPTS_OFFSET",
        // rubash#2586: the readonly-OPTIND shadow counter travels with
        // its OFFSET pair — carrying one without the other left a
        // respawned child with a stale char offset against a fresh
        // counter (or vice versa), replaying/skipping getopts args.
        "__RUBASH_GETOPTS_OPTIND",
        // A GNU fork keeps the variable context stack, so a local that
        // shadows an exported outer binding keeps the shadowed name's
        // child-env contribution — the LOCAL_EXPORT_ENV stash is that
        // model's transport across the respawn (rubash#970).
        "__RUBASH_LOCAL_EXPORT_ENV",
        "__RUBASH_ULIMIT_C",
        "__RUBASH_ULIMIT_F",
        "__RUBASH_ULIMIT_N",
        "__RUBASH_UMASK",
        "__RUBASH_SECONDS_OFFSET",
        "__RUBASH_SHELL_START_EPOCH",
        "__RUBASH_TRAP_ORIG_IGN",
        "__RUBASH_POSIX_MODE",
        "__RUBASH_ZSH_OPTIONS",
        "__RUBASH_PHYSICAL_PWD",
        "__RUBASH_SHOPT_STATE",
        "__RUBASH_SHOPT_CHECKHASH",
        "__RUBASH_COMPATIBLE_SHELL_PATH",
        "__RUBASH_SHELL_ROOT",
        "__RUBASH_TEMP_PATH",
    ];
    key.starts_with("__RUBASH_SETOPT_") || KEYS.contains(&key)
}

/// rubash#3049: keys that are fork/respawn TRANSPORT — serialized shell
/// state and per-spawn protocol stamps — not session state. GNU's exec
/// boundary hands a child exactly the export environment (variables.c
/// maybe_make_export_env / make_env_array_from_var_list); a forked
/// subshell needs no environment entries at all because it shares the
/// parent's table in-process (execute_cmd.c execute_in_subshell). This
/// family exists only because rubash emulates the fork with a respawned
/// `rubash -c` process, where the environment IS the state channel —
/// so once such a process is born, its environ carries the transport
/// residue of its own birth forever. Two boundary rules follow:
///
/// * the residue must never reach a REAL child's environment
///   (compose_child_environment's process-env mirror arm would
///   otherwise re-project it — the respawned child's
///   __RUBASH_*_VARS attribute lists landed in the exec'd
///   bats-exec-test, where VarTable::from_values applied them and
///   BATS_TEST_TAGS was born `declare -ar`, deadlocking bats-core
///   test #71 on its readonly assignment);
/// * a process NOT born as a validated respawn must not import it
///   (Executor::new_inner strips the family before from_values reads
///   the attribute strings).
///
/// Deliberately NOT in the family: the binary/host config entries
/// (COMPATIBLE_SHELL_PATH/SHELL_ROOT/TEMP_PATH — a spawned child keeps
/// its lineage's install resolution), the ULIMIT_*/UMASK carriers
/// (kernel resource state persists across GNU's execve; the env keys
/// are the Windows-side model for real children too), the
/// orig-ignores carrier (__RUBASH_TRAP_ORIG_IGN — trap.c
/// original_signals SIG_IGN dispositions DO cross execve), the
/// __RUBASH_FD_* delivery stamps (per-spawn fd-table transport minted
/// fresh by the direct spawner for THIS child — init.rs consumes and
/// strips them at entry, so they are delivery, not residue; stripping
/// them for every non-respawn birth severed ALL fd>=3 delivery to
/// children — rubash#3856), and arbitrary session-mirrored probe keys
/// (the G-03 mirrored-assignment contract keeps a session's own
/// mirrored `__RUBASH_*` writes visible to its children).
pub(in crate::executor) fn fork_respawn_transport_key(key: &str) -> bool {
    const KEYS: &[&str] = &[
        // Variable-table serializations (the attribute strings
        // VarTable::from_values imports, plus the list-only state sets
        // the protocol importers read at their own sites).
        "__RUBASH_ARRAY_VARS",
        "__RUBASH_ASSOC_VARS",
        "__RUBASH_ASSOC_128_VARS",
        "__RUBASH_CAPCASE_VARS",
        "__RUBASH_DECLARED_UNSET_VARS",
        "__RUBASH_EXPORTED_VARS",
        "__RUBASH_EXPORTED_FUNCTIONS",
        "__RUBASH_INTEGER_VARS",
        "__RUBASH_INVISIBLE_VARS",
        "__RUBASH_LOWERCASE_VARS",
        "__RUBASH_NAMEREF_VARS",
        "__RUBASH_READONLY_VARS",
        "__RUBASH_READONLY_FUNCTIONS",
        "__RUBASH_TRACE_VARS",
        "__RUBASH_UNSET_DYNAMIC_VARS",
        "__RUBASH_UPPERCASE_VARS",
        "__RUBASH_FUNC_TRACE_FUNCTIONS",
        "__RUBASH_UNSTREAMED_FUNCTION_BODIES",
        // Structural shell state: hash/dirstack/builtin/getopts/
        // option/clock transports a forked subshell inherits
        // in-process but an exec'd shell re-derives (SHELLOPTS /
        // BASHOPTS are the exported carriers for options).
        "__RUBASH_HASH_TABLE",
        "__RUBASH_DIR_STACK",
        "__RUBASH_DISABLED_BUILTINS",
        "__RUBASH_GETOPTS_OFFSET",
        "__RUBASH_GETOPTS_OPTIND",
        "__RUBASH_LOCAL_EXPORT_ENV",
        "__RUBASH_SECONDS_OFFSET",
        "__RUBASH_SHELL_START_EPOCH",
        "__RUBASH_POSIX_MODE",
        "__RUBASH_ZSH_OPTIONS",
        "__RUBASH_PHYSICAL_PWD",
        "__RUBASH_SHOPT_STATE",
        "__RUBASH_SHOPT_CHECKHASH",
        // Spawn bookkeeping: the respawn claim/stamps, subshell-depth
        // and $BASHPID seeds, the positional/fork-context transports
        // (FORK_TRANSPORT_KEYS covers the __RUBASH_FORK_* prefix),
        // the virtual-stdin delivery, the command-file and
        // state-file pointers, and the per-spawn payload prefixes.
        "__RUBASH_SHELL_PID",
        "__RUBASH_SPAWNER_PID",
        "__RUBASH_SUBSHELL_DEPTH",
        "__RUBASH_BG_DEPTH",
        "__RUBASH_BG_BASHPID_BASE",
        "__RUBASH_COPROC_CHILD",
        "__RUBASH_ASYNC_CHILD",
        "__RUBASH_BACKGROUND_WRAPPER",
        crate::invocation::RESPAWN_STATE_FILE_ENV_KEY,
        "__RUBASH_INHERIT_PROCESS_STDIN",
        "__RUBASH_CMD_FILE",
        "__RUBASH_CMD_START_LINE",
        "__RUBASH_COMSUB_BODY",
        "__RUBASH_FUNCTION_STDIN",
        "__RUBASH_FUNCTION_STDIN_OFFSET",
        "__RUBASH_PIPESTATUS_STATE",
        "__RUBASH_EVAL_CONTEXT",
        "__RUBASH_CNF_HANDLER_RUNNING",
        "__RUBASH_FILE_MODES",
        // The signal-trap depth/status pair (the only two spellings in
        // this sub-family).
        "__RUBASH_SIGNAL_TRAP_DEPTH",
        "__RUBASH_SIGNAL_TRAP_STATUS",
        // Diagnostic-frame and invocation-classification transports:
        // a fresh shell derives all of these from its own argv and
        // entry state (init.rs strips the same set for non-respawns).
        "__RUBASH_SCRIPT_NAME",
        "__RUBASH_TOP_LEVEL_NAME",
        "__RUBASH_CURRENT_FILE",
        "__RUBASH_CURRENT_LINE",
        "__RUBASH_CURRENT_FUNCTION",
        "__RUBASH_CURRENT_COMMAND",
        "__RUBASH_LAST_COMMAND",
        "__RUBASH_IN_SOURCE",
        "__RUBASH_EXTGLOB_ENABLED_LINE",
        "__RUBASH_LINE_OFFSET",
        "__RUBASH_ARGV0_AFTER_UNSET",
        "__RUBASH_SHELL_NAME",
        "__RUBASH_SESSION_SCOPE",
        "__RUBASH_INTERACTIVE",
        "__RUBASH_INTERACTIVE_FLAG_OFF",
        "__RUBASH_IS_C",
        "__RUBASH_LOGIN_SHELL",
        "__RUBASH_READ_STDIN",
        "__RUBASH_SOURCED_RC",
        "__RUBASH_STARTUP_FILES_DONE",
        "__RUBASH_NO_EDITING",
        "__RUBASH_NO_PROFILE",
        "__RUBASH_NO_RC",
        "__RUBASH_PC_NOHIST",
        "__RUBASH_DEBUGGER_MODE",
        "__RUBASH_DEBUGGER_ATTEMPTED",
    ];
    KEYS.contains(&key)
        || key.starts_with("__RUBASH_FORK_")
        || key.starts_with("__RUBASH_SETOPT_")
        || key.starts_with("__RUBASH_ALIAS_")
        || key.starts_with("__RUBASH_PARSE_")
        || key.starts_with("__RUBASH_COMPOUND_SYNTAX_ERROR")
        || (key.starts_with("__RUBASH_TRAP") && key != crate::builtins::trap::TRAP_ORIG_IGNORES)
}

/// Transport key for the fork-inherited positional parameters. GNU fork
/// children inherit the parent's dollar_vars verbatim (variables.c), but
/// a respawned `rubash -c` child re-imports only its process
/// environment, so every fork-shaped spawn (async/subshell wrapper,
/// coproc, both process-substitution forms) stamps the live positional
/// window under this key and `Executor::new` consumes and strips it for
/// internal respawns (init.rs). Without the stamp the child's body saw
/// `$#` = 0 — bats-exec-suite read `"$@"` inside `< <(...)` as one empty
/// word and ran zero tests (rubash#2032). The value rides the indexed
/// array storage codec (`format_indexed_array_values`/`array_values`),
/// the same round-trip the array-variable transport uses.
pub(in crate::executor) const POSITIONAL_PARAMS_TRANSPORT: &str = "__RUBASH_POSITIONAL_PARAMS";

/// Transport key for the parent's live errexit-suppression depth at
/// spawn. GNU's fork-shaped children that execute the SAME command
/// node (the async `&` wrapper's execute_in_subshell fork,
/// execute_cmd.c:1576, and the coproc child's execute_coproc ->
/// execute_in_subshell, execute_cmd.c:2546) carry the node's
/// CMD_IGNORE_RETURN — set by `!` (execute_cmd.c:655-656) or an
/// enclosing ignore_return propagation arm — so `set -e` inside an
/// inverted coproc/`&` body is ignored (exec16's "reached coproc
/// body"). A respawned `rubash -c` child has no node; this stamp
/// seeds its suppress_errexit. process_substitute children are
/// excluded: GNU re-parses the producer's saved command text
/// (subst.c:6362), so no flag survives — verified
/// `! cat < <(set -e; false; echo x)` prints nothing under 5.3.20.
pub(in crate::executor) const SUPPRESS_ERREXIT_TRANSPORT: &str = "__RUBASH_SUPPRESS_ERREXIT";

/// RAII scope for the executor's `expansion_probe_active` flag
/// (rubash#2585); restores the prior value on drop, so a nested probe
/// scope cannot disarm an outer one and early returns are covered —
/// the same shape as `SpeculativeExpansionGuard` for the thread-local
/// SPECULATIVE_EXPANSION flag (pipeline_exec.rs).
pub(in crate::executor) struct ExpansionProbeGuard {
    flag: std::rc::Rc<std::cell::Cell<bool>>,
    saved: bool,
}

impl ExpansionProbeGuard {
    pub(in crate::executor) fn new(flag: &std::rc::Rc<std::cell::Cell<bool>>) -> Self {
        Self {
            saved: flag.replace(true),
            flag: flag.clone(),
        }
    }
}

impl Drop for ExpansionProbeGuard {
    fn drop(&mut self) {
        self.flag.set(self.saved);
    }
}

impl Executor {
    /// The live positional window (`shift`-visible `$1..$#`) rendered
    /// for a fork-shaped respawn; see POSITIONAL_PARAMS_TRANSPORT.
    pub(in crate::executor) fn positional_params_transport_value(&self) -> String {
        format_indexed_array_values(self.shell_state.positional_window().to_vec())
    }
}

/// rubash#2058: transport keys for the rest of the fork-inherited
/// state (docs/design/respawn-transport-census.md). A GNU fork copies
/// the parent's call-stack arrays and PIPESTATUS cell verbatim
/// (variables.c); a respawned `rubash -c` child re-imports only its
/// process environment, so every fork-shaped spawn (async/subshell
/// wrapper, coproc, both process-substitution forms) stamps the
/// parent's effective views under these keys and `Executor::new`
/// consumes and strips them for internal respawns (init.rs). List
/// values ride the indexed-array storage codec, the same round-trip
/// POSITIONAL_PARAMS_TRANSPORT uses. The ARGV bottom frame gets its
/// own key whose PRESENCE is the materialization bit: GNU's fork
/// inherits `bash_argv_initialized` as-is, and an unmaterialized
/// child's reads must not synthesize a bottom from the live window
/// while the child runs in the parent's function context
/// (variables.c:1656 get_bashargcv initializes only at
/// variable_context == 0).
pub(in crate::executor) const FORK_FUNCNAME_TRANSPORT: &str = "__RUBASH_FORK_FUNCNAME";
pub(in crate::executor) const FORK_SOURCE_TRANSPORT: &str = "__RUBASH_FORK_BASH_SOURCE";
pub(in crate::executor) const FORK_LINENO_TRANSPORT: &str = "__RUBASH_FORK_BASH_LINENO";
pub(in crate::executor) const FORK_ARGV_TRANSPORT: &str = "__RUBASH_FORK_BASH_ARGV";
pub(in crate::executor) const FORK_ARGC_TRANSPORT: &str = "__RUBASH_FORK_BASH_ARGC";
pub(in crate::executor) const FORK_ARGV_BOTTOM_TRANSPORT: &str = "__RUBASH_FORK_ARGV_BOTTOM";
/// rubash#3909: the `bash_argv_initialized` flag (variables.c:5640-5650)
/// rides the fork transport next to the bottom frame it qualifies.
/// Presence with value "1" is the bit (the FORK_IN_FUNCTION
/// convention): the spawner stamps it exactly when the flag is set, so
/// a respawn child of a flag-set/bottom-None parent does not wrongly
/// synthesize a ([0]="0") frame from its live window at its first
/// top-level read. A child whose parent never armed extdebug gets no
/// entry and is born with the flag clear, exactly as the parent was.
pub(in crate::executor) const FORK_BASH_ARGV_INITIALIZED_TRANSPORT: &str =
    "__RUBASH_FORK_BASH_ARGV_INITIALIZED";
pub(in crate::executor) const FORK_IN_FUNCTION_TRANSPORT: &str = "__RUBASH_FORK_IN_FUNCTION";
pub(in crate::executor) const FORK_MAIN_FRAME_TRANSPORT: &str = "__RUBASH_FORK_MAIN_FRAME";
pub(in crate::executor) const FORK_PIPESTATUS_TRANSPORT: &str = "__RUBASH_FORK_PIPESTATUS";
pub(in crate::executor) const FORK_FN_SPANS_TRANSPORT: &str = "__RUBASH_FORK_FN_SPANS";
/// rubash#2332: source-line spans of the respawned child's ROOT body —
/// the inline TEXT body that becomes the child's main `-c` source
/// (an async `( list ) &` / coproc compound body), sibling to the
/// per-function carrier above. Element 0 of the value is the count of
/// prelude function-definition commands ahead of the body in the
/// child's source (0 for coproc children); element 1 is the body's
/// comma-joined `line.end_line.logical_line` triples in
/// collect_command_line_spans order. See
/// docs/design/respawn-root-line-spans.md.
pub(in crate::executor) const FORK_ROOT_SPANS_TRANSPORT: &str = "__RUBASH_FORK_ROOT_SPANS";
/// rubash#3347: the spawner's logical cwd (GNU
/// the_current_working_directory, builtins/common.c:589), stamped by
/// fork_context_transport_entries and consumed by the child's startup
/// funnel under the provenance gate (init.rs). A GNU fork child
/// inherits tcwd verbatim (execute_cmd.c:1576 execute_in_subshell,
/// subst.c:6362 process_substitute); a respawned `rubash -c` child
/// re-imports only its environment, so without this stamp its startup
/// re-derives the logical cwd from the exported $PWD — and a forged
/// $PWD moves it (variables.c:907 set_pwd adopts any same-directory
/// spelling). Like the FORK_* keys, the child's init strips it
/// unconditionally via FORK_TRANSPORT_KEYS.
pub(in crate::executor) const LOGICAL_CWD_TRANSPORT: &str = "__RUBASH_LOGICAL_CWD";
/// rubash#2456: the spawner's effective login state, stamped ONLY by
/// the two process-substitution spawn sites. GNU's procsub fork
/// (subst.c:6412-6445 process_substitute) is the one fork-shaped child
/// whose `login_shell` is never reset: the only reset sites are
/// execute_cmd.c:1636-1637 (execute_in_subshell) and
/// execute_cmd.c:5474-5475 (execute_subshell_builtin_or_function),
/// and neither lies on the procsub path, so the child inherits the
/// parent's CURRENT global — the value the logout gate reads,
/// `invocation_login && !subshell_login_cleared`, at the moment the
/// substitution is constructed. Presence with value "1" is the bit
/// (the FORK_IN_FUNCTION convention): the spawner stamps it exactly
/// when its effective state is on, and every other respawn form
/// (async, coproc, subshell wrappers) is born non-login because GNU's
/// reset sites clear the global on those paths. The child's init
/// consumes it under the provenance gate (init.rs) and the key is
/// stripped unconditionally with the rest of this family.
pub(in crate::executor) const FORK_LOGIN_SHELL_TRANSPORT: &str = "__RUBASH_FORK_LOGIN_SHELL";

/// Every FORK_* transport key, for the child's unconditional strip.
pub(in crate::executor) const FORK_TRANSPORT_KEYS: [&str; 14] = [
    FORK_FUNCNAME_TRANSPORT,
    FORK_SOURCE_TRANSPORT,
    FORK_LINENO_TRANSPORT,
    FORK_ARGV_TRANSPORT,
    FORK_ARGC_TRANSPORT,
    FORK_ARGV_BOTTOM_TRANSPORT,
    FORK_BASH_ARGV_INITIALIZED_TRANSPORT,
    FORK_IN_FUNCTION_TRANSPORT,
    FORK_MAIN_FRAME_TRANSPORT,
    FORK_PIPESTATUS_TRANSPORT,
    FORK_FN_SPANS_TRANSPORT,
    FORK_ROOT_SPANS_TRANSPORT,
    FORK_LOGIN_SHELL_TRANSPORT,
    LOGICAL_CWD_TRANSPORT,
];

impl Executor {
    /// rubash#2058: the fork-context stamp every fork-shaped respawn
    /// carries, as (key, value) entries the spawn sites merge into the
    /// child's environment next to POSITIONAL_PARAMS_TRANSPORT. The
    /// stack lists are this shell's EFFECTIVE views (own frames above
    /// any frames this shell itself inherited), so a nested respawn
    /// re-stamps the full chain. The bottom frame is peeked, never
    /// installed: forking in GNU does not materialize the parent's
    /// arrays, so the stamp must not either.
    pub(in crate::executor) fn fork_context_transport_entries(&self) -> Vec<(String, String)> {
        let inherited = self.shell_state.inherited_fork_frames.as_ref();
        let mut argv_frames = self.shell_state.bash_argv_stack.clone();
        let mut argc_frames = self.shell_state.bash_argc_stack.clone();
        if let Some(inherited) = inherited {
            argv_frames.extend(inherited.argv_frames.iter().cloned());
            argc_frames.extend(inherited.argc_frames.iter().cloned());
        }
        let mut entries = vec![
            (
                FORK_FUNCNAME_TRANSPORT.to_string(),
                format_indexed_array_values(
                    self.dynamic_stack_array_values("FUNCNAME")
                        .unwrap_or_default(),
                ),
            ),
            (
                FORK_SOURCE_TRANSPORT.to_string(),
                format_indexed_array_values(
                    self.dynamic_stack_array_values("BASH_SOURCE")
                        .unwrap_or_default(),
                ),
            ),
            (
                FORK_LINENO_TRANSPORT.to_string(),
                format_indexed_array_values(self.bash_lineno_view()),
            ),
            (
                FORK_ARGV_TRANSPORT.to_string(),
                format_indexed_array_values(argv_frames),
            ),
            (
                FORK_ARGC_TRANSPORT.to_string(),
                format_indexed_array_values(argc_frames),
            ),
            (
                FORK_PIPESTATUS_TRANSPORT.to_string(),
                crate::shell::state::encode_pipestatus_state(
                    self.shell_state.pipestatus_shape,
                    &self.shell_state.pipestatus,
                ),
            ),
        ];
        // The installed bottom frame rides verbatim; its key is
        // present exactly when the parent's arrays have materialized.
        // A respawn child's own bottom was seeded into its RefCell at
        // birth, so the plain peek covers the nested case.
        if let Some(bottom) = self.shell_state.bash_arg_bottom.borrow().clone() {
            entries.push((
                FORK_ARGV_BOTTOM_TRANSPORT.to_string(),
                format_indexed_array_values(bottom),
            ));
        }
        // rubash#3909: the flag rides with the frame it qualifies —
        // stamped exactly when set (the FORK_IN_FUNCTION convention),
        // so a respawn child of a flag-set/bottom-None parent keeps
        // the initialized-but-empty state instead of synthesizing.
        if self.shell_state.bash_argv_initialized.get() {
            entries.push((
                FORK_BASH_ARGV_INITIALIZED_TRANSPORT.to_string(),
                "1".to_string(),
            ));
        }
        if self.shell_state.function_depth > 0 || inherited.is_some_and(|frames| frames.in_function)
        {
            entries.push((FORK_IN_FUNCTION_TRANSPORT.to_string(), "1".to_string()));
        }
        if self.has_script_main_frame() || inherited.is_some_and(|frames| frames.has_main_frame) {
            entries.push((FORK_MAIN_FRAME_TRANSPORT.to_string(), "1".to_string()));
        }
        // rubash#2202: per-function source-line spans for exactly the
        // bodies background_function_prelude re-renders into the
        // child (rubash#2543: background_prelude_function_entries, so
        // a name skipped because its BASH_FUNC_*%% carrier already
        // rides the child env gets no dangling span entry). GNU's
        // forked child executes the parent's parsed
        // tree, whose nodes keep their parse lines (make_cmd.c:196);
        // the respawn child's re-parse collapses every body onto the
        // prelude's own lines, so the recorded span is the only way
        // back to per-command definition lines. Each entry is `name=`
        // plus the comma-joined span list format_line_spans renders
        // (0 = None) in collect_command_line_spans traversal order;
        // the set rides the indexed-array codec like its siblings.
        let span_entries: Vec<String> = self
            .background_prelude_function_entries()
            .iter()
            .map(|(name, body)| {
                let mut spans = Vec::new();
                crate::parser::collect_command_line_spans(&body.commands, &mut spans);
                let rendered = format_line_spans(&spans);
                format!("{name}={rendered}")
            })
            .collect();
        if !span_entries.is_empty() {
            entries.push((
                FORK_FN_SPANS_TRANSPORT.to_string(),
                format_indexed_array_values(span_entries),
            ));
        }
        // rubash#3347: the session's logical cwd rides the same stamp.
        // The value is already the canonical form cd/startup
        // established (or the verbatim uncanonicalized `cd .`
        // make_absolute form GNU stores); the child's init adopts it
        // verbatim once it still names the session cwd, and falls
        // back to the $PWD adoption when it does not. An empty value
        // is the unknown-birth state — there is no logical value to
        // carry, and stamping it would let a stale empty ride past
        // the child's validation.
        if let Some(logical) = self.shell_state.logical_cwd.clone() {
            if !logical.is_empty() {
                entries.push((LOGICAL_CWD_TRANSPORT.to_string(), logical));
            }
        }
        entries
    }
}

/// rubash#2332: one span slot of the FN/ROOT span codecs — `0`
/// encodes None, any other value is the line itself.
fn parse_span_slot(text: &str) -> Option<Option<usize>> {
    match text.parse::<usize>() {
        Ok(0) => Some(None),
        Ok(line) => Some(Some(line)),
        Err(_) => None,
    }
}

/// rubash#2332: parse a comma-joined
/// `line.end_line.logical_line.first_word_end_line` list (the
/// encoding both span carriers share; the fourth field is
/// rubash#3054). Returns None when any entry is malformed — a span
/// that cannot be parsed must never reach a re-seed. The empty
/// string is an empty vector. A three-field entry is the pre-#3054
/// form and reads with no first-element stamp; parent and child are
/// the same binary, so it can only arrive from a hand-built stamp,
/// never from a live transport.
fn parse_line_spans(text: &str) -> Option<Vec<crate::parser::CommandLineSpan>> {
    let mut spans = Vec::new();
    if !text.is_empty() {
        for entry in text.split(',') {
            let fields: Vec<&str> = entry.split('.').collect();
            if fields.len() != 3 && fields.len() != 4 {
                return None;
            }
            let first_word_end_line = if fields.len() == 4 {
                parse_span_slot(fields[3])
            } else {
                Some(None)
            };
            match (
                parse_span_slot(fields[0]),
                parse_span_slot(fields[1]),
                parse_span_slot(fields[2]),
                first_word_end_line,
            ) {
                (Some(line), Some(end_line), Some(logical_line), Some(first_word_end_line)) => {
                    spans.push(crate::parser::CommandLineSpan {
                        line,
                        end_line,
                        logical_line,
                        first_word_end_line,
                    });
                }
                _ => return None,
            }
        }
    }
    Some(spans)
}

/// rubash#2332: format spans in the list encoding parse_line_spans
/// reads (None renders as 0).
fn format_line_spans(spans: &[crate::parser::CommandLineSpan]) -> String {
    spans
        .iter()
        .map(|span| {
            format!(
                "{}.{}.{}.{}",
                span.line.unwrap_or(0),
                span.end_line.unwrap_or(0),
                span.logical_line.unwrap_or(0),
                span.first_word_end_line.unwrap_or(0)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// rubash#2202: decode a __RUBASH_FORK_FN_SPANS stamp value into the
/// per-function span map the child's definition path consumes
/// (init.rs). Malformed entries are dropped individually — a span
/// that cannot be parsed must never reach the re-seed.
pub(in crate::executor) fn decode_function_line_spans(
    value: &str,
) -> std::collections::HashMap<String, Vec<crate::parser::CommandLineSpan>> {
    let mut map = std::collections::HashMap::new();
    for entry in array_values(value) {
        let Some((name, nodes)) = entry.split_once('=') else {
            continue;
        };
        if let Some(spans) = parse_line_spans(nodes) {
            map.insert(name.to_string(), spans);
        }
    }
    map
}

/// rubash#2332: encode a __RUBASH_FORK_ROOT_SPANS stamp value —
/// element 0 is the count of prelude function-definition commands
/// ahead of the body in the child's `-c` source, element 1 the root
/// body's span list.
pub(in crate::executor) fn encode_root_line_spans(
    prelude_defs: usize,
    spans: &[crate::parser::CommandLineSpan],
) -> String {
    format_indexed_array_values(vec![prelude_defs.to_string(), format_line_spans(spans)])
}

/// rubash#2332: decode a __RUBASH_FORK_ROOT_SPANS stamp value into
/// (prelude definition count, root span vector). Any malformation
/// drops the whole stamp: the root body is one unit, and a partially
/// decoded span vector would misalign every command after the gap.
pub(in crate::executor) fn decode_root_line_spans(
    value: &str,
) -> Option<(usize, Vec<crate::parser::CommandLineSpan>)> {
    let values = array_values(value);
    let prelude_defs: usize = values.first()?.parse().ok()?;
    let spans = parse_line_spans(values.get(1)?)?;
    Some((prelude_defs, spans))
}

/// The parent-source line of the first command that will execute from
/// `cmd`'s transported text: descend through subshell / brace-group /
/// pipeline / list / wrapper nodes to the first leaf. The respawn
/// line seed must name where the child's body text starts — GNU's
/// forked child reports each body command's own line, and the
/// serialized body collapses onto the seed line, so the seed is the
/// first leaf's line (compound nodes' own `line` is their closing
/// line, which mis-seeded async subshells by the body's height).
pub(in crate::executor) fn first_leaf_command_line(cmd: &CommandNode) -> Option<usize> {
    if let Some(subshell) = &cmd.subshell_command {
        if let Some(first) = subshell.body.first() {
            return first_leaf_command_line(first);
        }
    }
    if let Some(group) = &cmd.brace_group {
        if let Some(first) = group.body.first() {
            return first_leaf_command_line(first);
        }
    }
    if let Some(pipeline) = &cmd.pipeline_command {
        if let Some(first) = pipeline.stages.first() {
            return first_leaf_command_line(first);
        }
    }
    if let Some(list) = &cmd.and_or_list {
        if let Some(first) = list.commands.first() {
            return first_leaf_command_line(first);
        }
    }
    if let Some(background) = &cmd.background_command {
        return first_leaf_command_line(&background.command);
    }
    if let Some(time) = &cmd.time_command {
        return first_leaf_command_line(&time.command);
    }
    if let Some(inverted) = &cmd.inverted_command {
        return first_leaf_command_line(&inverted.command);
    }
    cmd.line
}

/// niubash#362: true when a respawned async `&` child's `-c`
/// payload is too large to ride the OS command line / a single argv entry.
///
/// Windows caps the whole `CreateProcessW` lpCommandLine at 32767 UTF-16
/// units, so measure the exact encoded length (program + `-c` + script).
/// Unix caps one execve argument at MAX_ARG_STRLEN (32 pages * 4096 =
/// 131072 bytes), so compare the raw byte length. Either way the caller
/// switches to the temp-file transport (__RUBASH_CMD_FILE).
#[cfg(windows)]
pub(in crate::executor) fn respawn_source_needs_file(exe: &std::path::Path, source: &str) -> bool {
    const CMDLINE_LIMIT: usize = 32_767;
    // Headroom for the exe path, the "-c " separator and quoting growth.
    const HEADROOM: usize = 256;
    let args = ["-c".to_string(), source.to_string()];
    crate::fd::encoded_command_line_len(exe, &args) + HEADROOM > CMDLINE_LIMIT
}

#[cfg(unix)]
pub(in crate::executor) fn respawn_source_needs_file(_exe: &std::path::Path, source: &str) -> bool {
    const ARG_STRLEN_LIMIT: usize = 131_072;
    const HEADROOM: usize = 256;
    source.len() + HEADROOM > ARG_STRLEN_LIMIT
}

#[cfg(not(any(unix, windows)))]
pub(in crate::executor) fn respawn_source_needs_file(
    _exe: &std::path::Path,
    _source: &str,
) -> bool {
    false
}

/// `CARGO_BIN_EXE_rubash` is trustworthy only when it names a real
/// engine binary: `cargo test --lib` and `cargo nextest` export the
/// lib-test binary itself under that name, and every cargo test
/// artifact lands in `target/<profile>/deps` while the engine bin sits
/// directly under the profile dir. A `-c` respawn pointed at the test
/// binary prints `error: Unrecognized option: 'c'` and drops its body
/// (rubash#2875), so reject env values inside a `deps` dir — or missing
/// on disk — and let the current-exe helper take over.
pub(crate) fn test_rubash_binary_from_env() -> Option<std::path::PathBuf> {
    let candidate = std::path::PathBuf::from(std::env::var_os("CARGO_BIN_EXE_rubash")?);
    let parent = candidate.parent()?;
    if candidate.is_file() && parent.file_name().and_then(|name| name.to_str()) != Some("deps") {
        Some(candidate)
    } else {
        None
    }
}

/// rubash#3292: resolve the real rubash binary for a procsub /
/// background / coproc child spawned from a test binary. Cargo only
/// builds the bin (and sets `CARGO_BIN_EXE_rubash`) for full `cargo
/// test` runs; a lib-only run (`cargo test --lib`) on a fresh target
/// has neither, and the spawn chain would otherwise fall back to the
/// lib-test binary itself, which rejects `-c` (`Unrecognized option:
/// 'c'`, issue2875). Search, in order, the profile dir the test
/// binary lives under, the `CARGO_TARGET_DIR` target dir
/// (debug/release), the release sibling of the test binary's profile
/// dir, and the debug/release profile dirs under each ancestor of the
/// test binary (nested/workspace layouts). Only a file literally
/// named `rubash[.exe]` counts — a test binary is `rubash-<hash>`
/// under `deps/` or `build/<pkg>/<fingerprint>/out/` and never
/// matches. Production behaviour is unchanged: the test-layout gate
/// still returns `None` outside a test binary, so a real shell keeps
/// falling back to `current_exe()`.
pub(crate) fn test_rubash_binary_from_current_exe() -> Option<std::path::PathBuf> {
    let current = std::env::current_exe().ok()?;
    let dir = current.parent()?;
    // Test-binary layouts: `deps/` on the classic build dir, and
    // `build/<pkg>/<fingerprint>/out/` on the current cargo build-dir
    // layout (the unit-test binary for a package with a build script is
    // placed inside the package's own out dir).
    let dir_name = dir.file_name().and_then(|name| name.to_str());
    if dir_name != Some("deps") && dir_name != Some("out") {
        return None;
    }
    let binary_name = if cfg!(windows) {
        "rubash.exe"
    } else {
        "rubash"
    };
    let mut profile_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(debug_dir) = dir.parent() {
        profile_dirs.push(debug_dir.to_path_buf());
        if let Some(target_dir) = debug_dir.parent() {
            profile_dirs.push(target_dir.join("release"));
        }
    }
    if let Some(target_dir) = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from) {
        profile_dirs.push(target_dir.join("debug"));
        profile_dirs.push(target_dir.join("release"));
    }
    let mut ancestor = current.parent();
    let mut depth = 0usize;
    while let Some(dir) = ancestor {
        if depth >= 8 {
            break;
        }
        profile_dirs.push(dir.join("debug"));
        profile_dirs.push(dir.join("release"));
        ancestor = dir.parent();
        depth += 1;
    }
    profile_dirs
        .into_iter()
        .map(|dir| dir.join(binary_name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod expansion_probe_guard_tests {
    //! rubash#2585: the probe guard must restore the PRIOR value, so a
    //! nested probe scope leaves an outer probe armed (the plain-bool
    //! set(true)/set(false) pair let the inner scope's set(false)
    //! silently disarm the outer one).
    use super::ExpansionProbeGuard;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn issue2585_nested_guard_restores_outer_armed_state() {
        let flag = Rc::new(Cell::new(false));
        {
            let _outer = ExpansionProbeGuard::new(&flag);
            assert!(flag.get());
            {
                let _inner = ExpansionProbeGuard::new(&flag);
                assert!(flag.get());
            }
            // Inner drop must restore the outer scope's `true`, not `false`.
            assert!(flag.get());
        }
        assert!(!flag.get());
    }

    #[test]
    fn issue2585_guard_restores_prior_true() {
        let flag = Rc::new(Cell::new(true));
        {
            let _guard = ExpansionProbeGuard::new(&flag);
            assert!(flag.get());
        }
        assert!(flag.get());
    }
}
