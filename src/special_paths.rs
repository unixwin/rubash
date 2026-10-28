//! The special-path registry: the single home for "is this path
//! spelling special, and how" knowledge (vfs audit §4.3, slice S1).
//!
//! Before this module, the judgments that a path spelling is a network
//! redirect target, the null device, an fd alias, or a synthetic /proc
//! file lived as independent copies across the executor (audit §3.2
//! counted nine copies of special-path knowledge in the wider tree),
//! and each open point hand-wrote its own subset of the check sequence
//! (audit §3.3: six ladders, each missing different steps — the
//! #1887-class "the sibling path has it, this one doesn't" hazard).
//! The registry is the one definition every consumer asks:
//!
//! - the open ladders classify through [`classify_special_path`] and
//!   route by class (net / null / fd alias / synthetic content / plain
//!   filesystem), keeping only their own semantic pre-steps (noclobber,
//!   session-cwd anchoring, consume/probe discipline);
//! - the per-class query functions ([`is_net_redirect_target`],
//!   [`is_shell_null_device`], [`fd_alias_target`],
//!   [`synthetic_proc_content`], [`is_synthetic_link_path`]) are the
//!   same judgments in the shapes existing consumers already ask; they
//!   are defined here once, from the same bodies, so a query answer
//!   and a classification can never drift apart.
//!
//! Boundary (audit §4.2): the classification *semantics* here are the
//! shell's own on every platform (GNU invents /dev/tcp, /dev/fd and
//! friends in the shell, not the OS). The *simulation* behind the
//! synthetic-/proc class is Windows-only: it delegates to
//! [`crate::proc_vfs`], whose generator is `#[cfg(windows)]` with a
//! stub that answers `None` elsewhere, so off Windows the class is
//! simply empty and real procfs serves the path. Endpoint
//! construction (connecting the socket, dup'ing the fd, building the
//! content pipe) stays with the consumers for now; the mapping table
//! inside `shell_path_to_windows` joins this registry in a later
//! slice (audit §4.5 S3), and the namespace-bridge exclusion is
//! deliberately not part of this registry (audit §3.4: its current
//! form is the convergence ceiling).

/// How a path spelling is special to the shell, if it is. The classes
/// mirror the audit §4.3 registry categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecialPathClass {
    /// `/dev/tcp/host/port`, `/dev/udp/host/port` — not a filesystem
    /// path: opening connects a socket (GNU redir.c netopen,
    /// rubash#1026).
    NetRedirect,
    /// `/dev/null` (plus the DOS spelling `NUL` on Windows) — the
    /// sink/source-of-EOF device; opens route to the mapped device.
    NullDevice,
    /// `/dev/stdin`, `/dev/stdout`, `/dev/stderr`, `/dev/fd/N`,
    /// `/proc/self/fd/N` — an alias for a dup of the named fd in the
    /// shell's fd table, never a filesystem open.
    FdAlias(u32),
    /// A synthetic /proc file whose bytes the engine generates
    /// (Windows-only simulation; the class is empty off Windows).
    SyntheticProc,
    /// A synthetic /proc LINK entry (`/proc/self/exe`, `/proc/<pid>/cwd`):
    /// the entry has no bytes of its own; it resolves to a target path
    /// elsewhere in the filesystem (rubash#1900). Resolution is
    /// `proc_vfs::proc_link_target`; this class is the classification of
    /// the spelling plus the named process being alive, so a dead
    /// pid's entries classify as plain filesystem paths and fail
    /// loudly there, exactly like a vanished procfs directory.
    /// Windows-only simulation, like SyntheticProc. Admission
    /// self-check (the two-criteria rule): the /proc entry-name set is
    /// defined by the kernel's procfs — a CLOSED universe — and a miss
    /// falls back to the real filesystem, i.e. a loud ENOENT, never a
    /// silent wrong answer.
    SyntheticLink,
    /// Not special: an ordinary filesystem path (the path mapper and
    /// the filesystem own it from here).
    Filesystem,
}

