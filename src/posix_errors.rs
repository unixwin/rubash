//! GNU strerror-style text for host `io::Error` values.
//!
//! On Windows, `std::io::Error`'s Display renders the localized
//! FormatMessage text (GBK/zh-CN on the hosts GNU compatibility is
//! measured against), which leaks into shell diagnostics and breaks
//! GNU Bash output comparison. GNU Bash prints ASCII `strerror` text,
//! so every diagnostic that surfaces an `io::Error` must go through
//! [`message`] instead of raw Display.

/// Canonical diagnostic text for `err`.
///
/// OS errors are mapped to the GNU/POSIX strerror wording; errors
/// synthesized by rubash carry their own English payloads and pass
/// through unchanged.
pub fn message(err: &std::io::Error) -> String {
    match err.raw_os_error() {
        Some(code) => {
            let mapped = os_error_message(code);
            if mapped == "Unknown error" {
                // The static table only covers codes the suite has met.
                // `io::Error::to_string` runs FormatMessageW on Windows
                // (strerror on Unix), giving the real OS text plus the
                // numeric code — "Unknown error" discarded both, which hid
                // the actual CreateProcess failure behind spawn errors
                // like `sort: Unknown error` (niubash#141).
                err.to_string()
            } else {
                mapped.to_string()
            }
        }
        None => err.to_string(),
    }
}

/// GNU-shaped error for a failed operation on `target`: the payload
/// becomes `"<target>: <strerror>"` (e.g. `/etc/passwd: No such file
/// or directory`), matching how GNU Bash reports redirect failures.
/// The [`std::io::ErrorKind`] is preserved for kind-based consumers.
pub fn path_error(target: &str, err: std::io::Error) -> std::io::Error {
    // POSIX open(2) on a name no file can ever match (an unexpanded glob
    // such as `redir1.*`) reports ENOENT. Windows maps the same name to
    // ERROR_INVALID_NAME/EINVAL, so kind-based consumers and the
    // strerror text below would both diverge from GNU (redir.tests:184:
    // GNU prints "No such file or directory"). Translate EINVAL for
    // wildcard-bearing names to the ENOENT kind and text.
    // Windows reports ERROR_INVALID_NAME (123) for wildcard characters in
    // a path; older toolchains surface it as InvalidInput rather than
    // InvalidArgument, so match the raw code as well as the kind.
    let invalid_name = if cfg!(windows) {
        err.raw_os_error() == Some(123) || err.kind() == std::io::ErrorKind::InvalidInput
    } else {
        false
    };
    if invalid_name && target.chars().any(|ch| matches!(ch, '*' | '?' | '[')) {
        return std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{target}: No such file or directory"),
        );
    }
    std::io::Error::new(err.kind(), format!("{target}: {}", message(&err)))
}

/// Same mapping for a raw OS error code (Win32 on Windows, errno elsewhere).
pub fn os_error_message(code: i32) -> &'static str {
    #[cfg(windows)]
    {
        win32_message(code)
    }
    #[cfg(not(windows))]
    {
        errno_message(code)
    }
}

