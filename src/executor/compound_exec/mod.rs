mod background;
mod case;
mod control_flow;
mod coproc;
mod subshell;
mod transport;

use std::fs::File;

/// Resolved stdio disposition for one of a background child's fds 0-2,
/// tracking what GNU's do_redirections would leave the descriptor
/// pointing at inside the forked subshell.
pub(in crate::executor) enum BackgroundStdio {
    /// fd keeps the parent's handle (GNU: the subshell inherited it).
    Inherit,
    /// fd is /dev/null (async stdin default, `>&-`, `<&-`, `>/dev/null`).
    Null,
    /// fd is an open file.
    File(File),
}

impl Clone for BackgroundStdio {
    fn clone(&self) -> Self {
        match self {
            Self::Inherit => Self::Inherit,
            Self::Null => Self::Null,
            Self::File(file) => file.try_clone().map(Self::File).unwrap_or(Self::Inherit),
        }
    }
}

pub(in crate::executor) use case::{quoted_case_pattern_end, skip_braced_case_pattern_unit};
pub(in crate::executor) use control_flow::{
    command_is_time_prefixed_compound, compound_command_has_redirects, posix_time_dash_word,
    time_dash_word_not_keyword,
};
pub(in crate::executor) use transport::{
    decode_function_line_spans, decode_root_line_spans, encode_root_line_spans,
    first_leaf_command_line, fork_respawn_transport_key, respawn_source_needs_file,
    rubash_spawn_inherited_state, ExpansionProbeGuard, FORK_ARGC_TRANSPORT,
    FORK_ARGV_BOTTOM_TRANSPORT, FORK_ARGV_TRANSPORT, FORK_BASH_ARGV_INITIALIZED_TRANSPORT,
    FORK_FN_SPANS_TRANSPORT, FORK_FUNCNAME_TRANSPORT, FORK_IN_FUNCTION_TRANSPORT,
    FORK_LINENO_TRANSPORT, FORK_LOGIN_SHELL_TRANSPORT, FORK_MAIN_FRAME_TRANSPORT,
    FORK_PIPESTATUS_TRANSPORT, FORK_ROOT_SPANS_TRANSPORT, FORK_SOURCE_TRANSPORT,
    FORK_TRANSPORT_KEYS, LOGICAL_CWD_TRANSPORT, POSITIONAL_PARAMS_TRANSPORT,
    SUPPRESS_ERREXIT_TRANSPORT,
};
pub(crate) use transport::{test_rubash_binary_from_current_exe, test_rubash_binary_from_env};
