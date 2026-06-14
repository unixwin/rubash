//! command module.
//!
//! GNU Bash source ownership:
// - execute_cmd.c
// - execute_cmd.h

pub(crate) fn is_reserved_word(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "while"
            | "do"
            | "done"
            | "until"
            | "for"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "select"
            | "time"
            | "coproc"
    )
}

pub(crate) fn arithmetic_command_words(words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    if words
        .first()
        .is_some_and(|word| is_shell_builtin_name(word) || is_reserved_word(word))
    {
        return false;
    }
    words.len() > 1
        || words.iter().any(|word| {
            word.bytes().any(|byte| {
                matches!(
                    byte,
                    b'[' | b']'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'<'
                        | b'>'
                        | b'='
                        | b'!'
                        | b'&'
                        | b'|'
                        | b'^'
                        | b'~'
                        | b'?'
                )
            })
        })
}

pub(crate) fn arithmetic_command_expr(words: &[String]) -> Option<String> {
    let word = words.first()?;
    if words.len() != 1 {
        return None;
    }
    word.trim()
        .strip_prefix("((")
        .and_then(|expr| expr.strip_suffix("))"))
        .map(|expr| expr.trim().to_string())
}

pub(crate) fn is_shell_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "." | ":"
            | "["
            | "alias"
            | "bg"
            | "bind"
            | "break"
            | "builtin"
            | "caller"
            | "cd"
            | "command"
            | "compgen"
            | "complete"
            | "compopt"
            | "continue"
            | "declare"
            | "dirs"
            | "disown"
            | "echo"
            | "enable"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "false"
            | "fc"
            | "fg"
            | "getopts"
            | "hash"
            | "help"
            | "history"
            | "jobs"
            | "kill"
            | "let"
            | "local"
            | "logout"
            | "mapfile"
            | "popd"
            | "printf"
            | "pushd"
            | "pwd"
            | "read"
            | "readarray"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "shopt"
            | "source"
            | "suspend"
            | "test"
            | "times"
            | "trap"
            | "true"
            | "type"
            | "typeset"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "wait"
    )
}
