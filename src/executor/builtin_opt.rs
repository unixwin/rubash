//! The single getopt for builtins (Stage-2 project ④, D2).
//!
//! A state machine porting `builtins/bashgetopt.c:61-192`
//! (`internal_getopt`) — behavior ported, not code translated —
//! over the spec string the declaration table carries for the
//! builtin. One instance per call: GNU's global getopt state
//! (`reset_internal_getopt`) is dissolved by ownership, so nested
//! builtin calls each hold their own machine and there is no
//! save/restore protocol. The `getopts` builtin's `OPTIND` is user
//! visible state and unrelated to this machine.
//!
//! Spec syntax (same shape as GNU's): plain letters, `':'`
//! (required argument), `';'` (optional argument), `'#'` (numeric
//! argument, `valid_number` grammar), a leading `'+'` (plus mode:
//! `+x` words are option words too). A leading `':'` in a spec
//! (fc's `":e:lnrs"`) is inert — `bashgetopt.c` rejects the option
//! character `':'` before consulting the spec.
//!
//! The scan rules, from `bashgetopt.c`:
//!
//! * A word is an option word when it starts with `-` (or with
//!   `+` in plus mode) and has a second character (`ISOPT` /
//!   `NOTOPT`, `bashgetopt.c:36-37`); a bare `-` / `+` is an
//!   operand. Scanning stops at the first non-option word — there
//!   is no permutation in `internal_getopt`, POSIX mode or not
//!   (the file has no `posixly_correct` branch; the `posix`
//!   constructor bit is the hook project ② owns, and in Phase 0 it
//!   changes nothing — pinned by test).
//! * `--help` exactly (`ISHELP`, `common.h:27`) in option position
//!   yields `Help`; `--` exactly ends scanning (consumed).
//! * An option character absent from the spec (or `':'` itself)
//!   reports `"{sign}{char}: invalid option"` through the `Diag`
//!   and ends scanning (`End`, status `EX_USAGE`) — the `.def`
//!   loops' `default:` arm then prints usage and returns
//!   `EX_USAGE`, so only the first bad character is ever reported.
//! * `':'` / `';'`: an attached rest-of-word is the argument; else
//!   the next word is taken when it exists (for `':'` always; for
//!   `';'` only when it does not look like an option); else `';'`
//!   yields the option with no argument, while `':'` reports
//!   `"{errstr}: option requires an argument"` and ends.
//! * `'#'`: an attached rest-of-word starting with a digit is the
//!   argument; an attached non-digit yields the option with no
//!   argument and NO error; else the next word is taken when it
//!   exists and is a valid number, else `"{errstr}: numeric
//!   argument required"` is reported and scanning ends.
//!
//! `Grammar` / `NoScan` rows are never scanned: the first `next()`
//! returns `End` with all words as operands (their `--help`
//! behaviour, if any, is the `CHECK_HELPOPT` first-word check the
//! dispatcher performs per the row's `help_entry` bit — Phase 1).

use super::builtin_diag::{valid_number, Diag, DiagKind, EX_USAGE};
use super::builtin_table::{BuiltinRow, OptSpec};

/// One step of the option scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptEvent<'a> {
    /// An option character with no argument.
    Opt(u8),
    /// An option character with its argument.
    OptArg(u8, &'a str),
    /// `--help` in option position (`GETOPT_HELP`).
    Help,
    /// Scanning is over: `--` was consumed, the first non-option
    /// word arrived, the words ran out, or an error was reported
    /// (status `EX_USAGE` then). `operands()` gives the rest.
    End,
}

/// How an option character takes its argument, from the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgKind {
    None,
    Required,
    Optional,
    Numeric,
}

