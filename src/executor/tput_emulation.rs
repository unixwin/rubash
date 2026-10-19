//! #800A inline `tput` arm — the Windows spawn-budget face of the eco-harvest
//! SLOW list (kiddae/dotfiles hermine `.bashrc`: ~32 `$(tput …)` spawns paid
//! ~300ms each on the harvest runner, source_wall 9.7s).
//!
//! Triage (2026-10-19, release builds, this host): rubash's own spawn
//! machinery is at parity — 32×`uname` wall = 15-29ms (~0.5ms/spawn, the
//! benchmarks/29 probe) — while 32×`tput` costs ~550ms: the ~17ms/spawn delta
//! lives INSIDE the child (winuxcmd's tput dispatcher path; `true` on the
//! same 4MB binary answers in ~1ms). The arm removes the spawn entirely for
//! the invocation shapes the harvest corpus pays for, the same
//! execution-routing shape as the `tty`/`cat`/`sed` arms of
//! `external_file_builtins` (and the same hidden-builtin contract as
//! `uname`/`arch`, docs/builtins.md §3: introspection surfaces keep
//! reporting the PATH binary).
//!
//! Admission is a WHITELIST (rubash#117 discipline: the fall-through keeps
//! answering, so admission must be provable, not plausible — a blacklist of
//! symptoms is exactly the shape #117 forbids). Every admitted
//! (TERM, invocation) pair answers with bytes verified byte-for-byte against
//! the GNU oracle (WSL ncurses tput, Ubuntu 24.04 ncurses 6.4, probes
//! 2026-10-19 via `wsl bash` script files; raw od dumps in the PR for
//! #800). Anything not in the tables returns `None` and the real PATH
//! binary answers exactly as before — the arm can only remove spawns, never
//! change their output.
//!
//! Windows-only by construction: on unix the real ncurses tput reads the
//! SYSTEM terminfo database (including user/custom entries), which is
//! authoritative over any static table, and the unix spawn floor is ~1ms
//! (research/unix-spawn-cost-20261012.md) — there is no pathology to buy
//! down and nothing to be gained by risking divergence.

use std::borrow::Cow;

