//! The builtin differential-matrix generator (Stage-2 project ④, D5).
//!
//! The case set is GENERATED from `BUILTIN_TABLE` — not hand-written
//! — so a new builtin row cannot hide from the matrix: the
//! generator covers every row, every spec letter, every
//! `--help`/`--` position, and the missing-arg / illegal-character
//! probes, in the four script contexts (`DESIGN.md` §4.5).
//!
//! Dimensions, per row:
//!
//! * `--help` first word (every row — pins the `help_entry` bit),
//!   `--help` after an option and after an operand (`Getopt` rows),
//!   `-- --help` (`Getopt` rows — pins that `--` ends scanning).
//! * One illegal-option probe (`-Z`; no spec in the table contains
//!   `Z`) — for `Grammar`/`NoScan` rows `-Z` is an operand, which
//!   is itself the pin.
//! * The bare invocation (every row except `times`, whose CPU-time
//!   values are non-deterministic — the row is still covered by its
//!   `--help` and `-Z` cells; its shape is pinned by the E0 ledger
//!   and by a dedicated unit test, not by a normalized comparison).
//! * Per spec letter (`Getopt` rows): the bare `-<letter>`; a
//!   cluster of the first two no-arg letters; for `':'` the
//!   attached (`-<letter>VALUE`), separate (`-<letter> VALUE`) and
//!   missing (`-<letter>` last) forms; for `';'` the attached,
//!   separate-with-a-non-option, and separate-with-an-option-word
//!   (`-<letter> -Z`) forms; for `'#'` the attached digit
//!   (`-<letter>42`), attached non-digit (`-<letter>X`), separate
//!   valid (`-<letter> 42`), separate invalid (`-<letter> xyz`) and
//!   separate negative (`-<letter> -1`) forms.
//! * Contexts: script file, `-c`, inside a function, `set -o posix`.
//!
//! The generator itself is pure and fast; its invariants are pinned
//! by the unit tests below. Execution is report-mode only: the
//! `#[ignore]`d `matrix_report` test writes the generated cases as
//! JSON lines for the driver (`matrix/run-matrix.py`), which runs
//! each case under GNU 5.3.20 and rubash (script files, separate
//! streams, timeout) and writes the ledger. The driver is NOT part
//! of the crate and never runs in CI.

use super::builtin_table::{BuiltinRow, OptSpec, BUILTIN_TABLE};
use std::io;
use std::path::Path;

/// The execution context of a matrix cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixContext {
    /// A script file passed to the shell.
    ScriptFile,
    /// The same script passed via `-c`.
    DashC,
    /// The command inside a function that is then called.
    Function,
    /// `set -o posix` precedes the command.
    Posix,
}

impl MatrixContext {
    pub fn slug(self) -> &'static str {
        match self {
            MatrixContext::ScriptFile => "file",
            MatrixContext::DashC => "dash-c",
            MatrixContext::Function => "function",
            MatrixContext::Posix => "posix",
        }
    }

    /// Wrap a command line in this context's script text.
    pub fn wrap(self, cmd: &str) -> String {
        match self {
            MatrixContext::ScriptFile | MatrixContext::DashC => format!("{cmd}\n"),
            MatrixContext::Function => format!("f() {{\n  {cmd}\n}}\nf\n"),
            MatrixContext::Posix => format!("set -o posix\n{cmd}\n"),
        }
    }
}

/// One generated matrix cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCase {
    /// Stable id: `{builtin}--{slug}--{context}`.
    pub id: String,
    pub builtin: &'static str,
    pub context: MatrixContext,
    /// The command words (for the ledger; the script is what runs).
    pub words: Vec<String>,
    /// The full script text for this context.
    pub script: String,
    /// Compare stdout/stderr with digit runs normalized. Reserved
    /// for future non-deterministic cells; currently always false —
    /// the only such cell (`times` bare) is skipped outright rather
    /// than normalized.
    pub normalize: bool,
}

/// The contexts, in the order the ledger lists them.
pub const CONTEXTS: [MatrixContext; 4] = [
    MatrixContext::ScriptFile,
    MatrixContext::DashC,
    MatrixContext::Function,
    MatrixContext::Posix,
];

/// Generate the full case set from the table.
pub fn generate_cases() -> Vec<MatrixCase> {
    let mut cases = Vec::new();
    for row in BUILTIN_TABLE {
        for context in CONTEXTS {
            generate_row_cases(row, context, &mut cases);
        }
    }
    cases
}

fn cmd(words: &[String]) -> String {
    words.join(" ")
}

fn push(
    cases: &mut Vec<MatrixCase>,
    row: &BuiltinRow,
    context: MatrixContext,
    slug: &str,
    words: Vec<String>,
    normalize: bool,
) {
    let id = format!("{}--{}--{}", row.name, slug, context.slug());
    let script = context.wrap(&cmd(&words));
    cases.push(MatrixCase {
        id,
        builtin: row.name,
        context,
        words,
        script,
        normalize,
    });
}