/// The option scanner for one builtin call.
///
/// Two lifetimes, deliberately: `'a` is the words/row/sink region
/// (everything the events and the `Diag` borrow), `'d` is the
/// borrow of the `Diag` itself. A single lifetime ties the diag
/// borrow's duration to the sink's, so a test (or a future caller)
/// that reads the `Diag`'s status after the scan cannot borrow it —
/// the borrow would last until the sink's last use. Splitting `'d`
/// lets the machine be dropped (or simply go dead) and the `Diag`
/// be read afterwards.
pub struct BuiltinOpt<'a, 'd> {
    row: &'a BuiltinRow,
    words: &'a [String],
    diag: &'d mut Diag<'a>,
    /// The parsed spec: (option byte, argument kind) pairs, plus
    /// whether the spec opens with `'+'` (plus mode). Parsed into
    /// a fixed stack buffer at construction (`parse_spec_into`):
    /// the declare family's predicate probes construct this
    /// machine several times per call, and the earlier shared
    /// memoization put a global mutex lock + hash lookup on every
    /// construction — itself a measurable per-call tax on the
    /// fast builtins (E4, PR #2968). Only a spec too large for the
    /// buffer (none in the table; the longest carries 25 pairs)
    /// falls back to the memoized leaked parse (`parsed_spec`).
    spec: SpecPairs,
    plus: bool,
    /// Index of the word being scanned.
    index: usize,
    /// Byte offset within the current word (1 = just past the sign).
    sp: usize,
    /// Whether the scan has finished (an `End` was returned).
    done: bool,
    /// The option character that ended the scan in error
    /// (`list_optopt` in GNU's globals), or `None` if no error
    /// character has been seen. `set`'s scan switch reads it: an
    /// error on `'?'` itself maps to EXECUTION_SUCCESS
    /// (builtins/set.def:684-686).
    optopt: Option<u8>,
    /// The sign byte (`-` or `+`) of the word that produced the
    /// most recent event (`list_opttype` in GNU's globals).
    /// `set`'s `'i'` refusal builds its errstr from it
    /// (builtins/set.def:677-683).
    opttype: u8,
    /// Whether the row is scanned at all (`Getopt` rows only).
    scannable: bool,
    /// The POSIX-mode context bit (project ②'s hook; no effect in
    /// Phase 0 — `bashgetopt.c` has no POSIX branch).
    #[allow(dead_code)]
    posix: bool,
}

impl<'a, 'd> BuiltinOpt<'a, 'd> {
    pub fn new(
        row: &'a BuiltinRow,
        words: &'a [String],
        diag: &'d mut Diag<'a>,
        posix: bool,
    ) -> Self {
        let (spec, plus, scannable) = match row.opt_spec {
            OptSpec::Getopt(spec) => {
                let plus = spec.starts_with('+');
                let mut buffer = [(0, ArgKind::None); MAX_SPEC_PAIRS];
                let spec = match parse_spec_into(spec, &mut buffer) {
                    Some(len) => SpecPairs::Stack(buffer, len),
                    None => SpecPairs::Shared(parsed_spec(spec)),
                };
                (spec, plus, true)
            }
            OptSpec::Grammar | OptSpec::NoScan => (
                SpecPairs::Stack([(0, ArgKind::None); MAX_SPEC_PAIRS], 0),
                false,
                false,
            ),
        };
        BuiltinOpt {
            row,
            words,
            diag,
            spec,
            plus,
            index: 0,
            sp: 1,
            done: false,
            optopt: None,
            opttype: b'-',
            scannable,
            posix,
        }
    }

