//! `:`, `true`, and `false` builtins.
//!
//! GNU Bash source ownership:
// - builtins/colon.def

/// Exit status for the `:` and `true` builtins.
pub const SUCCESS: i32 = 0;

/// Exit status for the `false` builtin.
pub const FAILURE: i32 = 1;

pub fn colon() -> i32 {
    SUCCESS
}

pub fn true_builtin() -> i32 {
    SUCCESS
}

pub fn false_builtin() -> i32 {
    FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_always_succeeds() {
        assert_eq!(colon(), SUCCESS);
    }

    #[test]
    fn true_always_succeeds() {
        assert_eq!(true_builtin(), SUCCESS);
    }

    #[test]
    fn false_always_fails() {
        assert_eq!(false_builtin(), FAILURE);
    }
}
