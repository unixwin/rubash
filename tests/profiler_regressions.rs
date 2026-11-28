//! rubash#815 script profiler: launcher interface, semantic zero-disturbance,
//! and output structure.
//!
//! The profiler is a pure observer over the executor's dispatch seams (GNU
//! anchors cited in src/executor/profiler.rs). The load-bearing property is
//! byte-identity: `rubash --profile script` must produce byte-identical
//! stdout, stderr, and rc as plain `rubash script` — that is what makes the
//! 83-suite GNU differential carry over to profile mode unexamined. These
//! tests pin that property on a class-covering probe set, then pin the
//! deterministic parts of the text-tree and collapsed-stack outputs.

use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};

fn run_rubash(args: &[&str], dir: &std::path::Path) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    let code = child.wait().expect("wait rubash").code();
    (stdout, stderr, code)
}

struct TempScript {
    dir: std::path::PathBuf,
}

impl TempScript {
    fn new(name: &str, body: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-815-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("probe.sh"), body).unwrap();
        TempScript { dir }
    }

    fn script(&self) -> String {
        self.dir.join("probe.sh").to_string_lossy().into_owned()
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Scripts covering every instrumented seam class: functions (recursion,
/// `return`), the top-level dispatch (builtins), external spawn, flat
/// subshell, command substitution, pipelines (inline arms + helpers).
const PROBE_SCRIPTS: &[(&str, &str)] = &[
    (
        "functions",
        "f() {\n  echo in-f $1\n  g $1\necho after-g\n}\ng() {\n  [ \"$1\" -gt 0 ] && return 0\n  return 1\n}\nf 3\nf x\n",
    ),
    (
        "comsub-subshell",
        "r=$(echo cap)\necho \"r=$r\"\n( echo sub )\nv=${x:-$(printf '%s' dflt)}\necho \"v=$v\"\n",
    ),
    (
        "pipeline",
        "printf 'b\\na\\n' | sort\nprintf 'x\\n' | grep x\nyes | head -1\n",
    ),
    (
        "external",
        "ls .\n./definitely-missing-binary-815 2>/dev/null\necho rc=$?\n",
    ),
    (
        "aborts",
        "er() {\n  return 3\n}\ner\necho rc=$?\n( exit 5 )\necho sub-rc=$?\nif false; then\n  :\nfi\nfor i in 1 2 3; do echo $i; done\ncase a in a) echo case-a ;; esac\n",
    ),
    (
        "arith-loops",
        "x=0\nfor ((i=0; i<20; i++)); do x=$((x + i)); done\necho x=$x\nwhile [ $x -gt 0 ]; do x=$((x - 5)); done\necho x=$x\n",
    ),
];

fn profile_argv(script: &str, collapsed_out: &str) -> Vec<String> {
    vec![
        "--profile".to_string(),
        "--profile-output".to_string(),
        collapsed_out.to_string(),
        script.to_string(),
    ]
}

/// The killer property, scripted: profile mode changes no visible byte.
#[test]
fn profile_mode_is_byte_identical_to_profile_off() {
    for (name, body) in PROBE_SCRIPTS {
        let temp = TempScript::new(name, body);
        let (plain_out, plain_err, plain_rc) = run_rubash(&[&temp.script()], &temp.dir);
        let collapsed_path = temp.dir.join("p.collapsed");
        let argv = profile_argv(&temp.script(), &collapsed_path.to_string_lossy());
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (prof_out, prof_err, prof_rc) = run_rubash(&argv, &temp.dir);
        assert_eq!(
            plain_out, prof_out,
            "stdout diverged under --profile (suite {name})"
        );
        assert_eq!(
            plain_err, prof_err,
            "stderr diverged under --profile (suite {name})"
        );
        assert_eq!(
            plain_rc, prof_rc,
            "rc diverged under --profile (suite {name})"
        );
        assert!(
            collapsed_path.exists(),
            "collapsed report missing for {name}"
        );
    }
}

/// The unique collapsed-stack path set of a fixed script is deterministic.
/// Same probe script as the committed fixture; the expected list below is
/// the fixture's masked content spelled out for a readable failure diff.
#[test]
fn collapsed_stack_paths_are_deterministic() {
    let body = include_str!("fixtures/profiler/demo.sh");
    let temp = TempScript::new("collapsed", body);
    let collapsed_path = temp.dir.join("p.collapsed");
    let argv = profile_argv(&temp.script(), &collapsed_path.to_string_lossy());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let _ = run_rubash(&argv, &temp.dir);
    let collapsed = fs::read_to_string(&collapsed_path).unwrap();

    let mut paths: Vec<String> = collapsed
        .lines()
        .map(|line| {
            let (path, weight) = line.rsplit_once(' ').expect("collapsed line shape");
            let weight: u64 = weight.parse().expect("collapsed weight is an integer");
            assert!(weight > 0, "collapsed self-time weight must be positive");
            path.to_string()
        })
        .collect();
    paths.sort();
    // fib(5) with the -le 2 base: 15 invocations, every recursion level
    // nesting inside a $(comsub) of its caller; wrap's builtin/local frames,
    // the external spawn, and the flat subshell are their own frames.
    // Sorted lexicographically — this list is the structure fingerprint.
    let expected: Vec<&str> = vec![
        "main",
        "main;wrap",
        "main;wrap;$(comsub)",
        "main;wrap;$(comsub);fib",
        "main;wrap;$(comsub);fib;$(comsub)",
        "main;wrap;$(comsub);fib;$(comsub);fib",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub)",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib;$(comsub)",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib;$(comsub);fib",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib;$(comsub);fib;builtin:[",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib;$(comsub);fib;builtin:echo",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib;builtin:[",
        "main;wrap;$(comsub);fib;$(comsub);fib;$(comsub);fib;builtin:echo",
        "main;wrap;$(comsub);fib;$(comsub);fib;builtin:[",
        "main;wrap;$(comsub);fib;$(comsub);fib;builtin:echo",
        "main;wrap;$(comsub);fib;builtin:[",
        "main;wrap;$(comsub);fib;builtin:echo",
        "main;wrap;(subshell)",
        "main;wrap;(subshell);builtin:echo",
        "main;wrap;builtin:echo",
        "main;wrap;builtin:local",
        "main;wrap;external:ls",
    ];
    assert_eq!(paths, expected, "collapsed path set diverged");
}

/// The text report: attribution faces present, per-function table, indented
/// call tree, three-way separation.
#[test]
fn text_report_structure() {
    let body = "\
hot() {
  local i=0
  while [ $i -lt 50 ]; do
    i=$((i + 1))
  done
}
cold() {
  echo cold
}
hot
cold
ls . >/dev/null 2>&1
( echo sub )
r=$(echo cap)
";
    let temp = TempScript::new("text", body);
    let text_path = temp.dir.join("report.txt");
    let collapsed_path = temp.dir.join("report.txt.collapsed");
    let argv = vec![
        "--profile".to_string(),
        "--profile-output".to_string(),
        text_path.to_string_lossy().into_owned(),
        temp.script(),
    ];
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let _ = run_rubash(&argv, &temp.dir);
    let report = fs::read_to_string(&text_path).unwrap();
    let collapsed = fs::read_to_string(&collapsed_path).unwrap();
    assert!(!collapsed.is_empty(), "collapsed output empty");

    // Header + total wall line.
    assert!(report.contains("# rubash script profile"));
    assert!(report.contains("# total wall:"));

    // Attribution faces, three-way separated from the interpreter face.
    for face in [
        "interpreter self",
        "builtins",
        "external commands",
        "subshell + comsub",
    ] {
        assert!(report.contains(face), "attribution face missing: {face}");
    }
    // Faces are a self-time split: the four percentages must sum to ~100%.
    let mut percent_sum = 0.0f64;
    for line in report.lines() {
        let is_face_line = line.starts_with("interpreter self")
            || line.starts_with("builtins")
            || line.starts_with("external commands")
            || line.starts_with("subshell + comsub");
        let pct = if is_face_line {
            line.split_whitespace().find_map(|token| {
                token
                    .strip_suffix('%')
                    .and_then(|value| value.parse::<f64>().ok())
            })
        } else {
            None
        };
        if let Some(pct) = pct {
            percent_sum += pct;
        }
    }
    assert!(
        (99.0..=101.0).contains(&percent_sum),
        "attribution faces sum to {percent_sum}% (must be ~100)"
    );

    // Per-function table names both functions.
    assert!(report.contains("  hot"), "function row missing: hot");
    assert!(report.contains("  cold"), "function row missing: cold");

    // Indented call tree carries the root and the nested faces.
    assert!(report.contains("main [1] self"));
    assert!(report.contains("  (subshell) [1]"));
    assert!(report.contains("  $(comsub) [1]"));
    assert!(report.contains("  external:ls [1]"));
}