fn generate_row_cases(row: &BuiltinRow, context: MatrixContext, cases: &mut Vec<MatrixCase>) {
    let name = row.name.to_string();
    // --help first word (every row: pins help_entry).
    push(
        cases,
        row,
        context,
        "help-first",
        vec![name.clone(), "--help".to_string()],
        false,
    );
    // The bare invocation (every row except times: its CPU-time
    // values are non-deterministic; the row is still covered by
    // the --help and -Z cells, and its shape by the E0 ledger).
    if row.name != "times" {
        push(cases, row, context, "bare", vec![name.clone()], false);
    }
    // The illegal-option probe (every row).
    push(
        cases,
        row,
        context,
        "bad-opt-Z",
        vec![name.clone(), "-Z".to_string()],
        false,
    );
    let OptSpec::Getopt(spec) = row.opt_spec else {
        return;
    };
    // --help after an option, after an operand, and -- --help.
    let first_noarg = spec_letters(spec)
        .into_iter()
        .find(|(_, k)| *k == ArgKind::None);
    if let Some((letter, _)) = first_noarg {
        push(
            cases,
            row,
            context,
            "help-after-opt",
            vec![name.clone(), format!("-{letter}"), "--help".to_string()],
            false,
        );
    }
    push(
        cases,
        row,
        context,
        "help-after-operand",
        vec![name.clone(), "x".to_string(), "--help".to_string()],
        false,
    );
    push(
        cases,
        row,
        context,
        "dashdash-help",
        vec![name.clone(), "--".to_string(), "--help".to_string()],
        false,
    );
    // Per spec letter.
    let letters = spec_letters(spec);
    let noarg: Vec<u8> = letters
        .iter()
        .filter(|(_, k)| *k == ArgKind::None)
        .map(|(c, _)| *c)
        .collect();
    if noarg.len() >= 2 {
        push(
            cases,
            row,
            context,
            "cluster",
            vec![
                name.clone(),
                format!("-{}{}", noarg[0] as char, noarg[1] as char),
            ],
            false,
        );
    }
    for (letter, kind) in &letters {
        let l = *letter as char;
        match kind {
            ArgKind::None => {
                push(
                    cases,
                    row,
                    context,
                    &format!("opt-{l}"),
                    vec![name.clone(), format!("-{l}")],
                    false,
                );
            }
            ArgKind::Required => {
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-attached"),
                    vec![name.clone(), format!("-{l}VALUE")],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-separate"),
                    vec![name.clone(), format!("-{l}"), "VALUE".to_string()],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-missing"),
                    vec![name.clone(), format!("-{l}")],
                    false,
                );
            }
            ArgKind::Optional => {
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-attached"),
                    vec![name.clone(), format!("-{l}VALUE")],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-separate"),
                    vec![name.clone(), format!("-{l}"), "VALUE".to_string()],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-optword"),
                    vec![name.clone(), format!("-{l}"), "-Z".to_string()],
                    false,
                );
            }
            ArgKind::Numeric => {
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-digits"),
                    vec![name.clone(), format!("-{l}42")],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-nondigit"),
                    vec![name.clone(), format!("-{l}X")],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-separate"),
                    vec![name.clone(), format!("-{l}"), "42".to_string()],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-badnum"),
                    vec![name.clone(), format!("-{l}"), "xyz".to_string()],
                    false,
                );
                push(
                    cases,
                    row,
                    context,
                    &format!("arg-{l}-negative"),
                    vec![name.clone(), format!("-{l}"), "-1".to_string()],
                    false,
                );
            }
        }
    }
}

/// The argument kind of a spec letter (mirrors `builtin_opt`'s
/// spec parsing, without the machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgKind {
    None,
    Required,
    Optional,
    Numeric,
}

