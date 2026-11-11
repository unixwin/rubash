//! rubash#1141: production timing for materialized input streams.
//!
//! GNU connects pipeline elements and process substitutions with real
//! pipes (execute_cmd.c:2620 execute_pipeline; subst.c:6362
//! process_substitute), so a consumer's `read`/`select` wait races the
//! producer's writes: GNU read.def arms the timeout when the read starts
//! (read.def:506-519) and, when it fires, assigns the input that had
//! arrived by the deadline and returns 128+SIGALRM (read.def:528-554).
//! Rubash materializes compound producers and substitution bodies into a
//! captured buffer BEFORE the consumer starts, so the arrival schedule is
//! lost and an armed deadline can never fire (the consumer sees the whole
//! buffer instantly, however long the producer took).
//!
//! This module restores the schedule without moving production off the
//! executor thread (the executor state is Rc<RefCell>, non-Send, and
//! Windows has no fork): while a producer stage / substitution body runs,
//! the capture sinks record each append as an arrival event
//! `(cumulative length, elapsed since production start)`. The finished
//! [`TimedInput`] is attached to the consumer's input (a pipeline stage's
//! FUNCTION_STDIN mirror or an fd-table text endpoint), and the record
//! readers consult it when a deadline is armed.
//!
//! Verdict math (all durations relative to production start `t0`, with
//! `D` = total production duration): the consumer's commands run in wall
//! time AFTER materialization, so a consumer action at wall instant `W`
//! sits at virtual (GNU) time `g(W) = (W - t0) - D + skipped_wait`, where
//! `skipped_wait` accumulates the virtual waits that earlier reads on
//! this same stream judged but did not actually sleep through. A record
//! whose last byte arrived at `A` is servable under an absolute deadline
//! `DL` iff `A <= (DL - t0) - D + skipped_wait`; on timeout the reader
//! serves only the prefix that had arrived by that virtual deadline
//! (read.def:539-562 assigns exactly that partial input) and
//! `skipped_wait` advances by the full timeout, keeping the virtual
//! clock honest for the next read.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Arrival schedule of one materialized input stream, shared between the
/// endpoint/ mirror that carries the bytes and every reader of it.
#[derive(Debug)]
pub(crate) struct TimedInput {
    /// Wall instant the producer started (stage execution / substitution
    /// body start).
    started: Instant,
    /// Total production duration; also the arrival time of EOF.
    duration: Duration,
    /// `(cumulative length, elapsed)` per recorded append, sorted by
    /// cumulative length; lengths are in the consumer's units (transport
    /// text bytes for FUNCTION_STDIN, raw bytes for fd-table endpoints).
    arrivals: Vec<(usize, Duration)>,
    /// Virtual wait already accounted to earlier reads on this stream.
    skipped_wait: Duration,
    /// Length of the FUNCTION_STDIN buffer this timing was recorded for;
    /// a staged buffer of a different length is a different input and
    /// must not be judged against this schedule. Unused (0) for fd-table
    /// endpoints, whose timing lives inside the endpoint itself.
    expected_len: usize,
}

pub(crate) type SharedTimedInput = Rc<RefCell<TimedInput>>;

/// Outcome of judging one record read against a [`TimedInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimedVerdict {
    /// Serve the record (or the EOF) normally.
    Serve,
    /// The record/EOF had not arrived by the virtual deadline. Serve
    /// only the input prefix ending at `available_end` (an absolute
    /// offset in the reader's units) as the timed-out partial record.
    Timeout { available_end: usize },
}

impl TimedInput {
    pub(crate) fn new(
        started: Instant,
        duration: Duration,
        arrivals: Vec<(usize, Duration)>,
        expected_len: usize,
    ) -> Self {
        Self {
            started,
            duration,
            arrivals,
            skipped_wait: Duration::ZERO,
            expected_len,
        }
    }

    pub(crate) fn shared(self) -> SharedTimedInput {
        Rc::new(RefCell::new(self))
    }

    /// The same schedule re-based for a reader whose buffer starts
    /// `base` units into the stream (a function call carving the
    /// parent's remaining FUNCTION_STDIN): arrival segments shift down
    /// by `base`, segments fully consumed by the carve drop out, and the
    /// virtual-wait accounting carries over. The clone's accounting
    /// diverges from the parent's from here on — waits consumed inside
    /// the carve are not credited back to the parent's later reads.
    pub(crate) fn shifted(&self, base: usize) -> Option<TimedInput> {
        if base > self.expected_len {
            return None;
        }
        let arrivals = self
            .arrivals
            .iter()
            .filter(|&&(cum, _)| cum > base)
            .map(|&(cum, elapsed)| (cum - base, elapsed))
            .collect();
        Some(TimedInput {
            started: self.started,
            duration: self.duration,
            arrivals,
            skipped_wait: self.skipped_wait,
            expected_len: self.expected_len - base,
        })
    }

