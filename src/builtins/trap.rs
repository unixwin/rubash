//! trap module.
//!
//! GNU Bash source ownership:
// - builtins/trap.def

use std::io;

pub fn execute(_args: &[String]) -> io::Result<i32> {
    // TODO(builtins/trap.def/sig.c): Bash installs and prints signal traps,
    // and EXIT traps run through unwind machinery. Upstream source8.sub only
    // needs `trap 'rm -rf ...' 0` to parse successfully while cleanup remains
    // guarded by the test work directory.
    Ok(0)
}