    /// The row this machine scans for.
    pub fn row(&self) -> &'a BuiltinRow {
        self.row
    }

    /// The words after the scan point (`loptend`): the operands.
    /// After `Help`, the `--help` word itself is included (GNU's
    /// `loptend = lcurrent` on the help path); after an error, the
    /// word holding the error is skipped.
    pub fn operands(&self) -> &'a [String] {
        &self.words[self.index.min(self.words.len())..]
    }

    /// The option character that ended the scan in error
    /// (`list_optopt`), or `None` when the scan ended cleanly.
    pub fn optopt(&self) -> Option<u8> {
        self.optopt
    }

    /// The sign byte of the word that produced the most recent
    /// event (`list_opttype`): `b'-'` or `b'+'`.
    pub fn opttype(&self) -> u8 {
        self.opttype
    }

    /// The next scan step. Illegal characters, missing arguments
    /// and bad numeric arguments are reported through the `Diag`
    /// (status `EX_USAGE`) and yield `End`.
    pub fn next(&mut self) -> OptEvent<'a> {
        if self.done || !self.scannable {
            self.done = true;
            return OptEvent::End;
        }
        if self.index >= self.words.len() {
            self.done = true;
            return OptEvent::End;
        }
        let word = &self.words[self.index];
        if self.sp == 1 {
            // At a word boundary: decide whether this word is an
            // option word at all.
            if !is_option_word(word.as_bytes(), self.plus) {
                self.done = true;
                return OptEvent::End;
            }
            if word == "--help" {
                // ISHELP: the help word itself stays in operands().
                self.done = true;
                return OptEvent::Help;
            }
            if word == "--" {
                self.index += 1;
                self.sp = 1;
                self.done = true;
                return OptEvent::End;
            }
        }
        let bytes = word.as_bytes();
        let sign = bytes[0];
        self.opttype = sign;
        let c = bytes[self.sp];
        // An option character of ':' is always invalid, whatever
        // the spec says (bashgetopt.c:99).
        let kind = if c == b':' {
            None
        } else {
            self.spec
                .as_slice()
                .iter()
                .find(|(b, _)| *b == c)
                .map(|(_, k)| *k)
        };
        let Some(kind) = kind else {
            // Invalid: errstr is the sign + the one character.
            let errstr = format!("{}{}", sign as char, diagnostic_byte(c));
            self.optopt = Some(c);
            self.diag.error(DiagKind::InvalidOpt(errstr));
            debug_assert_eq!(self.diag.status(), EX_USAGE);
            self.advance_past_bad_char();
            self.done = true;
            return OptEvent::End;
        };
        match kind {
            ArgKind::None => {
                self.advance_sp();
                OptEvent::Opt(c)
            }
            ArgKind::Required | ArgKind::Optional => {
                // Attached rest-of-word is the argument.
                if self.sp + 1 < bytes.len() {
                    let arg = &word[self.sp + 1..];
                    self.index += 1;
                    self.sp = 1;
                    return OptEvent::OptArg(c, arg);
                }
                // Else the next word, when it exists (for ':'
                // always; for ';' only when it is not an option).
                let take_next = match self.words.get(self.index + 1) {
                    Some(next) => {
                        kind == ArgKind::Required || !is_option_word(next.as_bytes(), self.plus)
                    }
                    None => false,
                };
                if take_next {
                    let arg = &self.words[self.index + 1];
                    self.index += 2;
                    self.sp = 1;
                    return OptEvent::OptArg(c, arg);
                }
                if kind == ArgKind::Optional {
                    self.index += 1;
                    self.sp = 1;
                    return OptEvent::Opt(c);
                }
                let errstr = format!("{}{}", sign as char, diagnostic_byte(c));
                self.optopt = Some(c);
                self.diag.error(DiagKind::NeedArg(errstr));
                debug_assert_eq!(self.diag.status(), EX_USAGE);
                self.sp = 1;
                self.done = true;
                OptEvent::End
            }
            ArgKind::Numeric => {
                if self.sp + 1 < bytes.len() {
                    // Attached rest: a leading digit takes it as
                    // the argument; a non-digit yields no argument
                    // and no error (bashgetopt.c:145-153).
                    if bytes[self.sp + 1].is_ascii_digit() {
                        let arg = &word[self.sp + 1..];
                        self.index += 1;
                        self.sp = 1;
                        return OptEvent::OptArg(c, arg);
                    }
                    self.index += 1;
                    self.sp = 1;
                    return OptEvent::Opt(c);
                }
                match self.words.get(self.index + 1) {
                    Some(next) if valid_number(next).is_some() => {
                        let arg = &self.words[self.index + 1];
                        self.index += 2;
                        self.sp = 1;
                        OptEvent::OptArg(c, arg)
                    }
                    _ => {
                        let errstr = format!("{}{}", sign as char, diagnostic_byte(c));
                        self.optopt = Some(c);
                        self.diag.error(DiagKind::NeedNumArg(errstr));
                        debug_assert_eq!(self.diag.status(), EX_USAGE);
                        self.sp = 1;
                        self.done = true;
                        OptEvent::End
                    }
                }
            }
        }
    }

    /// Advance one character; at the end of the word move to the
    /// next word.
    fn advance_sp(&mut self) {
        let word = &self.words[self.index];
        self.sp += 1;
        if self.sp >= word.len() {
            self.index += 1;
            self.sp = 1;
        }
    }

    /// Advance past a bad option character (`bashgetopt.c:99-105`).
    /// The operands then start after the error word (GNU's
    /// `loptend` after an error points one word further when the
    /// bad character ended the word — unobservable, since the
    /// builtin returns `EX_USAGE` without consulting it — so the
    /// machine takes the uniform "after the error word" reading).
    fn advance_past_bad_char(&mut self) {
        self.index += 1;
        self.sp = 1;
    }
}