/// Abort forms attribute to the function boundary: `return` inside the
/// function and `exit` inside a subshell stay inside their frames.
#[test]
fn abort_forms_attribute_correctly() {
    let body = "\
er() {
  return 3
}
er
( exit 5 )
echo done
";
    let temp = TempScript::new("aborts", body);
    let collapsed_path = temp.dir.join("p.collapsed");
    let argv = profile_argv(&temp.script(), &collapsed_path.to_string_lossy());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (_, _, rc) = run_rubash(&argv, &temp.dir);
    assert_eq!(rc, Some(0));
    let collapsed = fs::read_to_string(&collapsed_path).unwrap();
    assert!(
        collapsed.contains("\nmain;er;builtin:return "),
        "return abort must land inside er: {collapsed}"
    );
    assert!(
        collapsed.contains("\nmain;(subshell);builtin:exit "),
        "subshell exit must stay inside the subshell frame: {collapsed}"
    );
}

/// `--profile-output <base>.collapsed` writes the text report next to it as
/// `<base>.txt` (the swapped-suffix form).
#[test]
fn profile_output_collapsed_suffix_form() {
    let temp = TempScript::new("suffix", "echo hi\n");
    let collapsed_path = temp.dir.join("p.collapsed");
    let argv = profile_argv(&temp.script(), &collapsed_path.to_string_lossy());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let _ = run_rubash(&argv, &temp.dir);
    assert!(collapsed_path.exists());
    assert!(temp.dir.join("p.txt").exists());
}