#[cfg(windows)]
fn win32_message(code: i32) -> &'static str {
    match code {
        1 => "Invalid argument",            // ERROR_INVALID_FUNCTION
        2 => "No such file or directory",   // ERROR_FILE_NOT_FOUND
        3 => "No such file or directory",   // ERROR_PATH_NOT_FOUND
        4 => "Too many open files",         // ERROR_TOO_MANY_OPEN_FILES
        5 => "Permission denied",           // ERROR_ACCESS_DENIED
        6 => "Bad file descriptor",         // ERROR_INVALID_HANDLE
        13 => "Invalid argument",           // ERROR_INVALID_DATA
        15 => "Invalid cross-device link",  // ERROR_NOT_SAME_DEVICE
        19 => "Read-only file system",      // ERROR_WRITE_PROTECT
        32 => "Device or resource busy",    // ERROR_SHARING_VIOLATION
        33 => "Device or resource busy",    // ERROR_LOCK_VIOLATION
        39 => "No space left on device",    // ERROR_HANDLE_DISK_FULL
        80 => "File exists",                // ERROR_FILE_EXISTS
        87 => "Invalid argument",           // ERROR_INVALID_PARAMETER
        112 => "No space left on device",   // ERROR_DISK_FULL
        123 => "Invalid argument",          // ERROR_INVALID_NAME
        126 => "No such file or directory", // ERROR_MOD_NOT_FOUND
        127 => "No such file or directory", // ERROR_PROC_NOT_FOUND
        131 => "Invalid argument",          // ERROR_NEGATIVE_SEEK
        145 => "Directory not empty",       // ERROR_DIR_NOT_EMPTY
        183 => "File exists",               // ERROR_ALREADY_EXISTS
        193 => "Exec format error",         // ERROR_BAD_EXE_FORMAT
        206 => "File name too long",        // ERROR_FILENAME_EXCED_RANGE
        232 => "Broken pipe",               // ERROR_NO_DATA
        267 => "Not a directory",           // ERROR_DIRECTORY
        _ => "Unknown error",
    }
}

#[cfg(not(windows))]
fn errno_message(code: i32) -> &'static str {
    match code {
        1 => "Operation not permitted",            // EPERM
        2 => "No such file or directory",          // ENOENT
        4 => "Interrupted system call",            // EINTR
        5 => "Input/output error",                 // EIO
        9 => "Bad file descriptor",                // EBADF
        11 => "Resource temporarily unavailable",  // EAGAIN
        12 => "Cannot allocate memory",            // ENOMEM
        13 => "Permission denied",                 // EACCES
        16 => "Device or resource busy",           // EBUSY
        17 => "File exists",                       // EEXIST
        18 => "Invalid cross-device link",         // EXDEV
        20 => "Not a directory",                   // ENOTDIR
        21 => "Is a directory",                    // EISDIR
        22 => "Invalid argument",                  // EINVAL
        24 => "Too many open files",               // EMFILE
        27 => "File too large",                    // EFBIG
        28 => "No space left on device",           // ENOSPC
        30 => "Read-only file system",             // EROFS
        32 => "Broken pipe",                       // EPIPE
        36 => "File name too long",                // ENAMETOOLONG
        39 => "Directory not empty",               // ENOTEMPTY
        40 => "Too many levels of symbolic links", // ELOOP
        _ => "Unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_codes_map_to_gnu_strerror_text() {
        use std::io::Error;
        assert_eq!(
            message(&Error::from_raw_os_error(2)),
            "No such file or directory"
        );
        assert_eq!(
            message(&Error::from_raw_os_error(3)),
            "No such file or directory"
        );
        assert_eq!(message(&Error::from_raw_os_error(80)), "File exists");
        assert_eq!(message(&Error::from_raw_os_error(5)), "Permission denied");
        assert_eq!(
            message(&Error::from_raw_os_error(206)),
            "File name too long"
        );
    }

    #[test]
    fn synthesized_payloads_pass_through() {
        let err = std::io::Error::new(std::io::ErrorKind::Other, "pipeline stderr reader panicked");
        assert_eq!(message(&err), "pipeline stderr reader panicked");
    }

    #[cfg(windows)]
    #[test]
    fn unmapped_windows_code_keeps_os_text_and_number() {
        // niubash#141: an unmapped CreateProcess failure must surface the
        // OS message and the raw code, not collapse to "Unknown error".
        let text = message(&std::io::Error::from_raw_os_error(9999));
        assert!(text.contains("9999"), "raw code must survive, got {text:?}");
        assert_ne!(text, "Unknown error");
    }

    #[test]
    fn no_localized_text_survives_os_errors() {
        let err = std::io::Error::from_raw_os_error(2);
        let text = message(&err);
        assert!(
            text.is_ascii(),
            "os error text must be ASCII strerror wording, got {text:?}"
        );
    }
}
