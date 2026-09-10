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
            "cp" => {
                // Fast-path decline design (mirrors external_sed): flag forms
                // beyond the simple in-process copies (e.g. -r/-v/-p) are
                // handled by the real cp -- winuxcmd in the product, the
                // host coreutils in the test harness -- both GNU-shaped.
                if cmd.words[1..]
                    .iter()
                    .any(|arg| arg.starts_with('-') && arg != "-")
                {
                    return Ok(false);
                }
                self.external_cp(cmd)
            }
            "rm" => self.external_rm(cmd),
            "rmdir" => self.external_rmdir(cmd),
            "cat" | "/bin/cat" | "/usr/bin/cat" => {
                // cat -n/-b/-s/-A and friends are delegated to the real cat;
                // the in-process copy only serves plain operand forms.
                if cmd.words[1..]
                    .iter()
                    .any(|arg| arg.starts_with('-') && arg != "-")
                {
                    return Ok(false);
                }
                self.external_cat(cmd)
            }
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
        // GNU mkdir (coreutils): operands processed independently; without
        // -p an existing directory is an error ("File exists"), -p suppresses
        // only that error; -m MODE applies the emulated permission bits.
        // Option-looking words are flags (-- ends parsing, -m consumes one).
        let mut parents = false;
        let mut verbose = false;
        let mut mode: Option<String> = None;
        let mut mode_value_pending = false;
        let mut end_of_flags = false;
        let mut operands: Vec<String> = Vec::new();
        for word in &cmd.words[1..] {
            if end_of_flags {
                operands.push(word.clone());
                continue;
            }
            if word == "--" {
                end_of_flags = true;
                continue;
            }
            if !mode_value_pending && word.starts_with('-') && word.len() > 1 {
                for ch in word.chars().skip(1) {
                    match ch {
                        'p' => parents = true,
                        'v' => verbose = true,
                        'm' => mode_value_pending = true,
                        _ => {}
                    }
                }
                continue;
            }
            if mode_value_pending {
                mode_value_pending = false;
                mode = Some(word.clone());
                continue;
            }
            operands.push(word.clone());
        }

        if operands.is_empty() {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "mkdir: missing operand\nTry 'mkdir --help' for more information."
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let mut stderr = Vec::new();
        let mut status = 0;
        for path in &operands {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            let result = if parents {
                fs::create_dir_all(&target)
            } else if target.is_dir() {
                Err(io::Error::new(io::ErrorKind::AlreadyExists, "File exists"))
            } else {
                fs::create_dir(&target)
            };
            match result {
                Ok(()) => {
                    if verbose {
                        writeln!(&mut stderr, "created directory '{}'", expanded)?;
                    }
                    if let Some(mode) = &mode {
                        let windows = target.to_string_lossy().into_owned();
                        let base = crate::builtins::test::emulated_file_mode(&expanded, &self.env_vars)
                            .unwrap_or_else(|| self.default_emulated_mode(&windows));
                        if let Some(new_mode) = apply_chmod_mode(base, mode) {
                            store_emulated_file_mode(&mut self.env_vars, &windows, new_mode);
                        }
                    }
                }
                Err(error) => {
                    let suppress = parents && error.kind() == io::ErrorKind::AlreadyExists;
                    if !suppress {
                        status = 1;
                        let message = if error.kind() == io::ErrorKind::AlreadyExists {
                            "File exists".to_string()
                        } else {
                            crate::posix_errors::message(&error)
                        };
                        writeln!(
                            &mut stderr,
                            "mkdir: cannot create directory \u{2018}{}\u{2019}: {message}",
                            expanded
                        )?;
                    }
                }
            }
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = status;
        Ok(true)
    }

    fn external_touch(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU touch.c processes every operand independently: a failed create
        // prints `touch: cannot touch 'FILE': ...` and continues with the
        // remaining files, leaving exit status 1 (touch.c: do_touch loop).
        let mut operands: Vec<String> = Vec::new();
        let mut end_of_flags = false;
        for word in &cmd.words[1..] {
            if end_of_flags {
                operands.push(word.clone());
                continue;
            }
            if word == "--" {
                end_of_flags = true;
                continue;
            }
            if word.starts_with('-') && word.len() > 1 {
                continue;
            }
            operands.push(word.clone());
        }
        if operands.is_empty() {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "touch: missing file operand\nTry 'touch --help' for more information."
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }
        let mut failed = false;
        let mut stderr = Vec::new();
        for path in &operands {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            if let Err(error) = File::create(target) {
                writeln!(
                    &mut stderr,
                    "touch: cannot touch '{}': {}",
                    expanded,
                    crate::posix_errors::message(&error)
                )?;
                failed = true;
            }
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = if failed { 1 } else { 0 };
        Ok(true)
    }

    fn external_cp(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let mut args = Vec::new();
        for word in &cmd.words[1..] {
            if !word.starts_with('-') {
                args.push(self.expand_word(word));
            }
        }

        if args.len() == 2
            && crate::executor::path::is_shell_null_device(&args[0])
            && crate::executor::path::is_shell_null_device(&args[1])
        {
            // GNU copy.c rejects `cp /dev/null /dev/null` as a same-file
            // copy before any data would move.
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "cp: '{}' and '{}' are the same file",
                args[0], args[1]
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        if args.is_empty() {
            let mut stderr = Vec::new();
            writeln!(&mut stderr, "cp: missing file operand")?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }
        if args.len() == 1 {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "cp: missing destination file operand after '{}'",
                args[0]
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let destination =
            shell_path_to_windows(args.last().expect("cp destination"), &self.env_vars);
        if args.len() > 2 && !destination.is_dir() {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "cp: target '{}' is not a directory",
                args.last().expect("cp destination")
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        // GNU null-device semantics: /dev/null reads as an empty stream and
        // discards writes (coreutils copy.c opens the source O_RDONLY and
        // gets EOF, so `cp /dev/null TARGET` creates/truncates TARGET as an
        // empty file -- basename "null" when TARGET is a directory -- while
        // `cp FILE /dev/null` discards FILE with status 0). Windows has no
        // device CopyFileExW can stat, so model both directions explicitly
        // instead of failing with "Invalid argument".
        if args.len() == 2
            && crate::executor::path::is_shell_null_device(args.last().expect("cp destination"))
        {
            self.exit_code = 0;
            return Ok(true);
        }

        // GNU copy.c processes sources independently: a failed source is
        // reported ("cannot stat" vs "cannot create" wording) and the
        // remaining sources are still copied; any failure leaves status 1.
        let mut stderr = Vec::new();
        let mut status = 0;
        for source in &args[..args.len() - 1] {
            if crate::executor::path::is_shell_null_device(source) {
                let name = source
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
                    writeln!(
                        &mut stderr,
                        "cp: cannot create '{}': {}",
                        target_path.display(),
                        crate::posix_errors::message(&error)
                    )?;
                }
                continue;
            }

            let source_path = shell_path_to_windows(source, &self.env_vars);
            let target_path = if destination.is_dir() {
                let Some(name) = source_path.file_name() else {
                    status = 1;
                    writeln!(
                        &mut stderr,
                        "cp: cannot stat '{}': No such file or directory",
                        source
                    )?;
                    continue;
                };
                destination.join(name)
            } else {
                destination.clone()
            };

            if let Err(error) = fs::copy(&source_path, &target_path) {
                // GNU cp wording: source stat failures use "cannot stat",
                // everything else reports the destination operation.
                if !source_path.exists() {
                    writeln!(
                        &mut stderr,
                        "cp: cannot stat '{}': {}",
                        source,
                        crate::posix_errors::message(&error)
                    )?;
                } else {
                    writeln!(
                        &mut stderr,
                        "cp: cannot create '{}': {}",
                        target_path.display(),
                        crate::posix_errors::message(&error)
                    )?;
                }
                status = 1;
            }
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = status;
        Ok(true)
    }

    fn external_rm(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU rm (coreutils remove.c): operands processed independently with
        // exit status 1 if any failed. -f suppresses only nonexistent-file
        // diagnostics; removing a directory without -r/-R fails with "Is a
        // directory" even under -f; -v prints "removed 'x'" after success.
        let mut force = false;
        let mut recursive = false;
        let mut verbose = false;
        let mut end_of_flags = false;
        let mut operands: Vec<String> = Vec::new();
        for arg in &cmd.words[1..] {
            if end_of_flags {
                operands.push(arg.clone());
                continue;
            }
            if arg == "--" {
                end_of_flags = true;
                continue;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg.chars().skip(1) {
                    match ch {
                        'f' => force = true,
                        'r' | 'R' => recursive = true,
                        'v' => verbose = true,
                        _ => {}
                    }
                }
                continue;
            }
            operands.push(arg.clone());
        }

        if operands.is_empty() {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "rm: missing operand\nTry 'rm --help' for more information."
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let mut stderr = Vec::new();
        let mut stdout = Vec::new();
        let mut status = 0;
        for path in &operands {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            let was_dir = target.is_dir();
            let result = if was_dir {
                if recursive {
                    fs::remove_dir_all(&target)
                } else {
                    Err(io::Error::new(io::ErrorKind::Other, "Is a directory"))
                }
            } else {
                fs::remove_file(&target)
            };
            match result {
                Ok(()) => {
                    if verbose {
                        // GNU rm -v reports removals on stdout.
                        writeln!(&mut stdout, "removed '{}'", expanded)?;
                    }
                }
                Err(error) => {
                    let suppress = force && error.kind() == io::ErrorKind::NotFound;
                    if !suppress {
                        status = 1;
                        let message = if was_dir {
                            "Is a directory".to_string()
                        } else if matches!(
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
        }
        if !stdout.is_empty() {
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = status;
        Ok(true)
    }

    fn external_rmdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU rmdir (coreutils rmd-ir.c... rmdir.c): every operand must be an
        // empty directory; failures are reported per operand and any failure
        // leaves exit status 1.
        let mut end_of_flags = false;
        let mut operands: Vec<String> = Vec::new();
        for arg in &cmd.words[1..] {
            if end_of_flags {
                operands.push(arg.clone());
                continue;
            }
            if arg == "--" {
                end_of_flags = true;
                continue;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                continue;
            }
            operands.push(arg.clone());
        }

        if operands.is_empty() {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "rmdir: missing operand\nTry 'rmdir --help' for more information."
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }

        let mut stderr = Vec::new();
        let mut status = 0;
        for path in &operands {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            let result = if target.is_dir() {
                fs::remove_dir(&target)
            } else {
                Err(io::Error::new(io::ErrorKind::NotFound, "No such file or directory"))
            };
            if let Err(error) = result {
                status = 1;
                let message = if target.is_dir() {
                    "Directory not empty".to_string()
                } else if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                ) {
                    "No such file or directory".to_string()
                } else {
                    crate::posix_errors::message(&error)
                };
                writeln!(
                    &mut stderr,
                    "rmdir: failed to remove '{}': {message}",
                    expanded
                )?;
            }
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = status;
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

        // GNU cat (coreutils cat.c): operands processed in order, a missing
        // file is reported per operand and the remaining files are still
        // concatenated; any failure leaves exit status 1.
        let mut output = Vec::new();
        let mut stderr = Vec::new();
        let mut failures = 0usize;
        for word in cat_file_operands(cmd) {
            let expanded = self.expand_word(word);
            match fs::read(shell_path_to_windows(&expanded, &self.env_vars)) {
                Ok(bytes) => output.extend(bytes),
                Err(_) => {
                    writeln!(
                        &mut stderr,
                        "cat: {}: No such file or directory",
                        expanded
                    )?;
                    failures += 1;
                }
            }
        }
        self.write_cat_output(cmd, &output)?;
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = if failures > 0 { 1 } else { 0 };
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
        // GNU mkfifo (coreutils): operands processed independently; a create
        // failure (including an existing node) is reported per operand and
        // any failure leaves exit status 1.
        let mut stderr = Vec::new();
        let mut status = 0;
        for path in cmd.words.iter().skip(1).filter(|arg| !arg.starts_with('-')) {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            if let Err(error) = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                status = 1;
                let message = if error.kind() == io::ErrorKind::AlreadyExists {
                    "File exists".to_string()
                } else {
                    crate::posix_errors::message(&error)
                };
                writeln!(
                    &mut stderr,
                    "mkfifo: cannot create fifo '{}': {message}",
                    expanded
                )?;
            }
        }
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        self.exit_code = status;
        Ok(true)
    }
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
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "chmod: missing operand\nTry 'chmod --help' for more information."
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        };
        if files.is_empty() {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "chmod: missing operand after \u{2018}{mode}\u{2019}\nTry 'chmod --help' for more information."
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 1;
            return Ok(true);
        }
        let mut failures = 0usize;
        for file in &files {
            let windows = crate::executor::path::shell_path_to_windows(file, &self.env_vars)
                .to_string_lossy()
                .to_string();
            if !std::path::Path::new(&windows).exists()
                && crate::builtins::test::emulated_file_mode(file, &self.env_vars).is_none()
            {
                let mut stderr = Vec::new();
                writeln!(
                    &mut stderr,
                    "chmod: cannot access '{}': No such file or directory",
                    file
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                failures += 1;
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
                    let mut stderr = Vec::new();
                    writeln!(&mut stderr, "chmod: invalid mode: '{}'", mode)?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                }
            }
        }
        self.exit_code = i32::from(failures > 0);
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
