//! Resumable brace-group scan cache (rubash#176 / #178).
//!
//! GNU reads a script once, token by token (`parse.y:3557 read_token`): the
//! reader state carries across every construct and no text is ever
//! re-scanned. Rubash's batch tokenizer instead re-tokenizes the whole
//! accumulated logical line after each appended physical line
//! (`tokenize_with_heredocs`), and inside every pass each still-open `{`
//! runs `skip_brace` / `brace_group_contains_heredoc_operator` from its own
//! offset to the end of the buffer. With `d` open brace groups across `n`
//! physical lines that sums to O(d·n²) — the near-cubic amplifier measured
//! in rubash#176 (nested depth) and #178 (repeated multi-line units).
//!
//! The two scans are deterministic left-to-right DFAs over the logical
//! line. Because a joined logical line only ever grows by appends (a
//! backslash-join pop, a comsub-heredoc rotation, or the `\x1c` EOF-marker
//! insert all *rewrite* it and clear the cache instead), a scan that
//! stopped at end-of-input can be resumed from its last position with its
//! captured state, and a scan that finished inside the already-verified
//! prefix never needs to run again. This module stores those continuations,
//! keyed by the byte offset of the opening `{`; the two scans keep
//! separate tables because they ask different questions about the same
//! brace.
//!
//! Soundness rules enforced by the producers (`skip.rs`, `scanner.rs`):
//! - A *closed* result is only recorded when the close decision did not
//!   consult bytes past the end of the current input (post-`}` peek and
//!   `brace_close_can_end_compact_group` report that).
//! - An *unclosed* (resume) result is only recorded when no lookahead in
//!   the scan ran to end-of-input undecided (the `esac)` case-pattern
//!   lookahead reports truncation and poisons the resume).
//! - Every non-append mutation of the logical line clears the cache.

/// Snapshot of `skip_brace`'s outer-loop state at the moment its scan
/// reached end-of-input without finding a close. Resuming the loop from
/// `pos` with these fields reproduces exactly what a fresh scan over the
/// longer input would do from the opening `{`.
#[derive(Clone)]
pub(crate) struct BraceScanResume {
    /// Byte offset where scanning stopped (end of input at record time).
    pub(crate) pos: usize,
    pub(crate) depth: usize,
    pub(crate) case_depth: usize,
    pub(crate) word: String,
    pub(crate) word_boundary: bool,
    pub(crate) current_word_boundary: bool,
    /// True when the scan stopped inside a `#' comment that had no
    /// terminator yet: the resume first consumes comment text through the
    /// next newline, like the in-scan comment branch does.
    pub(crate) pending_comment: bool,
    pub(crate) comment_start: bool,
    pub(crate) comment_at: Option<usize>,
    pub(crate) saw_top_level_whitespace: bool,
    pub(crate) ansi_single: bool,
    pub(crate) escaped: bool,
}

/// Outcome of `skip_brace` worth remembering for the `{` at some offset.
#[derive(Clone)]
pub(crate) enum BraceScanEntry {
    /// The group closed: byte offset just past the `}` plus the
    /// word-initial comment offset found at the group's top level.
    Closed {
        end: usize,
        comment_start: Option<usize>,
    },
    /// The scan ran to end-of-input unclosed; state to resume from.
    Unclosed(BraceScanResume),
}

/// Snapshot of `brace_group_contains_heredoc_operator`'s scan state.
#[derive(Clone)]
pub(crate) struct HeredocOpScanResume {
    /// Byte offset (relative to the scan start, which is the byte after
    /// the `{`) where scanning stopped.
    pub(crate) index: usize,
    pub(crate) depth: usize,
    pub(crate) single: bool,
    pub(crate) double: bool,
    pub(crate) escaped: bool,
}

/// Outcome of the heredoc-operator scan worth remembering.
#[derive(Clone)]
pub(crate) enum HeredocOpEntry {
    /// An unquoted `<<` sits inside the group (in the verified prefix, so
    /// stable under appends).
    Found,
    /// The group closed with no `<<` (stable).
    Absent,
    /// The scan ran to end-of-input without deciding.
    Resume(HeredocOpScanResume),
}

#[derive(Default)]
pub(crate) struct BraceScanCache {
    brace_entries: Vec<(usize, BraceScanEntry)>,
    operator_entries: Vec<(usize, HeredocOpEntry)>,
}

impl BraceScanCache {
    pub(crate) fn clear(&mut self) {
        self.brace_entries.clear();
        self.operator_entries.clear();
    }

    pub(crate) fn lookup_brace(&self, brace_offset: usize) -> Option<&BraceScanEntry> {
        self.brace_entries
            .iter()
            .rev()
            .find(|(offset, _)| *offset == brace_offset)
            .map(|(_, entry)| entry)
    }

    pub(crate) fn record_brace(&mut self, brace_offset: usize, entry: BraceScanEntry) {
        match self
            .brace_entries
            .iter_mut()
            .rev()
            .find(|(offset, _)| *offset == brace_offset)
        {
            Some(slot) => slot.1 = entry,
            None => self.brace_entries.push((brace_offset, entry)),
        }
    }

    pub(crate) fn lookup_operator(&self, brace_offset: usize) -> Option<&HeredocOpEntry> {
        self.operator_entries
            .iter()
            .rev()
            .find(|(offset, _)| *offset == brace_offset)
            .map(|(_, entry)| entry)
    }

    pub(crate) fn record_operator(&mut self, brace_offset: usize, entry: HeredocOpEntry) {
        match self
            .operator_entries
            .iter_mut()
            .rev()
            .find(|(offset, _)| *offset == brace_offset)
        {
            Some(slot) => slot.1 = entry,
            None => self.operator_entries.push((brace_offset, entry)),
        }
    }
}