/// The committed fixture: the normalized collapsed output of the fixture
/// script must match byte-for-byte (times masked, structure exact).
#[test]
fn fixture_collapsed_matches_normalized_output() {
    let fixture_body = include_str!("fixtures/profiler/demo.sh");
    let temp = TempScript::new("fixture", fixture_body);
    let collapsed_path = temp.dir.join("p.collapsed");
    let argv = profile_argv(&temp.script(), &collapsed_path.to_string_lossy());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let _ = run_rubash(&argv, &temp.dir);
    let collapsed = fs::read_to_string(&collapsed_path).unwrap();
    let normalized = normalize_collapsed(&collapsed);
    let expected = include_str!("fixtures/profiler/demo.collapsed.masked");
    assert_eq!(
        normalized.trim_end(),
        expected.trim_end(),
        "normalized collapsed output diverged from the committed fixture"
    );
}

/// The committed TEXT fixture: the text call tree of the fixture script,
/// with every time/percentage masked (counts and call-tree shape are
/// deterministic and must match byte-for-byte).
#[test]
fn fixture_text_tree_matches_normalized_output() {
    let fixture_body = include_str!("fixtures/profiler/demo.sh");
    let temp = TempScript::new("fixture-text", fixture_body);
    let text_path = temp.dir.join("demo.txt");
    let argv = vec![
        "--profile".to_string(),
        "--profile-output".to_string(),
        text_path.to_string_lossy().into_owned(),
        temp.script(),
    ];
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let _ = run_rubash(&argv, &temp.dir);
    let report = fs::read_to_string(&text_path).unwrap();
    // Column padding depends on the time magnitude (`7.39ms` vs
    // `126.23ms`), so whitespace is collapsed inside normalize_text before
    // the deterministic line-set sort. Applying the same normalization to
    // the fixture keeps frame names and call counts strict without letting
    // column-width timing jitter reorder otherwise identical lines.
    assert_eq!(
        normalize_text(&report),
        normalize_text(include_str!("fixtures/profiler/demo.txt.masked")),
        "normalized text tree diverged from the committed fixture"
    );
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Masks the time/percentage fields of the text report (wall durations and
/// self% vary run to run); call counts and tree shape stay exact.
fn normalize_text(report: &str) -> String {
    // Collapse each line before sorting: column alignment depends on the
    // unmasked timing width, while the line set itself is deterministic.
    // Sorted so sibling rows whose SELF order is timing-dependent (close
    // durations reorder across runs) compare as the same line set.
    let mut lines: Vec<String> = report
        .lines()
        .map(|line| collapse_ws(&regex_mask_times(line)))
        .collect();
    lines.sort();
    lines.join("\n")
}

fn regex_mask_times(line: &str) -> String {
    // The test binary cannot pull a regex crate (repo has none); a small
    // scanner masks `<digits>[.<digits>]<unit>` tokens (ns/us/ms/s) and
    // `<digits>.<digit>%` shares.
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let number = &line[start..i];
            let rest = &line[i..];
            let is_time = rest.starts_with("ns")
                || rest.starts_with("us")
                || rest.starts_with("ms")
                || (rest.starts_with('s') && !rest.starts_with("self"));
            let is_share = number.contains('.') && rest.starts_with('%');
            if is_time {
                out.push('T');
                if rest.starts_with("ms") || rest.starts_with("us") || rest.starts_with("ns") {
                    i += 2;
                } else if rest.starts_with('s') {
                    i += 1;
                }
            } else if is_share {
                out.push_str("P%");
                i += 1;
            } else {
                out.push_str(number);
            }
        } else {
            out.push(b as char);
            i += 1;
        }
    }
    out
}

/// Masks the time weight of every collapsed line, keeping the path set and
/// line order (sorted by path) — the deterministic part of the output.
fn normalize_collapsed(collapsed: &str) -> String {
    let mut lines: Vec<String> = collapsed
        .lines()
        .map(|line| match line.rsplit_once(' ') {
            Some((path, weight)) if weight.bytes().all(|b| b.is_ascii_digit()) => {
                format!("{path} W")
            }
            _ => line.to_string(),
        })
        .collect();
    lines.sort();
    lines.join("\n")
}