/// Render one option byte for a diagnostic string. The byte comes
/// from user argv, so it can be a UTF-8 continuation byte (>= 0x80)
/// and must never be widened with `as char` — that Latin-1 widening
/// is exactly what `scripts/check-utf8-boundary-hygiene.sh` guards.
/// ASCII bytes render as themselves (so every covered diagnostic is
/// byte-identical to the `as char` form); a non-ASCII byte degrades
/// to U+FFFD, the codebase's standard lossy rendering for arbitrary
/// bytes in a `String` (cf. `command_substitution.rs`, `identity.rs`).
/// (GNU echoes the raw byte here — `enable -$'\xa0'` prints the
/// 0xA0 byte — so a non-ASCII invalid-option byte is a pre-existing
/// diagnostic corner either way, not something this helper newly
/// diverges on.)
fn diagnostic_byte(byte: u8) -> String {
    String::from_utf8_lossy(&[byte]).into_owned()
}

/// `ISOPT` / `NOTOPT` (`bashgetopt.c:36-37`): a word is an option
/// word when it starts with `-` (or `+` in plus mode) and has a
/// second character. A bare `-` / `+` is an operand.
pub(super) fn is_option_word(word: &[u8], plus: bool) -> bool {
    if word.len() < 2 {
        return false;
    }
    word[0] == b'-' || (plus && word[0] == b'+')
}

/// The length of the option region at the head of `words`: the
/// maximal prefix of option words. `internal_getopt` stops the
/// scan before the first non-option word (`bashgetopt.c:77-84`),
/// before `--` (consumed as the delimiter), and before `--help`
/// (GETOPT_HELP ends the scan with the word itself left in
/// `loptend`); the region ends at the same three boundaries, so
/// `words[..option_region_end(words, plus)]` is exactly the set of
/// words a GNU scan reads option letters from, and every word from
/// the returned index on is an operand (plus the `--` delimiter
/// itself when the region ended there).
///
/// Every declare-family scan that reads option letters off the raw
/// word list — instead of driving the machine above — must bound
/// itself to this prefix: a word like `-i` behind an operand is a
/// NAME in GNU, and a scan that roams past the first operand
/// misreads it as an option (rubash#3338). The family specs
/// (declare's `+acfgilnprtuxAFGI`, unset's `fnv`) are flag-only —
/// no option takes an argument — so no word inside the region
/// consumes a following word, and a pure prefix computation is
/// the whole rule.
pub(super) fn option_region_end(words: &[String], plus: bool) -> usize {
    let mut end = 0;
    for word in words {
        if word == "--" || word == "--help" {
            break;
        }
        if !is_option_word(word.as_bytes(), plus) {
            break;
        }
        end += 1;
    }
    end
}

/// Capacity of the stack parse buffer (`parse_spec_into`): the
/// longest spec in the builtin table (`complete`'s) carries 25
/// pairs, so 64 leaves wide headroom; the table-walk test in this
/// module pins that every row spec fits.
const MAX_SPEC_PAIRS: usize = 64;

