//! Typed-cell ↔ mirror coherence regression net (rubash#616 follow-up).
//!
//! rubash keeps array state twice while the typed-store migration is in
//! flight: `shell_state.variables` holds the structured cell
//! (`ShellValue::IndexedArray` / `AssociativeArray`) that the O(1) read
//! fast paths trust, while `shell_state.env_vars` holds the rendered
//! mirror projection every storage reader parses. GNU has ONE owner
//! (`bind_variable` writes the ARRAY/ASSOC object, array.c `array_insert`),
//! so the invariant here is:
//!
//! > **fresh ⇒ cell == mirror**
//!
//! A cell is *fresh* while no unsynced mirror write is pending
//! (`VarTable::array_cells_dirty` is clear). A stale (dirty) cell is legal:
//! the mirror is authoritative until the next executor sync point. This
//! file drives a deterministic, bounded random write sequence through the
//! engine and, after every step, checks the invariant two ways:
//!
//! 1. `Executor::verify_typed_cells()` (also called at every command
//!    boundary in debug builds, see `command_execute.rs`) walks every
//!    structured array cell and asserts it equals its mirror parse.
//!    The boundary call may skip per-name-fresh cells (rubash#1510);
//!    the direct call below uses `verify_typed_cells_forced()` so this
//!    net always re-proves every cell in full.
//! 2. Every readback route (`${a[i]}`, `${#a[@]}`, `${a[@]}`, `declare -p`)
//!    is compared against a mirror oracle parsed directly out of
//!    `Executor::env_vars()`.
//!
//! The generator is a fixed-seed xorshift PRNG over a bounded step count,
//! so a failure is reproducible from the seed printed in the panic. The
//! expectation for the focused red case below is GNU bash 5.3.20.
//!
//! Debug builds only: the verifier and the read paths it guards are
//! `debug_assertions`/dev-profile code.

// The verifier entry points (`verify_typed_cells_forced`) exist only
// under `debug_assertions`, so this whole target is gated to match:
// in release builds it compiles to an empty target instead of failing
// to resolve the verifier.
#![cfg(debug_assertions)]

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::sync::Mutex;

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

/// Serializes the tests in this file: the in-process executor writes the
/// real process environment (`set_process_env`), which a concurrently
/// spawned subprocess would inherit.
static LOCK: Mutex<()> = Mutex::new(());

const SEED: u64 = 0x5EED_2026_1021;
const STEPS: usize = 256;

/// xorshift64* — small, deterministic, no external crates.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

fn run(exec: &mut Executor, source: &str) {
    let tokens = tokenize(source);
    let ast = parse(&tokens);
    // A failing command (bad subscript, arithmetic error) still leaves a
    // well-defined state; only the coherence invariant matters here.
    let _ = exec.execute_ast(&ast);
}

/// One readback of `name` through the public parameter-expansion routes.
struct Readback {
    len: String,
    all: String,
    e0: String,
    e1: String,
    last: String,
    declare: String,
}

fn readback(exec: &mut Executor, name: &str) -> Readback {
    // `declare -p` on an unset name prints to stderr; the redirect keeps
    // the command substitution's stdout empty instead of leaking noise.
    let script = format!(
        "TCF_LEN=${{#{name}[@]}}; TCF_ALL=${{{name}[@]}}; \
         TCF_E0=${{{name}[0]}}; TCF_E1=${{{name}[1]}}; \
         TCF_LAST=${{{name}[-1]}}; TCF_DECL=$(declare -p {name} 2>/dev/null)"
    );
    run(exec, &script);
    let get = |key: &str| exec.get_env(key).unwrap_or("").to_string();
    Readback {
        len: get("TCF_LEN"),
        all: get("TCF_ALL"),
        e0: get("TCF_E0"),
        e1: get("TCF_E1"),
        last: get("TCF_LAST"),
        declare: get("TCF_DECL"),
    }
}