/// The spec's option letters in order, with their argument kinds.
/// A leading `'+'` and a leading `':'` are skipped.
fn spec_letters(spec: &str) -> Vec<(u8, ArgKind)> {
    let bytes = spec.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    if bytes.first() == Some(&b'+') {
        i = 1;
    }
    while i < bytes.len() {
        let c = bytes[i];
        if c == b':' {
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

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Write the generated cases as JSON lines (`{"id":…,"builtin":…,
/// "context":…,"words":[…],"script":…,"normalize":…}`) for the
/// driver. Returns the case count.
pub fn write_cases_json(path: &Path) -> io::Result<usize> {
    let cases = generate_cases();
    let mut out = String::new();
    for case in &cases {
        let words: Vec<String> = case
            .words
            .iter()
            .map(|w| format!("\"{}\"", json_escape(w)))
            .collect();
        out.push_str(&format!(
            "{{\"id\":\"{}\",\"builtin\":\"{}\",\"context\":\"{}\",\"words\":[{}],\"script\":\"{}\",\"normalize\":{}}}\n",
            json_escape(&case.id),
            json_escape(case.builtin),
            case.context.slug(),
            words.join(","),
            json_escape(&case.script),
            case.normalize,
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, out)?;
    Ok(cases.len())
}

#[cfg(test)]
mod tests {
    use super::super::builtin_table::{builtin_table_row, BUILTIN_TABLE};
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_row_has_cells_in_every_context() {
        let cases = generate_cases();
        for row in BUILTIN_TABLE {
            for context in CONTEXTS {
                let n = cases
                    .iter()
                    .filter(|c| c.builtin == row.name && c.context == context)
                    .count();
                assert!(n >= 3, "{} in {:?} has only {n} cells", row.name, context);
            }
        }
    }

    #[test]
    fn case_ids_are_unique_and_scripts_are_nonempty() {
        let cases = generate_cases();
        let mut ids = HashSet::new();
        for case in &cases {
            assert!(ids.insert(case.id.clone()), "duplicate id {}", case.id);
            assert!(!case.script.is_empty(), "empty script for {}", case.id);
            assert!(
                case.script.ends_with('\n'),
                "script must end with a newline: {}",
                case.id
            );
        }
    }

    #[test]
    fn every_spec_letter_has_its_cells() {
        // No-arg letters get `opt-<l>`; Required/Optional get the
        // `arg-<l>-attached` cell; Numeric gets `arg-<l>-digits`.
        // (ulimit's `n` is Optional `;`, not Numeric `#` — no row
        // in the table carries a `#` letter; that machine path is
        // pinned by the synthetic-row unit test in builtin_opt.)
        let cases = generate_cases();
        for row in BUILTIN_TABLE {
            let OptSpec::Getopt(spec) = row.opt_spec else {
                continue;
            };
            for (letter, kind) in spec_letters(spec) {
                let l = letter as char;
                let slug = match kind {
                    ArgKind::None => format!("opt-{l}"),
                    ArgKind::Required | ArgKind::Optional => format!("arg-{l}-attached"),
                    ArgKind::Numeric => format!("arg-{l}-digits"),
                };
                let want = format!("{}--{}--file", row.name, slug);
                assert!(
                    cases.iter().any(|c| c.id == want),
                    "missing cell {want} for {}{l}",
                    row.name
                );
            }
        }
    }

    #[test]
    fn help_first_cell_exists_for_every_row() {
        let cases = generate_cases();
        for row in BUILTIN_TABLE {
            let want = format!("{}--help-first--file", row.name);
            assert!(cases.iter().any(|c| c.id == want), "missing {want}");
        }
    }

    #[test]
    fn times_has_no_bare_cell_and_nothing_is_normalized() {
        // The bare cell is skipped for times (non-deterministic
        // CPU-time values); every other row has one per context.
        // No normalize cell is generated at all yet.
        let cases = generate_cases();
        assert!(cases.iter().all(|c| !c.normalize), "normalize is reserved");
        for row in BUILTIN_TABLE {
            let bare = cases.iter().filter(|c| {
                c.builtin == row.name && c.id.starts_with(&format!("{}--bare--", row.name))
            });
            if row.name == "times" {
                assert_eq!(bare.count(), 0, "times must have no bare cell");
            } else {
                assert_eq!(bare.count(), 4, "{} must have 4 bare cells", row.name);
            }
        }
    }

    #[test]
    fn matrix_counts_are_stable() {
        // A tripwire, not a target: if the count moves, the
        // generator changed and the ledger's baseline moved with it.
        // Hand-computed from the table: 779 per context (725 Getopt
        // + 39 NoScan + 15 Grammar), 3116 total.
        let cases = generate_cases();
        assert_eq!(cases.len(), 3116);
        let file = cases
            .iter()
            .filter(|c| c.context == MatrixContext::ScriptFile)
            .count();
        assert_eq!(file, 779);
    }

    /// Report mode: write the generated cases for the driver.
    /// Run explicitly: `cargo test --locked --lib builtin_matrix::matrix_report -- --ignored --nocapture`.
    /// The path is `$P4_MATRIX_OUT` or
    /// `<crate>/target/p4-matrix/cases.jsonl`.
    #[test]
    #[ignore]
    fn matrix_report() {
        let path = std::env::var("P4_MATRIX_OUT").unwrap_or_else(|_| {
            format!(
                "{}/target/p4-matrix/cases.jsonl",
                env!("CARGO_MANIFEST_DIR")
            )
        });
        let n = write_cases_json(Path::new(&path)).expect("write cases");
        // Also write the per-row cell counts for the ledger header.
        let cases = generate_cases();
        let mut per_row: Vec<(String, usize)> = BUILTIN_TABLE
            .iter()
            .map(|row| {
                (
                    row.name.to_string(),
                    cases.iter().filter(|c| c.builtin == row.name).count(),
                )
            })
            .collect();
        per_row.sort();
        let mut counts = String::new();
        for (name, n) in &per_row {
            counts.push_str(&format!("{name}: {n}\n"));
        }
        let counts_path = Path::new(&path).with_file_name("counts.txt");
        std::fs::write(counts_path, counts).expect("write counts");
        assert_eq!(builtin_table_row("cd").unwrap().name, "cd");
        eprintln!("matrix: wrote {n} cases to {path}");
    }
}