/// Classify a path spelling against the registry. The check order is
/// the canonical ladder order shared by the open points: net, null,
/// fd alias, synthetic content, filesystem. The classes are disjoint
/// by spelling, so the order only matters for the one overlap family
/// — `/proc/self/fd/N` is an fd alias, not synthetic content (the
/// proc generator itself answers `None` for it) — and the alias wins,
/// exactly as the reference ladder (open_input_redirect_impl) has
/// always sequenced it.
pub(crate) fn classify_special_path(target: &str) -> SpecialPathClass {
    if is_net_redirect_target(target) {
        SpecialPathClass::NetRedirect
    } else if is_shell_null_device(target) {
        SpecialPathClass::NullDevice
    } else if let Some(fd) = fd_alias_target(target) {
        SpecialPathClass::FdAlias(fd)
    } else if is_synthetic_link_path(target) {
        // Link spellings and content spellings are disjoint (exe/cwd
        // generate no bytes), so this arm's position ahead of the
        // content check is for definition completeness, per the #1917
        // design's canonical order: fd alias, link, content.
        SpecialPathClass::SyntheticLink
    } else if synthetic_proc_content(target).is_some() {
        SpecialPathClass::SyntheticProc
    } else {
        SpecialPathClass::Filesystem
    }
}

/// rubash#1026: GNU redir.c netopen — `/dev/tcp/host/port` and
/// `/dev/udp/host/port` are not filesystem paths: opening one
/// connects a socket (TCP connect(2); UDP bind+connect, which
/// succeeds even with no listener). Host may be a name (resolved by
/// the OS) or an address; the port may be numeric or a service name
/// resolved through /etc/services (getservbyname).
pub(crate) fn is_net_redirect_target(target: &str) -> bool {
    target.starts_with("/dev/tcp/") || target.starts_with("/dev/udp/")
}

/// Whether `path` spells the null device: the POSIX spelling (with
/// backslashes normalized, as Windows spellings arrive in both
/// forms) plus, on Windows, the DOS device name in any case.
pub(crate) fn is_shell_null_device(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized == "/dev/null" || (cfg!(windows) && normalized.eq_ignore_ascii_case("NUL"))
}

/// GNU redir.c opens `/dev/stdin`/`/dev/stdout`/`/dev/stderr`,
/// `/dev/fd/N`, and `/proc/self/fd/N` through the OS's fd-alias device
/// files, which the kernel resolves to a dup of fd N — behaviorally
/// the same as `>&N`/`<&N`. Windows has no such filesystem, so the
/// names are recognized here and flow through the fd-dup machinery
/// (niubash#118: `>> /dev/stdout` used to land on the CONOUT$ device
/// path and fail with Permission denied).
pub(crate) fn fd_alias_target(target: &str) -> Option<u32> {
    match target {
        "/dev/stdin" => return Some(0),
        "/dev/stdout" => return Some(1),
        "/dev/stderr" => return Some(2),
        _ => {}
    }
    let fd = target
        .strip_prefix("/dev/fd/")
        .or_else(|| target.strip_prefix("/proc/self/fd/"))?;
    (!fd.is_empty() && fd.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| fd.parse::<u32>().ok())
        .flatten()
}

/// The registry's entry for the synthetic-/proc class: the file's
/// generated bytes, or `None` when `target` is not a synthetic /proc
/// file. The generator lives in [`crate::proc_vfs`] (the Windows-only
/// simulation; off Windows it answers `None` and the path falls
/// through to the real filesystem). Each call generates a fresh
/// snapshot, matching procfs semantics.
pub(crate) fn synthetic_proc_content(target: &str) -> Option<Vec<u8>> {
    crate::proc_vfs::proc_file_content(target)
}

/// The registry's classification query for the synthetic-link class:
/// whether `target` names a synthetic /proc link entry whose process
/// is alive. Resolution to the target path is a separate question
/// (`proc_vfs::proc_link_target`) because one entry
/// (`/proc/self/cwd`) resolves through the caller's session state,
/// which classification must not need.
pub(crate) fn is_synthetic_link_path(target: &str) -> bool {
    crate::proc_vfs::proc_is_link_path(target)
}