/// A parsed spec: the (option byte, argument kind) pairs of one
/// row's spec string, held either in the machine's own stack
/// buffer (the universal case) or, for a spec too large for the
/// buffer, in the memoized leaked slice (`parsed_spec`).
enum SpecPairs {
    Stack([(u8, ArgKind); MAX_SPEC_PAIRS], usize),
    Shared(&'static [(u8, ArgKind)]),
}

impl SpecPairs {
    fn as_slice(&self) -> &[(u8, ArgKind)] {
        match self {
            SpecPairs::Stack(buffer, len) => &buffer[..*len],
            SpecPairs::Shared(slice) => slice,
        }
    }
}

/// Parse a spec string into (option byte, argument kind) pairs,
/// writing them into `out`; returns the pair count, or `None`
/// when the spec carries more pairs than the buffer holds. The
/// parse mirrors `parse_spec` step for step: a leading `'+'`
/// enables plus mode and is not an option; the `':'` / `';'` /
/// `'#'` after a letter sets its argument kind; a `':'` where a
/// letter is expected (fc's leading one) is skipped as inert.
fn parse_spec_into(spec: &str, out: &mut [(u8, ArgKind); MAX_SPEC_PAIRS]) -> Option<usize> {
    let bytes = spec.as_bytes();
    let mut n = 0;
    let mut i = 0;
    if bytes.first() == Some(&b'+') {
        i = 1;
    }
    while i < bytes.len() {
        let c = bytes[i];
        if c == b':' {
            // A ':' where an option letter is expected (only the
            // leading one, in practice) is not an option.
            i += 1;
            continue;
        }
        let kind = match bytes.get(i + 1) {
            Some(b':') => {
                i += 1;
                ArgKind::Required
            }
            Some(b';') => {
                i += 1;
                ArgKind::Optional
            }
            Some(b'#') => {
                i += 1;
                ArgKind::Numeric
            }
            _ => ArgKind::None,
        };
        if n >= MAX_SPEC_PAIRS {
            return None;
        }
        out[n] = (c, kind);
        n += 1;
        i += 1;
    }
    Some(n)
}

/// The oversized-spec fallback: `parse_spec` memoized per distinct
/// spec string — the parse is a pure function of the row's static
/// spec, so it is computed once and the result leaked into a
/// shared slice. Bounded by the number of distinct row specs (one
/// per builtin). Steady-state constructions never reach this (see
/// `parse_spec_into`); it exists so a hypothetical spec larger
/// than the stack buffer still parses exactly as before.
fn parsed_spec(spec: &'static str) -> &'static [(u8, ArgKind)] {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static [(u8, ArgKind)]>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap();
    if let Some(hit) = cache.get(spec) {
        return hit;
    }
    let parsed: &'static [(u8, ArgKind)] = Box::leak(parse_spec(spec).into_boxed_slice());
    cache.insert(spec, parsed);
    parsed
}

fn parse_spec(spec: &str) -> Vec<(u8, ArgKind)> {
    let bytes = spec.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    if bytes.first() == Some(&b'+') {
        i = 1;
    }
    while i < bytes.len() {
        let c = bytes[i];
        if c == b':' {
            // A ':' where an option letter is expected (only the
            // leading one, in practice) is not an option.
            i += 1;
            continue;
        }
        let kind = match bytes.get(i + 1) {
            Some(b':') => {
                i += 1;
                ArgKind::Required
            }
            Some(b';') => {
                i += 1;
                ArgKind::Optional
            }
            Some(b'#') => {
                i += 1;
                ArgKind::Numeric
            }
            _ => ArgKind::None,
        };
        out.push((c, kind));
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::builtin_diag::Prolog;
    use super::super::builtin_table::{
        builtin_table_row, BuiltinFlags, BuiltinRow, DocId, HandlerSlot,
    };
    // `Introspection` is `pub(crate)` in the registry but only
    // privately imported by the table module — import it here
    // directly (E0603 if taken through the table).
    use super::super::command_registry::Introspection;
    use super::*;

    /// The prolog the flushed diagnostics render under
    /// (`"t.sh: line 1: "`), for the sink assertions below.
    fn test_prolog() -> Prolog {
        Prolog {
            interactive: false,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: Some("t.sh".to_string()),
            line: Some("1".to_string()),
        }
    }

    fn diag_for<'a>(name: &str, sink: &'a mut Vec<u8>) -> Diag<'a> {
        Diag::new(builtin_table_row(name).unwrap(), sink)
    }

    fn words(ss: &[&str]) -> Vec<String> {
        ss.iter().map(|s| s.to_string()).collect()
    }

    fn drain<'a, 'd>(opt: &mut BuiltinOpt<'a, 'd>) -> Vec<OptEvent<'a>> {
        let mut events = Vec::new();
        loop {
            let ev = opt.next();
            let end = ev == OptEvent::End;
            events.push(ev);
            if end {
                break;
            }
        }
        events
    }

