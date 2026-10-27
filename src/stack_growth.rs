//! On-demand stack growth for the recursive tree walkers (rubash#1801).
//!
//! The parser and the executor walk the command tree recursively, exactly
//! like GNU: `execute_cmd.c` dispatches compound commands by recursive C
//! calls, and the only depth limit GNU knows is FUNCNEST for function
//! calls (`execute_cmd.c:324-325` `funcnest`/`funcnest_max`, enforced at
//! `execute_cmd.c:5200` only when FUNCNEST > 0; `variables.c:5945`
//! `sv_funcnest`) — compound nesting itself is bounded only by the real
//! stack. GNU owns its stack (the shell process's main thread), but this
//! crate is a library: hosts call `Executor::execute_ast` and
//! `parser::parse` on threads they own, and the walkers' debug call chains
//! burn ~150KB of stack per nesting level (the figure `src/main.rs` cites
//! when it gives the binary driver a 512MiB stack thread). On an ordinary
//! host thread — libtest's 2MiB test threads, an embedder's worker — a
//! script nested a handful of levels, or a function chain a few calls
//! deep, overflowed the stack and took the whole process down (SIGSEGV /
//! SIGABRT in the executor_tests harness, rubash#1801).
//!
//! The fix sits at the resource layer, not the semantics layer: the two
//! recursion hubs (`parser::parse_loop::run_command_loop` and
//! `Executor::execute_ast_inner`, through which every nested command list
//! funnels) pass through [`ensure_stack`], which grows the current
//! thread's stack when the remainder runs low. No depth is counted and no
//! limit is imposed — behavior is identical at every depth; the stack
//! simply stops being the binding constraint before memory is, matching
//! the contract GNU gets from owning its process.
//!
//! Growth is provided by `stacker`, whose assembly backend (`psm`) only
//! supports a fixed platform matrix, so the dependency is gated in
//! `Cargo.toml` to the targets psm 0.1.32 actually covers (architecture
//! × OS × target environment; OpenHarmony, for instance, is a Linux-OS
//! target with an `ohos` environment that psm has no support for, and
//! its SDK assembler rejects psm's assembly outright). On any target
//! outside the matrix `stacker` is not part of the build at all and
//! [`ensure_stack`] invokes the closure directly — exactly the behavior
//! this crate had before #1801. Recursion depth there is bounded by the
//! host thread's own stack, as it was before the fix; every platform
//! psm supports keeps the grown stack.

/// Remaining-stack floor: when less than this is left, grow before
/// descending further. One walker level can burn ~250KB of debug stack
/// (fat opt-0 frames along the dispatch chain), so the floor keeps several
/// levels of headroom under the deepest single inter-hub chain observed.
#[cfg(all(
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "sparc",
        target_arch = "sparc64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ),
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "windows",
        target_os = "cygwin",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "aix",
        target_os = "fuchsia",
        target_os = "redox"
    ),
    any(
        target_env = "",
        target_env = "gnu",
        target_env = "musl",
        target_env = "msvc",
        target_env = "uclibc",
        target_env = "sim",
        target_env = "macabi"
    )
))]
const STACK_RED_ZONE_BYTES: usize = 1024 * 1024;

/// Size of each freshly grown stack segment. The reservation is virtual —
/// pages are committed on use — so an 8MiB segment costs address space,
/// not RAM, until the recursion actually descends into it.
#[cfg(all(
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "sparc",
        target_arch = "sparc64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ),
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "windows",
        target_os = "cygwin",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "aix",
        target_os = "fuchsia",
        target_os = "redox"
    ),
    any(
        target_env = "",
        target_env = "gnu",
        target_env = "musl",
        target_env = "msvc",
        target_env = "uclibc",
        target_env = "sim",
        target_env = "macabi"
    )
))]
const STACK_SEGMENT_BYTES: usize = 8 * 1024 * 1024;

/// Run `f` on the current thread, growing the stack first if the remaining
/// stack is below the red-zone floor. Growth never migrates threads:
/// `Executor` is a single-thread session object (it holds `Rc` state), so
/// `stacker` switches only the stack segment under the same thread.
///
/// This variant is compiled exactly where the `stacker` dependency is
/// enabled in `Cargo.toml` (the psm-supported target matrix); keep the
/// two cfg expressions in sync.
#[cfg(all(
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "sparc",
        target_arch = "sparc64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ),
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "windows",
        target_os = "cygwin",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "aix",
        target_os = "fuchsia",
        target_os = "redox"
    ),
    any(
        target_env = "",
        target_env = "gnu",
        target_env = "musl",
        target_env = "msvc",
        target_env = "uclibc",
        target_env = "sim",
        target_env = "macabi"
    )
))]
pub(crate) fn ensure_stack<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(STACK_RED_ZONE_BYTES, STACK_SEGMENT_BYTES, f)
}

/// Run `f` on the current thread. Fallback for targets outside psm's
/// supported matrix, where `stacker` is not part of the build (see the
/// module docs): the closure runs directly on the host thread's stack,
/// which is exactly the pre-#1801 behavior on those targets.
#[cfg(not(all(
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "sparc",
        target_arch = "sparc64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ),
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "windows",
        target_os = "cygwin",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "aix",
        target_os = "fuchsia",
        target_os = "redox"
    ),
    any(
        target_env = "",
        target_env = "gnu",
        target_env = "musl",
        target_env = "msvc",
        target_env = "uclibc",
        target_env = "sim",
        target_env = "macabi"
    )
)))]
pub(crate) fn ensure_stack<R>(f: impl FnOnce() -> R) -> R {
    f()
}