/// The mirror projection parsed into an oracle shape.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mirror {
    /// No binding, or a non-array (scalar) binding.
    Absent,
    Indexed(BTreeMap<usize, String>),
    Assoc(BTreeMap<String, String>),
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else {
                out.push(ch);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

/// Split a storage body (`[k]=v [k]=v`) on whitespace, respecting the
/// double-quoted value spans the renderer emits.
fn split_segments(inner: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            '\\' if quoted => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// `[key]=value` → `(key, value)`, quote-aware on the subscript close.
fn parse_segment(segment: &str) -> Option<(String, String)> {
    let rest = segment.strip_prefix('[')?;
    let mut quoted = false;
    let mut close = None;
    for (offset, ch) in rest.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ']' if !quoted => {
                close = Some(offset);
                break;
            }
            _ => {}
        }
    }
    let close = close?;
    let key = &rest[..close];
    let value = rest[close + 1..].strip_prefix('=')?;
    Some((unquote(key), unquote(value)))
}

fn parse_storage(raw: &str) -> Mirror {
    let body = raw.strip_prefix('\u{1d}').unwrap_or(raw).trim();
    let Some(inner) = body
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Mirror::Absent;
    };
    let pairs: Vec<(String, String)> = split_segments(inner)
        .into_iter()
        .filter_map(|segment| parse_segment(&segment))
        .collect();
    if pairs.iter().all(|(key, _)| key.parse::<usize>().is_ok()) {
        Mirror::Indexed(
            pairs
                .into_iter()
                .map(|(key, value)| (key.parse().expect("numeric key"), value))
                .collect(),
        )
    } else {
        Mirror::Assoc(pairs.into_iter().collect())
    }
}

fn parse_mirror(raw: Option<&str>) -> Mirror {
    match raw {
        Some(raw) => parse_storage(raw),
        None => Mirror::Absent,
    }
}

/// Parse the `declare -p` readback by dropping its `declare -a NAME=`
/// prefix and reusing the storage parser on the assignment RHS.
fn parse_declare(decl: &str) -> Mirror {
    match decl.find('=') {
        Some(eq) => parse_storage(&decl[eq + 1..]),
        None => Mirror::Absent,
    }
}

/// Compare one variable's readback against its mirror oracle.
fn check(exec: &mut Executor, name: &str) -> Result<(), String> {
    let oracle = parse_mirror(exec.env_vars().get(name).map(String::as_str));
    let rb = readback(exec, name);
    let declare = parse_declare(&rb.declare);
    let fail = |what: &str, got: &str, want: &str| {
        Err(format!(
            "{name}: {what} read `{got}` but mirror oracle says `{want}` \
             (mirror={:?}, declare={:?})",
            exec.env_vars().get(name),
            rb.declare,
        ))
    };
    match &oracle {
        Mirror::Absent => {
            if rb.len != "0" {
                return fail("length", &rb.len, "0");
            }
            if !rb.all.is_empty() {
                return fail("all-elements", &rb.all, "");
            }
            if !matches!(declare, Mirror::Absent) {
                return fail("declare -p", &rb.declare, "(absent)");
            }
        }
        Mirror::Indexed(map) => {
            let want_len = map.len().to_string();
            if rb.len != want_len {
                return fail("length", &rb.len, &want_len);
            }
            let want_all = map.values().cloned().collect::<Vec<_>>().join(" ");
            if rb.all != want_all {
                return fail("all-elements", &rb.all, &want_all);
            }
            let want_e0 = map.get(&0).cloned().unwrap_or_default();
            if rb.e0 != want_e0 {
                return fail("element[0]", &rb.e0, &want_e0);
            }
            let want_e1 = map.get(&1).cloned().unwrap_or_default();
            if rb.e1 != want_e1 {
                return fail("element[1]", &rb.e1, &want_e1);
            }
            if let Some((_, want_last)) = map.iter().next_back() {
                if rb.last != *want_last {
                    return fail("element[-1]", &rb.last, want_last);
                }
            }
            if declare != oracle {
                return fail("declare -p", &rb.declare, "(mirror render)");
            }
        }
        Mirror::Assoc(map) => {
            let want_len = map.len().to_string();
            if rb.len != want_len {
                return fail("length", &rb.len, &want_len);
            }
            // `${A[@]}` iterates in the table's hash order, not the mirror
            // render order, so compare the value multiset instead.
            let mut got: Vec<String> = rb.all.split_whitespace().map(str::to_string).collect();
            got.sort();
            let mut want: Vec<String> = map.values().cloned().collect();
            want.sort();
            if got != want {
                return fail("all-elements", &rb.all, &want.join(" "));
            }
            let want_e0 = map.get("k0").cloned().unwrap_or_default();
            if rb.e0 != want_e0 {
                return fail("element[k0]", &rb.e0, &want_e0);
            }
            if declare != oracle {
                return fail("declare -p", &rb.declare, "(mirror render)");
            }
        }
    }
    Ok(())
}