/// TEMPORARY diagnostic for the A-1 read-face hunt (remove with the
/// commit that resolves it): appends one line to ./rb-proc-trace.log
/// when that file exists in the process cwd; a no-op otherwise.
pub(crate) fn proc_trace(point: &str, detail: &str) {
    let path = std::path::Path::new("rb-proc-trace.log");
    if path.exists() {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = writeln!(file, "{point}: {detail}");
        }
    }
}

/// Resolve a synthetic-link path to its target (rubash#1900): both
/// spellings — the OS-native one for opens/metadata and the
/// shell-namespace display one for readlink-style answers. The
/// caller supplies the session cwd's forms for the self/cwd entry.
/// None when the path is not a live synthetic link or its target
/// cannot be resolved (dead pid, protected process).
pub(crate) fn synthetic_link_target(
    target: &str,
    self_cwd: Option<&crate::proc_vfs::ProcSelfCwd>,
) -> Option<crate::proc_vfs::ProcLinkTarget> {
    if !is_synthetic_link_path(target) {
        return None;
    }
    crate::proc_vfs::proc_link_target(target, self_cwd)
}

/// The registry's enumeration face for synthetic /proc directories:
/// the entry names of `/proc`, `/proc/self`, or a live `/proc/<pid>`,
/// from the one enumeration source in [`crate::proc_vfs`]. Glob and
/// (later) completion ask here so the names a user can complete are
/// exactly the names the content and link faces serve.
pub(crate) fn synthetic_proc_dir_entries(target: &str) -> Option<Vec<String>> {
    crate::proc_vfs::proc_dir_entries(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-registry judgments, restated from their original
    /// definitions, so the battery below proves the move changed no
    /// answer. (The net/null/alias bodies were copied verbatim into
    /// this module; this reference copy lives only in the test so a
    /// future edit to one side trips the comparison.)
    fn legacy_is_net(target: &str) -> bool {
        target.starts_with("/dev/tcp/") || target.starts_with("/dev/udp/")
    }

    fn legacy_is_null(path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        normalized == "/dev/null" || (cfg!(windows) && normalized.eq_ignore_ascii_case("NUL"))
    }

    fn legacy_fd_alias(target: &str) -> Option<u32> {
        match target {
            "/dev/stdin" => return Some(0),
            "/dev/stdout" => return Some(1),
            "/dev/stderr" => return Some(2),
            _ => {}
        }
        let fd = target
            .strip_prefix("/dev/fd/")
            .or_else(|| target.strip_prefix("/proc/self/fd/"))?;
        (!fd.is_empty() && fd.chars().all(|ch| ch.is_ascii_digit()))
            .then(|| fd.parse::<u32>().ok())
            .flatten()
    }

    fn expected_proc() -> SpecialPathClass {
        if cfg!(windows) {
            SpecialPathClass::SyntheticProc
        } else {
            SpecialPathClass::Filesystem
        }
    }

    fn expected_null_dos() -> SpecialPathClass {
        if cfg!(windows) {
            SpecialPathClass::NullDevice
        } else {
            SpecialPathClass::Filesystem
        }
    }

    fn expected_link() -> SpecialPathClass {
        if cfg!(windows) {
            SpecialPathClass::SyntheticLink
        } else {
            SpecialPathClass::Filesystem
        }
    }

    #[test]
    fn classify_battery_matches_legacy_judgments() {
        use SpecialPathClass::*;
        let battery: &[(&str, SpecialPathClass)] = &[
            // Net (netopen spellings; the bare prefix already counts,
            // exactly as the legacy starts_with judgment answered).
            ("/dev/tcp/127.0.0.1/80", NetRedirect),
            ("/dev/udp/example.com/53", NetRedirect),
            ("/dev/tcp/", NetRedirect),
            ("/dev/tcp", Filesystem),
            ("/dev/udp", Filesystem),
            ("/dev/tcpx/1", Filesystem),
            // Null device.
            ("/dev/null", NullDevice),
            ("\\dev\\null", NullDevice),
            ("NUL", expected_null_dos()),
            ("nul", expected_null_dos()),
            ("/dev/null/", Filesystem),
            ("x/dev/null", Filesystem),
            // Fd aliases.
            ("/dev/stdin", FdAlias(0)),
            ("/dev/stdout", FdAlias(1)),
            ("/dev/stderr", FdAlias(2)),
            ("/dev/fd/0", FdAlias(0)),
            ("/dev/fd/63", FdAlias(63)),
            ("/dev/fd/03", FdAlias(3)),
            ("/proc/self/fd/2", FdAlias(2)),
            ("/dev/fd/", Filesystem),
            ("/dev/fd/3x", Filesystem),
            ("/dev/fd/-1", Filesystem),
            ("/dev/fd/4294967296", Filesystem),
            ("/proc/self/fd/", Filesystem),
            ("/dev/fd", Filesystem),
            // Synthetic /proc (Windows-only class; off Windows the
            // same spellings are ordinary filesystem paths).
            ("/proc/meminfo", expected_proc()),
            ("/proc/cpuinfo", expected_proc()),
            ("/proc/self/cmdline", expected_proc()),
            ("/proc/self/status", expected_proc()),
            ("/proc/definitely-not-a-file", Filesystem),
            ("/proc/self/fd/abc", Filesystem),
            // Synthetic links (rubash#1900): the self spellings are
            // live by definition; a pid outside the allocatable range
            // is dead on every machine, so its entries fall through to
            // the filesystem and fail loudly there.
            ("/proc/self/exe", expected_link()),
            ("/proc/self/cwd", expected_link()),
            ("/proc/4294967294/exe", Filesystem),
            ("/proc/4294967294/cwd", Filesystem),
            ("/proc/self/exe/child", Filesystem),
            ("/proc/self/status/child", Filesystem),
            // The one overlap family: the fd-alias reading wins over
            // the /proc reading on every platform.
            ("/proc/self/fd/0", FdAlias(0)),
            // Ordinary paths and not-yet-implemented device spellings
            // (/dev/zero is audit D5, a later slice) stay Filesystem.
            ("/dev/tty", Filesystem),
            ("/dev/zero", Filesystem),
            ("etc/hosts", Filesystem),
            ("/tmp/x", Filesystem),
            ("", Filesystem),
        ];
        for (target, expected) in battery {
            let class = classify_special_path(target);
            assert_eq!(class, *expected, "classify({target:?})");
            // Every per-class query must agree with the classification
            // and with the legacy judgment it replaced.
            assert_eq!(
                is_net_redirect_target(target),
                legacy_is_net(target),
                "net query drift for {target:?}"
            );
            assert_eq!(
                is_net_redirect_target(target),
                class == NetRedirect,
                "net query vs classify for {target:?}"
            );
            assert_eq!(
                is_shell_null_device(target),
                legacy_is_null(target),
                "null query drift for {target:?}"
            );
            assert_eq!(
                is_shell_null_device(target),
                class == NullDevice,
                "null query vs classify for {target:?}"
            );
            assert_eq!(
                fd_alias_target(target),
                legacy_fd_alias(target),
                "alias query drift for {target:?}"
            );
            assert_eq!(
                fd_alias_target(target),
                match class {
                    FdAlias(fd) => Some(fd),
                    _ => None,
                },
                "alias query vs classify for {target:?}"
            );
            assert_eq!(
                synthetic_proc_content(target).is_some(),
                class == SyntheticProc,
                "proc query vs classify for {target:?}"
            );
            assert_eq!(
                is_synthetic_link_path(target),
                class == SyntheticLink,
                "link query vs classify for {target:?}"
            );
        }
    }

    /// rubash#1900: resolution answers exactly for the link class —
    /// self/cwd passes the caller's session-cwd forms through
    /// untouched, and non-link spellings resolve to None.
    #[test]
    fn link_target_resolution_matches_classification() {
        if cfg!(windows) {
            let cwd = crate::proc_vfs::self_cwd_forms(std::path::Path::new("C:\\session"));
            let target = synthetic_link_target("/proc/self/cwd", Some(&cwd))
                .expect("self/cwd resolves through the session forms");
            assert_eq!(target.native, "C:\\session");
            assert_eq!(target.display, cwd.display);
            assert!(synthetic_link_target("/proc/self/exe", None).is_some());
        }
        assert!(synthetic_link_target("/proc/self/status", None).is_none());
        assert!(synthetic_link_target("/etc/passwd", None).is_none());
    }
}
