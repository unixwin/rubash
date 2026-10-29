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
//! The judgments are also the registry's public consumer face: glob
//! asks [`synthetic_proc_dir_entries`] in-crate, and the host's
//! interactive faces (niubash completion and syntax highlighting,
//! vfs audit slice S5) ask the classification and the enumeration
//! faces ([`special_dev_entries`], [`synthetic_proc_dir_entries`])
//! from outside the crate, so the names a user can complete or gets
//! styled as paths are exactly the names the open points serve —
//! never a second, host-side copy of the special-path knowledge.
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
pub enum SpecialPathClass {
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
pub fn classify_special_path(target: &str) -> SpecialPathClass {
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
pub fn is_net_redirect_target(target: &str) -> bool {
    target.starts_with("/dev/tcp/") || target.starts_with("/dev/udp/")
}

/// Whether `path` spells the null device: the POSIX spelling (with
/// backslashes normalized, as Windows spellings arrive in both
/// forms) plus, on Windows, the DOS device name in any case.
pub fn is_shell_null_device(path: &str) -> bool {
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
pub fn fd_alias_target(target: &str) -> Option<u32> {
    match target {
        "/dev/stdin" => return Some(0),
        "/dev/stdout" => return Some(1),
        "/dev/stderr" => return Some(2),
        _ => {}
    }
    let fd = target
        .strip_prefix("/dev/fd/")
        .or_else(|| target.strip_prefix("/proc/self/fd/"))?;
    canonical_fd_number(fd)
}

/// The registry's numeric field is canonical decimal — exactly the
/// spelling set the kernel's fd-alias names use: ASCII digits only, no
/// sign, no whitespace, and no leading zero ("0" itself excepted).
///
/// GNU anchor: on the oracle platform (Linux) bash is built with
/// HAVE_DEV_FD, so `_redir_special_filenames` (redir.c:601) does not
/// intercept `/dev/fd/N` and the literal open decides — procfs serves
/// only canonical names, so `/dev/fd/00`, `/dev/fd/03`, `/dev/fd/007`,
/// `/proc/self/fd/00` fail ENOENT on every face (cat, redirect,
/// `test -r`, `source`; issue rubash#1973 oracle probes, bash
/// 5.3.20). The no-dev-fd fallback (`redir.c:630 redir_special_open`,
/// `all_digits` general.c:233) likewise never parses a sign.
fn canonical_fd_number(fd: &str) -> Option<u32> {
    if !fd.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if fd.len() > 1 && fd.starts_with('0') {
        return None;
    }
    fd.parse::<u32>().ok()
}

/// The registry's entry for the synthetic-/proc class: the file's
/// generated bytes, or `None` when `target` is not a synthetic /proc
/// file. The generator lives in [`crate::proc_vfs`] (the Windows-only
/// simulation; off Windows it answers `None` and the path falls
/// through to the real filesystem). Each call generates a fresh
/// snapshot, matching procfs semantics.
pub fn synthetic_proc_content(target: &str) -> Option<Vec<u8>> {
    crate::proc_vfs::proc_file_content(target)
}

/// The registry's classification query for the synthetic-link class:
/// whether `target` names a synthetic /proc link entry whose process
/// is alive. Resolution to the target path is a separate question
/// (`proc_vfs::proc_link_target`) because one entry
/// (`/proc/self/cwd`) resolves through the caller's session state,
/// which classification must not need.
pub fn is_synthetic_link_path(target: &str) -> bool {
    crate::proc_vfs::proc_is_link_path(target)
}

/// Resolve a synthetic-link path to its target (rubash#1900): both
/// spellings — the OS-native one for opens/metadata and the
/// shell-namespace display one for readlink-style answers. The
/// caller supplies the session cwd's forms for the self/cwd entry.
/// None when the path is not a live synthetic link or its target
/// cannot be resolved (dead pid, protected process).
pub fn synthetic_link_target(
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
pub fn synthetic_proc_dir_entries(target: &str) -> Option<Vec<String>> {
    crate::proc_vfs::proc_dir_entries(target)
}

/// The listable children of the shell-namespace `/dev` directory:
/// `(name, is_prefix_dir)`. Interactive completion lists exactly
/// these names under a `/dev/` prefix (niubash#627, vfs audit §6.4),
/// so the static completion tier can never promise a spelling the
/// open points do not serve, nor hide one they do.
///
/// Leaf entries are the spellings that classify as a device or fd
/// alias (`/dev/null`, `/dev/stdin`, `/dev/stdout`, `/dev/stderr`).
/// Prefix-dir entries are families whose children are NOT a fixed
/// name set: `fd` (children are the session's live fd numbers — the
/// dynamic tier, enumerated from the fd table itself, never from
/// this list), and `tcp`/`udp` (children are free-text host/port
/// pairs; the dir itself is listed as the guide form, and nothing
/// completes past it — GNU has no hostname completion there).
///
/// Spellings the registry does not classify — `/dev/zero`,
/// `/dev/full`, `/dev/tty` today — are deliberately absent: to the
/// open points they are plain filesystem paths (the enclave's real
/// directory serves them), so listing them here would invent a
/// simulation the engine does not perform (audit D5 owns them).
pub fn special_dev_entries() -> &'static [(&'static str, bool)] {
    &[
        ("null", false),
        ("stdin", false),
        ("stdout", false),
        ("stderr", false),
        ("fd", true),
        ("tcp", true),
        ("udp", true),
    ]
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
        // Issue rubash#1973: the pre-registry body also accepted
        // leading-zero spellings, which GNU rejects everywhere; the
        // mirror carries the corrected canonical judgment so the
        // battery below keeps tripping on any future drift.
        canonical_fd_number(fd)
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
            ("/dev/fd/10", FdAlias(10)),
            ("/proc/self/fd/2", FdAlias(2)),
            // Issue rubash#1973: leading-zero fd spellings are NOT fd
            // aliases — GNU 5.3.20 rejects them on every face (the
            // oracle's procfs serves only canonical names), so they are
            // plain filesystem paths that fail ENOENT there.
            ("/dev/fd/00", Filesystem),
            ("/dev/fd/01", Filesystem),
            ("/dev/fd/03", Filesystem),
            ("/dev/fd/007", Filesystem),
            ("/dev/fd/000", Filesystem),
            ("/proc/self/fd/00", Filesystem),
            ("/proc/self/fd/03", Filesystem),
            // GNU also rejects signs on every face (redir.c:630
            // redir_special_open → all_digits); the registry has never
            // accepted them — kept pinned.
            ("/dev/fd/+3", Filesystem),
            ("/proc/self/fd/+3", Filesystem),
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

    /// Issue rubash#1973 regression pin: the leading-zero family fails
    /// fd-alias recognition spelling by spelling (GNU 5.3.20 oracle:
    /// cat ENOENT, redirect ENOENT, `test -r` false, `source` ENOENT —
    /// procfs serves only canonical names, so even an OPEN fd 3 does
    /// not make `/dev/fd/03` resolve), while "0" and the canonical
    /// multi-digit forms keep their FdAlias answers.
    #[test]
    fn issue1973_leading_zero_fd_spellings_are_not_aliases() {
        for spelling in [
            "/dev/fd/00",
            "/dev/fd/01",
            "/dev/fd/03",
            "/dev/fd/007",
            "/dev/fd/000",
            "/proc/self/fd/00",
            "/proc/self/fd/03",
            "/proc/self/fd/007",
        ] {
            assert_eq!(
                fd_alias_target(spelling),
                None,
                "leading-zero spelling must not be an fd alias: {spelling:?}"
            );
            assert_eq!(
                classify_special_path(spelling),
                SpecialPathClass::Filesystem,
                "leading-zero spelling must classify as filesystem: {spelling:?}"
            );
        }
        for (spelling, fd) in [
            ("/dev/fd/0", 0),
            ("/dev/fd/10", 10),
            ("/dev/fd/63", 63),
            ("/proc/self/fd/0", 0),
            ("/proc/self/fd/2", 2),
        ] {
            assert_eq!(
                fd_alias_target(spelling),
                Some(fd),
                "canonical spelling must stay an fd alias: {spelling:?}"
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

    /// niubash#627 (vfs audit §6.5): the listable /dev table must
    /// agree with the classification judgments in both directions —
    /// every listed name is special to the open points, and every
    /// /dev spelling family the registry treats as special is
    /// listed. Probed over the audit's known /dev vocabulary, so a
    /// future registry arm (a /dev/zero simulation, audit D5) trips
    /// this pin until the table is updated in the same change.
    #[test]
    fn special_dev_entries_match_classification() {
        fn family_is_special(name: &str) -> bool {
            classify_special_path(&format!("/dev/{name}")) != SpecialPathClass::Filesystem
                || matches!(
                    classify_special_path(&format!("/dev/{name}/0")),
                    SpecialPathClass::FdAlias(_)
                )
                || classify_special_path(&format!("/dev/{name}/example.invalid/80"))
                    == SpecialPathClass::NetRedirect
        }

        let listed: Vec<&str> = special_dev_entries()
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for (name, is_dir) in special_dev_entries() {
            assert!(
                family_is_special(name),
                "listed /dev entry is not special: {name}"
            );
            if *is_dir {
                // A prefix dir's own spelling is a plain directory to
                // the open points; only its children are special.
                assert_eq!(
                    classify_special_path(&format!("/dev/{name}")),
                    SpecialPathClass::Filesystem,
                    "prefix dir spelling must stay Filesystem: {name}"
                );
            } else {
                assert_ne!(
                    classify_special_path(&format!("/dev/{name}")),
                    SpecialPathClass::Filesystem,
                    "leaf entry must classify as special: {name}"
                );
            }
        }
        for name in [
            "null", "zero", "full", "stdin", "stdout", "stderr", "fd", "tcp", "udp", "tty", "ptmx",
            "console", "random", "urandom",
        ] {
            assert_eq!(
                listed.contains(&name),
                family_is_special(name),
                "/dev table vs classification drift for {name}"
            );
        }
    }
}