const OP_COUNT: usize = 25;

/// Materialize one random write step. `step` seeds unique literal values so
/// a later step cannot silently reuse an earlier one's data.
fn step_command(op: usize, rng: &mut Rng, step: usize) -> String {
    let v0 = format!("v{step}");
    let v1 = format!("v{}b", step);
    let v2 = format!("v{}c", step);
    let index = rng.below(6);
    let number = rng.below(90) + 10;
    let key = format!("k{}", rng.below(3));
    match op {
        0 => format!("a=({v0} {v1} {v2})"),
        1 => format!("a[{index}]={v0}"),
        2 => format!("a[{index}]+={v0}"),
        3 => format!("a+=({v0})"),
        4 => format!("unset 'a[{index}]'"),
        5 => "unset a".to_string(),
        6 => format!("declare -a a=({v0} {v1})"),
        7 => format!("declare -g a=({v0})"),
        8 => format!("printf -v 'a[{index}]' %s {v0}"),
        9 => format!("read -r -a a <<< '{v0} {v1} {v2}'"),
        10 => format!("mapfile -t a <<< $'m{step}\\nm{step}b'"),
        11 => format!("(( a[{index}] = {number} ))"),
        12 => format!("(( a[{index}]++ ))"),
        13 => format!("a[$((j++))]={v0}"),
        14 => format!("a[-1]={v0}"),
        15 => format!(": \"${{a[{index}]={v0}}}\""),
        16 => format!("declare -n tc_ref=a; tc_ref[{index}]={v0}"),
        17 => format!("tc_local() {{ local a=({v0} {v1}); }}; tc_local"),
        18 => format!("export a=({v0})"),
        19 => format!("declare -A A=([k0]={v0} [k1]={v1})"),
        20 => format!("A[{key}]={v0}"),
        21 => format!("unset 'A[{key}]'"),
        22 => format!("A+=([{key}]={v0})"),
        23 => "[[ ab12 =~ ([a-z]+)([0-9]+) ]]".to_string(),
        24 => "coproc tc_coproc { :; }".to_string(),
        _ => unreachable!("op out of range"),
    }
}

/// Deterministic random write sequence over the indexed array `a` and the
/// associative array `A`. Every step runs the engine (which invokes
/// `verify_typed_cells` at the command boundary in debug builds), then the
/// verifier and the four readback routes are compared against the mirror.
#[test]
fn typed_cell_mirror_coherence_survives_random_writes() {
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut exec = Executor::new();
    run(&mut exec, "j=0");
    let mut rng = Rng(SEED);
    for step in 0..STEPS {
        let op = rng.below(OP_COUNT);
        let command = step_command(op, &mut rng, step);
        // The command-boundary verifier fires inside `run`; catch its panic
        // so the failing step/op/command are named in the report instead of
        // a bare assertion location.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| run(&mut exec, &command))) {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic>".to_string());
            panic!("seed={SEED:#x} step={step} op={op} cmd={command:?}: {detail}");
        }
        // The direct call uses the forced variant: the boundary verifier
        // may skip per-name-fresh cells (rubash#1510), but this test's own
        // systemic check must re-prove every cell after every step.
        exec.verify_typed_cells_forced();
        for name in ["a", "A"] {
            if let Err(message) = check(&mut exec, name) {
                panic!("seed={SEED:#x} step={step} op={op} cmd={command:?}: {message}");
            }
        }
    }
}

/// Focused red case (rubash#616 follow-up): GNU bash 5.3.20 grows the
/// array through a parameter default-assignment, so `count` is 3.
/// `parameter_patterns.rs` writes the mirror without syncing the typed
/// cell (and without marking the table dirty), so rubash's typed
/// `${#a[@]}` still reports 2. The product fix belongs to a separate
/// change; this test pins the GNU truth.
#[test]
fn default_assignment_grows_array_length() {
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([
            "-c",
            r#"a=(one two); echo "${a[5]=Z}"; echo "count=${#a[@]}""#,
        ])
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stdout, "Z\ncount=3\n",
        "GNU bash 5.3.20 truth; stderr={stderr:?} status={:?}",
        output.status
    );
}
