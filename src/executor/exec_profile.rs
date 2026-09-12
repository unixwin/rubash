//! Temporary per-command phase profiling (gated by RUBASH_EXEC_PROFILE=1).
//! Accumulates wall time of execute_command phases in static atomics and
//! prints a summary when the outermost execute_ast returns.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

pub static P_ENABLED: AtomicBool = AtomicBool::new(false);
pub static P_INIT: AtomicBool = AtomicBool::new(false);

pub static P_LINECMD: AtomicU64 = AtomicU64::new(0); // set_current_line + set_current_command
pub static P_HEREDOC: AtomicU64 = AtomicU64::new(0); // heredoc error checks
pub static P_SCANS: AtomicU64 = AtomicU64::new(0); // unclosed cmdsub / unterminated extglob scans
pub static P_DISPATCH: AtomicU64 = AtomicU64::new(0); // execute_initial_command_node + post-scan part
pub static P_EMPTY: AtomicU64 = AtomicU64::new(0); // execute_empty_words_command
pub static P_EXPAND: AtomicU64 = AtomicU64::new(0); // expand_command_words
pub static P_MATCMD: AtomicU64 = AtomicU64::new(0); // execute_materialized_command
pub static P_FOR_TEST: AtomicU64 = AtomicU64::new(0); // arith-for condition eval
pub static P_FOR_BODY: AtomicU64 = AtomicU64::new(0); // arith-for body
pub static P_FOR_UPDATE: AtomicU64 = AtomicU64::new(0); // arith-for update eval
pub static P_UPSTREAM: AtomicU64 = AtomicU64::new(0); // try_upstream_scripts per ast
pub static P_JOBS: AtomicU64 = AtomicU64::new(0); // coproc scan + job refresh + signal traps per command
pub static P_CHAIN: AtomicU64 = AtomicU64::new(0); // alias/time/if/pipe matcher chain per command
pub static P_TOTAL: AtomicU64 = AtomicU64::new(0); // whole execute_command
pub static P_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn ensure_init() {
    if !P_INIT.swap(true, Ordering::Relaxed) {
        if std::env::var("RUBASH_EXEC_PROFILE").as_deref() == Ok("1") {
            P_ENABLED.store(true, Ordering::Relaxed);
        }
    }
}

pub struct PhaseTimer {
    start: Instant,
    sink: &'static AtomicU64,
}

impl PhaseTimer {
    pub fn new(sink: &'static AtomicU64) -> Option<PhaseTimer> {
        if P_ENABLED.load(Ordering::Relaxed) {
            Some(PhaseTimer {
                start: Instant::now(),
                sink,
            })
        } else {
            None
        }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        let ns = self.start.elapsed().as_nanos() as u64;
        self.sink.fetch_add(ns, Ordering::Relaxed);
    }
}

pub fn print_summary() {
    if !P_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let count = P_COUNT.load(Ordering::Relaxed);
    let total = P_TOTAL.load(Ordering::Relaxed);
    let linecmd = P_LINECMD.load(Ordering::Relaxed);
    let heredoc = P_HEREDOC.load(Ordering::Relaxed);
    let scans = P_SCANS.load(Ordering::Relaxed);
    let dispatch = P_DISPATCH.load(Ordering::Relaxed);
    let empty = P_EMPTY.load(Ordering::Relaxed);
    let expand = P_EXPAND.load(Ordering::Relaxed);
    let matcmd = P_MATCMD.load(Ordering::Relaxed);
    let for_test = P_FOR_TEST.load(Ordering::Relaxed);
    let for_body = P_FOR_BODY.load(Ordering::Relaxed);
    let for_update = P_FOR_UPDATE.load(Ordering::Relaxed);
    let upstream = P_UPSTREAM.load(Ordering::Relaxed);
    let jobs = P_JOBS.load(Ordering::Relaxed);
    let chain = P_CHAIN.load(Ordering::Relaxed);
    let f = |ns: u64| format!("{:.1}ms", ns as f64 / 1_000_000.0);
    eprintln!(
        "[exec-profile] commands={count} total={} linecmd={} heredoc={} scans={} dispatch={} empty={} expand={} matcmd={} for_test={} for_body={} for_update={} upstream={} jobs={} chain={}",
        f(total),
        f(linecmd),
        f(heredoc),
        f(scans),
        f(dispatch),
        f(empty),
        f(expand),
        f(matcmd),
        f(for_test),
        f(for_body),
        f(for_update),
        f(upstream),
        f(jobs),
        f(chain)
    );
}
