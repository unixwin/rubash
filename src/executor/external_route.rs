//! The external-command route seam (niubash#567 P3; ffi-route
//! DESIGN §2.1): a host-registered provider the executor consults
//! when a simple command is about to be spawned, after the name has
//! resolved through the ordinary hash/PATH walk to a concrete
//! program. The provider may execute the call in-process (an
//! accelerator DLL is the motivating host) or decline it, in which
//! case the engine spawns the resolved program exactly as it would
//! have without a provider. With no provider registered the seam is
//! inert: dispatch is byte-identical to the pre-seam engine.
//!
//! This seam is deliberately NOT the buffered host channel
//! ([`super::HostExternalCommandOutput`], the 2026-07-31 closure):
//! that signature has no stdin channel, no cwd, stdio as whole-buffer
//! copies, and no way to express cancellation or signal death — the
//! defect set of the retired in-process route, restated in safe
//! Rust. The two coexist: the buffered channel keeps its embedding
//! uses, and nothing here extends or reuses its signature.
//!
//! The contract a provider executes under:
//!
//! - **argv is final.** `argv` is the exact vector the spawn path
//!   would exec: post-expansion, post dev-fd / proc-operand
//!   materialization, `argv[0]` the command word as typed. A host
//!   must not rewrite, reorder, or augment it — a command that
//!   needs argument adaptation to behave identically is not
//!   routable, and the host's classifier must decline it.
//! - **env is complete.** `env` is the whole environment the
//!   spawned child would receive, in GNU block composition order
//!   (variables.c maybe_make_export_env: the command's prefix
//!   assignments first, then the shell's exported variables, then
//!   exported functions; add_or_supercede throughout), finished
//!   with `_` set to the resolved program pathname
//!   (put_command_name_into_env), superseding any inherited or
//!   prefix-assigned entry.
//! - **stdio is live handles, never buffers.** Each slot is either
//!   the process's own standard handle (the provider's "keep
//!   current" case) or a concrete OS handle the engine resolved
//!   for this call — redirect files, dup'd fd-table descriptions,
//!   the command-substitution capture pipe, a temp file carrying a
//!   heredoc body. The engine upholds the same drain discipline as
//!   the spawn path: a capture pipe handed to the provider is
//!   drained concurrently while the provider runs (rubash#2094), so
//!   a provider may write without regard to pipe capacity, exactly
//!   as a child may. There is no capture layer and no copy-back.
//! - **cwd is the session cwd**, absolute — the directory the
//!   spawned child would be born in (never the process cwd by
//!   accident; the engine passes `Command::current_dir` the same
//!   value on the spawn path).
//! - **cancellation is a peek, never a consume.** `cancel_check`
//!   reports whether an interrupt delivery for this process is
//!   pending in the engine's signal mailbox. It requeues what it
//!   reads: the boundary machinery adjudicates the delivery after
//!   the call exactly as it does after a foreground wait. A host
//!   composes its own interrupt sources on top (the returned
//!   [`ExternalRouteOutcome::Cancelled`] is how a call reports
//!   that the user interrupted it, from whichever source).
//!
//! Engine-side decline gates (the provider is not consulted, and
//! the spawn proceeds untouched, when any holds): the program runs
//! through the shell wrapper (`used_shell` — a script, not a native
//! identity a route could serve); the command carries fd >= 3 or
//! `{var}` redirects (their bindings travel the spawn-only
//! `__RUBASH_FD_*` channel / descriptor pins); dev-fd operand
//! materialization rewrote a word or holds deferred flushes; the
//! stdio planner declined the call's fd 1/2 (the legacy piped
//! shapes, whose byte routing is the spawn path's own machinery);
//! fd 1/2 resolve closed (the write-error surface belongs to the
//! legacy route); or the stdin source is a shape the seam does not
//! realize (see `route_stdin_handle`). Pipeline stages are not
//! consulted in this version of the seam: the stage executors'
//! stdio is a mediated payload model, not the live-handle model
//! above, and the buffered stage channel is exactly the shape this
//! seam exists to replace; stage routing is a follow-up that must
//! realize stage stdio as handles first.

use std::path::Path;

