//! Issue unixwin/rubash#2211: `cd` inside an un-redirected command
//! substitution resolved relative operands against the STARTUP cwd
//! after the shell had cd'd.
//!
//! GNU keeps exactly one cwd: cd moves the process's
//! (builtins/cd.def change_to_directory -> chdir), and the child forked
//! by subst.c:7143 command_substitute inherits it, so a relative
//! operand inside $( ) resolves against the directory the shell most
//! recently cd'd to. Rubash's G-02 split makes the SESSION cwd field
//! the physical truth (cd moves only the field; every spawn passes it
//! via Command::current_dir), but the two in-process collect-path
//! shortcuts resolved bare filesystem operands against the PROCESS
//! cwd, which never moves after startup:
//! - the `$(cd X && pwd)` fast path canonicalized the bare target, and
//! - the `$(<file)` quick read opened the bare operand.
//! After any earlier cd both therefore named (or failed to find) the
//! startup tree, while the redirected and plain-subshell forms — which
//! run the real builtin in the cloned executor — resolved correctly.
//!
//! The same cluster corrects the fast path's admission: it computes a
//! pure pathname resolution, so operand forms whose GNU resolution is
//! not one decline to the real executor (leading `-` = cd options /
//! the OLDPWD keyword with its printed destination, cd.def:341-352;
//! CDPATH-eligible names while CDPATH carries a nonempty unit,
//! cd.def step 5; operands expanding to empty, cd.def:331-336; and, at
//! the failure arms, bare names cdable_vars could still claim,
//! cd.def LCD_DOVARS). And it fixes the $PWD double-slash seam: the
//! exactly-two-slashes evidence for a logical destination is the
//! spelling the path was built FROM, not the joined text — joining
//! "/" + "etc" manufactures a `//` neither side spelled, and joining
//! "//" + "etc" manufactures a collapsible `///` (GNU: cd /; cd etc
//! binds /etc, cd //; cd etc binds //etc).
//!
//! All expectations measured against GNU bash 5.3.20 (owner-compiled
//! oracle) with script files, 2026-10-30, master 80bf1950. Controls
//! (redirected forms, plain subshell, startup cwd) pass on master too;
//! the pins named for the defect shapes are red on master.
//!
//! Follow-up (same day): the anchor regressed the golden
//! catfile-comsub p3/q9 shapes — a quoted empty operand joined onto
//! the session cwd manufactured the directory itself and read as
//! silent rc 0, where GNU's open of the word as spelled fails ENOENT
//! (builtins/evalstring.c open_redir_file). The
//! `issue2211_comsub_cat_file_quoted_empty_operand` pin covers that
//! class; it is red on the anchored code, and on un-fixed master it
//! passes only by accident (the un-anchored open of "" fails alone).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Fixture {
    root: PathBuf,
    launch: PathBuf,
    base_a: PathBuf,
    base_b: PathBuf,
    cdroot: PathBuf,
    home: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2211_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let launch = root.join("launch");
    let base_a = root.join("alpha").join("base");
    let base_b = root.join("deep").join("deeper").join("base");
    let cdroot = root.join("cdroot");
    let home = root.join("home");
    for dir in [
        &launch,
        &base_a,
        &base_b,
        &cdroot,
        &home,
        &base_a.join("sub"),
        &base_b.join("sub"),
        &cdroot.join("via"),
    ] {
        std::fs::create_dir_all(dir).expect("create fixture dir");
    }
    std::fs::write(base_a.join("relfile.txt"), "RELATIVE-DATA\n").expect("write relfile");
    // pwd -P expectations are physical paths: canonicalize the root so
    // a symlinked temp dir cannot skew the comparison.
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture root");
    let launch = root.join("launch");
    let base_a = root.join("alpha").join("base");
    let base_b = root.join("deep").join("deeper").join("base");
    let cdroot = root.join("cdroot");
    let home = root.join("home");
    Fixture {
        root,
        launch,
        base_a,
        base_b,
        cdroot,
        home,
    }
}