    /// True when `len` is the buffer length this schedule belongs to.
    pub(crate) fn matches_len(&self, len: usize) -> bool {
        self.expected_len == len
    }

    /// The recorded arrival segments, in cumulative-length order.
    pub(crate) fn arrivals(&self) -> &[(usize, Duration)] {
        &self.arrivals
    }

    /// The same schedule re-expressed for a rewritten buffer
    /// (rubash#3293): the caller maps the arrival segments into the
    /// new buffer's coordinates and supplies its total length, while
    /// the production identity (start, duration) and the virtual-wait
    /// accounting carry over. A byte-served record rewrites the
    /// FUNCTION_STDIN mirror (the tail is re-encoded), which changes
    /// the buffer length the schedule was recorded for; without the
    /// rebase every later read on the stream fails `matches_len` and
    /// the timed judge stays silently off for the rest of the stream.
    pub(crate) fn rebased(
        &self,
        arrivals: Vec<(usize, Duration)>,
        expected_len: usize,
    ) -> TimedInput {
        TimedInput {
            started: self.started,
            duration: self.duration,
            arrivals,
            skipped_wait: self.skipped_wait,
            expected_len,
        }
    }

    /// Arrival time of the byte prefix ending at `end`: the events, in
    /// cumulative-length order, are the buffer's append segments, and
    /// the prefix exists once EVERY segment covering it has landed —
    /// the latest segment arrival up to the one that crosses `end`.
    /// EOF and anything past the last append arrive at the production
    /// end.
    fn arrival_of(&self, end: usize) -> Duration {
        let mut latest = Duration::ZERO;
        for &(cum, elapsed) in &self.arrivals {
            latest = latest.max(elapsed);
            if cum >= end {
                return latest;
            }
        }
        self.duration.max(latest)
    }

    /// Longest CONTIGUOUS prefix that had fully arrived by virtual time
    /// `budget`: segments count in buffer order, so the walk stops at
    /// the first segment that had not landed by `budget` (later segments
    /// cannot extend a prefix with a hole in it).
    fn available_end(&self, budget: Duration) -> usize {
        let mut end = 0;
        for &(cum, elapsed) in &self.arrivals {
            if elapsed > budget {
                break;
            }
            end = cum;
        }
        end
    }

    /// Judge one record read. `record_end` is the absolute end offset of
    /// the record the reader would serve from the full buffer, or `None`
    /// when the reader hit EOF. `current_offset` is the reader's stream
    /// position (the partial record on timeout is the arrived prefix
    /// beyond it). `deadline` is the reader's armed absolute deadline, if
    /// any; without one the read always serves, but its virtual wait
    /// still accrues so a later deadline on this stream keeps a true
    /// clock.
    pub(crate) fn judge(
        &mut self,
        record_end: Option<usize>,
        current_offset: usize,
        deadline: Option<Instant>,
    ) -> TimedVerdict {
        let arrival = match record_end {
            Some(end) => self.arrival_of(end),
            None => self.duration,
        };
        let now = Instant::now();
        // Virtual (GNU) time of this read's start: wall time since the
        // producer started, minus the materialization skew (production
        // itself), plus the virtual waits earlier reads accounted.
        let g = now
            .saturating_duration_since(self.started)
            .saturating_sub(self.duration)
            + self.skipped_wait;
        if let Some(deadline) = deadline {
            // The deadline in virtual terms: (DL - t0) - D + skipped.
            let budget_end = deadline
                .saturating_duration_since(self.started)
                .saturating_sub(self.duration)
                + self.skipped_wait;
            if arrival > budget_end {
                // GNU waited the whole timeout: advance the virtual
                // clock by it (budget_end - g == the armed timeout, up
                // to the microseconds between arming and judging).
                self.skipped_wait += budget_end.saturating_sub(g);
                return TimedVerdict::Timeout {
                    available_end: self.available_end(budget_end).max(current_offset),
                };
            }
        }
        // Served: the read would have waited from g until the record
        // completed (never negative — a record completed in the past
        // waits zero).
        self.skipped_wait += arrival.saturating_sub(g);
        TimedVerdict::Serve
    }
}