/// One stdio endpoint of a routed call, in OS terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRouteHandle {
    /// The process's own standard handle for this slot: the routed
    /// call reads/writes the shell's current stdin/stdout/stderr,
    /// which is what an inheriting child would receive. Providers
    /// map this to their ABI's "keep current" value (NULL in the
    /// winux FFI ABI v2).
    Inherit,
    /// A concrete OS handle — a Windows `HANDLE` value or a unix
    /// file descriptor number — borrowed for the duration of the
    /// call. The engine keeps the owning object alive until the
    /// provider returns; the provider must not close it or retain
    /// it past the call.
    Handle(usize),
}

impl ExternalRouteHandle {
    /// The raw OS value, or 0 for [`ExternalRouteHandle::Inherit`]
    /// (the NULL spelling provider ABIs use for "keep current").
    pub fn raw_or_zero(self) -> usize {
        match self {
            ExternalRouteHandle::Inherit => 0,
            ExternalRouteHandle::Handle(raw) => raw,
        }
    }
}

/// The three stdio endpoints of a routed call, resolved by the
/// engine from the command's redirections and the session's fd
/// table — the same bindings the spawned child would inherit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalRouteStdio {
    pub stdin: ExternalRouteHandle,
    pub stdout: ExternalRouteHandle,
    pub stderr: ExternalRouteHandle,
}

/// One routed external call, as data. All slices are borrowed from
/// the dispatch in flight; the provider executes synchronously and
/// must not retain them.
pub struct ExternalRouteRequest<'a> {
    /// The command word as typed (`argv[0]`), the name the host's
    /// route classifier judges.
    pub name: &'a str,
    /// The full final argv, `argv[0]` included — byte-identical to
    /// the vector the spawn path would exec.
    pub argv: &'a [String],
    /// The program the hash/PATH walk resolved `name` to: the
    /// identity the spawn path would exec and the route's fallback
    /// identity. A host binds its claims to this resolution (a name
    /// is only the route's when it resolves to the served program).
    pub program: &'a Path,
    /// The complete child environment as ordered pairs (see the
    /// module docs for the composition contract).
    pub env: &'a [(String, String)],
    /// The session's current working directory (absolute): the
    /// routed call's working directory.
    pub cwd: &'a Path,
    /// The call's resolved stdio endpoints.
    pub stdio: &'a ExternalRouteStdio,
    /// Non-consuming interrupt peek (see the module docs). The
    /// provider polls this while a long call runs and reports
    /// [`ExternalRouteOutcome::Cancelled`] when it turns true and
    /// the call stops cooperatively.
    pub cancel_check: &'a (dyn Fn() -> bool + 'a),
}

/// How a routed call ended, from the provider's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRouteOutcome {
    /// The call ran in-process; the value is its exit code, which
    /// becomes the command's `$?` verbatim (the reap-observation
    /// bookkeeping of an in-process route applies, as for the
    /// engine's other in-process routes: no child exists, so there
    /// is no signal death to note and no SIGCHLD to trap).
    Completed(i32),
    /// The provider does not serve this call (its classifier
    /// declined it, or its serving layer failed before the call
    /// ran): the engine spawns the resolved program with the
    /// already-prepared spawn state, byte-identically to having no
    /// provider. A provider returning `Declined` must not have
    /// produced output or other observable effects for the call.
    Declined,
    /// The call was interrupted by the user before or during
    /// execution and must NOT fall through to spawn: the engine
    /// records the interrupted status (130) and runs the same
    /// signal adjudication a blocked foreground wait runs, so a
    /// trapped SIGINT still dispatches at the command boundary
    /// and an untrapped one unwinds with the session's usual
    /// classification.
    Cancelled,
}

