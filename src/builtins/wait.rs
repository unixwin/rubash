//! wait module.
//!
//! GNU Bash source ownership:
// - builtins/wait.def

/// rubash#2898: how an all-digit wait operand classifies under GNU's
/// rule (builtins/wait.def:272-291): a digit-leading word is a pid
/// only when `valid_number` succeeds AND the value round-trips
/// through `pid_t` (a 32-bit int) unchanged — `pid_value ==
/// (pid_t)pid_value`. A value above INT_MAX (2147483648) or a digit
/// string too long for intmax is NOT a pid at all: wait reports it
/// with sh_badpid (`` `N': not a pid or valid job spec ``) and returns
/// EXECUTION_FAILURE at once. An in-range value is the pid — and the
/// pid is what wait_for_single_pid prints in its not-a-child report
/// (jobs.c:2753), so `wait 007` names `pid 7`, never the raw word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitPidClass {
    /// Not an all-digit operand (jobspec / other text forms).
    NotPid,
    /// All digits, in pid_t range: the parsed pid value.
    Pid(u32),
    /// All digits but outside pid_t range (or beyond intmax).
    BadPid,
}

pub(crate) fn classify_wait_pid_operand(operand: &str) -> WaitPidClass {
    if operand.is_empty() || !operand.bytes().all(|byte| byte.is_ascii_digit()) {
        return WaitPidClass::NotPid;
    }
    match operand.parse::<u64>() {
        Ok(value) if value <= i32::MAX as u64 => WaitPidClass::Pid(value as u32),
        _ => WaitPidClass::BadPid,
    }
}
