use super::*;
use std::io::IsTerminal;

impl Executor {
    pub(in crate::executor) fn handle_external_file_builtins(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if !self.external_file_builtins_enabled {
            return Ok(false);
        }
        match cmd.words[0].as_str() {
            "/bin/pwd" | "/usr/bin/pwd" => {
                let mut pwd_cmd = cmd.clone();
                pwd_cmd.words[0] = "pwd".to_string();
                self.exit_code = self.execute_pwd(&pwd_cmd)?;
                Ok(true)
            }
            "mkdir" => self.external_mkdir(cmd),
            "touch" => self.external_touch(cmd),
            "chmod" => self.external_chmod(cmd),
            "cp" => self.external_cp(cmd),
            "rm" => self.external_rm(cmd),
            "rmdir" => self.external_rmdir(cmd),
            "cat" | "/bin/cat" | "/usr/bin/cat" => self.external_cat(cmd),
            "sed" => self.external_sed(cmd),
            "mkfifo" => self.external_mkfifo(cmd),
            "tty" | "/bin/tty" | "/usr/bin/tty" => self.external_tty(cmd),
            _ => Ok(false),
        }
    }

    fn external_tty(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let silent = cmd
            .words
            .iter()
            .skip(1)
            .any(|arg| arg == "-s" || arg == "--silent" || arg == "--quiet");
        let output = if std::io::stdin().is_terminal() {
            // A real tty device name is platform-specific; the non-tty case is
            // the compatibility-critical path for bashdb command input.
            "/dev/tty
"
        } else {
            "not a tty
"
        };
        if !silent {
            self.write_cat_output(cmd, output.as_bytes())?;
        }
        self.exit_code = if std::io::stdin().is_terminal() { 0 } else { 1 };
        Ok(true)
    }