/// A host's external-command route provider (§5.5 capability).
///
/// Registered on a session through
/// [`SessionBuilder::external_route_provider`](crate::api::SessionBuilder::external_route_provider)
/// or directly on an executor through
/// [`Executor::set_external_route_provider`](crate::Executor::set_external_route_provider).
/// The engine is platform-neutral about the provider: it knows
/// nothing of DLLs, tiers, or command sets — classification policy
/// is entirely the host's, expressed through
/// [`ExternalRouteProvider::execute_route`]'s decline verdicts and
/// [`ExternalRouteProvider::route_annotation`]'s introspection
/// answers.
pub trait ExternalRouteProvider {
    /// Execute one routed call, or decline it. Called synchronously
    /// on the session's thread, at most once per external simple
    /// command, only after the engine's own decline gates pass.
    fn execute_route(&mut self, request: &ExternalRouteRequest<'_>) -> ExternalRouteOutcome;

    /// The introspection annotation for `name`: when this provider
    /// would claim the name, a short phrase the `type` /
    /// `command -V` describe faces append to the name's resolved
    /// path line (e.g. `winuxcmd FFI (in-process via
    /// winuxcore.dll)`). `None` — the default — claims nothing, and
    /// the describe output stays byte-identical to an engine with
    /// no provider. This is a pure route query: it must not force
    /// loads or execute anything. `command -v` / `type -t` /
    /// `type -p` never consult it — the external identity they
    /// report stays the resolved path in every switch state
    /// (ffi-route DESIGN §1.6).
    fn route_annotation(&self, _name: &str) -> Option<String> {
        None
    }
}

/// The executor's storage for a registered route provider (the
/// manual `Debug` mirrors `HostExternalCommandHandler`'s: a
/// provider is an opaque host object to the engine).
pub(crate) struct ExternalRouteProviderSlot(pub Box<dyn ExternalRouteProvider>);

impl std::fmt::Debug for ExternalRouteProviderSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExternalRouteProviderSlot(..)")
    }
}

