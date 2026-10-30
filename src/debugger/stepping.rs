//! The stepping arming predicates (debugger design section 6.1;
//! WP-4).
//!
//! Stepping is arming: a resume installs a one-shot rule, and the
//! consult evaluates the rule against each live seam. The rules are
//! pure functions of the installed [`StepArming`] and facts the
//! engine reads from its own dynamic state — the frame depth is
//! `shell_state.function_depth`, the same depth the trap path and
//! `FUNCNAME` are built from. This module keeps no stack of its
//! own: a shadow stack would drift from the engine's on exactly the
//! paths that matter (entry skips, force-returns, sourced files),
//! and the design's depth definition (section 6.2: the number of
//! function frames currently executing) is the engine's count by
//! construction.
//!
//! Two evaluation points exist because the design's rules name two
//! kinds of arrival:
//!
//! - [`arming_stops_at_seam`] — an ordinary command seam, carrying
//!   the depth of the frame the seam fires in.
//! - [`arming_stops_at_frame_return`] — the return site of a frame
//!   (DE-04), where a `Finish` arming is serviced in the caller.

use super::event::FrameReturn;
use super::session::StepArming;

/// Does `arming` stop at a command seam firing at `depth`?
///
/// - `StepInto` stops at the next seam, whatever frame it is in —
///   into a called function, into a sourced file, in place
///   (section 6.1's step-into row).
/// - `StepOver` stops at the next seam no deeper than the depth it
///   was armed from: a called function runs to completion (or to a
///   breakpoint inside it) without stopping, and a loop iteration's
///   next head seam — same depth — does stop.
/// - `Finish` does not stop at seams *inside* the frame being
///   finished; its stop is serviced at the frame's return site
///   ([`arming_stops_at_frame_return`]). The seam rule here is the
///   backstop for the paths where no return site reports (the
///   extdebug longjmp shapes of section 6.1): once execution is
///   back in a *shallower* frame than the finished one, the next
///   seam is the return-point stop.
/// - `Continue` never stops at a seam.
pub fn arming_stops_at_seam(arming: StepArming, depth: usize) -> bool {
    match arming {
        StepArming::Continue => false,
        StepArming::StepInto => true,
        StepArming::StepOver { max_depth } => depth <= max_depth,
        StepArming::Finish { frame_depth } => depth < frame_depth,
        // `step-`: the first seam back in a shallower frame. The
        // depth rule matches Finish's backstop; the difference
        // is service — `arming_stops_at_frame_return` never
        // services this arming, so the stop is that seam itself.
        StepArming::StepOutOfFrame { frame_depth } => depth < frame_depth,
        // `step+` is a file rule, not a depth rule: without the
        // seam's file it cannot be decided here. The consult
        // uses `arming_stops_at_seam_in_file`.
        StepArming::StepInFile => false,
    }
}

/// Does `arming` stop at a command seam firing at `depth` in the
/// file whose canonical identity is `seam_identity`?
///
/// The file-aware form of [`arming_stops_at_seam`]: every arming
/// delegates to the depth rule except `StepInFile` (`step+`),
/// which stops at the first seam whose identity equals the
/// session's recorded step-file scope. Both strings are
/// canonical identities (converged by
/// `super::resolution::file_identity` at their own points), so
/// the comparison is string equality on identities — a seam in
/// a different file, however spelled, never matches.
pub fn arming_stops_at_seam_in_file(
    arming: StepArming,
    depth: usize,
    seam_identity: Option<&str>,
    step_file_scope: Option<&str>,
) -> bool {
    match arming {
        StepArming::StepInFile => {
            matches!((seam_identity, step_file_scope), (Some(seam), Some(scope)) if seam == scope)
        }
        other => arming_stops_at_seam(other, depth),
    }
}

/// Does `arming` stop at this frame return?
///
/// Only `Finish` is serviced at return sites, and only when the
/// returning frame is the frame the arming names: the session then
/// stops in the caller, before the caller's next seam (section
/// 6.1's finish row). Every route counts — a frame that returns by
/// completion, by `return`, by entry skip, or by an entry
/// force-return has still left the frame the user asked to finish.
pub fn arming_stops_at_frame_return(arming: StepArming, frame: &FrameReturn) -> bool {
    matches!(
        arming,
        StepArming::Finish { frame_depth } if frame_depth == frame.frame_depth
    )
}

#[cfg(test)]
mod tests {
    use super::{arming_stops_at_frame_return, arming_stops_at_seam};
    use crate::debugger::event::{FrameReturn, ReturnRoute};
    use crate::debugger::session::StepArming;

    fn frame_return(depth: usize, route: ReturnRoute) -> FrameReturn {
        FrameReturn {
            function: Some("f".to_string()),
            frame_depth: depth,
            status: 0,
            route,
        }
    }