    fn external_mkdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU mkdir (coreutils) parses options before operands: -p (parents,
        // already implied by create_dir_all), -m MODE which consumes a value,
        // -v verbose; `--` ends option parsing so a following `-p` is an
        // operand. Without this the flag itself became a literal directory
        // entry (`mkdir -p d` created `./-p`).
        let mut mode_value_pending = false;
        let mut no_more_flags = false;
        for word in &cmd.words[1..] {
            let expanded = self.expand_word(word);
            if !no_more_flags && !mode_value_pending && expanded == "--" {
                no_more_flags = true;
                continue;
            }
            if !no_more_flags
                && !mode_value_pending
                && expanded.starts_with('-')
                && expanded != "-"
            {
                if expanded == "-m" {
                    mode_value_pending = true;
                }
                continue;
            }
            if mode_value_pending {
                mode_value_pending = false;
                continue;
            }
            fs::create_dir_all(shell_path_to_windows(&expanded, &self.env_vars))?
                ;
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_touch(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU touch.c processes every operand independently: a failed create
        // prints `touch: cannot touch 'FILE': ...` and continues with the
        // remaining files, leaving exit status 1 (touch.c: do_touch loop).
        let mut failed = false;
        for path in &cmd.words[1..] {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            if let Err(error) = File::create(target) {
                eprintln!(
                    "{}touch: cannot touch '{}': {}",
                    self.diagnostic_prefix(),
                    expanded,
                    error
                );
                failed = true;
            }
        }
        self.exit_code = if failed { 1 } else { 0 };
        Ok(true)
    }

    fn external_cp(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU cp (coreutils cp.c) parses long and short options before and
        // between operands, and `--` ends option parsing. -r/-R/-a recurse
        // into directories, -n/--no-clobber skips files that already exist
        // (and warns that the short form is non-portable), -i/--interactive
        // prompts before overwriting, -v/--verbose prints `'src' -> 'dst', -p preserves metadata,
        // -u/--update copies only when the source is newer or the destination
        // is absent, --remove-destination unlinks the destination first, and
        // -t/--target-directory routes every operand into one directory.
        // Tolerated but unimplemented (accepted as no-ops, like cp itself):
        // -f, -l, -s, -d/-P, --parents, --backup, -S/--suffix.
        let mut recursive = false;
        let mut no_clobber = false;
        let mut interactive = false;
        let mut verbose = false;
        let mut update_only = false;
        let mut remove_dest = false;
        let mut target_dir: Option<String> = None;
        let mut no_more_flags = false;
        let mut operands: Vec<String> = Vec::new();

        let prefix = self.diagnostic_prefix();
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut words = cmd.words[1..].iter();

        while let Some(word) = words.next() {
            let expanded = self.expand_word(word);
            if no_more_flags {
                operands.push(expanded);
                continue;
            }
            if expanded == "--" {
                no_more_flags = true;
                continue;
            }
            if expanded == "-" || !expanded.starts_with('-') {
                operands.push(expanded);
                continue;
            }
            if let Some(rest) = expanded.strip_prefix("--") {
                let (name, value) = match rest.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_string())),
                    None => (rest, None),
                };
                match name {
                    "recursive" => recursive = true,
                    "archive" => recursive = true,
                    "no-clobber" => no_clobber = true,
                    "interactive" => interactive = true,
                    "verbose" => verbose = true,
                    "preserve" => {}
                    "update" => update_only = true,
                    "remove-destination" => remove_dest = true,
                    "force" | "link" | "symbolic-link" | "parents" | "backup"
                    | "no-dereference" | "no-preserve"
                    | "suffix" | "context" => {}
                    "target-directory" | "target-dir" => {
                        target_dir = value.or_else(|| {
                            words.next().map(|next| self.expand_word(next))
                        });
                    }
                    "help" => {
                        let _ = writeln!(stderr, "{}", cp_usage_text());
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                    other => {
                        let _ = writeln!(
                            stderr,
                            "{}cp: unknown option '--{}'",
                            prefix, other
                        );
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
                continue;
            }
            // Short option cluster such as -upv; -t and -S take the next
            // word as their value.
            let chars: Vec<char> = expanded[1..].chars().collect();
            let mut index = 0;
            while index < chars.len() {
                match chars[index] {
                    'r' | 'R' | 'a' => recursive = true,
                    'n' => no_clobber = true,
                    'i' => interactive = true,
                    'v' => verbose = true,
                    'p' => {}
                    'u' => update_only = true,
                    'f' | 'l' | 's' | 'd' | 'P' | 'Z' => {}
                    't' => {
                        target_dir = words.next().map(|next| self.expand_word(next));
                        break;
                    }
                    'S' => {
                        words.next();
                        break;
                    }
                    other => {
                        let _ = writeln!(
                            stderr,
                            "{}cp: invalid option -- '{}'",
                            prefix, other
                        );
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 1;
                        return Ok(true);
                    }
                }
                index += 1;
            }
        }

        if no_clobber {
            let _ = writeln!(
                stderr,
                "{}cp: warning: behavior of -n is non-portable and may change in future; use --update=none instead",
                prefix
            );
        }

        // With -t DIR every operand is a source copied into DIR, so the
        // directory becomes the trailing destination operand.
        let effective: Vec<String> = match target_dir {
            Some(dir) if !operands.is_empty() => {
                let mut all = operands.clone();
                all.push(dir);
                all
            }
            _ => operands,
        };

        // GNU copy.c rejects `cp /dev/null /dev/null` as a same-file copy
        // before any data would move.
        if effective.len() == 2
            && crate::executor::path::is_shell_null_device(&effective[0])
            && crate::executor::path::is_shell_null_device(&effective[1])
        {
            let _ = writeln!(
                stderr,
                "{}cp: '{}' and '{}' are the same file",
                prefix, effective[0], effective[1]
            );
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        if effective.len() < 2 {
            let _ = match effective.last() {
                Some(last) => writeln!(
                    stderr,
                    "{}cp: missing destination file operand after '{}'\nTry 'cp --help' for more information.",
                    prefix, last
                ),
                None => writeln!(
                    stderr,
                    "{}cp: missing file operand\nTry 'cp --help' for more information.",
                    prefix
                ),
            };
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let destination_word = &effective[effective.len() - 1];
        let destination = shell_path_to_windows(destination_word, &self.env_vars);

        // GNU null-device semantics: coreutils copy.c opens the destination
        // and the write goes nowhere, so `cp FILE /dev/null` succeeds without
        // creating anything. Windows has no device CopyFileExW can stat, so
        // model both directions explicitly.
        if effective.len() == 2
            && crate::executor::path::is_shell_null_device(destination_word)
        {
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 0;
            return Ok(true);
        }

        if effective.len() > 2 && !destination.exists() {
            let _ = writeln!(
                stderr,
                "{}cp: target '{}': No such file or directory",
                prefix, destination_word
            );
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let mut status = 0;
        for source_word in &effective[..effective.len() - 1] {
            // /dev/null as a source reads as an empty stream, so the copy
            // creates or truncates the target as an empty file (basename
            // "null" when the destination is a directory).
            if crate::executor::path::is_shell_null_device(source_word) {
                let name = source_word
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or("null")
                    .to_string();
                let target_path = if destination.is_dir() {
                    destination.join(name)
                } else {
                    destination.clone()
                };
                if let Err(error) = File::create(&target_path) {
                    status = 1;
                    let _ = writeln!(
                        stderr,
                        "{}cp: cannot create '{}': {}",
                        prefix, target_path.display(), crate::posix_errors::message(&error)
                    );
                }
                continue;
            }
            let source = shell_path_to_windows(source_word, &self.env_vars);
            let (target, target_display) = if destination.is_dir() {
                let Some(name) = source.file_name() else {
                    let _ = writeln!(
                        stderr,
                        "{}cp: missing destination file operand after '{}'",
                        prefix, source_word
                    );
                    status = 1;
                    continue;
                };
                let leaf = name.to_string_lossy().to_string();
                (
                    destination.join(name),
                    format!("{}/{}", destination_word.trim_end_matches('/'), leaf),
                )
            } else {
                (destination.clone(), destination_word.clone())
            };

            if !source.exists() {
                let _ = writeln!(
                    stderr,
                    "{}cp: cannot stat '{}': No such file or directory",
                    prefix, source_word
                );
                status = 1;
                continue;
            }
            if source.is_dir() && !recursive {
                let _ = writeln!(
                    stderr,
                    "{}cp: -r not specified; omitting directory '{}'",
                    prefix, source_word
                );
                status = 1;
                continue;
            }
            if target.exists() {
                if no_clobber {
                    continue;
                }
                if interactive {
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    stdout.clear();
                    stderr.clear();
                    if !cp_confirm_overwrite(&target_display) {
                        status = 1;
                        continue;
                    }
                }
            }
            if remove_dest {
                let _ = fs::remove_file(&target);
            }
            if update_only && target.exists() && cp_source_not_newer(&source, &target) {
                continue;
            }

            let result = if source.is_dir() {
                cp_copy_tree(&source, &target, 0)
            } else {
                fs::copy(&source, &target).map(|_| ())
            };
            match result {
                Ok(_) => {
                    if !source.is_dir() {
                        cp_set_modified_now(&target);
                    }
                    if verbose {
                        let _ = writeln!(
                            stdout,
                            "{} -> {}",
                            cp_quoted_name(source_word),
                            cp_quoted_name(&target_display)
                        );
                    }
                }
                Err(error) => {
                    let _ = writeln!(
                        stderr,
                        "{}cp: cannot create '{}': {}",
                        prefix, target.display(), crate::posix_errors::message(&error)
                    );
                    status = 1;
                }
            }
        }

        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        self.exit_code = status;
        Ok(true)
    }

    fn external_rm(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let force = cmd
            .words
            .iter()
            .skip(1)
            .any(|arg| arg.starts_with('-') && arg.contains('f'));
        let mut status = 0;
        let mut stderr = Vec::new();
        for path in cmd.words.iter().skip(1).filter(|arg| !arg.starts_with('-')) {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            let result = if target.is_dir() {
                fs::remove_dir_all(&target)
            } else {
                fs::remove_file(&target)
            };
            if let Err(error) = result {
                if !force {
                    status = 1;
                    let message = if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                    ) || (cfg!(windows)
                        && contains_windows_forbidden_posix_filename_char(&expanded))
                    {
                        "No such file or directory".to_string()
                    } else {
                        crate::posix_errors::message(&error)
                    };
                    writeln!(&mut stderr, "rm: cannot remove '{}': {message}", expanded)?;
                }
            }
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = status;
        Ok(true)
    }

    fn external_rmdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in &cmd.words[1..] {
            let _ = fs::remove_dir(shell_path_to_windows(
                &self.expand_word(path),
                &self.env_vars,
            ));
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_cat(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_word(&redirect.target);
                if let Some(fd) = redirect_target_fd(&target) {
                    if let Some(FdReadEndpoint::CoprocStdout(pid)) = self.fd_table.read_endpoint(fd)
                    {
                        if let Some(mut reader) = self.coproc_stdout_readers.remove(&pid) {
                            use std::io::Read;
                            let mut input = Vec::new();
                            reader.read_to_end(&mut input)?;
                            self.fd_table.close_input(fd);
                            self.write_cat_output(cmd, &input)?;
                            self.exit_code = 0;
                            return Ok(true);
                        }
                    }
                }
            }
        }

        if cmd.heredoc.is_some() {
            let input = self.stdin_string_for_command_mut(cmd).unwrap_or_default();
            if let Some(redirect) = &cmd.append {
                let target = self.expand_word(&redirect.target);
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(input.as_bytes())?;
                self.exit_code = 0;
                return Ok(true);
            }

            if let Some(redirect) = &cmd.redirect_out {
                let target = self.expand_word(&redirect.target);
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(input.as_bytes())?;
                self.exit_code = 0;
                return Ok(true);
            }
        }

        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            self.write_cat_output(cmd, input.as_bytes())?;
            self.exit_code = 0;
            return Ok(true);
        }

        if !cat_has_file_operands(cmd) {
            if let Some(input) = self.read_function_stdin('\0', None, false) {
                self.write_cat_output(cmd, input.as_bytes())?;
                self.exit_code = 0;
                return Ok(true);
            }
            if cmd.redirect_in.is_none()
                && cmd.heredoc.is_none()
                && cmd.here_string.is_none()
                && self.env_vars.get(INHERIT_PROCESS_STDIN).map(String::as_str) == Some("1")
            {
                return self.stream_inherited_cat(cmd);
            }
            if cmd.words.len() <= 1 {
                return Ok(false);
            }
            return Ok(false);
        }

        let mut output = Vec::new();
        for word in cat_file_operands(cmd) {
            let target = self.expand_word(word);
            match fs::read(shell_path_to_windows(&target, &self.env_vars)) {
                Ok(bytes) => output.extend(bytes),
                Err(_) => {
                    let mut stderr = Vec::new();
                    writeln!(
                        &mut stderr,
                        "{}cat: {}: No such file or directory",
                        self.diagnostic_prefix(),
                        target
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    self.exit_code = 1;
                    return Ok(true);
                }
            }
        }
        self.write_cat_output(cmd, &output)?;
        self.exit_code = 0;
        Ok(true)
    }

    fn stream_inherited_cat(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        use std::io::Read;

        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = stdin.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            self.write_cat_output(cmd, &buffer[..count])?;
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_sed(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let args = cmd.words[1..]
            .iter()
            .map(|word| self.expand_word(word))
            .collect::<Vec<_>>();
        if apply_simple_sed_args("", &args).is_none() {
            return Ok(false);
        }
        let Some(input) = self
            .stdin_string_for_command_mut(cmd)
            .or_else(|| self.read_function_stdin('\0', None, false))
            .or_else(|| self.read_inherited_process_stdin_to_string())
        else {
            return Ok(false);
        };
        let Some(output) = apply_simple_sed_args(&input, &args) else {
            return Ok(false);
        };
        self.write_cat_output(cmd, output.as_bytes())?;
        self.exit_code = 0;
        Ok(true)
    }

    fn external_mkfifo(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in &cmd.words[1..] {
            let target = shell_path_to_windows(&self.expand_word(path), &self.env_vars);
            let _ = File::create(target)?;
        }
        self.exit_code = 0;
        Ok(true)
    }
}

/// cp's help text, scoped to the forms this implementation honours.
fn cp_usage_text() -> &'static str {
    "Usage: cp [OPTION]... SOURCE DEST\n\n\
     Copy SOURCE to DEST, or multiple SOURCES to DIRECTORY.\n\n\
     Supported: -a -d -f -i -l -n -P -p -R -r -s -u -v --archive --backup\
       --force --help --interactive --link --no-clobber --no-dereference\
       --parents --preserve[=ATTR_LIST] --remove-destination --recursive\
       --suffix=SUFFIX --symbolic-link --target-directory=DIR\
       --target-dir=DIR --update\n"
}

/// cp quotes names with quotearg_colon: single quotes around the name, and
/// an embedded single quote becomes '\'' .
fn cp_quoted_name(name: &str) -> String {
    let mut out = String::new();
    out.push('\u{27}');
    for ch in name.chars() {
        if ch == '\u{27}' {
            out.push('\u{27}');
            out.push('\u{5c}');
            out.push('\u{27}');
            out.push('\u{27}');
        } else {
            out.push(ch);
        }
    }
    out.push('\u{27}');
    out
}

/// cp's overwrite prompt. EOF or anything but a leading y/Y is a refusal.
fn cp_confirm_overwrite(target_display: &str) -> bool {
    use std::io::{BufRead, Write};
    let mut prompt = String::from("cp: overwrite ");
    prompt.push_str(&cp_quoted_name(target_display));
    prompt.push_str("? ");
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(prompt.as_bytes());
    let _ = stderr.flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(0) => false,
        Ok(_) => {
            answer.trim_start().starts_with(|c: char| c == 'y' || c == 'Y')
        }
        Err(_) => false,
    }
}

/// cp without -p sets the destination's modification time to now. Rust's
/// fs::copy preserves the source's timestamps (it uses CopyFileW on
/// Windows), so the value must be reset explicitly; otherwise -u compares
/// against the stale source time instead of the copy time.
#[cfg(windows)]
fn cp_set_modified_now(target: &std::path::Path) {
    use std::ffi::c_void;

    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const INVALID_HANDLE_VALUE: usize = usize::MAX;

    extern "system" {
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *mut c_void,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: usize,
        ) -> usize;
        fn SetFileTime(
            hFile: usize,
            lpCreationTime: *const c_void,
            lpLastAccessTime: *const c_void,
            lpLastWriteTime: *const c_void,
        ) -> i32;
        fn CloseHandle(hObject: usize) -> i32;
    }

    // FILETIME counts 100-nanosecond intervals since 1601-01-01 in UTC,
    // while SystemTime is anchored at 1970-01-01. The 11644473600-second
    // offset converts between the two.
    const FILETIME_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = (duration.as_secs() + FILETIME_EPOCH_OFFSET_SECS) * 10_000_000
        + (duration.subsec_nanos() as u64) / 100;
    let mut wide: Vec<u16> = target.to_string_lossy().encode_utf16().collect();
    wide.push(0);

    let last_write: i64 = ticks as i64;
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        if handle != INVALID_HANDLE_VALUE {
            SetFileTime(
                handle,
                std::ptr::null(),
                std::ptr::null(),
                &last_write as *const i64 as *const c_void,
            );
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
fn cp_set_modified_now(_target: &std::path::Path) {}

/// -u/--update: skip the copy when the source is not strictly newer than
/// the destination. Missing metadata is not grounds for skipping.
fn cp_source_not_newer(source: &std::path::Path, target: &std::path::Path) -> bool {
    match (
        fs::metadata(source).and_then(|meta| meta.modified()),
        fs::metadata(target).and_then(|meta| meta.modified()),
    ) {
        (Ok(source_time), Ok(target_time)) => source_time <= target_time,
        _ => false,
    }
}

/// cp -r: directories are created and walked in sorted entry order so the
/// result is deterministic. Symlinks are followed, which is cp's default
/// (without -d); the depth cap turns a link cycle into an error instead of
/// a stack overflow.
fn cp_copy_tree(source: &std::path::Path, target: &std::path::Path, depth: usize) -> io::Result<()> {
    if depth > 40 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "too many levels of symbolic links",
        ));
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return cp_copy_tree(&fs::canonicalize(source)?, target, depth + 1);
    }
    if metadata.is_dir() {
        fs::create_dir_all(target)?;
        let mut entries: Vec<_> = fs::read_dir(source)?.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            cp_copy_tree(entry.path().as_path(), &target.join(entry.file_name()), depth + 1)?;
        }
        return Ok(());
    }
    fs::copy(source, target)?;
    Ok(())
}

