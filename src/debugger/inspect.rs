//! WP-6 inspection rendering (debugger design §7; PLAN row 6).
//!
//! Every renderer here consumes the §5.3 snapshots from
//! `crate::api::views` — the same captures the Session API serves —
//! so the debugger's inspection output and the API can never drift
//! into two renderings of one state. Declaration text inside a
//! snapshot is produced by the shared `declare -p` printer at
//! capture time; the frame rendering zips the same dynamic views
//! `caller` reads. Frontends (CLI, DAP) call these and own only the
//! transport.

use crate::api::views::{
    DynamicStateSnapshot, FrameSnapshot, InspectionSnapshot, VariableSnapshot, VariableValue,
};

/// Which slice of a snapshot's variables a listing renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariablePartition {
    /// Every captured variable.
    All,
    /// Bindings in a function-local scope.
    Locals,
    /// Global bindings only.
    Globals,
    /// Indexed and associative arrays.
    Arrays,
    /// Exported variables.
    Exported,
    /// Readonly variables.
    Readonly,
}

/// Render the call stack, innermost frame first, one frame per
/// line: `#<depth> <function> at <source>:<line>`.
pub fn render_where(snapshot: &InspectionSnapshot) -> String {
    snapshot
        .frames
        .iter()
        .enumerate()
        .map(|(depth, frame)| {
            format!(
                "#{depth} {} at {}:{}",
                frame.function, frame.source, frame.line
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `caller`'s presentation rule for a frame source: the synthetic
/// inline-string source renders as `environment` (the mapping
/// `execute_caller` applies before the builtin prints).
fn present_source(source: &str) -> &str {
    if source == "main" {
        "environment"
    } else {
        source
    }
}

/// Render one frame exactly as `caller <level>` prints it:
/// `{BASH_LINENO[level]} {FUNCNAME[level+1]} {BASH_SOURCE[level+1]}`
/// (caller.def:108-121). In frame terms that is the *caller's*
/// frame — its call-site line, name, and presented source. `None`
/// when the level names no frame, matching the builtin's silent
/// failure.
pub fn render_caller(frames: &[FrameSnapshot], level: usize) -> Option<String> {
    let frame = frames.get(level + 1)?;
    Some(format!(
        "{} {} {}",
        frame.line,
        frame.function,
        present_source(&frame.source)
    ))
}

/// Render the no-argument `caller` form: `{BASH_LINENO[0]}
/// {BASH_SOURCE[1]}` (caller.def:89-105). `None` outside a function
/// call, where the builtin fails.
pub fn render_caller_bare(frames: &[FrameSnapshot]) -> Option<String> {
    let frame = frames.get(1)?;
    Some(format!("{} {}", frame.line, present_source(&frame.source)))
}

/// Render a frame's arguments, one per line in call order
/// (`1: first`, `2: second`, …).
pub fn render_info_args(frame: &FrameSnapshot) -> String {
    frame
        .args
        .iter()
        .enumerate()
        .map(|(index, arg)| format!("{}: {}", index + 1, arg))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rendered value of one snapshot variable for the
/// stop-and-diff watch (design section 5.4): the value the
/// watch compares across stops and quotes in its banner. A
/// scalar renders as itself; an array renders as a
/// parenthesized `[key]=value` list — the whole value, not the
/// element-0 reading a plain `$NAME` expansion would give, so
/// an element write is a visible difference. An unset variable
/// renders as `None` (the watch's "no value" coordinate).
pub fn render_watch_value(variable: &VariableSnapshot) -> Option<String> {
    match &variable.value {
        VariableValue::Unset => None,
        VariableValue::Scalar(text) => Some(text.clone()),
        VariableValue::Indexed(elements) => Some(format!(
            "({})",
            elements
                .iter()
                .map(|(index, value)| format!("[{index}]={value}"))
                .collect::<Vec<_>>()
                .join(" ")
        )),
        VariableValue::Associative(elements) => Some(format!(
            "({})",
            elements
                .iter()
                .map(|(key, value)| format!("[{key}]={value}"))
                .collect::<Vec<_>>()
                .join(" ")
        )),
    }
}

/// Render a variable listing as `declare -p` lines: the snapshot
/// already carries each variable's declaration text from the shared
/// printer, so this is selection and joining, never re-rendering.
pub fn render_variables(snapshot: &InspectionSnapshot, partition: VariablePartition) -> String {
    snapshot
        .variables
        .iter()
        .filter(|variable| match partition {
            VariablePartition::All => true,
            VariablePartition::Locals => variable.scope_depth.is_some(),
            VariablePartition::Globals => variable.scope_depth.is_none(),
            VariablePartition::Arrays => {
                matches!(
                    variable.value,
                    VariableValue::Indexed(_) | VariableValue::Associative(_)
                )
            }
            VariablePartition::Exported => variable.attrs.exported,
            VariablePartition::Readonly => variable.attrs.readonly,
        })
        .filter_map(|variable| variable.declaration.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render PIPESTATUS in its own array-element shape:
/// `([0]="1" [1]="0" [2]="1")`.
pub fn render_pipestatus(dynamic: &DynamicStateSnapshot) -> String {
    let elements = dynamic
        .pipestatus
        .iter()
        .map(|(index, status)| format!("[{index}]=\"{status}\""))
        .collect::<Vec<_>>()
        .join(" ");
    format!("({elements})")
}

/// Render the dynamic inspection state (§7.3) as labeled lines.
pub fn render_dynamic_state(dynamic: &DynamicStateSnapshot) -> String {
    let mut lines = vec![
        format!("last_status: {}", dynamic.last_status),
        format!("pipestatus: {}", render_pipestatus(dynamic)),
    ];
    if let Some(command) = &dynamic.bash_command {
        lines.push(format!("bash_command: {command}"));
    }
    if let Some(line) = dynamic.current_line {
        lines.push(format!("current_line: {line}"));
    }
    lines.join("\n")
}
