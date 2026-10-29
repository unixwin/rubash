//! rubash#1657 — adjudication pin (no code change). The report
//! claimed `~name` for a system user absent from the shell's own
//! list expands to `/`. Tracing the tilde owner showed there IS no
//! shell-side user list on unix: `passwd_home_for_user` is
//! `libc::getpwnam_r` verbatim (GNU tilde.c sh_get_home_dir does the
//! same lookup), a miss keeps the word verbatim, and a hit returns
//! the passwd entry's home — including entries whose pw_dir really
//! is `/`. The reported shape came from a platform whose libc
//! passwd database (Android bionic's synthetic AID entries) records
//! `/` as those users' home directories: both shells print `/`
//! there because the DATABASE says so, not because either shell
//! invented it. Verified locally against GNU bash 5.3.20: ~daemon,
//! ~bin, ~root resolve to their getent homes on both shells and an
//! unknown name stays literal on both. This file pins that contract
//! so a future "helpful" fallback (or hardcoded table) cannot pass
//! silently.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i1657-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path).replace(&case.to_string_lossy().to_string(), "SH"),
            rc,
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The home directory recorded for `name` in the system passwd
/// database, read the same way getpwnam sees it on this machine.
fn passwd_home(name: &str) -> Option<String> {
    let data = fs::read_to_string("/etc/passwd").ok()?;
    for line in data.lines() {
        let mut fields = line.split(':');
        if fields.next()? == name {
            // name:passwd:uid:gid:gecos:home:shell
            let _passwd = fields.next()?;
            let _uid = fields.next()?;
            let _gid = fields.next()?;
            let _gecos = fields.next()?;
            return fields.next().map(str::to_string);
        }
    }
    None
}

#[cfg(unix)]
#[test]
fn issue1657_known_system_users_expand_to_their_passwd_home() {
    let s = Scratch::new("systemusers");
    // root is present in every unix passwd database; daemon/bin are
    // the reported class (system users a hand-rolled list would
    // miss) — each is asserted only when the database has it.
    let mut script = String::new();
    let mut expected = String::new();
    for name in ["root", "daemon", "bin"] {
        if let Some(home) = passwd_home(name) {
            script.push_str(&format!("echo ~{name}\n"));
            expected.push_str(&format!("{home}\n"));
            script.push_str(&format!("echo ~{name}/sub\n"));
            expected.push_str(&format!("{home}/sub\n"));
        }
    }
    assert!(!script.is_empty(), "passwd database must list root");
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, expected);
    assert_eq!(err, "");
}

#[cfg(unix)]
#[test]
fn issue1657_unknown_user_stays_literal() {
    let s = Scratch::new("unknown");
    // GNU: a name getpwnam cannot resolve keeps the tilde word
    // verbatim (no fallback to `/`, no empty home).
    let (out, err, rc) = s.run("echo ~nosuchuser1657xyz\necho ~nosuchuser1657xyz/path\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "~nosuchuser1657xyz\n~nosuchuser1657xyz/path\n");
    assert_eq!(err, "");
}
