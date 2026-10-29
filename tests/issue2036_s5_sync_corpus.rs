//! Three-way sync corpus for rubash#2036 S5: the 34 builtin/WinuxCmd
//! conflict names (audit §2.2) as data (`tests/difftest/cases-2036/
//! corpus.json`), executed against the engine binary and compared to
//! recorded expectations.
//!
//! Expectations were anchored by dual-running every case as a script
//! file under GNU bash 5.3.20 with GNU coreutils 9.4 (the sync target
//! WinuxCmd's own differential CI also pins against) on 2026-10-29:
//! - `status: "gnu"` cases assert the GNU measurement — the engine must
//!   be indistinguishable from GNU on that shape.
//! - `status: "registered"` cases pin the engine's CURRENT measured
//!   behavior where a divergence is adjudicated-keep (a ruled
//!   stand_down Replace cell) or not yet ruled (#2046 B2/B3/C1/C2);
//!   the GNU measurement is stored in the case's `gnu` field so the
//!   future ruling starts from data, not archaeology.
//!
//! The Windows three-way run of this same case set (engine x applet x
//! GNU on a real Windows host) is the audit's win-diff battery
//! (pkg10 H cell); this test is the Linux CI gate and is unix-only.
//!
//! Attribution note (S4 four-quadrant measurement, correcting audit §4
//! and S3 §3.2-a): the simple-command face replaces BOTH script and ELF
//! PATH candidates for cat (shape-blind); the pipeline face yields both
//! shapes because the concurrent direct-spawn planner claims resolvable
//! stage members before the inline arm. The audit's "script yields,
//! ELF is replaced" reading compared a pipeline measurement against a
//! simple-command one.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- JSON
// Minimal JSON reader for the corpus file (objects / arrays / strings /
// numbers / literals). The repo carries no serde dependency; the corpus
// is the only JSON consumer in the test tree, so the parser lives here
// and fails loudly on anything outside the subset it implements.

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
    fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Num(n) => Some(*n as i64),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn parse(text: &'a str) -> Result<Json, String> {
        let mut parser = JsonParser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        let value = parser.value()?;
        parser.ws();
        if parser.pos != parser.bytes.len() {
            return Err(format!("trailing input at byte {}", parser.pos));
        }
        Ok(value)
    }

    fn ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Result<u8, String> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| "unexpected end of input".to_string())
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            _ => self.number(),
        }
    }

    fn literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(text.as_bytes()) {
            self.pos += text.len();
            Ok(value)
        } else {
            Err(format!("bad literal at byte {}", self.pos))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && matches!(
                self.bytes[self.pos],
                b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
            )
        {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "bad number".to_string())?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number '{text}'"))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek()? != b'"' {
            return Err(format!("expected string at byte {}", self.pos));
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let byte = self.peek()?;
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.peek()?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // Surrogate pair: \uXXXX\uXXXX.
                                if self.bytes[self.pos..].starts_with(b"\\u") {
                                    self.pos += 2;
                                    let lo = self.hex4()?;
                                    let c = 0x10000
                                        + (((hi - 0xD800) as u32) << 10)
                                        + (lo as u32 - 0xDC00);
                                    char::from_u32(c).unwrap_or('\u{FFFD}')
                                } else {
                                    '\u{FFFD}'
                                }
                            } else {
                                char::from_u32(hi as u32).unwrap_or('\u{FFFD}')
                            };
                            out.push(ch);
                        }
                        _ => return Err(format!("bad escape at byte {}", self.pos)),
                    }
                }
                _ => {
                    // Raw UTF-8 byte: copy the whole sequence verbatim.
                    let start = self.pos - 1;
                    let width = if byte >= 0xF0 {
                        4
                    } else if byte >= 0xE0 {
                        3
                    } else if byte >= 0xC0 {
                        2
                    } else {
                        1
                    };
                    let end = (start + width).min(self.bytes.len());
                    self.pos = end;
                    out.push_str(
                        std::str::from_utf8(&self.bytes[start..end])
                            .map_err(|_| "bad utf-8 in string".to_string())?,
                    );
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.bytes.len() {
            return Err("short \\u escape".to_string());
        }
        let text = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| "bad \\u escape".to_string())?;
        self.pos += 4;
        u16::from_str_radix(text, 16).map_err(|_| format!("bad \\u escape '{text}'"))
    }

    fn object(&mut self) -> Result<Json, String> {
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        self.ws();
        if self.peek()? == b'}' {
            self.pos += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            if self.peek()? != b':' {
                return Err(format!("expected ':' at byte {}", self.pos));
            }
            self.pos += 1;
            let value = self.value()?;
            pairs.push((key, value));
            self.ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    return Ok(Json::Obj(pairs));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.ws();
        if self.peek()? == b']' {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }
}

// ------------------------------------------------------------- harness

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

const SYS_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

struct CaseResult {
    rc: i64,
    stdout: String,
    stderr: String,
}

/// Run one script file with a hard 20s bound. Both pipes are drained on
/// reader threads WHILE waiting (the #2056 lesson: a harness that waits
/// for exit before draining has no real bound at all).
fn run_bounded(
    dir: &Path,
    path: &str,
    extra_env: &[(String, String)],
    script: &Path,
) -> Result<CaseResult, String> {
    let mut cmd = Command::new(shell_bin());
    cmd.arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", path)
        .env("HOME", dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut term_seen = false;
    for (key, value) in extra_env {
        if key == "TERM" {
            term_seen = true;
            if value.is_empty() {
                continue; // empty TERM in the data means: remove it
            }
        }
        cmd.env(key, value);
    }
    if !term_seen {
        cmd.env("TERM", "xterm");
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn rubash: {e}"))?;
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("TIMEOUT after 20s".to_string());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(format!("try_wait: {e}")),
        }
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    Ok(CaseResult {
        rc: status.code().unwrap_or(-1) as i64,
        stdout: String::from_utf8_lossy(&stdout).replace('\r', ""),
        stderr: String::from_utf8_lossy(&stderr).replace('\r', ""),
    })
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-s5corpus-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn load_corpus() -> Json {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("difftest")
        .join("cases-2036")
        .join("corpus.json");
    let text = fs::read_to_string(&path).expect("read corpus.json");
    JsonParser::parse(&text).expect("parse corpus.json")
}

fn str_map(value: Option<&Json>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(Json::Obj(pairs)) = value {
        for (k, v) in pairs {
            if let Some(s) = v.as_str() {
                out.push((k.clone(), s.to_string()));
            }
        }
    }
    out
}

fn run_case(case: &Json) -> Result<(), String> {
    let id = case.get("id").and_then(Json::as_str).unwrap_or("<no id>");
    let dir = scratch(id);
    let result = (|| -> Result<(), String> {
        for (rel, content) in str_map(case.get("files")) {
            fs::write(dir.join(&rel), content).map_err(|e| format!("write {rel}: {e}"))?;
        }
        let fakebin = dir.join("fakebin");
        let fakes = str_map(case.get("fakes"));
        let elf_fakes: Vec<String> = case
            .get("elf_fakes")
            .and_then(Json::as_arr)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if !fakes.is_empty() || !elf_fakes.is_empty() {
            fs::create_dir_all(&fakebin).map_err(|e| format!("mkdir fakebin: {e}"))?;
            for (name, body) in &fakes {
                let fp = fakebin.join(name);
                fs::write(&fp, format!("#!/bin/sh\n{body}\n"))
                    .map_err(|e| format!("write fake {name}: {e}"))?;
                fs::set_permissions(&fp, fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("chmod fake {name}: {e}"))?;
            }
            for name in &elf_fakes {
                let fp = fakebin.join(name);
                fs::copy("/bin/true", &fp).map_err(|e| format!("elf fake {name}: {e}"))?;
                fs::set_permissions(&fp, fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("chmod elf fake {name}: {e}"))?;
            }
        }
        let path = match case.get("path").and_then(Json::as_str) {
            Some("fakes") => format!("{}:{}", fakebin.display(), SYS_PATH),
            Some("nonexist") => "/nonexist".to_string(),
            _ => SYS_PATH.to_string(),
        };
        let script_text = case
            .get("script")
            .and_then(Json::as_str)
            .ok_or("case has no script")?;
        let script_path = dir.join("case.sh");
        fs::write(&script_path, format!("{script_text}\n"))
            .map_err(|e| format!("write case.sh: {e}"))?;
        let extra_env = str_map(case.get("env"));
        let got = run_bounded(&dir, &path, &extra_env, &script_path)
            .map_err(|e| format!("case {id}: {e}"))?;
        let expect = case.get("expect").ok_or("case has no expect")?;
        let want_rc = expect.get("rc").and_then(Json::as_i64).ok_or("expect.rc")?;
        let dir_text = dir.display().to_string();
        let fill = |text: &str| text.replace("{DIR}", &dir_text);
        let mut problems = Vec::new();
        if got.rc != want_rc {
            problems.push(format!("rc: want {want_rc}, got {}", got.rc));
        }
        if let Some(want) = expect.get("stdout").and_then(Json::as_str) {
            let want = fill(want);
            if got.stdout != want {
                problems.push(format!("stdout: want {:?}, got {:?}", want, got.stdout));
            }
        }
        if let Some(needle) = expect.get("stdout_contains").and_then(Json::as_str) {
            if !got.stdout.contains(needle) {
                problems.push(format!(
                    "stdout: want containing {:?}, got {:?}",
                    needle, got.stdout
                ));
            }
        }
        if let Some(want) = expect.get("stderr").and_then(Json::as_str) {
            let want = fill(want);
            if got.stderr != want {
                problems.push(format!("stderr: want {:?}, got {:?}", want, got.stderr));
            }
        }
        if let Some(needle) = expect.get("stderr_contains").and_then(Json::as_str) {
            if !got.stderr.contains(needle) {
                problems.push(format!(
                    "stderr: want containing {:?}, got {:?}",
                    needle, got.stderr
                ));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!("case {id}: {}", problems.join("; ")))
        }
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}

// ---------------------------------------------------------------- tests

/// The audit §2.2 conflict universe: any face claiming a name that
/// WinuxCmd also registers. The corpus must cover every one of them.
const CONFLICT_NAMES: [&str; 34] = [
    "[", "arch", "basename", "cat", "chmod", "cp", "diff", "dirname", "echo", "env", "false",
    "grep", "head", "kill", "mkdir", "mkfifo", "printf", "pwd", "rm", "rmdir", "sed", "sleep",
    "sort", "stty", "test", "time", "touch", "tput", "tr", "true", "tty", "uname", "wc", "yes",
];

#[test]
fn corpus_covers_the_conflict_universe() {
    let doc = load_corpus();
    let cases = doc.get("cases").and_then(Json::as_arr).expect("cases");
    let mut names = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for case in cases {
        let name = case.get("name").and_then(Json::as_str).expect("case.name");
        names.insert(name.to_string());
        let id = case.get("id").and_then(Json::as_str).expect("case.id");
        assert!(ids.insert(id.to_string()), "duplicate case id {id}");
        let status = case
            .get("status")
            .and_then(Json::as_str)
            .expect("case.status");
        assert!(
            status == "gnu" || status == "registered",
            "case {id}: bad status {status}"
        );
        if status == "registered" {
            assert!(
                case.get("gnu").is_some(),
                "case {id}: registered cases must record the GNU measurement"
            );
        }
    }
    let want: BTreeSet<String> = CONFLICT_NAMES.iter().map(|s| s.to_string()).collect();
    assert_eq!(names, want, "corpus name coverage drifted from audit §2.2");
}

#[test]
fn sync_corpus_engine_matches_recorded_expectations() {
    let doc = load_corpus();
    let cases = doc.get("cases").and_then(Json::as_arr).expect("cases");
    let mut failures = Vec::new();
    for case in cases {
        if let Err(problem) = run_case(case) {
            failures.push(problem);
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
