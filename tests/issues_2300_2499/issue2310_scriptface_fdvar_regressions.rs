//! rubash#2310 — the same-exe script face (execute_direct_shell_
//! script: `rubash child.sh ...` run in-process as a fresh shell)
//! serviced a command's `{var}` redirects against the AMBIENT fd
//! table instead of the command's own redirect list at each entry's
//! position. GNU's walk is one ordered pass: do_redirections
//! (redir.c:237) applies every element of the list to the live
//! descriptor table in order, a REDIR_VARASSIGN dup allocates with
//! fcntl(F_DUPFD) against the table as bound so far
//! (redir.c:1144-1150) and copies the descriptor as bound at that
//! walk position (redir.c:1193), and a `{var}<&-` close closes the
//! descriptor the variable holds at its position
//! (redir.c:1239-1248). The funnel (apply_dynamic_fd_var_
//! redirects) resolved dups before any of the command's own binds
//! existed and only RECORDED closes, and nothing on this face
//! revisited either — so `rubash obs.sh {v}<&- 9<f9 {w}<&9`
//! delivered the ambient seed at the {w} slot instead of the f9
//! content GNU delivers, and a `{v}<&-` close never reached the
//! child at all. The spawn faces learned at-position resolution
//! with #2272/#2279; this face now binds its numbered inputs in
//! segments split at the `{var}` entries and applies each entry's
//! dup/close effect at its segment point, when the table holds
//! exactly the binds GNU's walk would have produced by then.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (the oracle
//! binary, script-file probes). The child is the nested rubash
//! binary itself (the same-exe face under test) or an ENOEXEC
//! script child observing its OWN fd table with builtin reads —
//! never an ENOEXEC script observing a spawn delivery.
//!
//! Known cross-face gap NOT pinned here: a close positioned
//! between a bind and a dup of the same descriptor
//! (`9<f9 {v}<&- {w}<&9`, v=9) makes GNU reject the command
//! (redir.c walk: the dup's source is closed at its position);
//! both rubash faces currently run the child. That is a
//! pending-close-position question for the applier, separate from
//! this issue's dup-resolution and close-delivery defects.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2310-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if name.ends_with(".sh") {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
        }
    }
}

/// Run `driver.sh` (with `@RUBASH@` replaced by this build's
/// binary path) as the outer shell's main script, stdin null.
fn run_driver(dir: &Path, driver: &str) -> (String, String, Option<i32>) {
    write(dir, "driver.sh", &driver.replace("@RUBASH@", RUBASH));
    let output = Command::new(RUBASH)
        .arg("driver.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash driver");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fixtures(dir: &Path) {
    write(dir, "seed", "SEED\n");
    write(dir, "f9", "F9\n");
    write(dir, "obs_10.sh", "b=$(cat <&10)\necho \"fd10=[$b]\"\n");
    write(dir, "obs_11.sh", "b=$(cat <&11)\necho \"fd11=[$b]\"\n");
    write(
        dir,
        "obs_9_10.sh",
        "a=$(cat <&9)\nb=$(cat <&10)\necho \"fd9=[$a] fd10=[$b]\"\n",
    );
    write(
        dir,
        "obs_9.sh",
        "if IFS= read -r -u 9 line; then echo \"fd9=[$line]\"; else echo \"fd9=CLOSED\"; fi\n",
    );
}

#[test]
fn issue2310_headline_dup_resolves_own_earlier_bind() {
    // The issue's headline shape: {w} must resolve fd 9 as bound
    // EARLIER IN THE SAME LIST (9<f9), not the ambient seed the
    // parent holds on fd 9. GNU delivers the f9 content at fd 10;
    // the parent keeps its seed binding and its own variables.
    let dir = scratch("headline");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed\nv=9\n@RUBASH@ obs_10.sh {v}<&- 9<f9 {w}<&9\necho \"parent: fd9=[$(cat <&9)] w=[${w-UNSET}] v=[$v]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "fd10=[F9]\nparent: fd9=[SEED] w=[UNSET] v=[9]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2310_headline_via_enoexec_child() {
    // The same list through the ENOEXEC entry into the same face.
    let dir = scratch("enoexec");
    fixtures(&dir);
    let (stdout, stderr, code) =
        run_driver(&dir, "exec 9<seed\nv=9\n./obs_10.sh {v}<&- 9<f9 {w}<&9\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd10=[F9]\n", "stderr: {stderr}");
}

#[test]
fn issue2310_dup_shares_offset_with_own_bind() {
    // The dup copies the descriptor as bound at its position
    // (redir.c:1193 dup2): fd 9 and fd 10 are ONE open file
    // description, so reading fd 9 drains what fd 10 would read.
    let dir = scratch("shared");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed\nv=9\n@RUBASH@ obs_9_10.sh {v}<&- 9<f9 {w}<&9\necho \"parent: fd9=[$(cat <&9)]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "fd9=[F9] fd10=[]\nparent: fd9=[SEED]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2310_dup_before_bind_keeps_ambient() {
    // Position is everything: a dup that comes BEFORE the bind in
    // the same list resolves against the ambient table (the seed),
    // exactly as GNU's walk reaches it before 9<f9 is applied.
    let dir = scratch("beforebind");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed\n@RUBASH@ obs_10.sh {w}<&9 9<f9\necho \"w=[${w-UNSET}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd10=[SEED]\nw=[UNSET]\n", "stderr: {stderr}");
}

#[test]
fn issue2310_output_dup_resolves_own_bind() {
    // The output spelling of the dup ({w}>&9) resolves the same
    // way (redir.c r_duplicating_output shares the walk).
    let dir = scratch("outdup");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed\nv=9\n@RUBASH@ obs_10.sh {v}<&- 9<f9 {w}>&9\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd10=[F9]\n", "stderr: {stderr}");
}

#[test]
fn issue2310_dup_chain_through_numbered_dup() {
    // A numbered dup in the same list (10<&9) is itself an
    // own-list bind: {w}<&10 must see it. The ambient table has
    // no fd 10 at all, so an ambient-only resolution binds
    // nothing and the child sees the slot closed.
    let dir = scratch("chain");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(&dir, "@RUBASH@ obs_11.sh 9<f9 10<&9 {w}<&10\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd11=[F9]\n", "stderr: {stderr}");
}

#[test]
fn issue2310_close_entry_closes_for_child_only() {
    // Close control: {v}<&- with v naming the parent's fd 9
    // closes it for the child (redir.c:1239-1248) and leaves the
    // parent's binding untouched.
    let dir = scratch("close");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed\nv=9\n@RUBASH@ obs_9.sh {v}<&-\necho \"parent: fd9=[$(cat <&9)] v=[$v]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "fd9=CLOSED\nparent: fd9=[SEED] v=[9]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2310_bind_then_close_restores_parent() {
    // Bind-then-close in one list: the child sees fd 9 closed,
    // and the parent's seed binding — shadowed by the child's
    // 9<f9 bind and then closed — comes back whole.
    let dir = scratch("bindclose");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed\nv=9\n@RUBASH@ obs_9.sh 9<f9 {v}<&-\necho \"parent: fd9=[$(cat <&9)] v=[$v]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "fd9=CLOSED\nparent: fd9=[SEED] v=[9]\n",
        "stderr: {stderr}"
    );
}
