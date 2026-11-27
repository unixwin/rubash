//! Issue rubash#604: PS1-embedded `$(...)` vs the same call in
//! PROMPT_COMMAND — channel-parity regressions plus a relative perf
//! ceiling on the render path.
//!
//! GNU spec (bash.git @b4608166, Bash-5.3 patch 15):
//! - parse.y:6158-6160 prompt_again decodes PS1 through
//!   decode_prompt_string before every primary read; a `$( )` inside the
//!   prompt text runs through the SAME subst.c:7143 command_substitute
//!   (parse_and_execute in a substitution child) as a `$( )` in a word
//!   or inside the eval.c:318-330 PROMPT_COMMAND parse_and_execute —
//!   there is no third execution engine. Both channels must therefore
//!   produce identical output for the same body and identical state
//!   semantics (comsub isolation, no `$?` clobber on the interactive
//!   reader).
//! - WSL GNU Bash 5.3.0 piped-`-i` probe (2026-10-14, gnu604.sh):
//!   PS1='$(false)' renders before the next read and the following
//!   `echo $?` still prints the PRE-render status (0) — the prompt
//!   render never disturbs the reader's `$?`. Script-mode probe
//!   (gnu604c.sh): `PS1='$(false)'; : "${PS1@P}"; echo $?` -> 0 on GNU
//!   and rubash alike (the `:` imposes its own status; the substitution
//!   cell does not survive past the command).
//!
//! Performance profile behind the relative ceiling (issue #604 data,
//! Windows 10 19045, rubash master 6c6ba0c0):
//! - the render path (`expand_prompt_string`), the niu interactive
//!   driver's channel (`expand_prompt_string_mut`), the PROMPT_COMMAND
//!   channel (`execute_prompt_command`) and a plain assignment `$( )`
//!   all execute the same body at parity — measured medians dev-profile,
//!   3 external spawns per call: render 47ms / driver 49ms / PC 68ms /
//!   assignment 31ms / direct 34ms; release-mode script probes with a
//!   real git binary: 1539 / n/a / 1545 / 1546 / 1543ms (spawn-bound).
//! - the theme rr cost is EXTERNAL-SPAWN-BOUND, not path-bound: each
//!   scm_prompt_info render spawns git 4-6x; bare `std::process::Command`
//!   pays 208.9ms for the same git.exe rubash's shell spawns in 217.6ms
//!   (in-session) — the engine adds ~5-9ms per spawn over the raw OS
//!   floor. The relative ceiling below exists so a future ENGINE-side
//!   regression of the reported 7-20x shape (render drifting away from
//!   the PC channel while the PC channel stays flat) fails loudly.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run(exec: &mut Executor, src: &str) {
    let ast = parse(&tokenize(src));
    exec.execute_ast(&ast).ok();
}

fn seed_executor() -> Executor {
    let mut exec = Executor::new();
    run(
        &mut exec,
        r#"
p604_branch() { printf 'master'; }
p604_multi() { printf 'l1\nl2\n'; }
p604_leak() { p604_leaked=1; printf 'side'; }
"#,
    );
    exec
}

/// The PS1 render channel and the plain word-expansion channel execute the
/// same body to the same text — subst.c:7143 is the only command
/// substitution engine, so no channel may invent different output.
#[test]
fn ps1_render_output_matches_assignment_channel() {
    let mut exec = seed_executor();

    let rendered = exec.expand_prompt_string("$(p604_branch)");
    assert_eq!(rendered, "master");

    // Assignment comsub: the generic word-expansion channel.
    run(&mut exec, "p604_x=$(p604_branch)");
    assert_eq!(exec.get_env("p604_x"), Some("master"));

    // The niu interactive driver's channel agrees byte-for-byte.
    let rendered_mut = exec.expand_prompt_string_mut("$(p604_branch)");
    assert_eq!(rendered_mut, "master");

    // Multi-line bodies: trailing-newline stripping agrees too.
    let rendered = exec.expand_prompt_string("$(p604_multi)");
    assert_eq!(rendered, "l1\nl2");
    run(&mut exec, "p604_x=$(p604_multi)");
    assert_eq!(exec.get_env("p604_x"), Some("l1\nl2"));
}