/// The seam's cancellation peek: is a SIGINT delivery for this
/// process pending in the engine's signal mailbox? Peek + requeue,
/// never consumes — the same reduced predicate the fifo rendezvous
/// polls (`winux_fifo::fifo_sigint_pending`, Windows) generalized
/// to the mailbox both ports record into: trapped and untrapped
/// deliveries alike count (the provider only learns "the user
/// interrupted"; what the delivery MEANS is adjudicated later by
/// the boundary machinery that owns trap state).
pub(crate) fn route_sigint_pending() -> bool {
    let signals =
        crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
    let pending = signals.contains(&crate::builtins::kill::SIGINT_NUMBER);
    crate::builtins::kill::requeue_pending_signals(signals);
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Executor;
    use std::cell::RefCell;
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use std::rc::Rc;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Behavior {
        /// Read the stdin handle to EOF, write the bytes to the
        /// stdout handle (the `cat` shape).
        CopyStdin,
        /// Write the value of the request env entry named by
        /// argv[1] (+ "\n") to the stdout handle; rc 1 when
        /// absent (the `printenv` shape).
        PrintEnv,
        /// Complete immediately with this exit code.
        Fixed(i32),
        /// Report cooperative cancellation immediately.
        CancelNow,
        /// Claim nothing at execution time (annotation may still
        /// claim): every call declines.
        DeclineAll,
    }

    #[derive(Default)]
    struct Record {
        calls: Vec<String>,
        envs: Vec<Vec<(String, String)>>,
        cwds: Vec<PathBuf>,
    }

    struct MockRoute {
        claims: Vec<String>,
        behavior: Behavior,
        record: Rc<RefCell<Record>>,
    }

    /// Borrow an OS handle as a File without taking ownership:
    /// the engine keeps its realized objects alive for the call;
    /// the provider must not close what it borrowed.
    fn borrow_file(handle: ExternalRouteHandle) -> Option<std::mem::ManuallyDrop<std::fs::File>> {
        match handle {
            ExternalRouteHandle::Inherit => None,
            ExternalRouteHandle::Handle(raw) => {
                #[cfg(unix)]
                {
                    use std::os::unix::io::FromRawFd;
                    Some(std::mem::ManuallyDrop::new(unsafe {
                        std::fs::File::from_raw_fd(raw as std::os::unix::io::RawFd)
                    }))
                }
                #[cfg(windows)]
                {
                    use std::os::windows::io::FromRawHandle;
                    Some(std::mem::ManuallyDrop::new(unsafe {
                        std::fs::File::from_raw_handle(raw as std::os::windows::io::RawHandle)
                    }))
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = raw;
                    None
                }
            }
        }
    }

    impl ExternalRouteProvider for MockRoute {
        fn execute_route(&mut self, request: &ExternalRouteRequest) -> ExternalRouteOutcome {
            if self.behavior == Behavior::DeclineAll
                || !self.claims.iter().any(|name| name == request.name)
            {
                return ExternalRouteOutcome::Declined;
            }
            {
                let mut record = self.record.borrow_mut();
                record.calls.push(request.name.to_string());
                record.envs.push(request.env.to_vec());
                record.cwds.push(request.cwd.to_path_buf());
            }
            match self.behavior {
                Behavior::CopyStdin => {
                    let mut bytes = Vec::new();
                    if let Some(mut stdin) = borrow_file(request.stdio.stdin) {
                        let _ = stdin.read_to_end(&mut bytes);
                    }
                    if let Some(mut stdout) = borrow_file(request.stdio.stdout) {
                        let _ = stdout.write_all(&bytes);
                        let _ = stdout.flush();
                    } else {
                        let _ = std::io::stdout().write_all(&bytes);
                    }
                    ExternalRouteOutcome::Completed(0)
                }
                Behavior::PrintEnv => {
                    let key = request.argv.get(1).map(String::as_str).unwrap_or("");
                    match request.env.iter().find(|(name, _)| name == key) {
                        Some((_, value)) => {
                            if let Some(mut stdout) = borrow_file(request.stdio.stdout) {
                                let _ = writeln!(stdout, "{value}");
                            } else {
                                println!("{value}");
                            }
                            ExternalRouteOutcome::Completed(0)
                        }
                        None => ExternalRouteOutcome::Completed(1),
                    }
                }
                Behavior::Fixed(status) => ExternalRouteOutcome::Completed(status),
                Behavior::CancelNow => ExternalRouteOutcome::Cancelled,
                Behavior::DeclineAll => ExternalRouteOutcome::Declined,
            }
        }

        fn route_annotation(&self, name: &str) -> Option<String> {
            if self.claims.iter().any(|claimed| claimed == name) {
                Some("winuxcmd FFI (in-process via winuxcore.dll)".to_string())
            } else {
                None
            }
        }
    }

    fn run(executor: &mut Executor, source: &str) -> i32 {
        crate::script_driver::run_source(executor, source, false)
    }

    fn routed_executor(claims: &[&str], behavior: Behavior) -> (Executor, Rc<RefCell<Record>>) {
        let record = Rc::new(RefCell::new(Record::default()));
        let mut executor = Executor::new();
        executor.set_external_route_provider(Box::new(MockRoute {
            claims: claims.iter().map(|name| name.to_string()).collect(),
            behavior,
            record: record.clone(),
        }));
        (executor, record)
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rubash-ffi-route-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).expect("output file")
    }

    /// One equivalence case: the same script under (a) no
    /// provider (the spawn baseline) and (b) a claiming provider
    /// must produce the same exit code and the same bytes in
    /// every file it writes — AND the routed run must prove the
    /// seam was actually consulted (a name the emulation layer
    /// claims ahead of the seam, like `cat`, would make the
    /// comparison vacuous; the battery's copy twin is `dd`,
    /// which no engine face claims).
    #[cfg(unix)]
    fn assert_equivalent(
        tag: &str,
        claims: &[&str],
        behavior: Behavior,
        script: &str,
        files: &[&str],
    ) {
        let mut outcomes = Vec::new();
        for routed in [false, true] {
            let dir = scratch(&format!("{tag}-{}", if routed { "route" } else { "spawn" }));
            std::fs::write(dir.join("in.txt"), "alpha\nbeta\n").expect("input");
            let (mut executor, record) = if routed {
                routed_executor(claims, behavior)
            } else {
                (Executor::new(), Rc::new(RefCell::new(Record::default())))
            };
            assert_eq!(run(&mut executor, &format!("cd {}", dir.display())), 0);
            let status = run(&mut executor, script);
            if routed && behavior != Behavior::DeclineAll {
                assert!(
                    record
                        .borrow()
                        .calls
                        .iter()
                        .any(|call| claims.contains(&call.as_str())),
                    "{tag}: the route was never consulted (calls: {:?})",
                    record.borrow().calls
                );
            }
            let mut bytes = Vec::new();
            for file in files {
                bytes.push(std::fs::read(dir.join(file)).unwrap_or_default());
            }
            outcomes.push((status, bytes));
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert_eq!(outcomes[0], outcomes[1], "route/spawn divergence in {tag}");
    }

    #[cfg(unix)]
    #[test]
    fn stdin_and_redirect_shapes_are_byte_equivalent() {
        // Heredoc body through the route's temp-file realization.
        assert_equivalent(
            "heredoc",
            &["dd"],
            Behavior::CopyStdin,
            "dd <<'EOF' > out.txt\nhello route\nEOF\n",
            &["out.txt"],
        );
        // Herestring.
        assert_equivalent(
            "herestring",
            &["dd"],
            Behavior::CopyStdin,
            "dd <<< 'one line' > out.txt",
            &["out.txt"],
        );
        // Plain file in, file out.
        assert_equivalent(
            "file",
            &["dd"],
            Behavior::CopyStdin,
            "dd < in.txt > out.txt",
            &["out.txt"],
        );
        // Append across two routed calls.
        assert_equivalent(
            "append",
            &["dd"],
            Behavior::CopyStdin,
            "dd < in.txt > out.txt\ndd < in.txt >> out.txt",
            &["out.txt"],
        );
        // An exec-installed fd 0 is shared by offset: the second
        // reader sees EOF, on both routes.
        assert_equivalent(
            "execfd0",
            &["dd"],
            Behavior::CopyStdin,
            "exec 0<in.txt\ndd > first.txt\ndd > second.txt",
            &["first.txt", "second.txt"],
        );
        // The FUNCTION_STDIN buffer: a function's stdin is the
        // call's herestring, drained by the routed reader.
        assert_equivalent(
            "funcstdin",
            &["dd"],
            Behavior::CopyStdin,
            "f() { dd > out.txt; }\nf <<< 'via function'",
            &["out.txt"],
        );
        // A heredoc body with an embedded substitution: the
        // substitution runs EXACTLY ONCE whichever route the call
        // takes — the route pull performs it (the feed's own
        // pull), a decline hands the pulled text to the spawn's
        // feed instead of pulling again. The counter file is the
        // side-effect ledger: one line per run of the body.
        let body_script =
            "dd <<EOF > out.txt\nval=$(printf sub; printf 'x\\n' >> counter.txt)\nEOF\n";
        assert_equivalent(
            "heredoc-comsub-routed",
            &["dd"],
            Behavior::CopyStdin,
            body_script,
            &["out.txt", "counter.txt"],
        );
        assert_equivalent(
            "heredoc-comsub-declined",
            &["dd"],
            Behavior::DeclineAll,
            body_script,
            &["out.txt", "counter.txt"],
        );
    }

    #[cfg(unix)]
    #[test]
    fn capture_and_env_shapes_are_byte_equivalent() {
        // Command-substitution capture past the pipe buffer: the
        // drain thread must deliver every byte on the route.
        let dir = scratch("bigcap");
        let big: Vec<u8> = (0..200_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        std::fs::write(dir.join("big.bin"), &big).expect("big input");
        for routed in [false, true] {
            let (mut executor, record) = if routed {
                routed_executor(&["dd"], Behavior::CopyStdin)
            } else {
                (Executor::new(), Rc::new(RefCell::new(Record::default())))
            };
            assert_eq!(run(&mut executor, &format!("cd {}", dir.display())), 0);
            assert_eq!(
                run(
                    &mut executor,
                    "x=$(dd < big.bin)\nprintf '%s' \"$x\" > cap.txt"
                ),
                0
            );
            assert_eq!(std::fs::read(dir.join("cap.txt")).unwrap(), big);
            if routed {
                assert_eq!(record.borrow().calls, vec!["dd".to_string()]);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);

        // Prefix assignment, exported variable, and the `_` pair.
        assert_equivalent(
            "prefixenv",
            &["printenv"],
            Behavior::PrintEnv,
            "FOO=bar printenv FOO > out.txt",
            &["out.txt"],
        );
        assert_equivalent(
            "exportenv",
            &["printenv"],
            Behavior::PrintEnv,
            "export FOO=bar\nprintenv FOO > out.txt",
            &["out.txt"],
        );
        assert_equivalent(
            "appendenv",
            &["printenv"],
            Behavior::PrintEnv,
            "A=xy\nA+=z printenv A > out.txt",
            &["out.txt"],
        );
        assert_equivalent(
            "underscore",
            &["printenv"],
            Behavior::PrintEnv,
            "printenv _ > out.txt",
            &["out.txt"],
        );
    }

    #[cfg(unix)]
    #[test]
    fn cwd_reaches_the_provider_and_the_child_environment() {
        let dir = scratch("cwd");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).expect("subdir");
        let (mut executor, record) = routed_executor(&["printenv"], Behavior::PrintEnv);
        assert_eq!(run(&mut executor, &format!("cd {}", dir.display())), 0);
        assert_eq!(run(&mut executor, "cd sub"), 0);
        assert_eq!(run(&mut executor, "printenv PWD > pwd.txt"), 0);
        // The routed call ran in the session cwd...
        assert_eq!(record.borrow().cwds, vec![sub.clone()]);
        // ...and its PWD answer matches the spawn answer.
        let mut baseline = Executor::new();
        assert_eq!(run(&mut baseline, &format!("cd {}", dir.display())), 0);
        assert_eq!(run(&mut baseline, "cd sub"), 0);
        assert_eq!(run(&mut baseline, "printenv PWD > pwd-base.txt"), 0);
        assert_eq!(read(&sub, "pwd.txt"), read(&sub, "pwd-base.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn status_and_cancellation_plumbing() {
        // A provider-reported status becomes $? verbatim.
        let (mut executor, _r) = routed_executor(&["dd"], Behavior::Fixed(42));
        assert_eq!(run(&mut executor, "dd < /dev/null"), 42);
        // The spawn twin reports dd's own 0: the seam's contract
        // is that the provider's status IS the call's status.
        let mut baseline = Executor::new();
        assert_eq!(run(&mut baseline, "dd < /dev/null"), 0);

        // Cooperative cancellation lands as the interrupted
        // status 130. (The seam-level shape: the provider reports
        // the stop; a real long-running call's interrupt arrives
        // through the request's cancel_check — the composition
        // niubash's provider tests pin, and the Windows real-DLL
        // proof is niubash#567 gate item ③.)
        let (mut executor, _r) = routed_executor(&["dd"], Behavior::CancelNow);
        assert_eq!(run(&mut executor, "dd < /dev/null"), 130);
    }

    #[cfg(unix)]
    #[test]
    fn pipeline_stages_are_not_consulted_in_v1() {
        let dir = scratch("pipe");
        let (mut executor, record) = routed_executor(&["dd"], Behavior::CopyStdin);
        assert_eq!(run(&mut executor, &format!("cd {}", dir.display())), 0);
        assert_eq!(run(&mut executor, "printf 'x\\n' | dd > out.txt"), 0);
        assert_eq!(read(&dir, "out.txt"), "x\n");
        assert!(
            record.borrow().calls.is_empty(),
            "pipeline stages must stay on the spawn paths in v1"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn declining_provider_is_byte_identical_to_no_provider() {
        assert_equivalent(
            "declineall",
            &["dd"],
            Behavior::DeclineAll,
            "dd < in.txt > out.txt\ndd <<'EOF' >> out.txt\nmore\nEOF\n",
            &["out.txt"],
        );
    }

    #[cfg(unix)]
    #[test]
    fn annotation_is_checkable_and_never_noisy() {
        let dir = scratch("annotate");
        let (mut executor, _r) = routed_executor(&["dd"], Behavior::CopyStdin);
        assert_eq!(run(&mut executor, &format!("cd {}", dir.display())), 0);
        // Verbose: the resolved-path line carries the suffix.
        assert_eq!(run(&mut executor, "type dd > t1.txt"), 0);
        let typed = read(&dir, "t1.txt");
        assert!(
            typed.contains(" — winuxcmd FFI (in-process via winuxcore.dll)"),
            "annotated type output, got: {typed}"
        );
        // command -V is the same Verbose face.
        assert_eq!(run(&mut executor, "command -V dd > t2.txt"), 0);
        assert!(read(&dir, "t2.txt").contains(" — winuxcmd FFI"));
        // Reusable (command -v) and TypeOnly stay byte-plain.
        assert_eq!(run(&mut executor, "command -v dd > t3.txt"), 0);
        assert!(!read(&dir, "t3.txt").contains("FFI"));
        assert_eq!(run(&mut executor, "type -t dd > t4.txt"), 0);
        assert_eq!(read(&dir, "t4.txt"), "file\n");
        // An unclaimed name is unannotated.
        assert_eq!(run(&mut executor, "type ls > t5.txt"), 0);
        assert!(!read(&dir, "t5.txt").contains("FFI"));
        // No provider at all: `type dd` is byte-identical to the
        // claiming session's line minus the suffix.
        let mut baseline = Executor::new();
        assert_eq!(run(&mut baseline, &format!("cd {}", dir.display())), 0);
        assert_eq!(run(&mut baseline, "type dd > t6.txt"), 0);
        assert_eq!(
            typed.replace(" — winuxcmd FFI (in-process via winuxcore.dll)", ""),
            read(&dir, "t6.txt")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