fn cat_file_operands(cmd: &CommandNode) -> Vec<&String> {
    let mut operands = Vec::new();
    let mut skip_next = false;
    let redirect_targets = cat_redirect_targets(cmd);

    for word in cmd.words.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if is_cat_redirect_operator_word(word) {
            skip_next = true;
            continue;
        }
        if word.starts_with('-') {
            continue;
        }
        if redirect_targets.iter().any(|target| *target == word) {
            continue;
        }
        operands.push(word);
    }

    operands
}

fn cat_redirect_targets(cmd: &CommandNode) -> Vec<&String> {
    [
        cmd.redirect_in.as_ref(),
        cmd.redirect_out.as_ref(),
        cmd.append.as_ref(),
        cmd.redirect_err.as_ref(),
        cmd.redirect_err_append.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(|redirect| &redirect.target)
    .collect()
}

fn cat_has_file_operands(cmd: &CommandNode) -> bool {
    !cat_file_operands(cmd).is_empty()
}

fn is_cat_redirect_operator_word(word: &str) -> bool {
    matches!(
        word,
        "<" | ">" | ">|" | ">>" | "2>" | "2>|" | "2>>" | "&>" | "&>>"
    ) || word.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && matches!(
            word.chars()
                .skip_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .as_str(),
            "<" | ">" | ">|" | ">>"
        )
}

impl Executor {
    /// GNU chmod: [options] mode file... The emulated mode bits live in
    /// __RUBASH_FILE_MODES so test -r/-w/-x observe them (Windows has no
    /// POSIX mode bits). Symbolic clauses [ugoa]*[+-=][rwxX]+ (comma
    /// separated) and octal modes are honored; unknown option-looking words
    /// that precede the mode are ignored the way coreutils skips -f/-R/-v.
    fn external_chmod(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let mut mode: Option<&str> = None;
        let mut files: Vec<String> = Vec::new();
        for arg in &cmd.words[1..] {
            if mode.is_none() {
                if matches!(arg.as_str(), "-f" | "-R" | "-v" | "-c" | "--") || arg.starts_with("--")
                {
                    continue;
                }
                if arg.starts_with('-') && arg.len() > 1 && arg[1..].chars().all(|ch| "fRvc".contains(ch))
                {
                    continue;
                }
                mode = Some(arg);
                continue;
            }
            files.push(arg.clone());
        }
        let Some(mode) = mode else {
            self.exit_code = 0;
            return Ok(true);
        };
        let mut failures = 0usize;
        for file in &files {
            let windows = crate::executor::path::shell_path_to_windows(file, &self.env_vars)
                .to_string_lossy()
                .to_string();
            // DrvFs (any Windows drive or \\wsl$ UNC) does not support POSIX
            // chmod bits; GNU on WSL leaves files on DrvFs 0777 after
            // `chmod -x`, so `test -x` stays true. To match GNU baseline
            // (posix2 negative -x expects failure when run as root on DrvFs),
            // skip emulation for any Windows drive path and let `test -x`
            // fall back to existence. This also covers the per-suite TMPDIR
            // when WSL does not forward TMPDIR to the Windows child (it
            // becomes C:\Users\...\Temp).
            let is_drive = windows.len() >= 2
                && windows.as_bytes()[1] == b':'
                && windows.as_bytes()[0].is_ascii_alphabetic();
            if is_drive || windows.starts_with("\\\\wsl$") || windows.starts_with("//wsl$") {
                continue;
            }
            let base = crate::builtins::test::emulated_file_mode(file, &self.env_vars)
                .unwrap_or_else(|| self.default_emulated_mode(&windows));
            match apply_chmod_mode(base, mode) {
                Some(new_mode) => {
                    store_emulated_file_mode(&mut self.env_vars, &windows, new_mode);
                }
                None => {
                    failures += 1;
                    eprintln!("chmod: invalid mode: '{}'", mode);
                }
            }
        }
        self.exit_code = i32::from(failures > 0 || files.is_empty());
        Ok(true)
    }

    /// Default rwx bits for a file never chmod'd: readable and writable like
    /// a fresh Windows file; executable only for extension-based executables
    /// (GNU-on-Linux would say not executable for a fresh text file).
    fn default_emulated_mode(&self, windows: &str) -> u32 {
        let executable = std::path::Path::new(windows)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "exe" | "com" | "bat" | "cmd"))
            .unwrap_or(false);
        let mut mode = 0o600u32;
        if executable {
            mode |= 0o111;
        }
        mode
    }
}