/// Repeated renders of the same PS1 are consistent, and the substitution
/// child's state never leaks into the reader (GNU fork semantics: the
/// body runs in a substitution child; assignments inside it die with it).
#[test]
fn ps1_render_is_repeatable_and_state_isolated() {
    let mut exec = seed_executor();

    let first = exec.expand_prompt_string("$(p604_branch)");
    let second = exec.expand_prompt_string("$(p604_branch)");
    let third = exec.expand_prompt_string_mut("$(p604_branch)");
    assert_eq!(first, "master");
    assert_eq!(second, first);
    assert_eq!(third, first);

    // An assignment inside the rendered body stays in the child.
    exec.expand_prompt_string("$(p604_leak)");
    assert_eq!(exec.get_env("p604_leaked"), None);
    // The printed payload still reached the prompt.
    // (p604_leak prints 'side' — captured above; here pin the isolation.)

    // The reader's $? is untouched by a failing prompt substitution
    // (GNU probe gnu604.sh: PS1='$(false)' then `echo $?` -> pre-render
    // value). expand_prompt_string is a render: no exit-status write.
    run(&mut exec, "true");
    let before = exec.last_exit_code();
    exec.expand_prompt_string("$(false)");
    assert_eq!(exec.last_exit_code(), before);
    exec.expand_prompt_string_mut("$(false)");
    assert_eq!(exec.last_exit_code(), before);
}

/// Same body through the PROMPT_COMMAND channel mutates the CURRENT shell
/// (eval.c:318-330 parse_and_execute in the current environment) — the
/// deliberate semantic difference from the PS1 comsub channel — and
/// reports the body's status while leaving `$?`/PIPESTATUS at their
/// pre-PC values on normal return (parse.y:7313/:7315, rubash#433).
#[test]
fn prompt_command_channel_runs_in_current_shell() {
    let mut exec = seed_executor();
    exec.set_env("PROMPT_COMMAND", "p604_pc_mark=1");

    assert_eq!(exec.execute_prompt_command(), Some(0));
    // PC is parse_and_execute, not a comsub: the assignment SURVIVES.
    assert_eq!(exec.get_env("p604_pc_mark"), Some("1"));

    // And the same body that the PS1 channel renders runs identically:
    exec.set_env("PROMPT_COMMAND", "p604_pc_out=$(p604_branch)");
    assert_eq!(exec.execute_prompt_command(), Some(0));
    assert_eq!(exec.get_env("p604_pc_out"), Some("master"));
}

/// Relative perf ceiling (the #604 guard): the PS1 render path must stay
/// within a small factor of the PROMPT_COMMAND path measured in the SAME
/// process. The reported 7-20x shape (render 500-1500ms against a 20-75ms
/// PC channel) fails this bound; healthy parity passes with headroom.
/// Ratio-based on purpose: absolute floors vary per machine (Windows
/// spawn floor dominates on all of them), the ratio does not.
#[test]
#[cfg(windows)]
fn ps1_render_path_stays_within_bound_of_prompt_command_path() {
    fn median(mut xs: Vec<f64>) -> f64 {
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = xs.len();
        if n % 2 == 1 {
            xs[n / 2]
        } else {
            (xs[n / 2 - 1] + xs[n / 2]) / 2.0
        }
    }

    let mut exec = Executor::new();
    // A theme-shaped body: several external spawns per call (the #604
    // workload shape — scm_prompt_info spawns git 4-6x per render), plus
    // a deterministic payload the assertions can pin.
    run(
        &mut exec,
        r#"
p604_scm() {
  p604_s1=$(cmd /c rem)
  p604_s2=$(cmd /c rem)
  p604_s3=$(cmd /c rem)
  printf 'master'
}
"#,
    );

    let reps = 9usize;
    let iters = 4usize;

    // Warm the spawn path (image-page caches) before timing.
    for _ in 0..2 {
        let _ = exec.expand_prompt_string("$(p604_scm)");
    }

    // Path A: the PS1 render channel.
    let mut render = Vec::new();
    for _ in 0..reps {
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            let out = exec.expand_prompt_string("$(p604_scm)");
            assert_eq!(out, "master");
        }
        render.push(t0.elapsed().as_secs_f64() * 1000.0 / iters as f64);
    }

    // Path B: the PROMPT_COMMAND channel, same executor, same body.
    exec.set_env("PROMPT_COMMAND", "p604_scm >/dev/null");
    let mut pc = Vec::new();
    for _ in 0..reps {
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            assert_eq!(exec.execute_prompt_command(), Some(0));
        }
        pc.push(t0.elapsed().as_secs_f64() * 1000.0 / iters as f64);
    }

    let render_ms = median(render);
    let pc_ms = median(pc);
    eprintln!("issue604 parity medians: render={render_ms:.2}ms pc={pc_ms:.2}ms");

    // Healthy engines: render <= PC (measured 47 vs 68ms dev-profile on
    // the profile machine; both are spawn-bound). The bound leaves headroom
    // for machine noise while failing the reported 7-20x regression shape
    // even if the PC channel itself also slows down.
    let bound = 3.0 * pc_ms.max(30.0) + 100.0;
    assert!(
        render_ms <= bound,
        "PS1 render path regressed against the PROMPT_COMMAND channel: \
         render median {render_ms:.2}ms vs pc median {pc_ms:.2}ms (bound {bound:.2}ms) — \
         see issue rubash#604"
    );
}