/// Per-append arrival events recorded while one producer runs, split by
/// capture sink. Compound stages and substitution bodies assemble their
/// output as `field capture ++ thread capture` (in that order), so the
/// thread sink's lengths are offset by the field sink's totals when the
/// recording is finished. Each event stores both the raw byte length and
/// the transport-text length of the sink buffer: pipeline consumers read
/// transport text (FUNCTION_STDIN), fd-table consumers read raw bytes.
#[derive(Debug, Default)]
pub(crate) struct CaptureTiming {
    started: Option<Instant>,
    field: Vec<(usize, usize, Duration)>,
    thread: Vec<(usize, usize, Duration)>,
    /// `(raw, text)` totals of the prefix (field) sink, snapshotted at
    /// the assembly merge point; falls back to the field sink's final
    /// totals when no merge point ran.
    prefix: Option<(usize, usize)>,
}

impl CaptureTiming {
    pub(crate) fn new() -> Self {
        Self {
            started: Some(Instant::now()),
            ..Self::default()
        }
    }

    /// Record one append to the field capture: `appended` is the byte
    /// slice just written, `raw_len` the sink buffer's new total. The
    /// text total accrues the append's isolated transport-text width
    /// (exact unless an append splits a multi-byte character across the
    /// boundary, a ±few-bytes approximation confined to that append).
    pub(crate) fn note_field(&mut self, appended: &[u8], raw_len: usize) {
        if let Some(started) = self.started {
            let elapsed = started.elapsed();
            let text_len = self.field.last().map(|&(_, t, _)| t).unwrap_or(0)
                + crate::executor::substitution_metadata::bytes_to_shell_text(appended).len();
            if self.field.last().map(|&(r, t, _)| (r, t)) != Some((raw_len, text_len)) {
                self.field.push((raw_len, text_len, elapsed));
            }
        }
    }

    /// Record one append to the thread-local capture (see note_field).
    pub(crate) fn note_thread(&mut self, appended: &[u8], raw_len: usize) {
        if let Some(started) = self.started {
            let elapsed = started.elapsed();
            let text_len = self.thread.last().map(|&(_, t, _)| t).unwrap_or(0)
                + crate::executor::substitution_metadata::bytes_to_shell_text(appended).len();
            if self.thread.last().map(|&(r, t, _)| (r, t)) != Some((raw_len, text_len)) {
                self.thread.push((raw_len, text_len, elapsed));
            }
        }
    }

    /// Snapshot the prefix sink's totals at the field++thread assembly
    /// merge point from the assembled field bytes.
    pub(crate) fn set_prefix(&mut self, field_bytes: &[u8]) {
        self.prefix = Some((
            field_bytes.len(),
            crate::executor::substitution_metadata::bytes_to_shell_text(field_bytes).len(),
        ));
    }

    /// Build the consumer-side schedule. `total` is the assembled
    /// output's length in the requested unit; a closing event at the
    /// production end covers any bytes that bypassed the hooked sinks
    /// (their true arrival is no later than the production end).
    pub(crate) fn finish(
        self,
        total: usize,
        text_units: bool,
        expected_len: usize,
    ) -> Option<TimedInput> {
        let started = self.started?;
        let duration = started.elapsed();
        let pick = |raw: usize, text: usize| if text_units { text } else { raw };
        let (prefix_raw, prefix_text) = self
            .prefix
            .unwrap_or_else(|| self.field.last().map(|&(r, t, _)| (r, t)).unwrap_or((0, 0)));
        let prefix_total = pick(prefix_raw, prefix_text);
        let mut arrivals: Vec<(usize, Duration)> = Vec::new();
        for &(raw, text, elapsed) in &self.field {
            arrivals.push((pick(raw, text), elapsed));
        }
        for &(raw, text, elapsed) in &self.thread {
            arrivals.push((prefix_total + pick(raw, text), elapsed));
        }
        arrivals.sort_by_key(|&(cum, _)| cum);
        // For a given cumulative length keep the EARLIEST arrival: a
        // prefix exists once every append covering it has landed, and
        // appends to one sink are sequential, so the first event that
        // reaches a length is the honest one after merging.
        arrivals.dedup_by(|next, kept| {
            if next.0 == kept.0 {
                kept.1 = kept.1.min(next.1);
                true
            } else {
                false
            }
        });
        if arrivals.last().map(|&(cum, _)| cum).unwrap_or(0) < total {
            arrivals.push((total, duration));
        }
        Some(TimedInput::new(started, duration, arrivals, expected_len))
    }
}

pub(crate) type SharedCaptureTiming = Rc<RefCell<CaptureTiming>>;