    #[test]
    fn step_into_stops_at_every_seam() {
        for depth in [0, 1, 2, 9] {
            assert!(arming_stops_at_seam(StepArming::StepInto, depth));
        }
    }

    #[test]
    fn continue_stops_at_no_seam() {
        for depth in [0, 1, 2, 9] {
            assert!(!arming_stops_at_seam(StepArming::Continue, depth));
        }
    }

    #[test]
    fn step_over_stops_at_its_own_depth_and_shallower_only() {
        let arming = StepArming::StepOver { max_depth: 2 };
        assert!(arming_stops_at_seam(arming, 0));
        assert!(arming_stops_at_seam(arming, 1));
        assert!(arming_stops_at_seam(arming, 2), "same depth: stops");
        assert!(
            !arming_stops_at_seam(arming, 3),
            "a called frame's seams are stepped over"
        );
    }

    #[test]
    fn finish_stops_at_no_seam_inside_the_finished_frame() {
        let arming = StepArming::Finish { frame_depth: 2 };
        assert!(!arming_stops_at_seam(arming, 2), "inside the frame");
        assert!(!arming_stops_at_seam(arming, 3), "inside a callee");
        assert!(
            arming_stops_at_seam(arming, 1),
            "backstop: the first seam back in the caller stops"
        );
        assert!(arming_stops_at_seam(arming, 0));
    }

    #[test]
    fn finish_is_serviced_at_its_own_frames_return_site() {
        let arming = StepArming::Finish { frame_depth: 2 };
        assert!(arming_stops_at_frame_return(
            arming,
            &frame_return(2, ReturnRoute::Completed)
        ));
        assert!(
            !arming_stops_at_frame_return(arming, &frame_return(1, ReturnRoute::Completed)),
            "a shallower frame returning is not the finish"
        );
        assert!(
            !arming_stops_at_frame_return(arming, &frame_return(3, ReturnRoute::Completed)),
            "a deeper frame returning is not the finish either"
        );
    }

    #[test]
    fn finish_matches_every_return_route() {
        let arming = StepArming::Finish { frame_depth: 1 };
        for route in [
            ReturnRoute::Completed,
            ReturnRoute::EntrySkipped,
            ReturnRoute::EntryForceReturn,
            ReturnRoute::SourcedFile,
        ] {
            assert!(
                arming_stops_at_frame_return(arming, &frame_return(1, route)),
                "route {route:?} still leaves the finished frame"
            );
        }
    }

    #[test]
    fn step_out_of_frame_stops_at_the_first_shallower_seam_only() {
        let arming = StepArming::StepOutOfFrame { frame_depth: 2 };
        assert!(!arming_stops_at_seam(arming, 2), "inside the frame");
        assert!(!arming_stops_at_seam(arming, 3), "inside a callee");
        assert!(arming_stops_at_seam(arming, 1), "back in the caller");
        assert!(arming_stops_at_seam(arming, 0));
        // ...and unlike Finish it is never serviced at a return
        // site: its stop is the shallower seam itself.
        assert!(!arming_stops_at_frame_return(
            arming,
            &frame_return(2, ReturnRoute::Completed)
        ));
    }

    #[test]
    fn step_in_file_compares_canonical_identities() {
        use super::arming_stops_at_seam_in_file;
        let arming = StepArming::StepInFile;
        assert!(arming_stops_at_seam_in_file(
            arming,
            3,
            Some("/x/a.sh"),
            Some("/x/a.sh")
        ));
        assert!(!arming_stops_at_seam_in_file(
            arming,
            0,
            Some("/x/b.sh"),
            Some("/x/a.sh")
        ));
        assert!(!arming_stops_at_seam_in_file(
            arming,
            0,
            None,
            Some("/x/a.sh")
        ));
        assert!(!arming_stops_at_seam_in_file(
            arming,
            0,
            Some("/x/a.sh"),
            None
        ));
        // The depth-only predicate cannot decide a file rule.
        assert!(!arming_stops_at_seam(arming, 0));
        // Other armings ignore the file entirely.
        assert!(arming_stops_at_seam_in_file(
            StepArming::StepInto,
            5,
            Some("/x/b.sh"),
            Some("/x/a.sh")
        ));
    }

    #[test]
    fn only_finish_is_serviced_at_return_sites() {
        let frame = frame_return(1, ReturnRoute::Completed);
        assert!(!arming_stops_at_frame_return(StepArming::Continue, &frame));
        assert!(!arming_stops_at_frame_return(StepArming::StepInto, &frame));
        assert!(!arming_stops_at_frame_return(
            StepArming::StepOver { max_depth: 1 },
            &frame
        ));
    }
}