impl Fixture {
    fn run(&self, script: &str, launch: &Path) -> (String, String, Option<i32>) {
        let script_path = self.root.join("probe.sh");
        std::fs::write(&script_path, script).expect("write probe script");
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script_path)
            .current_dir(launch)
            .env("BASE_A", &self.base_a)
            .env("BASE_B", &self.base_b)
            .env("CDROOT", &self.cdroot)
            .env("HOME", &self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        let out = child.wait_with_output().expect("wait rubash");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn issue2211_comsub_cd_uses_current_cwd_after_prior_cd() {
    let fx = fixture("minimal");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"and: [$(cd sub && pwd -P)]\"
echo \"logical: [$(cd sub && pwd)]\"
echo \"semicolon: [$(cd sub; pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let want = display(&fx.base_a.join("sub"));
    assert_eq!(
        stdout,
        format!("and: [{want}]\nlogical: [{want}]\nsemicolon: [{want}]\n"),
        "stderr: {stderr}"
    );
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_redirected_controls() {
    let fx = fixture("redirected");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"err: [$(cd sub 2>/dev/null && pwd -P)]\"
echo \"out: [$(cd sub >/dev/null && pwd -P)]\"
echo \"in: [$(cd sub </dev/null && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let want = display(&fx.base_a.join("sub"));
    assert_eq!(
        stdout,
        format!("err: [{want}]\nout: [{want}]\nin: [{want}]\n"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2211_comsub_cd_plain_subshell_control() {
    let fx = fixture("subshell");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"subshell: [$( (cd sub && pwd -P) )]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let want = display(&fx.base_a.join("sub"));
    assert_eq!(stdout, format!("subshell: [{want}]\n"), "stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_at_startup_cwd_control() {
    let fx = fixture("startup");
    // The shell process starts inside BASE_A: no prior cd at all.
    let script = "echo \"minimal: [$(cd sub && pwd -P)]\"\n";
    let (stdout, stderr, _) = fx.run(script, &fx.base_a);
    let want = display(&fx.base_a.join("sub"));
    assert_eq!(stdout, format!("minimal: [{want}]\n"), "stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_follows_latest_cwd() {
    let fx = fixture("latest");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"in-a: [$(cd sub && pwd -P)]\"
cd \"$BASE_B\" || exit 1
echo \"in-b: [$(cd sub && pwd -P)]\"
echo \"in-b-dot: [$(cd . && pwd -P)]\"
echo \"in-b-dotdot: [$(cd .. && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let a_sub = display(&fx.base_a.join("sub"));
    let b_sub = display(&fx.base_b.join("sub"));
    let b = display(&fx.base_b);
    let b_parent = display(fx.base_b.parent().expect("base_b parent"));
    assert_eq!(
        stdout,
        format!("in-a: [{a_sub}]\nin-b: [{b_sub}]\nin-b-dot: [{b}]\nin-b-dotdot: [{b_parent}]\n"),
        "stderr: {stderr}"
    );
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_dash_uses_oldpwd() {
    let fx = fixture("dash");
    let script = "\
cd \"$BASE_A\" || exit 1
cd \"$BASE_B\" || exit 1
echo \"dash: [$(cd - && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let a = display(&fx.base_a);
    // cd - prints the destination into the capture, then pwd -P prints
    // it again (builtins/cd.def:341-352 LCD_PRINTPATH).
    assert_eq!(stdout, format!("dash: [{a}\n{a}]\n"), "stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_bare_flag_is_home_move() {
    let fx = fixture("bareflag");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"bare-P: [$(cd -P && pwd)]\"
echo \"bare-L: [$(cd -L && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let home = display(&fx.home);
    assert_eq!(
        stdout,
        format!("bare-P: [{home}]\nbare-L: [{home}]\n"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2211_comsub_cd_cdpath_hit() {
    let fx = fixture("cdpath");
    let script = "\
cd \"$BASE_A\" || exit 1
CDPATH=\"$CDROOT\"
echo \"hit: [$(cd via && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let via = display(&fx.cdroot.join("via"));
    // A CDPATH hit announces the found path, then pwd -P repeats it.
    assert_eq!(stdout, format!("hit: [{via}\n{via}]\n"), "stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_cdable_vars_hit() {
    let fx = fixture("cdable");
    let script = "\
cd \"$BASE_A\" || exit 1
shopt -s cdable_vars
dest=\"$CDROOT/via\"
echo \"hit: [$(cd dest && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let via = display(&fx.cdroot.join("via"));
    assert_eq!(stdout, format!("hit: [{via}\n{via}]\n"), "stderr: {stderr}");
}

#[test]
fn issue2211_comsub_cd_empty_expansion_operands() {
    let fx = fixture("empty");
    let script = "\
cd \"$BASE_A\" || exit 1
e=
echo \"unquoted: [$(cd $e && pwd -P)]\"
echo \"quoted: [$(cd \"$e\" && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    let home = display(&fx.home);
    // Unquoted empty expansion removes the operand (the $HOME move);
    // a quoted empty word is the "null directory" error (cd.def:331-336).
    assert_eq!(
        stdout,
        format!("unquoted: [{home}]\nquoted: []\n"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("cd: null directory"),
        "expected null-directory diagnostic, got: {stderr}"
    );
}

#[test]
fn issue2211_comsub_cd_failure_arms_unchanged() {
    let fx = fixture("failure");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"missing: [$(cd nosuch && pwd -P)]\"
echo \"file: [$(cd relfile.txt && pwd -P)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    assert_eq!(stdout, "missing: []\nfile: []\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cd: nosuch: No such file or directory"),
        "missing-target diagnostic drifted: {stderr}"
    );
    assert!(
        stderr.contains("cd: relfile.txt: Not a directory"),
        "not-a-directory diagnostic drifted: {stderr}"
    );
}

#[test]
fn issue2211_comsub_cat_file_quoted_empty_operand() {
    // The catfile-comsub golden's p3/q9 shapes (follow-up to the
    // session-cwd anchor above): a QUOTED empty operand is one empty
    // word, and GNU's open_redir_file (builtins/evalstring.c:759)
    // opens it as spelled — open("") fails ENOENT, reported as
    // ": No such file or directory" with EXECUTION_FAILURE. The
    // anchor must not manufacture the session directory out of it:
    // joining "" onto the session cwd names the directory itself,
    // whose read model is silent rc 0. The unquoted empty expansion
    // (p6) is a different arm — zero words, the ambiguous redirect —
    // and a nonempty relative operand still reads through the anchor.
    // On the anchored-but-unfixed code the three quoted-empty rows
    // report rc=0 with no diagnostic; on un-fixed master they pass
    // only because the un-anchored open of "" fails on its own.
    let fx = fixture("catempty");
    let script = "\
cd \"$BASE_A\" || exit 1
unset uf
y=$(< \"$uf\"); echo \"p3 rc=$?\"
x=$(< \"\"); echo \"q9 rc=$?\"
e=
y=$(< \"$e\"); echo \"varq rc=$?\"
y=$(< $uf); echo \"p6 rc=$?\"
z=$(< relfile.txt); echo \"ctrl [$z] rc=$?\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    assert_eq!(
        stdout, "p3 rc=1\nq9 rc=1\nvarq rc=1\np6 rc=1\nctrl [RELATIVE-DATA] rc=0\n",
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches(": No such file or directory").count(),
        3,
        "quoted-empty diagnostics drifted: {stderr}"
    );
    assert!(
        stderr.contains("$uf: ambiguous redirect"),
        "unquoted-empty diagnostic drifted: {stderr}"
    );
}

#[test]
fn issue2211_comsub_cat_file_relative_operand() {
    let fx = fixture("catfile");
    let script = "\
cd \"$BASE_A\" || exit 1
echo \"quick: [$(<relfile.txt)]\"
echo \"plain: [$(cat relfile.txt)]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    assert_eq!(
        stdout, "quick: [RELATIVE-DATA]\nplain: [RELATIVE-DATA]\n",
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn issue2211_pwd_double_slash_only_from_spelling() {
    let fx = fixture("dslash");
    let script = "\
cd /
echo \"root-plus: [$(cd etc && pwd -P)] PWD=[$PWD]\"
cd etc
echo \"root-join: PWD=[$PWD]\"
cd /
cd ..
echo \"root-up: PWD=[$PWD]\"
cd //
echo \"dslash: PWD=[$PWD]\"
cd etc
echo \"dslash-join: PWD=[$PWD]\"
cd //etc
echo \"dslash-abs: PWD=[$PWD]\"
cd ///
echo \"tslash: PWD=[$PWD]\"
";
    let (stdout, stderr, _) = fx.run(script, &fx.launch);
    assert_eq!(
        stdout,
        "root-plus: [/etc] PWD=[/]\n\
         root-join: PWD=[/etc]\n\
         root-up: PWD=[/]\n\
         dslash: PWD=[//]\n\
         dslash-join: PWD=[//etc]\n\
         dslash-abs: PWD=[//etc]\n\
         tslash: PWD=[/]\n",
        "stderr: {stderr}"
    );
}
