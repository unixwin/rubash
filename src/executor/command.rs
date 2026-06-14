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

pub(crate) fn empty_quoted_index_lvalue(expr: &str) -> bool {
    // TODO(execute_cmd.c/expr.c/arrayfunc.c): Bash diagnoses empty quoted
    // indexed-array lvalues in arithmetic commands/expansions before binding
    // the element, while `let 'a[""]=1'` still reaches evalexp. Keep this
    // context-specific until the parser has ARITH_COM/ARITH_FOR_EXPRS nodes.
    let bytes = expr.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !is_name_start(bytes[index]) {
            index += 1;
            continue;
        }
        index += 1;
        while index < bytes.len() && is_name_char(bytes[index]) {
            index += 1;
        }
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) != Some(&b'[') {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let Some(quote @ (b'"' | b'\'')) = bytes.get(index).copied() else {
            continue;
        };
        index += 1;
        if bytes.get(index) != Some(&quote) {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) != Some(&b']') {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) == Some(&b'=') {
            return true;
        }
    }
    false
}

fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_name_char(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
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