/// What an admitted invocation must do, byte-exact against GNU.
#[cfg(windows)]
pub(crate) enum TputOutcome {
    /// The exact bytes ncurses tput writes to stdout; exit status 0.
    Bytes(Cow<'static, str>),
    /// The capability is absent for this terminal (ncurses: no output,
    /// exit status 1 — verified: `TERM=vt100 tput setaf 1`, `TERM=linux
    /// tput smcup`).
    Absent,
}

/// One terminal's no-parameter capability strings, as verified from the
/// GNU oracle. `None` = capability absent for that terminal (silent rc 1).
#[cfg(windows)]
struct TermCaps {
    bold: &'static str,
    sgr0: &'static str,
    clear: &'static str,
    civis: Option<&'static str>,
    cnorm: Option<&'static str>,
    el: &'static str,
    ed: &'static str,
    smcup: Option<&'static str>,
    rmcup: Option<&'static str>,
}

/// The setaf/setab parameter families, per the oracle:
/// - `Xterm8` (8-color entries xterm/screen/linux): any admitted N answers
///   `\e[3{N}m` / `\e[4{N}m` — the termcap AF/AB fallback `\E[3%p1%dm`
///   formats N verbatim (verified 0, 1, 7, 8, 9, 10, 15, 16, 255 on
///   xterm/screen/linux: setaf 10 = `\e[310m`, setab 255 = `\e[4255m`).
/// - `Colors256` (xterm-256color/screen-256color/tmux-256color): 0-7 →
///   `\e[3{N}m`/`\e[4{N}m`; 8-15 → `\e[9{N-8}m`/`\e[10{N-8}m`; 16+ →
///   `\e[38;5;{N}m`/`\e[48;5;{N}m` (verified 8, 9, 15, 16, 255, 256,
///   1000 for setaf and 8, 9, 16, 255 for setab).
#[cfg(windows)]
enum Colors {
    Xterm8,
    Colors256,
}

#[cfg(windows)]
const XTERM: TermCaps = TermCaps {
    bold: "\x1b[1m",
    sgr0: "\x1b(B\x1b[m",
    clear: "\x1b[H\x1b[2J\x1b[3J",
    civis: Some("\x1b[?25l"),
    cnorm: Some("\x1b[?12l\x1b[?25h"),
    el: "\x1b[K",
    ed: "\x1b[J",
    smcup: Some("\x1b[?1049h\x1b[22;0;0t"),
    rmcup: Some("\x1b[?1049l\x1b[23;0;0t"),
};

#[cfg(windows)]
const SCREEN: TermCaps = TermCaps {
    bold: "\x1b[1m",
    sgr0: "\x1b[m\x0f",
    clear: "\x1b[H\x1b[J",
    civis: Some("\x1b[?25l"),
    cnorm: Some("\x1b[34h\x1b[?25h"),
    el: "\x1b[K",
    ed: "\x1b[J",
    smcup: Some("\x1b[?1049h"),
    rmcup: Some("\x1b[?1049l"),
};

#[cfg(windows)]
const TMUX: TermCaps = TermCaps {
    bold: "\x1b[1m",
    sgr0: "\x1b[m\x0f",
    // tmux-256color's clear carries E3 (verified: `\e[H\e[J\e[3J`) where
    // plain screen's does not — the one string that differs from SCREEN.
    clear: "\x1b[H\x1b[J\x1b[3J",
    civis: Some("\x1b[?25l"),
    cnorm: Some("\x1b[34h\x1b[?25h"),
    el: "\x1b[K",
    ed: "\x1b[J",
    smcup: Some("\x1b[?1049h"),
    rmcup: Some("\x1b[?1049l"),
};

#[cfg(windows)]
const LINUX: TermCaps = TermCaps {
    bold: "\x1b[1m",
    sgr0: "\x1b[m\x0f",
    clear: "\x1b[H\x1b[J\x1b[3J",
    civis: Some("\x1b[?25l\x1b[?1c"),
    cnorm: Some("\x1b[?25h\x1b[?0c"),
    el: "\x1b[K",
    ed: "\x1b[J",
    smcup: None,
    rmcup: None,
};

#[cfg(windows)]
const VT100: TermCaps = TermCaps {
    bold: "\x1b[1m",
    sgr0: "\x1b[m\x0f",
    clear: "\x1b[H\x1b[J",
    civis: None,
    cnorm: None,
    el: "\x1b[K",
    ed: "\x1b[J",
    smcup: None,
    rmcup: None,
};

#[cfg(windows)]
fn term_caps(term: &str) -> Option<(&'static TermCaps, Option<Colors>)> {
    let caps = match term {
        "xterm" | "xterm-256color" => &XTERM,
        "screen" | "screen-256color" => &SCREEN,
        "tmux-256color" => &TMUX,
        "linux" => &LINUX,
        "vt100" => &VT100,
        // An unlisted TERM (dumb, rxvt, st, custom entries, wrong case…)
        // stays with the real binary, which reads the real terminfo.
        _ => return None,
    };
    // The 256-color entries share their terminal's base strings but carry
    // the setaf/setab parameter family of the -256color terminfo entry.
    let colors = match term {
        "xterm-256color" | "screen-256color" | "tmux-256color" => Some(Colors::Colors256),
        // Every other admitted entry is an 8-color terminal, except
        // vt100 (no color capabilities at all — silent rc 1).
        "vt100" => None,
        _ => Some(Colors::Xterm8),
    };
    Some((caps, colors))
}

/// Resolve one `tput` invocation against the admitted whitelist.
///
/// `term` is the CHILD-VISIBLE TERM (exported shell state or tempenv
/// override — the same value a spawned tput would read; the caller owns
/// that resolution). `args` are the already-expanded argv words after the
/// command name. `None` = not admitted: the caller falls through to the
/// real PATH binary and nothing about today's behavior changes.
#[cfg(windows)]
pub(crate) fn resolve_admitted_tput(term: &str, args: &[String]) -> Option<TputOutcome> {
    // GNU tput answers exactly one capname operand; option forms (`-T`,
    // `-S`, `--`), missing operands (usage, rc 2) keep the real binary's
    // own handling. Per-capability arity is enforced in each arm below —
    // extra operands are GNU's rc-4 "unknown terminfo capability" surface
    // (`tput bold extra`), also the real binary's to answer.
    if args.is_empty() {
        return None;
    }
    let cap = &args[0];
    // A leading `-` operand is GNU's option/error surface (`tput setaf -1`
    // errors with "invalid option"), never one of the admitted shapes —
    // fall through.
    if cap.starts_with('-') {
        return None;
    }
    let (caps, colors) = term_caps(term)?;
    let borrowed = |bytes: &'static str| TputOutcome::Bytes(Cow::Borrowed(bytes));
    let optional = |slot: Option<&'static str>| slot.map(borrowed).unwrap_or(TputOutcome::Absent);
    let outcome = match cap.as_str() {
        "bold" if args.len() == 1 => Some(borrowed(caps.bold)),
        "sgr0" if args.len() == 1 => Some(borrowed(caps.sgr0)),
        "clear" if args.len() == 1 => Some(borrowed(caps.clear)),
        "el" if args.len() == 1 => Some(borrowed(caps.el)),
        "ed" if args.len() == 1 => Some(borrowed(caps.ed)),
        "civis" if args.len() == 1 => Some(optional(caps.civis)),
        "cnorm" if args.len() == 1 => Some(optional(caps.cnorm)),
        "smcup" if args.len() == 1 => Some(optional(caps.smcup)),
        "rmcup" if args.len() == 1 => Some(optional(caps.rmcup)),
        "setaf" | "setab" if args.len() == 2 => {
            // `tput setaf` with no operand prints the raw `%p1` string
            // (rc 0) and non-numeric operands error with rc 4 — both
            // stay with the real binary.
            let n = admitted_color_number(&args[1])?;
            let Some(colors) = colors else {
                return Some(TputOutcome::Absent);
            };
            let bytes = match (cap.as_str(), colors, n) {
                // 8-color entries format N verbatim into the termcap
                // fallback (`\e[3%p1%dm`): setaf 10 = `\e[310m`.
                ("setaf", Colors::Xterm8, n) => format!("\x1b[3{n}m"),
                ("setab", Colors::Xterm8, n) => format!("\x1b[4{n}m"),
                // 256-color entries: 0-7 base colors, 8-15 the bright
                // AIX pairs (`\e[9Nm`, NO `;5;` palette segment — verified
                // setaf 9 = `\e[91m`), 16+ the 256-color palette.
                ("setaf", Colors::Colors256, 0..=7) => format!("\x1b[3{n}m"),
                ("setab", Colors::Colors256, 0..=7) => format!("\x1b[4{n}m"),
                ("setaf", Colors::Colors256, 8..=15) => format!("\x1b[{}m", 90 + n - 8),
                ("setab", Colors::Colors256, 8..=15) => format!("\x1b[{}m", 100 + n - 8),
                ("setaf", Colors::Colors256, n) => format!("\x1b[38;5;{n}m"),
                ("setab", Colors::Colors256, n) => format!("\x1b[48;5;{n}m"),
                _ => return None,
            };
            Some(TputOutcome::Bytes(Cow::Owned(bytes)))
        }
        "cup" => {
            // `cup Y X` → `\e[{Y+1};{X+1}H` (tparm %i, 1-based; verified
            // 0 0 → 1;1, 5 10 → 6;11, 99 123 → 100;124). GNU's missing-
            // second-arg form (`cup 3` → `\e[4;1H`) and negative operands
            // (getopt "invalid option") stay with the real binary.
            if args.len() != 3 {
                return None;
            }
            let (Some(row), Some(column)) = (digits_only_i32(&args[1]), digits_only_i32(&args[2]))
            else {
                return None;
            };
            // Admission bound keeps the +1 arithmetic inside the verified
            // shape (i32::MAX would overflow). 0 is admitted — verified
            // `cup 0 0` → `\e[1;1H`; the canonical-decimal gate rules out
            // negatives and strtol base-0 spellings, whose getopt/error
            // surface stays with the real binary.
            if row > 255 || column > 255 {
                return None;
            }
            Some(TputOutcome::Bytes(Cow::Owned(format!(
                "\x1b[{};{}H",
                row + 1,
                column + 1
            ))))
        }
        // Everything else — init, reset, lines, cols, unknown names —
        // keeps the real binary's answer (rc 4 / rc 2 / runtime size
        // queries are terminal-state dependent and NOT admitted).
        _ => return None,
    };
    outcome
}

/// The setaf/setab numeric operand. GNU's tparm parameter parsing is
/// strtol base-0: `010` is OCTAL 8 (`\e[38m`), `+010` octal with a sign,
/// `009`/`08` truncate at the first illegal octal digit (`\e[30m`), and
/// `000000000010` is octal 10 — every one of these byte-diverges from a
/// decimal read (CR #884 B1). Replicating strtol inside the arm is exactly
/// the re-implementation shape #117 forbids, so the arm NARROWS instead:
/// only plain canonical decimal (`0`, `7`, `255`) is admitted; a leading
/// `0` (beyond the lone zero), a `+`/`-` sign, or any non-decimal text
/// falls through to the real binary, which answers with its own
/// strtol-faithful value.
#[cfg(windows)]
fn admitted_color_number(arg: &str) -> Option<i32> {
    if !canonical_decimal(arg) {
        return None;
    }
    let n: i32 = arg.parse().ok()?;
    (0..=255).contains(&n).then_some(n)
}

/// Canonical decimal only: optional nothing — no sign, and a leading zero
/// only for the literal `"0"` itself. `strtol`-shaped inputs (`010`,
/// `+7`, `007`, `08`, `000000000010`) are rejected so the real binary's
/// base-0 semantics keep answering.
#[cfg(windows)]
fn canonical_decimal(arg: &str) -> bool {
    !arg.is_empty()
        && arg.bytes().all(|byte| byte.is_ascii_digit())
        && (arg == "0" || !arg.starts_with('0'))
}

#[cfg(windows)]
fn digits_only_i32(arg: &str) -> Option<i32> {
    if !canonical_decimal(arg) {
        return None;
    }
    arg.parse().ok()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    fn bytes_of(term: &str, args: &[&str]) -> Option<String> {
        match resolve_admitted_tput(term, &words(args))? {
            TputOutcome::Bytes(bytes) => Some(bytes.into_owned()),
            TputOutcome::Absent => None,
        }
    }

    fn absent(term: &str, args: &[&str]) -> bool {
        matches!(
            resolve_admitted_tput(term, &words(args)),
            Some(TputOutcome::Absent)
        )
    }

    #[test]
    fn oracle_byte_rows() {
        assert_eq!(bytes_of("xterm", &["bold"]).as_deref(), Some("\x1b[1m"));
        assert_eq!(
            bytes_of("xterm", &["sgr0"]).as_deref(),
            Some("\x1b(B\x1b[m")
        );
        assert_eq!(bytes_of("screen", &["sgr0"]).as_deref(), Some("\x1b[m\x0f"));
        assert_eq!(
            bytes_of("tmux-256color", &["clear"]).as_deref(),
            Some("\x1b[H\x1b[J\x1b[3J")
        );
        assert_eq!(
            bytes_of("xterm", &["setaf", "10"]).as_deref(),
            Some("\x1b[310m")
        );
        assert_eq!(
            bytes_of("xterm-256color", &["setaf", "9"]).as_deref(),
            Some("\x1b[91m")
        );
        assert_eq!(
            bytes_of("xterm-256color", &["setaf", "16"]).as_deref(),
            Some("\x1b[38;5;16m")
        );
        assert_eq!(
            bytes_of("xterm", &["cup", "0", "0"]).as_deref(),
            Some("\x1b[1;1H")
        );
    }

    #[test]
    fn absent_capability_rows() {
        assert!(absent("vt100", &["setaf", "1"]));
        assert!(absent("vt100", &["civis"]));
        assert!(absent("linux", &["smcup"]));
    }

    #[test]
    fn non_admitted_shapes_return_none() {
        // Shapes where at least one vendor answers rc 0 — they must never
        // be admitted (the integration layer cannot pin them
        // host-agnostically; see tests/issue800_tput_inline_arm_regressions).
        for args in [
            &["bold", "extra"][..],
            &["setaf"][..],
            &["setaf", "abc"][..],
            &["setaf", "300"][..],
            &["setaf", "-1"][..],
            &["setaf", "4294967296"][..],
            // CR #884 B1: strtol base-0 operand spellings — octal
            // (`010`, `017`, `000000000010`), signed (`+010`, `+7`), and
            // illegal-octal truncations (`009`, `08`) — all fall through;
            // a decimal read would byte-diverge from GNU.
            &["setaf", "010"][..],
            &["setaf", "017"][..],
            &["setaf", "009"][..],
            &["setaf", "08"][..],
            &["setaf", "+010"][..],
            &["setaf", "+7"][..],
            &["setaf", "000000000010"][..],
            &["setab", "010"][..],
            &["cup", "010", "010"][..],
            &["cup", "008", "010"][..],
            &["cup", "3"][..],
            &["cup", "-1", "5"][..],
            &["cup", "5", "10", "9"][..],
            &["-T", "xterm", "bold"][..],
            &["bold", "-S"][..],
            &["init"][..],
            &["reset"][..],
            &["lines"][..],
            &["cols"][..],
            &["zzz"][..],
            &[""][..],
        ] {
            assert!(
                resolve_admitted_tput("xterm", &words(args)).is_none(),
                "shape {args:?} must not be admitted"
            );
        }
        // Arity guards on the no-parameter caps.
        assert!(resolve_admitted_tput("xterm", &words(&["bold", "x"])).is_none());
        assert!(resolve_admitted_tput("xterm", &words(&[])).is_none());
        // Unlisted / case-different TERM values never admit.
        for term in ["dumb", "Xterm", "xterm-8color", "putty-256color"] {
            assert!(resolve_admitted_tput(term, &words(&["bold"])).is_none());
        }
    }
}