    #[test]
    fn scans_plain_and_clustered_options() {
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["-L", "-P"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::Opt(b'L'), OptEvent::Opt(b'P'), OptEvent::End]
        );
        assert!(opt.operands().is_empty());
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty());

        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["-LP", "dir"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::Opt(b'L'), OptEvent::Opt(b'P'), OptEvent::End]
        );
        assert_eq!(opt.operands(), ["dir"]);
    }

    fn cd_row() -> &'static BuiltinRow {
        builtin_table_row("cd").unwrap()
    }

    #[test]
    fn required_arg_attached_separate_and_missing() {
        // wait -p VAR / -pVAR / -p (missing).
        let mut sink = Vec::new();
        let mut diag = diag_for("wait", &mut sink);
        let w = words(&["-p", "VAR"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("wait").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'p', "VAR"), OptEvent::End]
        );
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty());

        let mut sink = Vec::new();
        let mut diag = diag_for("wait", &mut sink);
        let w = words(&["-pVAR"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("wait").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'p', "VAR"), OptEvent::End]
        );

        let mut sink = Vec::new();
        let mut diag = diag_for("wait", &mut sink);
        let w = words(&["-p"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("wait").unwrap(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        // Canonical order: the machine is dead before the Diag is
        // read, and the sink is read (moved) only after the Diag's
        // last use — the sink is borrowed by the Diag until then.
        drop(opt);
        assert_eq!(diag.status(), EX_USAGE);
        diag.flush_with(test_prolog);
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "t.sh: line 1: wait: -p: option requires an argument\n"
        );
    }

    #[test]
    fn required_arg_takes_an_option_looking_word() {
        // ':' takes the next word whatever it looks like.
        let mut sink = Vec::new();
        let mut diag = diag_for("wait", &mut sink);
        let w = words(&["-p", "-x"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("wait").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'p', "-x"), OptEvent::End]
        );
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty());
    }

    #[test]
    fn optional_arg_rules() {
        // set -o (optional arg): "-o" + non-option word takes it;
        // "-o" + option word takes nothing and keeps scanning.
        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-o", "vi"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'o', "vi"), OptEvent::End]
        );

        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-o", "-x"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::Opt(b'o'), OptEvent::Opt(b'x'), OptEvent::End]
        );
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty());

        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-o"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::Opt(b'o'), OptEvent::End]);
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty(), "';' with no arg is silent");
    }

    #[test]
    fn numeric_arg_rules() {
        // The `#` (numeric-argument) spec kind has NO row in the
        // table — ulimit's `n` is Optional `;`, not `#` (its
        // `-nX` takes "X" as the attached argument; that shape is
        // pinned by `optional_arg_rules`). So the `#` path is
        // pinned with a synthetic row carrying the `n#` spec.
        // Cases: attached digit, attached non-digit (silent, no
        // argument), separate valid number, separate invalid word,
        // separate negative number.
        let row = BuiltinRow {
            name: "synthetic-numeric",
            handler: HandlerSlot::Ulimit,
            flags: BuiltinFlags::DISABLEABLE,
            opt_spec: OptSpec::Getopt("n#"),
            doc: DocId::Topic("ulimit"),
            introspection: Introspection::Builtin,
            help_entry: true,
        };
        let mut sink = Vec::new();
        let mut diag = diag_for("ulimit", &mut sink);
        let w = words(&["-n1024"]);
        let mut opt = BuiltinOpt::new(&row, &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'n', "1024"), OptEvent::End]
        );

        let mut sink = Vec::new();
        let mut diag = diag_for("ulimit", &mut sink);
        let w = words(&["-nX"]);
        let mut opt = BuiltinOpt::new(&row, &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::Opt(b'n'), OptEvent::End]);
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty(), "attached non-digit is silent");

        let mut sink = Vec::new();
        let mut diag = diag_for("ulimit", &mut sink);
        let w = words(&["-n", "42"]);
        let mut opt = BuiltinOpt::new(&row, &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'n', "42"), OptEvent::End]
        );

        let mut sink = Vec::new();
        let mut diag = diag_for("ulimit", &mut sink);
        let w = words(&["-n", "xyz"]);
        let mut opt = BuiltinOpt::new(&row, &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        drop(opt);
        diag.flush_with(test_prolog);
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "t.sh: line 1: ulimit: -n: numeric argument required\n"
        );

        let mut sink = Vec::new();
        let mut diag = diag_for("ulimit", &mut sink);
        let w = words(&["-n", "-1"]);
        let mut opt = BuiltinOpt::new(&row, &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'n', "-1"), OptEvent::End]
        );
    }

    #[test]
    fn invalid_option_reports_once_and_ends() {
        // B6-F3: `cd -zx` reports only -z (the .def loop's default
        // arm returns EX_USAGE on the first '?').
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["-zx"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert!(opt.operands().is_empty());
        // Canonical order (see the wait missing-arg case above).
        drop(opt);
        assert_eq!(diag.status(), EX_USAGE);
        diag.flush_with(test_prolog);
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "t.sh: line 1: cd: -z: invalid option\n"
        );
    }

    #[test]
    fn plus_mode_scans_plus_words_and_keeps_the_sign() {
        // shopt has no plus mode (spec "psuoq" opens with no '+'):
        // "+z" is an operand, and the machine reports nothing —
        // the "invalid shell option name" comes from shopt's own
        // operand handling (verified byte-identical on the oracle:
        // `shopt +z` -> "shopt: +z: invalid shell option name",
        // rc 1, on both shells). declare +a scans as an option.
        let mut sink = Vec::new();
        let mut diag = diag_for("shopt", &mut sink);
        let w = words(&["+z"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("shopt").unwrap(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.operands(), ["+z"]);
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty(), "the machine reports nothing here");

        let mut sink = Vec::new();
        let mut diag = diag_for("declare", &mut sink);
        let w = words(&["+a", "-r"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("declare").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::Opt(b'a'), OptEvent::Opt(b'r'), OptEvent::End]
        );
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty());
    }

    #[test]
    fn plus_words_are_operands_without_plus_mode() {
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["+L"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.operands(), ["+L"]);
    }

    #[test]
    fn help_and_dashdash_positions() {
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["--help"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        // Help ends the scan; the drain's next step is End.
        assert_eq!(drain(&mut opt), [OptEvent::Help, OptEvent::End]);
        assert_eq!(opt.operands(), ["--help"]);

        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["-L", "--help"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::Opt(b'L'), OptEvent::Help, OptEvent::End]
        );

        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["--", "--help"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.operands(), ["--help"]);

        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["dir", "--help"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.operands(), ["dir", "--help"]);

        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w = words(&["-"]);
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.operands(), ["-"]);
    }

    #[test]
    fn issue3338_option_region_end_stops_like_internal_getopt() {
        // The region is the maximal option-word prefix: it ends
        // before the first operand, before `--`, and before
        // `--help` (bashgetopt.c:77-84) — the same boundaries the
        // machine above stops at, for scans that read letters off
        // the raw word list instead of driving the machine.
        assert_eq!(option_region_end(&words(&[]), true), 0);
        assert_eq!(option_region_end(&words(&["x", "-i"]), true), 0);
        assert_eq!(option_region_end(&words(&["-i", "x", "-p"]), true), 1);
        assert_eq!(option_region_end(&words(&["-i", "--", "-p"]), true), 1);
        assert_eq!(option_region_end(&words(&["-i", "--help"]), true), 1);
        assert_eq!(option_region_end(&words(&["--help", "-i"]), true), 0);
        assert_eq!(option_region_end(&words(&["-", "-i"]), true), 0);
        assert_eq!(option_region_end(&words(&["-p", "-i"]), true), 2);
        // Plus mode admits `+` words into the region; without it a
        // `+` word is the first operand.
        assert_eq!(option_region_end(&words(&["+x", "-i"]), true), 2);
        assert_eq!(option_region_end(&words(&["+x", "-i"]), false), 0);
        assert_eq!(option_region_end(&words(&["-f", "-v"]), false), 2);
    }

    #[test]
    fn no_options_rows_reject_any_option_word() {
        // times -z: no_options -> invalid + usage, EX_USAGE.
        let mut sink = Vec::new();
        let mut diag = diag_for("times", &mut sink);
        let w = words(&["-z"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("times").unwrap(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        drop(opt);
        diag.flush_with(test_prolog);
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "t.sh: line 1: times: -z: invalid option\n"
        );
    }

    #[test]
    fn grammar_and_noscan_rows_do_not_scan() {
        for name in [
            "test", "[", "kill", "let", "env", ":", "echo", "break", "exit",
        ] {
            let mut sink = Vec::new();
            let mut diag = diag_for(name, &mut sink);
            let w = words(&["-z", "--help", "x"]);
            let mut opt = BuiltinOpt::new(builtin_table_row(name).unwrap(), &w, &mut diag, false);
            assert_eq!(drain(&mut opt), [OptEvent::End], "row {name} scanned");
            assert_eq!(opt.operands(), ["-z", "--help", "x"]);
            drop(opt);
            diag.flush_with(test_prolog);
            assert!(sink.is_empty(), "row {name} reported");
        }
    }

    #[test]
    fn empty_words_end_immediately() {
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let w: Vec<String> = Vec::new();
        let mut opt = BuiltinOpt::new(cd_row(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert!(opt.operands().is_empty());
    }

    #[test]
    fn posix_bit_changes_nothing_in_phase_0() {
        // bashgetopt.c has no posixly_correct branch: the scan is
        // identical with the bit on or off.
        let row = cd_row();
        let w = words(&["-L", "dir"]);
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let mut off = BuiltinOpt::new(row, &w, &mut diag, false);
        let ev_off = drain(&mut off);
        let ops_off = off.operands().to_vec();
        let mut sink = Vec::new();
        let mut diag = diag_for("cd", &mut sink);
        let mut on = BuiltinOpt::new(row, &w, &mut diag, true);
        let ev_on = drain(&mut on);
        assert_eq!(ev_off, ev_on);
        assert_eq!(ops_off, on.operands());
    }

    #[test]
    fn optopt_records_the_error_character() {
        // set.def:684-686 reads list_optopt after the scan: an
        // error on '?' itself maps to EXECUTION_SUCCESS, any other
        // bad character to EX_USAGE.
        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-?"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.optopt(), Some(b'?'));

        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-z?"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(drain(&mut opt), [OptEvent::End]);
        assert_eq!(opt.optopt(), Some(b'z'), "the FIRST bad character is kept");

        // A clean scan leaves optopt empty.
        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-e", "-o", "vi"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [
                OptEvent::Opt(b'e'),
                OptEvent::OptArg(b'o', "vi"),
                OptEvent::End
            ]
        );
        assert_eq!(opt.optopt(), None);
    }

    #[test]
    fn opttype_tracks_the_event_words_sign() {
        // set.def:677-683 builds the `set -i` refusal's errstr from
        // list_opttype — the sign of the word carrying the 'i'.
        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["-ei"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(opt.next(), OptEvent::Opt(b'e'));
        assert_eq!(opt.next(), OptEvent::Opt(b'i'));
        assert_eq!(opt.opttype(), b'-');

        let mut sink = Vec::new();
        let mut diag = diag_for("set", &mut sink);
        let w = words(&["+i"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("set").unwrap(), &w, &mut diag, false);
        assert_eq!(opt.next(), OptEvent::Opt(b'i'));
        assert_eq!(opt.opttype(), b'+');
    }

    #[test]
    fn stack_parse_matches_vec_parse_for_every_table_spec() {
        // The stack parse (parse_spec_into) replaced the global
        // memoized parse on the construction path (E4, PR #2968):
        // pin that it agrees with the Vec parse (parse_spec) on
        // every row spec in the table, and that every row spec
        // fits the stack buffer — a spec that outgrew the buffer
        // would silently take the Shared fallback (correct, but
        // the hot path would pay the lock again unnoticed).
        use super::super::builtin_table::BUILTIN_TABLE;
        let mut checked = 0;
        for row in BUILTIN_TABLE {
            let OptSpec::Getopt(spec) = row.opt_spec else {
                continue;
            };
            let mut buffer = [(0, ArgKind::None); MAX_SPEC_PAIRS];
            let len = parse_spec_into(spec, &mut buffer)
                .unwrap_or_else(|| panic!("spec for {} exceeds the stack buffer", row.name));
            assert_eq!(
                &buffer[..len],
                parse_spec(spec).as_slice(),
                "stack parse disagrees with vec parse for {}",
                row.name
            );
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn fc_leading_colon_in_spec_is_inert() {
        let mut sink = Vec::new();
        let mut diag = diag_for("fc", &mut sink);
        let w = words(&["-e", "vi"]);
        let mut opt = BuiltinOpt::new(builtin_table_row("fc").unwrap(), &w, &mut diag, false);
        assert_eq!(
            drain(&mut opt),
            [OptEvent::OptArg(b'e', "vi"), OptEvent::End]
        );
        drop(opt);
        diag.flush_with(test_prolog);
        assert!(sink.is_empty());
    }
}