/// Apply one chmod MODE operand (octal or symbolic clauses) to BASE.
fn apply_chmod_mode(base: u32, mode: &str) -> Option<u32> {
    let trimmed = mode.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed.len() <= 4
        {
            return u32::from_str_radix(trimmed, 8).ok().map(|value| value & 0o777);
        }
    let mut current = base;
    for clause in trimmed.split(',') {
        let mut chars = clause.chars().peekable();
        let mut who = 0u32;
        let mut who_seen = false;
        while let Some(&ch) = chars.peek() {
            let bit = match ch {
                'u' => 0o700,
                'g' => 0o070,
                'o' => 0o007,
                'a' => 0o777,
                _ => break,
            };
            who |= bit;
            who_seen = true;
            chars.next();
        }
        if !who_seen {
            who = 0o777;
        }
        let op = chars.next()?;
        if !matches!(op, '+' | '-' | '=') {
            return None;
        }
        let mut perms = 0u32;
        while let Some(&ch) = chars.peek() {
            match ch {
                'r' => perms |= 0o444,
                'w' => perms |= 0o222,
                'x' => perms |= 0o111,
                'X' => {
                    // Directory, or some execute bit already set.
                    if (current & 0o111) != 0 {
                        perms |= 0o111;
                    }
                }
                _ => return None,
            }
            chars.next();
        }
        if chars.next().is_some() {
            return None;
        }
        match op {
            '+' => {
                current |= who & perms;
            }
            '-' => {
                current &= !(who & perms);
            }
            '=' => {
                current = (current & !who) | (who & perms);
            }
            _ => return None,
        }
    }
    Some(current & 0o777)
}

fn store_emulated_file_mode(env_vars: &mut HashMap<String, String>, windows: &str, mode: u32) {
    let key = crate::builtins::test::EMULATED_FILE_MODES;
    let entries = env_vars.get(key).cloned().unwrap_or_default();
    let mut kept: Vec<String> = entries
        .split('\x1f')
        .filter(|entry| {
            !entry.is_empty()
                && entry.rsplit_once('=').map(|(path, _)| path != windows).unwrap_or(true)
        })
        .map(str::to_string)
        .collect();
    kept.push(format!("{}={:o}", windows, mode));
    env_vars.insert(key.to_string(), kept.join("\x1f"));
}
